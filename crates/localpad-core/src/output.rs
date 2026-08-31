//! The output abstraction every native adapter implements, plus a monitor
//! output used by the admin dashboard and tests.

use serde::{Deserialize, Serialize};

use crate::mapping::{OutputEvent, OutputFrame};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputCapabilities {
    pub pointer: bool,
    pub keyboard: bool,
    pub gamepad: bool,
    pub motion: bool,
    /// Human-readable adapter name for the dashboard.
    pub name: &'static str,
}

/// One native output backend. Implementations must be safe to drop at any
/// time and must make `release_all` idempotent: it is called on disconnect,
/// timeout, mode change, panic recovery and shutdown.
pub trait InputOutput: Send {
    /// Apply the continuous state for one tick.
    fn apply_frame(&mut self, frame: &OutputFrame) -> anyhow::Result<()>;
    /// Apply one discrete transition (key or mouse button).
    fn apply_event(&mut self, event: &OutputEvent) -> anyhow::Result<()>;
    /// Release every held key, button and axis.
    fn release_all(&mut self) -> anyhow::Result<()>;
    fn capabilities(&self) -> OutputCapabilities;
}

/// An output that records what it receives. Used by tests and as the safe
/// fallback when no native backend is available.
#[derive(Default)]
pub struct NullOutput {
    pub frames: u64,
    pub events: Vec<OutputEvent>,
    pub released: u32,
}

impl InputOutput for NullOutput {
    fn apply_frame(&mut self, _frame: &OutputFrame) -> anyhow::Result<()> {
        self.frames += 1;
        Ok(())
    }

    fn apply_event(&mut self, event: &OutputEvent) -> anyhow::Result<()> {
        self.events.push(event.clone());
        Ok(())
    }

    fn release_all(&mut self) -> anyhow::Result<()> {
        self.released += 1;
        Ok(())
    }

    fn capabilities(&self) -> OutputCapabilities {
        OutputCapabilities {
            pointer: false,
            keyboard: false,
            gamepad: false,
            motion: false,
            name: "monitor only",
        }
    }
}
