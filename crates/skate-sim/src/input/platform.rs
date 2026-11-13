//! Device transport. SDL3 gamepads are the default on every platform; each
//! state is re-expressed as the XInput-shaped `XboxState`, so raw signed
//! axes/trigger bytes reach the TU3 converter without Bevy/gilrs deadzones or
//! normalized-axis reconstruction. `SKATE3_INPUT=xinput` selects the original
//! Windows XInput transport, which is also the fallback if SDL cannot start.
use super::controller_kind::{ControllerKind, XinputCaps};
use skate_core::input::xbox::XboxState;
use std::sync::{Arc, OnceLock};
use std::sync::atomic::{AtomicU64, Ordering};

/// XInput `wButtons` bits, the layout `xbox::convert` consumes.
pub(crate) const BUTTON_NAMES: [(&str, u16); 14] = [
    ("dpad_up", 0x0001), ("dpad_down", 0x0002), ("dpad_left", 0x0004), ("dpad_right", 0x0008),
    ("start", 0x0010), ("back", 0x0020), ("left_stick", 0x0040), ("right_stick", 0x0080),
    ("left_shoulder", 0x0100), ("right_shoulder", 0x0200),
    ("a", 0x1000), ("b", 0x2000), ("x", 0x4000), ("y", 0x8000),
];

pub(crate) fn button_mask(name: &str) -> Option<u16> {
    BUTTON_NAMES.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|&(_, bit)| bit)
}

/// XInput has no paddle inputs, so each paddle (right1, left1, right2, left2)
/// is folded into the XInput button mask chosen in settings/controller.json.
static PADDLES: AtomicU64 = AtomicU64::new(0);

pub(crate) fn set_paddles(masks: [u16; 4]) {
    let packed = masks.iter().enumerate().fold(0u64, |acc, (i, &m)| acc | u64::from(m) << (16 * i));
    PADDLES.store(packed, Ordering::Relaxed);
}

fn paddles() -> [u16; 4] {
    let packed = PADDLES.load(Ordering::Relaxed);
    std::array::from_fn(|i| (packed >> (16 * i)) as u16)
}

/// SDL trigger range 0..=32767 back to the XInput byte. SDL expands XInput's
/// byte as b*257 over the full axis, then rescales to 0..=32767, so rounding to
/// nearest returns the original byte exactly.
pub(crate) fn xinput_trigger(value: i16) -> u8 {
    ((u32::from(value.max(0) as u16) * 255 + 16383) / 32767) as u8
}

/// SDL reports stick Y as positive-down by storing `~y`; invert it the same way.
pub(crate) fn xinput_y(value: i16) -> i16 {
    !value
}

pub(crate) struct DevicePacket {
    pub number: u32,
    pub state: XboxState,
    pub subtype: u8,
    /// Who is in the slot. Metadata only: gameplay reads `state`/`subtype`.
    pub kind: Option<Arc<ControllerKind>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeviceError {
    Disconnected,
    State(u32),
    Capabilities(u32),
    /// No device backend could be started.
    #[cfg(not(windows))]
    Unavailable,
}

/// XInput capability identity of one slot: the subtype `xbox::convert` needs
/// and the controller kind built from the same capability read.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct XinputIdentity {
    pub subtype: u8,
    pub kind: Arc<ControllerKind>,
}

/// Device identity is metadata; raw input is still sampled every host frame.
/// Refresh periodically as well as after errors, so hot swaps cannot leave a
/// subtype cached indefinitely even if Windows never exposes a disconnect.
pub(crate) struct CapabilityCache<T = XinputIdentity> {
    value: Option<(T, std::time::Instant)>,
}
impl<T> Default for CapabilityCache<T> {
    fn default() -> Self {
        Self { value: None }
    }
}
impl<T: Clone> CapabilityCache<T> {
    pub(crate) fn invalidate(&mut self) {
        self.value = None;
    }
    #[cfg_attr(not(windows), allow(dead_code))]
    fn get(
        &mut self,
        now: std::time::Instant,
        read: impl FnOnce() -> Result<T, DeviceError>,
    ) -> Result<T, DeviceError> {
        if let Some((subtype, expires)) = &self.value {
            if now < *expires {
                return Ok(subtype.clone());
            }
        }
        self.value = None;
        let subtype = read()?;
        self.value = Some((subtype.clone(), now + std::time::Duration::from_secs(1)));
        Ok(subtype)
    }
}


// skate-sim: the SDL3 and XInput device backends are not part of this crate. Raw pad state
// reaches the simulation through `ControllerInput::sample_raw_for_test`; polling reports no device.
pub(crate) fn poll_cached(
    index: usize,
    _cache: &mut CapabilityCache,
) -> Result<DevicePacket, DeviceError> {
    assert!(index < 4);
    Err(DeviceError::Disconnected)
}

pub(crate) fn poll(index: usize) -> Result<DevicePacket, DeviceError> {
    poll_cached(index, &mut CapabilityCache::default())
}

#[cfg(test)]
mod conversion_tests {
    use super::*;

    /// SDL's XInput expansion: byte*257-32768 on the full axis, then the
    /// gamepad layer rescales -32768..=32767 onto 0..=32767.
    fn sdl_trigger(byte: u8) -> i16 {
        let axis = i32::from(byte) * 257 - 32768;
        ((axis + 32768) * 32767 / 65535) as i16
    }

    #[test]
    fn every_trigger_byte_survives_the_sdl_round_trip() {
        for byte in 0..=255u8 {
            assert_eq!(xinput_trigger(sdl_trigger(byte)), byte);
        }
        assert_eq!(xinput_trigger(-1), 0);
    }

    #[test]
    fn stick_y_inversion_restores_xinput_values_without_overflow() {
        for y in [i16::MIN, -1, 0, 1, 12345, i16::MAX] {
            assert_eq!(xinput_y(!y), y);
        }
        assert_eq!(xinput_y(i16::MIN), i16::MAX);
    }

    #[test]
    fn paddle_masks_round_trip_and_names_use_xinput_bits() {
        set_paddles([0x1000, 0, 0x0100, 0x8000]);
        assert_eq!(paddles(), [0x1000, 0, 0x0100, 0x8000]);
        set_paddles([0; 4]);
        assert_eq!(button_mask("A"), Some(0x1000));
        assert_eq!(button_mask("left_shoulder"), Some(0x0100));
        assert_eq!(button_mask("guide"), None);
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    #[test]
    fn capability_cache_refreshes_and_never_caches_errors() {
        let start = std::time::Instant::now();
        let mut cache = CapabilityCache::<u8>::default();
        assert_eq!(cache.get(start, || Ok(1)), Ok(1));
        assert_eq!(
            cache.get(start + std::time::Duration::from_millis(999), || panic!(
                "redundant capability query"
            )),
            Ok(1)
        );
        assert_eq!(
            cache.get(start + std::time::Duration::from_secs(1), || Ok(2)),
            Ok(2)
        );
        cache.invalidate();
        assert_eq!(
            cache.get(start, || Err(DeviceError::Capabilities(5))),
            Err(DeviceError::Capabilities(5))
        );
        assert_eq!(cache.get(start, || Ok(3)), Ok(3));
        cache.invalidate();
        assert_eq!(cache.get(start, || Ok(4)), Ok(4));
    }
}

