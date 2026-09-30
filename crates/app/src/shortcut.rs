//! Keyboard shortcut for "Swap ports 1 and 2": registration with RegisterHotKey, and
//! capture of a new shortcut. Capture installs a low-level keyboard hook only while
//! it waits for the key (at most a few seconds), then removes it.

use cps_core::hotkey::{Hotkey, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN, is_modifier_key};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicPtr, Ordering};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, HOT_KEY_MODIFIERS, MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey, VK_CONTROL, VK_LWIN, VK_MENU,
    VK_RWIN, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, KBDLLHOOKSTRUCT, PostMessageW, SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_APP,
    WM_KEYDOWN, WM_SYSKEYDOWN,
};

/// Posted to the app window with (modifiers, virtual key) when a key was captured.
pub const WM_SHORTCUT_CAPTURED: u32 = WM_APP + 2;
pub const HOTKEY_ID: i32 = 1;
pub const VK_ESCAPE: u32 = 0x1B;

static HOOK: AtomicPtr<core::ffi::c_void> = AtomicPtr::new(null_mut());
static TARGET: AtomicPtr<core::ffi::c_void> = AtomicPtr::new(null_mut());

pub fn register(hwnd: HWND, hk: &Hotkey) -> bool {
    unsafe { RegisterHotKey(hwnd, HOTKEY_ID, hk.modifiers as HOT_KEY_MODIFIERS | MOD_NOREPEAT, hk.vk) != 0 }
}

pub fn unregister(hwnd: HWND) {
    unsafe { UnregisterHotKey(hwnd, HOTKEY_ID) };
}

fn down(vk: u16) -> bool {
    unsafe { GetAsyncKeyState(vk as i32) as u16 & 0x8000 != 0 }
}

extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && matches!(wparam as u32, WM_KEYDOWN | WM_SYSKEYDOWN) {
        let vk = unsafe { (*(lparam as *const KBDLLHOOKSTRUCT)).vkCode };
        if !is_modifier_key(vk) {
            let mut mods = 0;
            for (key, flag) in [(VK_CONTROL, MOD_CONTROL), (VK_MENU, MOD_ALT), (VK_SHIFT, MOD_SHIFT)] {
                if down(key) {
                    mods |= flag;
                }
            }
            if down(VK_LWIN) || down(VK_RWIN) {
                mods |= MOD_WIN;
            }
            unsafe {
                PostMessageW(
                    TARGET.load(Ordering::Relaxed),
                    WM_SHORTCUT_CAPTURED,
                    mods as usize,
                    vk as isize,
                )
            };
            // The key is used for the shortcut only; it does not reach other programs.
            return 1;
        }
    }
    unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
}

pub fn start_capture(hwnd: HWND) -> bool {
    stop_capture();
    TARGET.store(hwnd, Ordering::Relaxed);
    let h = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), GetModuleHandleW(std::ptr::null()), 0) };
    HOOK.store(h, Ordering::Relaxed);
    !h.is_null()
}

pub fn stop_capture() {
    let h = HOOK.swap(null_mut(), Ordering::Relaxed);
    if !h.is_null() {
        unsafe { UnhookWindowsHookEx(h) };
    }
}
