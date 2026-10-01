# CLAUDE.md

Durable context for working on controller-port-switcher. Current state and next steps are in HANDOFF.md.

## Goal

Windows assigns XInput slots 0-3 in connection order and has no API to reassign them. This tool lets one technical user choose, per game, which physical controller appears as player 1-4, without unplugging, without a driver and without administrator rights.

Mechanism: a proxy DLL named like the XInput DLL the game uses (`xinput1_4.dll`, `xinput1_3.dll` or `xinput9_1_0.dll`) is placed next to the game executable. It remaps user indices, then calls the real XInput from System32. A notification area app (no window) manages games and the order.

Reference hardware: an Xbox controller (XInput reports 045E:02FF; the USB device is 045E:0B12) and a Hori Real Arcade Pro.4 arcade stick (0F0D:008C). Both report XInput subtype Gamepad (1), so rules cannot rely on subtype.

Repository: https://github.com/glitch191/controller-port-switcher (public, MIT, default branch `main`).

## Rules from the project owner

- All project files, UI text, comments, docs and commit messages are in English. Conversation with the owner is in French.
- Never use the em dash (U+2014) or en dash (U+2013) anywhere (files, UI, logs, commits, and chat replies). Use commas, colons, parentheses or "-".
- No emoji or icon evoking AI (sparkles, wand, robot, brain, crystal ball, `auto_awesome`, etc.) and no mention of AI anywhere. The software has no AI feature.
- Keep it simple and light; no feature that was not requested. Ask before changing scope.
- Never modify a real game's folder without the owner's explicit confirmation. Use `target\game-test` (xinput-probe as a stand-in game).
- For hardware tests needing a physical action or observation, ask for one action at a time and wait.
- Check before finishing: search for `[\u2013\u2014]`, non-ASCII characters and AI icon names in all files outside `target`.

## Architecture

```
Cargo.toml          Root workspace: crates/core, crates/probe, crates/app. Excludes crates/proxy.
rust-toolchain.toml stable + targets x86_64-pc-windows-msvc, i686-pc-windows-msvc + rustfmt, clippy
.cargo/config.toml  +crt-static for both targets
rustfmt.toml        max_width = 120
crates/core         cps-core: mapping rules and resolution, no-alloc per-game config parser (proxycfg),
                    app config with serde (appcfg, feature "serde"), PE analysis (pe), device name choice
                    (devname), hotkey text format (hotkey), tooltip and menu text (tooltip). std parts behind
                    feature "std" (default); the proxy uses it with default-features = false.
crates/proxy        SEPARATE Cargo workspace: xinput-proxy (no_std rlib) + thin cdylib crates
                    xinput1_3/, xinput1_4/, xinput9_1_0/ (each: exports.def + build.rs + export! macro call).
crates/probe        xinput-probe test tool (--dll, --watch [s], --rumble N).
crates/app          Tray app (windows subsystem). build.rs runs a nested cargo build of the proxy workspace
                    for both targets (always --release, into target/proxy) and embeds the 6 DLLs with
                    include_bytes!; it also compiles assets/app.rc (icons + manifest) with embed-resource.
assets              icon-light.ico (dark glyph, light taskbar; exe icon), icon-dark.ico, app.manifest, app.rc
.github/workflows   ci.yml (fmt, clippy -D warnings, test, build, artifacts), release.yml (tag v* -> release)
docs/how-it-works.md Technical details (read it before changing the proxy).
```

App modules (`crates/app/src`): `main.rs` (window, tray events, identify, vibrate, shortcut capture, autostart sync), `menu.rs` (menu build and commands, running game context, move/swap), `games.rs` (config IO, install/remove, status), `procs.rs` (process and module inspection), `names.rs` (model names via CfgMgr32), `xinput.rs` (system XInput by full path), `tray.rs`, `shortcut.rs`, `autostart.rs`, `win.rs`.

## Commands

```bash
cargo build --release                      # app + probe; builds and embeds the proxy DLLs
cargo test --workspace                     # 51 tests (43 core, 8 app) at v1.0.0
cargo fmt --all --check                    # also run in crates/proxy
cargo clippy --release --workspace --all-targets -- -D warnings
cd crates/proxy && cargo clippy --release --workspace --target x86_64-pc-windows-msvc -- -D warnings
target\release\controller-port-switcher.exe --status | Out-String   # tooltip text, no icon (PowerShell needs the pipe)
target\release\xinput-probe.exe --dll xinput1_4.dll --watch 30
```

Release: push a tag `vX.Y.Z` on `main`; release.yml attaches `controller-port-switcher.exe` only; xinput-probe is a local test tool and is not published (owner's request; CI artifacts omit it too). Workspace version is in `Cargo.toml`, `crates/proxy/Cargo.toml` and `assets/app.manifest` (1.0.0 at the last release).

Verify proxy exports after touching the proxy: compare `dumpbin /exports` of `target/proxy/<triple>/release/<dll>` with `C:\Windows\System32` (x64) and `C:\Windows\SysWOW64` (x86). All six matched names and ordinals at v1.0.0. On the original machine dumpbin was at `C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Tools\MSVC\14.44.35207\bin\Hostx64\x64\dumpbin.exe`.

## Conventions

- Rust edition 2024, `windows-sys` 0.61 (raw Win32), `serde`/`serde_json` for the app config only, `embed-resource` as build dependency. No other dependencies.
- Release profile: opt-level "z", lto, codegen-units 1, panic abort, strip; static CRT.
- Code must pass fmt and clippy with `-D warnings` (CI enforces both).
- Work on a branch and open a PR to `main`; CI must be green before merging.
- Error messages say what to do and are shown in native message boxes, never in a console.

## Decisions and why

- **Per-game proxy DLL** rather than driver or virtual controllers: no admin rights, no driver, only affects one game. Verified: XInput DLLs are not in KnownDLLs, so a DLL in the game folder wins, for static imports and `LoadLibrary(name)`, x64 and x86.
- **Identity by VID/PID** from undocumented `XInputGetCapabilitiesEx` (ordinal 108 of xinput1_4, also used by SDL). Raw Input correlation was not needed. Identical models share an id; `instance` picks among them in slot order. Without ids, device rules fall back to their saved slot.
- **Resolution order**: device rules, then fixed slots, then `auto` players take remaining slots ascending; a slot is never assigned twice. "Move to port" writes the current order explicitly, then swaps two entries.
- **Proxy is no_std, /NOENTRY**, links only memcpy/memcmp from static `libvcruntime` (plus `kernel32` for x86); about 17-20 KB per DLL. With std it was about 200 KB.
- **Exports**: rustc exports every `#[no_mangle]` symbol of the whole crate graph, so the proxy functions are mangled and each thin crate defines only its own unmangled exports via `xinput_proxy::export!`; `exports.def` sets ordinals and NONAME.
- **Separate proxy workspace**: Cargo feature unification would turn on cps-core's `std` (enabled by the app) in the no_std DLL build (duplicate `panic_impl`).
- **Embedded DLLs always built in release**: unoptimized no_std builds pull unwinding code from `core` that needs the full CRT. Debug builds of the thin crates link `libucrt` only so `cargo build` works there.
- **Proxy rules**: real XInput loaded by full path from System32 lazily (never in DllMain); config checked at most once per second inside a call, no thread; mapping in one AtomicU32; refresh guarded by an AtomicBool, not a lock; pass-through on any error; log off by default.
- **Undocumented ordinals** 100-104, 108, 109 are remapped. Argument counts were read from the x86 system DLL (`ret N`); ordinal 109's name is unknown (called `XInputOrdinal109`).
- **Tray app**: hidden top-level window (a message-only window would not get `TaskbarCreated`); tooltip refreshed on device-interface notifications (HID and XUSB) with one-shot timers at 300 ms and 1.5 s, and on hover (at most every 500 ms); only Identify polls (50 ms, 15 s max). The working set is trimmed after startup and menu actions.
- **Per-game order + default order**: a running listed game (detected when the menu opens or the shortcut is pressed) gets its own order; otherwise the default order changes. Owner-approved scope change (the original spec had per-game Player 1-4 submenus).
- **Running proxy detection reads module memory** for the proxy marker, because Windows keeps a loaded module's original path after the file is renamed.
- **Loaded DLLs are renamed, not overwritten**: updating or removing a proxy the game holds moves it to `<dll>.cps-old`, deleted later (next install/remove or next menu opening). An identical proxy is left in place.
- **Model names** from CfgMgr32: device-reported product string, then Windows friendly name, skipping generic and driver names; cached, refreshed after device changes.
- **Swap shortcut**: no default (owner's request); RegisterHotKey; a low-level keyboard hook exists only during capture (10 s max).
- **Start with Windows**: HKCU Run value, enabled at first launch (owner's request), remembered when turned off; the path is updated at each launch.
- **Tooltip ports are numbered 1-4** (like players); menu labels add the VID:PID.

## Out of scope (from the original spec)

No main window, web UI, WebView; no live input display; no custom controller labels; no virtual controllers, drivers, ViGEmBus or HidHide; no device disabling; no button remapping, macros or vibration tuning; no DirectInput; no injection into running processes; no automatic game launch detection (the menu checks running games only when opened); no UWP support; no anti-cheat bypass; no auto-update, installer or online sync. Start with Windows was added later at the owner's request.

## Environment pitfalls

- The Claude desktop app is MSIX-packaged: processes launched from the agent's shell see a virtualized `%APPDATA%` (writes go to `%LOCALAPPDATA%\Packages\Claude_*\LocalCache\Roaming`). To test with the real profile, start the app through Explorer: `Start-Process explorer.exe -ArgumentList <path to exe>`. Uncertain: whether HKCU registry writes are also virtualized for those processes.
- `FindWindow` on the app's class returned 0 from the agent's shell; enumerate windows by process id instead. Close the app cleanly by posting WM_CLOSE (0x10) to its hidden window (class `controller-port-switcher`), never by killing it (ghost icon).
- The running app locks `target\release\controller-port-switcher.exe`: close it before `cargo build --release`.
- Rust tests run in parallel in one process: tests that load DLLs must not assume an exact module list.
- The PowerShell tool reports native programs' stderr as errors even on success (git, gh). `gh` was not on PATH in the agent's shell; it was at `C:\Program Files\GitHub CLI\gh.exe`.
- Editing Rust through bash heredocs with embedded Python broke string escapes and line continuations once; prefer the Edit/Write tools for Rust strings.
