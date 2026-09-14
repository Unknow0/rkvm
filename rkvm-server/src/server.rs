use rkvm_net::event::Event;
use rkvm_net::key::{Key, KeyEvent};
use rkvm_input::monitor::{Monitor, MonitorPlatform};
use rkvm_input::device::DeviceSpec;
use rkvm_net::auth::{AuthChallenge, AuthResponse, AuthStatus};
use rkvm_net::message::Message;
use rkvm_net::version::Version;
use rkvm_net::Update;
use slab::Slab;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::net::{SocketAddr, IpAddr};
use std::time::Instant;
use thiserror::Error;
use tokio::io::{AsyncWriteExt, BufStream};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::channel;
use tokio_rustls::{TlsAcceptor, server::TlsStream};

use crate::client::{Client, LocalClient, RemoteClient};
use crate::config::ClientConfig;

#[derive(Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("Incompatible client version (got {client}, expected {server})")]
    Version { server: Version, client: Version },
    #[error("Invalid password")]
    Auth,
    #[error(transparent)]
    Rand(#[from] rand::Error),
}

pub async fn run(
    listen: SocketAddr,
    acceptor: TlsAcceptor,
    password: &str,
    switch_keys: &HashSet<Key>,
    propagate_switch_keys: bool,
    server_goto_keys: &Option<Vec<Key>>,
    clients_config: &Vec<ClientConfig>,
    device_allowlist: Vec<DeviceSpec>,
) -> Result<(), Error> {
    let listener = TcpListener::bind(&listen).await.map_err(Error::Io)?;
    tracing::info!("Listening on {}", listen);

    let mut monitor = Monitor::new(device_allowlist);
    let mut init_updates = Slab::<Update>::new();
    let mut clients: Slab<Client> = Slab::new();
    
    let mut current = 0;
    let mut previous = 0;
    let mut changed = false;
    let mut pressed_keys = HashSet::new();
    let mut all_switch_keys = switch_keys.clone();
    let mut static_client_indices: HashMap<IpAddr, usize> = HashMap::new();
    let mut goto_keys: HashMap<Vec<Key>, usize> = HashMap::new();

    if let Some(keys) = server_goto_keys {
        goto_keys.insert(keys.clone(), 0);
        all_switch_keys.extend(keys);
    }

    // Insert local client at index 0
    let mut local_client = LocalClient::new();
    #[cfg(target_os = "linux")]
    local_client.set_registry(monitor.registry());
    let local_idx = clients.insert(Client::Local(local_client));

    // Insert placeholder clients for static clients
    for c in clients_config {
        let idx = clients.insert(Client::Empty);
        static_client_indices.insert(c.addr, idx);
        if let Some(k) = &c.goto_keys {
            let keys: Vec<Key> = k.clone().into_iter().map(Into::into).collect();
            goto_keys.insert(keys.clone(), idx);
            all_switch_keys.extend(keys);
        }
    }
    let (sender, mut receiver) = channel(16);
    loop {
        tokio::select! {
            result = listener.accept() => {
                let (stream, addr) = result.map_err(Error::Io)?;
                let acceptor = acceptor.clone();
                let password = password.to_owned();

                let init_updates = init_updates.iter().map(|(_,u)| u.clone()).collect();
                let init_co = async move || {
                    init_connection(init_updates, stream, acceptor, &password).await
                };
                // Find if it's a static client
                let idx = if let Some(&idx) = static_client_indices.get(&addr.ip()) {
                    if clients[idx].is_connected() {
                        tracing::warn!(%addr, "Static client already connected, rejecting duplicate connection");
                        continue;
                    }
                    idx
                } else {
                    clients.vacant_entry().key()
                };

                let remote = RemoteClient::new(idx, addr, init_co, sender.clone());
                clients[idx] = Client::Remote(remote);
            }
            result = monitor.read() => {
                let update = result.map_err(Error::Io)?;

                match update {
                    Update::CreateDevice { .. } => {
                        for (_, client) in clients.iter_mut() {
                            let _ = client.send(update.clone()).await;
                        }
                        init_updates.insert(update);
                    }
                     Update::DestroyDevice { .. } => {
                        for (_, client) in clients.iter_mut() {
                            let _ = client.send(update.clone()).await;
                        }
                    }
                    Update::Event { ref event, .. } => {
                        let mut press = false;

                        if let Event::Key(KeyEvent { key, down }) = event {
                            if all_switch_keys.contains(&key) {
                                press = true;

                                match down {
                                    true => pressed_keys.insert(*key),
                                    false => pressed_keys.remove(key),
                                };
                            }
                        }

                        let mut idx = current;

                        if press {
                            let exists = |check_idx: usize| {
                                clients.get(check_idx).is_some_and(|c| c.is_connected())
                            };

                            if changed {
                                idx = previous;

                                if pressed_keys.is_empty() {
                                    changed = false
                                }
                            } else {
                                for (keys, &i) in &goto_keys {
                                    if exists(i) && keys.iter().all(|k| pressed_keys.contains(k)) {
                                        current = i;
                                        changed = true;
                                        break;
                                    }
                                }

                                if !changed && switch_keys.is_subset(&pressed_keys) {
                                    loop {
                                        current = (current + 1) % (clients.len() + 1);
                                        if exists(current) {
                                            break;
                                        }
                                    }

                                    changed = true;
                                }
                                if changed {
                                    previous = idx;
                                    if current == local_idx {
                                        tracing::info!(idx = %current, "Switched to local");
                                    } else if let Some(_) = clients.get(current) {
                                        tracing::info!(idx = %current, "Switched to remote client");
                                    }
                                }
                            }
                        }

                        if press && !propagate_switch_keys {
                            continue;
                        }

                        // Send event only to target client
                        if let Some(client) = clients.get_mut(idx) {
                            let _ = client.send(update).await;
                        }
                    }
                    _ => {}
                }
            }
            disconnect  = receiver.recv() => {
                if let Some((idx,addr)) = disconnect {
                    if static_client_indices.contains_key(&addr.ip()) {
                        clients[idx] = Client::Empty;
                    } else {
                        clients.remove(idx);
                    }
                }
            }
        }
    }
}

async fn init_connection(mut init_updates: VecDeque<Update>, stream: TcpStream, acceptor: TlsAcceptor, password: &str) -> Result<BufStream<TlsStream<TcpStream>>, Error> {
    let stream = rkvm_net::timeout(rkvm_net::TLS_TIMEOUT, acceptor.accept(stream)).await?;
    tracing::info!("TLS connected");

    let mut stream = BufStream::with_capacity(1024, 1024, stream);

    rkvm_net::timeout(rkvm_net::WRITE_TIMEOUT, async {
        Version::CURRENT.encode(&mut stream).await?;
        stream.flush().await?;

        Ok(())
    })
    .await?;

    let version = rkvm_net::timeout(rkvm_net::READ_TIMEOUT, Version::decode(&mut stream)).await?;
    if version != Version::CURRENT {
        return Err(Error::Version {
            server: Version::CURRENT,
            client: version,
        });
    }

    let challenge = AuthChallenge::generate().await?;

    rkvm_net::timeout(rkvm_net::WRITE_TIMEOUT, async {
        challenge.encode(&mut stream).await?;
        stream.flush().await?;

        Ok(())
    })
    .await?;

    let response = rkvm_net::timeout(rkvm_net::READ_TIMEOUT, AuthResponse::decode(&mut stream)).await?;
    let status = match response.verify(&challenge, password) {
        true => AuthStatus::Passed,
        false => AuthStatus::Failed,
    };

    rkvm_net::timeout(rkvm_net::WRITE_TIMEOUT, async {
        status.encode(&mut stream).await?;
        stream.flush().await?;

        Ok(())
    })
    .await?;

    if status == AuthStatus::Failed {
        return Err(Error::Auth);
    }

    tracing::info!("Authenticated successfully");

     loop {
        let update = match init_updates.pop_front() {
            Some(update) => update,
            None => break,
        };
        let start = Instant::now();
        rkvm_net::timeout(rkvm_net::WRITE_TIMEOUT, async {
            update.encode(&mut stream).await?;
            stream.flush().await?;

            Ok(())
        })
        .await?;

        tracing::trace!(duration = ?start.elapsed(), "Wrote an update");
    }

    Ok(stream)
}
