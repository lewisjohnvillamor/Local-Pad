//! LAN-facing controller listener: static controller/setup pages, the
//! pairing endpoint and the input WebSocket. This router never exposes
//! admin routes, filesystem access or configuration.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use localpad_core::frame::{Key, MouseButton};
use localpad_core::mapping::MappingEngine;
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::assets;
use crate::pairing::RedeemOutcome;
use crate::protocol::{
    ClientMessage, ErrorCode, ServerMessage, HEARTBEAT_INTERVAL_MS, MAX_WS_MESSAGE_BYTES,
    STALE_AFTER_MS,
};
use crate::sessions::{ActiveConnection, ConnCommand};
use crate::state::{AdminEvent, AppState, MonitorSnapshot, PendingApproval, SERVER_VERSION};

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(spa))
        .route("/controller", get(spa))
        .route("/controller/{*rest}", get(spa))
        .route("/setup", get(spa))
        .route("/setup/{*rest}", get(setup_route))
        .route("/assets/{*path}", get(serve_asset))
        .route("/favicon.svg", get(|| async { assets::asset("favicon.svg") }))
        .route("/api/pair", post(pair))
        .route("/ws", get(input_ws))
        .layer(axum::middleware::from_fn(add_headers))
        .with_state(state)
}

async fn add_headers(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    assets::security_headers(next.run(request).await)
}

async fn spa() -> Response {
    assets::index()
}

async fn setup_route(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(rest): axum::extract::Path<String>,
) -> Response {
    if rest == "localpad-ca.crt" {
        return match &state.tls {
            Some(identity) => (
                [
                    (axum::http::header::CONTENT_TYPE, "application/x-x509-ca-cert"),
                    (
                        axum::http::header::CONTENT_DISPOSITION,
                        "attachment; filename=\"localpad-ca.crt\"",
                    ),
                ],
                identity.ca_cert_pem.clone(),
            )
                .into_response(),
            None => (StatusCode::NOT_FOUND, "server is running without TLS").into_response(),
        };
    }
    assets::index()
}

async fn serve_asset(axum::extract::Path(path): axum::extract::Path<String>) -> Response {
    assets::asset(&format!("assets/{path}"))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PairRequest {
    /// Pairing secret from the QR fragment, or the six-digit code.
    code: String,
    #[serde(default)]
    device_name: Option<String>,
}

fn clean_device_name(name: Option<String>) -> String {
    let name = name.unwrap_or_default();
    let cleaned: String = name
        .chars()
        .filter(|c| !c.is_control())
        .take(48)
        .collect();
    if cleaned.trim().is_empty() {
        "Phone".to_string()
    } else {
        cleaned.trim().to_string()
    }
}

async fn pair(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(request): Json<PairRequest>,
) -> Response {
    let outcome = state
        .pairing
        .lock()
        .unwrap()
        .redeem(addr.ip(), &request.code);
    match outcome {
        RedeemOutcome::RateLimited => {
            return (StatusCode::TOO_MANY_REQUESTS, "too many attempts").into_response();
        }
        RedeemOutcome::Rejected => {
            return (StatusCode::FORBIDDEN, "invalid or expired pairing code").into_response();
        }
        RedeemOutcome::Accepted => {}
    }

    let name = clean_device_name(request.device_name);

    if state.config.require_approval {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let request_id = state.next_approval_id();
        state.approvals.lock().unwrap().insert(
            request_id,
            PendingApproval {
                device_id_hint: request_id,
                name: name.clone(),
                ip: addr.ip(),
                respond: tx,
            },
        );
        state.broadcast(AdminEvent::ApprovalRequested {
            device_id: request_id.to_string(),
            name: name.clone(),
        });
        state.broadcast(AdminEvent::Status);
        let approved = tokio::time::timeout(Duration::from_secs(60), rx)
            .await
            .ok()
            .and_then(|r| r.ok())
            .unwrap_or(false);
        state.approvals.lock().unwrap().remove(&request_id);
        if !approved {
            state.broadcast(AdminEvent::Status);
            return (StatusCode::FORBIDDEN, "the computer declined this device").into_response();
        }
    }

    let (token, session) = state
        .sessions
        .lock()
        .unwrap()
        .issue(&name, addr.ip(), true);
    {
        let mut prefs = state.prefs.lock().unwrap();
        if !prefs.known_devices.contains(&name) {
            prefs.known_devices.push(name.clone());
            prefs.save(&state.data_dir);
        }
    }
    state.broadcast(AdminEvent::Status);
    tracing::info!(device = %session.device_id, name = %name, ip = %addr.ip(), "device paired");
    Json(serde_json::json!({
        "token": token,
        "deviceId": session.device_id,
        "deviceName": name,
    }))
    .into_response()
}

async fn input_ws(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    upgrade: WebSocketUpgrade,
) -> Response {
    upgrade
        .max_message_size(MAX_WS_MESSAGE_BYTES)
        .max_frame_size(MAX_WS_MESSAGE_BYTES)
        .on_upgrade(move |socket| input_socket(state, socket, addr))
}

async fn send_msg(socket: &mut WebSocket, msg: &ServerMessage) -> bool {
    match serde_json::to_string(msg) {
        Ok(text) => socket.send(Message::Text(text.into())).await.is_ok(),
        Err(_) => false,
    }
}

async fn input_socket(state: Arc<AppState>, mut socket: WebSocket, addr: SocketAddr) {
    // First message must authenticate within 5 seconds.
    let auth = tokio::time::timeout(Duration::from_secs(5), socket.recv()).await;
    let token = match auth {
        Ok(Some(Ok(Message::Text(text)))) => {
            match serde_json::from_str::<ClientMessage>(&text) {
                Ok(ClientMessage::Auth { token }) => token,
                _ => {
                    let _ = send_msg(
                        &mut socket,
                        &ServerMessage::Error {
                            code: ErrorCode::AuthRequired,
                            message: "authenticate first".to_string(),
                        },
                    )
                    .await;
                    return;
                }
            }
        }
        _ => return,
    };
    let device = state.sessions.lock().unwrap().authenticate(&token);
    let Some(device) = device else {
        let _ = send_msg(
            &mut socket,
            &ServerMessage::Error {
                code: ErrorCode::BadToken,
                message: "unknown or expired session; pair again".to_string(),
            },
        )
        .await;
        return;
    };

    // One controller at a time. A reconnect from the same device replaces
    // the old connection; a different device is refused.
    let (command_tx, mut command_rx) = mpsc::channel::<ConnCommand>(16);
    {
        let old = {
            let sessions = state.sessions.lock().unwrap();
            sessions.active.as_ref().map(|a| (a.device_id.clone(), a.commands.clone()))
        };
        if let Some((old_id, old_commands)) = old {
            if old_id == device.device_id {
                let _ = old_commands
                    .send(ConnCommand::Disconnect {
                        reason: "replaced by a new connection".to_string(),
                    })
                    .await;
            } else {
                let _ = send_msg(
                    &mut socket,
                    &ServerMessage::Error {
                        code: ErrorCode::Busy,
                        message: "another controller is already connected".to_string(),
                    },
                )
                .await;
                return;
            }
        }
        state.sessions.lock().unwrap().active = Some(ActiveConnection {
            device_id: device.device_id.clone(),
            commands: command_tx.clone(),
            connected_at: Instant::now(),
        });
    }
    state.broadcast(AdminEvent::Status);
    tracing::info!(device = %device.device_id, ip = %addr.ip(), "controller connected");

    let layout = state.active_layout();
    let mut engine = MappingEngine::new(&layout);
    let welcome = ServerMessage::Welcome {
        device_id: device.device_id.clone(),
        device_name: device.name.clone(),
        layout,
        server_version: SERVER_VERSION.to_string(),
        heartbeat_interval_ms: HEARTBEAT_INTERVAL_MS,
    };
    if !send_msg(&mut socket, &welcome).await {
        cleanup(&state, &device.device_id, &mut engine).await;
        return;
    }

    let mut last_seen = Instant::now();
    let mut last_frame_at = Instant::now();
    let mut last_sequence: u64 = 0;
    let mut bad_messages: u32 = 0;
    let mut dropped_frames: u64 = 0;
    // Token bucket: 120 frames per second sustained, burst of 240.
    let mut bucket: f32 = 240.0;
    let mut bucket_refill_at = Instant::now();
    // Latency measurement over WebSocket ping/pong.
    let mut last_ping_sent: Option<Instant> = None;
    let mut latency_ms: Option<f32> = None;
    // Admin monitor throttle.
    let mut last_monitor_at = Instant::now() - Duration::from_secs(1);
    let mut frames_window: u32 = 0;
    let mut window_start = Instant::now();
    let mut frames_per_second: f32 = 0.0;

    let mut watchdog = tokio::time::interval(Duration::from_millis(500));
    let mut shutdown = state.shutdown_rx();

    let disconnect_reason: String = loop {
        tokio::select! {
            _ = watchdog.tick() => {
                if last_seen.elapsed() > Duration::from_millis(STALE_AFTER_MS) {
                    break "heartbeat timed out".to_string();
                }
                if last_ping_sent.is_none() {
                    last_ping_sent = Some(Instant::now());
                    let _ = socket.send(Message::Ping(Vec::new().into())).await;
                }
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    let _ = send_msg(&mut socket, &ServerMessage::Bye {
                        reason: "server stopping".to_string(),
                    }).await;
                    break "server stopping".to_string();
                }
            }
            command = command_rx.recv() => {
                match command {
                    Some(ConnCommand::SetLayout(id)) => {
                        if let Some(layout) = state.layouts.get(&id) {
                            apply_release(&state, &mut engine);
                            engine = MappingEngine::new(layout);
                            let _ = send_msg(&mut socket, &ServerMessage::Layout {
                                layout: layout.clone(),
                            }).await;
                        }
                    }
                    Some(ConnCommand::Recenter) => engine.recenter(),
                    Some(ConnCommand::ReleaseAll) => apply_release(&state, &mut engine),
                    Some(ConnCommand::Disconnect { reason }) => {
                        let _ = send_msg(&mut socket, &ServerMessage::Bye { reason: reason.clone() }).await;
                        break reason;
                    }
                    None => break "command channel closed".to_string(),
                }
            }
            incoming = socket.recv() => {
                let message = match incoming {
                    Some(Ok(m)) => m,
                    Some(Err(_)) | None => break "socket closed".to_string(),
                };
                match message {
                    Message::Text(text) => {
                        let parsed: Result<ClientMessage, _> = serde_json::from_str(&text);
                        let Ok(parsed) = parsed else {
                            bad_messages += 1;
                            if bad_messages > 20 {
                                break "too many malformed messages".to_string();
                            }
                            continue;
                        };
                        last_seen = Instant::now();
                        match parsed {
                            ClientMessage::Frame(mut frame) => {
                                // Refill the rate bucket.
                                let elapsed = bucket_refill_at.elapsed().as_secs_f32();
                                bucket_refill_at = Instant::now();
                                bucket = (bucket + elapsed * 120.0).min(240.0);
                                if bucket < 1.0 {
                                    dropped_frames += 1;
                                    continue;
                                }
                                bucket -= 1.0;

                                if frame.sanitize().is_err() {
                                    bad_messages += 1;
                                    dropped_frames += 1;
                                    if bad_messages > 20 {
                                        break "too many invalid frames".to_string();
                                    }
                                    continue;
                                }
                                // Drop stale or duplicate sequence numbers.
                                if frame.sequence != 0 && frame.sequence <= last_sequence {
                                    dropped_frames += 1;
                                    continue;
                                }
                                last_sequence = frame.sequence;

                                let dt = last_frame_at.elapsed().as_secs_f32().clamp(0.0, 0.1);
                                last_frame_at = Instant::now();
                                let (out, events) = engine.process_frame(&frame, dt);
                                {
                                    let mut outputs = state.outputs.lock().unwrap();
                                    outputs.apply_events(&events);
                                    outputs.apply_frame(&out);
                                }

                                frames_window += 1;
                                if window_start.elapsed() >= Duration::from_secs(1) {
                                    frames_per_second =
                                        frames_window as f32 / window_start.elapsed().as_secs_f32();
                                    frames_window = 0;
                                    window_start = Instant::now();
                                }
                                if last_monitor_at.elapsed() >= Duration::from_millis(66) {
                                    last_monitor_at = Instant::now();
                                    state.broadcast(AdminEvent::Monitor(MonitorSnapshot {
                                        frame: out,
                                        raw_buttons: frame.buttons,
                                        frames_per_second,
                                        latency_ms,
                                        held_inputs: engine.held_count(),
                                        sequence: frame.sequence,
                                        dropped_frames,
                                    }));
                                }
                            }
                            ClientMessage::Key { code, down } => {
                                match Key::new(&code) {
                                    Some(key) => {
                                        let events = engine.process_key(key, down);
                                        state.outputs.lock().unwrap().apply_events(&events);
                                    }
                                    None => {
                                        bad_messages += 1;
                                        if bad_messages > 20 {
                                            break "too many invalid keys".to_string();
                                        }
                                    }
                                }
                            }
                            ClientMessage::MouseButton { button, down } => {
                                let button = match button.as_str() {
                                    "left" => Some(MouseButton::Left),
                                    "right" => Some(MouseButton::Right),
                                    "middle" => Some(MouseButton::Middle),
                                    _ => None,
                                };
                                if let Some(button) = button {
                                    let events = engine.process_mouse_button(button, down);
                                    state.outputs.lock().unwrap().apply_events(&events);
                                }
                            }
                            ClientMessage::Neutral => apply_release(&state, &mut engine),
                            ClientMessage::Recenter => engine.recenter(),
                            ClientMessage::Heartbeat { t } => {
                                let _ = send_msg(&mut socket, &ServerMessage::HeartbeatAck { t }).await;
                            }
                            ClientMessage::SetLayout { id } => {
                                if state.layouts.contains_key(&id) {
                                    // set_profile pushes the layout back via
                                    // our command channel.
                                    let state = state.clone();
                                    let id = id.clone();
                                    tokio::spawn(async move {
                                        state.set_profile(&id).await;
                                    });
                                }
                            }
                            ClientMessage::Bye => break "controller left".to_string(),
                            ClientMessage::Auth { .. } => {} // already authenticated
                        }
                    }
                    Message::Pong(_) => {
                        if let Some(sent) = last_ping_sent.take() {
                            latency_ms = Some(sent.elapsed().as_secs_f32() * 1000.0);
                        }
                        last_seen = Instant::now();
                    }
                    Message::Ping(_) => { last_seen = Instant::now(); }
                    Message::Close(_) => break "socket closed".to_string(),
                    Message::Binary(_) => {
                        bad_messages += 1;
                        if bad_messages > 20 {
                            break "binary messages are not part of the protocol".to_string();
                        }
                    }
                }
            }
        }
    };

    tracing::info!(device = %device.device_id, reason = %disconnect_reason, "controller disconnected");
    cleanup(&state, &device.device_id, &mut engine).await;
}

/// Release everything the engine holds and push the releases to outputs.
fn apply_release(state: &AppState, engine: &mut MappingEngine) {
    let events = engine.release_all();
    let mut outputs = state.outputs.lock().unwrap();
    outputs.apply_events(&events);
    let _ = outputs.release_all();
}

async fn cleanup(state: &Arc<AppState>, device_id: &str, engine: &mut MappingEngine) {
    apply_release(state, engine);
    {
        let mut sessions = state.sessions.lock().unwrap();
        if sessions
            .active
            .as_ref()
            .is_some_and(|a| a.device_id == device_id)
        {
            sessions.active = None;
        }
    }
    state.broadcast(AdminEvent::Status);
}
