# How it works

## Why a proxy DLL

XInput assigns user indices 0-3 in connection order, and Windows offers no API to
reassign them. Instead of a driver or virtual controllers, controller-port-switcher
changes what a single game sees: it places a DLL named like the XInput DLL the game
uses (`xinput1_4.dll`, `xinput1_3.dll` or `xinput9_1_0.dll`) in the game's folder.

When a program loads a DLL by name, Windows searches the program's folder before
System32, unless the DLL is listed under the `KnownDLLs` registry key. The XInput
DLLs are not listed there. This was checked on Windows 11 for the three names, for
64-bit and 32-bit programs, both for DLLs in the import table and for
`LoadLibrary("xinputX.dll")`. Programs that call `SetDefaultDllDirectories` or
`LoadLibraryEx` with `LOAD_LIBRARY_SEARCH_SYSTEM32`, or that load XInput by full
path, skip the game folder and are not affected.

## The proxy

`crates/proxy` holds the implementation; `xinput1_3`, `xinput1_4` and `xinput9_1_0`
are thin crates that export the subset of functions each system DLL has, with the
same names and ordinals (`exports.def`). Exports were compared with `dumpbin
/exports` against the system DLLs for both architectures:

| DLL | Named exports | By ordinal only |
|---|---|---|
| xinput1_4.dll | DllMain, XInputGetState, XInputSetState, XInputGetCapabilities, XInputEnable, XInputGetBatteryInformation, XInputGetKeystroke, XInputGetAudioDeviceIds | 100, 101, 102, 103, 104, 108, 109 |
| xinput1_3.dll | DllMain, XInputGetState, XInputSetState, XInputGetCapabilities, XInputEnable, XInputGetDSoundAudioDeviceGuids, XInputGetBatteryInformation, XInputGetKeystroke | 100, 101, 102, 103 |
| xinput9_1_0.dll | DllMain, XInputGetCapabilities, XInputGetDSoundAudioDeviceGuids, XInputGetState, XInputSetState | none |

The ordinal-only exports are undocumented: 100 XInputGetStateEx, 101
XInputWaitForGuideButton, 102 XInputCancelGuideButtonWait, 103
XInputPowerOffController, 104 XInputGetBaseBusInformation, 108
XInputGetCapabilitiesEx. Their argument counts were read from the 32-bit system DLL
(bytes popped by `ret`), and each one checks a user index argument, including 109,
whose name is unknown. Every function that takes a user index is remapped;
`XInputEnable` passes through. For `XInputGetKeystroke` with `XUSER_INDEX_ANY`, the
slot reported in the keystroke is translated back to the player index, and
keystrokes from slots no player reads are dropped.

Rules the proxy follows:

- **Loading**: the real XInput with the same file name is loaded from System32 by
  full path, on the first call, never in `DllMain` (the loader lock is held there).
  If it is missing (xinput1_3 ships with the DirectX runtime), `xinput1_4.dll` is
  used.
- **Reloading**: at most once per second, inside a call, the proxy checks the date
  of `controller-port-switcher.json` next to it. It creates no thread. A flag, not a
  lock, makes sure only one thread refreshes; the others keep going.
- **Hot path**: a state call does one atomic load of a packed table (one byte per
  player), then calls the real function. No allocation, no lock.
- **Failure**: a missing or invalid config means pass-through. The config parser is
  written without allocation. The DLL is `no_std`, links no C runtime (except
  `memcpy` and `memcmp` from the static vcruntime library) and has no entry point
  (`/NOENTRY`), so there is nothing to initialize and nothing to fail at load time.
- **Log**: off by default, enabled with `"log": true`, written only on reload or
  mapping change.

The proxy crates form a separate Cargo workspace because the app enables the `std`
feature of `cps-core`, and Cargo would otherwise unify that feature into the
`no_std` DLL build. The app's `build.rs` builds the proxy workspace for
`x86_64-pc-windows-msvc` and `i686-pc-windows-msvc` in release mode and embeds the
six DLLs with `include_bytes!`, so the app stays a single file.

## Recognizing controllers

XInput reports only a subtype per slot (both reference devices, an Xbox controller
and a Hori Real Arcade Pro.4, report "Gamepad"). The undocumented
`XInputGetCapabilitiesEx` (ordinal 108 of `xinput1_4.dll`, also used by SDL) adds
the USB vendor and product id, which is enough to tell different models apart.

A rule such as `{ "rule": "device", "device": "0F0D:008C", "instance": 0 }` is
resolved by the proxy once per second: it finds the slot where that device is
connected (the `instance`-th one with that id, in slot order) and maps the player
to it. Resolution order: device rules, then fixed slots, then `auto` players take
the remaining slots in ascending order. A slot is never given to two players.
"Move to port" writes the current order out explicitly and swaps two entries, so
the result does not depend on `auto`.

Limit: identical controllers share an id; among themselves they follow the order
Windows gives them. When the system does not report ids, device rules fall back to
the slot the device had when the rule was made.

## Controller names

XInput provides no name either. The app looks for the device node whose instance id
contains the XInput interface marker `IG_` and the vendor and product id (for
example `HID\VID_045E&PID_02FF&IG_00\...`), then walks up its parents while they
share the vendor id, which reaches the physical USB or Bluetooth device. The name
is, in order of preference:

1. the product string reported by the device (`DEVPKEY_Device_BusReportedDeviceDesc`),
   for example "Real Arcade Pro.4";
2. the name Windows shows (`DEVPKEY_Device_FriendlyName`), for example
   "Xbox Controller".

Generic strings ("Controller", "USB Input Device", "HID-compliant game controller")
and driver names ("Xbox 360 Controller for Windows") are skipped. This uses
CfgMgr32 and works without administrator rights. Names are cached per id and read
again after a device change.

## Refreshing without polling

The app has a hidden top-level window (a message-only window would not receive the
`TaskbarCreated` broadcast sent when Explorer restarts). It registers for device
interface notifications on the HID and XUSB classes. After an arrival or removal it
refreshes the tooltip twice, after 300 ms and 1.5 s, because XInput may take a
moment to see the change. It also refreshes when the pointer hovers the icon (at
most every 500 ms). The only repeating timer is the 50 ms one used by **Identify
controllers**, stopped after a button press or 15 seconds.

## Running games

When the menu opens, the app lists processes with
`QueryFullProcessImageNameW` (limited query rights, no administrator rights) and
compares their paths with the games in the list. For a matching process it lists
loaded modules with a Toolhelp snapshot to see whether the proxy DLL from the game
folder is loaded. Protected processes cannot be inspected; the menu then says
"proxy state unknown". **Add running game...** uses the same snapshot to find
programs outside the Windows folder that have an XInput DLL loaded.

## Choosing the DLL name for a new game

For **Add game...**, the executable's PE headers give the architecture, and the
import table, then the delay-load import table, give the XInput DLL. Games that
load XInput with `LoadLibrary` have it in neither; the file is then searched for the
DLL names as strings, and `xinput1_3.dll` is used if nothing is found (the app says
so). **Add running game...** avoids the guess by reading the DLL the game actually
loaded.

## Shortcut, updates and startup

- **Swap shortcut**: registered with `RegisterHotKey`, so nothing runs until it is
  pressed. **Set shortcut...** installs a low-level keyboard hook only until the
  next key (or 10 seconds), reads the modifiers held with it, and removes the hook.
  A letter or digit needs a modifier; F1-F24, Pause and Scroll Lock may be used
  alone. If another program already owns the combination, Windows refuses it and
  the app says so.
- **Outdated proxies**: every proxy DLL contains a marker string. A DLL with the
  marker whose bytes differ from the DLL embedded in the running app is from
  another version; **Update proxy** replaces it without a backup, since it is ours.
- **Start with Windows**: a `REG_SZ` value named `controller-port-switcher` under
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`. The first launch creates it
  and records `"start_with_windows": true` in config.json; unchecking the menu entry
  deletes it and records `false`, which later launches respect.
- **Battery**: `XInputGetBatteryInformation` for the gamepad; wired and unknown
  battery types are not shown.
