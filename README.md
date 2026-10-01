# controller-port-switcher

Choose which XInput controller is player 1, 2, 3 or 4 in a given game, without
unplugging anything, without a driver and without administrator rights.

Windows gives XInput slots in the order controllers were connected, and there is no
API to change it. Some games insist that player 1 is the first slot, so an arcade
stick connected after a gamepad ends up as player 2. controller-port-switcher puts a
small XInput proxy DLL next to the game's executable; the proxy swaps the slots
before calling the real XInput from System32. The order can be changed from the
notification area while the game runs.

It is a single small executable with a notification area icon: no window, no
background polling, no network access, no telemetry.

## Features

- **Tooltip** listing the four ports with the model name of each controller
  ("Port 2 Real Arcade Pro.4") and the battery level of wireless ones, updated when
  a device is plugged in or removed.
- **Menu** that opens on the icon and shows the ports as the running game sees
  them. "Move to port N" swaps two controllers; the change applies within a second,
  without restarting the game. **Swap ports 1 and 2** does the most common change
  in one click.
- **Keyboard shortcut** for "Swap ports 1 and 2", useful in full-screen games. There
  is none by default: pick one from the menu (**Swap shortcut** > **Set shortcut...**).
- **Order remembered per game**, with a default order for games that have none.
- **Running game detection** when the menu opens: the first line tells whether the
  proxy is installed and loaded by the game.
- **Add running game...** lists programs that have XInput loaded, which also tells
  which XInput DLL the game uses.
- **Identify controllers**: press a button and the tooltip tells which port reacted.
  **Vibrate** on a port shakes that controller for a moment (if it has motors).
- **Install and remove the proxy** per game. An existing DLL with the same name is
  never replaced without confirmation; it is backed up and restored on removal.
  A proxy left by another version of the app shows as **outdated**, with **Update
  proxy**. **Remove all proxies** cleans every game folder at once.
- **Enable proxy log** per game, to find out why a game ignores the order.
- **Start with Windows**, on after the first launch; turn it off in the menu.
- Controllers are recognized by vendor and product id, so a controller stays on its
  port even if Windows assigns slots in another order next time.

The menu looks like this:

```
Running: MyGame
Swap ports 1 and 2
Port 1: Real Arcade Pro.4 (0F0D:008C)   >  Move to port 2 / 3 / 4, Vibrate
Port 2: Xbox Controller (045E:02FF)     >  Move to port 1 / 3 / 4, Vibrate
Port 3: Empty
Port 4: Empty
-----------------------------------------
Identify controllers
Games                                   >  one submenu per game, Remove all proxies
Add game...
Add running game...
-----------------------------------------
Swap shortcut: none                     >  Set shortcut..., Clear shortcut
Start with Windows
Open config folder
Quit
```

## Requirements

- Windows 10 or 11, x64. Games can be 64-bit or 32-bit.
- Controllers that work in XInput mode (Xbox controllers, most arcade sticks and
  pads with an XInput switch). DirectInput-only devices are not supported.

## Download

Each [release](../../releases) has `controller-port-switcher.exe` (the app, with the
proxy DLLs inside). The executable is not code-signed, so Windows SmartScreen may
warn on first launch ("More info", then "Run anyway"). You can also build it
yourself.

## Build

You need:

- [Rust](https://rustup.rs/) (stable). `rust-toolchain.toml` makes rustup install
  both targets used here, `x86_64-pc-windows-msvc` and `i686-pc-windows-msvc`.
- Visual Studio Build Tools with the "Desktop development with C++" workload
  (MSVC linker and the Windows SDK, which provides `rc.exe`).

```bash
cargo build --release
```

This produces `target\release\controller-port-switcher.exe`, which embeds the six
proxy DLLs (three names, x64 and x86), and `target\release\xinput-probe.exe` (a test
tool, not included in releases). The DLLs themselves are built by the app's build script into `target\proxy\`.

Run the tests:

```bash
cargo test --workspace
```

Pushing a tag such as `v0.2.0` makes the release workflow build the executables and
attach them to a GitHub release.

## Run

```bash
cargo run --release -p controller-port-switcher
```

Or start `target\release\controller-port-switcher.exe` directly; it can be copied
anywhere, nothing else is needed. On Windows 11 the icon may first appear in the
hidden icons area (the ^ arrow); drag it to the taskbar to keep it visible.

The first launch turns on **Start with Windows** (a per-user entry under
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, no administrator rights).
Uncheck it in the menu to turn it off; the choice is remembered. If the executable
is moved, the entry follows it at the next launch. Disabling the app in the Startup
apps page of Task Manager also works and is respected.

## Usage

1. **Add game...** (pick the executable) or, while the game is running,
   **Add running game...**.
2. Open **Games**, the game, **Install proxy**. Restart the game if it was running.
3. While the game runs, click the icon and use **Swap ports 1 and 2** or **Move to
   port N** on a controller, or press your swap shortcut.

When no listed game is running, "Move to port" edits the default order, used by
every game that has no order of its own. **Use default order** in a game's submenu
makes it follow the default order again.

## Configuration

Everything is stored in `%APPDATA%\controller-port-switcher\config.json`
(**Open config folder** in the menu). It is read each time the menu opens, so
edits apply without restarting the app.

```json
{
  "default_players": [ { "rule": "auto" }, { "rule": "auto" }, { "rule": "auto" }, { "rule": "auto" } ],
  "swap_hotkey": "Ctrl+Alt+S",
  "start_with_windows": true,
  "games": [
    {
      "name": "MyGame",
      "exe": "D:\\Games\\MyGame\\MyGame.exe",
      "arch": "x64",
      "dll": "xinput1_3.dll",
      "players": [
        { "rule": "device", "device": "0F0D:008C", "instance": 0, "slot": 1 },
        { "rule": "device", "device": "045E:02FF", "instance": 0, "slot": 0 },
        { "rule": "auto" },
        { "rule": "auto" }
      ],
      "log": false
    }
  ]
}
```

- `dll`: the XInput DLL the proxy is installed as. It is detected from the
  executable's import table (or from the running process); if the order has no
  effect, set it to `xinput1_4.dll`, `xinput1_3.dll` or `xinput9_1_0.dll`.
- `players`: one rule per player. `auto` takes the next free controller in
  Windows order, `device` targets a controller by vendor and product id
  (`instance` picks among identical models), `slot` a fixed XInput slot, `none`
  reports no controller. Without `players`, the game uses `default_players`.
- `swap_hotkey`: shortcut for "Swap ports 1 and 2" (absent by default). Set it
  from the menu; if you edit it here, restart the app. Modifiers are Ctrl, Alt,
  Shift and Win; F1-F24 may be used alone.
- `start_with_windows`: mirrors the menu entry.
- `log`: **Enable proxy log** in the menu. When true, the proxy writes `controller-port-switcher-proxy.log` next to
  the game when its config is reloaded or the mapping changes.

Installing the proxy writes two files next to the game executable: the DLL and
`controller-port-switcher.json`, which the proxy reads. Removing the proxy deletes
both (and the log) and restores a backed-up DLL if there was one.

## Testing without a game

`xinput-probe` loads XInput by name, so a proxy DLL placed in its folder is used
instead of the system one. It prints connection, type, vendor and product id and
buttons for XInput indices 0-3.

```bash
xinput-probe --dll xinput1_4.dll --watch 30
```

Options: `--dll NAME`, `--watch [SECONDS]` (print again when something changes),
`--rumble INDEX` (vibrate for one second). To test the proxy, copy
`target\proxy\x86_64-pc-windows-msvc\release\xinput1_4.dll` next to
`xinput-probe.exe`, or add the probe as a game in the app and install the proxy.

`controller-port-switcher.exe --status` prints the tooltip text to the console and
exits without creating an icon:

```
controller-port-switcher
Port 1 Xbox Controller
Port 2 Real Arcade Pro.4
Port 3 Empty
Port 4 Empty
```

From PowerShell, pipe it (`controller-port-switcher.exe --status | Out-String`) so
the shell waits for the output of this GUI program.

## Footprint

Measured on Windows 11 with Rust 1.98.1, release build:

| Item | Size |
|---|---|
| `controller-port-switcher.exe` (six DLLs embedded) | 522 KB |
| `xinput-probe.exe` | 244 KB |
| Proxy DLLs, x64 | 16.5 to 18.5 KB |
| Proxy DLLs, x86 | 19 to 20 KB |

At rest the app used 0 ms of CPU time over 60 seconds (0 %), a working set of
4.8 MB or less, and about 3 MB of private memory (measured with `Get-Process`).
The tooltip is refreshed on device notifications and when the pointer hovers the
icon; the only timer that polls is the short one used by **Identify controllers**.
The shortcut uses `RegisterHotKey`; a keyboard hook is installed only while
**Set shortcut...** waits for a key (10 seconds at most). The proxy DLLs depend only on `kernel32.dll` and
link no C runtime.

## Known limitations

- **Anti-cheat**: games protected by an anti-cheat system may refuse to start or
  flag a DLL placed in their folder. Do not use the proxy with online competitive
  games.
- **Microsoft Store (UWP) games** cannot be modified this way.
- **Other input APIs**: games that use Windows.Gaming.Input, GameInput, DirectInput
  or raw HID instead of XInput are not affected. The first line of the menu shows
  "proxy not loaded" in that case.
- **XInput loaded from elsewhere**: games that load XInput by full path from
  System32, or that restrict DLL search to System32, bypass the proxy. So do
  launchers whose real game executable is in another folder: add the real one.
- **Steam Input** creates its own virtual controllers inside the game process; the
  interaction with the proxy has not been tested. Steam's own controller order
  setting is an alternative for Steam games.
- **Battery level** is shown only for wireless controllers that report it through
  XInput; it has not been checked with a wireless controller yet.
- **Identical controllers** (same vendor and product id) have the same name; use
  **Identify controllers** to tell them apart. Their order among themselves
  follows Windows.
- The order only changes inside games where the proxy is installed. Windows and
  other programs keep the connection order.
- The tray icon may be in the Windows 11 hidden icons area until moved.

More details on the proxy, device names and design choices are in
[docs/how-it-works.md](docs/how-it-works.md).

## Project structure

```
crates/core     Shared logic, no hardware access: mapping rules, config formats,
                PE import analysis, device name choice, tooltip text. Unit tests.
crates/proxy    XInput proxy (no_std), separate Cargo workspace:
  xinput1_3/      thin crates that build it as xinput1_3.dll, xinput1_4.dll and
  xinput1_4/      xinput9_1_0.dll, with the same exports and ordinals as the
  xinput9_1_0/    system DLLs (exports.def)
crates/probe    xinput-probe test tool
crates/app      Notification area app; build.rs builds and embeds the DLLs
assets          Icons (light and dark taskbar), manifest, resource script
.github         CI (format, clippy, tests, build) and release workflows
```

## License

MIT, see [LICENSE](LICENSE).
