//! localpad-core: platform-independent input model, layouts, mapping and
//! output abstractions. This crate must stay free of platform-specific
//! dependencies; native adapters live in the localpad-output-* crates.

pub mod frame;
pub mod layout;
pub mod mapping;
pub mod motion;
pub mod output;

pub use frame::{FrameError, GamepadButton, InputFrame, Key, MouseButton, PROTOCOL_VERSION};
pub use layout::{Control, ControlKind, Layout, LayoutError};
pub use mapping::{MappingEngine, OutputEvent, OutputFrame};
pub use motion::{MotionProcessor, MotionSample, MotionSettings};
pub use output::{InputOutput, OutputCapabilities};
