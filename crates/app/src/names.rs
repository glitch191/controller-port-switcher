//! Controller model names read from the Windows device tree (CfgMgr32, no
//! administrator rights). Names are cached per device id and read again only after
//! a device change.

use cps_core::devname::{self, NodeNames};
use cps_core::mapping::{DeviceId, SLOTS, SlotState};
use std::cell::RefCell;
use std::collections::HashMap;
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    CM_GETIDLIST_FILTER_PRESENT, CM_Get_DevNode_PropertyW, CM_Get_Device_ID_List_SizeW, CM_Get_Device_ID_ListW,
    CM_Get_Device_IDW, CM_Get_Parent, CM_LOCATE_DEVNODE_NORMAL, CM_Locate_DevNodeW, CR_SUCCESS,
};
use windows_sys::Win32::Devices::Properties::{
    DEVPKEY_Device_BusReportedDeviceDesc, DEVPKEY_Device_DeviceDesc, DEVPKEY_Device_FriendlyName,
};
use windows_sys::Win32::Foundation::DEVPROPKEY;

/// Present device instance ids.
fn present_devices() -> Vec<String> {
    unsafe {
        let mut len = 0u32;
        if CM_Get_Device_ID_List_SizeW(&mut len, std::ptr::null(), CM_GETIDLIST_FILTER_PRESENT) != CR_SUCCESS {
            return Vec::new();
        }
        let mut buf = vec![0u16; len as usize];
        if CM_Get_Device_ID_ListW(std::ptr::null(), buf.as_mut_ptr(), len, CM_GETIDLIST_FILTER_PRESENT) != CR_SUCCESS {
            return Vec::new();
        }
        buf.split(|&c| c == 0)
            .filter(|s| !s.is_empty())
            .map(String::from_utf16_lossy)
            .collect()
    }
}

fn property(devinst: u32, key: &DEVPROPKEY) -> Option<String> {
    let mut kind = 0u32;
    let mut buf = [0u16; 256];
    let mut size = (buf.len() * 2) as u32;
    let r = unsafe { CM_Get_DevNode_PropertyW(devinst, key, &mut kind, buf.as_mut_ptr().cast(), &mut size, 0) };
    if r != CR_SUCCESS {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

fn instance_id(devinst: u32) -> Option<String> {
    let mut buf = [0u16; 512];
    let r = unsafe { CM_Get_Device_IDW(devinst, buf.as_mut_ptr(), buf.len() as u32, 0) };
    if r != CR_SUCCESS {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

fn node_names(devinst: u32, id: String) -> NodeNames {
    NodeNames {
        instance_id: id,
        bus_reported: property(devinst, &DEVPKEY_Device_BusReportedDeviceDesc),
        friendly: property(devinst, &DEVPKEY_Device_FriendlyName)
            .or_else(|| property(devinst, &DEVPKEY_Device_DeviceDesc)),
    }
}

/// Reads the name of one device: its XInput interface node and the ancestors that
/// share its vendor id (the physical device), then lets cps-core choose.
fn read_name(devices: &[String], id: DeviceId) -> Option<String> {
    let start = devices.iter().find(|d| devname::is_xinput_node(d, id))?;
    let wide: Vec<u16> = start.encode_utf16().chain(Some(0)).collect();
    let mut devinst = 0u32;
    if unsafe { CM_Locate_DevNodeW(&mut devinst, wide.as_ptr(), CM_LOCATE_DEVNODE_NORMAL) } != CR_SUCCESS {
        return None;
    }
    let mut chain = vec![node_names(devinst, start.clone())];
    for _ in 0..4 {
        let mut parent = 0u32;
        if unsafe { CM_Get_Parent(&mut parent, devinst, 0) } != CR_SUCCESS {
            break;
        }
        let Some(pid) = instance_id(parent) else { break };
        if !devname::same_vendor(&pid, id.vid) {
            break;
        }
        chain.push(node_names(parent, pid));
        devinst = parent;
    }
    devname::pick(&chain)
}

#[derive(Default)]
pub struct Names {
    cache: RefCell<HashMap<DeviceId, Option<String>>>,
}

impl Names {
    /// Forgets cached names (after a device change).
    pub fn clear(&self) {
        self.cache.borrow_mut().clear();
    }

    /// Model names for the connected slots (None when unknown or empty).
    pub fn for_slots(&self, slots: &[SlotState; SLOTS]) -> [Option<String>; SLOTS] {
        let mut cache = self.cache.borrow_mut();
        let ids: Vec<DeviceId> = slots.iter().filter(|s| s.connected).filter_map(|s| s.id).collect();
        if ids.iter().any(|id| !cache.contains_key(id)) {
            let devices = present_devices();
            for id in ids {
                cache.entry(id).or_insert_with(|| read_name(&devices, id));
            }
        }
        slots.map(|s| match (s.connected, s.id) {
            (true, Some(id)) => cache.get(&id).cloned().flatten(),
            _ => None,
        })
    }
}
