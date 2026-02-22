use crate::config::Config;
use crate::stream::{RkvmStream, RkvmWriter};

use rkvm_input::writer::{Writer,WriterPlatform,WriterBuilderPlatform};
use rkvm_net::auth::{AuthChallenge, AuthStatus};
use rkvm_net::message::Message;
use rkvm_net::version::Version;
use rkvm_net::Update;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{self, stdout, BufWriter};
use std::path::Path;
use std::time::Instant;
use thiserror::Error;
use tokio::fs;
use tokio::io::{AsyncRead, AsyncWriteExt, BufStream};
use tokio::net::TcpStream;
use tokio::time;
use tokio_rustls::rustls::{self, ServerName};
use tokio_rustls::TlsConnector;
use tracing_subscriber::{fmt,Registry,EnvFilter};
use tracing_subscriber::prelude::*;
#[cfg(target_os="windows")]
use windows::core;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Network error: {0}")]
    Network(io::Error),
    #[error("Io error: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Rustls(#[from] rustls::Error),
    #[error("Toml error: {0}")]
    Toml(#[from] toml::de::Error),
    #[cfg(target_os="windows")]
    #[error("Windows API error: {0}")]
    Windows(#[from] core::Error),
    #[error("Incompatible server version (got {server}, expected {client})")]
    Version { server: Version, client: Version },
    #[error("Invalid password")]
    Auth,
}

pub fn init_tracing<P: AsRef<Path>>(log_level: &String, log_file: &Option<P>) {
    let filter = EnvFilter::new(log_level);
    if let Some(path) = log_file {
        let file = OpenOptions::new().create(true).append(true).open(path).unwrap();
        let fmt_layer = fmt::layer().with_ansi(false).with_writer(move || BufWriter::new(file.try_clone().unwrap())).without_time();
        let registry = Registry::default().with(filter).with(fmt_layer);
        tracing::subscriber::set_global_default(registry).unwrap();
    } else {
        let fmt_layer = fmt::layer().with_writer(stdout).without_time();
        let registry = Registry::default().with(filter).with(fmt_layer);
        tracing::subscriber::set_global_default(registry).unwrap();
    }
}

pub async fn init_config<P: AsRef<Path> + ?Sized> (path: &P) -> Result<Config,Error> {
    let config = fs::read_to_string(path).await?;
    let config = toml::from_str::<Config>(&config)?;
    return Ok(config);
    }

pub async fn init_stream(hostname: &ServerName, port: u16, connector: &TlsConnector, password: &str) -> Result<RkvmStream,Error> {
    // Intentionally don't impose any timeout for TCP connect.
    let stream = match hostname {
        ServerName::DnsName(name) => TcpStream::connect(&(name.as_ref(), port)).await,
        ServerName::IpAddress(address) => TcpStream::connect(&(*address, port)).await,
        _ => unimplemented!("Unhandled rustls ServerName variant: {:?}", hostname),
    }
    .map_err(Error::Network)?;

    tracing::info!("Connected to server");

    let stream = rkvm_net::timeout(
        rkvm_net::TLS_TIMEOUT,
        connector.connect(hostname.clone(), stream),
    )
    .await
    .map_err(Error::Network)?;

    tracing::info!("TLS connected");

    let mut stream = BufStream::with_capacity(1024, 1024, stream);

    rkvm_net::timeout(rkvm_net::WRITE_TIMEOUT, async {
        Version::CURRENT.encode(&mut stream).await?;
        stream.flush().await?;

        Ok(())
    })
    .await
    .map_err(Error::Network)?;

    let version = rkvm_net::timeout(rkvm_net::READ_TIMEOUT, Version::decode(&mut stream))
        .await
        .map_err(Error::Network)?;

    if version != Version::CURRENT {
        return Err(Error::Version {
            server: Version::CURRENT,
            client: version,
        });
    }

    let challenge = rkvm_net::timeout(rkvm_net::READ_TIMEOUT, AuthChallenge::decode(&mut stream))
        .await
        .map_err(Error::Network)?;

    let response = challenge.respond(password);

    rkvm_net::timeout(rkvm_net::WRITE_TIMEOUT, async {
        response.encode(&mut stream).await?;
        stream.flush().await?;

        Ok(())
    })
    .await
    .map_err(Error::Network)?;

    let status = rkvm_net::timeout(rkvm_net::READ_TIMEOUT, AuthStatus::decode(&mut stream))
        .await
        .map_err(Error::Network)?;

    match status {
        AuthStatus::Passed => {}
        AuthStatus::Failed => return Err(Error::Auth),
    }

    tracing::info!("Authenticated successfully");
    Ok(RkvmStream::Tcp(stream))
}

pub async fn run<R,W,H>(mut reader: R, mut writer: W, mut handler: H) -> Result<(), Error> 
    where
        R: AsyncRead + Send + Unpin,
        W: RkvmWriter + Send,
        H: AsyncFnMut(Update) -> Result<(), Error>, {

    let mut start = Instant::now();

    let mut interval = time::interval(rkvm_net::PING_INTERVAL + rkvm_net::READ_TIMEOUT);

    // Interval ticks immediately after creation.
    interval.tick().await;

    loop {
        let update = tokio::select! {
            update = Update::decode(&mut reader) => update.map_err(Error::Network)?,
            _ = interval.tick() => return Err(Error::Network(io::Error::new(io::ErrorKind::TimedOut, "Ping timed out"))),
        };

        tracing::debug!("received {:?}", update);
        interval.reset();

        if let Update::Ping = &update {
            let duration = start.elapsed();
            tracing::debug!(duration = ?duration, "Received ping");

            start = Instant::now();

            writer.send(Update::Pong).await?;

            let duration = start.elapsed();
            tracing::debug!(duration = ?duration, "Sent pong");
        }
        handler(update).await?
    }
}

pub async fn handler(writers: &mut HashMap<usize,Writer>, update: Update) -> Result<(), Error> {
    match update {
        Update::CreateDevice {
            id,
            name,
            vendor,
            product,
            version,
            rel,
            abs,
            keys,
            delay,
            period,
        } => {
            let entry = writers.entry(id);
            if let Entry::Occupied(_) = entry {
                return Err(Error::Network(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Server created the same device twice",
                )));
            }

            let writer = async {
                Writer::builder()?
                    .name(&name)
                    .vendor(vendor)
                    .product(product)
                    .version(version)
                    .rel(rel)?
                    .abs(abs)?
                    .key(keys)?
                    .delay(delay)?
                    .period(period)?
                    .build()
                    .await
            }
            .await?;

            entry.or_insert(writer);

            tracing::info!(
                id = %id,
                name = ?name,
                vendor = %vendor,
                product = %product,
                version = %version,
                "Created new device"
            );
        }
        Update::DestroyDevice { id } => {
            if writers.remove(&id).is_none() {
                return Err(Error::Network(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Server destroyed a nonexistent device",
                )));
            }

            tracing::info!(id = %id, "Destroyed device");
        }
        Update::Event { id, event } => {
            let writer = writers.get_mut(&id).ok_or_else(|| {
                Error::Network(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Server sent an event to a nonexistent device",
                ))
            })?;

            writer.write(&event).await?;

            tracing::trace!(id = %id, "Wrote an event to device");
        }
        Update::Ping => {}
        Update::Pong => {}
    }
    Ok(())
}
