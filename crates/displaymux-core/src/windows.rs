use std::{collections::HashMap, ffi::c_void, mem::size_of, ptr};

use serde::Deserialize;
use windows_sys::core::BOOL;
use windows_sys::Win32::{
    Devices::Display::{
        DestroyPhysicalMonitor, GetNumberOfPhysicalMonitorsFromHMONITOR,
        GetPhysicalMonitorsFromHMONITOR, GetVCPFeatureAndVCPFeatureReply, SetVCPFeature,
        PHYSICAL_MONITOR,
    },
    Foundation::{LPARAM, RECT},
    Graphics::Gdi::{
        EnumDisplayDevicesW, EnumDisplayMonitors, GetMonitorInfoW, DISPLAY_DEVICEW, HDC, HMONITOR,
        MONITORINFOEXW,
    },
    UI::WindowsAndMessaging::EDD_GET_DEVICE_INTERFACE_NAME,
};
use wmi::WMIConnection;

use crate::{
    DisplayInput, DisplayMuxError, MonitorControl, MonitorDescriptor, MonitorFingerprint, MonitorId,
};

const INPUT_SOURCE_VCP_CODE: u8 = 0x60;

pub struct WindowsMonitorController;

impl WindowsMonitorController {
    pub fn new() -> Result<Self, DisplayMuxError> {
        Ok(Self)
    }

    fn enumerate_native(&self) -> Result<Vec<NativeMonitor>, DisplayMuxError> {
        let wmi_monitors = query_wmi_monitors()?;
        let logical_monitors = enumerate_logical_monitors()?;
        let mut native_monitors = Vec::new();

        for logical in logical_monitors {
            let device_path = monitor_device_path(logical)?;
            let identity = parse_device_path(&device_path)?;
            let wmi_monitor = wmi_monitors
                .get(&identity.wmi_instance_key)
                .ok_or_else(|| {
                    DisplayMuxError::Backend(format!(
                        "無法取得 {} 的 EDID 序號；為避免誤控，已停止列舉",
                        device_path
                    ))
                })?;
            let physical_monitors = physical_monitors(logical)?;

            for (index, physical) in physical_monitors.into_iter().enumerate() {
                let description_buffer = physical.szPhysicalMonitorDescription;
                let description = wide_string(&description_buffer);
                let name = decode_edid_text(&wmi_monitor.user_friendly_name)
                    .filter(|value| !value.is_empty())
                    .unwrap_or(description);
                let id = MonitorId::new(format!(
                    "{}::physical:{index}",
                    device_path.to_ascii_uppercase()
                ));

                native_monitors.push(NativeMonitor {
                    descriptor: MonitorDescriptor {
                        id,
                        name,
                        fingerprint: MonitorFingerprint::new(
                            &identity.manufacturer_id,
                            &identity.product_code,
                            decode_edid_text(&wmi_monitor.serial_number_id),
                        ),
                        active: wmi_monitor.active,
                    },
                    handle: physical.hPhysicalMonitor,
                });
            }
        }

        Ok(native_monitors)
    }

    fn find_native(&self, id: &MonitorId) -> Result<NativeMonitor, DisplayMuxError> {
        self.enumerate_native()?
            .into_iter()
            .find(|monitor| monitor.descriptor.id == *id)
            .ok_or_else(|| DisplayMuxError::MonitorNoLongerAvailable(id.as_str().to_owned()))
    }
}

impl MonitorControl for WindowsMonitorController {
    fn enumerate(&self) -> Result<Vec<MonitorDescriptor>, DisplayMuxError> {
        Ok(self
            .enumerate_native()?
            .into_iter()
            .map(|monitor| monitor.descriptor.clone())
            .collect())
    }

    fn read_input(&self, monitor: &MonitorId) -> Result<DisplayInput, DisplayMuxError> {
        let native = self.find_native(monitor)?;
        let mut code_type = 0;
        let mut current = 0;
        let mut maximum = 0;

        // SAFETY: `native.handle` is an owned, live physical-monitor handle. All out-pointers
        // reference initialized local `u32` values for the duration of the call.
        let succeeded = unsafe {
            GetVCPFeatureAndVCPFeatureReply(
                native.handle,
                INPUT_SOURCE_VCP_CODE,
                &mut code_type,
                &mut current,
                &mut maximum,
            )
        };
        if succeeded == 0 {
            return Err(last_windows_error("無法讀取共用螢幕目前的輸入來源"));
        }

        DisplayInput::new(current)
    }

    fn write_input(&self, monitor: &MonitorId, input: DisplayInput) -> Result<(), DisplayMuxError> {
        let native = self.find_native(monitor)?;

        // SAFETY: `native.handle` is an owned, live physical-monitor handle, VCP 0x60 is the
        // MCCS input-source feature, and `DisplayInput` restricts values to one byte.
        let succeeded =
            unsafe { SetVCPFeature(native.handle, INPUT_SOURCE_VCP_CODE, input.value()) };
        if succeeded == 0 {
            return Err(last_windows_error("無法切換共用螢幕輸入來源"));
        }

        Ok(())
    }
}

struct NativeMonitor {
    descriptor: MonitorDescriptor,
    handle: *mut c_void,
}

impl Drop for NativeMonitor {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: this type exclusively owns the physical-monitor handle and drops it once.
            let succeeded = unsafe { DestroyPhysicalMonitor(self.handle) };
            if succeeded == 0 {
                tracing::warn!(
                    monitor_id = self.descriptor.id.as_str(),
                    "failed to release physical monitor handle"
                );
            }
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct WmiMonitorId {
    instance_name: String,
    active: bool,
    serial_number_id: Vec<u16>,
    user_friendly_name: Vec<u16>,
}

fn query_wmi_monitors() -> Result<HashMap<String, WmiMonitorId>, DisplayMuxError> {
    let connection = WMIConnection::with_namespace_path("ROOT\\WMI")
        .map_err(|error| backend_error("無法連線 Windows WMI 螢幕資料", error))?;
    let monitors: Vec<WmiMonitorId> = connection
        .raw_query(
            "SELECT InstanceName, Active, SerialNumberID, UserFriendlyName FROM WmiMonitorID",
        )
        .map_err(|error| backend_error("無法讀取 Windows 螢幕 EDID", error))?;

    Ok(monitors
        .into_iter()
        .map(|monitor| (normalize_wmi_instance(&monitor.instance_name), monitor))
        .collect())
}

struct DeviceIdentity {
    manufacturer_id: String,
    product_code: String,
    wmi_instance_key: String,
}

fn parse_device_path(device_path: &str) -> Result<DeviceIdentity, DisplayMuxError> {
    let normalized = device_path
        .trim_start_matches("\\\\?\\")
        .to_ascii_uppercase();
    let parts = normalized.split('#').collect::<Vec<_>>();
    if parts.len() < 3 || parts[0] != "DISPLAY" || parts[1].len() < 4 {
        return Err(DisplayMuxError::Backend(format!(
            "無法解析 Windows 螢幕裝置路徑：{device_path}"
        )));
    }

    let hardware_id = parts[1];
    Ok(DeviceIdentity {
        manufacturer_id: hardware_id[..3].to_owned(),
        product_code: hardware_id[3..].to_owned(),
        wmi_instance_key: format!("DISPLAY\\{}\\{}", hardware_id, parts[2]),
    })
}

fn normalize_wmi_instance(instance: &str) -> String {
    instance
        .strip_suffix("_0")
        .unwrap_or(instance)
        .to_ascii_uppercase()
}

fn decode_edid_text(values: &[u16]) -> Option<String> {
    let text = values
        .iter()
        .copied()
        .take_while(|value| *value != 0)
        .filter_map(|value| char::from_u32(value as u32))
        .collect::<String>()
        .trim()
        .to_owned();
    (!text.is_empty()).then_some(text)
}

fn wide_string(values: &[u16]) -> String {
    String::from_utf16_lossy(
        &values
            .iter()
            .copied()
            .take_while(|value| *value != 0)
            .collect::<Vec<_>>(),
    )
}

fn monitor_device_path(monitor: HMONITOR) -> Result<String, DisplayMuxError> {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;

    // SAFETY: `info` has the required `cbSize`, remains live for the call, and the cast is valid
    // because MONITORINFOEXW begins with MONITORINFO as required by Win32.
    let info_succeeded = unsafe { GetMonitorInfoW(monitor, &mut info.monitorInfo) };
    if info_succeeded == 0 {
        return Err(last_windows_error("無法取得 Windows 邏輯螢幕資訊"));
    }

    let mut device = DISPLAY_DEVICEW {
        cb: size_of::<DISPLAY_DEVICEW>() as u32,
        ..Default::default()
    };

    // SAFETY: `info.szDevice` is a null-terminated buffer populated by GetMonitorInfoW and
    // `device` is initialized with the correct structure size.
    let device_succeeded = unsafe {
        EnumDisplayDevicesW(
            info.szDevice.as_ptr(),
            0,
            &mut device,
            EDD_GET_DEVICE_INTERFACE_NAME,
        )
    };
    if device_succeeded == 0 {
        return Err(last_windows_error("無法取得 Windows 實體螢幕裝置路徑"));
    }

    let device_path = wide_string(&device.DeviceID);
    if device_path.is_empty() {
        return Err(DisplayMuxError::Backend(
            "Windows 未提供螢幕裝置路徑；為避免誤控，已停止操作".to_owned(),
        ));
    }

    Ok(device_path)
}

fn physical_monitors(monitor: HMONITOR) -> Result<Vec<PHYSICAL_MONITOR>, DisplayMuxError> {
    let mut count = 0;

    // SAFETY: `count` is a valid out-pointer and `monitor` came from EnumDisplayMonitors.
    let count_succeeded = unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(monitor, &mut count) };
    if count_succeeded == 0 {
        return Err(last_windows_error("無法取得實體螢幕數量"));
    }

    let mut physical = (0..count)
        .map(|_| PHYSICAL_MONITOR::default())
        .collect::<Vec<_>>();

    // SAFETY: the vector has exactly `count` initialized slots and remains allocated for the call.
    let enumerate_succeeded =
        unsafe { GetPhysicalMonitorsFromHMONITOR(monitor, count, physical.as_mut_ptr()) };
    if enumerate_succeeded == 0 {
        return Err(last_windows_error("無法列舉實體螢幕控制介面"));
    }

    Ok(physical)
}

fn enumerate_logical_monitors() -> Result<Vec<HMONITOR>, DisplayMuxError> {
    unsafe extern "system" fn callback(
        monitor: HMONITOR,
        _device_context: HDC,
        _bounds: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        // SAFETY: `data` is the pointer to the live Vec passed to EnumDisplayMonitors below;
        // Win32 invokes callbacks synchronously before that Vec leaves scope.
        let monitors = unsafe { &mut *(data as *mut Vec<HMONITOR>) };
        monitors.push(monitor);
        1
    }

    let mut monitors = Vec::new();
    let data = &mut monitors as *mut Vec<HMONITOR> as LPARAM;

    // SAFETY: null HDC/clip enumerate all desktop monitors, callback has the required ABI, and
    // `data` points to a live Vec for the synchronous duration of the call.
    let succeeded =
        unsafe { EnumDisplayMonitors(ptr::null_mut(), ptr::null(), Some(callback), data) };
    if succeeded == 0 {
        return Err(last_windows_error("無法列舉 Windows 邏輯螢幕"));
    }

    Ok(monitors)
}

fn last_windows_error(action: &str) -> DisplayMuxError {
    DisplayMuxError::Backend(format!("{action}：{}", std::io::Error::last_os_error()))
}

fn backend_error(action: &str, error: impl std::fmt::Display) -> DisplayMuxError {
    DisplayMuxError::Backend(format!("{action}：{error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_monitor_interface_path() {
        let identity = parse_device_path(
            r"\\?\DISPLAY#AUS3554#5&5405411&0&UID4353#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}",
        )
        .expect("path parses");

        assert_eq!(identity.manufacturer_id, "AUS");
        assert_eq!(identity.product_code, "3554");
        assert_eq!(
            identity.wmi_instance_key,
            r"DISPLAY\AUS3554\5&5405411&0&UID4353"
        );
    }

    #[test]
    fn normalizes_wmi_instance_suffix() {
        assert_eq!(
            normalize_wmi_instance(r"DISPLAY\AUS3554\5&5405411&0&UID4353_0"),
            r"DISPLAY\AUS3554\5&5405411&0&UID4353"
        );
    }
}
