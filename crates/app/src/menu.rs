//! Context menu, rebuilt from config.json and the connected controllers each time it
//! opens, and the handling of the chosen command.

use crate::games::{self, Status};
use crate::win::{self, wide};
use cps_core::appcfg::AppConfig;
use cps_core::mapping::{Rule, SLOTS, SlotState, instance_of, rule_for_slot};
use cps_core::tooltip::{controller_label, describe};
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{HWND, POINT};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyMenu, GetCursorPos, HMENU, InsertMenuItemW, MENUITEMINFOW, MFS_CHECKED, MFS_DISABLED,
    MFT_RADIOCHECK, MFT_SEPARATOR, MFT_STRING, MIIM_FTYPE, MIIM_ID, MIIM_STATE, MIIM_STRING, MIIM_SUBMENU,
    PostMessageW, SetForegroundWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx, WM_NULL,
};

pub const ID_IDENTIFY: u32 = 1;
const ID_ADD: u32 = 2;
const ID_OPEN_CONFIG: u32 = 3;
pub const ID_QUIT: u32 = 4;

// Game commands: GAME_BASE + game * GAME_STRIDE + code.
const GAME_BASE: u32 = 1000;
const GAME_STRIDE: u32 = 100;
const MAX_GAMES: usize = 600;
// Player choice code: player * 20 + choice (0 = Auto, 1 = None, 2 + slot, 9 = saved device).
const CHOICE_AUTO: u32 = 0;
const CHOICE_NONE: u32 = 1;
const CHOICE_SLOT: u32 = 2;
const CHOICE_SAVED: u32 = 9;
const CODE_INSTALL: u32 = 90;
const CODE_REMOVE_PROXY: u32 = 91;
const CODE_REMOVE_GAME: u32 = 92;

struct Item<'a> {
    text: &'a str,
    id: u32,
    checked: bool,
    radio: bool,
    enabled: bool,
    submenu: HMENU,
}

impl<'a> Item<'a> {
    fn new(text: &'a str, id: u32) -> Self {
        Self { text, id, checked: false, radio: false, enabled: true, submenu: null_mut() }
    }
}

fn append(menu: HMENU, item: Item) {
    let text = wide(item.text.replace('&', "&&"));
    let mut mii: MENUITEMINFOW = unsafe { std::mem::zeroed() };
    mii.cbSize = size_of::<MENUITEMINFOW>() as u32;
    mii.fMask = MIIM_FTYPE | MIIM_STATE | MIIM_ID | MIIM_STRING | MIIM_SUBMENU;
    mii.fType = MFT_STRING | if item.radio { MFT_RADIOCHECK } else { 0 };
    mii.fState = if item.checked { MFS_CHECKED } else { 0 } | if item.enabled { 0 } else { MFS_DISABLED };
    mii.wID = item.id;
    mii.hSubMenu = item.submenu;
    mii.dwTypeData = text.as_ptr() as *mut u16;
    unsafe { InsertMenuItemW(menu, u32::MAX, 1, &mii) };
}

fn separator(menu: HMENU) {
    let mut mii: MENUITEMINFOW = unsafe { std::mem::zeroed() };
    mii.cbSize = size_of::<MENUITEMINFOW>() as u32;
    mii.fMask = MIIM_FTYPE;
    mii.fType = MFT_SEPARATOR;
    unsafe { InsertMenuItemW(menu, u32::MAX, 1, &mii) };
}

/// Slot the rule currently points to, if that controller is connected.
fn rule_slot(rule: &Rule, slots: &[SlotState; SLOTS]) -> Option<usize> {
    match *rule {
        Rule::Slot(s) => Some(s as usize),
        Rule::Device { id, instance, .. } => {
            (0..SLOTS).find(|&s| slots[s].connected && slots[s].id == Some(id) && instance_of(slots, s) == instance)
        }
        _ => None,
    }
}

fn rule_text(rule: &Rule, slots: &[SlotState; SLOTS]) -> String {
    match (*rule, rule_slot(rule, slots)) {
        (Rule::Auto, _) => "Auto".into(),
        (Rule::None, _) => "None".into(),
        (Rule::Slot(s), _) if !slots[s as usize].connected => format!("Slot {s} (empty)"),
        (_, Some(s)) => controller_label(s, &slots[s]),
        (Rule::Device { id, instance, .. }, None) => saved_device_text(id, instance),
        (Rule::Slot(s), None) => format!("Slot {s}"),
    }
}

fn saved_device_text(id: cps_core::mapping::DeviceId, instance: u8) -> String {
    let nth = if instance > 0 { format!(" #{}", instance + 1) } else { String::new() };
    format!("Device {id}{nth} (not connected)")
}

fn player_menu(g: usize, p: usize, rule: &Rule, slots: &[SlotState; SLOTS]) -> HMENU {
    let menu = unsafe { CreatePopupMenu() };
    let base = GAME_BASE + g as u32 * GAME_STRIDE + p as u32 * 20;
    let radio = |text: &str, code: u32, checked: bool| {
        append(menu, Item { radio: true, checked, ..Item::new(text, base + code) });
    };
    radio("Auto (next free controller)", CHOICE_AUTO, *rule == Rule::Auto);
    radio("None", CHOICE_NONE, *rule == Rule::None);
    separator(menu);
    let current = rule_slot(rule, slots);
    for (s, slot) in slots.iter().enumerate() {
        if slot.connected {
            radio(&controller_label(s, slot), CHOICE_SLOT + s as u32, current == Some(s));
        }
    }
    match *rule {
        Rule::Device { id, instance, .. } if current.is_none() => {
            radio(&saved_device_text(id, instance), CHOICE_SAVED, true);
        }
        Rule::Slot(s) if !slots[s as usize].connected => radio(&format!("Slot {s} (empty)"), CHOICE_SAVED, true),
        _ => {}
    }
    if !slots.iter().any(|s| s.connected) {
        append(menu, Item { enabled: false, ..Item::new("No controller connected", 0) });
    }
    menu
}

fn build(cfg: &Result<AppConfig, String>, slots: &[SlotState; SLOTS], identifying: bool) -> HMENU {
    let menu = unsafe { CreatePopupMenu() };
    let identify = if identifying { "Identify controllers (waiting for a button)" } else { "Identify controllers" };
    append(menu, Item::new(identify, ID_IDENTIFY));
    separator(menu);
    match cfg {
        Err(_) => append(menu, Item { enabled: false, ..Item::new("config.json is invalid: open the config folder", 0) }),
        Ok(cfg) => {
            for (g, game) in cfg.games.iter().enumerate().take(MAX_GAMES) {
                let status = games::status(game);
                let sub = unsafe { CreatePopupMenu() };
                let bits = cps_core::pe::Arch::parse(&game.arch).map(|a| a.bits()).unwrap_or(0);
                let info = format!("{bits}-bit, {}, proxy {}", game.dll, status.label());
                append(sub, Item { enabled: false, ..Item::new(&info, 0) });
                separator(sub);
                for (p, rule) in game.players.iter().enumerate() {
                    let text = format!("Player {}: {}", p + 1, rule_text(rule, slots));
                    append(sub, Item { submenu: player_menu(g, p, rule, slots), ..Item::new(&text, 0) });
                }
                separator(sub);
                let code = |c: u32| GAME_BASE + g as u32 * GAME_STRIDE + c;
                let installed = status == Status::Installed;
                let install_text = if installed { "Reinstall proxy" } else { "Install proxy" };
                append(sub, Item { enabled: status != Status::ExeMissing, ..Item::new(install_text, code(CODE_INSTALL)) });
                append(sub, Item { enabled: installed, ..Item::new("Remove proxy", code(CODE_REMOVE_PROXY)) });
                append(sub, Item::new("Remove from list", code(CODE_REMOVE_GAME)));
                let text = format!("{} ({})", game.name, status.label());
                append(menu, Item { submenu: sub, ..Item::new(&text, 0) });
            }
        }
    }
    append(menu, Item { enabled: cfg.is_ok(), ..Item::new("Add game...", ID_ADD) });
    separator(menu);
    append(menu, Item::new("Open config folder", ID_OPEN_CONFIG));
    append(menu, Item::new("Quit", ID_QUIT));
    menu
}

/// Shows the menu at the cursor and returns the chosen command (0 if none).
pub fn show(hwnd: HWND, slots: &[SlotState; SLOTS], identifying: bool) -> u32 {
    let cfg = games::load();
    let menu = build(&cfg, slots, identifying);
    let mut pt = POINT { x: 0, y: 0 };
    unsafe {
        GetCursorPos(&mut pt);
        // Required so the menu closes when the user clicks elsewhere.
        SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenuEx(menu, TPM_RIGHTBUTTON | TPM_RETURNCMD, pt.x, pt.y, hwnd, null_mut());
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
        cmd as u32
    }
}

/// Runs a game or config command. Identify and Quit are handled by the caller.
pub fn run(hwnd: HWND, cmd: u32, slots: &[SlotState; SLOTS]) {
    let result = match cmd {
        ID_ADD => games::add(hwnd),
        ID_OPEN_CONFIG => {
            let dir = games::config_dir();
            let _ = std::fs::create_dir_all(&dir);
            win::open_folder(&dir);
            Ok(())
        }
        c if c >= GAME_BASE => game_command(hwnd, ((c - GAME_BASE) / GAME_STRIDE) as usize, (c - GAME_BASE) % GAME_STRIDE, slots),
        _ => Ok(()),
    };
    if let Err(e) = result {
        win::error(hwnd, &e);
    }
}

fn game_command(hwnd: HWND, g: usize, code: u32, slots: &[SlotState; SLOTS]) -> Result<(), String> {
    let mut cfg = games::load()?;
    let Some(game) = cfg.games.get(g).cloned() else { return Ok(()) };
    match code {
        CODE_INSTALL => games::install(hwnd, &game),
        CODE_REMOVE_PROXY => games::remove_proxy(&game),
        CODE_REMOVE_GAME => {
            if games::status(&game) == Status::Installed {
                let text = format!(
                    "The proxy is still installed for {}. Remove it from the game folder too?\n\n\
                     Yes: remove the proxy and the game from the list.\nNo: keep the proxy, only remove the game from the list.",
                    game.name
                );
                match win::ask(hwnd, &text) {
                    None => return Ok(()),
                    Some(true) => games::remove_proxy(&game)?,
                    Some(false) => {}
                }
            }
            cfg.games.remove(g);
            games::save(&cfg)
        }
        _ if code < SLOTS as u32 * 20 => {
            let (p, choice) = ((code / 20) as usize, code % 20);
            let rule = match choice {
                CHOICE_AUTO => Rule::Auto,
                CHOICE_NONE => Rule::None,
                c if (CHOICE_SLOT..CHOICE_SLOT + SLOTS as u32).contains(&c) => {
                    let s = (c - CHOICE_SLOT) as usize;
                    if !slots[s].connected {
                        return Err(format!("{} is not connected any more.", describe(&slots[s])));
                    }
                    rule_for_slot(slots, s)
                }
                _ => return Ok(()),
            };
            set_rule(&mut cfg.games[g].players, p, rule);
            games::save(&cfg)?;
            if games::status(&cfg.games[g]) == Status::Installed {
                games::write_proxy_config(&cfg.games[g])?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Assigns `rule` to player `p`. A player that had the same controller gets `p`'s
/// previous choice, so choosing a controller swaps instead of leaving a gap.
fn set_rule(players: &mut [Rule; SLOTS], p: usize, rule: Rule) {
    let same = |a: &Rule, b: &Rule| match (a, b) {
        (Rule::Device { id: i1, instance: n1, .. }, Rule::Device { id: i2, instance: n2, .. }) => i1 == i2 && n1 == n2,
        (Rule::Slot(a), Rule::Slot(b)) => a == b,
        _ => false,
    };
    let old = players[p];
    if matches!(rule, Rule::Device { .. } | Rule::Slot(_)) {
        for q in 0..SLOTS {
            if q != p && same(&players[q], &rule) {
                players[q] = old;
            }
        }
    }
    players[p] = rule;
}
