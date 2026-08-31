//! Local certificate authority and per-boot leaf certificate. The CA key
//! never leaves the host; only the public CA certificate is offered for
//! download on the /setup page.

use std::net::IpAddr;
use std::path::Path;

use anyhow::Context;
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose,
    SanType,
};

#[derive(Clone)]
pub struct TlsIdentity {
    pub ca_cert_pem: String,
    pub server_cert_pem: String,
    pub server_key_pem: String,
}

const CA_CERT_FILE: &str = "localpad-ca.crt";
const CA_KEY_FILE: &str = "localpad-ca.key";

fn write_private(path: &Path, contents: &str) -> anyhow::Result<()> {
    std::fs::write(path, contents)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn load_or_create_ca(dir: &Path) -> anyhow::Result<(Certificate, KeyPair, String)> {
    let cert_path = dir.join(CA_CERT_FILE);
    let key_path = dir.join(CA_KEY_FILE);
    if cert_path.exists() && key_path.exists() {
        let cert_pem = std::fs::read_to_string(&cert_path)?;
        let key_pem = std::fs::read_to_string(&key_path)?;
        let key = KeyPair::from_pem(&key_pem).context("stored CA key is unreadable")?;
        let params = CertificateParams::from_ca_cert_pem(&cert_pem)
            .context("stored CA certificate is unreadable")?;
        // Re-derive an issuer certificate from the stored parameters. The
        // phone trusts the original PEM; signatures still verify because
        // the key is the same and the issuer name is preserved.
        let issuer = params.self_signed(&key)?;
        return Ok((issuer, key, cert_pem));
    }

    let mut params = CertificateParams::default();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params
        .distinguished_name
        .push(DnType::CommonName, "LocalPad Local CA");
    params
        .distinguished_name
        .push(DnType::OrganizationName, "LocalPad");
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let key = KeyPair::generate()?;
    let cert = params.self_signed(&key)?;
    let cert_pem = cert.pem();
    std::fs::create_dir_all(dir)?;
    std::fs::write(&cert_path, &cert_pem)?;
    write_private(&key_path, &key.serialize_pem())?;
    tracing::info!(path = %cert_path.display(), "created LocalPad local certificate authority");
    Ok((cert, key, cert_pem))
}

/// Load or create the CA, then issue a fresh leaf certificate covering
/// localpad.local and the host's current LAN addresses. The leaf is
/// regenerated on every boot so address changes never serve a stale SAN.
pub fn ensure_identity(dir: &Path, addresses: &[IpAddr]) -> anyhow::Result<TlsIdentity> {
    let (ca_cert, ca_key, ca_cert_pem) = load_or_create_ca(dir)?;

    let mut sans: Vec<SanType> = vec![
        SanType::DnsName("localpad.local".try_into()?),
        SanType::DnsName("localhost".try_into()?),
    ];
    for addr in addresses {
        sans.push(SanType::IpAddress(*addr));
    }
    let mut params = CertificateParams::default();
    params.subject_alt_names = sans;
    params
        .distinguished_name
        .push(DnType::CommonName, "LocalPad Server");
    let key = KeyPair::generate()?;
    let cert = params.signed_by(&key, &ca_cert, &ca_key)?;

    Ok(TlsIdentity {
        ca_cert_pem,
        server_cert_pem: cert.pem(),
        server_key_pem: key.serialize_pem(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn ca_persists_and_leaf_regenerates() {
        let dir = std::env::temp_dir().join(format!("localpad-tls-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let addrs = [IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20))];
        let first = ensure_identity(&dir, &addrs).unwrap();
        let second = ensure_identity(&dir, &addrs).unwrap();
        assert_eq!(first.ca_cert_pem, second.ca_cert_pem, "CA must be stable");
        assert_ne!(
            first.server_cert_pem, second.server_cert_pem,
            "leaf is per boot"
        );
        assert!(first.server_cert_pem.contains("BEGIN CERTIFICATE"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
