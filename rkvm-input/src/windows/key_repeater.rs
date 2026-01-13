use tokio::task::JoinHandle;
use tokio::time::{Duration, Instant, interval_at,Interval};
use windows::Win32::UI::Input::KeyboardAndMouse::*;

use crate::key::Keyboard;

pub struct KeyRepeater {
    delay: Duration,
    period: Duration,

    key: Keyboard,
    task: Option<JoinHandle<()>>,

}

impl Drop for KeyRepeater {
    fn drop(&mut self) {
        self.stop();
    }
}

impl KeyRepeater {
    pub fn new(key: Keyboard, scan: u16,  flags: KEYBD_EVENT_FLAGS, delay: Duration, period: Duration) -> Self {
        let mut kr = Self {
            delay: delay,
            period: period,
            key: key,
            task: None,
        };

        kr.start(scan, flags);
        
        kr
    }

    pub fn key(&mut self, key: Keyboard, flags: KEYBD_EVENT_FLAGS, scan: u16, down: &bool) -> bool {
        if self.key == key {
            if !down {
                self.stop();
                return true;
            }
            return false;
        }

        if *down {
            self.key=key;
            self.start(scan, flags);
       }
       return false;
    }

    fn stop(&mut self) {
        match &self.task {
            None => return,
            Some(t) => {
                t.abort();
                self.task = None;
            }
        }
    }

    fn start(&mut self, scan: u16, flags: KEYBD_EVENT_FLAGS) {
        self.stop();
        
        let start = Instant::now() + self.delay;
        let interval = interval_at(start, self.period);
        self.task = Some(tokio::spawn(run(interval, scan, flags)));
    }
}

async fn run(mut interval: Interval, scan: u16, flags: KEYBD_EVENT_FLAGS) {
    loop {
        interval.tick().await;

        let input = INPUT {
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
        };
        unsafe {
            SendInput(&[input], size_of::<INPUT>() as i32);
        }
    }
}