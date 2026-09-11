use ddc::Ddc;
use ddc_macos::Monitor;

use crate::{
    DisplayInput, DisplayMuxError, MonitorControl, MonitorDescriptor, MonitorFingerprint, MonitorId,
};

const INPUT_SELECT_VCP_CODE: u8 = 0x60;

/// macOS DDC/CI adapter. `ddc-macos` chooses the Intel IOKit or Apple Silicon
/// display service at runtime, so the same implementation covers Mac mini,
/// MacBook Air and MacBook Pro connection paths that expose DDC.
pub struct MacOsMonitorController;

impl MacOsMonitorController {
    pub const fn new() -> Self {
        Self
    }
}

impl Default for MacOsMonitorController {
    fn default() -> Self {
        Self::new()
    }
}

impl MonitorControl for MacOsMonitorController {
    fn enumerate(&self) -> Result<Vec<MonitorDescriptor>, DisplayMuxError> {
        Monitor::enumerate()
            .map_err(backend_error)?
            .into_iter()
            .map(|monitor| descriptor(&monitor))
            .collect()
    }

    fn read_input(&self, monitor_id: &MonitorId) -> Result<DisplayInput, DisplayMuxError> {
        let mut monitor = find_monitor(monitor_id)?;
        let value = monitor
            .get_vcp_feature(INPUT_SELECT_VCP_CODE)
            .map_err(backend_error)?;
        DisplayInput::new(u32::from(value.value()))
    }

    fn write_input(
        &self,
        monitor_id: &MonitorId,
        input: DisplayInput,
    ) -> Result<(), DisplayMuxError> {
        let mut monitor = find_monitor(monitor_id)?;
        monitor
            .set_vcp_feature(INPUT_SELECT_VCP_CODE, input.value() as u16)
            .map_err(backend_error)
    }
}

fn find_monitor(monitor_id: &MonitorId) -> Result<Monitor, DisplayMuxError> {
    Monitor::enumerate()
        .map_err(backend_error)?
        .into_iter()
        .find(|monitor| id_for(monitor) == *monitor_id)
        .ok_or_else(|| DisplayMuxError::MonitorNoLongerAvailable(monitor_id.as_str().to_owned()))
}

fn descriptor(monitor: &Monitor) -> Result<MonitorDescriptor, DisplayMuxError> {
    let edid = monitor.edid().ok_or_else(|| {
        DisplayMuxError::Backend(format!(
            "無法讀取 {} 的 EDID，為避免誤控已略過此顯示器",
            monitor.description()
        ))
    })?;
    let fingerprint = fingerprint_from_edid(&edid)?;
    Ok(MonitorDescriptor {
        id: id_for(monitor),
        name: monitor.description(),
        fingerprint,
        active: true,
    })
}

fn id_for(monitor: &Monitor) -> MonitorId {
    MonitorId::new(format!("macos:{}", monitor.handle().id))
}

fn fingerprint_from_edid(edid: &[u8]) -> Result<MonitorFingerprint, DisplayMuxError> {
    if edid.len() < 16 {
        return Err(DisplayMuxError::Backend(
            "顯示器 EDID 長度不足，無法安全識別裝置".to_owned(),
        ));
    }

    let manufacturer = u16::from_be_bytes([edid[8], edid[9]]);
    let manufacturer_id = [
        manufacturer_character((manufacturer >> 10) & 0x1f),
        manufacturer_character((manufacturer >> 5) & 0x1f),
        manufacturer_character(manufacturer & 0x1f),
    ]
    .into_iter()
    .collect::<String>();
    let product_code = format!("{:04X}", u16::from_le_bytes([edid[10], edid[11]]));
    let serial = u32::from_le_bytes([edid[12], edid[13], edid[14], edid[15]]);

    Ok(MonitorFingerprint::new(
        manufacturer_id,
        product_code,
        (serial != 0).then(|| serial.to_string()),
    ))
}

fn manufacturer_character(value: u16) -> char {
    if (1..=26).contains(&value) {
        char::from_u32(u32::from(value) + 64).unwrap_or('?')
    } else {
        '?'
    }
}

fn backend_error(error: impl std::fmt::Display) -> DisplayMuxError {
    DisplayMuxError::Backend(format!(
        "macOS 無法透過目前的 HDMI／USB-C／Thunderbolt 路徑使用 DDC/CI：{error}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_asus_edid_identity() {
        let mut edid = [0_u8; 128];
        let manufacturer = (1_u16 << 10) | (21_u16 << 5) | 19_u16;
        [edid[8], edid[9]] = manufacturer.to_be_bytes();
        [edid[10], edid[11]] = 0x3554_u16.to_le_bytes();
        [edid[12], edid[13], edid[14], edid[15]] = 278_504_u32.to_le_bytes();

        let fingerprint = fingerprint_from_edid(&edid).unwrap();

        assert_eq!(fingerprint.manufacturer_id, "AUS");
        assert_eq!(fingerprint.product_code, "3554");
        assert_eq!(fingerprint.serial_number.as_deref(), Some("278504"));
    }
}
