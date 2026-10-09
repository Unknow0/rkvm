use rkvm_net::event::Event;
use rkvm_net::key::{Key, KeyEvent};
use rkvm_input::monitor::{Monitor, MonitorPlatform};
use rkvm_input::device::DeviceSpec;
use rkvm_net::auth::{AuthChallenge, AuthStatus};
use rkvm_net::message::Message;
use rkvm_net::version::Version;
use rkvm_net::{ClientStart, LedState, Update};
use slab::Slab;
use std::collections::{HashMap, VecDeque};
use std::io;
use std::net::{SocketAddr, IpAddr};
use thiserror::Error;
use tokio::io::{AsyncWriteExt, BufStream};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{sleep, Duration, Instant};
use tokio::sync::mpsc::{channel, Sender};
use tokio_rustls::{TlsAcceptor, server::TlsStream};
use tracing::Instrument;

use crate::client::{Client, LocalClient, RemoteClient};
use crate::config::ClientConfig;
use crate::set::Set;
use crate::state::{KeyAction, KeyPressed, KeyState};

const NEVER: Duration = Duration::from_secs(366 * 24* 3600);

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
    propagate_delay: Duration,
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

    let mut state = KeyState::new(propagate_delay.as_nanos() == 0);
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
        let idx = clients.insert(Client::Reserved);
        static_client_indices.insert(c.addr, idx);
        if let Some(k) = &c.goto_keys {
            let keys: Set<Key> = k.clone().into_iter().map(Into::into).collect();
            state.add_action(keys, KeyAction::Goto(idx));
        }
    }
    let (disconnect_tx, mut disconnect_rx) = channel(16);
    let (client_tx, mut client_rx) = channel(16);
    let mut pending_keys = Vec::new();
    let mut pressed_keys = Vec::new();
    let pending_timer = sleep(NEVER);
    tokio::pin!(pending_timer);
    loop {
        tokio::select! {
            result = listener.accept() => {
                let (stream, addr) = result.map_err(Error::Io)?;
                let acceptor = acceptor.clone();
                let password = password.to_owned();

                // Find if it's a static client
                let idx = if let Some(&idx) = static_client_indices.get(&addr.ip()) {
                    if clients[idx].is_connected() {
                        tracing::warn!(%addr, "Static client already connected, rejecting duplicate connection");
                        continue;
                    }
                    clients[idx] = Client::Connecting;
                    idx
                } else {
                    clients.insert(Client::Connecting)
                };

                let init_updates = init_updates.iter().map(|(_,u)| u.clone()).collect();
                let tx = client_tx.clone();
                let dx = disconnect_tx.clone();

                let span = tracing::info_span!("connection", addr = %addr, idx = %idx);
                tokio::spawn(async move {
                    match init_client(idx, addr, stream, acceptor, &password, init_updates, dx.clone()).await {
                        Ok(c) => {
                            tracing::info!("Client ready");
                            let _ = tx.send((idx, Client::Remote(c)));
                        }
                        Err(e) => {
                            tracing::warn!("Failed to connect client: {:?}", e);
                            let _ = dx.send((idx, addr)).await;
                        }
                    }
                }.instrument(span));
            }
            result = monitor.read() => {
                let update = result.map_err(Error::Io)?;

                match update {
                    Update::CreateDevice { .. } => {
                        broadcast(&mut clients, update.clone()).await;
                        init_updates.insert(update);
                    }
                     Update::DestroyDevice { id, .. } => {
                        remove_device(id, &mut pending_keys, &mut state);
                        remove_device(id, &mut pressed_keys, &mut state);
                        broadcast(&mut clients, update).await;
                    }
                    Update::Event { id, ref event, .. } => {
                        match event {
                            Event::Key(KeyEvent { key, down }) => {
                                if let Some(client) = clients.get_mut(current) {
                                    if let Some(leds) = client.update_leds(key, down) {
                                        let _ = monitor.update_leds(leds).await;
                                    }
                                }
                                let action = state.update(key, down);
								match action {
                                        KeyAction::NextClient => {
                                            pending_keys.clear();
                                            pending_timer.as_mut().reset(Instant::now() + NEVER);
                                            if state.propagate() {
                                                send(&mut clients, current, update).await;
                                            }
                                            let next = next_client(&clients, current);
                                            current = switch_client(&mut clients, &mut monitor, current, next, &pressed_keys).await;
                                        }
                                        KeyAction::Goto(goto) => {
                                            pending_keys.clear();
                                            pending_timer.as_mut().reset(Instant::now() + NEVER);
                                            if state.propagate() {
                                                send(&mut clients, current, update.clone()).await;
                                            }
                                            current = switch_client(&mut clients, &mut monitor, current, goto, &pressed_keys).await;
                                        }
                                        KeyAction::Delay => {
                                            pending_keys.push(KeyPressed{id: id, key: *key});
                                            pending_timer.as_mut().reset(Instant::now() + propagate_delay);
                                        }
                                        KeyAction::Forward => {
											if !pending_keys.is_empty() {
												send_keys(&mut clients, current, &pending_keys, true).await;
												pressed_keys.append(&mut pending_keys);
												pending_timer.as_mut().reset(Instant::now() + NEVER);
											}
                                            send(&mut clients, current, update.clone()).await;
                                            match down {
                                                true => pressed_keys.push(KeyPressed{id: id, key: *key}),
                                                false => pressed_keys.retain(|k| k.id != id || k.key != *key),
                                            };
                                        }
                                    }                            }
                            _ => send(&mut clients, current, update).await
                        }
                    }
                    _ => {}
                }
            }
            connect = client_rx.recv() => {
                if let Some((idx, c)) = connect {
                    clients[idx] = c;
                }
            }
            disconnect  = disconnect_rx.recv() => {
                if let Some((idx,addr)) = disconnect {
                    if static_client_indices.get(&addr.ip()) == Some(&idx) {
                        clients[idx] = Client::Reserved;
                    } else {
                        clients.remove(idx);
                    }
                }
            }
            _ = &mut pending_timer => {
                send_keys(&mut clients, current, &pending_keys, true).await;
                pressed_keys.append(&mut pending_keys);
                pending_timer.as_mut().reset(Instant::now() + NEVER);
            }
        }
    }
}

async fn broadcast(clients: &mut Slab<Client>, update: Update) {
    tracing::trace!("Broadcasting update: {:?}", update);
    for (_, client) in clients.iter_mut() {
        let _ = client.send(update.clone()).await;
    }
}

async fn send(clients: &mut Slab<Client>, idx: usize, update: Update) {
    tracing::trace!(%idx, "Sending update: {:?}", update);
    if let Some(client) = clients.get_mut(idx) {
        let _ = client.send(update).await;
    }
}

async fn send_keys(clients: &mut Slab<Client>, idx: usize, keys: &Vec<KeyPressed>, down: bool) {
    if let Some(client) = clients.get_mut(idx) {
        for k in keys {
            let update = Update::Event{ id: k.id, event: Event::Key(KeyEvent{key: k.key, down: down})};
            tracing::trace!(%idx, "Sending update: {:?}", update);
            let _ = client.send(update).await;
        }
    }
}

fn remove_device(id: usize, keys: &mut Vec<KeyPressed>, state: &mut KeyState) {
    keys.retain(|k| {
        if k.id == id {
            state.update(&k.key, &false);
            false
        } else {
            true
        }
    })
}

fn next_client(clients: &Slab<Client>, mut idx: usize) -> usize {
    loop {
        idx = (idx + 1) % clients.capacity();
        if clients.contains(idx) {
            return idx;
        }
    }
}

async fn switch_client(clients: &mut Slab<Client>, monitor: &mut Monitor, current: usize, next: usize, pressed_keys: &Vec<KeyPressed>) -> usize {
    if current == next || !clients.get(next).is_some_and(|client| client.is_connected()) {
        return current;
    }
    let _ = send_keys(clients, current, pressed_keys, false).await;
    let _ = send_keys(clients, next, pressed_keys, true).await;
    if next == 0 {
        tracing::info!(idx = %next, "Switched to local");
    } else {
        tracing::info!(idx = %next, "Switched to remote client");
    }

    if let Some(client) = clients.get(next) {
        if let Ok(leds) = client.leds() {
            let _ = monitor.update_leds(leds).await;
        }
    }
    next
}

async fn init_client(idx: usize, addr: SocketAddr, stream: TcpStream, acceptor: TlsAcceptor, password: &str, mut init_updates: VecDeque<Update>, disconnect_tx: Sender<(usize,SocketAddr)>) -> Result<RemoteClient,Error> {
    let (mut stream,leds) = init_connection(stream, acceptor, password).await?;
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
    Ok(RemoteClient::new(idx, addr, stream, leds, disconnect_tx))
}

async fn init_connection(stream: TcpStream, acceptor: TlsAcceptor, password: &str) -> Result<(BufStream<TlsStream<TcpStream>>,LedState), Error> {
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

    let client_start = rkvm_net::timeout(rkvm_net::READ_TIMEOUT, ClientStart::decode(&mut stream)).await?;
    let status = match client_start.auth.verify(&challenge, password) {
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
    Ok((stream, client_start.leds))
}
