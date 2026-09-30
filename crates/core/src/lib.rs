//! Shared logic for controller-port-switcher. Nothing here touches hardware or Win32,
//! so everything is unit-testable. Without the default `std` feature only the parts the
//! proxy DLL needs are built, with no allocation.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "std")]
pub mod devname;
pub mod mapping;
#[cfg(feature = "std")]
pub mod pe;
pub mod proxycfg;
#[cfg(feature = "std")]
pub mod tooltip;

#[cfg(feature = "serde")]
pub mod appcfg;

/// Project name, used for the tooltip title, the config folder and file names.
pub const PROJECT: &str = "controller-port-switcher";

/// Name of the per-game configuration file written next to the proxy DLL.
pub const PROXY_CONFIG_FILE: &str = "controller-port-switcher.json";

/// Name of the optional proxy log file (only written when `"log": true`).
pub const PROXY_LOG_FILE: &str = "controller-port-switcher-proxy.log";

/// Byte string embedded in every proxy DLL so the app can recognize its own DLL
/// without adding an export the system XInput does not have.
pub const PROXY_MARKER: &[u8] = b"controller-port-switcher proxy marker v1";

/// XInput DLL names the proxy can stand in for.
pub const XINPUT_DLLS: [&str; 3] = ["xinput1_4.dll", "xinput1_3.dll", "xinput9_1_0.dll"];
