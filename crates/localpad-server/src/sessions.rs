//! Device sessions and the single active controller connection.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Instant;

use serde::Serialize;
use tokio::sync::mpsc;

use crate::pairing::hash_hex;

/// Commands the server can send into the active connection task.
#[derive(Debug, Clone)]
pub enum ConnCommand {
    SetLayout(String),
    Recenter,
    ReleaseAll,
    Disconnect { reason: String },
}

/// A paired device (token issued, may or may not be connected right now).
#[derive(Debug, Clone)]
pub struct DeviceSession {
    pub device_id: String,
    pub name: String,
    pub paired_at: Instant,
    pub last_ip: Option<IpAddr>,
    pub approved: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceSummary {
    pub device_id: String,
    pub name: String,
    pub connected: bool,
    pub approved: bool,
}

pub struct ActiveConnection {
    pub device_id: String,
    pub commands: mpsc::Sender<ConnCommand>,
    pub connected_at: Instant,
}

#[derive(Default)]
pub struct SessionMap {
    /// sha256(token) -> device session.
    by_token_hash: HashMap<String, DeviceSession>,
    pub active: Option<ActiveConnection>,
    next_device_number: u32,
}

impl SessionMap {
    /// Issue a session for a newly paired device; returns the raw token.
    pub fn issue(&mut self, name: &str, ip: IpAddr, approved: bool) -> (String, DeviceSession) {
        let token = crate::pairing::random_token();
        self.next_device_number += 1;
        let session = DeviceSession {
            device_id: format!("device-{}", self.next_device_number),
            name: name.to_string(),
            paired_at: Instant::now(),
            last_ip: Some(ip),
            approved,
        };
        self.by_token_hash
            .insert(hash_hex(token.as_bytes()), session.clone());
        (token, session)
    }

    pub fn authenticate(&mut self, token: &str) -> Option<DeviceSession> {
        self.by_token_hash.get(&hash_hex(token.as_bytes())).cloned()
    }

    pub fn approve(&mut self, device_id: &str) -> bool {
        let mut found = false;
        for session in self.by_token_hash.values_mut() {
            if session.device_id == device_id {
                session.approved = true;
                found = true;
            }
        }
        found
    }

    pub fn revoke(&mut self, device_id: &str) {
        self.by_token_hash.retain(|_, s| s.device_id != device_id);
    }

    pub fn summaries(&self) -> Vec<DeviceSummary> {
        let connected_id = self.active.as_ref().map(|a| a.device_id.clone());
        let mut seen = std::collections::HashSet::new();
        let mut out: Vec<DeviceSummary> = self
            .by_token_hash
            .values()
            .filter(|s| seen.insert(s.device_id.clone()))
            .map(|s| DeviceSummary {
                device_id: s.device_id.clone(),
                name: s.name.clone(),
                connected: connected_id.as_deref() == Some(&s.device_id),
                approved: s.approved,
            })
            .collect();
        out.sort_by(|a, b| a.device_id.cmp(&b.device_id));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn token_authenticates_and_revokes() {
        let mut map = SessionMap::default();
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let (token, session) = map.issue("Ana's phone", ip, true);
        let got = map.authenticate(&token).expect("token should authenticate");
        assert_eq!(got.device_id, session.device_id);
        assert!(map.authenticate("wrong-token").is_none());
        map.revoke(&session.device_id);
        assert!(map.authenticate(&token).is_none());
    }

    #[test]
    fn approval_flow() {
        let mut map = SessionMap::default();
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let (token, session) = map.issue("Pending phone", ip, false);
        assert!(!map.authenticate(&token).unwrap().approved);
        assert!(map.approve(&session.device_id));
        assert!(map.authenticate(&token).unwrap().approved);
        assert!(!map.approve("device-999"));
    }
}
