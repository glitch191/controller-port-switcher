//! controller-port-switcher: notification area icon that shows the XInput slots and
//! manages per-game XInput proxies. No window, no console, no polling while idle: the
//! tooltip is refreshed on device notifications and when the pointer hovers the icon.

#![windows_subsystem = "windows"]

mod autostart;
mod games;
mod menu;
mod names;
mod procs;
mod shortcut;
mod tray;
mod win;
mod xinput;

use cps_core::PROJECT;
use cps_core::hotkey::Hotkey;
use cps_core::mapping::{SLOTS, SlotState};
use cps_core::tooltip::{self, SlotInfo, display_name};
use std::cell::{Cell, OnceCell};
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};
use win::wide;
use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{CreateMutexW, GetCurrentProcess, SetProcessWorkingSetSize};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DBT_DEVTYP_DEVICEINTERFACE, DEV_BROADCAST_DEVICEINTERFACE_W, DEVICE_NOTIFY_WINDOW_HANDLE,
    DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, HICON, KillTimer, MSG, PostQuitMessage,
    RegisterClassW, RegisterDeviceNotificationW, RegisterWindowMessageW, SetTimer, TranslateMessage, WM_CONTEXTMENU,
    WM_DESTROY, WM_DEVICECHANGE, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_ENDSESSION, WM_HOTKEY, WM_MOUSEMOVE,
    WM_SETTINGCHANGE, WM_TIMER, WNDCLASSW, WS_OVERLAPPED,
};

const NIN_SELECT: u32 = 0x400;
const NIN_KEYSELECT: u32 = 0x401;
const DBT_DEVNODES_CHANGED: usize = 0x0007;
const DBT_DEVICEARRIVAL: usize = 0x8000;
const DBT_DEVICEREMOVECOMPLETE: usize = 0x8004;

// One-shot timers after a device change: XInput may need a moment to see the device.
const TIMER_REFRESH_SOON: usize = 1;
const TIMER_REFRESH_LATER: usize = 2;
// Short polling only while "Identify controllers" waits for a button.
const TIMER_IDENTIFY: usize = 3;
const TIMER_IDENTIFY_END: usize = 4;
const IDENTIFY_TICK_MS: u32 = 50;
const IDENTIFY_TIMEOUT_TICKS: u32 = 15_000 / IDENTIFY_TICK_MS;
const IDENTIFY_RESULT_MS: u32 = 8_000;
const TIMER_VIBRATE_END: usize = 5;
const VIBRATE_MS: u32 = 700;
const TIMER_CAPTURE_END: usize = 6;
const CAPTURE_MS: u32 = 10_000;

/// Device interface classes watched for arrival and removal: HID (Xbox One and later
/// controllers through xinputhid) and XUSB (Xbox 360 class controllers).
const GUID_DEVINTERFACE_HID: windows_sys::core::GUID =
    windows_sys::core::GUID::from_u128(0x4d1e55b2_f16f_11cf_88cb_001111000030);
const GUID_DEVINTERFACE_XUSB: windows_sys::core::GUID =
    windows_sys::core::GUID::from_u128(0xec87f1e3_c13b_4100_b5f7_8b84d54260cb);

struct Identify {
    ticks: u32,
    /// Inputs held when the wait started (or since): only new presses count.
    held: [Option<u32>; SLOTS],
}

struct App {
    hwnd: HWND,
    xinput: Option<xinput::XInput>,
    names: names::Names,
    icon: Cell<HICON>,
    taskbar_created: u32,
    last_refresh: Cell<Option<Instant>>,
    identify: Cell<Option<Identify>>,
    /// True while the tooltip shows the Identify prompt or result.
    tip_busy: Cell<bool>,
    /// Physical slot currently vibrating from the "Vibrate" menu entry.
    vibrating: Cell<Option<usize>>,
}

thread_local! {
    static APP: OnceCell<App> = const { OnceCell::new() };
}

fn with_app(f: impl FnOnce(&App)) {
    APP.with(|a| {
        if let Some(app) = a.get() {
            f(app)
        }
    });
}

impl App {
    fn slots(&self) -> [SlotState; SLOTS] {
        self.xinput.as_ref().map(|x| x.slots()).unwrap_or_default()
    }

    /// Model names and battery levels for the connected slots.
    fn infos(&self, slots: &[SlotState; SLOTS]) -> [SlotInfo; SLOTS] {
        slot_infos(self.xinput.as_ref(), &self.names, slots)
    }

    fn status_text(&self) -> String {
        match &self.xinput {
            Some(x) => {
                let slots = x.slots();
                tooltip::status_text(PROJECT, &slots, &self.infos(&slots))
            }
            None => format!("{PROJECT}\nXInput is not available on this system"),
        }
    }

    fn refresh(&self) {
        self.last_refresh.set(Some(Instant::now()));
        if !self.tip_busy.get() {
            tray::set_tip(self.hwnd, &self.status_text());
        }
    }

    fn add_icon(&self) {
        self.icon.set(tray::load_icon(self.icon.get()));
        tray::add(self.hwnd, self.icon.get(), &self.status_text());
    }

    fn reload_icon(&self) {
        self.icon.set(tray::load_icon(self.icon.get()));
        tray::set_icon(self.hwnd, self.icon.get());
    }

    fn start_identify(&self) {
        let Some(x) = &self.xinput else { return };
        self.identify.set(Some(Identify {
            ticks: 0,
            held: x.inputs(),
        }));
        self.tip_busy.set(true);
        tray::set_tip(self.hwnd, &format!("{PROJECT}\nPress a button on a controller"));
        unsafe {
            KillTimer(self.hwnd, TIMER_IDENTIFY_END);
            SetTimer(self.hwnd, TIMER_IDENTIFY, IDENTIFY_TICK_MS, None);
        }
    }

    fn identify_tick(&self) {
        let (Some(x), Some(mut state)) = (&self.xinput, self.identify.take()) else {
            unsafe { KillTimer(self.hwnd, TIMER_IDENTIFY) };
            return;
        };
        let now = x.inputs();
        let pressed = (0..SLOTS).find(|&s| match (now[s], state.held[s]) {
            (Some(n), Some(h)) => n & !h != 0,
            (Some(n), None) => n != 0,
            _ => false,
        });
        state.ticks += 1;
        let text = match pressed {
            Some(s) => {
                let slots = x.slots();
                let infos = self.infos(&slots);
                Some(format!(
                    "{PROJECT}\nPort {} responded: {}",
                    s + 1,
                    display_name(&slots[s], &infos[s])
                ))
            }
            None if state.ticks >= IDENTIFY_TIMEOUT_TICKS => Some(format!("{PROJECT}\nNo button pressed")),
            None => None,
        };
        match text {
            Some(text) => {
                tray::set_tip(self.hwnd, &tooltip::truncate(&text, tooltip::TIP_MAX));
                unsafe {
                    KillTimer(self.hwnd, TIMER_IDENTIFY);
                    SetTimer(self.hwnd, TIMER_IDENTIFY_END, IDENTIFY_RESULT_MS, None);
                }
            }
            None => {
                // Released inputs can count again on the next press.
                for (held, now) in state.held.iter_mut().zip(now) {
                    *held = match (*held, now) {
                        (Some(h), Some(n)) => Some(h & n),
                        _ => Some(0),
                    };
                }
                self.identify.set(Some(state));
            }
        }
    }

    fn end_identify(&self) {
        unsafe { KillTimer(self.hwnd, TIMER_IDENTIFY_END) };
        self.tip_busy.set(false);
        self.refresh();
    }

    fn show_menu(&self) {
        let identifying = self.tip_busy.get();
        let slots = self.slots();
        let infos = self.infos(&slots);
        match menu::show(self.hwnd, slots, infos, identifying) {
            (0, _) => {}
            (menu::ID_IDENTIFY, _) => self.start_identify(),
            (menu::ID_QUIT, _) => unsafe {
                DestroyWindow(self.hwnd);
            },
            (menu::ID_SET_HOTKEY, _) => self.start_capture(),
            (menu::ID_CLEAR_HOTKEY, _) => {
                shortcut::unregister(self.hwnd);
                if let Err(e) = menu::save_hotkey(None) {
                    win::error(self.hwnd, &e);
                }
            }
            (cmd, ctx) => match menu::vibrate_target(cmd, &ctx) {
                Some(slot) => self.vibrate(slot),
                None => menu::run(self.hwnd, cmd, &ctx),
            },
        }
        trim_memory();
    }

    fn vibrate(&self, slot: usize) {
        let Some(x) = &self.xinput else { return };
        if let Some(previous) = self.vibrating.replace(Some(slot)) {
            x.vibrate(previous as u32, 0);
        }
        x.vibrate(slot as u32, 40_000);
        unsafe { SetTimer(self.hwnd, TIMER_VIBRATE_END, VIBRATE_MS, None) };
    }

    fn stop_vibration(&self) {
        unsafe { KillTimer(self.hwnd, TIMER_VIBRATE_END) };
        if let (Some(x), Some(slot)) = (&self.xinput, self.vibrating.take()) {
            x.vibrate(slot as u32, 0);
        }
    }

    fn swap(&self) {
        if let Err(e) = menu::swap_first_two(self.slots()) {
            win::error(self.hwnd, &e);
        }
    }

    fn start_capture(&self) {
        win::info(
            self.hwnd,
            "After you click OK, press the shortcut for \"Swap ports 1 and 2\", for example Ctrl+Alt+S.\n\n\
             Use Ctrl, Alt, Shift or Win with a key, or an F key alone. Esc cancels. You have 10 seconds.",
        );
        if !shortcut::start_capture(self.hwnd) {
            win::error(self.hwnd, "Cannot read the keyboard to set the shortcut.");
            return;
        }
        unsafe { SetTimer(self.hwnd, TIMER_CAPTURE_END, CAPTURE_MS, None) };
    }

    /// A key was pressed during capture: validate, register and save it.
    fn captured(&self, modifiers: u32, vk: u32) {
        shortcut::stop_capture();
        unsafe { KillTimer(self.hwnd, TIMER_CAPTURE_END) };
        if vk == shortcut::VK_ESCAPE && modifiers == 0 {
            return;
        }
        let hk = match Hotkey::new(modifiers, vk) {
            Ok(hk) => hk,
            Err(e) => return win::error(self.hwnd, &format!("{e} The shortcut was not changed.")),
        };
        shortcut::unregister(self.hwnd);
        if !shortcut::register(self.hwnd, &hk) {
            register_saved_hotkey(self.hwnd, false);
            return win::error(
                self.hwnd,
                &format!("{hk} is already used by another program. Choose another shortcut."),
            );
        }
        match menu::save_hotkey(Some(hk)) {
            Ok(()) => win::info(self.hwnd, &format!("\"Swap ports 1 and 2\" is now on {hk}.")),
            Err(e) => win::error(self.hwnd, &e),
        }
    }

    fn capture_timed_out(&self) {
        shortcut::stop_capture();
        unsafe { KillTimer(self.hwnd, TIMER_CAPTURE_END) };
        win::info(self.hwnd, "No key was pressed. The shortcut was not changed.");
    }
}

fn slot_infos(xinput: Option<&xinput::XInput>, names: &names::Names, slots: &[SlotState; SLOTS]) -> [SlotInfo; SLOTS] {
    let names = names.for_slots(slots);
    let mut out: [SlotInfo; SLOTS] = Default::default();
    for (i, (info, name)) in out.iter_mut().zip(names).enumerate() {
        if slots[i].connected {
            info.name = name;
            info.battery = xinput.and_then(|x| x.battery(i as u32));
        }
    }
    out
}

/// Registers the shortcut saved in config.json. Problems are reported only when
/// `report` is set (at startup), so the user knows why the shortcut does nothing.
fn register_saved_hotkey(hwnd: HWND, report: bool) {
    let Ok(cfg) = games::load() else { return };
    let Some(text) = cfg.swap_hotkey else { return };
    let problem = match Hotkey::parse(&text) {
        Ok(hk) if shortcut::register(hwnd, &hk) => return,
        Ok(hk) => format!("{hk} is already used by another program."),
        Err(e) => e,
    };
    if report {
        win::error(
            hwnd,
            &format!("The \"Swap ports 1 and 2\" shortcut does not work: {problem} Set another one from the menu."),
        );
    }
}

/// First launch turns "Start with Windows" on; later launches keep the Run entry
/// pointing at this executable if it moved. Turning it off in the menu is remembered.
fn sync_autostart() {
    let Ok(mut cfg) = games::load() else { return };
    match cfg.start_with_windows {
        None => {
            if autostart::enable().is_ok() {
                cfg.start_with_windows = Some(true);
                let _ = games::save(&cfg);
            }
        }
        Some(true) => {
            let _ = autostart::enable();
        }
        Some(false) => {}
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let mut handled = true;
    match msg {
        tray::WM_TRAY => match (lparam & 0xFFFF) as u32 {
            WM_CONTEXTMENU | NIN_SELECT | NIN_KEYSELECT => with_app(|a| a.show_menu()),
            WM_MOUSEMOVE => with_app(|a| {
                if a.last_refresh
                    .get()
                    .is_none_or(|t| t.elapsed() > Duration::from_millis(500))
                {
                    a.refresh();
                }
            }),
            _ => {}
        },
        WM_DEVICECHANGE => {
            if matches!(
                wparam,
                DBT_DEVICEARRIVAL | DBT_DEVICEREMOVECOMPLETE | DBT_DEVNODES_CHANGED
            ) {
                unsafe {
                    SetTimer(hwnd, TIMER_REFRESH_SOON, 300, None);
                    SetTimer(hwnd, TIMER_REFRESH_LATER, 1500, None);
                }
            }
        }
        WM_TIMER => match wparam {
            TIMER_REFRESH_SOON | TIMER_REFRESH_LATER => {
                unsafe { KillTimer(hwnd, wparam) };
                with_app(|a| {
                    a.names.clear();
                    a.refresh();
                });
            }
            TIMER_IDENTIFY => with_app(|a| a.identify_tick()),
            TIMER_IDENTIFY_END => with_app(|a| a.end_identify()),
            TIMER_VIBRATE_END => with_app(|a| a.stop_vibration()),
            TIMER_CAPTURE_END => with_app(|a| a.capture_timed_out()),
            _ => {}
        },
        WM_HOTKEY if wparam as i32 == shortcut::HOTKEY_ID => with_app(|a| a.swap()),
        shortcut::WM_SHORTCUT_CAPTURED => with_app(|a| a.captured(wparam as u32, lparam as u32)),
        WM_SETTINGCHANGE | WM_DISPLAYCHANGE | WM_DPICHANGED => with_app(|a| a.reload_icon()),
        WM_ENDSESSION => tray::remove(hwnd),
        WM_DESTROY => {
            tray::remove(hwnd);
            unsafe { PostQuitMessage(0) };
        }
        _ => handled = false,
    }
    let mut taskbar_created = false;
    with_app(|a| taskbar_created = msg == a.taskbar_created);
    if taskbar_created {
        // Explorer restarted: the icon must be added again.
        with_app(|a| a.add_icon());
        return 0;
    }
    if handled {
        0
    } else {
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }
}

/// Returns pages touched by startup or a menu action to the system, so the idle
/// footprint stays small. They are paged back in if needed.
fn trim_memory() {
    unsafe { SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX) };
}

fn register_device_notifications(hwnd: HWND) {
    for guid in [GUID_DEVINTERFACE_HID, GUID_DEVINTERFACE_XUSB] {
        let mut filter: DEV_BROADCAST_DEVICEINTERFACE_W = unsafe { std::mem::zeroed() };
        filter.dbcc_size = size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>() as u32;
        filter.dbcc_devicetype = DBT_DEVTYP_DEVICEINTERFACE;
        filter.dbcc_classguid = guid;
        unsafe { RegisterDeviceNotificationW(hwnd, (&raw const filter).cast(), DEVICE_NOTIFY_WINDOW_HANDLE) };
    }
}

fn main() {
    if std::env::args().skip(1).any(|a| a == "--status") {
        let text = match xinput::XInput::load() {
            Some(x) => {
                let slots = x.slots();
                tooltip::status_text(PROJECT, &slots, &slot_infos(Some(&x), &names::Names::default(), &slots))
            }
            None => format!("{PROJECT}\nXInput is not available on this system"),
        };
        win::print_console(&text);
        return;
    }

    // Single instance: a second launch exits without adding another icon.
    let mutex_name = wide(format!("Local\\{PROJECT}"));
    let _mutex = unsafe { CreateMutexW(null(), 1, mutex_name.as_ptr()) };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        return;
    }

    let class = wide(PROJECT);
    let hwnd = unsafe {
        let hinstance = GetModuleHandleW(null());
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassW(&wc);
        // A hidden top-level window (never shown): unlike a message-only window it
        // receives the TaskbarCreated broadcast.
        CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            hinstance,
            null(),
        )
    };
    if hwnd.is_null() {
        win::error(null_mut(), "Cannot create the notification window.");
        return;
    }
    register_device_notifications(hwnd);

    let app = App {
        hwnd,
        xinput: xinput::XInput::load(),
        names: names::Names::default(),
        icon: Cell::new(null_mut()),
        taskbar_created: unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) },
        last_refresh: Cell::new(None),
        identify: Cell::new(None),
        tip_busy: Cell::new(false),
        vibrating: Cell::new(None),
    };
    APP.with(|a| {
        let _ = a.set(app);
    });
    with_app(|a| a.add_icon());
    sync_autostart();
    register_saved_hotkey(hwnd, true);
    trim_memory();

    let mut msg: MSG = unsafe { std::mem::zeroed() };
    while unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) } > 0 {
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
