//! "Start with Windows": a value under HKCU\...\Run (per user, no administrator
//! rights). If the user disables the entry in Task Manager, Windows records that
//! separately (StartupApproved) and keeps it disabled even when the value is rewritten.

use crate::win::wide;
use cps_core::PROJECT;
use std::ptr::null_mut;
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

fn command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\"", exe.display()))
}

/// Current value of the Run entry, if any.
fn current() -> Option<String> {
    let key = wide(RUN_KEY);
    let name = wide(PROJECT);
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            buf.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if r != 0 {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

pub fn is_enabled() -> bool {
    current().is_some()
}

/// Adds the Run entry, or updates it when the executable moved.
pub fn enable() -> Result<(), String> {
    let cmd = command().ok_or("Cannot find the path of this program.")?;
    if current().as_deref() == Some(cmd.as_str()) {
        return Ok(());
    }
    let data = wide(&cmd);
    let r = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            wide(RUN_KEY).as_ptr(),
            wide(PROJECT).as_ptr(),
            REG_SZ,
            data.as_ptr().cast(),
            (data.len() * 2) as u32,
        )
    };
    if r == 0 {
        Ok(())
    } else {
        Err(format!("Cannot turn on Start with Windows (error {r})."))
    }
}

pub fn disable() -> Result<(), String> {
    let r = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), wide(PROJECT).as_ptr()) };
    // 2 = ERROR_FILE_NOT_FOUND: already off.
    if r == 0 || r == 2 {
        Ok(())
    } else {
        Err(format!("Cannot turn off Start with Windows (error {r})."))
    }
}
