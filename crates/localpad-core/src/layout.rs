//! Versioned controller layout schema and validation. Layouts are untrusted
//! input (users can import JSON files), so validation is strict.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use crate::frame::is_allowed_key_code;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Orientation {
    Landscape,
    Portrait,
    Any,
}

/// What kind of native output a layout drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputMode {
    /// Pointer, scroll wheel and keyboard keys.
    Pointer,
    /// Buttons and sticks translated to keyboard keys.
    Keyboard,
    /// Buttons, sticks and motion streamed over the DSU protocol.
    Dsu,
    /// Native virtual gamepad where the platform supports one.
    Gamepad,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ControlKind {
    /// Momentary button. Binding decides what it emits.
    Button,
    /// Four-way pad emitting dpad_up/down/left/right logical ids.
    Dpad,
    /// Analog stick; id must be `left` or `right`.
    Stick,
    /// Relative pointer surface (drives pointer_delta).
    Touchpad,
    /// Vertical scroll strip (drives scroll_delta).
    Scroll,
    /// Analog trigger; id must be `l2` or `r2`.
    Trigger,
    /// Opens the phone soft keyboard and forwards key transitions.
    Keyboard,
    /// Motion enable / recenter block.
    Motion,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Control {
    #[serde(rename = "type")]
    pub kind: ControlKind,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    /// Center position, normalized 0..1 of the play area.
    pub x: f32,
    pub y: f32,
    /// Diameter / main dimension, normalized 0..1 of the shorter screen edge.
    pub size: f32,
    /// Optional width override for rectangular controls, normalized 0..1.
    #[serde(default)]
    pub width: Option<f32>,
    /// Optional height override, normalized 0..1.
    #[serde(default)]
    pub height: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Layout {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub orientation: Orientation,
    pub output: OutputMode,
    #[serde(default)]
    pub description: Option<String>,
    pub controls: Vec<Control>,
    /// Map of logical control id to binding target, e.g.
    /// `"a": "keyboard:KeyX"`, `"b": "mouse:left"`, `"a": "gamepad:a"`,
    /// `"left": "stickkeys:KeyW,KeyS,KeyA,KeyD"`, `"gyro": "mouse"`.
    #[serde(default)]
    pub bindings: std::collections::BTreeMap<String, String>,
}

pub const LAYOUT_SCHEMA_VERSION: u32 = 1;
pub const MAX_CONTROLS: usize = 64;
pub const MAX_LAYOUT_BYTES: usize = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum LayoutError {
    #[error("unsupported layout schema version {0}")]
    UnsupportedVersion(u32),
    #[error("layout json is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("layout is too large")]
    TooLarge,
    #[error("layout has too many controls")]
    TooManyControls,
    #[error("layout id must be lowercase alphanumeric with dashes: {0:?}")]
    BadId(String),
    #[error("control {0:?} is out of the 0..1 coordinate range")]
    BadGeometry(String),
    #[error("duplicate control id {0:?}")]
    DuplicateId(String),
    #[error("control kind {0:?} requires id {1}")]
    BadControlId(String, &'static str),
    #[error("binding {0:?} -> {1:?} is not a valid target")]
    BadBinding(String, String),
}

fn valid_slug(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// Check a binding target string without applying it.
pub fn is_valid_binding_target(target: &str) -> bool {
    if target == "none" {
        return true;
    }
    match target.split_once(':') {
        Some(("keyboard", code)) => is_allowed_key_code(code),
        Some(("mouse", btn)) => matches!(btn, "left" | "right" | "middle"),
        Some(("gamepad", id)) => crate::frame::GamepadButton::from_id(id).is_some()
            || matches!(id, "left_stick" | "right_stick" | "l2" | "r2"),
        Some(("stickkeys", codes)) => {
            let parts: Vec<&str> = codes.split(',').collect();
            parts.len() == 4 && parts.iter().all(|c| is_allowed_key_code(c))
        }
        None => matches!(target, "mouse" | "right_stick" | "steer" | "dsu" | "stickmouse"),
        _ => false,
    }
}

impl Layout {
    pub fn parse(bytes: &[u8]) -> Result<Layout, LayoutError> {
        if bytes.len() > MAX_LAYOUT_BYTES {
            return Err(LayoutError::TooLarge);
        }
        let layout: Layout = serde_json::from_slice(bytes)?;
        layout.validate()?;
        Ok(layout)
    }

    pub fn validate(&self) -> Result<(), LayoutError> {
        if self.schema_version != LAYOUT_SCHEMA_VERSION {
            return Err(LayoutError::UnsupportedVersion(self.schema_version));
        }
        if !valid_slug(&self.id) {
            return Err(LayoutError::BadId(self.id.clone()));
        }
        if self.controls.len() > MAX_CONTROLS {
            return Err(LayoutError::TooManyControls);
        }
        let mut ids = HashSet::new();
        for control in &self.controls {
            let label = control
                .id
                .clone()
                .or_else(|| control.label.clone())
                .unwrap_or_else(|| format!("{:?}", control.kind));
            let in_range = |v: f32| (0.0..=1.0).contains(&v) && v.is_finite();
            if !in_range(control.x)
                || !in_range(control.y)
                || !in_range(control.size)
                || !control.width.is_none_or(in_range)
                || !control.height.is_none_or(in_range)
            {
                return Err(LayoutError::BadGeometry(label));
            }
            if let Some(id) = &control.id {
                if !valid_slug(id) {
                    return Err(LayoutError::BadId(id.clone()));
                }
                if !ids.insert(id.clone()) {
                    return Err(LayoutError::DuplicateId(id.clone()));
                }
            }
            match control.kind {
                ControlKind::Button if control.id.is_none() => {
                    return Err(LayoutError::BadControlId(label, "a unique id"));
                }
                ControlKind::Stick => {
                    if !matches!(control.id.as_deref(), Some("left") | Some("right")) {
                        return Err(LayoutError::BadControlId(label, "`left` or `right`"));
                    }
                }
                ControlKind::Trigger => {
                    if !matches!(control.id.as_deref(), Some("l2") | Some("r2")) {
                        return Err(LayoutError::BadControlId(label, "`l2` or `r2`"));
                    }
                }
                _ => {}
            }
        }
        for (source, target) in &self.bindings {
            if !valid_slug(source) || !is_valid_binding_target(target) {
                return Err(LayoutError::BadBinding(source.clone(), target.clone()));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_layout() -> serde_json::Value {
        serde_json::json!({
            "schemaVersion": 1,
            "id": "test-pad",
            "name": "Test",
            "orientation": "landscape",
            "output": "keyboard",
            "controls": [
                { "type": "dpad", "x": 0.16, "y": 0.62, "size": 0.25 },
                { "type": "button", "id": "a", "label": "A", "x": 0.82, "y": 0.6, "size": 0.12 }
            ],
            "bindings": { "a": "keyboard:KeyX", "dpad_up": "keyboard:ArrowUp" }
        })
    }

    #[test]
    fn accepts_valid_layout() {
        let bytes = serde_json::to_vec(&base_layout()).unwrap();
        Layout::parse(&bytes).unwrap();
    }

    #[test]
    fn rejects_bad_schema_version() {
        let mut v = base_layout();
        v["schemaVersion"] = 2.into();
        let err = Layout::parse(&serde_json::to_vec(&v).unwrap()).unwrap_err();
        assert!(matches!(err, LayoutError::UnsupportedVersion(2)));
    }

    #[test]
    fn rejects_out_of_range_geometry() {
        let mut v = base_layout();
        v["controls"][1]["x"] = serde_json::json!(1.5);
        assert!(Layout::parse(&serde_json::to_vec(&v).unwrap()).is_err());
    }

    #[test]
    fn rejects_bad_binding_target() {
        let mut v = base_layout();
        v["bindings"]["a"] = "shell:reboot".into();
        assert!(Layout::parse(&serde_json::to_vec(&v).unwrap()).is_err());
        let mut v = base_layout();
        v["bindings"]["a"] = "keyboard:NotAKey".into();
        assert!(Layout::parse(&serde_json::to_vec(&v).unwrap()).is_err());
    }

    #[test]
    fn rejects_duplicate_ids() {
        let mut v = base_layout();
        v["controls"][0] = serde_json::json!({
            "type": "button", "id": "a", "x": 0.5, "y": 0.5, "size": 0.1
        });
        assert!(matches!(
            Layout::parse(&serde_json::to_vec(&v).unwrap()).unwrap_err(),
            LayoutError::DuplicateId(_)
        ));
    }

    #[test]
    fn binding_targets() {
        assert!(is_valid_binding_target("keyboard:KeyA"));
        assert!(is_valid_binding_target("mouse:left"));
        assert!(is_valid_binding_target("gamepad:a"));
        assert!(is_valid_binding_target("gamepad:left_stick"));
        assert!(is_valid_binding_target("stickkeys:KeyW,KeyS,KeyA,KeyD"));
        assert!(is_valid_binding_target("none"));
        assert!(!is_valid_binding_target("stickkeys:KeyW,KeyS"));
        assert!(!is_valid_binding_target("exec:ls"));
    }
}
