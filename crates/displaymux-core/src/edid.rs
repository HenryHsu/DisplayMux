use crate::{MonitorResolution, ResolutionSource};

const EDID_BLOCK_SIZE: usize = 128;
const EDID_HEADER: [u8; 8] = [0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00];

pub(crate) fn is_valid(edid: &[u8]) -> bool {
    if edid.len() < EDID_BLOCK_SIZE || edid[..8] != EDID_HEADER {
        return false;
    }

    let block_count = usize::from(edid[126]) + 1;
    let Some(required_len) = block_count.checked_mul(EDID_BLOCK_SIZE) else {
        return false;
    };
    required_len <= edid.len()
        && edid[..required_len]
            .chunks_exact(EDID_BLOCK_SIZE)
            .all(|block| block.iter().fold(0_u8, |sum, byte| sum.wrapping_add(*byte)) == 0)
}

pub(crate) fn preferred_resolution(
    edid: Option<&[u8]>,
    core_graphics_mode: Option<MonitorResolution>,
) -> (Option<MonitorResolution>, Option<ResolutionSource>) {
    if let Some(resolution) = edid
        .filter(|value| is_valid(value))
        .and_then(max_resolution)
    {
        return (Some(resolution), Some(ResolutionSource::Edid));
    }

    (
        core_graphics_mode,
        core_graphics_mode.map(|_| ResolutionSource::CoreGraphicsDisplayMode),
    )
}

fn max_resolution(edid: &[u8]) -> Option<MonitorResolution> {
    let mut maximum = None;
    for offset in [54, 72, 90, 108] {
        consider_detailed_timing(edid, offset, &mut maximum);
    }

    for block in edid[EDID_BLOCK_SIZE..].chunks_exact(EDID_BLOCK_SIZE) {
        if block[0] != 0x02 {
            continue;
        }
        let detailed_timing_start = usize::from(block[2]);
        if !(4..=109).contains(&detailed_timing_start) {
            continue;
        }
        for offset in (detailed_timing_start..127).step_by(18) {
            consider_detailed_timing(block, offset, &mut maximum);
        }
    }
    maximum
}

fn consider_detailed_timing(block: &[u8], offset: usize, maximum: &mut Option<MonitorResolution>) {
    let Some(timing) = block.get(offset..offset + 18) else {
        return;
    };
    if timing[0] == 0 && timing[1] == 0 {
        return;
    }

    let width = u32::from(timing[2]) | (u32::from(timing[4] & 0xf0) << 4);
    let height = u32::from(timing[5]) | (u32::from(timing[7] & 0xf0) << 4);
    if width == 0 || height == 0 {
        return;
    }
    let candidate = MonitorResolution::new(width, height);
    let candidate_key = (u64::from(width) * u64::from(height), width);
    let current_key = maximum
        .map(|current| {
            (
                u64::from(current.width) * u64::from(current.height),
                current.width,
            )
        })
        .unwrap_or_default();
    if candidate_key > current_key {
        *maximum = Some(candidate);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edid_with_timings(timings: &[(u32, u32)]) -> [u8; EDID_BLOCK_SIZE] {
        let mut edid = [0_u8; EDID_BLOCK_SIZE];
        edid[..8].copy_from_slice(&EDID_HEADER);
        for (index, (width, height)) in timings.iter().take(4).enumerate() {
            let offset = 54 + index * 18;
            edid[offset] = 1;
            edid[offset + 2] = *width as u8;
            edid[offset + 4] = ((*width >> 8) as u8) << 4;
            edid[offset + 5] = *height as u8;
            edid[offset + 7] = ((*height >> 8) as u8) << 4;
        }
        edid[127] = 0_u8.wrapping_sub(
            edid[..127]
                .iter()
                .fold(0_u8, |sum, byte| sum.wrapping_add(*byte)),
        );
        edid
    }

    #[test]
    fn valid_edid_uses_the_largest_detailed_timing() {
        for expected in [(3440, 1440), (2560, 1080), (2560, 1440), (3840, 2160)] {
            let edid = edid_with_timings(&[(1920, 1080), expected]);
            let (resolution, source) =
                preferred_resolution(Some(&edid), Some(MonitorResolution::new(1280, 720)));
            assert_eq!(
                resolution,
                Some(MonitorResolution::new(expected.0, expected.1))
            );
            assert_eq!(source, Some(ResolutionSource::Edid));
        }
    }

    #[test]
    fn malformed_or_missing_edid_uses_core_graphics_physical_pixels() {
        let fallback = Some(MonitorResolution::new(3440, 1440));
        for edid in [None, Some(&[0_u8; 32][..]), Some(&[0_u8; 128][..])] {
            assert_eq!(
                preferred_resolution(edid, fallback),
                (fallback, Some(ResolutionSource::CoreGraphicsDisplayMode))
            );
        }
    }

    #[test]
    fn valid_edid_without_a_timing_uses_core_graphics_physical_pixels() {
        let edid = edid_with_timings(&[]);
        let fallback = Some(MonitorResolution::new(2560, 1440));
        assert_eq!(
            preferred_resolution(Some(&edid), fallback),
            (fallback, Some(ResolutionSource::CoreGraphicsDisplayMode))
        );
    }

    #[test]
    fn bad_checksum_invalidates_an_otherwise_well_formed_edid() {
        let mut edid = edid_with_timings(&[(3840, 2160)]);
        edid[20] = edid[20].wrapping_add(1);
        assert!(!is_valid(&edid));
    }
}
