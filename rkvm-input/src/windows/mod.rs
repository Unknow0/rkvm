pub mod writer;
pub mod monitor;
pub mod injector;
pub mod writer_simple;

mod normalizer;
mod key_repeater;

use rkvm_net::LedState;
use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardState;

pub fn led_state() -> LedState {
    unsafe {
        let mut keys = [0u8; 256];
        if GetKeyboardState(&mut keys).is_ok() {
            LedState {
                num_lock: (keys[0x90] & 0x01) != 0,
                caps_lock: (keys[0x14] & 0x01) != 0,
                scroll_lock: (keys[0x91] & 0x01) != 0,
            }
        } else {
            LedState { num_lock: true, caps_lock: false, scroll_lock: false, }
        }
    }
}
