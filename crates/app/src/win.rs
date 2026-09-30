//! Small wrappers over Win32: wide strings, native message boxes, the file dialog,
//! opening a folder and writing to the parent console.

use cps_core::PROJECT;
use std::ffi::OsStr;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, HWND, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_WRITE, OPEN_EXISTING, WriteFile};
use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE, WriteConsoleW};
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    IDNO, IDYES, MB_DEFBUTTON2, MB_ICONERROR, MB_ICONINFORMATION, MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, MB_YESNO,
    MB_YESNOCANCEL, MESSAGEBOX_STYLE, MessageBoxW, SW_SHOWNORMAL,
};

pub fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}

fn message(hwnd: HWND, text: &str, style: MESSAGEBOX_STYLE) -> i32 {
    unsafe { MessageBoxW(hwnd, wide(text).as_ptr(), wide(PROJECT).as_ptr(), style | MB_SETFOREGROUND) }
}

pub fn error(hwnd: HWND, text: &str) {
    message(hwnd, text, MB_OK | MB_ICONERROR);
}

pub fn info(hwnd: HWND, text: &str) {
    message(hwnd, text, MB_OK | MB_ICONINFORMATION);
}

/// Yes/No question, "No" by default.
pub fn confirm(hwnd: HWND, text: &str) -> bool {
    message(hwnd, text, MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2) == IDYES
}

/// Yes/No/Cancel question: Some(true) for Yes, Some(false) for No, None for Cancel.
pub fn ask(hwnd: HWND, text: &str) -> Option<bool> {
    match message(hwnd, text, MB_YESNOCANCEL | MB_ICONWARNING) {
        IDYES => Some(true),
        IDNO => Some(false),
        _ => None,
    }
}

/// Native "open file" dialog filtered on executables.
pub fn pick_exe(hwnd: HWND) -> Option<PathBuf> {
    let filter: Vec<u16> = "Programs (*.exe)\0*.exe\0All files\0*.*\0\0".encode_utf16().collect();
    let title = wide("Add game");
    let mut file = vec![0u16; 4096];
    let mut ofn: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    ofn.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = hwnd;
    ofn.lpstrFilter = filter.as_ptr();
    ofn.lpstrFile = file.as_mut_ptr();
    ofn.nMaxFile = file.len() as u32;
    ofn.lpstrTitle = title.as_ptr();
    ofn.Flags = OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
    if unsafe { GetOpenFileNameW(&mut ofn) } == 0 {
        return None;
    }
    let len = file.iter().position(|&c| c == 0).unwrap_or(0);
    Some(PathBuf::from(std::ffi::OsString::from_wide(&file[..len])))
}

pub fn open_folder(path: &Path) {
    unsafe { ShellExecuteW(null_mut(), wide("open").as_ptr(), wide(path).as_ptr(), null(), null(), SW_SHOWNORMAL) };
}

/// Writes to the console of the process that started us (the app is a GUI program,
/// so it has no console of its own), or to redirected output when there is one.
pub fn print_console(text: &str) {
    unsafe {
        let out = GetStdHandle(STD_OUTPUT_HANDLE);
        if !out.is_null() && out != INVALID_HANDLE_VALUE {
            let mut written = 0;
            let text = format!("{text}\r\n");
            WriteFile(out, text.as_ptr(), text.len() as u32, &mut written, null_mut());
            return;
        }
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return;
        }
        let con = CreateFileW(
            wide("CONOUT$").as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_WRITE,
            null(),
            OPEN_EXISTING,
            0,
            null_mut(),
        );
        if con != INVALID_HANDLE_VALUE {
            let text: Vec<u16> = format!("\r\n{text}\r\n").encode_utf16().collect();
            let mut written = 0;
            WriteConsoleW(con, text.as_ptr(), text.len() as u32, &mut written, null());
        }
    }
}
