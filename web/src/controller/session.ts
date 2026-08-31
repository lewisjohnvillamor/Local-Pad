// Controller session: pairing, the WebSocket, the 60 Hz frame loop, wake
// lock and lifecycle handling. React components mutate the input state and
// subscribe to connection changes.

import type { ClientMessage, InputFrame, Layout, ServerMessage } from "../protocol/messages";
import { PROTOCOL_VERSION } from "../protocol/messages";
import { MotionCapture, requestMotionPermission } from "./motion";

export type Phase =
  | "needs-pairing"
  | "pairing"
  | "connecting"
  | "waiting-approval"
  | "connected"
  | "ended";

export interface SessionView {
  phase: Phase;
  layout: Layout | null;
  deviceName: string;
  latencyMs: number | null;
  motionOn: boolean;
  error: string | null;
  paused: boolean;
}

const TOKEN_KEY = "localpad-token";
const FRAME_INTERVAL_MS = 1000 / 60;

function guessDeviceName(): string {
  const ua = navigator.userAgent;
  if (/iPhone/.test(ua)) return "iPhone";
  if (/iPad/.test(ua)) return "iPad";
  if (/Android/.test(ua)) return "Android phone";
  return "Phone";
}

export class ControllerSession {
  // Mutable input state, written by controls and drained by the frame loop.
  buttons = 0;
  leftStick: [number, number] = [0, 0];
  rightStick: [number, number] = [0, 0];
  triggers: [number, number] = [0, 0];
  private pointerAccum: [number, number] = [0, 0];
  private scrollAccum: [number, number] = [0, 0];

  readonly motion = new MotionCapture();

  private view: SessionView = {
    phase: "needs-pairing",
    layout: null,
    deviceName: guessDeviceName(),
    latencyMs: null,
    motionOn: false,
    error: null,
    paused: false,
  };
  private listeners = new Set<() => void>();
  private socket: WebSocket | null = null;
  private sequence = 0;
  private lastSentButtons = 0;
  private lastSentSticks = "";
  private frameTimer: number | undefined;
  private heartbeatTimer: number | undefined;
  private reconnectTimer: number | undefined;
  private reconnectDelay = 500;
  private manualClose = false;
  private wakeLock: WakeLockSentinel | null = null;

  // ---- React subscription plumbing ----

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  getView = (): SessionView => this.view;

  private update(partial: Partial<SessionView>) {
    this.view = { ...this.view, ...partial };
    this.listeners.forEach((l) => l());
  }

  // ---- input mutation API used by controls ----

  setButton(bit: number, down: boolean) {
    this.buttons = down ? this.buttons | bit : this.buttons & ~bit;
  }

  setDpadBits(mask: number, bits: number) {
    this.buttons = (this.buttons & ~mask) | bits;
  }

  addPointer(dx: number, dy: number) {
    this.pointerAccum[0] += dx;
    this.pointerAccum[1] += dy;
  }

  addScroll(dx: number, dy: number) {
    this.scrollAccum[0] += dx;
    this.scrollAccum[1] += dy;
  }

  sendKey(code: string, down: boolean) {
    this.send({ type: "key", code, down });
  }

  sendMouseButton(button: "left" | "right" | "middle", down: boolean) {
    this.send({ type: "mouseButton", button, down });
  }

  tapMouse(button: "left" | "right" | "middle") {
    this.sendMouseButton(button, true);
    window.setTimeout(() => this.sendMouseButton(button, false), 50);
  }

  tapKey(code: string) {
    this.sendKey(code, true);
    window.setTimeout(() => this.sendKey(code, false), 50);
  }

  recenter() {
    this.send({ type: "recenter" });
  }

  releaseEverything() {
    this.neutralizeLocal();
    this.send({ type: "neutral" });
  }

  async enableMotion(): Promise<boolean> {
    if (!window.isSecureContext) {
      this.update({
        error: "Motion needs the HTTPS setup. Open the setup page from the dashboard.",
      });
      return false;
    }
    const granted = await requestMotionPermission();
    if (granted) {
      this.motion.start();
      this.update({ motionOn: true, error: null });
    } else {
      this.update({ error: "The browser declined motion access." });
    }
    return granted;
  }

  // ---- pairing ----

  get hasToken(): boolean {
    try {
      return localStorage.getItem(TOKEN_KEY) !== null;
    } catch {
      return false;
    }
  }

  async pair(code: string): Promise<boolean> {
    this.update({ phase: "pairing", error: null });
    try {
      const response = await fetch("/api/pair", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ code, deviceName: this.view.deviceName }),
      });
      if (!response.ok) {
        const message =
          response.status === 429
            ? "Too many attempts. Wait a minute and try again."
            : "That code was not accepted. Create a fresh one on the computer.";
        this.update({ phase: "needs-pairing", error: message });
        return false;
      }
      const body = (await response.json()) as { token: string; deviceName: string };
      try {
        localStorage.setItem(TOKEN_KEY, body.token);
      } catch {
        // Private browsing: keep going with the in-memory token.
      }
      this.connect(body.token);
      return true;
    } catch {
      this.update({
        phase: "needs-pairing",
        error: "Could not reach the LocalPad server. Same Wi-Fi network?",
      });
      return false;
    }
  }

  /** Called once on mount: consume a QR fragment or resume a stored token. */
  begin() {
    const match = window.location.hash.match(/pair=([A-Za-z0-9_-]+)/);
    if (match) {
      history.replaceState(null, "", window.location.pathname);
      void this.pair(match[1]);
      return;
    }
    let token: string | null = null;
    try {
      token = localStorage.getItem(TOKEN_KEY);
    } catch {
      token = null;
    }
    if (token) {
      this.connect(token);
    } else {
      this.update({ phase: "needs-pairing" });
    }
  }

  // ---- socket lifecycle ----

  private connect(token: string) {
    this.manualClose = false;
    this.update({ phase: "connecting", error: null });
    const scheme = location.protocol === "https:" ? "wss" : "ws";
    const socket = new WebSocket(`${scheme}://${location.host}/ws`);
    this.socket = socket;

    socket.onopen = () => {
      socket.send(JSON.stringify({ type: "auth", token } satisfies ClientMessage));
    };

    socket.onmessage = (event) => {
      let message: ServerMessage;
      try {
        message = JSON.parse(event.data as string) as ServerMessage;
      } catch {
        return;
      }
      switch (message.type) {
        case "welcome":
          this.reconnectDelay = 500;
          this.neutralizeLocal();
          this.update({
            phase: "connected",
            layout: message.layout,
            deviceName: message.deviceName,
            error: null,
          });
          this.startLoops();
          void this.acquireWakeLock();
          break;
        case "layout":
          this.neutralizeLocal();
          this.update({ layout: message.layout });
          break;
        case "heartbeatAck":
          this.update({ latencyMs: Math.max(0, performance.now() - message.t) });
          break;
        case "pendingApproval":
          this.update({ phase: "waiting-approval" });
          break;
        case "error":
          if (message.code === "bad_token") {
            try {
              localStorage.removeItem(TOKEN_KEY);
            } catch {
              // ignore
            }
            this.manualClose = true;
            this.update({ phase: "needs-pairing", error: "Session expired. Pair again." });
          } else if (message.code === "busy") {
            this.manualClose = true;
            this.update({
              phase: "ended",
              error: "Another phone is connected. Disconnect it from the dashboard first.",
            });
          }
          break;
        case "bye":
          this.manualClose = true;
          this.update({ phase: "ended", error: message.reason });
          break;
      }
    };

    socket.onclose = () => {
      this.stopLoops();
      if (this.manualClose) return;
      this.update({ phase: "connecting" });
      this.reconnectTimer = window.setTimeout(() => {
        let token: string | null = null;
        try {
          token = localStorage.getItem(TOKEN_KEY);
        } catch {
          token = null;
        }
        if (token) this.connect(token);
        else this.update({ phase: "needs-pairing" });
      }, this.reconnectDelay);
      this.reconnectDelay = Math.min(this.reconnectDelay * 2, 5000);
    };
  }

  private send(message: ClientMessage) {
    if (this.socket && this.socket.readyState === WebSocket.OPEN) {
      this.socket.send(JSON.stringify(message));
    }
  }

  private neutralizeLocal() {
    this.buttons = 0;
    this.lastSentButtons = 0;
    this.leftStick = [0, 0];
    this.rightStick = [0, 0];
    this.triggers = [0, 0];
    this.pointerAccum = [0, 0];
    this.scrollAccum = [0, 0];
  }

  private startLoops() {
    this.stopLoops();
    this.frameTimer = window.setInterval(() => this.tick(), FRAME_INTERVAL_MS);
    this.heartbeatTimer = window.setInterval(() => {
      this.send({ type: "heartbeat", t: performance.now() });
    }, 1000);
  }

  private stopLoops() {
    window.clearInterval(this.frameTimer);
    window.clearInterval(this.heartbeatTimer);
    this.frameTimer = undefined;
    this.heartbeatTimer = undefined;
  }

  private tick() {
    if (this.view.paused) return;
    const pointer: [number, number] = [this.pointerAccum[0], this.pointerAccum[1]];
    const scroll: [number, number] = [this.scrollAccum[0], this.scrollAccum[1]];
    this.pointerAccum = [0, 0];
    this.scrollAccum = [0, 0];

    const motionSample = this.view.motionOn ? this.motion.sample() : null;
    const sticksKey = JSON.stringify([this.leftStick, this.rightStick, this.triggers]);
    const dirty =
      pointer[0] !== 0 ||
      pointer[1] !== 0 ||
      scroll[0] !== 0 ||
      scroll[1] !== 0 ||
      this.buttons !== this.lastSentButtons ||
      sticksKey !== this.lastSentSticks ||
      (motionSample !== null && motionSample.orientation !== null);
    if (!dirty) return;

    this.sequence += 1;
    const frame: { type: "frame" } & InputFrame = {
      type: "frame",
      protocolVersion: PROTOCOL_VERSION,
      sequence: this.sequence,
      clientTimeMs: performance.now(),
      buttons: this.buttons,
      leftStick: this.leftStick,
      rightStick: this.rightStick,
      triggers: this.triggers,
      pointerDelta: pointer,
      scrollDelta: scroll,
      orientation: motionSample?.orientation ?? null,
      angularVelocity: motionSample?.angularVelocity ?? null,
      acceleration: motionSample?.acceleration ?? null,
    };
    this.lastSentButtons = this.buttons;
    this.lastSentSticks = sticksKey;
    this.send(frame);
  }

  // ---- browser lifecycle ----

  private async acquireWakeLock() {
    try {
      if ("wakeLock" in navigator) {
        this.wakeLock = await navigator.wakeLock.request("screen");
      }
    } catch {
      // Not fatal; the phone may dim.
    }
  }

  handleVisibilityChange() {
    if (document.visibilityState === "hidden") {
      this.update({ paused: true });
      this.releaseEverything();
    } else {
      this.update({ paused: false });
      void this.acquireWakeLock();
    }
  }

  handlePageHide() {
    this.releaseEverything();
    this.send({ type: "bye" });
  }

  destroy() {
    this.manualClose = true;
    this.stopLoops();
    window.clearTimeout(this.reconnectTimer);
    this.motion.stop();
    this.wakeLock?.release().catch(() => {});
    this.socket?.close();
  }
}
