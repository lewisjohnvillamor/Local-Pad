//! DSU (CemuHook) motion server. Streams buttons, sticks and calibrated
//! motion to Dolphin and compatible emulators over UDP without creating a
//! system-wide virtual controller.
//!
//! Protocol summary (all little-endian):
//!   header: magic "DSUS", u16 version (1001), u16 payload length,
//!           u32 crc32 (packet with crc zeroed), u32 sender id
//!   payload: u32 message type, then message-specific bytes.
//!   message types: 0x100000 version, 0x100001 port info, 0x100002 pad data.

use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use localpad_core::frame::GamepadButton;
use localpad_core::mapping::{OutputEvent, OutputFrame};
use localpad_core::output::{InputOutput, OutputCapabilities};

const MAGIC_SERVER: &[u8; 4] = b"DSUS";
const MAGIC_CLIENT: &[u8; 4] = b"DSUC";
const PROTOCOL_VERSION: u16 = 1001;
const MSG_VERSION: u32 = 0x0010_0000;
const MSG_PORT_INFO: u32 = 0x0010_0001;
const MSG_PAD_DATA: u32 = 0x0010_0002;
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);
const PAD_MAC: [u8; 6] = [0x0a, 0x1c, 0x4d, 0x00, 0x00, 0x01];

pub const DEFAULT_DSU_PORT: u16 = 26760;

struct ClientState {
    last_request: Instant,
}

struct Shared {
    clients: Mutex<HashMap<SocketAddr, ClientState>>,
    server_id: u32,
}

/// Build one complete DSU packet from a payload (message type + body).
fn build_packet(server_id: u32, payload: &[u8]) -> Vec<u8> {
    let mut packet = Vec::with_capacity(16 + payload.len());
    packet.extend_from_slice(MAGIC_SERVER);
    packet.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    packet.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    packet.extend_from_slice(&0u32.to_le_bytes()); // crc placeholder
    packet.extend_from_slice(&server_id.to_le_bytes());
    packet.extend_from_slice(payload);
    let crc = crc32fast::hash(&packet);
    packet[8..12].copy_from_slice(&crc.to_le_bytes());
    packet
}

fn verify_packet(buf: &[u8]) -> Option<(u32, &[u8])> {
    if buf.len() < 20 || &buf[0..4] != MAGIC_CLIENT {
        return None;
    }
    let mut copy = buf.to_vec();
    let claimed = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
    copy[8..12].fill(0);
    if crc32fast::hash(&copy) != claimed {
        return None;
    }
    let msg_type = u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]);
    Some((msg_type, &buf[20..]))
}

/// Slot header shared by port info and pad data responses: slot, state,
/// model (2 = full gyro), connection (2 = bluetooth), mac, battery (5 = full).
fn slot_header(connected: bool) -> [u8; 11] {
    let mut b = [0u8; 11];
    b[0] = 0; // slot
    b[1] = if connected { 2 } else { 0 };
    b[2] = 2;
    b[3] = 2;
    b[4..10].copy_from_slice(&PAD_MAC);
    b[10] = if connected { 5 } else { 0 };
    b
}

fn stick_byte(v: f32, invert: bool) -> u8 {
    let v = if invert { -v } else { v };
    (((v + 1.0) / 2.0) * 255.0).round().clamp(0.0, 255.0) as u8
}

/// Encode the pad data payload (message type + 84 bytes) for one frame.
fn encode_pad_data(frame: &OutputFrame, packet_number: u32, connected: bool) -> Vec<u8> {
    let mut p = Vec::with_capacity(84);
    p.extend_from_slice(&MSG_PAD_DATA.to_le_bytes());
    p.extend_from_slice(&slot_header(connected));
    p.push(u8::from(connected));
    p.extend_from_slice(&packet_number.to_le_bytes());

    let b = frame.gamepad_buttons;
    let pressed = |btn: GamepadButton| b & btn.bit() != 0;
    // Bitmask 1, LSB first: Share, L3, R3, Options, Up, Right, Down, Left.
    let mask1 = u8::from(pressed(GamepadButton::Select))
        | (u8::from(pressed(GamepadButton::L3)) << 1)
        | (u8::from(pressed(GamepadButton::R3)) << 2)
        | (u8::from(pressed(GamepadButton::Start)) << 3)
        | (u8::from(pressed(GamepadButton::DpadUp)) << 4)
        | (u8::from(pressed(GamepadButton::DpadRight)) << 5)
        | (u8::from(pressed(GamepadButton::DpadDown)) << 6)
        | (u8::from(pressed(GamepadButton::DpadLeft)) << 7);
    // Bitmask 2, LSB first: L2, R2, L1, R1, Triangle(X), Circle(A),
    // Cross(B), Square(Y). We map A/B/X/Y by position: A=east, B=south,
    // X=north, Y=west, matching Nintendo-style labels used by our layouts.
    let l2_digital = frame.triggers[0] > 0.5 || pressed(GamepadButton::L2);
    let r2_digital = frame.triggers[1] > 0.5 || pressed(GamepadButton::R2);
    let mask2 = u8::from(l2_digital)
        | (u8::from(r2_digital) << 1)
        | (u8::from(pressed(GamepadButton::L1)) << 2)
        | (u8::from(pressed(GamepadButton::R1)) << 3)
        | (u8::from(pressed(GamepadButton::X)) << 4)
        | (u8::from(pressed(GamepadButton::A)) << 5)
        | (u8::from(pressed(GamepadButton::B)) << 6)
        | (u8::from(pressed(GamepadButton::Y)) << 7);
    p.push(mask1);
    p.push(mask2);
    p.push(u8::from(pressed(GamepadButton::Guide))); // Home
    p.push(0); // Touch button

    p.push(stick_byte(frame.left_stick[0], false));
    p.push(stick_byte(frame.left_stick[1], true));
    p.push(stick_byte(frame.right_stick[0], false));
    p.push(stick_byte(frame.right_stick[1], true));

    // Analog pressure for dpad L/D/R/U, then Y/B/A/X, then R1/L1, R2/L2.
    let digital = |on: bool| if on { 255u8 } else { 0 };
    p.push(digital(pressed(GamepadButton::DpadLeft)));
    p.push(digital(pressed(GamepadButton::DpadDown)));
    p.push(digital(pressed(GamepadButton::DpadRight)));
    p.push(digital(pressed(GamepadButton::DpadUp)));
    p.push(digital(pressed(GamepadButton::Y)));
    p.push(digital(pressed(GamepadButton::B)));
    p.push(digital(pressed(GamepadButton::A)));
    p.push(digital(pressed(GamepadButton::X)));
    p.push(digital(pressed(GamepadButton::R1)));
    p.push(digital(pressed(GamepadButton::L1)));
    p.push((frame.triggers[1] * 255.0).round().clamp(0.0, 255.0) as u8);
    p.push((frame.triggers[0] * 255.0).round().clamp(0.0, 255.0) as u8);

    // Two touch slots, inactive.
    p.extend_from_slice(&[0u8; 12]);

    // Motion timestamp in microseconds.
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64;
    p.extend_from_slice(&ts.to_le_bytes());

    let motion = frame.motion.unwrap_or_default();
    // Accelerometer in g, then gyro pitch/yaw/roll in deg/s.
    p.extend_from_slice(&motion.acceleration[0].to_le_bytes());
    p.extend_from_slice(&motion.acceleration[1].to_le_bytes());
    p.extend_from_slice(&motion.acceleration[2].to_le_bytes());
    p.extend_from_slice(&motion.angular_velocity[0].to_le_bytes());
    p.extend_from_slice(&motion.angular_velocity[1].to_le_bytes());
    p.extend_from_slice(&motion.angular_velocity[2].to_le_bytes());
    p
}

/// The DSU server output adapter.
pub struct DsuOutput {
    socket: UdpSocket,
    shared: Arc<Shared>,
    packet_number: u32,
    port: u16,
}

impl DsuOutput {
    pub fn bind(port: u16) -> anyhow::Result<Self> {
        let socket = UdpSocket::bind(("0.0.0.0", port))?;
        socket.set_nonblocking(false)?;
        let shared = Arc::new(Shared {
            clients: Mutex::new(HashMap::new()),
            server_id: rand::random(),
        });
        let recv_socket = socket.try_clone()?;
        let recv_shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("dsu-recv".into())
            .spawn(move || receiver_loop(recv_socket, recv_shared))?;
        socket.set_nonblocking(true)?;
        tracing::info!(port, "DSU server listening");
        Ok(DsuOutput {
            socket,
            shared,
            packet_number: 0,
            port,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn client_count(&self) -> usize {
        self.shared.clients.lock().unwrap().len()
    }

    fn broadcast(&mut self, frame: &OutputFrame, connected: bool) {
        self.packet_number = self.packet_number.wrapping_add(1);
        let payload = encode_pad_data(frame, self.packet_number, connected);
        let packet = build_packet(self.shared.server_id, &payload);
        let mut clients = self.shared.clients.lock().unwrap();
        clients.retain(|_, c| c.last_request.elapsed() < CLIENT_TIMEOUT);
        for addr in clients.keys() {
            let _ = self.socket.send_to(&packet, addr);
        }
    }
}

fn receiver_loop(socket: UdpSocket, shared: Arc<Shared>) {
    let mut buf = [0u8; 128];
    loop {
        let (len, addr) = match socket.recv_from(&mut buf) {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(_) => return,
        };
        let Some((msg_type, _body)) = verify_packet(&buf[..len]) else {
            continue;
        };
        match msg_type {
            MSG_VERSION => {
                let mut payload = Vec::with_capacity(8);
                payload.extend_from_slice(&MSG_VERSION.to_le_bytes());
                payload.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
                payload.extend_from_slice(&[0, 0]);
                let _ = socket.send_to(&build_packet(shared.server_id, &payload), addr);
            }
            MSG_PORT_INFO => {
                // Report slot 0 as connected; slots 1..3 unused this release.
                let mut payload = Vec::with_capacity(16);
                payload.extend_from_slice(&MSG_PORT_INFO.to_le_bytes());
                payload.extend_from_slice(&slot_header(true));
                payload.push(0);
                let _ = socket.send_to(&build_packet(shared.server_id, &payload), addr);
            }
            MSG_PAD_DATA => {
                shared
                    .clients
                    .lock()
                    .unwrap()
                    .insert(addr, ClientState { last_request: Instant::now() });
            }
            _ => {}
        }
    }
}

impl InputOutput for DsuOutput {
    fn apply_frame(&mut self, frame: &OutputFrame) -> anyhow::Result<()> {
        self.broadcast(frame, true);
        Ok(())
    }

    fn apply_event(&mut self, _event: &OutputEvent) -> anyhow::Result<()> {
        // DSU carries no keyboard or mouse transitions.
        Ok(())
    }

    fn release_all(&mut self) -> anyhow::Result<()> {
        self.broadcast(&OutputFrame::default(), true);
        Ok(())
    }

    fn capabilities(&self) -> OutputCapabilities {
        OutputCapabilities {
            pointer: false,
            keyboard: false,
            gamepad: true,
            motion: true,
            name: "DSU motion server",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client_packet(msg_type: u32, body: &[u8]) -> Vec<u8> {
        let mut packet = Vec::new();
        packet.extend_from_slice(MAGIC_CLIENT);
        packet.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        packet.extend_from_slice(&((body.len() + 4) as u16).to_le_bytes());
        packet.extend_from_slice(&0u32.to_le_bytes());
        packet.extend_from_slice(&7u32.to_le_bytes());
        packet.extend_from_slice(&msg_type.to_le_bytes());
        packet.extend_from_slice(body);
        let crc = crc32fast::hash(&packet);
        packet[8..12].copy_from_slice(&crc.to_le_bytes());
        packet
    }

    #[test]
    fn packet_crc_roundtrip() {
        let packet = build_packet(42, &MSG_VERSION.to_le_bytes());
        let mut copy = packet.clone();
        let claimed = u32::from_le_bytes(packet[8..12].try_into().unwrap());
        copy[8..12].fill(0);
        assert_eq!(crc32fast::hash(&copy), claimed);
        assert_eq!(&packet[0..4], MAGIC_SERVER);
    }

    #[test]
    fn rejects_bad_crc() {
        let mut packet = client_packet(MSG_VERSION, &[]);
        let last = packet.len() - 1;
        packet[last] ^= 0xff;
        assert!(verify_packet(&packet).is_none());
    }

    #[test]
    fn pad_data_is_expected_size() {
        let payload = encode_pad_data(&OutputFrame::default(), 1, true);
        // 4 type + 11 slot + 1 connected + 4 counter + 4 masks/home/touch
        // + 4 sticks + 12 analog + 12 touch + 8 timestamp + 24 motion = 84.
        assert_eq!(payload.len(), 84);
        let packet = build_packet(1, &payload);
        assert_eq!(packet.len(), 100);
    }

    #[test]
    fn sticks_encode_centered_and_edges() {
        let mut frame = OutputFrame::default();
        let payload = encode_pad_data(&frame, 1, true);
        // Stick bytes at offsets 24..28 of the payload.
        assert!((126..=129).contains(&payload[24]));
        frame.left_stick = [1.0, -1.0]; // full right, full up
        let payload = encode_pad_data(&frame, 2, true);
        assert_eq!(payload[24], 255);
        assert_eq!(payload[25], 255, "up must encode as 255");
    }

    #[test]
    fn version_and_registration_flow() {
        let out = DsuOutput::bind(0).unwrap();
        let port = out.socket.local_addr().unwrap().port();
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        client
            .send_to(&client_packet(MSG_VERSION, &[]), ("127.0.0.1", port))
            .unwrap();
        let mut buf = [0u8; 128];
        let (len, _) = client.recv_from(&mut buf).unwrap();
        assert_eq!(&buf[0..4], MAGIC_SERVER);
        let version = u16::from_le_bytes([buf[20], buf[21]]);
        assert_eq!(version, PROTOCOL_VERSION);
        assert!(len >= 22);

        // Register for pad data, then push a frame and expect a data packet.
        let mut body = vec![1u8, 0u8];
        body.extend_from_slice(&[0u8; 6]);
        client
            .send_to(&client_packet(MSG_PAD_DATA, &body), ("127.0.0.1", port))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut out = out;
        while out.client_count() == 0 {
            assert!(Instant::now() < deadline, "registration never arrived");
            std::thread::sleep(Duration::from_millis(10));
        }
        out.apply_frame(&OutputFrame::default()).unwrap();
        let (len, _) = client.recv_from(&mut buf).unwrap();
        assert_eq!(len, 100);
        let msg_type = u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]);
        assert_eq!(msg_type, MSG_PAD_DATA);
    }
}
