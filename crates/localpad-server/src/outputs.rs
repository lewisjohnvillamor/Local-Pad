//! The output stack: whichever native adapter fits this platform, plus the
//! DSU server when a motion layout is active, plus the monitor feed for
//! the admin dashboard.

use localpad_core::layout::OutputMode;
use localpad_core::mapping::{OutputEvent, OutputFrame};
use localpad_core::output::{InputOutput, NullOutput, OutputCapabilities};
use localpad_output_dsu::DsuOutput;

pub struct OutputStack {
    platform: Box<dyn InputOutput>,
    dsu: Option<DsuOutput>,
    mode: OutputMode,
    /// Human-readable reason the platform adapter is a no-op, if it is.
    pub platform_warning: Option<String>,
}

fn create_platform_output(disabled: bool) -> (Box<dyn InputOutput>, Option<String>) {
    if disabled {
        return (
            Box::new(NullOutput::default()),
            Some("native output disabled with --no-native-output".to_string()),
        );
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        match localpad_output_enigo::EnigoOutput::new() {
            Ok(out) => return (Box::new(out), None),
            Err(e) => {
                return (
                    Box::new(NullOutput::default()),
                    Some(format!("native output unavailable: {e:#}")),
                )
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        match localpad_output_linux::LinuxUinputOutput::new() {
            Ok(out) => return (Box::new(out), None),
            Err(e) => {
                return (
                    Box::new(NullOutput::default()),
                    Some(format!(
                        "Linux uinput unavailable: {e:#}. Run `localpad doctor` for setup help."
                    )),
                )
            }
        }
    }
    #[allow(unreachable_code)]
    (
        Box::new(NullOutput::default()),
        Some("no native output adapter for this platform".to_string()),
    )
}

impl OutputStack {
    pub fn new(disabled: bool, initial_mode: OutputMode) -> Self {
        let (platform, platform_warning) = create_platform_output(disabled);
        let mut stack = OutputStack {
            platform,
            dsu: None,
            mode: initial_mode,
            platform_warning,
        };
        stack.set_mode(initial_mode);
        stack
    }

    /// Switch output mode; releases everything first so nothing stays held
    /// across the change.
    pub fn set_mode(&mut self, mode: OutputMode) {
        let _ = self.release_all();
        self.mode = mode;
        let wants_dsu = matches!(mode, OutputMode::Dsu);
        if wants_dsu && self.dsu.is_none() {
            match DsuOutput::bind(localpad_output_dsu::DEFAULT_DSU_PORT) {
                Ok(server) => self.dsu = Some(server),
                Err(e) => {
                    tracing::warn!(error = %e, "could not start the DSU server");
                }
            }
        }
        if !wants_dsu {
            self.dsu = None;
        }
    }

    pub fn mode(&self) -> OutputMode {
        self.mode
    }

    pub fn dsu_active(&self) -> bool {
        self.dsu.is_some()
    }

    pub fn dsu_clients(&self) -> usize {
        self.dsu.as_ref().map_or(0, |d| d.client_count())
    }

    pub fn capabilities(&self) -> OutputCapabilities {
        self.platform.capabilities()
    }

    pub fn apply_frame(&mut self, frame: &OutputFrame) {
        if let Err(e) = self.platform.apply_frame(frame) {
            tracing::warn!(error = %e, "platform output frame failed");
        }
        if let Some(dsu) = self.dsu.as_mut() {
            let _ = dsu.apply_frame(frame);
        }
    }

    pub fn apply_events(&mut self, events: &[OutputEvent]) {
        for event in events {
            if let Err(e) = self.platform.apply_event(event) {
                tracing::warn!(error = %e, "platform output event failed");
            }
        }
    }

    pub fn release_all(&mut self) -> anyhow::Result<()> {
        let platform = self.platform.release_all();
        if let Some(dsu) = self.dsu.as_mut() {
            let _ = dsu.release_all();
        }
        platform
    }
}
