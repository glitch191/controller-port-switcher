//! Text shown in the notification area tooltip and in menu labels.

use crate::mapping::{SLOTS, SlotState};

/// Windows limits NOTIFYICONDATAW::szTip to 128 UTF-16 units including the terminator.
pub const TIP_MAX: usize = 127;

pub fn subtype_name(subtype: u8) -> &'static str {
    match subtype {
        0x01 => "Gamepad",
        0x02 => "Wheel",
        0x03 => "Arcade stick",
        0x04 => "Flight stick",
        0x05 => "Dance pad",
        0x06 => "Guitar",
        0x07 => "Guitar alternate",
        0x08 => "Drum kit",
        0x0B => "Guitar bass",
        0x13 => "Arcade pad",
        _ => "Unknown",
    }
}

/// "Gamepad 045E:02FF", or just "Gamepad" when the id is unknown.
pub fn describe(slot: &SlotState) -> String {
    match slot.id {
        Some(id) => format!("{} {id}", subtype_name(slot.subtype)),
        None => subtype_name(slot.subtype).to_string(),
    }
}

/// One line per port, numbered 1-4 like players: "Port 1 Gamepad 045E:02FF" or
/// "Port 3 Empty". `index` is the XInput index (0-3).
pub fn slot_line(index: usize, slot: &SlotState) -> String {
    if slot.connected {
        format!("Port {} {}", index + 1, describe(slot))
    } else {
        format!("Port {} Empty", index + 1)
    }
}

/// Full tooltip: title, then one line per physical slot, truncated to TIP_MAX.
pub fn status_text(title: &str, slots: &[SlotState; SLOTS]) -> String {
    let mut lines = vec![title.to_string()];
    lines.extend(slots.iter().enumerate().map(|(i, s)| slot_line(i, s)));
    truncate(&lines.join("\n"), TIP_MAX)
}

fn units(s: &str) -> usize {
    s.encode_utf16().count()
}

/// Truncates to `max` UTF-16 units. Drops whole lines when possible and marks the cut
/// with "...", otherwise cuts inside the first line on a character boundary.
pub fn truncate(text: &str, max: usize) -> String {
    const MORE: &str = "...";
    if units(text) <= max {
        return text.to_string();
    }
    let mut kept = String::new();
    for line in text.split('\n') {
        let sep = if kept.is_empty() { 0 } else { 1 };
        if units(&kept) + sep + units(line) + 1 + MORE.len() > max {
            break;
        }
        if sep == 1 {
            kept.push('\n');
        }
        kept.push_str(line);
    }
    if !kept.is_empty() {
        kept.push('\n');
        kept.push_str(MORE);
        return kept;
    }
    let mut out = String::new();
    for c in text.chars() {
        if units(&out) + c.len_utf16() + MORE.len() > max {
            break;
        }
        out.push(c);
    }
    out.push_str(MORE);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMPTY: SlotState = SlotState { connected: false, subtype: 0, id: None };

    #[test]
    fn status_lists_all_slots() {
        let slots = [SlotState::connected(3, 0x0F0D, 0x008C), SlotState::connected(1, 0x045E, 0x02FF), EMPTY, EMPTY];
        assert_eq!(
            status_text("controller-port-switcher", &slots),
            "controller-port-switcher\nPort 1 Arcade stick 0F0D:008C\nPort 2 Gamepad 045E:02FF\nPort 3 Empty\nPort 4 Empty"
        );
    }

    #[test]
    fn unknown_id_and_subtype() {
        let s = SlotState { connected: true, subtype: 0x42, id: None };
        assert_eq!(slot_line(0, &s), "Port 1 Unknown");
        assert_eq!(describe(&SlotState::connected(1, 1, 2)), "Gamepad 0001:0002");
    }

    #[test]
    fn typical_status_fits_without_truncation() {
        let s = SlotState::connected(1, 0x045E, 0x02FF);
        let text = status_text("controller-port-switcher", &[s; 4]);
        assert!(!text.ends_with("..."), "{text}");
    }

    #[test]
    fn worst_case_status_is_truncated() {
        let s = SlotState::connected(0x07, 0xFFFF, 0xFFFF);
        let text = status_text("controller-port-switcher", &[s; 4]);
        assert!(units(&text) <= TIP_MAX, "{} units", units(&text));
    }

    #[test]
    fn long_text_is_truncated_on_line_boundary() {
        let text = (0..10).map(|i| format!("line {i} {}", "x".repeat(20))).collect::<Vec<_>>().join("\n");
        let out = truncate(&text, TIP_MAX);
        assert!(units(&out) <= TIP_MAX);
        assert!(out.ends_with("\n..."));
        assert!(out.lines().rev().skip(1).all(|l| l.len() == 27), "only whole lines kept: {out:?}");
    }

    #[test]
    fn long_single_line_is_cut_on_char_boundary() {
        let text = "\u{1D11E}".repeat(100);
        let out = truncate(&text, TIP_MAX);
        assert!(units(&out) <= TIP_MAX);
        assert!(out.ends_with("..."));
        assert_eq!(out.chars().filter(|&c| c == '\u{1D11E}').count(), 62);
    }

    #[test]
    fn short_text_unchanged() {
        assert_eq!(truncate("abc", TIP_MAX), "abc");
    }
}
