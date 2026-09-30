//! Games list and mappings kept by the tray app in %APPDATA%\controller-port-switcher.

use crate::mapping::{DeviceId, Rule, SLOTS};
use crate::proxycfg::VERSION;
use serde::{Deserialize, Serialize};

pub const APP_CONFIG_FILE: &str = "config.json";

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct AppConfig {
    /// Order used by games that have no order of their own.
    #[serde(default)]
    pub default_players: [Rule; SLOTS],
    #[serde(default)]
    pub games: Vec<Game>,
    /// Shortcut for "Swap ports 1 and 2", for example "Ctrl+Alt+S". None by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swap_hotkey: Option<String>,
    /// Whether the app starts with Windows. None until the first launch sets it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_with_windows: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Game {
    /// Display name in the menu (the exe file name without extension).
    pub name: String,
    /// Full path of the game executable.
    pub exe: String,
    /// "x64" or "x86".
    pub arch: String,
    /// XInput DLL the proxy is installed as, for example "xinput1_3.dll".
    /// Detected when the game is added; edit it here if the guess is wrong.
    pub dll: String,
    /// Order remembered for this game; None means the default order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub players: Option<[Rule; SLOTS]>,
    /// Enables the proxy log file next to the DLL.
    #[serde(default)]
    pub log: bool,
}

impl Game {
    pub fn rules(&self, default: &[Rule; SLOTS]) -> [Rule; SLOTS] {
        self.players.unwrap_or(*default)
    }
}

/// JSON shape of one rule, shared with the proxy parser.
#[derive(Serialize, Deserialize)]
struct RuleRepr {
    rule: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    device: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    instance: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    slot: Option<u8>,
}

impl Serialize for Rule {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let (rule, device, instance, slot) = match *self {
            Rule::Auto => ("auto", None, None, None),
            Rule::None => ("none", None, None, None),
            Rule::Slot(n) => ("slot", None, None, Some(n)),
            Rule::Device { id, instance, slot } => ("device", Some(id.to_string()), Some(instance), Some(slot)),
        };
        RuleRepr {
            rule: rule.into(),
            device,
            instance,
            slot,
        }
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for Rule {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let r = RuleRepr::deserialize(d)?;
        let slot = r.slot.filter(|&s| (s as usize) < SLOTS);
        match r.rule.as_str() {
            "auto" => Ok(Rule::Auto),
            "none" => Ok(Rule::None),
            "slot" => slot
                .map(Rule::Slot)
                .ok_or_else(|| D::Error::custom("\"slot\" must be 0-3")),
            "device" => {
                let id = r
                    .device
                    .as_deref()
                    .and_then(|s| DeviceId::parse(s.as_bytes()))
                    .ok_or_else(|| D::Error::custom("\"device\" must look like \"045E:02FF\""))?;
                Ok(Rule::Device {
                    id,
                    instance: r.instance.unwrap_or(0),
                    slot: slot.unwrap_or(0),
                })
            }
            other => Err(D::Error::custom(format!(
                "unknown rule \"{other}\" (use auto, none, slot or device)"
            ))),
        }
    }
}

/// Text of the per-game file written next to the proxy DLL.
pub fn proxy_config_json(log: bool, players: &[Rule; SLOTS]) -> String {
    #[derive(Serialize)]
    struct Out<'a> {
        version: u32,
        log: bool,
        players: &'a [Rule; SLOTS],
    }
    let mut s = serde_json::to_string_pretty(&Out {
        version: VERSION,
        log,
        players,
    })
    .unwrap_or_default();
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_round_trip() {
        let rules = [
            Rule::Device {
                id: DeviceId {
                    vid: 0x0F0D,
                    pid: 0x008C,
                },
                instance: 0,
                slot: 1,
            },
            Rule::Slot(2),
            Rule::None,
            Rule::Auto,
        ];
        let json = serde_json::to_string(&rules).unwrap();
        assert!(json.contains("\"0F0D:008C\""));
        let back: [Rule; 4] = serde_json::from_str(&json).unwrap();
        assert_eq!(back, rules);
    }

    #[test]
    fn missing_fields_default() {
        let g: Game =
            serde_json::from_str(r#"{"name":"g","exe":"C:\\g.exe","arch":"x64","dll":"xinput1_3.dll"}"#).unwrap();
        assert_eq!(g.players, None);
        assert_eq!(g.rules(&[Rule::None; 4]), [Rule::None; 4]);
        assert!(!g.log);
        let empty: AppConfig = serde_json::from_str("{}").unwrap();
        assert!(empty.games.is_empty());
        assert_eq!(empty.default_players, [Rule::Auto; 4]);
    }

    #[test]
    fn bad_rule_is_an_error() {
        assert!(serde_json::from_str::<Rule>(r#"{"rule":"slot","slot":9}"#).is_err());
        assert!(serde_json::from_str::<Rule>(r#"{"rule":"device","device":"x"}"#).is_err());
    }
}
