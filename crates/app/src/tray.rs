//! Notification area icon: add, update the tooltip, pick the icon for the taskbar
//! theme and DPI, remove.

use cps_core::tooltip::TIP_MAX;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows_sys::Win32::UI::HiDpi::{GetDpiForSystem, GetSystemMetricsForDpi};
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NOTIFYICON_VERSION_4,
    NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, HICON, IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW, SM_CXSMICON, WM_APP,
};

/// Callback message for icon events.
pub const WM_TRAY: u32 = WM_APP + 1;
const ICON_ID: u32 = 1;

fn data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut d: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    d.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    d.hWnd = hwnd;
    d.uID = ICON_ID;
    d
}

fn put_tip(d: &mut NOTIFYICONDATAW, tip: &str) {
    for (dst, src) in d.szTip.iter_mut().zip(tip.encode_utf16().take(TIP_MAX)) {
        *dst = src;
    }
}

pub fn add(hwnd: HWND, icon: HICON, tip: &str) -> bool {
    let mut d = data(hwnd);
    d.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
    d.uCallbackMessage = WM_TRAY;
    d.hIcon = icon;
    put_tip(&mut d, tip);
    if unsafe { Shell_NotifyIconW(NIM_ADD, &d) } == 0 {
        return false;
    }
    d.Anonymous.uVersion = NOTIFYICON_VERSION_4;
    unsafe { Shell_NotifyIconW(NIM_SETVERSION, &d) != 0 }
}

pub fn set_tip(hwnd: HWND, tip: &str) {
    let mut d = data(hwnd);
    d.uFlags = NIF_TIP | NIF_SHOWTIP;
    put_tip(&mut d, tip);
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &d) };
}

pub fn set_icon(hwnd: HWND, icon: HICON) {
    let mut d = data(hwnd);
    d.uFlags = NIF_ICON;
    d.hIcon = icon;
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &d) };
}

pub fn remove(hwnd: HWND) {
    let d = data(hwnd);
    unsafe { Shell_NotifyIconW(NIM_DELETE, &d) };
}

/// True when the taskbar uses the light theme (then a dark glyph is needed).
fn light_taskbar() -> bool {
    let key = crate::win::wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let value = crate::win::wide("SystemUsesLightTheme");
    let mut data: u32 = 0;
    let mut size = size_of::<u32>() as u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            null_mut(),
            (&raw mut data).cast(),
            &mut size,
        )
    };
    r == 0 && data != 0
}

/// Loads the icon matching the taskbar theme at the small-icon size for the current DPI.
/// The previous icon, if any, is destroyed.
pub fn load_icon(previous: HICON) -> HICON {
    let id: usize = if light_taskbar() { 1 } else { 2 };
    let size = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem()) };
    let icon = unsafe {
        LoadImageW(GetModuleHandleW(std::ptr::null()), id as *const u16, IMAGE_ICON, size, size, LR_DEFAULTCOLOR)
    };
    if !previous.is_null() {
        unsafe { DestroyIcon(previous) };
    }
    icon
}
