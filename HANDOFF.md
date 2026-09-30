# HANDOFF

State at handoff (2026-09-30). Durable context is in CLAUDE.md.

## Done

- **v1.0.0 released**: https://github.com/glitch191/controller-port-switcher/releases/tag/v1.0.0 (tag on `main` at `d8de31f`, assets `controller-port-switcher.exe` and `xinput-probe.exe`, CI and release workflows green).
- Features: tooltip with model names and battery; menu with running-game header, "Swap ports 1 and 2", "Move to port N", "Vibrate"; per-game and default order; Identify; Add game / Add running game; install, update, remove proxy (backup and restore of foreign DLLs, works while the game holds the DLL); Remove all proxies; Enable proxy log; swap shortcut (none by default); Start with Windows; `--status`; xinput-probe.
- Docs: README.md, docs/how-it-works.md, LICENSE (MIT, holder "glitch191").
- PR #1 merged: message text fixes and games that keep the proxy loaded.

## Test status at v1.0.0

Automated with the reference hardware connected:
- The six proxy DLLs export the same names and ordinals as the system DLLs (x64 and x86).
- DLL search order: the local DLL wins for all three names, static import and LoadLibrary, x64 and x86.
- xinput-probe through the proxy: pass-through, device swap, "None", invalid config falls back to pass-through, live reload within a second, x64 and x86.
- Swap applied live in the running probe from the menu and from the shortcut; order kept while the Xbox controller was unplugged and replugged.
- Install and remove in `target\game-test`: foreign DLL backed up and restored byte for byte; refusal leaves the folder unchanged.
- Idle footprint measured: 0 ms CPU over 60 s, working set 4.8 MB or less, about 3 MB private; exe 522 KB, DLLs 16.5-20 KB.

Confirmed by the owner (physical action or observation): tooltip and names; menu; Identify for each device; install confirmation (No, then Yes); "proxy not loaded" warning; Swap; rumble through the remapped index reached the Xbox controller; Vibrate; tooltip update on unplug and replug; shortcut capture and use; log message; remove proxy; Remove from list; Quit leaves no ghost icon; Start with Windows after a real reboot.

Not tested:
- Battery display: no wireless controller available.
- Real inversion of the plug order (arcade stick in slot 0): only covered by unit tests; the replug put the Xbox controller back in slot 0.
- Any real game: only xinput-probe was used. Anti-cheat, Steam Input, games restricting DLL search to System32 and launchers are untested.
- xinput1_3 and xinput9_1_0 proxies with the menu flow (they loaded and passed through in the early feasibility check only).
- Tooltip readability on a light taskbar and after DPI changes: not observed.

## In progress

Nothing uncommitted. The working tree on `main` is clean.

## Next steps (priority order)

1. **Tooltip line for the running game** (agreed with the owner, not started): when a listed game runs, add a line such as "In xinput-probe: 1 Real Arcade Pro.4, 2 Xbox Controller", because the tooltip shows the Windows order while the game sees another. Likely files: `crates/app/src/main.rs` (status_text, refresh on hover), `crates/app/src/menu.rs` (running_games/context would need to be shared), `crates/core/src/tooltip.rs` (text, truncation, tests). Open points: cost of a process scan on hover (not measured; hover refresh is throttled to 500 ms), wording with several running games, 127-character limit. Work on a branch, PR, then release 1.1.0.
2. **Release notes for v1.0.0**: the auto-generated notes only mention PR #1. Offered to write a feature summary; no answer yet.
3. **.gitattributes** (for example `* text=auto eol=lf`) to stop LF/CRLF warnings: offered, no answer yet.
4. **Test with a real game**: ask the owner for a game path and confirmation before touching its folder.
5. Local cleanup: delete the merged local branch `fix/message-line-breaks` and prune the stale remote-tracking ref (`git fetch --prune`); remove `target\game-test` when no longer needed.

## Pitfalls met (and fixed)

- rustc exported every `#[no_mangle]` function into each DLL: solved with mangled functions plus the per-crate `export!` macro.
- Workspace feature unification broke the no_std proxy: moved it to its own workspace.
- Debug no_std DLLs needed the full CRT: embedded DLLs are always release builds.
- `cargo clippy` in the root leaked the clippy wrapper into the nested proxy build: the build script removes `RUSTC_WRAPPER`/`RUSTC_WORKSPACE_WRAPPER`.
- Broken string continuations from scripted edits produced wide gaps in dialogs (fixed in PR #1).
- Windows keeps a loaded module's original path after rename: detection now reads module memory.
- A loaded DLL cannot be overwritten: rename to `.cps-old` instead.
- Parallel tests shared the process and saw each other's DLLs: CI failed once; the assertion was relaxed.
- MSIX virtualization of `%APPDATA%` made the app launched by the agent and the app launched by the owner use different configs. This looked like a shortcut bug; it was not.
- The owner's double press of the shortcut looked like "nothing happens", because the tooltip shows the Windows order, not the game's.

## Open questions

- License holder: "glitch191" was used; offered to change it to the owner's real name or another license. No answer.
- The v1.0.0 release notes and .gitattributes (see Next steps 2 and 3).
- Tooltip line design details (see Next step 1).

## Local machine state (original machine, may not apply elsewhere)

- The app runs from `target\release\controller-port-switcher.exe`, and the HKCU Run entry points to that development path. The owner should copy the exe to its final location and start it once there (the Run entry follows it).
- Real `%APPDATA%\controller-port-switcher\config.json`: exact content unverified (the agent's shell only sees a virtualized copy). Known from the owner's actions: swap shortcut set to Ctrl+Alt+S, `xinput-probe` removed from the list. Probably `start_with_windows: true` and an Auto or natural default order: not verified.
- `target\game-test` holds `xinput-probe.exe` and a copy of the system `xinput1_4.dll` (the "foreign DLL" for tests).
- Helper scripts used during the session (closing the app with WM_CLOSE, icon generator in pure Python) lived in the session scratchpad and will not migrate. The icons are committed in `assets/`; regenerating them would need a new script.
