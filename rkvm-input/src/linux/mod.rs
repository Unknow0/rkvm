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
use std::collections::BTreeMap;
use std::fs;
use std::io::{Error, Result, ErrorKind};
use std::path::Path;

pub fn led_state() -> LedState {
    read_sys_leds().unwrap_or_default()
}

#[derive(Debug, Default, Clone, Copy)]
struct SysfsLockState {
    pub num_lock: Option<bool>,
    pub caps_lock: Option<bool>,
    pub scroll_lock: Option<bool>,
}
impl SysfsLockState {
    fn score(&self) -> u8 {
        self.num_lock.is_some() as u8
            + self.caps_lock.is_some() as u8
            + self.scroll_lock.is_some() as u8
    }
}

fn read_sys_leds() -> Result<LedState> {
    let mut devices: BTreeMap<String, SysfsLockState> = BTreeMap::new();
    for entry in fs::read_dir("/sys/class/leds")? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();

        let Some((device, led)) = name.split_once("::") else {
            continue;
        };

        let field = match led {
            "numlock" => &mut devices.entry(device.to_owned()).or_default().num_lock,
            "capslock" => &mut devices.entry(device.to_owned()).or_default().caps_lock,
            "scrolllock" => &mut devices.entry(device.to_owned()).or_default().scroll_lock,
            _ => continue,
        };

        let brightness = entry.path().join("brightness");

        if let Ok(value) = read_led_brightness(&brightness) {
            *field = Some(value);
        }
    }

    Ok(devices.into_values().max_by_key(SysfsLockState::score).map_or_default(|l| LedState{num_lock: l.num_lock.unwrap_or(false), caps_lock: l.caps_lock.unwrap_or(false), scroll_lock: l.scroll_lock.unwrap_or(false)}))
}

fn read_led_brightness(path: &Path) -> Result<bool> {
    let value: u32 = fs::read_to_string(path)?
        .trim()
        .parse()
        .map_err(|e| Error::new(ErrorKind::InvalidData, e))?;

    Ok(value != 0)
}
