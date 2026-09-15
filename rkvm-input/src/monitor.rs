use crate::device::DeviceSpec;
use rkvm_net::Update;
use std::io::Error;

#[cfg(target_os = "windows")]
pub use {crate::windows::monitor::MonitorWindows as Monitor};
#[cfg(target_os = "linux")]
pub use {crate::linux::monitor::MonitorLinux as Monitor};

pub trait MonitorPlatform: Sized {
    fn new(device_allowlist: Vec<DeviceSpec>) -> Self;

    fn read<'a>(&'a mut self) -> impl std::future::Future<Output = Result<Update, Error>> + Send + 'a;
}
