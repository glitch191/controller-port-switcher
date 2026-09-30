//! Player-to-slot mapping rules and their resolution against the connected controllers.
//!
//! A "slot" is a physical XInput user index (0-3) assigned by Windows. A "player" is the
//! index the game asks for. The resolved map says which slot each player reads from.

use core::fmt;

pub const SLOTS: usize = 4;

/// Marker for "this player has no controller".
pub const NO_SLOT: u8 = 0xFF;

/// Packed identity map: player N reads slot N.
pub const IDENTITY: u32 = 0x0302_0100;

/// USB vendor and product id reported by XInput for a controller.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DeviceId {
    pub vid: u16,
    pub pid: u16,
}

impl DeviceId {
    /// Parses "VVVV:PPPP" (hexadecimal, case-insensitive).
    pub fn parse(s: &[u8]) -> Option<Self> {
        if s.len() != 9 || s[4] != b':' {
            return None;
        }
        Some(Self { vid: hex16(&s[..4])?, pid: hex16(&s[5..])? })
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04X}:{:04X}", self.vid, self.pid)
    }
}

fn hex16(s: &[u8]) -> Option<u16> {
    let mut v: u16 = 0;
    for &c in s {
        let d = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => return None,
        };
        v = (v << 4) | d as u16;
    }
    Some(v)
}

/// What a player should read from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Rule {
    /// Take the next slot not claimed by another rule, in natural order.
    #[default]
    Auto,
    /// Always report "not connected".
    None,
    /// A fixed physical slot.
    Slot(u8),
    /// A specific device. `instance` picks among several devices with the same id
    /// (in slot order); `slot` is the slot it had when chosen, used only when the
    /// system cannot report device ids.
    Device { id: DeviceId, instance: u8, slot: u8 },
}

/// What is currently plugged into one physical slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SlotState {
    pub connected: bool,
    /// XINPUT_DEVSUBTYPE_* value.
    pub subtype: u8,
    /// None when the system does not report ids (XInputGetCapabilitiesEx missing).
    pub id: Option<DeviceId>,
}

impl SlotState {
    pub fn connected(subtype: u8, vid: u16, pid: u16) -> Self {
        Self { connected: true, subtype, id: Some(DeviceId { vid, pid }) }
    }
}

/// Instance number of the device in `slot` among connected devices with the same id.
pub fn instance_of(slots: &[SlotState; SLOTS], slot: usize) -> u8 {
    let id = slots[slot].id;
    slots[..slot].iter().filter(|s| s.connected && s.id == id).count() as u8
}

/// Builds the rule that targets whatever is currently in `slot`.
pub fn rule_for_slot(slots: &[SlotState; SLOTS], slot: usize) -> Rule {
    match slots[slot].id {
        Some(id) if slots[slot].connected => {
            Rule::Device { id, instance: instance_of(slots, slot), slot: slot as u8 }
        }
        _ => Rule::Slot(slot as u8),
    }
}

/// Resolves rules to a map `player -> slot` (or NO_SLOT). Never assigns a slot twice.
/// Order: device rules, then fixed slots, then Auto players take the remaining slots.
pub fn resolve(rules: &[Rule; SLOTS], slots: &[SlotState; SLOTS]) -> [u8; SLOTS] {
    let ids_known = slots.iter().any(|s| s.connected && s.id.is_some());
    let mut map = [NO_SLOT; SLOTS];
    let mut claimed = [false; SLOTS];

    for (p, rule) in rules.iter().enumerate() {
        if let Rule::Device { id, instance, slot } = *rule {
            let found = (0..SLOTS)
                .filter(|&s| slots[s].connected && slots[s].id == Some(id))
                .nth(instance as usize);
            let target = match found {
                Some(s) => Some(s),
                None if !ids_known && (slot as usize) < SLOTS => Some(slot as usize),
                None => None,
            };
            if let Some(s) = target.filter(|&s| !claimed[s]) {
                map[p] = s as u8;
                claimed[s] = true;
            }
        }
    }
    for (p, rule) in rules.iter().enumerate() {
        if let Rule::Slot(s) = *rule {
            let s = s as usize;
            if s < SLOTS && !claimed[s] {
                map[p] = s as u8;
                claimed[s] = true;
            }
        }
    }
    let mut free = (0..SLOTS).filter(|&s| !claimed[s]);
    for (p, rule) in rules.iter().enumerate() {
        if *rule == Rule::Auto
            && let Some(s) = free.next()
        {
            map[p] = s as u8;
        }
    }
    map
}

/// Rules after moving whatever player `from` currently reads to player `to` (both
/// 0-3), the player at `to` taking `from`'s place. The current order is first written
/// out explicitly (devices where connected), so the result does not depend on Auto.
pub fn move_port(rules: &[Rule; SLOTS], slots: &[SlotState; SLOTS], from: usize, to: usize) -> [Rule; SLOTS] {
    let map = resolve(rules, slots);
    let mut out = [Rule::Auto; SLOTS];
    for (p, &s) in map.iter().enumerate() {
        out[p] = match s {
            NO_SLOT => Rule::None,
            s if slots[s as usize].connected => rule_for_slot(slots, s as usize),
            _ => Rule::Auto,
        };
    }
    out.swap(from, to);
    out
}

/// True when rules need device ids to resolve (the proxy then re-queries them).
pub fn uses_devices(rules: &[Rule; SLOTS]) -> bool {
    rules.iter().any(|r| matches!(r, Rule::Device { .. }))
}

pub fn pack(map: &[u8; SLOTS]) -> u32 {
    u32::from_le_bytes(*map)
}

pub fn unpack(packed: u32) -> [u8; SLOTS] {
    packed.to_le_bytes()
}

/// Player that reads `slot`, if any (used to translate keystroke user indices back).
pub fn player_for_slot(packed: u32, slot: u8) -> Option<u8> {
    unpack(packed).iter().position(|&s| s == slot).map(|p| p as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    const XBOX: DeviceId = DeviceId { vid: 0x045E, pid: 0x02FF };
    const HORI: DeviceId = DeviceId { vid: 0x0F0D, pid: 0x008C };
    const EMPTY: SlotState = SlotState { connected: false, subtype: 0, id: None };

    fn dev(id: DeviceId, instance: u8, slot: u8) -> Rule {
        Rule::Device { id, instance, slot }
    }

    fn two_devices() -> [SlotState; 4] {
        [SlotState::connected(1, 0x045E, 0x02FF), SlotState::connected(1, 0x0F0D, 0x008C), EMPTY, EMPTY]
    }

    #[test]
    fn all_auto_is_identity() {
        let map = resolve(&[Rule::Auto; 4], &two_devices());
        assert_eq!(pack(&map), IDENTITY);
    }

    #[test]
    fn swap_by_device() {
        let rules = [dev(HORI, 0, 1), dev(XBOX, 0, 0), Rule::Auto, Rule::Auto];
        assert_eq!(resolve(&rules, &two_devices()), [1, 0, 2, 3]);
    }

    #[test]
    fn device_follows_slot_changes() {
        // Same rules after the devices swapped their natural slots.
        let slots = [SlotState::connected(1, 0x0F0D, 0x008C), SlotState::connected(1, 0x045E, 0x02FF), EMPTY, EMPTY];
        let rules = [dev(HORI, 0, 1), dev(XBOX, 0, 0), Rule::Auto, Rule::Auto];
        assert_eq!(resolve(&rules, &slots), [0, 1, 2, 3]);
    }

    #[test]
    fn auto_fills_remaining_slots() {
        let rules = [dev(HORI, 0, 1), Rule::Auto, Rule::Auto, Rule::Auto];
        assert_eq!(resolve(&rules, &two_devices()), [1, 0, 2, 3]);
    }

    #[test]
    fn mapping_to_none() {
        let rules = [Rule::None, dev(HORI, 0, 1), Rule::Auto, Rule::Auto];
        assert_eq!(resolve(&rules, &two_devices()), [NO_SLOT, 1, 0, 2]);
    }

    #[test]
    fn empty_slot_stays_empty_but_mapped() {
        let rules = [Rule::Slot(3), Rule::Auto, Rule::Auto, Rule::Auto];
        assert_eq!(resolve(&rules, &two_devices()), [3, 0, 1, 2]);
    }

    #[test]
    fn missing_device_means_not_connected() {
        let slots = [SlotState::connected(1, 0x045E, 0x02FF), EMPTY, EMPTY, EMPTY];
        let rules = [dev(HORI, 0, 1), Rule::Auto, Rule::Auto, Rule::Auto];
        assert_eq!(resolve(&rules, &slots), [NO_SLOT, 0, 1, 2]);
    }

    #[test]
    fn two_devices_with_same_id() {
        let slots = [SlotState::connected(1, 0x045E, 0x02FF), SlotState::connected(1, 0x045E, 0x02FF), EMPTY, EMPTY];
        let rules = [dev(XBOX, 1, 1), dev(XBOX, 0, 0), Rule::Auto, Rule::Auto];
        assert_eq!(resolve(&rules, &slots), [1, 0, 2, 3]);
        assert_eq!(instance_of(&slots, 1), 1);
        assert_eq!(rule_for_slot(&slots, 1), dev(XBOX, 1, 1));
    }

    #[test]
    fn two_rules_on_same_device_do_not_duplicate() {
        let rules = [dev(HORI, 0, 1), dev(HORI, 0, 1), Rule::Auto, Rule::Auto];
        assert_eq!(resolve(&rules, &two_devices()), [1, NO_SLOT, 0, 2]);
    }

    #[test]
    fn falls_back_to_slot_hint_without_ids() {
        let no_ids = SlotState { connected: true, subtype: 1, id: None };
        let slots = [no_ids, no_ids, EMPTY, EMPTY];
        let rules = [dev(HORI, 0, 1), dev(XBOX, 0, 0), Rule::Auto, Rule::Auto];
        assert_eq!(resolve(&rules, &slots), [1, 0, 2, 3]);
    }

    #[test]
    fn device_id_parse_and_display() {
        assert_eq!(DeviceId::parse(b"0f0d:008C"), Some(HORI));
        assert_eq!(DeviceId::parse(b"0F0D-008C"), None);
        assert_eq!(DeviceId::parse(b"0F0D:08C"), None);
        assert_eq!(HORI.to_string(), "0F0D:008C");
    }

    #[test]
    fn move_stick_to_port_one() {
        let rules = move_port(&[Rule::Auto; 4], &two_devices(), 1, 0);
        assert_eq!(rules, [dev(HORI, 0, 1), dev(XBOX, 0, 0), Rule::Auto, Rule::Auto]);
        assert_eq!(resolve(&rules, &two_devices()), [1, 0, 2, 3]);
        // Moving it back restores the natural order.
        let back = move_port(&rules, &two_devices(), 0, 1);
        assert_eq!(resolve(&back, &two_devices()), [0, 1, 2, 3]);
    }

    #[test]
    fn move_to_empty_port() {
        let rules = move_port(&[Rule::Auto; 4], &two_devices(), 0, 2);
        assert_eq!(resolve(&rules, &two_devices()), [2, 1, 0, 3]);
    }

    #[test]
    fn move_keeps_none_players() {
        let rules = [Rule::None, Rule::Auto, Rule::Auto, Rule::Auto];
        let moved = move_port(&rules, &two_devices(), 1, 2);
        assert_eq!(moved[0], Rule::None);
        assert_eq!(resolve(&moved, &two_devices()), [NO_SLOT, 1, 0, 2]);
    }

    #[test]
    fn inverse_lookup() {
        let packed = pack(&[1, 0, NO_SLOT, 2]);
        assert_eq!(player_for_slot(packed, 0), Some(1));
        assert_eq!(player_for_slot(packed, 2), Some(3));
        assert_eq!(player_for_slot(packed, 3), None);
    }
}
