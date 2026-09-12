use crate::device::DeviceSpec;
use crate::monitor::MonitorPlatform;
use rkvm_net::Update;

use std::io::{Error, ErrorKind};
use tokio::sync::mpsc::{self, Receiver};

pub struct MonitorWindows {
    receiver: Receiver<Result<Update, Error>>,
}

impl MonitorPlatform for MonitorWindows {
    fn new(_device_allowlist: Vec<DeviceSpec>) -> Self {
        let (_sender, receiver) = mpsc::channel(1);
        // TODO: Implement Windows device monitoring
        // tokio::spawn(monitor(sender));

        Self { receiver }
    }

    async fn read(&mut self) -> Result<Update, Error> {
        self.receiver
            .recv()
            .await
            .ok_or_else(|| Error::new(ErrorKind::BrokenPipe, "Monitor task exited"))?
    }
}
