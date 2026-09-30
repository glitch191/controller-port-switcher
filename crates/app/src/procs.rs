//! Running processes, read only when the menu opens (nothing watches in the background).
//! Everything here works without administrator rights; protected processes (some
//! anti-cheat games) simply cannot be inspected.

use cps_core::XINPUT_DLLS;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, MODULEENTRY32W, Module32FirstW, Module32NextW, PROCESSENTRY32W, Process32FirstW,
    Process32NextW, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};

pub struct Process {
    pub pid: u32,
    pub exe: PathBuf,
}

fn wstr(buf: &[u16]) -> OsString {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    OsString::from_wide(&buf[..len])
}

fn image_path(pid: u32) -> Option<PathBuf> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len);
        CloseHandle(h);
        (ok != 0).then(|| PathBuf::from(OsString::from_wide(&buf[..len as usize])))
    }
}

/// Processes of this session whose executable path can be read, except this app.
pub fn running() -> Vec<Process> {
    let mut out = Vec::new();
    let me = unsafe { GetCurrentProcessId() };
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut more = Process32FirstW(snap, &mut e) != 0;
        while more {
            if e.th32ProcessID != 0
                && e.th32ProcessID != me
                && let Some(exe) = image_path(e.th32ProcessID)
            {
                out.push(Process {
                    pid: e.th32ProcessID,
                    exe,
                });
            }
            more = Process32NextW(snap, &mut e) != 0;
        }
        CloseHandle(snap);
    }
    out
}

/// Full paths of the modules loaded in a process, or None if it cannot be inspected.
pub fn modules(pid: u32) -> Option<Vec<PathBuf>> {
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid);
        if snap == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut out = Vec::new();
        let mut e: MODULEENTRY32W = std::mem::zeroed();
        e.dwSize = size_of::<MODULEENTRY32W>() as u32;
        let mut more = Module32FirstW(snap, &mut e) != 0;
        while more {
            out.push(PathBuf::from(wstr(&e.szExePath)));
            more = Module32NextW(snap, &mut e) != 0;
        }
        CloseHandle(snap);
        Some(out)
    }
}

/// XInput DLL names (lowercase, in XINPUT_DLLS order) among loaded modules.
pub fn xinput_modules(modules: &[PathBuf]) -> Vec<&'static str> {
    XINPUT_DLLS
        .iter()
        .copied()
        .filter(|d| {
            modules
                .iter()
                .any(|m| m.file_name().is_some_and(|f| f.eq_ignore_ascii_case(d)))
        })
        .collect()
}

pub fn same_path(a: &Path, b: &Path) -> bool {
    a.as_os_str().eq_ignore_ascii_case(b.as_os_str())
}

/// A running program that has XInput loaded, offered by "Add running game".
pub struct Candidate {
    pub exe: PathBuf,
    pub dll: &'static str,
}

/// Programs outside the Windows folder that have an XInput DLL loaded.
pub fn xinput_programs() -> Vec<Candidate> {
    let windir = std::env::var("WINDIR")
        .unwrap_or_else(|_| r"C:\Windows".into())
        .to_lowercase()
        + "\\";
    let mut out: Vec<Candidate> = Vec::new();
    for p in running() {
        let in_windows = p.exe.to_string_lossy().to_lowercase().starts_with(&windir);
        if in_windows || out.iter().any(|c| same_path(&c.exe, &p.exe)) {
            continue;
        }
        let Some(mods) = modules(p.pid) else { continue };
        if let Some(&dll) = xinput_modules(&mods).first() {
            out.push(Candidate { exe: p.exe, dll });
        }
    }
    out.sort_by_key(|c| c.exe.file_name().map(|f| f.to_ascii_lowercase()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sees_xinput_loaded_in_this_process() {
        let dir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
        let path = crate::win::wide(format!(r"{dir}\System32\xinput1_4.dll"));
        let m = unsafe { windows_sys::Win32::System::LibraryLoader::LoadLibraryW(path.as_ptr()) };
        assert!(!m.is_null());
        let me = unsafe { GetCurrentProcessId() };
        let mods = modules(me).expect("own modules");
        assert_eq!(xinput_modules(&mods), vec!["xinput1_4.dll"]);
        // The process list excludes this process but finds others (at least Explorer).
        assert!(running().iter().all(|p| p.pid != me));
        assert!(!running().is_empty());
    }
}
