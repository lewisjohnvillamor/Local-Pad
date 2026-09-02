// TypeScript mirror of crates/localpad-server/src/protocol.rs and the
// core frame/layout types. Keep field names in camelCase to match serde.

export const PROTOCOL_VERSION = 1;

export interface InputFrame {
  protocolVersion: number;
  sequence: number;
  clientTimeMs: number;
  buttons: number;
  leftStick: [number, number];
  rightStick: [number, number];
  triggers: [number, number];
  pointerDelta: [number, number];
  scrollDelta: [number, number];
  orientation: [number, number, number, number] | null;
  angularVelocity: [number, number, number] | null;
  acceleration: [number, number, number] | null;
}

export type ControlKind =
  | "button"
  | "dpad"
  | "stick"
  | "touchpad"
  | "scroll"
  | "trigger"
  | "keyboard"
  | "motion";

export interface Control {
  type: ControlKind;
  id?: string;
  label?: string;
  x: number;
  y: number;
  size: number;
  width?: number;
  height?: number;
}

export interface Layout {
  schemaVersion: number;
  id: string;
  name: string;
  orientation: "landscape" | "portrait" | "any";
  output: "pointer" | "keyboard" | "dsu" | "gamepad";
  description?: string;
  controls: Control[];
  bindings: Record<string, string>;
}

export type ClientMessage =
  | { type: "auth"; token: string }
  | ({ type: "frame" } & InputFrame)
  | { type: "key"; code: string; down: boolean }
  | { type: "mouseButton"; button: "left" | "right" | "middle"; down: boolean }
  | { type: "neutral" }
  | { type: "recenter" }
  | { type: "heartbeat"; t: number }
  | { type: "setLayout"; id: string }
  | { type: "bye" };

export interface LayoutSummary {
  id: string;
  name: string;
  orientation: "landscape" | "portrait" | "any";
  output: "pointer" | "keyboard" | "dsu" | "gamepad";
}

export type ServerMessage =
  | {
      type: "welcome";
      deviceId: string;
      deviceName: string;
      layout: Layout;
      layouts: LayoutSummary[];
      serverVersion: string;
      heartbeatIntervalMs: number;
    }
  | { type: "layout"; layout: Layout }
  | { type: "heartbeatAck"; t: number }
  | { type: "pendingApproval" }
  | { type: "error"; code: string; message: string }
  | { type: "bye"; reason: string };

export interface DeviceSummary {
  deviceId: string;
  name: string;
  connected: boolean;
  approved: boolean;
}

export interface StatusResponse {
  version: string;
  uptimeSecs: number;
  lanIp: string;
  allIps: string[];
  adminPort: number;
  controllerPort: number;
  controllerUrl: string;
  secure: boolean;
  requireApproval: boolean;
  profile: string;
  output: {
    name: string;
    pointer: boolean;
    keyboard: boolean;
    gamepad: boolean;
    motion: boolean;
    mode: string;
    dsuActive: boolean;
    dsuClients: number;
  };
  devices: DeviceSummary[];
  pairing: { code: string; url: string; expiresInSecs: number } | null;
  warnings: string[];
}

export interface MonitorSnapshot {
  frame: {
    mouseDelta: [number, number];
    scrollDelta: [number, number];
    gamepadButtons: number;
    leftStick: [number, number];
    rightStick: [number, number];
    triggers: [number, number];
    motion: unknown;
  };
  rawButtons: number;
  framesPerSecond: number;
  latencyMs: number | null;
  heldInputs: number;
  sequence: number;
  droppedFrames: number;
}

export type AdminEvent =
  | { type: "status" }
  | ({ type: "monitor" } & MonitorSnapshot)
  | { type: "approvalRequested"; deviceId: string; name: string }
  | { type: "toast"; level: string; message: string };
