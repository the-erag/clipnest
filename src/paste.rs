//! Sends the paste keystroke to the focused app after an item is picked.
//!
//! Wayland does not let one client type into another, so on Wayland Clipnest
//! creates a virtual keyboard through /dev/uinput (the kernel-level route that
//! ydotool uses). That device is root-only by default;
//! scripts/enable-autopaste.sh installs a udev rule that grants the logged-in
//! user access. On X11 sessions xdotool is used when installed.

use std::ffi::CString;
use std::time::{Duration, Instant};

// linux/input-event-codes.h
const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const SYN_REPORT: u16 = 0;
const KEY_LEFTCTRL: u16 = 29;
const KEY_LEFTSHIFT: u16 = 42;
const KEY_V: u16 = 47;
const KEY_INSERT: u16 = 110;

// linux/uinput.h
const UI_SET_EVBIT: u64 = 0x4004_5564;
const UI_SET_KEYBIT: u64 = 0x4004_5565;
const UI_DEV_SETUP: u64 = 0x405C_5503;
const UI_DEV_CREATE: u64 = 0x5501;
const UI_DEV_DESTROY: u64 = 0x5502;
const BUS_VIRTUAL: u16 = 0x06;
const UINPUT: &str = "/dev/uinput";

fn combo(keys: &str) -> &'static [u16] {
    match keys {
        "shift+insert" => &[KEY_LEFTSHIFT, KEY_INSERT],
        "ctrl+shift+v" => &[KEY_LEFTCTRL, KEY_LEFTSHIFT, KEY_V],
        _ => &[KEY_LEFTCTRL, KEY_V],
    }
}

fn xdotool_keys(keys: &str) -> &'static str {
    match keys {
        "shift+insert" => "shift+Insert",
        "ctrl+shift+v" => "ctrl+shift+v",
        _ => "ctrl+v",
    }
}

pub struct VirtualKeyboard {
    fd: libc::c_int,
}

impl VirtualKeyboard {
    pub fn new() -> std::io::Result<VirtualKeyboard> {
        let path = CString::new(UINPUT).unwrap();
        let fd = unsafe { libc::open(path.as_ptr(), libc::O_WRONLY | libc::O_NONBLOCK) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let keyboard = VirtualKeyboard { fd };
        let check = |r: libc::c_int| {
            if r < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        };
        unsafe {
            check(libc::ioctl(fd, UI_SET_EVBIT as _, EV_KEY as libc::c_int))?;
            check(libc::ioctl(fd, UI_SET_EVBIT as _, EV_SYN as libc::c_int))?;
            for code in [KEY_LEFTCTRL, KEY_LEFTSHIFT, KEY_V, KEY_INSERT] {
                check(libc::ioctl(fd, UI_SET_KEYBIT as _, code as libc::c_int))?;
            }
            // struct uinput_setup { struct input_id id; char name[80]; __u32 ff_effects_max; }
            let mut setup = [0u8; 92];
            for (i, value) in [BUS_VIRTUAL, 0x1209, 0xC11B, 1].iter().enumerate() {
                setup[i * 2..i * 2 + 2].copy_from_slice(&value.to_ne_bytes());
            }
            let name = b"Clipnest virtual keyboard";
            setup[8..8 + name.len()].copy_from_slice(name);
            check(libc::ioctl(fd, UI_DEV_SETUP as _, setup.as_ptr()))?;
            check(libc::ioctl(fd, UI_DEV_CREATE as _))?;
        }
        Ok(keyboard)
    }

    fn emit(&self, kind: u16, code: u16, value: i32) {
        // struct input_event { struct timeval time; __u16 type; __u16 code; __s32 value; }
        let mut event = [0u8; 24];
        event[16..18].copy_from_slice(&kind.to_ne_bytes());
        event[18..20].copy_from_slice(&code.to_ne_bytes());
        event[20..24].copy_from_slice(&value.to_ne_bytes());
        unsafe {
            libc::write(self.fd, event.as_ptr() as *const libc::c_void, event.len());
        }
    }

    fn key(&self, code: u16, value: i32) {
        self.emit(EV_KEY, code, value);
        self.emit(EV_SYN, SYN_REPORT, 0);
        std::thread::sleep(Duration::from_millis(8));
    }

    pub fn tap(&self, codes: &[u16]) {
        for &code in codes {
            self.key(code, 1);
        }
        for &code in codes.iter().rev() {
            self.key(code, 0);
        }
    }
}

impl Drop for VirtualKeyboard {
    fn drop(&mut self) {
        unsafe {
            libc::ioctl(self.fd, UI_DEV_DESTROY as _);
            libc::close(self.fd);
        }
    }
}

enum Method {
    Uinput(VirtualKeyboard),
    Xdotool,
}

pub struct Paster {
    method: Option<Method>,
    error: String,
    ready_at: Instant,
}

impl Paster {
    pub fn new() -> Paster {
        let mut paster = Paster {
            method: None,
            error: String::new(),
            ready_at: Instant::now(),
        };
        paster.detect();
        paster
    }

    fn detect(&mut self) {
        let x11 = std::env::var("XDG_SESSION_TYPE").map(|t| t == "x11").unwrap_or(false);
        if x11 && glib::find_program_in_path("xdotool").is_some() {
            self.method = Some(Method::Xdotool);
            return;
        }
        let path = CString::new(UINPUT).unwrap();
        if unsafe { libc::access(path.as_ptr(), libc::W_OK) } != 0 {
            self.error = format!("no write access to {UINPUT}");
            return;
        }
        match VirtualKeyboard::new() {
            Ok(keyboard) => {
                // The compositor needs a moment to pick up a new device.
                self.ready_at = Instant::now() + Duration::from_millis(500);
                self.method = Some(Method::Uinput(keyboard));
            }
            Err(err) => self.error = err.to_string(),
        }
    }

    /// Re-checks, so enabling access (scripts/enable-autopaste.sh) needs no restart.
    pub fn available(&mut self) -> bool {
        if self.method.is_none() {
            self.detect();
        }
        self.method.is_some()
    }

    pub fn describe(&self) -> String {
        match self.method {
            Some(Method::Uinput(_)) => "Ready (virtual keyboard via /dev/uinput)".into(),
            Some(Method::Xdotool) => "Ready (xdotool)".into(),
            None => format!(
                "Not set up: {}. Run enable-autopaste.sh once (see the README).",
                self.error
            ),
        }
    }

    /// How long to wait before pasting, given the requested delay.
    pub fn wait_for(&self, delay_ms: i64) -> Duration {
        let ready = self.ready_at.saturating_duration_since(Instant::now());
        Duration::from_millis(delay_ms.max(0) as u64).max(ready)
    }

    pub fn paste(&self, keys: &str) {
        match &self.method {
            Some(Method::Uinput(keyboard)) => keyboard.tap(combo(keys)),
            Some(Method::Xdotool) => {
                let spawned = std::process::Command::new("xdotool")
                    .args(["key", "--clearmodifiers", xdotool_keys(keys)])
                    .spawn();
                if let Err(err) = spawned {
                    eprintln!("clipnest: auto-paste failed: {err}");
                }
            }
            None => {}
        }
    }
}
