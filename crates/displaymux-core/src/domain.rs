use std::net::IpAddr;

use crate::DisplayMuxError;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MonitorId(String);

impl MonitorId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorFingerprint {
    pub manufacturer_id: String,
    pub product_code: String,
    pub serial_number: Option<String>,
}

impl MonitorFingerprint {
    pub fn new(
        manufacturer_id: impl Into<String>,
        product_code: impl Into<String>,
        serial_number: Option<impl Into<String>>,
    ) -> Self {
        Self {
            manufacturer_id: normalize_identifier(&manufacturer_id.into()),
            product_code: normalize_identifier(&product_code.into()),
            serial_number: serial_number.map(|value| value.into().trim().to_owned()),
        }
    }

    pub fn matches_exactly(&self, actual: &Self) -> bool {
        self.manufacturer_id
            .eq_ignore_ascii_case(&actual.manufacturer_id)
            && self.product_code.eq_ignore_ascii_case(&actual.product_code)
            && match (&self.serial_number, &actual.serial_number) {
                (Some(expected), Some(observed)) => expected == observed,
                (None, None) => true,
                _ => false,
            }
    }
}

fn normalize_identifier(value: &str) -> String {
    value.trim().to_ascii_uppercase()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorResolution {
    pub width: u32,
    pub height: u32,
}

impl MonitorResolution {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub fn is_ultrawide(&self) -> bool {
        if self.height == 0 {
            return false;
        }
        (self.width as f64 / self.height as f64) >= 2.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorDescriptor {
    pub id: MonitorId,
    pub name: String,
    pub fingerprint: MonitorFingerprint,
    pub active: bool,
    #[serde(default)]
    pub max_resolution: Option<MonitorResolution>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayInput(u32);

impl DisplayInput {
    pub fn new(value: u32) -> Result<Self, DisplayMuxError> {
        if (1..=u8::MAX as u32).contains(&value) {
            Ok(Self(value))
        } else {
            Err(DisplayMuxError::InvalidInput(value))
        }
    }

    pub const fn value(self) -> u32 {
        self.0
    }

    pub const fn standard_name(self) -> Option<&'static str> {
        match self.0 {
            0x01 => Some("VGA 1"),
            0x02 => Some("VGA 2"),
            0x03 => Some("DVI 1"),
            0x04 => Some("DVI 2"),
            0x05 => Some("Composite Video 1"),
            0x06 => Some("Composite Video 2"),
            0x07 => Some("S-Video 1"),
            0x08 => Some("S-Video 2"),
            0x09 => Some("Tuner 1"),
            0x0a => Some("Tuner 2"),
            0x0b => Some("Tuner 3"),
            0x0c => Some("Component Video 1"),
            0x0d => Some("Component Video 2"),
            0x0e => Some("Component Video 3"),
            0x0f => Some("DisplayPort 1"),
            0x10 => Some("DisplayPort 2"),
            0x11 => Some("HDMI 1"),
            0x12 => Some("HDMI 2"),
            _ => None,
        }
    }

    pub fn display_name(self) -> String {
        match self.standard_name() {
            Some(name) => format!("{name} (0x{:02X})", self.0),
            None => format!("自訂輸入 (0x{:02X})", self.0),
        }
    }

    pub fn parse_code(value: &str) -> Result<Self, DisplayMuxError> {
        let value = value.trim();
        let parsed = if let Some(hex) = value
            .strip_prefix("0x")
            .or_else(|| value.strip_prefix("0X"))
        {
            u32::from_str_radix(hex, 16)
        } else {
            value.parse::<u32>()
        }
        .map_err(|_| DisplayMuxError::InvalidInputCode(value.to_owned()))?;
        Self::new(parsed)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DestinationHost {
    Windows,
    Mac,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredPeer {
    pub id: String,
    pub name: String,
    pub platform: DestinationHost,
    pub address: IpAddr,
    pub port: u16,
    pub mac_address: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayMuxProfile {
    pub shared_monitor: MonitorFingerprint,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_standard_mccs_input_values() {
        let display_port = DisplayInput::new(0x0f).unwrap();
        let hdmi = DisplayInput::parse_code("0x11").unwrap();

        assert_eq!(display_port.display_name(), "DisplayPort 1 (0x0F)");
        assert_eq!(hdmi.display_name(), "HDMI 1 (0x11)");
    }

    #[test]
    fn preserves_manufacturer_specific_input_values() {
        let input = DisplayInput::parse_code("0x1B").unwrap();
        assert_eq!(input.display_name(), "自訂輸入 (0x1B)");
    }

    #[test]
    fn accepts_decimal_input_values() {
        assert_eq!(DisplayInput::parse_code("17").unwrap().value(), 0x11);
    }

    #[test]
    fn identifies_ultrawide_resolutions() {
        let ultrawide = MonitorResolution::new(3440, 1440);
        assert!(ultrawide.is_ultrawide());

        let standard = MonitorResolution::new(2560, 1440);
        assert!(!standard.is_ultrawide());

        let zero = MonitorResolution::new(1920, 0);
        assert!(!zero.is_ultrawide());
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SwitchMode {
    DryRun,
    Apply,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SwitchOutcome {
    DryRun {
        target: MonitorDescriptor,
        current: DisplayInput,
        requested: DisplayInput,
    },
    AlreadySelected {
        target: MonitorDescriptor,
        input: DisplayInput,
    },
    Switched {
        target: MonitorDescriptor,
        previous: DisplayInput,
        selected: DisplayInput,
    },
}
