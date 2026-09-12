use rkvm_net::event::Event;
use rkvm_net::key::{Key, KeyEvent};
use rkvm_input::monitor::{Monitor, MonitorPlatform};
use rkvm_net::rel::RelAxis;
use rkvm_net::abs::{AbsAxis, AbsInfo};
use rkvm_input::device::DeviceSpec;
use rkvm_net::auth::{AuthChallenge, AuthResponse, AuthStatus};
use rkvm_net::message::Message;
use rkvm_net::version::Version;
use rkvm_net::Update;
use slab::Slab;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::CString;
use std::io;
use std::net::{SocketAddr, IpAddr, Ipv4Addr};
use std::time::Instant;
use thiserror::Error;
use tokio::io::{AsyncWriteExt, BufStream};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{self, Receiver};
use tokio::time;
use tokio_rustls::TlsAcceptor;
use tracing::Instrument;

use crate::client::Client;
use crate::config::ClientConfig;

const ADDR_UNKNOWN: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0);

#[derive(Error, Debug)]
pub enum Error {
    #[error("Network error: {0}")]
    Network(io::Error),
    #[error("Input error: {0}")]
    Input(io::Error),
    #[error("Event queue overflow")]
    Overflow,
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
    let listener = TcpListener::bind(&listen).await.map_err(Error::Network)?;
    tracing::info!("Listening on {}", listen);

    let mut monitor = Monitor::new(device_allowlist);
    let mut devices = Slab::<Device>::new();
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
    let local_idx = clients.insert(Client::Local);

    // Insert placeholder clients for static clients
    for c in clients_config {
        let idx = clients.insert(Client::Static(None));
        static_client_indices.insert(c.addr, idx);
        if let Some(k) = &c.goto_keys {
            let keys: Vec<Key> = k.clone().into_iter().map(Into::into).collect();
            goto_keys.insert(keys.clone(), idx);
            all_switch_keys.extend(keys);
        }
    }

    loop {
        tokio::select! {
            result = listener.accept() => {
                let (stream, addr) = result.map_err(Error::Network)?;
                let acceptor = acceptor.clone();
                let password = password.to_owned();

                let init_updates = devices
                    .iter()
                    .map(|(id, device)| Update::CreateDevice {
                        id,
                        name: device.name.clone(),
                        version: device.version,
                        vendor: device.vendor,
                        product: device.product,
                        rel: device.rel.clone(),
                        abs: device.abs.clone(),
                        keys: device.keys.clone(),
                        delay: device.delay,
                        period: device.period,
                    })
                    .collect();

                let (sender, receiver) = mpsc::channel(1);

                // Find if it's a static client
                let client_idx = if let Some(idx) = static_client_indices.get(&addr.ip()).copied() {
                    clients[idx] = Client::Static(Some(sender));
                    idx
                } else {
                    // New dynamic client
                    clients.insert(Client::Remote(sender))
                };

                let span = tracing::info_span!("connection", addr = %addr, idx = %client_idx);
                tokio::spawn(
                    async move {
                        tracing::info!("Connected");

                        match client(init_updates, receiver, stream, acceptor, &password).await {
                            Ok(()) => tracing::info!("Disconnected"),
                            Err(err) => tracing::error!("Disconnected: {}", err),
                        }
                    }
                    .instrument(span),
                );
            }
            result = monitor.read() => {
                let update = result.map_err(Error::Input)?;

                match update {
                    Update::CreateDevice { .. } | Update::DestroyDevice { .. } => {
                        // Broadcast device changes to all connected clients
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
                        if let Some(client) = clients.get(idx) {
                            let _ = client.send(update).await;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

struct Device {
    name: CString,
    vendor: u16,
    product: u16,
    version: u16,
    rel: HashSet<RelAxis>,
    abs: HashMap<AbsAxis, AbsInfo>,
    keys: HashSet<Key>,
    delay: Option<i32>,
    period: Option<i32>,
}

#[derive(Error, Debug)]
enum ClientError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("Incompatible client version (got {client}, expected {server})")]
    Version { server: Version, client: Version },
    #[error("Invalid password")]
    Auth,
    #[error(transparent)]
    Rand(#[from] rand::Error),
}

async fn client(
    mut init_updates: VecDeque<Update>,
    mut receiver: Receiver<Update>,
    stream: TcpStream,
    acceptor: TlsAcceptor,
    password: &str,
) -> Result<(), ClientError> {
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
        return Err(ClientError::Version {
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
        return Err(ClientError::Auth);
    }

    tracing::info!("Authenticated successfully");

    let mut interval = time::interval(rkvm_net::PING_INTERVAL);

    loop {
        let recv = async {
            match init_updates.pop_front() {
                Some(update) => Some(update),
                None => receiver.recv().await,
            }
        };

        let update = tokio::select! {
            // Make sure pings have priority.
            // The client could time out otherwise.
            biased;

            _ = interval.tick() => Some(Update::Ping),
            recv = recv => recv,
        };

        let update = match update {
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

    Ok(())
}
