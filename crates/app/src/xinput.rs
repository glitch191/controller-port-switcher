//! Reads the physical slots from the system XInput (by full path, so a proxy placed
//! next to the app can never be picked up).

use crate::win::wide;
use cps_core::mapping::{SLOTS, SlotState};
use std::ffi::c_void;
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;

#[repr(C)]
#[derive(Default)]
struct Gamepad {
    buttons: u16,
    left_trigger: u8,
    right_trigger: u8,
    thumbs: [i16; 4],
}

#[repr(C)]
#[derive(Default)]
struct State {
    packet: u32,
    gamepad: Gamepad,
}

#[repr(C)]
#[derive(Default)]
struct Capabilities {
    kind: u8,
    subtype: u8,
    flags: u16,
    gamepad: Gamepad,
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

type GetState = unsafe extern "system" fn(u32, *mut State) -> u32;
type GetCaps = unsafe extern "system" fn(u32, u32, *mut Capabilities) -> u32;
type GetCapsEx = unsafe extern "system" fn(u32, u32, u32, *mut CapabilitiesEx) -> u32;

pub struct XInput {
    get_state: GetState,
    get_caps: GetCaps,
    get_caps_ex: Option<GetCapsEx>,
}

impl XInput {
    pub fn load() -> Option<Self> {
        let mut dir = [0u16; 260];
        let n = unsafe { GetSystemDirectoryW(dir.as_mut_ptr(), dir.len() as u32) } as usize;
        let path = String::from_utf16_lossy(dir.get(..n)?) + "\\xinput1_4.dll";
        unsafe {
            let m = LoadLibraryW(wide(&path).as_ptr());
            if m.is_null() {
                return None;
            }
            let get = |name: *const u8| GetProcAddress(m, name).map(|f| f as *const c_void);
            Some(Self {
                get_state: std::mem::transmute::<*const c_void, GetState>(get(c"XInputGetState".as_ptr().cast())?),
                get_caps: std::mem::transmute::<*const c_void, GetCaps>(get(c"XInputGetCapabilities".as_ptr().cast())?),
                // Undocumented XInputGetCapabilitiesEx: reports the USB vendor and product id.
                get_caps_ex: get(108 as *const u8).map(|f| std::mem::transmute::<*const c_void, GetCapsEx>(f)),
            })
        }
    }

    pub fn slots(&self) -> [SlotState; SLOTS] {
        let mut out = [SlotState::default(); SLOTS];
        for (i, slot) in out.iter_mut().enumerate() {
            let i = i as u32;
            if let Some(f) = self.get_caps_ex {
                let mut c = CapabilitiesEx::default();
                if unsafe { f(1, i, 0, &mut c) } == 0 {
                    *slot = SlotState::connected(c.caps.subtype, c.vid, c.pid);
                }
                continue;
            }
            let mut c = Capabilities::default();
            if unsafe { (self.get_caps)(i, 0, &mut c) } == 0 {
                *slot = SlotState { connected: true, subtype: c.subtype, id: None };
            }
        }
        out
    }

    /// Pressed inputs per slot as a bit set (buttons, triggers, sticks pushed far),
    /// or None when the slot is empty. Used by "Identify controllers".
    pub fn inputs(&self) -> [Option<u32>; SLOTS] {
        let mut out = [None; SLOTS];
        for (i, v) in out.iter_mut().enumerate() {
            let mut s = State::default();
            if unsafe { (self.get_state)(i as u32, &mut s) } != 0 {
                continue;
            }
            let g = &s.gamepad;
            let mut bits = g.buttons as u32;
            bits |= ((g.left_trigger > 128) as u32) << 16 | ((g.right_trigger > 128) as u32) << 17;
            for (k, &t) in g.thumbs.iter().enumerate() {
                bits |= ((t as i32).abs() > 24000) as u32 * (1 << (18 + k));
            }
            *v = Some(bits);
        }
        out
    }
}
