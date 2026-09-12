use rkvm_net::Update;

use std::collections::HashMap;
use std::io;
use tokio::sync::mpsc::Sender;

use crate::server::Error;

#[cfg(target_os = "linux")]
use rkvm_input::linux::writer::WriterLinux;
#[cfg(target_os = "windows")]
use rkvm_input::windows::writer::WriterWindows;
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
compile_error!("Unsupported OS");

pub struct LocalClient {
    #[cfg(target_os = "linux")]
    writer: WriterLinux,
    #[cfg(target_os = "windows")]
    writer: WriterWindows,
}

impl LocalClient {
    pub fn new(
        #[cfg(target_os = "linux")]
        writer: WriterLinux,
        #[cfg(target_os = "windows")]
        writer: WriterWindows,
    ) -> Self {
        LocalClient { writer }
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
            } => {
                self.writer
                    .create_device(id, name, vendor, product, version, rel.clone(), abs.clone(), keys.clone(), delay, period)
                    .await
                    .map_err(|e| Error::Network(e))
            }
            Update::Event { id, event } => {
                self.writer
                    .event(id, event)
                    .await
                    .map_err(|e| Error::Network(e))
            }
            Update::DestroyDevice { id } => {
                self.writer
                    .destroy_device(id)
                    .await
                    .map_err(|e| Error::Network(e))
            }
            _ => Ok(()),
        }
    }
}

pub enum Client {
    Local(LocalClient),
    Static(Option<Sender<Update>>),
    Remote(Sender<Update>),
}

impl Client {
    pub fn is_connected(&self) -> bool {
        match self {
            Client::Local(_) => true,
            Client::Static(opt) => opt.is_some(),
            Client::Remote(_) => true,
        }
    }

    pub async fn send(&mut self, update: Update) -> Result<(), Error> {
        match self {
            Client::Local(local) => local.send(update).await,
            Client::Static(opt) => match opt {
                Some(sender) => sender
                    .send(update)
                    .await
                    .map_err(|_| Error::Network(io::Error::new(io::ErrorKind::BrokenPipe, "Client disconnected"))),
                None => Ok(()),
            },
            Client::Remote(sender) => sender
                .send(update)
                .await
                .map_err(|_| Error::Network(io::Error::new(io::ErrorKind::BrokenPipe, "Client disconnected"))),
        }
    }
}
