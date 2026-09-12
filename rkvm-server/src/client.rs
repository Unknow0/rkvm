use rkvm_net::Update;

use std::io;
use tokio::sync::mpsc::Sender;

use crate::server::Error;

pub enum Client {
    Local,
    Static(Option<Sender<Update>>),
    Remote(Sender<Update>),
}

impl Client {
    pub fn is_connected(&self) -> bool {
        match self {
            Client::Static(opt) => opt.is_some(),
            _ => true
        }
    }
    pub async fn send(&self, update: Update) -> Result<(), Error> {
        match self {
            Client::Local => {
                // Local client doesn't use channel, events are handled internally
                Ok(())
            }
            Client::Static(opt) => {
                match opt {
                    Some(sender) => sender.send(update).await.map_err(|_| Error::Network(io::Error::new(io::ErrorKind::BrokenPipe, "Client disconnected"))),
                    None => Ok(())
                }
            }
            Client::Remote(sender) => sender.send(update).await.map_err(|_| Error::Network(io::Error::new(io::ErrorKind::BrokenPipe, "Client disconnected")))
        }
    }
}