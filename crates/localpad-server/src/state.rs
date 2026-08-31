//! Shared server state: configuration, network identity, TLS, layouts,
//! pairing, sessions, outputs and the admin event stream.

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

use anyhow::Context;
use localpad_core::layout::{Layout, OutputMode};
use localpad_core::mapping::OutputFrame;
use serde::Serialize;
use tokio::sync::{broadcast, oneshot, watch};

use crate::config::{Preferences, ServerConfig};
use crate::netinfo::{self, NetworkInfo};
use crate::outputs::OutputStack;
use crate::pairing::{PairingDisplay, PairingState};
use crate::sessions::{ConnCommand, SessionMap};
use crate::tls::TlsIdentity;

pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Built-in layouts compiled into the binary from /layouts.
const BUILTIN_LAYOUTS: &[&str] = &[
    include_str!("../../../layouts/touchpad.json"),
    include_str!("../../../layouts/keyboard-trackpad.json"),
    include_str!("../../../layouts/media-remote.json"),
    include_str!("../../../layouts/presentation.json"),
    include_str!("../../../layouts/gba.json"),
    include_str!("../../../layouts/snes.json"),
    include_str!("../../../layouts/xbox.json"),
    include_str!("../../../layouts/dual-stick.json"),
    include_str!("../../../layouts/dolphin.json"),
    include_str!("../../../layouts/steering-wheel.json"),
];

/// Events pushed to admin dashboard WebSocket subscribers.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum AdminEvent {
    /// Something structural changed; the dashboard refetches /api/status.
    Status,
    Monitor(MonitorSnapshot),
    ApprovalRequested { device_id: String, name: String },
    Toast { level: String, message: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorSnapshot {
    pub frame: OutputFrame,
    pub raw_buttons: u32,
    pub frames_per_second: f32,
    pub latency_ms: Option<f32>,
    pub held_inputs: usize,
    pub sequence: u64,
    pub dropped_frames: u64,
}

pub struct PendingApproval {
    pub device_id_hint: u32,
    pub name: String,
    pub ip: IpAddr,
    pub respond: oneshot::Sender<bool>,
}

pub struct AppState {
    pub config: ServerConfig,
    pub data_dir: PathBuf,
    pub network: NetworkInfo,
    pub tls: Option<TlsIdentity>,
    pub layouts: HashMap<String, Layout>,
    pub active_profile: Mutex<String>,
    pub pairing: Mutex<PairingState>,
    pub sessions: Mutex<SessionMap>,
    pub outputs: Mutex<OutputStack>,
    pub approvals: Mutex<HashMap<u32, PendingApproval>>,
    pub prefs: Mutex<Preferences>,
    pub started_at: Instant,
    admin_events: broadcast::Sender<AdminEvent>,
    shutdown_tx: watch::Sender<bool>,
    approval_counter: Mutex<u32>,
}

fn load_layouts(extra_dir: Option<&std::path::Path>) -> anyhow::Result<HashMap<String, Layout>> {
    let mut layouts = HashMap::new();
    for source in BUILTIN_LAYOUTS {
        let layout = Layout::parse(source.as_bytes()).context("built-in layout is invalid")?;
        layouts.insert(layout.id.clone(), layout);
    }
    if let Some(dir) = extra_dir {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "json") {
                    match std::fs::read(&path).map_err(anyhow::Error::from).and_then(|b| {
                        Layout::parse(&b).map_err(anyhow::Error::from)
                    }) {
                        Ok(layout) => {
                            layouts.insert(layout.id.clone(), layout);
                        }
                        Err(e) => {
                            tracing::warn!(path = %path.display(), error = %e, "skipping invalid layout");
                        }
                    }
                }
            }
        }
    }
    Ok(layouts)
}

impl AppState {
    pub async fn new(config: ServerConfig) -> anyhow::Result<Self> {
        let data_dir = config.resolve_data_dir()?;
        let network = netinfo::discover()?;

        if !netinfo::is_private(&config.bind_addr)
            && !config.bind_addr.is_unspecified()
            && !config.allow_remote
        {
            anyhow::bail!(
                "{} is not a private address. LocalPad is designed for local \
                 networks; pass --allow-remote if you accept the risk.",
                config.bind_addr
            );
        }
        if !netinfo::is_private(&network.lan_ip) {
            tracing::warn!(
                ip = %network.lan_ip,
                "this machine's address is not in a private range; \
                 anyone who can reach it can attempt pairing"
            );
        }

        let tls = if config.insecure_http {
            tracing::warn!(
                "running with --insecure-http: motion controls are disabled \
                 on phones because browsers require a secure context"
            );
            None
        } else {
            Some(crate::tls::ensure_identity(
                &data_dir.join("certs"),
                &network.all_ips,
            )?)
        };

        let layouts = load_layouts(config.layouts_dir.as_deref())?;
        let profile = if layouts.contains_key(&config.profile) {
            config.profile.clone()
        } else {
            anyhow::bail!(
                "unknown profile {:?}; run `localpad profiles` to list them",
                config.profile
            );
        };
        let initial_mode = layouts[&profile].output;
        let outputs = OutputStack::new(config.no_native_output, initial_mode);
        if let Some(warning) = &outputs.platform_warning {
            tracing::warn!("{warning}");
        }

        let (admin_events, _) = broadcast::channel(256);
        let (shutdown_tx, _) = watch::channel(false);
        let prefs = Preferences::load(&data_dir);

        Ok(AppState {
            config,
            data_dir,
            network,
            tls,
            layouts,
            active_profile: Mutex::new(profile),
            pairing: Mutex::new(PairingState::default()),
            sessions: Mutex::new(SessionMap::default()),
            outputs: Mutex::new(outputs),
            approvals: Mutex::new(HashMap::new()),
            prefs: Mutex::new(prefs),
            started_at: Instant::now(),
            admin_events,
            shutdown_tx,
            approval_counter: Mutex::new(0),
        })
    }

    pub fn broadcast(&self, event: AdminEvent) {
        let _ = self.admin_events.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AdminEvent> {
        self.admin_events.subscribe()
    }

    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    pub fn shutdown_rx(&self) -> watch::Receiver<bool> {
        self.shutdown_tx.subscribe()
    }

    pub fn active_layout(&self) -> Layout {
        let profile = self.active_profile.lock().unwrap().clone();
        self.layouts[&profile].clone()
    }

    /// Change the active profile. Returns the new layout, or None for an
    /// unknown id. Pushes the layout to the connected phone.
    pub async fn set_profile(&self, id: &str) -> Option<Layout> {
        let layout = self.layouts.get(id)?.clone();
        *self.active_profile.lock().unwrap() = id.to_string();
        {
            let mut prefs = self.prefs.lock().unwrap();
            prefs.last_profile = Some(id.to_string());
            prefs.save(&self.data_dir);
        }
        self.outputs.lock().unwrap().set_mode(layout.output);
        let commands = self
            .sessions
            .lock()
            .unwrap()
            .active
            .as_ref()
            .map(|a| a.commands.clone());
        if let Some(commands) = commands {
            let _ = commands.send(ConnCommand::SetLayout(id.to_string())).await;
        }
        self.broadcast(AdminEvent::Status);
        Some(layout)
    }

    /// Begin a fresh pairing session and return what to display.
    pub async fn new_pairing(&self, controller_url: &str) -> PairingDisplay {
        let (_, display) = self.pairing.lock().unwrap().begin(controller_url);
        self.broadcast(AdminEvent::Status);
        display
    }

    pub fn pairing_display(&self) -> Option<PairingDisplay> {
        self.pairing.lock().unwrap().display()
    }

    pub fn next_approval_id(&self) -> u32 {
        let mut counter = self.approval_counter.lock().unwrap();
        *counter += 1;
        *counter
    }

    /// Emergency release: neutralize the active connection and the outputs.
    pub async fn release_active(&self) {
        let commands = self
            .sessions
            .lock()
            .unwrap()
            .active
            .as_ref()
            .map(|a| a.commands.clone());
        if let Some(commands) = commands {
            let _ = commands.send(ConnCommand::ReleaseAll).await;
        }
        if let Ok(mut outputs) = self.outputs.lock() {
            let _ = outputs.release_all();
        }
    }

    pub fn output_mode(&self) -> OutputMode {
        self.outputs.lock().unwrap().mode()
    }
}
