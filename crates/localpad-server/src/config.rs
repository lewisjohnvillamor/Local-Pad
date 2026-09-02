//! Server configuration: CLI options plus persisted preferences.

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const DEFAULT_ADMIN_PORT: u16 = 7843;
pub const DEFAULT_CONTROLLER_PORT: u16 = 7844;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub admin_port: u16,
    pub controller_port: u16,
    pub bind_addr: IpAddr,
    pub insecure_http: bool,
    pub require_approval: bool,
    pub allow_remote: bool,
    pub profile: String,
    /// Directory for certificates and preferences. Defaults to the platform
    /// config dir; overridable for tests.
    pub data_dir: Option<PathBuf>,
    /// Extra directory of layout JSON files to load at boot.
    pub layouts_dir: Option<PathBuf>,
    /// Disable native output; input is only shown in the dashboard monitor.
    pub no_native_output: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            admin_port: DEFAULT_ADMIN_PORT,
            controller_port: DEFAULT_CONTROLLER_PORT,
            bind_addr: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            insecure_http: false,
            require_approval: false,
            allow_remote: false,
            profile: "touchpad".to_string(),
            data_dir: None,
            layouts_dir: None,
            no_native_output: false,
        }
    }
}

impl ServerConfig {
    pub fn resolve_data_dir(&self) -> anyhow::Result<PathBuf> {
        if let Some(dir) = &self.data_dir {
            std::fs::create_dir_all(dir)?;
            return Ok(dir.clone());
        }
        let dirs = directories::ProjectDirs::from("dev", "localpad", "localpad")
            .ok_or_else(|| anyhow::anyhow!("no home directory available"))?;
        let dir = dirs.config_dir().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }
}

/// Preferences persisted between runs (never secrets).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preferences {
    #[serde(default)]
    pub last_profile: Option<String>,
}

impl Preferences {
    pub fn load(dir: &std::path::Path) -> Preferences {
        std::fs::read(dir.join("preferences.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, dir: &std::path::Path) {
        if let Ok(bytes) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(dir.join("preferences.json"), bytes);
        }
    }
}
