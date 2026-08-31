//! The mapping engine turns sanitized client frames and transition events
//! into concrete output actions according to the active layout's bindings.
//! It owns edge detection (button down/up), tracks everything currently
//! held, and can synthesize the release of all held inputs at any moment.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::frame::{GamepadButton, InputFrame, Key, MouseButton};
use crate::layout::{Layout, OutputMode};
use crate::motion::{MotionProcessor, MotionSample};

/// A discrete transition an output adapter must perform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OutputEvent {
    Key { key: Key, down: bool },
    MouseButton { button: MouseButton, down: bool },
}

/// Continuous state for one tick, already mapped to output semantics.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputFrame {
    pub mouse_delta: [f32; 2],
    pub scroll_delta: [f32; 2],
    /// Gamepad state for DSU / virtual controller outputs.
    pub gamepad_buttons: u32,
    pub left_stick: [f32; 2],
    pub right_stick: [f32; 2],
    pub triggers: [f32; 2],
    pub motion: Option<MotionSample>,
}

/// How this layout uses gyro data, from the `gyro` binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GyroTarget {
    None,
    Mouse,
    RightStick,
    Steer,
    Dsu,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ButtonTarget {
    Key(Key),
    Mouse(MouseButton),
    Gamepad(GamepadButton),
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StickTarget {
    Gamepad,
    /// Up, down, left, right keys with a 0.5 press threshold.
    Keys([Key; 4]),
    /// Stick deflection moves the pointer at a fixed top speed.
    Mouse,
    None,
}

/// Pointer speed for stick-to-mouse mappings, in pixels per second at
/// full deflection.
const STICK_MOUSE_SPEED: f32 = 700.0;

pub struct MappingEngine {
    layout_id: String,
    output_mode: OutputMode,
    buttons: BTreeMap<GamepadButton, ButtonTarget>,
    sticks: [StickTarget; 2],
    gyro: GyroTarget,
    pub motion: MotionProcessor,
    prev_buttons: u32,
    prev_stick_keys: [u8; 2],
    held_keys: HashSet<Key>,
    held_mouse: HashSet<MouseButton>,
}

fn parse_button_target(target: &str) -> ButtonTarget {
    match target.split_once(':') {
        Some(("keyboard", code)) => Key::new(code).map(ButtonTarget::Key).unwrap_or(ButtonTarget::None),
        Some(("mouse", "left")) => ButtonTarget::Mouse(MouseButton::Left),
        Some(("mouse", "right")) => ButtonTarget::Mouse(MouseButton::Right),
        Some(("mouse", "middle")) => ButtonTarget::Mouse(MouseButton::Middle),
        Some(("gamepad", id)) => GamepadButton::from_id(id)
            .map(ButtonTarget::Gamepad)
            .unwrap_or(ButtonTarget::None),
        _ => ButtonTarget::None,
    }
}

impl MappingEngine {
    pub fn new(layout: &Layout) -> Self {
        let mut buttons = BTreeMap::new();
        let mut sticks = [StickTarget::None, StickTarget::None];
        let mut gyro = GyroTarget::None;

        // In gamepad-style outputs, unbound buttons pass straight through.
        let passthrough = matches!(layout.output, OutputMode::Dsu | OutputMode::Gamepad);
        if passthrough {
            for b in GamepadButton::ALL {
                buttons.insert(b, ButtonTarget::Gamepad(b));
            }
            sticks = [StickTarget::Gamepad, StickTarget::Gamepad];
            gyro = GyroTarget::Dsu;
        }

        for (source, target) in &layout.bindings {
            if source == "gyro" {
                gyro = match target.as_str() {
                    "mouse" => GyroTarget::Mouse,
                    "right_stick" => GyroTarget::RightStick,
                    "steer" => GyroTarget::Steer,
                    "dsu" => GyroTarget::Dsu,
                    _ => GyroTarget::None,
                };
                continue;
            }
            let stick_slot = match source.as_str() {
                "left" => Some(0),
                "right" => Some(1),
                _ => None,
            };
            if let Some(slot) = stick_slot {
                sticks[slot] = match target.split_once(':') {
                    None if target == "stickmouse" => StickTarget::Mouse,
                    Some(("gamepad", _)) => StickTarget::Gamepad,
                    Some(("stickkeys", codes)) => {
                        let keys: Vec<Key> =
                            codes.split(',').filter_map(Key::new).collect();
                        match <[Key; 4]>::try_from(keys) {
                            Ok(arr) => StickTarget::Keys(arr),
                            Err(_) => StickTarget::None,
                        }
                    }
                    _ => StickTarget::None,
                };
                continue;
            }
            if let Some(button) = GamepadButton::from_id(source) {
                buttons.insert(button, parse_button_target(target));
            }
        }

        MappingEngine {
            layout_id: layout.id.clone(),
            output_mode: layout.output,
            buttons,
            sticks,
            gyro,
            motion: MotionProcessor::default(),
            prev_buttons: 0,
            prev_stick_keys: [0; 2],
            held_keys: HashSet::new(),
            held_mouse: HashSet::new(),
        }
    }

    pub fn layout_id(&self) -> &str {
        &self.layout_id
    }

    pub fn output_mode(&self) -> OutputMode {
        self.output_mode
    }

    fn press_key(&mut self, key: Key, events: &mut Vec<OutputEvent>) {
        if self.held_keys.insert(key.clone()) {
            events.push(OutputEvent::Key { key, down: true });
        }
    }

    fn release_key(&mut self, key: &Key, events: &mut Vec<OutputEvent>) {
        if self.held_keys.remove(key) {
            events.push(OutputEvent::Key { key: key.clone(), down: false });
        }
    }

    fn press_mouse(&mut self, button: MouseButton, events: &mut Vec<OutputEvent>) {
        if self.held_mouse.insert(button) {
            events.push(OutputEvent::MouseButton { button, down: true });
        }
    }

    fn release_mouse(&mut self, button: MouseButton, events: &mut Vec<OutputEvent>) {
        if self.held_mouse.remove(&button) {
            events.push(OutputEvent::MouseButton { button, down: false });
        }
    }

    /// Process one sanitized frame. `dt` is the elapsed time since the last
    /// frame in seconds, used for rate-based motion mappings.
    pub fn process_frame(&mut self, frame: &InputFrame, dt: f32) -> (OutputFrame, Vec<OutputEvent>) {
        let mut events = Vec::new();
        let mut out = OutputFrame {
            mouse_delta: frame.pointer_delta,
            scroll_delta: frame.scroll_delta,
            triggers: frame.triggers,
            ..Default::default()
        };

        // Button edges.
        let changed = frame.buttons ^ self.prev_buttons;
        if changed != 0 {
            for b in GamepadButton::ALL {
                if changed & b.bit() == 0 {
                    continue;
                }
                let down = frame.buttons & b.bit() != 0;
                match self.buttons.get(&b).cloned() {
                    Some(ButtonTarget::Key(key)) => {
                        if down {
                            self.press_key(key, &mut events);
                        } else {
                            self.release_key(&key, &mut events);
                        }
                    }
                    Some(ButtonTarget::Mouse(btn)) => {
                        if down {
                            self.press_mouse(btn, &mut events);
                        } else {
                            self.release_mouse(btn, &mut events);
                        }
                    }
                    Some(ButtonTarget::Gamepad(_)) | Some(ButtonTarget::None) | None => {}
                }
            }
        }
        self.prev_buttons = frame.buttons;

        // Gamepad passthrough of the bitset, remapped through bindings.
        for b in GamepadButton::ALL {
            if frame.buttons & b.bit() != 0 {
                if let Some(ButtonTarget::Gamepad(mapped)) = self.buttons.get(&b) {
                    out.gamepad_buttons |= mapped.bit();
                }
            }
        }

        // Sticks.
        let sticks = [frame.left_stick, frame.right_stick];
        for slot in 0..2 {
            match self.sticks[slot].clone() {
                StickTarget::Gamepad => {
                    if slot == 0 {
                        out.left_stick = sticks[0];
                    } else {
                        out.right_stick = sticks[1];
                    }
                }
                StickTarget::Keys(keys) => {
                    let [x, y] = sticks[slot];
                    // Bits: up, down, left, right.
                    let state = (u8::from(y < -0.5))
                        | (u8::from(y > 0.5) << 1)
                        | (u8::from(x < -0.5) << 2)
                        | (u8::from(x > 0.5) << 3);
                    let prev = self.prev_stick_keys[slot];
                    for (i, key) in keys.iter().enumerate() {
                        let now = state & (1 << i) != 0;
                        let was = prev & (1 << i) != 0;
                        if now && !was {
                            self.press_key(key.clone(), &mut events);
                        } else if !now && was {
                            self.release_key(key, &mut events);
                        }
                    }
                    self.prev_stick_keys[slot] = state;
                }
                StickTarget::Mouse => {
                    let [x, y] = sticks[slot];
                    // Quadratic response keeps fine motion controllable.
                    let curve = |v: f32| v.abs() * v;
                    out.mouse_delta[0] += curve(x) * STICK_MOUSE_SPEED * dt;
                    out.mouse_delta[1] += curve(y) * STICK_MOUSE_SPEED * dt;
                }
                StickTarget::None => {}
            }
        }

        // Motion.
        if let Some(sample) =
            self.motion
                .process(frame.orientation, frame.angular_velocity, frame.acceleration)
        {
            match self.gyro {
                GyroTarget::Mouse => {
                    let [dx, dy] = self.motion.to_mouse_delta(&sample, dt);
                    out.mouse_delta[0] += dx;
                    out.mouse_delta[1] += dy;
                }
                GyroTarget::RightStick => {
                    out.right_stick = self.motion.to_stick(&sample);
                }
                GyroTarget::Steer => {
                    out.left_stick[0] = self.motion.to_steering(&sample);
                }
                GyroTarget::Dsu => {
                    out.motion = Some(sample);
                }
                GyroTarget::None => {}
            }
        }

        (out, events)
    }

    /// Handle an explicit key transition from the phone's soft keyboard.
    pub fn process_key(&mut self, key: Key, down: bool) -> Vec<OutputEvent> {
        let mut events = Vec::new();
        if down {
            self.press_key(key, &mut events);
        } else {
            self.release_key(&key, &mut events);
        }
        events
    }

    /// Handle an explicit mouse button transition.
    pub fn process_mouse_button(&mut self, button: MouseButton, down: bool) -> Vec<OutputEvent> {
        let mut events = Vec::new();
        if down {
            self.press_mouse(button, &mut events);
        } else {
            self.release_mouse(button, &mut events);
        }
        events
    }

    pub fn recenter(&mut self) {
        self.motion.recenter();
    }

    /// Release everything currently held and reset state. Safe to call any
    /// number of times; the second call returns no events.
    pub fn release_all(&mut self) -> Vec<OutputEvent> {
        let mut events = Vec::new();
        let keys: Vec<Key> = self.held_keys.drain().collect();
        for key in keys {
            events.push(OutputEvent::Key { key, down: false });
        }
        let buttons: Vec<MouseButton> = self.held_mouse.drain().collect();
        for button in buttons {
            events.push(OutputEvent::MouseButton { button, down: false });
        }
        self.prev_buttons = 0;
        self.prev_stick_keys = [0; 2];
        self.motion.reset();
        events
    }

    pub fn held_count(&self) -> usize {
        self.held_keys.len() + self.held_mouse.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Layout, LAYOUT_SCHEMA_VERSION};

    fn keyboard_layout() -> Layout {
        serde_json::from_value(serde_json::json!({
            "schemaVersion": LAYOUT_SCHEMA_VERSION,
            "id": "test",
            "name": "Test",
            "orientation": "landscape",
            "output": "keyboard",
            "controls": [],
            "bindings": {
                "a": "keyboard:KeyX",
                "b": "mouse:left",
                "dpad_up": "keyboard:ArrowUp",
                "left": "stickkeys:KeyW,KeyS,KeyA,KeyD",
                "gyro": "mouse"
            }
        }))
        .unwrap()
    }

    fn frame_with_buttons(buttons: u32) -> InputFrame {
        InputFrame { buttons, ..Default::default() }
    }

    #[test]
    fn button_edges_emit_transitions_once() {
        let mut engine = MappingEngine::new(&keyboard_layout());
        let down = frame_with_buttons(GamepadButton::A.bit());
        let (_, events) = engine.process_frame(&down, 0.016);
        assert_eq!(events, vec![OutputEvent::Key { key: Key::new("KeyX").unwrap(), down: true }]);
        // Held: no repeat events.
        let (_, events) = engine.process_frame(&down, 0.016);
        assert!(events.is_empty());
        let (_, events) = engine.process_frame(&frame_with_buttons(0), 0.016);
        assert_eq!(events, vec![OutputEvent::Key { key: Key::new("KeyX").unwrap(), down: false }]);
    }

    #[test]
    fn stick_threshold_maps_to_keys() {
        let mut engine = MappingEngine::new(&keyboard_layout());
        let mut f = InputFrame::default();
        f.left_stick = [0.0, -0.9];
        let (_, events) = engine.process_frame(&f, 0.016);
        assert_eq!(events, vec![OutputEvent::Key { key: Key::new("KeyW").unwrap(), down: true }]);
        f.left_stick = [0.0, 0.0];
        let (_, events) = engine.process_frame(&f, 0.016);
        assert_eq!(events, vec![OutputEvent::Key { key: Key::new("KeyW").unwrap(), down: false }]);
    }

    #[test]
    fn release_all_is_idempotent() {
        let mut engine = MappingEngine::new(&keyboard_layout());
        let (_, _) = engine.process_frame(
            &frame_with_buttons(GamepadButton::A.bit() | GamepadButton::B.bit()),
            0.016,
        );
        assert_eq!(engine.held_count(), 2);
        let events = engine.release_all();
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|e| matches!(
            e,
            OutputEvent::Key { down: false, .. } | OutputEvent::MouseButton { down: false, .. }
        )));
        assert!(engine.release_all().is_empty());
        assert_eq!(engine.held_count(), 0);
    }

    #[test]
    fn dsu_layout_passes_gamepad_state_through() {
        let layout: Layout = serde_json::from_value(serde_json::json!({
            "schemaVersion": LAYOUT_SCHEMA_VERSION,
            "id": "dsu",
            "name": "DSU",
            "orientation": "landscape",
            "output": "dsu",
            "controls": [],
            "bindings": {}
        }))
        .unwrap();
        let mut engine = MappingEngine::new(&layout);
        let mut f = frame_with_buttons(GamepadButton::A.bit());
        f.left_stick = [0.5, -0.5];
        f.orientation = Some([0.0, 0.0, 0.0, 1.0]);
        let (out, events) = engine.process_frame(&f, 0.016);
        assert!(events.is_empty(), "gamepad output emits no key events");
        assert_eq!(out.gamepad_buttons, GamepadButton::A.bit());
        assert_eq!(out.left_stick, [0.5, -0.5]);
        assert!(out.motion.is_some());
    }

    #[test]
    fn duplicate_key_press_from_two_sources_releases_once() {
        let mut engine = MappingEngine::new(&keyboard_layout());
        let events = engine.process_key(Key::new("KeyQ").unwrap(), true);
        assert_eq!(events.len(), 1);
        let events = engine.process_key(Key::new("KeyQ").unwrap(), true);
        assert!(events.is_empty(), "double press does not re-emit");
        let events = engine.process_key(Key::new("KeyQ").unwrap(), false);
        assert_eq!(events.len(), 1);
    }
}
