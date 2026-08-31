//! Integration tests: pairing over HTTP, WebSocket auth, the input loop,
//! and the single-controller rule, all against a real server instance on
//! ephemeral ports.

use futures_util::{SinkExt, StreamExt};
use localpad_server::config::ServerConfig;
use tokio_tungstenite::tungstenite::Message;

async fn start_test_server() -> localpad_server::RunningServer {
    let dir = std::env::temp_dir().join(format!(
        "localpad-test-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let config = ServerConfig {
        admin_port: 0,
        controller_port: 0,
        insecure_http: true,
        no_native_output: true,
        data_dir: Some(dir),
        ..Default::default()
    };
    localpad_server::start(config).await.expect("server starts")
}

fn rand_suffix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos() as u64
}

async fn pair(server: &localpad_server::RunningServer) -> (String, String) {
    let display = server.state.new_pairing("http://test/controller").await;
    let addr = server.controller_addr;
    let client = std::sync::Arc::new(());
    let _ = client;
    let body = serde_json::json!({ "code": display.code, "deviceName": "Test phone" });
    let response = tokio::task::spawn_blocking(move || {
        ureq::post(&format!("http://{addr}/api/pair"))
            .send_json(body)
            .expect("pair request succeeds")
            .into_json::<serde_json::Value>()
            .unwrap()
    })
    .await
    .unwrap();
    (
        response["token"].as_str().unwrap().to_string(),
        response["deviceId"].as_str().unwrap().to_string(),
    )
}

type Socket = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;

async fn connect_ws(server: &localpad_server::RunningServer, token: &str) -> Socket {
    let (mut socket, _) =
        tokio_tungstenite::connect_async(format!("ws://{}/ws", server.controller_addr))
            .await
            .expect("ws connects");
    socket
        .send(Message::Text(
            serde_json::json!({ "type": "auth", "token": token }).to_string().into(),
        ))
        .await
        .unwrap();
    socket
}

async fn next_json(socket: &mut Socket) -> serde_json::Value {
    loop {
        let message = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
            .await
            .expect("message before timeout")
            .expect("stream open")
            .expect("no ws error");
        if let Message::Text(text) = message {
            return serde_json::from_str(&text).unwrap();
        }
    }
}

#[tokio::test]
async fn pair_connect_input_disconnect() {
    let server = start_test_server().await;
    let (token, device_id) = pair(&server).await;

    let mut socket = connect_ws(&server, &token).await;
    let welcome = next_json(&mut socket).await;
    assert_eq!(welcome["type"], "welcome");
    assert_eq!(welcome["deviceId"], device_id.as_str());
    assert_eq!(welcome["layout"]["id"], "touchpad");
    assert!(welcome["heartbeatIntervalMs"].is_number(), "field casing");

    // Device shows as connected.
    let devices = server.state.sessions.lock().unwrap().summaries();
    assert!(devices.iter().any(|d| d.device_id == device_id && d.connected));

    // A valid frame and a heartbeat round-trip.
    socket
        .send(Message::Text(
            serde_json::json!({
                "type": "frame",
                "protocolVersion": 1,
                "sequence": 1,
                "buttons": 1u32 << 4,
                "pointerDelta": [4.0, -1.0],
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    socket
        .send(Message::Text(
            serde_json::json!({ "type": "heartbeat", "t": 12.5 }).to_string().into(),
        ))
        .await
        .unwrap();
    let ack = next_json(&mut socket).await;
    assert_eq!(ack["type"], "heartbeatAck");
    assert_eq!(ack["t"], 12.5);

    // Clean goodbye frees the slot.
    socket
        .send(Message::Text(
            serde_json::json!({ "type": "bye" }).to_string().into(),
        ))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(server.state.sessions.lock().unwrap().active.is_none());
}

#[tokio::test]
async fn bad_token_is_rejected() {
    let server = start_test_server().await;
    let mut socket = connect_ws(&server, "not-a-real-token").await;
    let reply = next_json(&mut socket).await;
    assert_eq!(reply["type"], "error");
    assert_eq!(reply["code"], "bad_token");
}

#[tokio::test]
async fn second_device_is_busy() {
    let server = start_test_server().await;
    let (token_a, _) = pair(&server).await;
    let (token_b, _) = pair(&server).await;

    let mut first = connect_ws(&server, &token_a).await;
    let welcome = next_json(&mut first).await;
    assert_eq!(welcome["type"], "welcome");

    let mut second = connect_ws(&server, &token_b).await;
    let reply = next_json(&mut second).await;
    assert_eq!(reply["type"], "error");
    assert_eq!(reply["code"], "busy");

    // The first connection is unaffected.
    first
        .send(Message::Text(
            serde_json::json!({ "type": "heartbeat", "t": 1.0 }).to_string().into(),
        ))
        .await
        .unwrap();
    let ack = next_json(&mut first).await;
    assert_eq!(ack["type"], "heartbeatAck");
}

#[tokio::test]
async fn same_device_reconnect_replaces_old_connection() {
    let server = start_test_server().await;
    let (token, _) = pair(&server).await;

    let mut first = connect_ws(&server, &token).await;
    assert_eq!(next_json(&mut first).await["type"], "welcome");

    let mut second = connect_ws(&server, &token).await;
    assert_eq!(next_json(&mut second).await["type"], "welcome");

    // The first socket gets a goodbye.
    let bye = next_json(&mut first).await;
    assert_eq!(bye["type"], "bye");
}

#[tokio::test]
async fn expired_pairing_code_is_refused() {
    let server = start_test_server().await;
    let _ = server.state.new_pairing("http://test/controller").await;
    let addr = server.controller_addr;
    let status = tokio::task::spawn_blocking(move || {
        match ureq::post(&format!("http://{addr}/api/pair"))
            .send_json(serde_json::json!({ "code": "000-000" }))
        {
            Ok(_) => 200,
            Err(ureq::Error::Status(code, _)) => code,
            Err(_) => 0,
        }
    })
    .await
    .unwrap();
    assert_eq!(status, 403);
}
