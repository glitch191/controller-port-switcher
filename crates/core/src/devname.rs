//! Picks a readable controller name from the Windows device tree. XInput gives no
//! name, only a vendor and product id; the app finds the device node of the XInput
//! interface (instance id with that VID/PID and "IG_"), walks up to the physical
//! device and passes the names it read here.

use crate::mapping::DeviceId;

/// Names read from one device node.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NodeNames {
    pub instance_id: String,
    /// Product string reported by the device itself (DEVPKEY_Device_BusReportedDeviceDesc).
    pub bus_reported: Option<String>,
    /// Name shown by Windows (friendly name, or the driver description).
    pub friendly: Option<String>,
}

/// Vendor id spellings used in instance ids: USB ("VID_045E") and Bluetooth
/// ("VID&0002045E", "VID&02045E" and the 0001 variants).
fn vid_patterns(vid: u16) -> [String; 5] {
    [
        format!("VID_{vid:04X}"),
        format!("VID&0002{vid:04X}"),
        format!("VID&0001{vid:04X}"),
        format!("VID&02{vid:04X}"),
        format!("VID&01{vid:04X}"),
    ]
}

/// True when the instance id is the XInput interface node of this device.
pub fn is_xinput_node(instance_id: &str, id: DeviceId) -> bool {
    let up = instance_id.to_ascii_uppercase();
    let pid = format!("PID_{:04X}", id.pid);
    let pid_bt = format!("PID&{:04X}", id.pid);
    up.contains("IG_") && vid_patterns(id.vid).iter().any(|v| up.contains(v.as_str())) && (up.contains(&pid) || up.contains(&pid_bt))
}

/// True when an ancestor still belongs to the same physical device (hubs and
/// Bluetooth radios have another vendor id or none).
pub fn same_vendor(instance_id: &str, vid: u16) -> bool {
    let up = instance_id.to_ascii_uppercase();
    vid_patterns(vid).iter().any(|v| up.contains(v.as_str()))
}

const GENERIC: [&str; 9] = [
    "controller",
    "game controller",
    "gamepad",
    "usb input device",
    "hid-compliant game controller",
    "xinput compatible hid device",
    "usb composite device",
    "bluetooth le xinput compatible input device",
    "xbox 360 controller for windows",
];

fn useful(name: &Option<String>) -> Option<String> {
    let n = name.as_deref()?.trim();
    // "Controller (Real Arcade Pro.4)" -> "Real Arcade Pro.4"
    let n = n
        .strip_prefix("Controller (")
        .and_then(|rest| rest.strip_suffix(')'))
        .unwrap_or(n)
        .trim();
    (!n.is_empty() && !GENERIC.iter().any(|g| n.eq_ignore_ascii_case(g))).then(|| n.to_string())
}

/// Chooses a name from the nodes of one device, ordered from the XInput interface
/// node up to the physical device. The product string reported by the device wins
/// (closest to the physical device first), then the name Windows shows. Generic
/// names ("Controller", "USB Input Device") and driver names are skipped.
pub fn pick(chain: &[NodeNames]) -> Option<String> {
    chain
        .iter()
        .rev()
        .find_map(|n| useful(&n.bus_reported))
        .or_else(|| chain.iter().rev().find_map(|n| useful(&n.friendly)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, bus: &str, friendly: &str) -> NodeNames {
        let opt = |s: &str| (!s.is_empty()).then(|| s.to_string());
        NodeNames { instance_id: id.into(), bus_reported: opt(bus), friendly: opt(friendly) }
    }

    // Chains read from the reference hardware.
    fn hori() -> Vec<NodeNames> {
        vec![
            node(r"USB\VID_0F0D&PID_008C&IG_00\9&33E8CE26&1&00", "Controller (Real Arcade Pro.4)", "USB Input Device"),
            node(r"USB\VID_0F0D&PID_008C\30B74CE6", "Real Arcade Pro.4", "Xbox 360 Controller for Windows"),
        ]
    }

    fn xbox() -> Vec<NodeNames> {
        vec![
            node(r"USB\VID_045E&PID_02FF&IG_00\00&00&0000ADB7588AED7E", "", "USB Input Device"),
            node(r"USB\VID_045E&PID_0B12\3039373130323839373633343039", "Controller", "Xbox Controller"),
        ]
    }

    #[test]
    fn names_of_reference_devices() {
        assert_eq!(pick(&hori()).as_deref(), Some("Real Arcade Pro.4"));
        assert_eq!(pick(&xbox()).as_deref(), Some("Xbox Controller"));
    }

    #[test]
    fn interface_product_string_is_unwrapped() {
        let chain = vec![node(r"USB\VID_0F0D&PID_008C&IG_00\x", "Controller (Real Arcade Pro.4)", "")];
        assert_eq!(pick(&chain).as_deref(), Some("Real Arcade Pro.4"));
    }

    #[test]
    fn only_generic_names_give_none() {
        let chain = vec![node(r"HID\VID_1234&PID_5678&IG_00\x", "Controller", "HID-compliant game controller")];
        assert_eq!(pick(&chain), None);
    }

    #[test]
    fn matches_xinput_nodes() {
        let xbox_id = DeviceId { vid: 0x045E, pid: 0x02FF };
        assert!(is_xinput_node(r"HID\VID_045E&PID_02FF&IG_00\8&1F197B8D&0&0000", xbox_id));
        assert!(is_xinput_node(r"usb\vid_045e&pid_02ff&ig_00\00&00", xbox_id));
        assert!(!is_xinput_node(r"USB\VID_045E&PID_0B12\3039", xbox_id));
        assert!(!is_xinput_node(r"HID\VID_045E&PID_02FE&IG_00\x", xbox_id));
        let bt = DeviceId { vid: 0x045E, pid: 0x0B13 };
        assert!(is_xinput_node(r"HID\{00001812-0000-1000-8000-00805F9B34FB}_DEV_VID&02045E_PID&0B13_REV&0509_IG_00\x", bt));
        assert!(same_vendor(r"USB\VID_045E&PID_0B12\3039", 0x045E));
        assert!(!same_vendor(r"USB\ROOT_HUB30\7&32FCECFC&0&0", 0x045E));
    }
}
