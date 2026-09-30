//! Context menu, rebuilt each time it opens from config.json, the connected
//! controllers and the running games.
//!
//! The top of the menu lists the four ports as the running game sees them (or the
//! default order when no listed game runs); "Move to port N" changes that order.

use crate::autostart;
use crate::games::{self, Status};
use crate::procs;
use crate::win::{self, wide};
use cps_core::appcfg::AppConfig;
use cps_core::mapping::{NO_SLOT, Rule, SLOTS, SlotState, move_port, resolve};
use cps_core::pe::Arch;
use cps_core::tooltip::{SlotInfo, menu_label};
use cps_core::{PROXY_LOG_FILE, hotkey::Hotkey};
use std::path::Path;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{HWND, POINT};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyMenu, GetCursorPos, HMENU, InsertMenuItemW, MENUITEMINFOW, MFS_CHECKED, MFS_DISABLED,
    MFT_SEPARATOR, MFT_STRING, MIIM_FTYPE, MIIM_ID, MIIM_STATE, MIIM_STRING, MIIM_SUBMENU, PostMessageW,
    SetForegroundWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx, WM_NULL,
};

pub const ID_IDENTIFY: u32 = 1;
const ID_ADD: u32 = 2;
const ID_ADD_RUNNING: u32 = 3;
const ID_OPEN_CONFIG: u32 = 4;
pub const ID_QUIT: u32 = 5;
const ID_SWAP: u32 = 6;
pub const ID_SET_HOTKEY: u32 = 7;
pub const ID_CLEAR_HOTKEY: u32 = 8;
const ID_AUTOSTART: u32 = 9;
const ID_REMOVE_ALL: u32 = 10;
// Move commands: MOVE_BASE + from * SLOTS + to (ports 0-3).
const MOVE_BASE: u32 = 100;
// Vibrate commands: VIBRATE_BASE + port.
const VIBRATE_BASE: u32 = 200;
// Game commands: GAME_BASE + game * GAME_STRIDE + code.
const GAME_BASE: u32 = 1000;
const GAME_STRIDE: u32 = 10;
const MAX_GAMES: usize = 6000;
const CODE_INSTALL: u32 = 0;
const CODE_REMOVE_PROXY: u32 = 1;
const CODE_DEFAULT_ORDER: u32 = 2;
const CODE_TOGGLE_LOG: u32 = 3;
const CODE_REMOVE_GAME: u32 = 4;
// Entries of the "Add running game" popup.
const CANDIDATE_BASE: u32 = 1;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ProxyState {
    Loaded,
    NotLoaded,
    /// The process could not be inspected (for example a protected game).
    Unknown,
}

struct RunningGame {
    index: usize,
    proxy: ProxyState,
}

/// What the menu showed, needed to run the chosen command.
pub struct Context {
    slots: [SlotState; SLOTS],
    infos: [SlotInfo; SLOTS],
    /// Listed games that are running; empty means the default order is edited.
    targets: Vec<usize>,
    /// Order shown in the menu (of the first running game, or the default).
    rules: [Rule; SLOTS],
}

impl Context {
    /// Physical slot shown at `port` (0-3), if a controller is there.
    fn slot_at(&self, port: usize) -> Option<usize> {
        let s = resolve(&self.rules, &self.slots)[port];
        (s != NO_SLOT && self.slots[s as usize].connected).then_some(s as usize)
    }
}

/// Listed games that are running now, and whether each one has loaded the proxy.
fn running_games(cfg: &AppConfig) -> Vec<RunningGame> {
    let procs = procs::running();
    let mut out = Vec::new();
    for (index, game) in cfg.games.iter().enumerate() {
        let Some(p) = procs.iter().find(|p| procs::same_path(&p.exe, Path::new(&game.exe))) else {
            continue;
        };
        let dll = Path::new(&game.exe).with_file_name(&game.dll);
        let proxy = match procs::proxy_loaded(p.pid, &dll) {
            None => ProxyState::Unknown,
            Some(true) => ProxyState::Loaded,
            Some(false) => ProxyState::NotLoaded,
        };
        out.push(RunningGame { index, proxy });
    }
    out
}

/// Reads the config and the running games, and builds the context for the order
/// being edited.
fn context(
    slots: [SlotState; SLOTS],
    infos: [SlotInfo; SLOTS],
) -> (Result<AppConfig, String>, Vec<RunningGame>, Context) {
    let cfg = games::load();
    let running = cfg.as_ref().map(running_games).unwrap_or_default();
    let targets: Vec<usize> = running.iter().map(|r| r.index).collect();
    let rules = match (&cfg, targets.first()) {
        (Ok(c), Some(&g)) => c.games[g].rules(&c.default_players),
        (Ok(c), None) => c.default_players,
        (Err(_), _) => [Rule::Auto; SLOTS],
    };
    (
        cfg,
        running,
        Context {
            slots,
            infos,
            targets,
            rules,
        },
    )
}

/// Swaps ports 1 and 2 of the running game(s), or of the default order (used by the
/// menu entry and the keyboard shortcut).
pub fn swap_first_two(slots: [SlotState; SLOTS]) -> Result<(), String> {
    let (cfg, _, ctx) = context(slots, Default::default());
    let mut cfg = cfg?;
    games::set_order(&mut cfg, &ctx.targets, move_port(&ctx.rules, &ctx.slots, 0, 1))
}

struct Item<'a> {
    text: &'a str,
    id: u32,
    enabled: bool,
    checked: bool,
    submenu: HMENU,
}

impl<'a> Item<'a> {
    fn new(text: &'a str, id: u32) -> Self {
        Self {
            text,
            id,
            enabled: true,
            checked: false,
            submenu: null_mut(),
        }
    }

    fn disabled(text: &'a str) -> Self {
        Self {
            enabled: false,
            ..Self::new(text, 0)
        }
    }
}

fn append(menu: HMENU, item: Item) {
    let text = wide(item.text.replace('&', "&&"));
    let mut mii: MENUITEMINFOW = unsafe { std::mem::zeroed() };
    mii.cbSize = size_of::<MENUITEMINFOW>() as u32;
    mii.fMask = MIIM_FTYPE | MIIM_STATE | MIIM_ID | MIIM_STRING | MIIM_SUBMENU;
    mii.fType = MFT_STRING;
    mii.fState = if item.enabled { 0 } else { MFS_DISABLED } | if item.checked { MFS_CHECKED } else { 0 };
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

fn header(cfg: &AppConfig, running: &[RunningGame]) -> String {
    if running.is_empty() {
        return "No game running: default order".into();
    }
    let names: Vec<String> = running
        .iter()
        .map(|r| {
            let game = &cfg.games[r.index];
            let status = games::status(game);
            let note = match (r.proxy, status) {
                (ProxyState::Loaded, Status::Outdated) => " (proxy outdated, update it after the game closes)",
                (ProxyState::Loaded, _) => "",
                (_, Status::NotInstalled | Status::Foreign | Status::ExeMissing) => " (proxy not installed)",
                (ProxyState::NotLoaded, _) => " (proxy not loaded, restart the game)",
                (ProxyState::Unknown, _) => " (proxy state unknown)",
            };
            format!("{}{note}", game.name)
        })
        .collect();
    format!("Running: {}", names.join(", "))
}

fn add_ports(menu: HMENU, ctx: &Context) {
    let map = resolve(&ctx.rules, &ctx.slots);
    let swappable = ctx.slot_at(0).is_some() || ctx.slot_at(1).is_some();
    append(
        menu,
        Item {
            enabled: swappable,
            ..Item::new("Swap ports 1 and 2", ID_SWAP)
        },
    );
    for (port, &slot) in map.iter().enumerate() {
        let Some(s) = ctx.slot_at(port) else {
            let what = if slot == NO_SLOT { "None" } else { "Empty" };
            append(menu, Item::disabled(&format!("Port {}: {what}", port + 1)));
            continue;
        };
        let sub = unsafe { CreatePopupMenu() };
        for to in (0..SLOTS).filter(|&to| to != port) {
            let id = MOVE_BASE + (port * SLOTS + to) as u32;
            append(sub, Item::new(&format!("Move to port {}", to + 1), id));
        }
        separator(sub);
        append(sub, Item::new("Vibrate", VIBRATE_BASE + port as u32));
        let text = format!("Port {}: {}", port + 1, menu_label(&ctx.slots[s], &ctx.infos[s]));
        append(
            menu,
            Item {
                submenu: sub,
                ..Item::new(&text, 0)
            },
        );
    }
}

fn games_menu(cfg: &AppConfig) -> HMENU {
    let menu = unsafe { CreatePopupMenu() };
    if cfg.games.is_empty() {
        append(menu, Item::disabled("No game yet: use Add game"));
    }
    let mut any_proxy = false;
    for (g, game) in cfg.games.iter().enumerate().take(MAX_GAMES) {
        games::delete_old_proxy(game);
        let status = games::status(game);
        any_proxy |= status.is_ours();
        let sub = unsafe { CreatePopupMenu() };
        let bits = Arch::parse(&game.arch).map(|a| a.bits()).unwrap_or(0);
        let order = if game.players.is_some() {
            "own order"
        } else {
            "default order"
        };
        append(
            sub,
            Item::disabled(&format!("{bits}-bit, {}, proxy {}, {order}", game.dll, status.label())),
        );
        separator(sub);
        let code = |c: u32| GAME_BASE + g as u32 * GAME_STRIDE + c;
        let install = match status {
            Status::Installed => "Reinstall proxy",
            Status::Outdated => "Update proxy",
            _ => "Install proxy",
        };
        append(
            sub,
            Item {
                enabled: status != Status::ExeMissing,
                ..Item::new(install, code(CODE_INSTALL))
            },
        );
        append(
            sub,
            Item {
                enabled: status.is_ours(),
                ..Item::new("Remove proxy", code(CODE_REMOVE_PROXY))
            },
        );
        append(
            sub,
            Item {
                enabled: game.players.is_some(),
                ..Item::new("Use default order", code(CODE_DEFAULT_ORDER))
            },
        );
        append(
            sub,
            Item {
                checked: game.log,
                ..Item::new("Enable proxy log", code(CODE_TOGGLE_LOG))
            },
        );
        append(sub, Item::new("Remove from list", code(CODE_REMOVE_GAME)));
        append(
            menu,
            Item {
                submenu: sub,
                ..Item::new(&format!("{} ({})", game.name, status.label()), 0)
            },
        );
    }
    separator(menu);
    append(
        menu,
        Item {
            enabled: any_proxy,
            ..Item::new("Remove all proxies", ID_REMOVE_ALL)
        },
    );
    menu
}

fn settings(menu: HMENU, cfg: &AppConfig) {
    let hotkey = cfg.swap_hotkey.as_deref().unwrap_or("none");
    let sub = unsafe { CreatePopupMenu() };
    append(sub, Item::new("Set shortcut...", ID_SET_HOTKEY));
    append(
        sub,
        Item {
            enabled: cfg.swap_hotkey.is_some(),
            ..Item::new("Clear shortcut", ID_CLEAR_HOTKEY)
        },
    );
    append(
        menu,
        Item {
            submenu: sub,
            ..Item::new(&format!("Swap shortcut: {hotkey}"), 0)
        },
    );
    append(
        menu,
        Item {
            checked: autostart::is_enabled(),
            ..Item::new("Start with Windows", ID_AUTOSTART)
        },
    );
}

fn build(cfg: &Result<AppConfig, String>, ctx: &Context, running: &[RunningGame], identifying: bool) -> HMENU {
    let menu = unsafe { CreatePopupMenu() };
    match cfg {
        Ok(cfg) => {
            append(menu, Item::disabled(&header(cfg, running)));
            add_ports(menu, ctx);
        }
        Err(_) => append(
            menu,
            Item::disabled("config.json is invalid: open the config folder to fix it"),
        ),
    }
    separator(menu);
    let identify = if identifying {
        "Identify controllers (waiting for a button)"
    } else {
        "Identify controllers"
    };
    append(menu, Item::new(identify, ID_IDENTIFY));
    if let Ok(cfg) = cfg {
        append(
            menu,
            Item {
                submenu: games_menu(cfg),
                ..Item::new("Games", 0)
            },
        );
    }
    append(
        menu,
        Item {
            enabled: cfg.is_ok(),
            ..Item::new("Add game...", ID_ADD)
        },
    );
    append(
        menu,
        Item {
            enabled: cfg.is_ok(),
            ..Item::new("Add running game...", ID_ADD_RUNNING)
        },
    );
    separator(menu);
    if let Ok(cfg) = cfg {
        settings(menu, cfg);
    }
    append(menu, Item::new("Open config folder", ID_OPEN_CONFIG));
    append(menu, Item::new("Quit", ID_QUIT));
    menu
}

fn track(hwnd: HWND, menu: HMENU) -> u32 {
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

/// Shows the menu at the cursor and returns the chosen command (0 if none) with the
/// context needed to run it.
pub fn show(hwnd: HWND, slots: [SlotState; SLOTS], infos: [SlotInfo; SLOTS], identifying: bool) -> (u32, Context) {
    let (cfg, running, ctx) = context(slots, infos);
    let menu = build(&cfg, &ctx, &running, identifying);
    (track(hwnd, menu), ctx)
}

/// Physical slot to vibrate when the command is a "Vibrate" entry.
pub fn vibrate_target(cmd: u32, ctx: &Context) -> Option<usize> {
    let port = cmd.checked_sub(VIBRATE_BASE).filter(|&p| (p as usize) < SLOTS)?;
    ctx.slot_at(port as usize)
}

/// Runs a command other than Identify, Quit, Vibrate and the shortcut entries
/// (handled by the caller).
pub fn run(hwnd: HWND, cmd: u32, ctx: &Context) {
    let result = match cmd {
        ID_ADD => games::add(hwnd),
        ID_ADD_RUNNING => add_running(hwnd),
        ID_SWAP => games::load()
            .and_then(|mut cfg| games::set_order(&mut cfg, &ctx.targets, move_port(&ctx.rules, &ctx.slots, 0, 1))),
        ID_AUTOSTART => toggle_autostart(),
        ID_REMOVE_ALL => remove_all(hwnd),
        ID_OPEN_CONFIG => {
            let dir = games::config_dir();
            let _ = std::fs::create_dir_all(&dir);
            win::open_folder(&dir);
            Ok(())
        }
        c if (MOVE_BASE..MOVE_BASE + (SLOTS * SLOTS) as u32).contains(&c) => {
            let (from, to) = (((c - MOVE_BASE) as usize) / SLOTS, ((c - MOVE_BASE) as usize) % SLOTS);
            games::load().and_then(|mut cfg| {
                let rules = move_port(&ctx.rules, &ctx.slots, from, to);
                games::set_order(&mut cfg, &ctx.targets, rules)
            })
        }
        c if c >= GAME_BASE => game_command(
            hwnd,
            ((c - GAME_BASE) / GAME_STRIDE) as usize,
            (c - GAME_BASE) % GAME_STRIDE,
        ),
        _ => Ok(()),
    };
    if let Err(e) = result {
        win::error(hwnd, &e);
    }
}

fn toggle_autostart() -> Result<(), String> {
    let mut cfg = games::load()?;
    let on = !autostart::is_enabled();
    if on {
        autostart::enable()?
    } else {
        autostart::disable()?
    }
    cfg.start_with_windows = Some(on);
    games::save(&cfg)
}

/// Saves a new swap shortcut (None clears it). Registering it is up to the caller.
pub fn save_hotkey(hotkey: Option<Hotkey>) -> Result<(), String> {
    let mut cfg = games::load()?;
    cfg.swap_hotkey = hotkey.map(|h| h.to_string());
    games::save(&cfg)
}

fn remove_all(hwnd: HWND) -> Result<(), String> {
    let cfg = games::load()?;
    let count = cfg.games.iter().filter(|g| games::status(g).is_ours()).count();
    let text = format!(
        "Remove the proxy from {count} game folder(s)?\n\nBacked-up DLLs are restored. The games stay in the list."
    );
    if !win::confirm(hwnd, &text) {
        return Ok(());
    }
    let removed = games::remove_all(&cfg)?;
    win::info(hwnd, &format!("Proxy removed from {removed} game folder(s)."));
    Ok(())
}

/// Lists running programs that have XInput loaded in a second popup at the cursor.
fn add_running(hwnd: HWND) -> Result<(), String> {
    let cfg = games::load()?;
    let candidates = procs::xinput_programs();
    if candidates.is_empty() {
        win::info(
            hwnd,
            "No running program has XInput loaded. Start the game, then try again.",
        );
        return Ok(());
    }
    let menu = unsafe { CreatePopupMenu() };
    append(menu, Item::disabled("Programs using XInput now:"));
    for (i, c) in candidates.iter().enumerate() {
        let name = c
            .exe
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        let listed = cfg.games.iter().any(|g| procs::same_path(Path::new(&g.exe), &c.exe));
        let text = if listed {
            format!("{name} ({}, already in the list)", c.dll)
        } else {
            format!("{name} ({})", c.dll)
        };
        append(
            menu,
            Item {
                enabled: !listed,
                ..Item::new(&text, CANDIDATE_BASE + i as u32)
            },
        );
    }
    let cmd = track(hwnd, menu);
    match cmd.checked_sub(CANDIDATE_BASE).and_then(|i| candidates.get(i as usize)) {
        Some(c) => games::add_running(hwnd, &c.exe, c.dll),
        None => Ok(()),
    }
}

fn game_command(hwnd: HWND, g: usize, code: u32) -> Result<(), String> {
    let mut cfg = games::load()?;
    let Some(game) = cfg.games.get(g).cloned() else {
        return Ok(());
    };
    let ours = games::status(&game).is_ours();
    match code {
        CODE_INSTALL => games::install(hwnd, &game, &cfg.default_players).map(|_| ()),
        CODE_REMOVE_PROXY => games::remove_proxy(&game),
        CODE_DEFAULT_ORDER => {
            cfg.games[g].players = None;
            games::save(&cfg)?;
            if ours {
                games::write_proxy_config(&cfg.games[g], &cfg.default_players)?;
            }
            Ok(())
        }
        CODE_TOGGLE_LOG => {
            cfg.games[g].log = !game.log;
            games::save(&cfg)?;
            if ours {
                games::write_proxy_config(&cfg.games[g], &cfg.default_players)?;
            }
            if cfg.games[g].log {
                let folder = Path::new(&game.exe)
                    .parent()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                win::info(
                    hwnd,
                    &format!(
                        "The proxy will write {PROXY_LOG_FILE} in {folder} when the game reads its configuration \
                         and when the order changes."
                    ),
                );
            }
            Ok(())
        }
        CODE_REMOVE_GAME => {
            if ours {
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
        _ => Ok(()),
    }
}
