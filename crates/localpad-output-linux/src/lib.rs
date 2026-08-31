//! Linux mouse and keyboard output through /dev/uinput. Requires either
//! root (discouraged) or a udev rule granting the user access to uinput;
//! `localpad doctor` explains the setup.

#[cfg(target_os = "linux")]
mod uinput_impl {
    use anyhow::Context;
    use evdev::uinput::{VirtualDevice, VirtualDeviceBuilder};
    use evdev::{AttributeSet, EventType, InputEvent, Key as EvKey, RelativeAxisType};
    use localpad_core::frame::{Key, MouseButton};
    use localpad_core::mapping::{OutputEvent, OutputFrame};
    use localpad_core::output::{InputOutput, OutputCapabilities};
    use std::collections::HashSet;

    /// Map a W3C KeyboardEvent.code to a Linux key code.
    fn map_key(code: &str) -> Option<EvKey> {
        if let Some(rest) = code.strip_prefix("Key") {
            let c = rest.chars().next()?;
            return Some(match c {
                'A' => EvKey::KEY_A, 'B' => EvKey::KEY_B, 'C' => EvKey::KEY_C,
                'D' => EvKey::KEY_D, 'E' => EvKey::KEY_E, 'F' => EvKey::KEY_F,
                'G' => EvKey::KEY_G, 'H' => EvKey::KEY_H, 'I' => EvKey::KEY_I,
                'J' => EvKey::KEY_J, 'K' => EvKey::KEY_K, 'L' => EvKey::KEY_L,
                'M' => EvKey::KEY_M, 'N' => EvKey::KEY_N, 'O' => EvKey::KEY_O,
                'P' => EvKey::KEY_P, 'Q' => EvKey::KEY_Q, 'R' => EvKey::KEY_R,
                'S' => EvKey::KEY_S, 'T' => EvKey::KEY_T, 'U' => EvKey::KEY_U,
                'V' => EvKey::KEY_V, 'W' => EvKey::KEY_W, 'X' => EvKey::KEY_X,
                'Y' => EvKey::KEY_Y, 'Z' => EvKey::KEY_Z,
                _ => return None,
            });
        }
        if let Some(rest) = code.strip_prefix("Digit") {
            let c = rest.chars().next()?;
            return Some(match c {
                '1' => EvKey::KEY_1, '2' => EvKey::KEY_2, '3' => EvKey::KEY_3,
                '4' => EvKey::KEY_4, '5' => EvKey::KEY_5, '6' => EvKey::KEY_6,
                '7' => EvKey::KEY_7, '8' => EvKey::KEY_8, '9' => EvKey::KEY_9,
                '0' => EvKey::KEY_0,
                _ => return None,
            });
        }
        Some(match code {
            "Enter" => EvKey::KEY_ENTER,
            "Escape" => EvKey::KEY_ESC,
            "Backspace" => EvKey::KEY_BACKSPACE,
            "Tab" => EvKey::KEY_TAB,
            "Space" => EvKey::KEY_SPACE,
            "Minus" => EvKey::KEY_MINUS,
            "Equal" => EvKey::KEY_EQUAL,
            "BracketLeft" => EvKey::KEY_LEFTBRACE,
            "BracketRight" => EvKey::KEY_RIGHTBRACE,
            "Backslash" => EvKey::KEY_BACKSLASH,
            "Semicolon" => EvKey::KEY_SEMICOLON,
            "Quote" => EvKey::KEY_APOSTROPHE,
            "Backquote" => EvKey::KEY_GRAVE,
            "Comma" => EvKey::KEY_COMMA,
            "Period" => EvKey::KEY_DOT,
            "Slash" => EvKey::KEY_SLASH,
            "CapsLock" => EvKey::KEY_CAPSLOCK,
            "ArrowUp" => EvKey::KEY_UP,
            "ArrowDown" => EvKey::KEY_DOWN,
            "ArrowLeft" => EvKey::KEY_LEFT,
            "ArrowRight" => EvKey::KEY_RIGHT,
            "Home" => EvKey::KEY_HOME,
            "End" => EvKey::KEY_END,
            "PageUp" => EvKey::KEY_PAGEUP,
            "PageDown" => EvKey::KEY_PAGEDOWN,
            "Insert" => EvKey::KEY_INSERT,
            "Delete" => EvKey::KEY_DELETE,
            "ShiftLeft" => EvKey::KEY_LEFTSHIFT,
            "ShiftRight" => EvKey::KEY_RIGHTSHIFT,
            "ControlLeft" => EvKey::KEY_LEFTCTRL,
            "ControlRight" => EvKey::KEY_RIGHTCTRL,
            "AltLeft" => EvKey::KEY_LEFTALT,
            "AltRight" => EvKey::KEY_RIGHTALT,
            "MetaLeft" => EvKey::KEY_LEFTMETA,
            "MetaRight" => EvKey::KEY_RIGHTMETA,
            "ContextMenu" => EvKey::KEY_COMPOSE,
            "AudioVolumeUp" => EvKey::KEY_VOLUMEUP,
            "AudioVolumeDown" => EvKey::KEY_VOLUMEDOWN,
            "AudioVolumeMute" => EvKey::KEY_MUTE,
            "MediaPlayPause" => EvKey::KEY_PLAYPAUSE,
            "MediaTrackNext" => EvKey::KEY_NEXTSONG,
            "MediaTrackPrevious" => EvKey::KEY_PREVIOUSSONG,
            "MediaStop" => EvKey::KEY_STOPCD,
            "F1" => EvKey::KEY_F1, "F2" => EvKey::KEY_F2, "F3" => EvKey::KEY_F3,
            "F4" => EvKey::KEY_F4, "F5" => EvKey::KEY_F5, "F6" => EvKey::KEY_F6,
            "F7" => EvKey::KEY_F7, "F8" => EvKey::KEY_F8, "F9" => EvKey::KEY_F9,
            "F10" => EvKey::KEY_F10, "F11" => EvKey::KEY_F11, "F12" => EvKey::KEY_F12,
            _ => return None,
        })
    }

    fn map_mouse(button: MouseButton) -> EvKey {
        match button {
            MouseButton::Left => EvKey::BTN_LEFT,
            MouseButton::Right => EvKey::BTN_RIGHT,
            MouseButton::Middle => EvKey::BTN_MIDDLE,
        }
    }

    pub struct LinuxUinputOutput {
        device: VirtualDevice,
        /// Fractional movement carried between frames so slow, fine motion
        /// is not lost to integer truncation.
        pointer_carry: [f32; 2],
        scroll_carry: [f32; 2],
        held: HashSet<EvKey>,
    }

    impl LinuxUinputOutput {
        pub fn new() -> anyhow::Result<Self> {
            let mut keys = AttributeSet::<EvKey>::new();
            for code in 0..=248u16 {
                // Advertise the full keyboard range plus mouse buttons.
                keys.insert(EvKey::new(code));
            }
            keys.insert(EvKey::BTN_LEFT);
            keys.insert(EvKey::BTN_RIGHT);
            keys.insert(EvKey::BTN_MIDDLE);
            let mut rel = AttributeSet::<RelativeAxisType>::new();
            rel.insert(RelativeAxisType::REL_X);
            rel.insert(RelativeAxisType::REL_Y);
            rel.insert(RelativeAxisType::REL_WHEEL);
            rel.insert(RelativeAxisType::REL_HWHEEL);
            let device = VirtualDeviceBuilder::new()
                .context("open /dev/uinput; run `localpad doctor` for setup help")?
                .name("LocalPad Virtual Input")
                .with_keys(&keys)?
                .with_relative_axes(&rel)?
                .build()?;
            Ok(LinuxUinputOutput {
                device,
                pointer_carry: [0.0; 2],
                scroll_carry: [0.0; 2],
                held: HashSet::new(),
            })
        }

        fn emit_key(&mut self, key: EvKey, down: bool) -> anyhow::Result<()> {
            if down {
                self.held.insert(key);
            } else {
                self.held.remove(&key);
            }
            let ev = InputEvent::new(EventType::KEY, key.code(), i32::from(down));
            self.device.emit(&[ev])?;
            Ok(())
        }
    }

    impl InputOutput for LinuxUinputOutput {
        fn apply_frame(&mut self, frame: &OutputFrame) -> anyhow::Result<()> {
            let mut events = Vec::new();
            for (i, axis) in [RelativeAxisType::REL_X, RelativeAxisType::REL_Y]
                .into_iter()
                .enumerate()
            {
                self.pointer_carry[i] += frame.mouse_delta[i];
                let whole = self.pointer_carry[i].trunc();
                if whole != 0.0 {
                    self.pointer_carry[i] -= whole;
                    events.push(InputEvent::new(EventType::RELATIVE, axis.0, whole as i32));
                }
            }
            // Browsers report scroll in pixels; a wheel detent is ~40 px.
            self.scroll_carry[0] += frame.scroll_delta[0] / 40.0;
            self.scroll_carry[1] += -frame.scroll_delta[1] / 40.0;
            let hwhole = self.scroll_carry[0].trunc();
            if hwhole != 0.0 {
                self.scroll_carry[0] -= hwhole;
                events.push(InputEvent::new(
                    EventType::RELATIVE,
                    RelativeAxisType::REL_HWHEEL.0,
                    hwhole as i32,
                ));
            }
            let vwhole = self.scroll_carry[1].trunc();
            if vwhole != 0.0 {
                self.scroll_carry[1] -= vwhole;
                events.push(InputEvent::new(
                    EventType::RELATIVE,
                    RelativeAxisType::REL_WHEEL.0,
                    vwhole as i32,
                ));
            }
            if !events.is_empty() {
                self.device.emit(&events)?;
            }
            Ok(())
        }

        fn apply_event(&mut self, event: &OutputEvent) -> anyhow::Result<()> {
            match event {
                OutputEvent::Key { key, down } => {
                    let Key(code) = key;
                    match map_key(code) {
                        Some(k) => self.emit_key(k, *down)?,
                        None => tracing::debug!(code, "no Linux mapping for key"),
                    }
                }
                OutputEvent::MouseButton { button, down } => {
                    self.emit_key(map_mouse(*button), *down)?;
                }
            }
            Ok(())
        }

        fn release_all(&mut self) -> anyhow::Result<()> {
            let held: Vec<EvKey> = self.held.drain().collect();
            for key in held {
                let ev = InputEvent::new(EventType::KEY, key.code(), 0);
                self.device.emit(&[ev])?;
            }
            self.pointer_carry = [0.0; 2];
            self.scroll_carry = [0.0; 2];
            Ok(())
        }

        fn capabilities(&self) -> OutputCapabilities {
            OutputCapabilities {
                pointer: true,
                keyboard: true,
                gamepad: false,
                motion: false,
                name: "Linux uinput",
            }
        }
    }
}

#[cfg(target_os = "linux")]
pub use uinput_impl::LinuxUinputOutput;

/// True when /dev/uinput exists and is writable by this process.
#[cfg(target_os = "linux")]
pub fn uinput_available() -> bool {
    std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/uinput")
        .is_ok()
}

#[cfg(not(target_os = "linux"))]
pub fn uinput_available() -> bool {
    false
}
