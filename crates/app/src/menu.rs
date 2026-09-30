//! Context menu, rebuilt each time it opens from config.json, the connected
//! controllers and the running games.
//!
//! The top of the menu lists the four ports as the running game sees them (or the
//! default order when no listed game runs); "Move to port N" changes that order.

use crate::games::{self, Status};
use crate::procs;
use crate::win::{self, wide};
use cps_core::appcfg::AppConfig;
use cps_core::mapping::{NO_SLOT, Rule, SLOTS, SlotState, move_port, resolve};
use cps_core::pe::Arch;
use cps_core::tooltip::describe;
use std::path::Path;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{HWND, POINT};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyMenu, GetCursorPos, HMENU, InsertMenuItemW, MENUITEMINFOW, MFS_DISABLED, MFT_SEPARATOR,
    MFT_STRING, MIIM_FTYPE, MIIM_ID, MIIM_STATE, MIIM_STRING, MIIM_SUBMENU, PostMessageW, SetForegroundWindow,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx, WM_NULL,
};

pub const ID_IDENTIFY: u32 = 1;
const ID_ADD: u32 = 2;
const ID_ADD_RUNNING: u32 = 3;
const ID_OPEN_CONFIG: u32 = 4;
pub const ID_QUIT: u32 = 5;
// Move commands: MOVE_BASE + from * SLOTS + to (ports 0-3).
const MOVE_BASE: u32 = 100;
// Game commands: GAME_BASE + game * GAME_STRIDE + code.
const GAME_BASE: u32 = 1000;
const GAME_STRIDE: u32 = 10;
const MAX_GAMES: usize = 6000;
const CODE_INSTALL: u32 = 0;
const CODE_REMOVE_PROXY: u32 = 1;
const CODE_DEFAULT_ORDER: u32 = 2;
const CODE_REMOVE_GAME: u32 = 3;
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
    /// Listed games that are running; empty means the default order is edited.
    targets: Vec<usize>,
    /// Order shown in the menu (of the first running game, or the default).
    rules: [Rule; SLOTS],
}

/// Listed games that are running now, and whether each one has loaded the proxy.
fn running_games(cfg: &AppConfig) -> Vec<RunningGame> {
    let procs = procs::running();
    let mut out = Vec::new();
    for (index, game) in cfg.games.iter().enumerate() {
        let Some(p) = procs.iter().find(|p| procs::same_path(&p.exe, Path::new(&game.exe))) else { continue };
        let dll = Path::new(&game.exe).with_file_name(&game.dll);
        let proxy = match procs::modules(p.pid) {
            None => ProxyState::Unknown,
            Some(mods) if mods.iter().any(|m| procs::same_path(m, &dll)) && games::status(game) == Status::Installed => {
                ProxyState::Loaded
            }
            Some(_) => ProxyState::NotLoaded,
        };
        out.push(RunningGame { index, proxy });
    }
    out
}

struct Item<'a> {
    text: &'a str,
    id: u32,
    enabled: bool,
    submenu: HMENU,
}

impl<'a> Item<'a> {
    fn new(text: &'a str, id: u32) -> Self {
        Self { text, id, enabled: true, submenu: null_mut() }
    }

    fn disabled(text: &'a str) -> Self {
        Self { enabled: false, ..Self::new(text, 0) }
    }
}

fn append(menu: HMENU, item: Item) {
    let text = wide(item.text.replace('&', "&&"));
    let mut mii: MENUITEMINFOW = unsafe { std::mem::zeroed() };
    mii.cbSize = size_of::<MENUITEMINFOW>() as u32;
    mii.fMask = MIIM_FTYPE | MIIM_STATE | MIIM_ID | MIIM_STRING | MIIM_SUBMENU;
    mii.fType = MFT_STRING;
    mii.fState = if item.enabled { 0 } else { MFS_DISABLED };
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
            let note = match (r.proxy, games::status(game)) {
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

fn add_ports(menu: HMENU, rules: &[Rule; SLOTS], slots: &[SlotState; SLOTS]) {
    let map = resolve(rules, slots);
    for (port, &slot) in map.iter().enumerate() {
        let connected = slot != NO_SLOT && slots[slot as usize].connected;
        if !connected {
            let what = if slot == NO_SLOT { "None" } else { "Empty" };
            append(menu, Item::disabled(&format!("Port {}: {what}", port + 1)));
            continue;
        }
        let sub = unsafe { CreatePopupMenu() };
        for to in (0..SLOTS).filter(|&to| to != port) {
            let id = MOVE_BASE + (port * SLOTS + to) as u32;
            append(sub, Item::new(&format!("Move to port {}", to + 1), id));
        }
        let text = format!("Port {}: {}", port + 1, describe(&slots[slot as usize]));
        append(menu, Item { submenu: sub, ..Item::new(&text, 0) });
    }
}

fn games_menu(cfg: &AppConfig) -> HMENU {
    let menu = unsafe { CreatePopupMenu() };
    if cfg.games.is_empty() {
        append(menu, Item::disabled("No game yet: use Add game"));
    }
    for (g, game) in cfg.games.iter().enumerate().take(MAX_GAMES) {
        let status = games::status(game);
        let sub = unsafe { CreatePopupMenu() };
        let bits = Arch::parse(&game.arch).map(|a| a.bits()).unwrap_or(0);
        let order = if game.players.is_some() { "own order" } else { "default order" };
        append(sub, Item::disabled(&format!("{bits}-bit, {}, proxy {}, {order}", game.dll, status.label())));
        separator(sub);
        let code = |c: u32| GAME_BASE + g as u32 * GAME_STRIDE + c;
        let installed = status == Status::Installed;
        let install = if installed { "Reinstall proxy" } else { "Install proxy" };
        append(sub, Item { enabled: status != Status::ExeMissing, ..Item::new(install, code(CODE_INSTALL)) });
        append(sub, Item { enabled: installed, ..Item::new("Remove proxy", code(CODE_REMOVE_PROXY)) });
        append(sub, Item { enabled: game.players.is_some(), ..Item::new("Use default order", code(CODE_DEFAULT_ORDER)) });
        append(sub, Item::new("Remove from list", code(CODE_REMOVE_GAME)));
        append(menu, Item { submenu: sub, ..Item::new(&format!("{} ({})", game.name, status.label()), 0) });
    }
    menu
}

fn build(cfg: &Result<AppConfig, String>, ctx: &Context, running: &[RunningGame], identifying: bool) -> HMENU {
    let menu = unsafe { CreatePopupMenu() };
    match cfg {
        Ok(cfg) => {
            append(menu, Item::disabled(&header(cfg, running)));
            add_ports(menu, &ctx.rules, &ctx.slots);
        }
        Err(_) => append(menu, Item::disabled("config.json is invalid: open the config folder to fix it")),
    }
    separator(menu);
    let identify = if identifying { "Identify controllers (waiting for a button)" } else { "Identify controllers" };
    append(menu, Item::new(identify, ID_IDENTIFY));
    if let Ok(cfg) = cfg {
        append(menu, Item { submenu: games_menu(cfg), ..Item::new("Games", 0) });
    }
    append(menu, Item { enabled: cfg.is_ok(), ..Item::new("Add game...", ID_ADD) });
    append(menu, Item { enabled: cfg.is_ok(), ..Item::new("Add running game...", ID_ADD_RUNNING) });
    separator(menu);
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
pub fn show(hwnd: HWND, slots: [SlotState; SLOTS], identifying: bool) -> (u32, Context) {
    let cfg = games::load();
    let running = cfg.as_ref().map(running_games).unwrap_or_default();
    let targets: Vec<usize> = running.iter().map(|r| r.index).collect();
    let rules = match (&cfg, targets.first()) {
        (Ok(c), Some(&g)) => c.games[g].rules(&c.default_players),
        (Ok(c), None) => c.default_players,
        (Err(_), _) => [Rule::Auto; SLOTS],
    };
    let ctx = Context { slots, targets, rules };
    let menu = build(&cfg, &ctx, &running, identifying);
    (track(hwnd, menu), ctx)
}

/// Runs a command other than Identify and Quit (handled by the caller).
pub fn run(hwnd: HWND, cmd: u32, ctx: &Context) {
    let result = match cmd {
        ID_ADD => games::add(hwnd),
        ID_ADD_RUNNING => add_running(hwnd),
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
        c if c >= GAME_BASE => game_command(hwnd, ((c - GAME_BASE) / GAME_STRIDE) as usize, (c - GAME_BASE) % GAME_STRIDE),
        _ => Ok(()),
    };
    if let Err(e) = result {
        win::error(hwnd, &e);
    }
}

/// Lists running programs that have XInput loaded in a second popup at the cursor.
fn add_running(hwnd: HWND) -> Result<(), String> {
    let cfg = games::load()?;
    let candidates = procs::xinput_programs();
    if candidates.is_empty() {
        win::info(hwnd, "No running program has XInput loaded. Start the game, then try again.");
        return Ok(());
    }
    let menu = unsafe { CreatePopupMenu() };
    append(menu, Item::disabled("Programs using XInput now:"));
    for (i, c) in candidates.iter().enumerate() {
        let name = c.exe.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        let listed = cfg.games.iter().any(|g| procs::same_path(Path::new(&g.exe), &c.exe));
        let text = if listed { format!("{name} ({}, already in the list)", c.dll) } else { format!("{name} ({})", c.dll) };
        append(menu, Item { enabled: !listed, ..Item::new(&text, CANDIDATE_BASE + i as u32) });
    }
    let cmd = track(hwnd, menu);
    match cmd.checked_sub(CANDIDATE_BASE).and_then(|i| candidates.get(i as usize)) {
        Some(c) => games::add_running(hwnd, &c.exe, c.dll),
        None => Ok(()),
    }
}

fn game_command(hwnd: HWND, g: usize, code: u32) -> Result<(), String> {
    let mut cfg = games::load()?;
    let Some(game) = cfg.games.get(g).cloned() else { return Ok(()) };
    match code {
        CODE_INSTALL => games::install(hwnd, &game, &cfg.default_players).map(|_| ()),
        CODE_REMOVE_PROXY => games::remove_proxy(&game),
        CODE_DEFAULT_ORDER => {
            cfg.games[g].players = None;
            games::save(&cfg)?;
            if games::status(&game) == Status::Installed {
                games::write_proxy_config(&cfg.games[g], &cfg.default_players)?;
            }
            Ok(())
        }
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
        _ => Ok(()),
    }
}
