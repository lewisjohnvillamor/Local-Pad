//! mDNS advertisement so phones can reach the controller at
//! localpad.local without typing an IP address.

use mdns_sd::{ServiceDaemon, ServiceInfo};

use crate::netinfo::NetworkInfo;

pub struct MdnsAdvertisement {
    daemon: ServiceDaemon,
    fullname: String,
}

pub fn advertise(network: &NetworkInfo, port: u16) -> anyhow::Result<MdnsAdvertisement> {
    let daemon = ServiceDaemon::new()?;
    let ip_strings: Vec<String> = network.all_ips.iter().map(|ip| ip.to_string()).collect();
    let service = ServiceInfo::new(
        "_localpad._tcp.local.",
        "LocalPad",
        "localpad.local.",
        ip_strings.join(",").as_str(),
        port,
        &[("path", "/controller")][..],
    )?;
    let fullname = service.get_fullname().to_string();
    daemon.register(service)?;
    tracing::info!("advertising localpad.local over mDNS");
    Ok(MdnsAdvertisement { daemon, fullname })
}

impl Drop for MdnsAdvertisement {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
    }
}
