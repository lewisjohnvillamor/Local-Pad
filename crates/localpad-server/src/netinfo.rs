//! Local network discovery: which addresses we sit on and whether they are
//! private. Binding beyond private ranges requires --allow-remote.

use std::net::IpAddr;

#[derive(Debug, Clone)]
pub struct NetworkInfo {
    pub lan_ip: IpAddr,
    pub all_ips: Vec<IpAddr>,
    pub hostname: String,
}

pub fn is_private(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_private() || v4.is_loopback() || v4.is_link_local()
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || (v6.segments()[0] & 0xfe00) == 0xfc00 // unique local fc00::/7
                || (v6.segments()[0] & 0xffc0) == 0xfe80 // link local
        }
    }
}

pub fn discover() -> anyhow::Result<NetworkInfo> {
    let lan_ip = local_ip_address::local_ip()
        .map_err(|e| anyhow::anyhow!("could not determine the LAN address: {e}"))?;
    let mut all_ips = vec![lan_ip];
    if let Ok(interfaces) = local_ip_address::list_afinet_netifas() {
        for (_, ip) in interfaces {
            if !ip.is_loopback() && is_private(&ip) && !all_ips.contains(&ip) {
                all_ips.push(ip);
            }
        }
    }
    let hostname = std::env::var("HOSTNAME").unwrap_or_else(|_| "this computer".to_string());
    Ok(NetworkInfo {
        lan_ip,
        all_ips,
        hostname,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_ranges() {
        assert!(is_private(&"192.168.1.5".parse().unwrap()));
        assert!(is_private(&"10.0.0.9".parse().unwrap()));
        assert!(is_private(&"172.16.4.4".parse().unwrap()));
        assert!(!is_private(&"8.8.8.8".parse().unwrap()));
        assert!(!is_private(&"172.32.0.1".parse().unwrap()));
        assert!(is_private(&"fe80::1".parse().unwrap()));
        assert!(!is_private(&"2001:4860:4860::8888".parse().unwrap()));
    }
}
