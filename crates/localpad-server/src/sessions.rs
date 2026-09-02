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
    /// Stable identifier the phone generates once and sends on every
    /// pairing, so re-pairing replaces the device instead of duplicating it.
    pub uid: Option<String>,
    pub paired_at: Instant,
    pub last_ip: Option<IpAddr>,
}

/// Most paired devices kept per boot; oldest are dropped beyond this.
const MAX_DEVICES: usize = 8;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceSummary {
    pub device_id: String,
    pub name: String,
    pub connected: bool,
}

pub struct ActiveConnection {
    pub device_id: String,
    pub commands: mpsc::Sender<ConnCommand>,
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
    /// A device re-pairing with the same uid (or, lacking one, the same
    /// name) keeps its device identity: old tokens are revoked, not
    /// accumulated.
    pub fn issue(&mut self, name: &str, uid: Option<&str>, ip: IpAddr) -> (String, DeviceSession) {
        let existing_id = self
            .by_token_hash
            .values()
            .find(|s| match (uid, &s.uid) {
                (Some(u), Some(known)) => u == known,
                (None, None) => s.name == name,
                _ => false,
            })
            .map(|s| s.device_id.clone());
        let device_id = match existing_id {
            Some(id) => {
                self.by_token_hash.retain(|_, s| s.device_id != id);
                id
            }
            None => {
                self.next_device_number += 1;
                format!("device-{}", self.next_device_number)
            }
        };

        // Cap how many paired devices we remember this boot.
        while self.device_count() >= MAX_DEVICES {
            let oldest = self
                .by_token_hash
                .values()
                .min_by_key(|s| s.paired_at)
                .map(|s| s.device_id.clone());
            match oldest {
                Some(id) => self.by_token_hash.retain(|_, s| s.device_id != id),
                None => break,
            }
        }

        let token = crate::pairing::random_token();
        let session = DeviceSession {
            device_id,
            name: name.to_string(),
            uid: uid.map(str::to_string),
            paired_at: Instant::now(),
            last_ip: Some(ip),
        };
        self.by_token_hash
            .insert(hash_hex(token.as_bytes()), session.clone());
        (token, session)
    }

    fn device_count(&self) -> usize {
        self.by_token_hash
            .values()
            .map(|s| s.device_id.as_str())
            .collect::<std::collections::HashSet<_>>()
            .len()
    }

    pub fn authenticate(&mut self, token: &str) -> Option<DeviceSession> {
        self.by_token_hash.get(&hash_hex(token.as_bytes())).cloned()
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
        let (token, session) = map.issue("Ana's phone", None, ip);
        let got = map.authenticate(&token).expect("token should authenticate");
        assert_eq!(got.device_id, session.device_id);
        assert!(map.authenticate("wrong-token").is_none());
        map.revoke(&session.device_id);
        assert!(map.authenticate(&token).is_none());
    }

    #[test]
    fn repairing_replaces_instead_of_duplicating() {
        let mut map = SessionMap::default();
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let (old_token, first) = map.issue("Ana's phone", Some("uid-1"), ip);
        let (new_token, second) = map.issue("Ana's phone", Some("uid-1"), ip);
        assert_eq!(first.device_id, second.device_id, "same device keeps its id");
        assert!(map.authenticate(&old_token).is_none(), "old token revoked");
        assert!(map.authenticate(&new_token).is_some());
        assert_eq!(map.summaries().len(), 1);

        // Without uids, the name is the fallback identity.
        let (_, a) = map.issue("Guest", None, ip);
        let (_, b) = map.issue("Guest", None, ip);
        assert_eq!(a.device_id, b.device_id);
        assert_eq!(map.summaries().len(), 2);
    }

    #[test]
    fn device_cap_drops_the_oldest() {
        let mut map = SessionMap::default();
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        for i in 0..(MAX_DEVICES + 3) {
            let _ = map.issue(&format!("Phone {i}"), Some(&format!("uid-{i}")), ip);
        }
        let summaries = map.summaries();
        assert_eq!(summaries.len(), MAX_DEVICES);
        assert!(!summaries.iter().any(|d| d.name == "Phone 0"), "oldest gone");
        assert!(summaries.iter().any(|d| d.name == format!("Phone {}", MAX_DEVICES + 2)));
    }
}
