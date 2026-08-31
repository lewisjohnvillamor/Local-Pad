//! macOS mouse and keyboard output through CoreGraphics events (via the
//! enigo bindings). Requires the Accessibility permission; during
//! development macOS attributes that permission to the terminal that
//! launched the process. `localpad doctor` explains the setup.

#[cfg(target_os = "macos")]
mod mac_impl {
    use anyhow::Context;
    use enigo::{
        Axis, Button, Coordinate, Direction, Enigo, Key as EKey, Keyboard, Mouse, Settings,
    };
    use localpad_core::frame::{Key, MouseButton};
    use localpad_core::mapping::{OutputEvent, OutputFrame};
    use localpad_core::output::{InputOutput, OutputCapabilities};
    use std::collections::HashSet;

    /// Map a W3C KeyboardEvent.code to an enigo key. Letters and digits go
    /// through Unicode so the user's keyboard layout applies.
    fn map_key(code: &str) -> Option<EKey> {
        if let Some(rest) = code.strip_prefix("Key") {
            let c = rest.chars().next()?.to_ascii_lowercase();
            return Some(EKey::Unicode(c));
        }
        if let Some(rest) = code.strip_prefix("Digit") {
            return Some(EKey::Unicode(rest.chars().next()?));
        }
        Some(match code {
            "Enter" | "NumpadEnter" => EKey::Return,
            "Escape" => EKey::Escape,
            "Backspace" => EKey::Backspace,
            "Tab" => EKey::Tab,
            "Space" => EKey::Space,
            "Minus" => EKey::Unicode('-'),
            "Equal" => EKey::Unicode('='),
            "BracketLeft" => EKey::Unicode('['),
            "BracketRight" => EKey::Unicode(']'),
            "Backslash" => EKey::Unicode('\\'),
            "Semicolon" => EKey::Unicode(';'),
            "Quote" => EKey::Unicode('\''),
            "Backquote" => EKey::Unicode('`'),
            "Comma" => EKey::Unicode(','),
            "Period" => EKey::Unicode('.'),
            "Slash" => EKey::Unicode('/'),
            "CapsLock" => EKey::CapsLock,
            "ArrowUp" => EKey::UpArrow,
            "ArrowDown" => EKey::DownArrow,
            "ArrowLeft" => EKey::LeftArrow,
            "ArrowRight" => EKey::RightArrow,
            "Home" => EKey::Home,
            "End" => EKey::End,
            "PageUp" => EKey::PageUp,
            "PageDown" => EKey::PageDown,
            "Delete" => EKey::Delete,
            "ShiftLeft" | "ShiftRight" => EKey::Shift,
            "ControlLeft" | "ControlRight" => EKey::Control,
            "AltLeft" | "AltRight" => EKey::Alt,
            "MetaLeft" | "MetaRight" => EKey::Meta,
            "AudioVolumeUp" => EKey::VolumeUp,
            "AudioVolumeDown" => EKey::VolumeDown,
            "AudioVolumeMute" => EKey::VolumeMute,
            "MediaPlayPause" => EKey::MediaPlayPause,
            "MediaTrackNext" => EKey::MediaNextTrack,
            "MediaTrackPrevious" => EKey::MediaPrevTrack,
            "F1" => EKey::F1, "F2" => EKey::F2, "F3" => EKey::F3, "F4" => EKey::F4,
            "F5" => EKey::F5, "F6" => EKey::F6, "F7" => EKey::F7, "F8" => EKey::F8,
            "F9" => EKey::F9, "F10" => EKey::F10, "F11" => EKey::F11, "F12" => EKey::F12,
            _ => return None,
        })
    }

    fn map_mouse(button: MouseButton) -> Button {
        match button {
            MouseButton::Left => Button::Left,
            MouseButton::Right => Button::Right,
            MouseButton::Middle => Button::Middle,
        }
    }

    pub struct MacCoreGraphicsOutput {
        enigo: Enigo,
        pointer_carry: [f32; 2],
        scroll_carry: [f32; 2],
        held_keys: HashSet<String>,
        held_buttons: HashSet<MouseButton>,
    }

    impl MacCoreGraphicsOutput {
        pub fn new() -> anyhow::Result<Self> {
            let enigo = Enigo::new(&Settings::default()).context(
                "could not create the CoreGraphics event source; \
                 grant LocalPad the Accessibility permission in \
                 System Settings, Privacy & Security, Accessibility",
            )?;
            Ok(MacCoreGraphicsOutput {
                enigo,
                pointer_carry: [0.0; 2],
                scroll_carry: [0.0; 2],
                held_keys: HashSet::new(),
                held_buttons: HashSet::new(),
            })
        }
    }

    impl InputOutput for MacCoreGraphicsOutput {
        fn apply_frame(&mut self, frame: &OutputFrame) -> anyhow::Result<()> {
            self.pointer_carry[0] += frame.mouse_delta[0];
            self.pointer_carry[1] += frame.mouse_delta[1];
            let dx = self.pointer_carry[0].trunc();
            let dy = self.pointer_carry[1].trunc();
            if dx != 0.0 || dy != 0.0 {
                self.pointer_carry[0] -= dx;
                self.pointer_carry[1] -= dy;
                self.enigo
                    .move_mouse(dx as i32, dy as i32, Coordinate::Rel)?;
            }
            // Browsers report scroll in pixels; a wheel line is ~40 px.
            self.scroll_carry[0] += frame.scroll_delta[0] / 40.0;
            self.scroll_carry[1] += frame.scroll_delta[1] / 40.0;
            let sx = self.scroll_carry[0].trunc();
            let sy = self.scroll_carry[1].trunc();
            if sx != 0.0 {
                self.scroll_carry[0] -= sx;
                self.enigo.scroll(sx as i32, Axis::Horizontal)?;
            }
            if sy != 0.0 {
                self.scroll_carry[1] -= sy;
                self.enigo.scroll(sy as i32, Axis::Vertical)?;
            }
            Ok(())
        }

        fn apply_event(&mut self, event: &OutputEvent) -> anyhow::Result<()> {
            match event {
                OutputEvent::Key { key, down } => {
                    let Key(code) = key;
                    match map_key(code) {
                        Some(k) => {
                            let dir = if *down { Direction::Press } else { Direction::Release };
                            self.enigo.key(k, dir)?;
                            if *down {
                                self.held_keys.insert(code.clone());
                            } else {
                                self.held_keys.remove(code);
                            }
                        }
                        None => tracing::debug!(code, "no macOS mapping for key"),
                    }
                }
                OutputEvent::MouseButton { button, down } => {
                    let dir = if *down { Direction::Press } else { Direction::Release };
                    self.enigo.button(map_mouse(*button), dir)?;
                    if *down {
                        self.held_buttons.insert(*button);
                    } else {
                        self.held_buttons.remove(button);
                    }
                }
            }
            Ok(())
        }

        fn release_all(&mut self) -> anyhow::Result<()> {
            let keys: Vec<String> = self.held_keys.drain().collect();
            for code in keys {
                if let Some(k) = map_key(&code) {
                    let _ = self.enigo.key(k, Direction::Release);
                }
            }
            let buttons: Vec<MouseButton> = self.held_buttons.drain().collect();
            for b in buttons {
                let _ = self.enigo.button(map_mouse(b), Direction::Release);
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
                name: "macOS CoreGraphics",
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub use mac_impl::MacCoreGraphicsOutput;
