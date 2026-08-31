//! Loopback-only admin API and dashboard. This router is never mounted on
//! the LAN listener. State-changing endpoints require a custom header so a
//! malicious web page cannot drive them cross-origin.

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::assets;
use crate::sessions::ConnCommand;
use crate::state::{AdminEvent, AppState, SERVER_VERSION};

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(|| async { Redirect::temporary("/admin") }))
        .route("/admin", get(spa))
        .route("/admin/{*rest}", get(spa))
        .route("/assets/{*path}", get(serve_asset))
        .route("/favicon.svg", get(|| async { assets::asset("favicon.svg") }))
        .route("/api/status", get(status))
        .route("/api/qr.svg", get(qr_svg))
        .route("/api/layouts", get(layouts))
        .route("/api/pairing/new", post(pairing_new))
        .route("/api/profile", post(set_profile))
        .route("/api/release", post(release))
        .route("/api/disconnect", post(disconnect))
        .route("/api/approval", post(approval))
        .route("/api/shutdown", post(shutdown))
        .route("/api/events", get(events_ws))
        .layer(axum::middleware::from_fn(guard_local))
        .with_state(state)
}

/// Reject requests whose Host header is not loopback (DNS rebinding guard)
/// and require the custom admin header on every POST (CSRF guard: browsers
/// only attach custom headers after a CORS preflight, which we never allow).
async fn guard_local(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let host_ok = request
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(|h| {
            let bare = h.rsplit_once(':').map_or(h, |(name, _)| name);
            matches!(bare, "127.0.0.1" | "localhost" | "[::1]")
        })
        .unwrap_or(false);
    if !host_ok {
        return (StatusCode::FORBIDDEN, "admin is loopback only").into_response();
    }
    if request.method() == axum::http::Method::POST
        && !request.headers().contains_key("x-localpad-admin")
    {
        return (StatusCode::FORBIDDEN, "missing X-LocalPad-Admin header").into_response();
    }
    assets::security_headers(next.run(request).await)
}

async fn spa() -> Response {
    assets::index()
}

async fn serve_asset(axum::extract::Path(path): axum::extract::Path<String>) -> Response {
    assets::asset(&format!("assets/{path}"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusResponse {
    version: String,
    uptime_secs: u64,
    lan_ip: String,
    all_ips: Vec<String>,
    admin_port: u16,
    controller_port: u16,
    controller_url: String,
    secure: bool,
    require_approval: bool,
    profile: String,
    output: OutputStatus,
    devices: Vec<crate::sessions::DeviceSummary>,
    pairing: Option<PairingStatus>,
    warnings: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OutputStatus {
    name: String,
    pointer: bool,
    keyboard: bool,
    gamepad: bool,
    motion: bool,
    mode: localpad_core::layout::OutputMode,
    dsu_active: bool,
    dsu_clients: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PairingStatus {
    code: String,
    url: String,
    expires_in_secs: u64,
}

fn controller_url(state: &AppState) -> String {
    let scheme = if state.config.insecure_http { "http" } else { "https" };
    format!(
        "{scheme}://{}:{}/controller",
        state.network.lan_ip, state.config.controller_port
    )
}

async fn status(State(state): State<Arc<AppState>>) -> Json<StatusResponse> {
    let (caps, mode, dsu_active, dsu_clients, warning) = {
        let outputs = state.outputs.lock().unwrap();
        (
            outputs.capabilities(),
            outputs.mode(),
            outputs.dsu_active(),
            outputs.dsu_clients(),
            outputs.platform_warning.clone(),
        )
    };
    let mut warnings = Vec::new();
    if let Some(w) = warning {
        warnings.push(w);
    }
    if state.config.insecure_http {
        warnings.push(
            "Running without HTTPS. Motion controls are unavailable on phones.".to_string(),
        );
    }
    let pairing = state.pairing_display().map(|p| PairingStatus {
        code: p.code,
        url: p.url,
        expires_in_secs: p.expires_in.as_secs(),
    });
    Json(StatusResponse {
        version: SERVER_VERSION.to_string(),
        uptime_secs: state.started_at.elapsed().as_secs(),
        lan_ip: state.network.lan_ip.to_string(),
        all_ips: state.network.all_ips.iter().map(|ip| ip.to_string()).collect(),
        admin_port: state.config.admin_port,
        controller_port: state.config.controller_port,
        controller_url: controller_url(&state),
        secure: !state.config.insecure_http,
        require_approval: state.config.require_approval,
        profile: state.active_profile.lock().unwrap().clone(),
        output: OutputStatus {
            name: caps.name.to_string(),
            pointer: caps.pointer,
            keyboard: caps.keyboard,
            gamepad: caps.gamepad,
            motion: caps.motion,
            mode,
            dsu_active,
            dsu_clients,
        },
        devices: state.sessions.lock().unwrap().summaries(),
        pairing,
        warnings,
    })
}

async fn layouts(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let mut list: Vec<&localpad_core::layout::Layout> = state.layouts.values().collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    Json(serde_json::json!({ "layouts": list }))
}

async fn qr_svg(State(state): State<Arc<AppState>>) -> Response {
    let Some(pairing) = state.pairing_display() else {
        return (StatusCode::NOT_FOUND, "no active pairing").into_response();
    };
    match qrcode::QrCode::new(pairing.url.as_bytes()) {
        Ok(code) => {
            let svg = code
                .render::<qrcode::render::svg::Color>()
                .quiet_zone(true)
                .min_dimensions(240, 240)
                .dark_color(qrcode::render::svg::Color("#101312"))
                .light_color(qrcode::render::svg::Color("#f4f6f4"))
                .build();
            (
                [(axum::http::header::CONTENT_TYPE, "image/svg+xml")],
                svg,
            )
                .into_response()
        }
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "qr failed").into_response(),
    }
}

async fn pairing_new(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let display = state.new_pairing(&controller_url(&state)).await;
    Json(serde_json::json!({
        "code": display.code,
        "url": display.url,
        "expiresInSecs": display.expires_in.as_secs(),
    }))
}

#[derive(Deserialize)]
struct ProfileRequest {
    id: String,
}

async fn set_profile(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ProfileRequest>,
) -> Response {
    match state.set_profile(&request.id).await {
        Some(layout) => Json(serde_json::json!({ "ok": true, "layout": layout })).into_response(),
        None => (StatusCode::NOT_FOUND, "unknown layout").into_response(),
    }
}

async fn release(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    state.release_active().await;
    state.broadcast(AdminEvent::Toast {
        level: "info".to_string(),
        message: "All inputs released".to_string(),
    });
    Json(serde_json::json!({ "ok": true }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DisconnectRequest {
    device_id: String,
    #[serde(default)]
    forget: bool,
}

async fn disconnect(
    State(state): State<Arc<AppState>>,
    Json(request): Json<DisconnectRequest>,
) -> Json<serde_json::Value> {
    let commands = {
        let sessions = state.sessions.lock().unwrap();
        sessions
            .active
            .as_ref()
            .filter(|a| a.device_id == request.device_id)
            .map(|a| a.commands.clone())
    };
    if let Some(commands) = commands {
        let _ = commands
            .send(ConnCommand::Disconnect {
                reason: "disconnected from the dashboard".to_string(),
            })
            .await;
    }
    if request.forget {
        state.sessions.lock().unwrap().revoke(&request.device_id);
    }
    state.broadcast(AdminEvent::Status);
    Json(serde_json::json!({ "ok": true }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApprovalRequest {
    request_id: u32,
    approve: bool,
}

async fn approval(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ApprovalRequest>,
) -> Response {
    let pending = state.approvals.lock().unwrap().remove(&request.request_id);
    match pending {
        Some(p) => {
            let _ = p.respond.send(request.approve);
            state.broadcast(AdminEvent::Status);
            Json(serde_json::json!({ "ok": true })).into_response()
        }
        None => (StatusCode::NOT_FOUND, "no such approval request").into_response(),
    }
}

async fn shutdown(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    state.broadcast(AdminEvent::Toast {
        level: "info".to_string(),
        message: "Server stopping".to_string(),
    });
    let state_clone = state.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        state_clone.shutdown();
    });
    Json(serde_json::json!({ "ok": true }))
}

async fn events_ws(
    State(state): State<Arc<AppState>>,
    upgrade: WebSocketUpgrade,
    headers: HeaderMap,
) -> Response {
    // Same-origin only: browsers always send Origin on WebSocket requests.
    let origin_ok = headers
        .get(axum::http::header::ORIGIN)
        .and_then(|o| o.to_str().ok())
        .map(|o| o.contains("127.0.0.1") || o.contains("localhost") || o.contains("[::1]"))
        .unwrap_or(true); // non-browser clients (CLI) send no Origin
    if !origin_ok {
        return (StatusCode::FORBIDDEN, "cross-origin admin socket").into_response();
    }
    upgrade.on_upgrade(move |socket| admin_socket(state, socket))
}

async fn admin_socket(state: Arc<AppState>, mut socket: WebSocket) {
    let mut events = state.subscribe();
    // Prime the dashboard so it renders without waiting for a change.
    let hello = serde_json::to_string(&AdminEvent::Status).unwrap();
    if socket.send(Message::Text(hello.into())).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            event = events.recv() => {
                match event {
                    Ok(event) => {
                        let text = match serde_json::to_string(&event) {
                            Ok(t) => t,
                            Err(_) => continue,
                        };
                        if socket.send(Message::Text(text.into())).await.is_err() {
                            return;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => return,
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(_)) => continue, // dashboard sends nothing meaningful
                    _ => return,
                }
            }
        }
    }
}
