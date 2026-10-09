use crate::device::DeviceSpec;
use crate::monitor::MonitorPlatform;
use crate::linux::interceptor::{Interceptor, OpenError};
use crate::linux::registry::Registry;
use rkvm_net::{LedState, Update};

use futures::StreamExt;
use inotify::{Inotify, WatchMask};
use std::ffi::OsStr;
use std::io::{Error, ErrorKind};
use std::path::Path;
use tokio::fs;
use tokio::sync::mpsc;
use tokio::sync::broadcast;

const EVENT_PATH: &str = "/dev/input";

pub struct MonitorLinux {
    receiver: mpsc::Receiver<Result<Update, Error>>,
    registry: Registry,
    leds: broadcast::Sender<LedState>,
}

impl MonitorLinux {
    pub fn registry(&self) -> Registry {
        self.registry.clone()
    }
}

impl MonitorPlatform for MonitorLinux {
    fn new(device_allowlist: Vec<DeviceSpec>) -> Self {
        let (sender, receiver) = mpsc::channel(4);
        let (leds, _) = broadcast::channel(4);
        let registry = Registry::new();
        tokio::spawn(monitor(sender, device_allowlist, registry.clone(), leds.clone()));

        Self { receiver, registry, leds }
    }

    async fn read(&mut self) -> Result<Update, Error> {
        self.receiver
            .recv()
            .await
            .ok_or_else(|| Error::new(ErrorKind::BrokenPipe, "Monitor task exited"))?
    }

    async fn update_leds(&mut self, leds: LedState) -> Result<(), Error> {
        self.leds.send(leds).map_err(|_| Error::new(ErrorKind::BrokenPipe, "Failed to update leds"))?;
        Ok(())
    }
}

async fn monitor(sender: mpsc::Sender<Result<Update, Error>>, device_allowlist: Vec<DeviceSpec>, registry: Registry, leds: broadcast::Sender<LedState>) {
    let run = async {
        let mut next_id = 0usize;

        let mut read_dir = fs::read_dir(EVENT_PATH).await?;

        let inotify = Inotify::init()?;
        inotify.watches().add(EVENT_PATH, WatchMask::CREATE)?;

        let mut stream = inotify.into_event_stream([0; 512])?;

        loop {
            let path = match read_dir.next_entry().await? {
                Some(entry) => entry.path(),
                None => match stream.next().await {
                    Some(event) => {
                        let event = event?;
                        let name = match event.name {
                            Some(name) => name,
                            None => continue,
                        };
                        Path::new(EVENT_PATH).join(&name)
                    }
                    None => break,
                },
            };

            if !path
                .file_name()
                .and_then(OsStr::to_str)
                .map_or(false, |name| name.starts_with("event"))
            {
                tracing::debug!("Skipping non event file {:?}", path);
                continue;
            }

            let interceptor = match Interceptor::open(&path, &registry, &device_allowlist).await {
                Ok(interceptor) => interceptor,
                Err(OpenError::Io(err)) => return Err(err),
                Err(OpenError::NotAppliable) => continue,
                Err(OpenError::NotMatchingAllowlist) => continue,
            };

            let id = next_id;
            next_id += 1;

            // Extract device metadata
            let name = interceptor.name().to_owned();
            let vendor = interceptor.vendor();
            let product = interceptor.product();
            let version = interceptor.version();
            let rel = interceptor.rel();
            let abs = interceptor.abs();
            let keys = interceptor.key();
            let repeat = interceptor.repeat();

            // Send CreateDevice update
            let create_update = Update::CreateDevice {
                id,
                name,
                vendor,
                product,
                version,
                rel,
                abs,
                keys,
                delay: repeat.delay,
                period: repeat.period,
            };

            if sender.send(Ok(create_update)).await.is_err() {
                return Ok(());
            }
            // Spawn task to read events from this device
            let sender = sender.clone();
            tokio::spawn(handle_events(id, interceptor, sender, leds.subscribe()));
        }

        Ok(())
    };

    tokio::select! {
        result = run => match result {
            Ok(_) => {},
            Err(err) => {
                let _ = sender.send(Err(err)).await;
            }
        },
        _ = sender.closed() => {}
    }
}

async fn handle_events(id: usize, mut interceptor: Interceptor, sender: mpsc::Sender<Result<Update, Error>>, mut leds: broadcast::Receiver<LedState>) {
    loop {
        tokio::select! {
            result = interceptor.read() => {
                match result {
                    Ok(event) => {
                        let update = Update::Event { id, event };
                        if sender.send(Ok(update)).await.is_err() {
                            break;
                        }
                    }
                    Err(err) => {
                        tracing::info!("Failed to get event for device {}: {:?}", id, err);
                        break;
                    }
                }
            }
            result = leds.recv() => {
                match result {
                    Ok(led_state) => {
                        if interceptor.supports_led() {
                            let _ = interceptor.write_led(&led_state).await;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
        }
    }

    // Send DestroyDevice when disconnected
    let _ = sender.send(Ok(Update::DestroyDevice { id })).await;
}
