//! XInput proxy. Built as xinput1_3.dll, xinput1_4.dll and xinput9_1_0.dll by the thin
//! crates next to this one; each picks its exports (names and ordinals) with a .def file.
//!
//! Rules followed here:
//! - The real XInput is loaded from System32 by full path, lazily on the first call,
//!   never in DllMain (loader lock).
//! - The per-game config is checked at most once per second, inside a call, without
//!   creating a thread. Checking is guarded by a flag, never by a blocking lock.
//! - State calls do one atomic load of the mapping table: no allocation, no lock.
//! - Any failure falls back to passing calls through unchanged.
//!
//! The crate is `no_std` and the DLLs link no C runtime (entry point disabled with
//! /NOENTRY), which keeps each DLL small.
//!
//! Functions here have Rust-mangled names; the thin crates export the subset their
//! system DLL has through `export!`, so no extra export leaks into a DLL.

#![cfg_attr(not(test), no_std)]
#![allow(non_snake_case)]

mod log;

use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::ptr::null_mut;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, Ordering::*};
use cps_core::mapping::{self, IDENTITY, NO_SLOT, Rule, SLOTS, SlotState};
use cps_core::{PROXY_CONFIG_FILE, PROXY_LOG_FILE, PROXY_MARKER, proxycfg};
use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, GENERIC_READ, HMODULE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileAttributesExW,
    GetFileExInfoStandard, OPEN_EXISTING, ReadFile, WIN32_FILE_ATTRIBUTE_DATA,
};
use windows_sys::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleFileNameW,
    GetModuleHandleExW, GetProcAddress, LoadLibraryW,
};
use windows_sys::Win32::System::SystemInformation::{GetSystemDirectoryW, GetTickCount64};

const ERROR_SUCCESS: u32 = 0;
const ERROR_DEVICE_NOT_CONNECTED: u32 = 1167;
const ERROR_EMPTY: u32 = 4306;
const XUSER_INDEX_ANY: u32 = 0xFF;
const PATH_MAX: usize = 1024;
const CONFIG_MAX: usize = 8192;

/// Kept in the binary so the app can tell its own DLL from a foreign one.
#[used]
static MARKER: &[u8] = PROXY_MARKER;

/// Packed player -> slot table read by every call.
static MAP: AtomicU32 = AtomicU32::new(IDENTITY);
/// Tick count (ms) after which the next config check may run.
static NEXT_CHECK: AtomicU64 = AtomicU64::new(0);
/// Set while one thread refreshes; other threads keep using MAP.
static BUSY: AtomicBool = AtomicBool::new(false);
static REAL: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static REAL_14: AtomicPtr<c_void> = AtomicPtr::new(null_mut());

struct State {
    ready: bool,
    config_path: [u16; PATH_MAX],
    log_path: [u16; PATH_MAX],
    mtime: u64,
    rules: [Rule; SLOTS],
    log: bool,
}

struct Shared(UnsafeCell<State>);
// Only touched by the thread that owns BUSY.
unsafe impl Sync for Shared {}

static STATE: Shared = Shared(UnsafeCell::new(State {
    ready: false,
    config_path: [0; PATH_MAX],
    log_path: [0; PATH_MAX],
    mtime: 0,
    rules: [Rule::Auto; SLOTS],
    log: false,
}));

// ---------------------------------------------------------------- paths and loading

/// Writes the full path of this DLL into `buf` and returns (directory length including
/// the trailing backslash, path length). The file name is `buf[dir_len..len]`.
fn own_path(buf: &mut [u16; PATH_MAX]) -> Option<(usize, usize)> {
    let mut me: HMODULE = null_mut();
    let flags = GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
    if unsafe { GetModuleHandleExW(flags, (&raw const MAP).cast(), &mut me) } == 0 {
        return None;
    }
    let n = unsafe { GetModuleFileNameW(me, buf.as_mut_ptr(), PATH_MAX as u32) } as usize;
    if n == 0 || n >= PATH_MAX {
        return None;
    }
    let dir_len = buf[..n].iter().rposition(|&c| c == b'\\' as u16)? + 1;
    Some((dir_len, n))
}

/// Appends `name` (ASCII) plus a terminator at `at`. Returns false if it does not fit.
fn put_ascii(buf: &mut [u16; PATH_MAX], at: usize, name: &str) -> bool {
    if at + name.len() + 1 > PATH_MAX {
        return false;
    }
    for (i, b) in name.bytes().enumerate() {
        buf[at + i] = b as u16;
    }
    buf[at + name.len()] = 0;
    true
}

/// Loads `System32\<name>`, where name is a wide slice without terminator.
fn load_system(name: &[u16]) -> HMODULE {
    let mut path = [0u16; PATH_MAX];
    let n = unsafe { GetSystemDirectoryW(path.as_mut_ptr(), PATH_MAX as u32) } as usize;
    if n == 0 || n + 1 + name.len() + 1 > PATH_MAX {
        return null_mut();
    }
    path[n] = b'\\' as u16;
    path[n + 1..n + 1 + name.len()].copy_from_slice(name);
    path[n + 1 + name.len()] = 0;
    unsafe { LoadLibraryW(path.as_ptr()) }
}

fn wide(s: &str, out: &mut [u16; 32]) -> usize {
    for (i, b) in s.bytes().enumerate().take(32) {
        out[i] = b as u16;
    }
    s.len().min(32)
}

/// The system XInput with the same file name as this DLL. Several threads may race
/// here; LoadLibraryW returns the same handle to all of them.
fn real() -> HMODULE {
    let m = REAL.load(Acquire);
    if !m.is_null() {
        return m;
    }
    let mut buf = [0u16; PATH_MAX];
    let mut m = match own_path(&mut buf) {
        Some((dir, end)) => load_system(&buf[dir..end]),
        None => null_mut(),
    };
    if m.is_null() {
        // Renamed DLL or missing legacy XInput (xinput1_3 ships with the DirectX runtime).
        let mut name = [0u16; 32];
        let n = wide("xinput1_4.dll", &mut name);
        m = load_system(&name[..n]);
    }
    REAL.store(m, Release);
    m
}

/// System xinput1_4.dll, the only one exporting XInputGetCapabilitiesEx (ordinal 108).
fn real_14() -> HMODULE {
    let m = REAL_14.load(Acquire);
    if !m.is_null() {
        return m;
    }
    let mut name = [0u16; 32];
    let n = wide("xinput1_4.dll", &mut name);
    let m = load_system(&name[..n]);
    REAL_14.store(m, Release);
    m
}

const fn ordinal(n: usize) -> *const u8 {
    n as *const u8
}

/// Resolves and caches an export of the real DLL. `F` must be a function pointer type.
unsafe fn proc<F: Copy>(cache: &AtomicPtr<c_void>, module: fn() -> HMODULE, name: *const u8) -> Option<F> {
    let mut p = cache.load(Relaxed);
    if p.is_null() {
        let m = module();
        if m.is_null() {
            return None;
        }
        p = unsafe { GetProcAddress(m, name) }? as *mut c_void;
        cache.store(p, Relaxed);
    }
    debug_assert_eq!(size_of::<F>(), size_of::<*mut c_void>());
    Some(unsafe { core::mem::transmute_copy(&p) })
}

// ---------------------------------------------------------------- config and mapping

/// Physical slot for a player index, or None for "not connected". Indices outside
/// 0-3 pass through so the real XInput reports the error itself.
fn slot_for(user: u32) -> Option<u32> {
    refresh_if_due();
    if user as usize >= SLOTS {
        return Some(user);
    }
    let s = mapping::unpack(MAP.load(Relaxed))[user as usize];
    (s != NO_SLOT).then_some(s as u32)
}

fn refresh_if_due() {
    let now = unsafe { GetTickCount64() };
    if now < NEXT_CHECK.load(Relaxed) || BUSY.swap(true, Acquire) {
        return;
    }
    NEXT_CHECK.store(now + 1000, Relaxed);
    // SAFETY: BUSY is held, so this thread is the only one touching STATE.
    refresh(unsafe { &mut *STATE.0.get() });
    BUSY.store(false, Release);
}

fn refresh(st: &mut State) {
    if !st.ready {
        let mut dir = [0u16; PATH_MAX];
        let Some((dir_len, _)) = own_path(&mut dir) else { return };
        st.config_path[..dir_len].copy_from_slice(&dir[..dir_len]);
        st.log_path[..dir_len].copy_from_slice(&dir[..dir_len]);
        if !put_ascii(&mut st.config_path, dir_len, PROXY_CONFIG_FILE)
            || !put_ascii(&mut st.log_path, dir_len, PROXY_LOG_FILE)
        {
            return;
        }
        st.ready = true;
    }

    let mtime = file_mtime(&st.config_path);
    let reloaded = mtime != st.mtime;
    if reloaded {
        st.mtime = mtime;
        let cfg = if mtime == 0 { None } else { read_config(&st.config_path) };
        st.rules = cfg.map(|c| c.players).unwrap_or([Rule::Auto; SLOTS]);
        st.log = cfg.is_some_and(|c| c.log);
        if st.log {
            log::config(&st.log_path, cfg.is_some(), &st.rules);
        }
    }
    if !reloaded && !mapping::uses_devices(&st.rules) {
        return;
    }
    let slots = mapping::uses_devices(&st.rules).then(query_slots);
    let map = mapping::pack(&mapping::resolve(&st.rules, &slots.unwrap_or_default()));
    if MAP.swap(map, Relaxed) != map && st.log {
        log::map(&st.log_path, map, slots.as_ref());
    }
}

/// Last write time, or 0 when the file does not exist.
fn file_mtime(path: &[u16; PATH_MAX]) -> u64 {
    let mut data: WIN32_FILE_ATTRIBUTE_DATA = unsafe { core::mem::zeroed() };
    let ok = unsafe { GetFileAttributesExW(path.as_ptr(), GetFileExInfoStandard, (&raw mut data).cast()) };
    if ok == 0 {
        return 0;
    }
    let FILETIME { dwLowDateTime: lo, dwHighDateTime: hi } = data.ftLastWriteTime;
    ((hi as u64) << 32 | lo as u64).max(1)
}

fn read_config(path: &[u16; PATH_MAX]) -> Option<proxycfg::ProxyConfig> {
    let share = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
    let h = unsafe {
        CreateFileW(path.as_ptr(), GENERIC_READ, share, null_mut(), OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, null_mut())
    };
    if h == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut buf = [0u8; CONFIG_MAX];
    let mut len = 0u32;
    let ok = unsafe { ReadFile(h, buf.as_mut_ptr(), CONFIG_MAX as u32, &mut len, null_mut()) };
    unsafe { CloseHandle(h) };
    let len = len as usize;
    if ok == 0 || len >= CONFIG_MAX {
        return None;
    }
    proxycfg::parse(&buf[..len])
}

#[repr(C)]
#[derive(Default)]
pub struct Capabilities {
    kind: u8,
    subtype: u8,
    flags: u16,
    gamepad: [u8; 12],
    vibration: [u16; 2],
}

#[repr(C)]
#[derive(Default)]
struct CapabilitiesEx {
    caps: Capabilities,
    vid: u16,
    pid: u16,
    revision: u16,
    reserved: u16,
    serial: u32,
}

/// What the real XInput reports for slots 0-3 (without remapping).
fn query_slots() -> [SlotState; SLOTS] {
    type GetCapsEx = unsafe extern "system" fn(u32, u32, u32, *mut CapabilitiesEx) -> u32;
    type GetCaps = unsafe extern "system" fn(u32, u32, *mut Capabilities) -> u32;
    static EX: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
    static CAPS: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
    let mut out = [SlotState::default(); SLOTS];
    if let Some(f) = unsafe { proc::<GetCapsEx>(&EX, real_14, ordinal(108)) } {
        for (i, slot) in out.iter_mut().enumerate() {
            let mut c = CapabilitiesEx::default();
            if unsafe { f(1, i as u32, 0, &mut c) } == ERROR_SUCCESS {
                *slot = SlotState::connected(c.caps.subtype, c.vid, c.pid);
            }
        }
    } else if let Some(f) = unsafe { proc::<GetCaps>(&CAPS, real, c"XInputGetCapabilities".as_ptr().cast()) } {
        for (i, slot) in out.iter_mut().enumerate() {
            let mut c = Capabilities::default();
            if unsafe { f(i as u32, 0, &mut c) } == ERROR_SUCCESS {
                *slot = SlotState { connected: true, subtype: c.subtype, id: None };
            }
        }
    }
    out
}

// ---------------------------------------------------------------- exports

/// Defines an export that remaps the user index argument `$user`, then forwards to the
/// real function `$name` (a C string literal or an ordinal).
macro_rules! remapped {
    ($export:ident = $name:expr, user = $user:ident, ($($arg:ident: $ty:ty),*)) => {
        pub extern "system" fn $export($($arg: $ty),*) -> u32 {
            static PROC: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
            let Some($user) = slot_for($user) else { return ERROR_DEVICE_NOT_CONNECTED };
            match unsafe { proc::<unsafe extern "system" fn($($ty),*) -> u32>(&PROC, real, $name) } {
                Some(f) => unsafe { f($($arg),*) },
                None => ERROR_DEVICE_NOT_CONNECTED,
            }
        }
    };
}

const fn name(s: &'static core::ffi::CStr) -> *const u8 {
    s.as_ptr().cast()
}

pub type P = *mut c_void;

remapped!(XInputGetState = name(c"XInputGetState"), user = user, (user: u32, state: P));
remapped!(XInputSetState = name(c"XInputSetState"), user = user, (user: u32, vibration: P));
remapped!(XInputGetCapabilities = name(c"XInputGetCapabilities"), user = user, (user: u32, flags: u32, caps: P));
remapped!(XInputGetBatteryInformation = name(c"XInputGetBatteryInformation"), user = user, (user: u32, dev_type: u8, info: P));
remapped!(XInputGetAudioDeviceIds = name(c"XInputGetAudioDeviceIds"), user = user, (user: u32, render: P, render_count: P, capture: P, capture_count: P));
remapped!(XInputGetDSoundAudioDeviceGuids = name(c"XInputGetDSoundAudioDeviceGuids"), user = user, (user: u32, render: P, capture: P));
// Undocumented exports, by ordinal. Argument counts were checked against the x86
// system DLL (stack bytes popped by `ret`); each takes a user index.
remapped!(XInputGetStateEx = ordinal(100), user = user, (user: u32, state: P));
remapped!(XInputWaitForGuideButton = ordinal(101), user = user, (user: u32, flags: u32, overlapped: P));
remapped!(XInputCancelGuideButtonWait = ordinal(102), user = user, (user: u32));
remapped!(XInputPowerOffController = ordinal(103), user = user, (user: u32));
remapped!(XInputGetBaseBusInformation = ordinal(104), user = user, (user: u32, info: P));
remapped!(XInputGetCapabilitiesEx = ordinal(108), user = user, (reserved: u32, user: u32, flags: u32, caps: P));
remapped!(XInputOrdinal109 = ordinal(109), user = user, (user: u32, data: P));

pub extern "system" fn XInputEnable(enable: i32) {
    static PROC: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
    if let Some(f) = unsafe { proc::<unsafe extern "system" fn(i32)>(&PROC, real, name(c"XInputEnable")) } {
        unsafe { f(enable) }
    }
}

#[repr(C)]
pub struct Keystroke {
    virtual_key: u16,
    unicode: u16,
    flags: u16,
    user_index: u8,
    hid_code: u8,
}

/// Keystrokes carry the slot they came from; translate it back to the player index.
/// With XUSER_INDEX_ANY, keystrokes from slots no player reads are dropped.
pub extern "system" fn XInputGetKeystroke(user: u32, reserved: u32, keystroke: *mut Keystroke) -> u32 {
    static PROC: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
    type F = unsafe extern "system" fn(u32, u32, *mut Keystroke) -> u32;
    let Some(f) = (unsafe { proc::<F>(&PROC, real, name(c"XInputGetKeystroke")) }) else {
        return ERROR_DEVICE_NOT_CONNECTED;
    };
    if user == XUSER_INDEX_ANY {
        refresh_if_due();
        for _ in 0..16 {
            let r = unsafe { f(XUSER_INDEX_ANY, reserved, keystroke) };
            if r != ERROR_SUCCESS || keystroke.is_null() {
                return r;
            }
            let slot = unsafe { (*keystroke).user_index };
            if let Some(p) = mapping::player_for_slot(MAP.load(Relaxed), slot) {
                unsafe { (*keystroke).user_index = p };
                return r;
            }
        }
        return ERROR_EMPTY;
    }
    let Some(slot) = slot_for(user) else { return ERROR_DEVICE_NOT_CONNECTED };
    let r = unsafe { f(slot, reserved, keystroke) };
    if r == ERROR_SUCCESS && !keystroke.is_null() && (user as usize) < SLOTS {
        unsafe { (*keystroke).user_index = user as u8 };
    }
    r
}

/// Exported by name like the system XInput. It is not the entry point (the DLLs are
/// linked with /NOENTRY) and does nothing: all work waits for the first XInput call.
pub extern "system" fn DllMain(_module: P, _reason: u32, _reserved: P) -> i32 {
    1
}

/// Never reached in practice (no code path indexes out of bounds or unwraps), but a
/// no_std crate must define it. Fails fast instead of leaving the game in a bad state.
#[cfg(not(test))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    // __fastfail(FAST_FAIL_FATAL_APP_EXIT)
    unsafe { core::arch::asm!("int 0x29", in("ecx") 7, options(noreturn)) }
}

/// Defines the unmangled exports of one DLL variant. Each name must also be listed in
/// the variant's exports.def, which sets its ordinal.
#[macro_export]
macro_rules! export {
    ($($name:ident)*) => { $( $crate::export!(@one $name); )* };
    (@one DllMain) => { $crate::export!(@def DllMain(m: $crate::P, reason: u32, reserved: $crate::P) -> i32); };
    (@one XInputEnable) => { $crate::export!(@def XInputEnable(enable: i32) -> ()); };
    (@one XInputGetKeystroke) => { $crate::export!(@def XInputGetKeystroke(user: u32, reserved: u32, k: *mut $crate::Keystroke) -> u32); };
    (@one XInputGetState) => { $crate::export!(@def XInputGetState(user: u32, state: $crate::P) -> u32); };
    (@one XInputSetState) => { $crate::export!(@def XInputSetState(user: u32, vibration: $crate::P) -> u32); };
    (@one XInputGetCapabilities) => { $crate::export!(@def XInputGetCapabilities(user: u32, flags: u32, caps: $crate::P) -> u32); };
    (@one XInputGetBatteryInformation) => { $crate::export!(@def XInputGetBatteryInformation(user: u32, dev_type: u8, info: $crate::P) -> u32); };
    (@one XInputGetAudioDeviceIds) => { $crate::export!(@def XInputGetAudioDeviceIds(user: u32, a: $crate::P, b: $crate::P, c: $crate::P, d: $crate::P) -> u32); };
    (@one XInputGetDSoundAudioDeviceGuids) => { $crate::export!(@def XInputGetDSoundAudioDeviceGuids(user: u32, a: $crate::P, b: $crate::P) -> u32); };
    (@one XInputGetStateEx) => { $crate::export!(@def XInputGetStateEx(user: u32, state: $crate::P) -> u32); };
    (@one XInputWaitForGuideButton) => { $crate::export!(@def XInputWaitForGuideButton(user: u32, flags: u32, overlapped: $crate::P) -> u32); };
    (@one XInputCancelGuideButtonWait) => { $crate::export!(@def XInputCancelGuideButtonWait(user: u32) -> u32); };
    (@one XInputPowerOffController) => { $crate::export!(@def XInputPowerOffController(user: u32) -> u32); };
    (@one XInputGetBaseBusInformation) => { $crate::export!(@def XInputGetBaseBusInformation(user: u32, info: $crate::P) -> u32); };
    (@one XInputGetCapabilitiesEx) => { $crate::export!(@def XInputGetCapabilitiesEx(reserved: u32, user: u32, flags: u32, caps: $crate::P) -> u32); };
    (@one XInputOrdinal109) => { $crate::export!(@def XInputOrdinal109(user: u32, data: $crate::P) -> u32); };
    (@def $name:ident($($arg:ident: $ty:ty),*) -> $ret:ty) => {
        #[unsafe(no_mangle)]
        pub extern "system" fn $name($($arg: $ty),*) -> $ret {
            $crate::$name($($arg),*)
        }
    };
}
