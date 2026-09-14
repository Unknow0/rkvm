use rkvm_net::Update;
use rkvm_net::message::Message;
use rkvm_input::writer::{DeviceWriter, EventWriter};

use std::future::Future;
use std::io;
use std::time:: Instant;
use tokio::io::{AsyncWriteExt, BufStream};
use tokio::time;
use tokio::net::TcpStream;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::task::JoinHandle;
use tokio_rustls::server::TlsStream;

use crate::server::Error;

#[cfg(target_os = "linux")]
use rkvm_input::linux::writer::WriterLinux;
#[cfg(target_os = "windows")]
use rkvm_input::windows::writer::WriterWindows;

pub struct LocalClient {
    #[cfg(target_os = "linux")]
    writer: WriterLinux,
    #[cfg(target_os = "windows")]
    writer: WriterWindows,
}

impl LocalClient {
    pub fn new() -> Self {
        #[cfg(target_os = "linux")]
        let writer= WriterLinux::new();
        #[cfg(target_os = "windows")]
        let writer = WriterWindows::new();
        LocalClient { writer }
    }

    #[cfg(target_os = "linux")]
    pub fn set_registry(&mut self, registry: rkvm_input::linux::registry::Registry) {
        self.writer.set_registry(registry);
    }
    
    pub async fn send(&mut self, update: Update) -> Result<(), Error> {
        match update {
            Update::CreateDevice {
                id,
                ref name,
                vendor,
                product,
                version,
                ref rel,
                ref abs,
                ref keys,
                delay,
                period,
            } => self.writer.create_device(id, name, vendor, product, version, rel.clone(), abs.clone(), keys.clone(), delay, period)
                    .await.map_err(Error::Io),
            Update::Event { id, event } => self.writer.event(id, event).await.map_err(Error::Io),
            Update::DestroyDevice { id } => self.writer.destroy_device(id).await.map_err(Error::Io),
            _ => Ok(()),
        }
    }
}

pub struct RemoteClient {
    sender: Sender<Update>,
    handle: JoinHandle<Result<(),Error>>,
}

impl RemoteClient {
    pub async fn new<F,Fut>(idx: usize, init: F) -> Self
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<BufStream<TlsStream<TcpStream>>, Error>> + Send + 'static {
        let (sender, receiver) = channel(16);
        let handle = tokio::spawn(async move {
            let co = init().await?;
            RemoteClient::run(receiver, co).await
        });
        RemoteClient { sender: sender, handle: handle}
    }

    pub async fn send(&self, update: Update) -> Result<(),Error> {
        self.sender.send(update).await.map_err(|_| Error::Io(io::Error::new(io::ErrorKind::BrokenPipe, "Client disconnected")))
    }

    async fn run(mut receiver: Receiver<Update>, mut stream: BufStream<TlsStream<TcpStream>>) -> Result<(), Error> {
        let mut interval = time::interval(rkvm_net::PING_INTERVAL);
        loop {
            let update = tokio::select! {
                // Make sure pings have priority.
                // The client could time out otherwise.
                biased;

                _ = interval.tick() => Some(Update::Ping),
                recv = receiver.recv() => recv,
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
            if matches!(update, Update::Stop) {
                break;
            }
        }

        Ok(())
    }
}
pub enum Client {
    Local(LocalClient),
    Empty,
    Remote(RemoteClient),
}

impl Client {
    pub fn is_connected(&self) -> bool {
        match self {
            Client::Local(_) => true,
            Client::Empty => false,
            Client::Remote(_) => true,
        }
    }

    pub async fn send(&mut self, update: Update) -> Result<(), Error> {
        match self {
            Client::Local(local) => local.send(update).await,
            Client::Empty => Err(Error::Io(io::Error::new(io::ErrorKind::BrokenPipe, "Client disconnected"))),
            Client::Remote(remote) => remote.send(update).await,
        }
    }
}
