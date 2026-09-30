//! xinput-probe: loads an XInput DLL by name (so a proxy in this folder wins over the
//! system one), then prints connection, subtype, device id and buttons for indices 0-3.

use cps_core::tooltip::subtype_name;
use std::ffi::c_void;
use std::time::{Duration, Instant};
use windows_sys::Win32::System::LibraryLoader::{GetModuleFileNameW, GetProcAddress, LoadLibraryW};

const USAGE: &str = "\
Usage: xinput-probe [--dll NAME] [--watch [SECONDS]] [--rumble INDEX]

  --dll NAME        XInput DLL to load (default xinput1_4.dll). A copy in this
                    folder is loaded first, which is how the proxy is tested.
  --watch [SECONDS] Print again every 500 ms when something changes
                    (for SECONDS, or until Ctrl+C).
  --rumble INDEX    Vibrate the controller at INDEX for about one second.";

#[repr(C)]
#[derive(Default, Clone, Copy, PartialEq)]
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
type SetState = unsafe extern "system" fn(u32, *mut [u16; 2]) -> u32;

struct XInput {
    path: String,
    get_state: GetState,
    get_caps: GetCaps,
    get_caps_ex: Option<GetCapsEx>,
    set_state: SetState,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn load(name: &str) -> Result<XInput, String> {
    unsafe {
        let m = LoadLibraryW(wide(name).as_ptr());
        if m.is_null() {
            return Err(format!("cannot load {name}"));
        }
        let mut buf = [0u16; 1024];
        let n = GetModuleFileNameW(m, buf.as_mut_ptr(), buf.len() as u32) as usize;
        let sym = |s: &[u8]| GetProcAddress(m, s.as_ptr()).map(|f| f as *const c_void);
        let need =
            |s: &[u8]| sym(s).ok_or_else(|| format!("{name} has no {}", String::from_utf8_lossy(&s[..s.len() - 1])));
        Ok(XInput {
            path: String::from_utf16_lossy(&buf[..n]),
            get_state: std::mem::transmute::<*const c_void, GetState>(need(b"XInputGetState\0")?),
            get_caps: std::mem::transmute::<*const c_void, GetCaps>(need(b"XInputGetCapabilities\0")?),
            get_caps_ex: GetProcAddress(m, 108 as *const u8).map(|f| std::mem::transmute::<_, GetCapsEx>(f)),
            set_state: std::mem::transmute::<*const c_void, SetState>(need(b"XInputSetState\0")?),
        })
    }
}

fn report(x: &XInput) -> String {
    let mut out = String::from("Index  State          Type               Device     Buttons\n");
    for i in 0..4 {
        let mut st = State::default();
        let r = unsafe { (x.get_state)(i, &mut st) };
        if r != 0 {
            let what = if r == 1167 {
                "not connected".to_string()
            } else {
                format!("error {r}")
            };
            out += &format!("{i:<6} {what:<14}\n");
            continue;
        }
        let mut c = Capabilities::default();
        let sub = if unsafe { (x.get_caps)(i, 0, &mut c) } == 0 {
            subtype_name(c.subtype)
        } else {
            "n/a"
        };
        let dev = match x.get_caps_ex {
            Some(f) => {
                let mut e = CapabilitiesEx::default();
                if unsafe { f(1, i, 0, &mut e) } == 0 {
                    format!("{:04X}:{:04X}", e.vid, e.pid)
                } else {
                    "n/a".into()
                }
            }
            None => "n/a".into(),
        };
        out += &format!(
            "{i:<6} {:<14} {sub:<18} {dev:<10} 0x{:04X}\n",
            "connected", st.gamepad.buttons
        );
    }
    out
}

fn main() {
    let mut dll = "xinput1_4.dll".to_string();
    let mut watch: Option<Option<u64>> = None;
    let mut rumble: Option<u32> = None;
    let mut args = std::env::args().skip(1).peekable();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--dll" => dll = args.next().unwrap_or_else(|| exit_usage()),
            "--watch" => {
                watch = Some(
                    args.next_if(|s| !s.starts_with("--"))
                        .map(|s| s.parse().unwrap_or_else(|_| exit_usage())),
                )
            }
            "--rumble" => {
                rumble = Some(
                    args.next()
                        .and_then(|s| s.parse().ok())
                        .filter(|&i| i < 4)
                        .unwrap_or_else(|| exit_usage()),
                )
            }
            _ => exit_usage(),
        }
    }
    let x = load(&dll).unwrap_or_else(|e| {
        eprintln!("Error: {e}");
        std::process::exit(1)
    });
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let local = exe_dir.is_some_and(|d| std::path::Path::new(&x.path).parent() == Some(d.as_path()));
    println!(
        "Loaded {} ({})",
        x.path,
        if local {
            "local copy, proxy under test"
        } else {
            "system XInput"
        }
    );

    if let Some(i) = rumble {
        let mut on = [32768u16, 32768u16];
        let r = unsafe { (x.set_state)(i, &mut on) };
        println!(
            "Rumble index {i}: {}",
            if r == 0 {
                "sent".to_string()
            } else {
                format!("error {r}")
            }
        );
        std::thread::sleep(Duration::from_millis(1000));
        unsafe { (x.set_state)(i, &mut [0, 0]) };
    }

    let mut last = report(&x);
    print!("{last}");
    if let Some(limit) = watch {
        let start = Instant::now();
        while limit.is_none_or(|s| start.elapsed() < Duration::from_secs(s)) {
            std::thread::sleep(Duration::from_millis(500));
            let now = report(&x);
            if now != last {
                println!("--- after {:.1} s", start.elapsed().as_secs_f32());
                print!("{now}");
                last = now;
            }
        }
    }
}

fn exit_usage() -> ! {
    eprintln!("{USAGE}");
    std::process::exit(2)
}
