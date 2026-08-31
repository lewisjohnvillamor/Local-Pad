//! The normalized input frame shared by every layout and platform, plus the
//! validation rules from the protocol: clamp everything, reject NaN and
//! infinity, reject unknown protocol versions, never trust client time.

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u16 = 1;

/// Hard limit on how large a serialized frame may be before parsing.
pub const MAX_FRAME_BYTES: usize = 4096;

/// Logical gamepad buttons carried in the `buttons` bitset of a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GamepadButton {
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
    A,
    B,
    X,
    Y,
    Start,
    Select,
    L1,
    R1,
    L2,
    R2,
    L3,
    R3,
    Guide,
}

impl GamepadButton {
    pub const ALL: [GamepadButton; 17] = [
        GamepadButton::DpadUp,
        GamepadButton::DpadDown,
        GamepadButton::DpadLeft,
        GamepadButton::DpadRight,
        GamepadButton::A,
        GamepadButton::B,
        GamepadButton::X,
        GamepadButton::Y,
        GamepadButton::Start,
        GamepadButton::Select,
        GamepadButton::L1,
        GamepadButton::R1,
        GamepadButton::L2,
        GamepadButton::R2,
        GamepadButton::L3,
        GamepadButton::R3,
        GamepadButton::Guide,
    ];

    pub fn bit(self) -> u32 {
        1 << (self as u32)
    }

    pub fn from_id(id: &str) -> Option<Self> {
        serde_json::from_value(serde_json::Value::String(id.to_string())).ok()
    }

    /// Mask of every bit this protocol version understands.
    pub fn valid_mask() -> u32 {
        Self::ALL.iter().fold(0, |m, b| m | b.bit())
    }
}

/// Mouse buttons carried in explicit transition events, never in the bitset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// A keyboard key identified by its W3C `KeyboardEvent.code` value
/// (for example `KeyA`, `Digit1`, `Enter`, `ArrowLeft`, `MediaPlayPause`).
/// Only codes on the allowlist are accepted from the network.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Key(pub String);

impl Key {
    pub fn new(code: &str) -> Option<Self> {
        if is_allowed_key_code(code) {
            Some(Key(code.to_string()))
        } else {
            None
        }
    }

    pub fn code(&self) -> &str {
        &self.0
    }
}

/// Allowlist of key codes a phone may inject. Anything outside this list is
/// rejected server-side so a compromised page cannot type arbitrary control
/// sequences we never mapped.
pub fn is_allowed_key_code(code: &str) -> bool {
    const FIXED: &[&str] = &[
        "Enter", "Escape", "Backspace", "Tab", "Space", "Minus", "Equal", "BracketLeft",
        "BracketRight", "Backslash", "Semicolon", "Quote", "Backquote", "Comma", "Period",
        "Slash", "CapsLock", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End",
        "PageUp", "PageDown", "Insert", "Delete", "ShiftLeft", "ShiftRight", "ControlLeft",
        "ControlRight", "AltLeft", "AltRight", "MetaLeft", "MetaRight", "ContextMenu",
        "AudioVolumeUp", "AudioVolumeDown", "AudioVolumeMute", "MediaPlayPause",
        "MediaTrackNext", "MediaTrackPrevious", "MediaStop", "BrightnessUp", "BrightnessDown",
        "PrintScreen", "NumpadEnter", "NumpadAdd", "NumpadSubtract", "NumpadMultiply",
        "NumpadDivide", "NumpadDecimal",
    ];
    if FIXED.contains(&code) {
        return true;
    }
    if let Some(rest) = code.strip_prefix("Key") {
        return rest.len() == 1 && rest.chars().all(|c| c.is_ascii_uppercase());
    }
    if let Some(rest) = code.strip_prefix("Digit") {
        return rest.len() == 1 && rest.chars().all(|c| c.is_ascii_digit());
    }
    if let Some(rest) = code.strip_prefix("Numpad") {
        return rest.len() == 1 && rest.chars().all(|c| c.is_ascii_digit());
    }
    if let Some(rest) = code.strip_prefix("F") {
        return matches!(rest.parse::<u8>(), Ok(n) if (1..=24).contains(&n));
    }
    false
}

/// One normalized state snapshot from a controller.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputFrame {
    pub protocol_version: u16,
    #[serde(default)]
    pub sequence: u64,
    /// Client wall clock in milliseconds. Advisory only: used for latency
    /// display, never for authorization or ordering.
    #[serde(default)]
    pub client_time_ms: f64,
    #[serde(default)]
    pub buttons: u32,
    #[serde(default)]
    pub left_stick: [f32; 2],
    #[serde(default)]
    pub right_stick: [f32; 2],
    #[serde(default)]
    pub triggers: [f32; 2],
    #[serde(default)]
    pub pointer_delta: [f32; 2],
    #[serde(default)]
    pub scroll_delta: [f32; 2],
    /// Device orientation as a unit quaternion in [x, y, z, w] order.
    #[serde(default)]
    pub orientation: Option<[f32; 4]>,
    /// Angular velocity in degrees per second: [pitch, yaw, roll].
    #[serde(default)]
    pub angular_velocity: Option<[f32; 3]>,
    /// Acceleration including gravity, in g: [x, y, z].
    #[serde(default)]
    pub acceleration: Option<[f32; 3]>,
}

impl Default for InputFrame {
    fn default() -> Self {
        InputFrame {
            protocol_version: PROTOCOL_VERSION,
            sequence: 0,
            client_time_ms: 0.0,
            buttons: 0,
            left_stick: [0.0; 2],
            right_stick: [0.0; 2],
            triggers: [0.0; 2],
            pointer_delta: [0.0; 2],
            scroll_delta: [0.0; 2],
            orientation: None,
            angular_velocity: None,
            acceleration: None,
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FrameError {
    #[error("unsupported protocol version {0}")]
    UnsupportedVersion(u16),
    #[error("frame contains a non-finite number")]
    NonFinite,
    #[error("frame sets unknown button bits")]
    UnknownButtons,
}

/// The largest single-frame pointer or scroll movement we accept, in
/// device-independent pixels. Anything bigger is a hostile or broken client.
const MAX_DELTA: f32 = 512.0;
/// Angular velocity ceiling in degrees per second.
const MAX_ANGULAR: f32 = 4000.0;
/// Acceleration ceiling in g.
const MAX_ACCEL: f32 = 16.0;

fn clamp_all(values: &mut [f32], min: f32, max: f32) -> Result<(), FrameError> {
    for v in values {
        if !v.is_finite() {
            return Err(FrameError::NonFinite);
        }
        *v = v.clamp(min, max);
    }
    Ok(())
}

impl InputFrame {
    /// Validate and clamp a frame in place. Returns an error for anything a
    /// well-behaved client would never send; callers must drop such frames.
    pub fn sanitize(&mut self) -> Result<(), FrameError> {
        if self.protocol_version != PROTOCOL_VERSION {
            return Err(FrameError::UnsupportedVersion(self.protocol_version));
        }
        if self.buttons & !GamepadButton::valid_mask() != 0 {
            return Err(FrameError::UnknownButtons);
        }
        if !self.client_time_ms.is_finite() {
            return Err(FrameError::NonFinite);
        }
        clamp_all(&mut self.left_stick, -1.0, 1.0)?;
        clamp_all(&mut self.right_stick, -1.0, 1.0)?;
        clamp_all(&mut self.triggers, 0.0, 1.0)?;
        clamp_all(&mut self.pointer_delta, -MAX_DELTA, MAX_DELTA)?;
        clamp_all(&mut self.scroll_delta, -MAX_DELTA, MAX_DELTA)?;
        if let Some(q) = self.orientation.as_mut() {
            clamp_all(q, -1.0, 1.0)?;
            let norm = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
            if norm < 1e-3 {
                // A zero quaternion carries no orientation; treat as absent.
                self.orientation = None;
            } else {
                for c in q.iter_mut() {
                    *c /= norm;
                }
            }
        }
        if let Some(w) = self.angular_velocity.as_mut() {
            clamp_all(w, -MAX_ANGULAR, MAX_ANGULAR)?;
        }
        if let Some(a) = self.acceleration.as_mut() {
            clamp_all(a, -MAX_ACCEL, MAX_ACCEL)?;
        }
        Ok(())
    }

    pub fn is_pressed(&self, button: GamepadButton) -> bool {
        self.buttons & button.bit() != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_have_distinct_bits() {
        let mut seen = 0u32;
        for b in GamepadButton::ALL {
            assert_eq!(seen & b.bit(), 0, "bit collision at {b:?}");
            seen |= b.bit();
        }
        assert_eq!(seen, GamepadButton::valid_mask());
    }

    #[test]
    fn sanitize_clamps_ranges() {
        let mut f = InputFrame {
            left_stick: [-3.0, 9.0],
            triggers: [-1.0, 7.0],
            pointer_delta: [10_000.0, -10_000.0],
            ..Default::default()
        };
        f.sanitize().unwrap();
        assert_eq!(f.left_stick, [-1.0, 1.0]);
        assert_eq!(f.triggers, [0.0, 1.0]);
        assert_eq!(f.pointer_delta, [MAX_DELTA, -MAX_DELTA]);
    }

    #[test]
    fn sanitize_rejects_non_finite() {
        let mut f = InputFrame {
            left_stick: [f32::NAN, 0.0],
            ..Default::default()
        };
        assert_eq!(f.sanitize(), Err(FrameError::NonFinite));
        let mut f = InputFrame {
            scroll_delta: [f32::INFINITY, 0.0],
            ..Default::default()
        };
        assert_eq!(f.sanitize(), Err(FrameError::NonFinite));
    }

    #[test]
    fn sanitize_rejects_unknown_version_and_buttons() {
        let mut f = InputFrame {
            protocol_version: 9,
            ..Default::default()
        };
        assert_eq!(f.sanitize(), Err(FrameError::UnsupportedVersion(9)));
        let mut f = InputFrame {
            buttons: 1 << 31,
            ..Default::default()
        };
        assert_eq!(f.sanitize(), Err(FrameError::UnknownButtons));
    }

    #[test]
    fn sanitize_normalizes_orientation() {
        let mut f = InputFrame {
            orientation: Some([0.0, 0.0, 0.0, 0.5]),
            ..Default::default()
        };
        f.sanitize().unwrap();
        assert_eq!(f.orientation, Some([0.0, 0.0, 0.0, 1.0]));

        let mut f = InputFrame {
            orientation: Some([0.0, 0.0, 0.0, 0.0]),
            ..Default::default()
        };
        f.sanitize().unwrap();
        assert_eq!(f.orientation, None);
    }

    #[test]
    fn key_allowlist() {
        assert!(Key::new("KeyA").is_some());
        assert!(Key::new("Digit7").is_some());
        assert!(Key::new("F12").is_some());
        assert!(Key::new("MediaPlayPause").is_some());
        assert!(Key::new("Key1").is_none());
        assert!(Key::new("F99").is_none());
        assert!(Key::new("Power").is_none());
        assert!(Key::new("rm -rf").is_none());
    }

    #[test]
    fn json_shape_is_stable() {
        let f = InputFrame::default();
        let v = serde_json::to_value(&f).unwrap();
        assert!(v.get("protocolVersion").is_some());
        assert!(v.get("leftStick").is_some());
        assert!(v.get("pointerDelta").is_some());
    }
}
