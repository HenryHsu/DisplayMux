use ddc::Ddc;
use ddc_macos::Monitor;

use crate::{
    edid, DisplayInput, DisplayMuxError, MonitorControl, MonitorDescriptor, MonitorFingerprint,
    MonitorId, MonitorResolution,
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
        Ok(Monitor::enumerate()
            .map_err(backend_error)?
            .into_iter()
            .map(|monitor| descriptor(&monitor))
            .collect())
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

fn descriptor(monitor: &Monitor) -> MonitorDescriptor {
    let raw_edid = monitor.edid();
    let handle = monitor.handle();
    let fingerprint = raw_edid
        .as_deref()
        .and_then(|value| match fingerprint_from_edid(value) {
            Ok(fingerprint) => Some(fingerprint),
            Err(error) => {
                tracing::warn!(
                    monitor = %monitor.description(),
                    error = %error,
                    "macOS monitor EDID is unusable; using CoreGraphics identity"
                );
                None
            }
        })
        .unwrap_or_else(|| {
            fingerprint_from_native_ids(
                handle.vendor_number(),
                handle.model_number(),
                monitor.serial_number(),
            )
        });
    let (max_resolution, resolution_source) =
        edid::preferred_resolution(raw_edid.as_deref(), core_graphics_resolution(monitor));

    MonitorDescriptor {
        id: id_for(monitor),
        name: monitor.description(),
        fingerprint,
        active: true,
        built_in: handle.is_builtin(),
        max_resolution,
        resolution_source,
    }
}

fn id_for(monitor: &Monitor) -> MonitorId {
    MonitorId::new(format!("macos:{}", monitor.handle().id))
}

fn fingerprint_from_edid(edid: &[u8]) -> Result<MonitorFingerprint, DisplayMuxError> {
    if !edid::is_valid(edid) {
        return Err(DisplayMuxError::Backend(
            "顯示器 EDID 標頭、長度或 checksum 無效，無法安全識別裝置".to_owned(),
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

fn fingerprint_from_native_ids(
    vendor_number: u32,
    model_number: u32,
    serial_number: Option<String>,
) -> MonitorFingerprint {
    let manufacturer = vendor_number as u16;
    let manufacturer_id = [
        manufacturer_character((manufacturer >> 10) & 0x1f),
        manufacturer_character((manufacturer >> 5) & 0x1f),
        manufacturer_character(manufacturer & 0x1f),
    ]
    .into_iter()
    .collect::<String>();

    MonitorFingerprint::new(
        manufacturer_id,
        format!("{:04X}", model_number),
        serial_number,
    )
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

fn core_graphics_resolution(monitor: &Monitor) -> Option<MonitorResolution> {
    let mode = monitor.handle().display_mode()?;
    let width = u32::try_from(mode.pixel_width()).ok()?;
    let height = u32::try_from(mode.pixel_height()).ok()?;
    (width > 0 && height > 0).then(|| MonitorResolution::new(width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finalize_edid(edid: &mut [u8; 128]) {
        edid[..8].copy_from_slice(&[0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00]);
        edid[127] = 0_u8.wrapping_sub(
            edid[..127]
                .iter()
                .fold(0_u8, |sum, byte| sum.wrapping_add(*byte)),
        );
    }

    #[test]
    fn parses_asus_edid_identity() {
        let mut edid = [0_u8; 128];
        let manufacturer = (1_u16 << 10) | (21_u16 << 5) | 19_u16;
        [edid[8], edid[9]] = manufacturer.to_be_bytes();
        [edid[10], edid[11]] = 0x3554_u16.to_le_bytes();
        [edid[12], edid[13], edid[14], edid[15]] = 278_504_u32.to_le_bytes();
        finalize_edid(&mut edid);

        let fingerprint = fingerprint_from_edid(&edid).unwrap();

        assert_eq!(fingerprint.manufacturer_id, "AUS");
        assert_eq!(fingerprint.product_code, "3554");
        assert_eq!(fingerprint.serial_number.as_deref(), Some("278504"));
    }

    #[test]
    fn builds_matching_identity_from_core_graphics_when_edid_is_missing() {
        let vendor_number = (1_u32 << 10) | (21_u32 << 5) | 19_u32;

        let fingerprint =
            fingerprint_from_native_ids(vendor_number, 0x3554, Some("278504".to_owned()));

        assert_eq!(fingerprint.manufacturer_id, "AUS");
        assert_eq!(fingerprint.product_code, "3554");
        assert_eq!(fingerprint.serial_number.as_deref(), Some("278504"));
    }
}
