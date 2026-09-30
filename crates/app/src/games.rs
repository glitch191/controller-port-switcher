//! Games list (%APPDATA%\controller-port-switcher\config.json) and proxy installation.

use crate::win;
use cps_core::appcfg::{APP_CONFIG_FILE, AppConfig, Game, proxy_config_json};
use cps_core::mapping::{Rule, SLOTS};
use cps_core::pe::{self, Arch, DllSource};
use cps_core::{PROJECT, PROXY_CONFIG_FILE, PROXY_LOG_FILE, XINPUT_DLLS};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::HWND;

const BACKUP_SUFFIX: &str = ".cps-backup";

pub fn config_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join(PROJECT)
}

fn config_path() -> PathBuf {
    config_dir().join(APP_CONFIG_FILE)
}

/// Reads the games list. A missing file is an empty list; a broken one is an error so
/// that it is never overwritten.
pub fn load() -> Result<AppConfig, String> {
    let path = config_path();
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| format!("{} is not valid: {e}. Fix or delete the file.", path.display())),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(AppConfig::default()),
        Err(e) => Err(format!("Cannot read {}: {e}", path.display())),
    }
}

pub fn save(cfg: &AppConfig) -> Result<(), String> {
    let path = config_path();
    let text = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())? + "\n";
    std::fs::create_dir_all(config_dir())
        .and_then(|_| write_atomic(&path, text.as_bytes()))
        .map_err(|e| format!("Cannot save {}: {e}", path.display()))
}

/// Writes through a temporary file and a rename, so readers (the proxy) never see a
/// half-written file.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".cps-tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    NotInstalled,
    Installed,
    /// Our proxy, from another version of the app: "Update proxy" replaces it.
    Outdated,
    Foreign,
    ExeMissing,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::NotInstalled => "not installed",
            Status::Installed => "installed",
            Status::Outdated => "outdated",
            Status::Foreign => "foreign DLL present",
            Status::ExeMissing => "exe not found",
        }
    }

    /// True when the DLL in the game folder is this proxy (any version).
    pub fn is_ours(self) -> bool {
        matches!(self, Status::Installed | Status::Outdated)
    }
}

fn game_dir(game: &Game) -> PathBuf {
    Path::new(&game.exe).parent().map(Path::to_path_buf).unwrap_or_default()
}

fn backup_path(dll: &Path) -> PathBuf {
    let mut p = dll.as_os_str().to_owned();
    p.push(BACKUP_SUFFIX);
    PathBuf::from(p)
}

pub fn status(game: &Game) -> Status {
    if !Path::new(&game.exe).is_file() {
        return Status::ExeMissing;
    }
    match std::fs::read(game_dir(game).join(&game.dll)) {
        Ok(bytes) if pe::is_proxy(&bytes) => match target(game) {
            Ok((arch, dll)) if bytes != dll_bytes(arch, dll) => Status::Outdated,
            _ => Status::Installed,
        },
        Ok(_) => Status::Foreign,
        Err(_) => Status::NotInstalled,
    }
}

/// Validated "arch" and "dll" of a game (both can be edited by hand in config.json).
fn target(game: &Game) -> Result<(Arch, &'static str), String> {
    let arch = Arch::parse(&game.arch)
        .ok_or_else(|| format!("config.json: \"arch\" of {} must be \"x64\" or \"x86\".", game.name))?;
    let dll = XINPUT_DLLS
        .iter()
        .copied()
        .find(|d| d.eq_ignore_ascii_case(&game.dll))
        .ok_or_else(|| {
            format!(
                "config.json: \"dll\" of {} must be one of {}.",
                game.name,
                XINPUT_DLLS.join(", ")
            )
        })?;
    Ok((arch, dll))
}

fn dll_bytes(arch: Arch, dll: &str) -> &'static [u8] {
    match (arch, dll) {
        (Arch::X64, "xinput1_3.dll") => include_bytes!(env!("CPS_DLL_X64_XINPUT1_3")),
        (Arch::X64, "xinput1_4.dll") => include_bytes!(env!("CPS_DLL_X64_XINPUT1_4")),
        (Arch::X64, _) => include_bytes!(env!("CPS_DLL_X64_XINPUT9_1_0")),
        (Arch::X86, "xinput1_3.dll") => include_bytes!(env!("CPS_DLL_X86_XINPUT1_3")),
        (Arch::X86, "xinput1_4.dll") => include_bytes!(env!("CPS_DLL_X86_XINPUT1_4")),
        (Arch::X86, _) => include_bytes!(env!("CPS_DLL_X86_XINPUT9_1_0")),
    }
}

/// Turns an I/O error into a message that says what to do.
fn io_message(e: &std::io::Error, action: &str, path: &Path) -> String {
    let dir = path.parent().unwrap_or(path).display();
    let file = path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    match (e.kind(), e.raw_os_error()) {
        (_, Some(32 | 33)) => format!("{file} is in use: close the game, then try again."),
        (ErrorKind::PermissionDenied, _) => format!(
            "Cannot write to the game folder ({dir}): run the game from a folder you own, or check its permissions. \
             If the game is running, close it first."
        ),
        _ => format!("Cannot {action} {}: {e}", path.display()),
    }
}

pub fn write_proxy_config(game: &Game, default: &[Rule; SLOTS]) -> Result<(), String> {
    let path = game_dir(game).join(PROXY_CONFIG_FILE);
    write_atomic(&path, proxy_config_json(game.log, &game.rules(default)).as_bytes())
        .map_err(|e| io_message(&e, "write", &path))
}

#[derive(Debug)]
struct Plan {
    arch: Arch,
    dll_name: &'static str,
    dll: PathBuf,
    backup: PathBuf,
    status: Status,
}

/// Checks that the proxy can be installed without losing any file.
fn plan_install(game: &Game) -> Result<Plan, String> {
    let (arch, dll_name) = target(game)?;
    let dll = game_dir(game).join(dll_name);
    let backup = backup_path(&dll);
    let status = status(game);
    if status == Status::ExeMissing {
        return Err(format!(
            "{} does not exist any more. Remove the game from the list and add it again.",
            game.exe
        ));
    }
    if status == Status::Foreign && backup.exists() {
        return Err(format!(
            "{dll_name} in {} is not this proxy, and a backup ({}) already exists.              Move one of the two files away, then try again.",
            game_dir(game).display(),
            backup.display()
        ));
    }
    Ok(Plan {
        arch,
        dll_name,
        dll,
        backup,
        status,
    })
}

/// Installs or updates the proxy after a confirmation that mentions anti-cheat systems
/// and, when needed, the backup of a foreign DLL.
/// Returns true when the proxy was written (false if the user declined).
pub fn install(hwnd: HWND, game: &Game, default: &[Rule; SLOTS]) -> Result<bool, String> {
    let plan = plan_install(game)?;
    if !plan.status.is_ours() {
        let mut text = format!(
            "Install the proxy for {}?\n\nThis writes {} ({}-bit) and {PROXY_CONFIG_FILE} to:\n{}\n\n",
            game.name,
            plan.dll_name,
            plan.arch.bits(),
            game_dir(game).display()
        );
        if plan.status == Status::Foreign {
            text += &format!(
                "A different {0} is already in this folder. It will be renamed to {0}{BACKUP_SUFFIX} \
                 and restored when you remove the proxy.\n\n",
                plan.dll_name
            );
        }
        text += "Games protected by an anti-cheat system may refuse to start or report this DLL.";
        if !win::confirm(hwnd, &text) {
            return Ok(false);
        }
    }
    install_files(game, &plan, default).map(|_| true)
}

fn install_files(game: &Game, plan: &Plan, default: &[Rule; SLOTS]) -> Result<(), String> {
    write_proxy_config(game, default)?;
    if plan.status == Status::Foreign {
        std::fs::rename(&plan.dll, &plan.backup).map_err(|e| io_message(&e, "rename", &plan.dll))?;
    }
    write_atomic(&plan.dll, dll_bytes(plan.arch, plan.dll_name)).map_err(|e| {
        if plan.status == Status::Foreign {
            let _ = std::fs::rename(&plan.backup, &plan.dll);
        }
        io_message(&e, "write", &plan.dll)
    })
}

/// Removes the proxy, its config and log, and restores a backed-up DLL.
pub fn remove_proxy(game: &Game) -> Result<(), String> {
    let (_, dll_name) = target(game)?;
    let dir = game_dir(game);
    let dll = dir.join(dll_name);
    let backup = backup_path(&dll);
    let remove = |p: &Path| match std::fs::remove_file(p) {
        Err(e) if e.kind() != ErrorKind::NotFound => Err(io_message(&e, "delete", p)),
        _ => Ok(()),
    };
    match status(game) {
        Status::Installed | Status::Outdated => remove(&dll)?,
        Status::Foreign => return Err(format!("{} is not this proxy; it was left in place.", dll.display())),
        Status::NotInstalled | Status::ExeMissing => {}
    }
    if backup.exists() && !dll.exists() {
        std::fs::rename(&backup, &dll).map_err(|e| io_message(&e, "restore", &dll))?;
    }
    remove(&dir.join(PROXY_CONFIG_FILE))?;
    remove(&dir.join(PROXY_LOG_FILE))
}

/// Removes the proxy from every listed game that has it. Returns how many were
/// removed, or the list of failures.
pub fn remove_all(cfg: &AppConfig) -> Result<usize, String> {
    let mut removed = 0;
    let mut errors = Vec::new();
    for game in cfg.games.iter().filter(|g| status(g).is_ours()) {
        match remove_proxy(game) {
            Ok(()) => removed += 1,
            Err(e) => errors.push(format!("{}: {e}", game.name)),
        }
    }
    if errors.is_empty() {
        Ok(removed)
    } else {
        Err(format!("Some proxies were not removed:\n\n{}", errors.join("\n")))
    }
}

/// Asks for an executable and adds it to the list.
pub fn add(hwnd: HWND) -> Result<(), String> {
    load()?;
    let Some(exe) = win::pick_exe(hwnd) else { return Ok(()) };
    add_exe(hwnd, &exe, None).map(|_| ())
}

/// Adds a running program (its loaded XInput DLL is known), then offers to install
/// the proxy right away.
pub fn add_running(hwnd: HWND, exe: &Path, dll: &'static str) -> Result<(), String> {
    let (cfg, g) = add_exe(hwnd, exe, Some(dll))?;
    let game = &cfg.games[g];
    if install(hwnd, game, &cfg.default_players)? {
        win::info(
            hwnd,
            &format!("Proxy installed. Restart {} so that it loads the proxy.", game.name),
        );
    }
    Ok(())
}

/// Adds an executable to the list and returns the saved config and the new index.
/// `dll` is the XInput DLL seen loaded in the running game, when known.
fn add_exe(hwnd: HWND, exe: &Path, dll: Option<&'static str>) -> Result<(AppConfig, usize), String> {
    let mut cfg = load()?;
    let exe_str = exe.to_string_lossy().into_owned();
    if cfg.games.iter().any(|g| g.exe.eq_ignore_ascii_case(&exe_str)) {
        return Err(format!("{exe_str} is already in the list."));
    }
    let bytes = std::fs::read(exe).map_err(|e| format!("Cannot read {exe_str}: {e}"))?;
    let found = pe::detect(&bytes).map_err(|e| format!("Cannot add {exe_str}: {e}."))?;
    let name = exe
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| exe_str.clone());
    let unsure = dll.is_none()
        && match found.source {
            DllSource::ImportTable | DllSource::DelayImport => false,
            DllSource::StringScan => !found.others.is_empty(),
            DllSource::Default => true,
        };
    let game = Game {
        name,
        exe: exe_str,
        arch: found.arch.as_str().into(),
        dll: dll.unwrap_or(found.dll).into(),
        players: None,
        log: false,
    };
    let note = unsure.then(|| {
        format!(
            "{} does not show clearly which XInput DLL it uses, so {} was chosen. If the order has no effect in \
             the game, use Add running game while the game is open, or change \"dll\" for this game in {} \
             (Open config folder) to one of {}.",
            game.name,
            game.dll,
            APP_CONFIG_FILE,
            XINPUT_DLLS.join(", ")
        )
    });
    cfg.games.push(game);
    save(&cfg)?;
    if let Some(note) = note {
        win::info(hwnd, &note);
    }
    let g = cfg.games.len() - 1;
    Ok((cfg, g))
}

/// Stores a new order for the given games, or as the default order when `games` is
/// empty, and updates the config of every installed proxy it affects.
pub fn set_order(cfg: &mut AppConfig, games: &[usize], rules: [Rule; SLOTS]) -> Result<(), String> {
    if games.is_empty() {
        cfg.default_players = rules;
    }
    for &g in games {
        if let Some(game) = cfg.games.get_mut(g) {
            game.players = Some(rules);
        }
    }
    save(cfg)?;
    let affected = |i: usize, g: &Game| {
        if games.is_empty() {
            g.players.is_none()
        } else {
            games.contains(&i)
        }
    };
    for (i, game) in cfg.games.iter().enumerate() {
        if affected(i, game) && status(game).is_ours() {
            write_proxy_config(game, &cfg.default_players)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_game(name: &str) -> (PathBuf, Game) {
        let dir = std::env::temp_dir().join(format!("{PROJECT}-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("game.exe");
        std::fs::write(&exe, b"MZ not a real game").unwrap();
        let game = Game {
            name: "game".into(),
            exe: exe.to_string_lossy().into_owned(),
            arch: "x86".into(),
            dll: "xinput1_3.dll".into(),
            players: None,
            log: false,
        };
        (dir, game)
    }

    fn listing(dir: &Path) -> Vec<(String, Vec<u8>)> {
        let mut v: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap())
            .map(|e| {
                (
                    e.file_name().to_string_lossy().into_owned(),
                    std::fs::read(e.path()).unwrap(),
                )
            })
            .collect();
        v.sort();
        v
    }

    #[test]
    fn install_and_remove_restore_the_folder() {
        let (dir, game) = test_game("plain");
        let before = listing(&dir);
        assert!(status(&game) == Status::NotInstalled);
        install_files(&game, &plan_install(&game).unwrap(), &[Rule::Auto; SLOTS]).unwrap();
        assert!(status(&game) == Status::Installed);
        assert!(dir.join(PROXY_CONFIG_FILE).exists());
        let dll = std::fs::read(dir.join("xinput1_3.dll")).unwrap();
        assert_eq!(pe::parse(&dll).unwrap().arch, Arch::X86);
        remove_proxy(&game).unwrap();
        assert_eq!(listing(&dir), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn foreign_dll_is_backed_up_and_restored() {
        let (dir, game) = test_game("foreign");
        std::fs::write(dir.join("xinput1_3.dll"), b"someone else's dll").unwrap();
        let before = listing(&dir);
        assert!(status(&game) == Status::Foreign);
        install_files(&game, &plan_install(&game).unwrap(), &[Rule::Auto; SLOTS]).unwrap();
        assert!(status(&game) == Status::Installed);
        assert_eq!(
            std::fs::read(dir.join("xinput1_3.dll.cps-backup")).unwrap(),
            b"someone else's dll"
        );
        // Removing a foreign DLL is refused; removing our proxy restores the original.
        remove_proxy(&game).unwrap();
        assert_eq!(listing(&dir), before);
        assert!(remove_proxy(&game).is_err());
        assert_eq!(listing(&dir), before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn older_proxy_is_outdated_and_updated() {
        let (dir, game) = test_game("outdated");
        // Another version of the proxy: has the marker but different bytes.
        let mut old = cps_core::PROXY_MARKER.to_vec();
        old.extend_from_slice(b" older build");
        std::fs::write(dir.join("xinput1_3.dll"), &old).unwrap();
        assert_eq!(status(&game), Status::Outdated);
        install_files(&game, &plan_install(&game).unwrap(), &[Rule::Auto; SLOTS]).unwrap();
        assert_eq!(status(&game), Status::Installed);
        assert!(
            !dir.join("xinput1_3.dll.cps-backup").exists(),
            "our own DLL is not backed up"
        );
        let cfg = AppConfig {
            games: vec![game.clone()],
            ..Default::default()
        };
        assert_eq!(remove_all(&cfg), Ok(1));
        assert_eq!(status(&game), Status::NotInstalled);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn existing_backup_blocks_install() {
        let (dir, game) = test_game("backup");
        std::fs::write(dir.join("xinput1_3.dll"), b"foreign").unwrap();
        std::fs::write(dir.join("xinput1_3.dll.cps-backup"), b"older backup").unwrap();
        assert!(plan_install(&game).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_dll_name_is_reported() {
        let (dir, mut game) = test_game("badname");
        game.dll = "dinput8.dll".into();
        assert!(plan_install(&game).unwrap_err().contains("\"dll\""));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
