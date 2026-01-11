use crate::writer::{WriterPlatform,WriterBuilderPlatform};
use crate::abs::{AbsAxis, AbsInfo};
use crate::event::Event;
use crate::key::{Key, KeyEvent,Keyboard};
use crate::rel::{RelAxis, RelEvent};

use std::ffi::CString;
use std::io::Error;

use windows::Win32::UI::Input::KeyboardAndMouse::*;

pub struct WriterWindows {
     buffer: Vec<INPUT>,
}

impl WriterWindows {
     fn push(&mut self, input: INPUT) {
        self.buffer.push(input);
    }

    fn flush(&mut self) -> Result<(), Error> {
        if self.buffer.is_empty() {
            return Ok(());
        }

        unsafe {
            let sent = SendInput(
                self.buffer.as_slice(),
                self.buffer.len() as i32,
            );

            if sent == 0 {
                return Err(Error::last_os_error());
            }
        }

        self.buffer.clear();
        Ok(())
    }
    
    fn key(&mut self, key: &Keyboard, down:&bool) {
        if let Some((scan, extended)) = map_key_to_scancode(key) {
            let mut flags = KEYEVENTF_SCANCODE;
            if !down { flags |= KEYEVENTF_KEYUP; }
            if extended { flags |= KEYEVENTF_EXTENDEDKEY; }

            self.push(INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY::default(),
                        wScan: scan,
                        dwFlags: flags,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            });
        }
    }

    fn mouse_move(&mut self, dx: i32, dy: i32) {
         self.push(INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx,
                    dy,
                    mouseData: 0,
                    dwFlags: MOUSEEVENTF_MOVE,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        })
    }

    fn mouse_wheel(&mut self, delta: i32) {
         self.push(INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: (delta * 120) as u32,
                    dwFlags: MOUSEEVENTF_WHEEL,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        })
    }
}

impl WriterPlatform for WriterWindows {
    type Builder = WriterWindowsBuilder;
     fn builder() -> Result<Self::Builder, Error> {
        Ok(WriterWindowsBuilder)
    }

    async fn write(&mut self, event: &Event) -> Result<(), Error> {
      match event {
            Event::Key(KeyEvent { key, down }) => {
                match key {
                    Key::Key(key) => self.key(key, down),
                    Key::Button(button) => {
                    }
                }
               
            }
            Event::Rel(RelEvent { axis, value }) => {
                match axis {
                    RelAxis::X => self.mouse_move(*value, 0),
                    RelAxis::Y => self.mouse_move(0, *value),
                    RelAxis::Wheel => self.mouse_wheel(*value),
                    _ => tracing::error!("Axe not handled: {:?}", axis),
                }
            }
            Event::Abs(_event) => {}
            Event::Sync(_) => self.flush()?
        }

        Ok(())
    }

    
}

pub struct WriterWindowsBuilder;

impl WriterBuilderPlatform for WriterWindowsBuilder {
    type Writer = WriterWindows;

    fn name(self, _name: &CString) -> Self {
        self
    }

    fn vendor(self, _value: u16) -> Self {
        self
    }

    fn product(self, _value: u16) -> Self {
        self
    }

    fn version(self, _value: u16) -> Self {
        self
    }
    fn rel<T: IntoIterator<Item = RelAxis>>(self, _items: T) -> Result<Self, Error> {
        Ok(self)
    }
    fn abs<T: IntoIterator<Item = (AbsAxis, AbsInfo)>>(self, _items: T) -> Result<Self, Error> {
        Ok(self)
    }
    fn key<T: IntoIterator<Item = Key>>(self, _items: T) -> Result<Self, Error> {
        Ok(self)
    }

    fn delay(self, _value: Option<i32>) -> Result<Self, Error> {
        Ok(self)
    }

    fn period(self, _value: Option<i32>) -> Result<Self, Error> {
        Ok(self)
    }

    async fn build(self) -> Result<Self::Writer, Error> {
        Ok(WriterWindows{
            buffer: Vec::with_capacity(16),
        })
    }
}

const fn map_key_to_scancode(key: &Keyboard) -> Option<(u16, bool)> {
    match key {
        // Letters
        Keyboard::A => Some((0x1E, false)),
        Keyboard::B => Some((0x30, false)),
        Keyboard::C => Some((0x2E, false)),
        Keyboard::D => Some((0x20, false)),
        Keyboard::E => Some((0x12, false)),
        Keyboard::F => Some((0x21, false)),
        Keyboard::G => Some((0x22, false)),
        Keyboard::H => Some((0x23, false)),
        Keyboard::I => Some((0x17, false)),
        Keyboard::J => Some((0x24, false)),
        Keyboard::K => Some((0x25, false)),
        Keyboard::L => Some((0x26, false)),
        Keyboard::M => Some((0x32, false)),
        Keyboard::N => Some((0x31, false)),
        Keyboard::O => Some((0x18, false)),
        Keyboard::P => Some((0x19, false)),
        Keyboard::Q => Some((0x10, false)),
        Keyboard::R => Some((0x13, false)),
        Keyboard::S => Some((0x1F, false)),
        Keyboard::T => Some((0x14, false)),
        Keyboard::U => Some((0x16, false)),
        Keyboard::V => Some((0x2F, false)),
        Keyboard::W => Some((0x11, false)),
        Keyboard::X => Some((0x2D, false)),
        Keyboard::Y => Some((0x15, false)),
        Keyboard::Z => Some((0x2C, false)),

        // Numbers
        Keyboard::N1 => Some((0x02, false)),
        Keyboard::N2 => Some((0x03, false)),
        Keyboard::N3 => Some((0x04, false)),
        Keyboard::N4 => Some((0x05, false)),
        Keyboard::N5 => Some((0x06, false)),
        Keyboard::N6 => Some((0x07, false)),
        Keyboard::N7 => Some((0x08, false)),
        Keyboard::N8 => Some((0x09, false)),
        Keyboard::N9 => Some((0x0A, false)),
        Keyboard::N0 => Some((0x0B, false)),

        // Arrows
        Keyboard::Up => Some((0x48, true)),
        Keyboard::Down => Some((0x50, true)),
        Keyboard::Left => Some((0x4B, true)),
        Keyboard::Right => Some((0x4D, true)),

        // Functions
        Keyboard::F1 => Some((0x3B, false)),
        Keyboard::F2 => Some((0x3C, false)),
        Keyboard::F3 => Some((0x3D, false)),
        Keyboard::F4 => Some((0x3E, false)),
        Keyboard::F5 => Some((0x3F, false)),
        Keyboard::F6 => Some((0x40, false)),
        Keyboard::F7 => Some((0x41, false)),
        Keyboard::F8 => Some((0x42, false)),
        Keyboard::F9 => Some((0x43, false)),
        Keyboard::F10 => Some((0x44, false)),
        Keyboard::F11 => Some((0x57, false)),
        Keyboard::F12 => Some((0x58, false)),

        // Special Keyboards
        Keyboard::Enter => Some((0x1C, false)),
        Keyboard::Esc => Some((0x01, false)),
        Keyboard::Backspace => Some((0x0E, false)),
        Keyboard::Tab => Some((0x0F, false)),
        Keyboard::Space => Some((0x39, false)),
        Keyboard::CapsLock => Some((0x3A, false)),
        Keyboard::LeftShift => Some((0x2A, false)),
        Keyboard::RightShift => Some((0x36, false)),
        Keyboard::LeftCtrl => Some((0x1D, false)),
        Keyboard::RightCtrl => Some((0x1D, true)),
        Keyboard::LeftAlt => Some((0x38, false)),
        Keyboard::RightAlt => Some((0x38, true)),
        Keyboard::LeftMeta => Some((0x5B, true)), // Windows Keyboard
        Keyboard::RightMeta => Some((0x5C, true)),

        Keyboard::Insert => Some((0x52, true)),
        Keyboard::Delete => Some((0x53, true)),
        Keyboard::Home => Some((0x47, true)),
        Keyboard::End => Some((0x4F, true)),
        Keyboard::PageUp => Some((0x49, true)),
        Keyboard::PageDown => Some((0x51, true)),

        _ => None, // ingore unsupported keys
    }
}