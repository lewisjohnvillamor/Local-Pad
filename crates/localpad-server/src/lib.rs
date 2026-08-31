//! The LocalPad host server: a loopback admin dashboard, a LAN HTTPS
//! controller listener, pairing, sessions, and the bridge from WebSocket
//! input to native output adapters.

pub mod admin;
pub mod assets;
pub mod config;
pub mod controller;
pub mod mdns;
pub mod netinfo;
pub mod outputs;
pub mod pairing;
pub mod protocol;
pub mod sessions;
pub mod state;
pub mod tls;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use anyhow::Context;

pub use config::ServerConfig;
pub use state::AppState;

/// Everything a caller needs to know about a running server.
pub struct RunningServer {
    pub admin_addr: SocketAddr,
    pub controller_addr: SocketAddr,
    pub controller_url: String,
    pub pairing: pairing::PairingDisplay,
    pub state: Arc<AppState>,
    admin_task: tokio::task::JoinHandle<()>,
    controller_task: tokio::task::JoinHandle<()>,
    _mdns: Option<mdns::MdnsAdvertisement>,
}

impl RunningServer {
    /// Wait until the server is asked to shut down, then stop listeners.
    pub async fn wait(self) {
        let mut shutdown = self.state.shutdown_rx();
        // Wait for the shutdown signal to flip to true.
        while !*shutdown.borrow() {
            if shutdown.changed().await.is_err() {
                break;
            }
        }
        // Give in-flight release_all work a moment, then stop listeners.
        self.state.release_active().await;
        self.admin_task.abort();
        self.controller_task.abort();
    }
}

/// Start both listeners, mDNS and the pairing session, without blocking.
pub async fn start(config: ServerConfig) -> anyhow::Result<RunningServer> {
    let state = Arc::new(AppState::new(config).await?);

    // Admin listener: loopback only, plain HTTP.
    let admin_router = admin::router(state.clone());
    let admin_bind = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), state.config.admin_port);
    let admin_listener = tokio::net::TcpListener::bind(admin_bind)
        .await
        .with_context(|| format!("could not bind admin port {admin_bind}"))?;
    let admin_addr = admin_listener.local_addr()?;
    let admin_task = tokio::spawn(async move {
        if let Err(e) = axum::serve(admin_listener, admin_router).await {
            tracing::error!(error = %e, "admin listener stopped");
        }
    });

    // Controller listener: LAN, HTTPS unless --insecure-http.
    let controller_router = controller::router(state.clone());
    let controller_bind = SocketAddr::new(state.config.bind_addr, state.config.controller_port);
    let (controller_addr, controller_task) = if state.config.insecure_http {
        let listener = tokio::net::TcpListener::bind(controller_bind)
            .await
            .with_context(|| format!("could not bind controller port {controller_bind}"))?;
        let addr = listener.local_addr()?;
        let task = tokio::spawn(async move {
            if let Err(e) = axum::serve(
                listener,
                controller_router
                    .into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            {
                tracing::error!(error = %e, "controller listener stopped");
            }
        });
        (addr, task)
    } else {
        let identity = state.tls.clone().context("TLS identity missing")?;
        let rustls_config = axum_server::tls_rustls::RustlsConfig::from_pem(
            identity.server_cert_pem.into_bytes(),
            identity.server_key_pem.into_bytes(),
        )
        .await
        .context("could not load the generated TLS certificate")?;
        let listener = std::net::TcpListener::bind(controller_bind)
            .with_context(|| format!("could not bind controller port {controller_bind}"))?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let task = tokio::spawn(async move {
            if let Err(e) = axum_server::from_tcp_rustls(listener, rustls_config)
                .serve(
                    controller_router
                        .into_make_service_with_connect_info::<SocketAddr>(),
                )
                .await
            {
                tracing::error!(error = %e, "controller listener stopped");
            }
        });
        (addr, task)
    };

    let scheme = if state.config.insecure_http { "http" } else { "https" };
    let lan_ip = state.network.lan_ip;
    let controller_url = format!("{scheme}://{lan_ip}:{}/controller", controller_addr.port());

    let mdns = match mdns::advertise(&state.network, controller_addr.port()) {
        Ok(m) => Some(m),
        Err(e) => {
            tracing::warn!(error = %e, "mDNS advertisement failed; use the IP address URL");
            None
        }
    };

    let pairing = state.new_pairing(&controller_url).await;

    Ok(RunningServer {
        admin_addr,
        controller_addr,
        controller_url,
        pairing,
        state,
        admin_task,
        controller_task,
        _mdns: mdns,
    })
}
