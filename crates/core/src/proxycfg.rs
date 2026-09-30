//! Per-game configuration read by the proxy DLL.
//!
//! Example:
//! ```json
//! { "version": 1, "log": false, "players": [
//!     { "rule": "device", "device": "0F0D:008C", "instance": 0, "slot": 1 },
//!     { "rule": "device", "device": "045E:02FF", "instance": 0, "slot": 0 },
//!     { "rule": "auto" },
//!     { "rule": "none" } ] }
//! ```
//! The parser below never allocates: the proxy runs it inside game threads.

use crate::mapping::{DeviceId, Rule, SLOTS};

pub const VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ProxyConfig {
    pub log: bool,
    pub players: [Rule; SLOTS],
}

/// Parses the per-game config. Returns None on any syntax or value error, in which
/// case the proxy passes calls through unchanged.
pub fn parse(text: &[u8]) -> Option<ProxyConfig> {
    let text = text.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(text);
    let mut p = Parser { b: text, i: 0 };
    let mut cfg = ProxyConfig::default();
    p.object(|p, key| {
        match key {
            b"log" => cfg.log = p.boolean()?,
            b"version" => {
                if p.number()? != VERSION {
                    return None;
                }
            }
            b"players" => {
                let mut n = 0;
                p.array(|p| {
                    let rule = parse_rule(p)?;
                    *cfg.players.get_mut(n)? = rule;
                    n += 1;
                    Some(())
                })?;
            }
            _ => p.skip_value(0)?,
        }
        Some(())
    })?;
    p.ws();
    (p.i == p.b.len()).then_some(cfg)
}

fn parse_rule(p: &mut Parser) -> Option<Rule> {
    let mut kind: &[u8] = b"";
    let mut device = None;
    let mut instance = 0u32;
    let mut slot = None;
    p.object(|p, key| {
        match key {
            b"rule" => kind = p.string()?,
            b"device" => device = Some(DeviceId::parse(p.string()?)?),
            b"instance" => instance = p.number()?,
            b"slot" => slot = Some(p.number()?),
            _ => p.skip_value(0)?,
        }
        Some(())
    })?;
    let slot = match slot {
        Some(s) if s < SLOTS as u32 => Some(s as u8),
        Some(_) => return None,
        None => None,
    };
    match kind {
        b"auto" => Some(Rule::Auto),
        b"none" => Some(Rule::None),
        b"slot" => Some(Rule::Slot(slot?)),
        b"device" => Some(Rule::Device {
            id: device?,
            instance: u8::try_from(instance).ok().filter(|&i| (i as usize) < SLOTS)?,
            slot: slot.unwrap_or(0),
        }),
        _ => None,
    }
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while matches!(self.b.get(self.i), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.i += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.b.get(self.i).copied()
    }

    fn eat(&mut self, c: u8) -> Option<()> {
        (self.peek()? == c).then(|| self.i += 1)
    }

    fn literal(&mut self, word: &[u8]) -> bool {
        self.ws();
        if self.b.get(self.i..).is_some_and(|rest| rest.starts_with(word)) {
            self.i += word.len();
            true
        } else {
            false
        }
    }

    /// Raw string content (escape sequences are kept as-is; none are expected in values).
    fn string(&mut self) -> Option<&'a [u8]> {
        self.eat(b'"')?;
        let start = self.i;
        loop {
            match *self.b.get(self.i)? {
                b'"' => break,
                b'\\' => self.i += 2,
                _ => self.i += 1,
            }
        }
        let s = self.b.get(start..self.i)?;
        self.i += 1;
        Some(s)
    }

    fn number(&mut self) -> Option<u32> {
        self.ws();
        let start = self.i;
        let mut v: u32 = 0;
        while let Some(&c @ b'0'..=b'9') = self.b.get(self.i) {
            v = v.checked_mul(10)?.checked_add((c - b'0') as u32)?;
            self.i += 1;
        }
        (self.i > start).then_some(v)
    }

    fn boolean(&mut self) -> Option<bool> {
        if self.literal(b"true") {
            Some(true)
        } else if self.literal(b"false") {
            Some(false)
        } else {
            None
        }
    }

    fn object(&mut self, mut f: impl FnMut(&mut Self, &'a [u8]) -> Option<()>) -> Option<()> {
        self.eat(b'{')?;
        if self.peek()? == b'}' {
            self.i += 1;
            return Some(());
        }
        loop {
            let key = self.string()?;
            self.eat(b':')?;
            f(self, key)?;
            match self.peek()? {
                b',' => self.i += 1,
                b'}' => {
                    self.i += 1;
                    return Some(());
                }
                _ => return None,
            }
        }
    }

    fn array(&mut self, mut f: impl FnMut(&mut Self) -> Option<()>) -> Option<()> {
        self.eat(b'[')?;
        if self.peek()? == b']' {
            self.i += 1;
            return Some(());
        }
        loop {
            f(self)?;
            match self.peek()? {
                b',' => self.i += 1,
                b']' => {
                    self.i += 1;
                    return Some(());
                }
                _ => return None,
            }
        }
    }

    fn skip_value(&mut self, depth: u32) -> Option<()> {
        if depth > 16 {
            return None;
        }
        match self.peek()? {
            b'{' => self.object(|p, _| p.skip_value(depth + 1)),
            b'[' => self.array(|p| p.skip_value(depth + 1)),
            b'"' => self.string().map(|_| ()),
            b'-' | b'0'..=b'9' => {
                let start = self.i;
                while matches!(self.b.get(self.i), Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')) {
                    self.i += 1;
                }
                (self.i > start).then_some(())
            }
            _ => (self.literal(b"true") || self.literal(b"false") || self.literal(b"null")).then_some(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HORI: DeviceId = DeviceId { vid: 0x0F0D, pid: 0x008C };

    #[test]
    fn parses_full_config() {
        let text = br#"{ "version": 1, "log": true, "players": [
            { "rule": "device", "device": "0F0D:008C", "instance": 0, "slot": 1 },
            { "rule": "slot", "slot": 3 },
            { "rule": "none" } ], "extra": {"a": [1, -2.5e3, null, "x\"y"]} }"#;
        let cfg = parse(text).unwrap();
        assert!(cfg.log);
        assert_eq!(
            cfg.players,
            [Rule::Device { id: HORI, instance: 0, slot: 1 }, Rule::Slot(3), Rule::None, Rule::Auto]
        );
    }

    #[test]
    fn empty_object_is_passthrough_config() {
        assert_eq!(parse(b"{}"), Some(ProxyConfig::default()));
        assert_eq!(parse(b"\xEF\xBB\xBF{ }\r\n"), Some(ProxyConfig::default()));
    }

    #[test]
    fn invalid_configs_are_rejected() {
        for bad in [
            &b""[..],
            b"not json",
            b"{",
            b"{\"players\": [ {\"rule\": \"slot\", \"slot\": 4} ]}",
            b"{\"players\": [ {\"rule\": \"device\", \"device\": \"zz\"} ]}",
            b"{\"players\": [ {\"rule\": \"teleport\"} ]}",
            b"{\"players\": [{},{},{},{},{}]}",
            b"{\"version\": 2}",
            b"{} trailing",
            b"{\"log\": 1}",
        ] {
            assert_eq!(parse(bad), None, "{}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn serde_output_is_readable_by_the_proxy_parser() {
        #[cfg(feature = "serde")]
        {
            let cfg = crate::appcfg::proxy_config_json(
                true,
                &[Rule::Device { id: HORI, instance: 1, slot: 2 }, Rule::None, Rule::Slot(0), Rule::Auto],
            );
            let parsed = parse(cfg.as_bytes()).unwrap();
            assert!(parsed.log);
            assert_eq!(parsed.players[0], Rule::Device { id: HORI, instance: 1, slot: 2 });
            assert_eq!(parsed.players[2], Rule::Slot(0));
        }
    }
}
