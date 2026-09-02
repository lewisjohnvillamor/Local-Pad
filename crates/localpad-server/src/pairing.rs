//! Pairing: short-lived, single-use secrets exchanged for session tokens.
//! Only hashes of secrets and tokens are kept in memory; the raw secret
//! exists in the QR URL fragment and the raw token only on the phone.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use data_encoding::BASE64URL_NOPAD;
use rand::RngCore;
use sha2::{Digest, Sha256};

pub const PAIRING_TTL: Duration = Duration::from_secs(5 * 60);
const MAX_CODE_ATTEMPTS: u32 = 5;
const RATE_WINDOW: Duration = Duration::from_secs(60);
const MAX_ATTEMPTS_PER_WINDOW: u32 = 10;

pub fn hash_hex(input: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input);
    data_encoding::HEXLOWER.encode(&hasher.finalize())
}

pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    BASE64URL_NOPAD.encode(&bytes)
}

fn random_code() -> String {
    // Six digits shown as 592-184. Uniform over 000000..999999.
    let n = rand::rng().next_u32() % 1_000_000;
    format!("{:03}-{:03}", n / 1000, n % 1000)
}

/// What the CLI and dashboard show for an open pairing session.
#[derive(Debug, Clone)]
pub struct PairingDisplay {
    pub code: String,
    pub url: String,
    pub expires_in: Duration,
}

struct PendingPairing {
    secret_hash: String,
    code: String,
    created: Instant,
    code_attempts: u32,
    /// URL including the secret fragment, needed to re-render the QR code.
    url: String,
}

#[derive(Default)]
pub struct PairingState {
    pending: Option<PendingPairing>,
    attempts: HashMap<IpAddr, (Instant, u32)>,
}

pub enum RedeemOutcome {
    /// Pairing accepted; the caller creates the session.
    Accepted,
    Rejected,
    RateLimited,
}

impl PairingState {
    /// Replace any open pairing session with a fresh one. Returns the raw
    /// secret (for the QR URL) exactly once; only its hash is stored.
    pub fn begin(&mut self, base_url: &str) -> (String, PairingDisplay) {
        let secret = random_token();
        let code = random_code();
        let url = format!("{base_url}#pair={secret}");
        self.pending = Some(PendingPairing {
            secret_hash: hash_hex(secret.as_bytes()),
            code: code.clone(),
            created: Instant::now(),
            code_attempts: 0,
            url: url.clone(),
        });
        (
            secret,
            PairingDisplay {
                code,
                url,
                expires_in: PAIRING_TTL,
            },
        )
    }

    pub fn display(&self) -> Option<PairingDisplay> {
        let p = self.pending.as_ref()?;
        let elapsed = p.created.elapsed();
        if elapsed >= PAIRING_TTL {
            return None;
        }
        Some(PairingDisplay {
            code: p.code.clone(),
            url: p.url.clone(),
            expires_in: PAIRING_TTL - elapsed,
        })
    }

    fn rate_limited(&mut self, ip: IpAddr) -> bool {
        let now = Instant::now();
        // Expired windows are dropped wholesale so the map cannot grow
        // without bound on a long-running server.
        self.attempts
            .retain(|_, (start, _)| now.duration_since(*start) <= RATE_WINDOW);
        let entry = self.attempts.entry(ip).or_insert((now, 0));
        entry.1 += 1;
        entry.1 > MAX_ATTEMPTS_PER_WINDOW
    }

    /// Try to redeem a pairing secret or numeric code. A success consumes
    /// the pairing session; nothing can redeem it twice.
    pub fn redeem(&mut self, ip: IpAddr, secret_or_code: &str) -> RedeemOutcome {
        if self.rate_limited(ip) {
            return RedeemOutcome::RateLimited;
        }
        let Some(p) = self.pending.as_mut() else {
            return RedeemOutcome::Rejected;
        };
        if p.created.elapsed() >= PAIRING_TTL {
            self.pending = None;
            return RedeemOutcome::Rejected;
        }
        let normalized = secret_or_code.trim();
        let by_secret = hash_hex(normalized.as_bytes()) == p.secret_hash;
        let by_code = normalized.replace(['-', ' '], "") == p.code.replace('-', "");
        if by_secret || by_code {
            self.pending = None;
            return RedeemOutcome::Accepted;
        }
        if !by_secret && normalized.len() < 16 {
            p.code_attempts += 1;
            if p.code_attempts >= MAX_CODE_ATTEMPTS {
                // Too many wrong numeric guesses burns the session.
                self.pending = None;
            }
        }
        RedeemOutcome::Rejected
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(192, 168, 1, last))
    }

    #[test]
    fn secret_redeems_once() {
        let mut state = PairingState::default();
        let (secret, display) = state.begin("https://192.168.1.2:7844/controller");
        assert!(display.url.contains("#pair="));
        assert!(matches!(state.redeem(ip(9), &secret), RedeemOutcome::Accepted));
        assert!(matches!(state.redeem(ip(9), &secret), RedeemOutcome::Rejected));
    }

    #[test]
    fn code_redeems_with_and_without_dash() {
        let mut state = PairingState::default();
        let (_, display) = state.begin("http://x");
        let plain = display.code.replace('-', "");
        assert!(matches!(state.redeem(ip(9), &plain), RedeemOutcome::Accepted));

        let (_, display) = state.begin("http://x");
        assert!(matches!(
            state.redeem(ip(9), &display.code),
            RedeemOutcome::Accepted
        ));
    }

    #[test]
    fn wrong_code_guesses_burn_the_session() {
        let mut state = PairingState::default();
        let (_, display) = state.begin("http://x");
        for _ in 0..MAX_CODE_ATTEMPTS {
            assert!(matches!(
                state.redeem(ip(9), "000-001"),
                RedeemOutcome::Rejected | RedeemOutcome::RateLimited
            ));
        }
        // Even the right code no longer works.
        assert!(matches!(
            state.redeem(ip(9), &display.code),
            RedeemOutcome::Rejected
        ));
    }

    #[test]
    fn per_ip_rate_limit_kicks_in() {
        let mut state = PairingState::default();
        let _ = state.begin("http://x");
        let mut limited = false;
        for _ in 0..30 {
            if matches!(state.redeem(ip(7), "junk"), RedeemOutcome::RateLimited) {
                limited = true;
                break;
            }
        }
        assert!(limited);
    }

    #[test]
    fn tokens_are_high_entropy_and_distinct() {
        let a = random_token();
        let b = random_token();
        assert_ne!(a, b);
        assert!(a.len() >= 43, "256 bits base64url is at least 43 chars");
    }
}
