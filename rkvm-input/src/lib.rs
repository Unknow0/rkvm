pub mod device;
pub mod monitor;
pub mod writer;

#[cfg(target_os = "windows")]
pub mod windows;
#[cfg(target_os = "linux")]
pub mod linux;

