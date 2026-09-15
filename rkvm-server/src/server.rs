use rkvm_net::event::Event;
use rkvm_net::key::{Key, KeyEvent};
use rkvm_input::monitor::{Monitor, MonitorPlatform};
use rkvm_input::device::DeviceSpec;
use rkvm_net::auth::{AuthChallenge, AuthResponse, AuthStatus};
use rkvm_net::message::Message;
use rkvm_net::version::Version;
use rkvm_net::Update;
use slab::Slab;
use std::collections::{HashMap, VecDeque};
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
use crate::set::Set;
use crate::state::{KeyAction, KeyState};

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
    switch_keys: Set<Key>,
    propagate_switch_keys: bool,
    server_goto_keys: Option<Set<Key>>,
    clients_config: Vec<ClientConfig>,
    device_allowlist: Vec<DeviceSpec>,
) -> Result<(), Error> {
    let listener = TcpListener::bind(&listen).await.map_err(Error::Io)?;
    tracing::info!("Listening on {}", listen);

    let mut monitor = Monitor::new(device_allowlist);
    let mut init_updates = Slab::<Update>::new();
    let mut clients: Slab<Client> = Slab::new();
    
    let mut current = 0;
    let mut static_client_indices: HashMap<IpAddr, usize> = HashMap::new();

    let mut state = KeyState::new(propagate_switch_keys);
    state.add_action(switch_keys, KeyAction::NextClient);
    if let Some(keys) = server_goto_keys {
        state.add_action(keys, KeyAction::Goto(0));
    }

    // Insert local client at index 0
    let mut local_client = LocalClient::new();
    #[cfg(target_os = "linux")]
    local_client.set_registry(monitor.registry());
    let _ = clients.insert(Client::Local(local_client));

    // Insert placeholder clients for static clients
    for c in clients_config {
        let idx = clients.insert(Client::Empty);
        static_client_indices.insert(c.addr, idx);
        if let Some(k) = &c.goto_keys {
            let keys: Set<Key> = k.clone().into_iter().map(Into::into).collect();
            state.add_action(keys, KeyAction::Goto(idx));
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
                if let Some(&idx) = static_client_indices.get(&addr.ip()) {
                    if clients[idx].is_connected() {
                        tracing::warn!(%addr, "Static client already connected, rejecting duplicate connection");
                        continue;
                    }
                    clients[idx] = Client::Remote(RemoteClient::new(idx, addr, init_co, sender.clone()));
                } else {
                    let idx = clients.vacant_key();
                    clients.insert(Client::Remote(RemoteClient::new(idx, addr, init_co, sender.clone())));
                }
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
                     Update::DestroyDevice { id, .. } => {
                        state.remove_device(id);
                        for (_, client) in clients.iter_mut() {
                            let _ = client.send(update.clone()).await;
                        }
                    }
                    Update::Event { id, ref event, .. } => {
                        let action = match event {
                            Event::Key(KeyEvent { key, down }) => state.update(id, key, down),
                            _ => KeyAction::Forward,
                        };

                        match action {
                            KeyAction::NextClient => {
                                let next = next_client(&clients, current);
                                current = switch_client(&mut clients, &state, current, next).await;
                            }
                            KeyAction::Goto(goto) => {
                                current = switch_client(&mut clients, &state, current, goto).await;
                            }
                            KeyAction::Delay => {
                                // TODO
                            }
                            KeyAction::Forward => {
                                if let Some(client) = clients.get_mut(current) {
                                    let _ = client.send(update).await;
                                }
                            }
                        };
                    }
                    _ => {}
                }
            }
            disconnect  = receiver.recv() => {
                if let Some((idx,addr)) = disconnect {
                    if static_client_indices.get(&addr.ip()) == Some(&idx) {
                        clients[idx] = Client::Empty;
                    } else {
                        clients.remove(idx);
                    }
                }
            }
        }
    }
}
fn next_client(clients: &Slab<Client>, mut idx: usize) -> usize {
    loop {
        idx = (idx + 1) % clients.capacity();
        if clients.contains(idx) {
            return idx;
        }
    }
}

async fn switch_client(clients: &mut Slab<Client>, state: &KeyState, current: usize, next: usize) -> usize {
    if current == next || !clients.get(next).is_some_and(|client| client.is_connected()) {
        return current;
    }
    if state.propagate() {
        if let Some(client) = clients.get_mut(current) {
            state.send_state(client, false).await;
        }
        if let Some(client) = clients.get_mut(next) {
            state.send_state(client, true).await;
        }
    }
    if next == 0 {
        tracing::info!(idx = %next, "Switched to local");
    } else {
        tracing::info!(idx = %next, "Switched to remote client");
    }
    next
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
