//! Optional proxy log, enabled with `"log": true` in the per-game config. Written only
//! when the config is reloaded or the mapping changes, never on each call.

use core::fmt::Write;
use core::ptr::null_mut;
use cps_core::mapping::{self, NO_SLOT, Rule, SLOTS, SlotState};
use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE, SYSTEMTIME};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_APPEND_DATA, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, OPEN_ALWAYS, WriteFile,
};
use windows_sys::Win32::System::SystemInformation::GetLocalTime;

/// Fixed-size text buffer: formatting never allocates.
struct Line {
    buf: [u8; 512],
    len: usize,
}

impl Write for Line {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let n = s.len().min(self.buf.len() - self.len);
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}

fn line() -> Line {
    let mut l = Line { buf: [0; 512], len: 0 };
    let mut t: SYSTEMTIME = unsafe { core::mem::zeroed() };
    unsafe { GetLocalTime(&mut t) };
    let _ = write!(
        l,
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} ",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    );
    l
}

fn append(path: &[u16], mut l: Line) {
    let _ = l.write_str("\r\n");
    let h = unsafe {
        CreateFileW(path.as_ptr(), FILE_APPEND_DATA, FILE_SHARE_READ, null_mut(), OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, null_mut())
    };
    if h == INVALID_HANDLE_VALUE {
        return;
    }
    let mut written = 0;
    unsafe {
        WriteFile(h, l.buf.as_ptr(), l.len as u32, &mut written, null_mut());
        CloseHandle(h);
    }
}

pub fn config(path: &[u16], valid: bool, rules: &[Rule; SLOTS]) {
    let mut l = line();
    if !valid {
        let _ = l.write_str("config missing or invalid: passing calls through");
    } else {
        let _ = l.write_str("config loaded:");
        for (p, r) in rules.iter().enumerate() {
            let _ = match r {
                Rule::Auto => write!(l, " P{}=auto", p + 1),
                Rule::None => write!(l, " P{}=none", p + 1),
                Rule::Slot(s) => write!(l, " P{}=slot {s}", p + 1),
                Rule::Device { id, instance, .. } => write!(l, " P{}={id}#{instance}", p + 1),
            };
        }
    }
    append(path, l);
}

/// `slots` is None when no rule targets a device (slots were not queried).
pub fn map(path: &[u16], packed: u32, slots: Option<&[SlotState; SLOTS]>) {
    let mut l = line();
    let _ = l.write_str("mapping:");
    for (p, s) in mapping::unpack(packed).iter().enumerate() {
        let _ = if *s == NO_SLOT { write!(l, " P{}=none", p + 1) } else { write!(l, " P{}=slot {s}", p + 1) };
    }
    let Some(slots) = slots else { return append(path, l) };
    let _ = l.write_str(" | slots:");
    for (i, s) in slots.iter().enumerate() {
        let _ = match (s.connected, s.id) {
            (true, Some(id)) => write!(l, " {i}={id}"),
            (true, None) => write!(l, " {i}=connected"),
            (false, _) => write!(l, " {i}=empty"),
        };
    }
    append(path, l);
}
