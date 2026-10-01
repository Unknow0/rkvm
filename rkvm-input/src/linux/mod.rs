pub mod writer;
pub mod monitor;
pub mod registry;

mod convert;
mod evdev;
mod glue;
mod uinput;
mod interceptor;

mod abs_convert;
mod button_convert;
mod keyboard_convert;
mod key_convert;
mod rel_convert;
mod sync_convert;

use rkvm_net::LedState;
use std::fs::File;
use std::os::fd::AsRawFd;

pub fn led_state() -> LedState {
    if let Ok(file) = File::open("/dev/console") {
        unsafe {
            let mut leds: u8 = 0;
            let fd = file.as_raw_fd();
            
            // ioctl(fd, KDGKLED, &leds)
            let ret = libc::ioctl(fd, 0x4b41, &mut leds);
            
            if ret == 0 {
                let leds = LedState {
                    num_lock: (leds & 0x02) != 0,     // bit 1
                    caps_lock: (leds & 0x04) != 0,    // bit 2
                    scroll_lock: (leds & 0x01) != 0,  // bit 0
                };
                tracing::info!("Found state: {:?}", leds);
                return leds;
            }
        }
    }
    LedState { num_lock: false, caps_lock: false, scroll_lock: false, }
}
