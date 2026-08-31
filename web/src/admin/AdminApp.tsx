import { useCallback, useEffect, useRef, useState } from "react";
import { apiGet, apiPost } from "../lib/api";
import { BUTTON_BITS } from "../protocol/buttons";
import type {
  AdminEvent,
  Layout,
  MonitorSnapshot,
  StatusResponse,
} from "../protocol/messages";

interface Toast {
  id: number;
  message: string;
}

interface Approval {
  requestId: string;
  name: string;
}

export function AdminApp() {
  const [status, setStatus] = useState<StatusResponse | null>(null);
  const [layouts, setLayouts] = useState<Layout[]>([]);
  const [monitor, setMonitor] = useState<MonitorSnapshot | null>(null);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [approvals, setApprovals] = useState<Approval[]>([]);
  const [connected, setConnected] = useState(false);
  const [qrBust, setQrBust] = useState(0);
  const toastId = useRef(0);

  const refetch = useCallback(() => {
    apiGet<StatusResponse>("/api/status").then(setStatus).catch(() => {});
    apiGet<{ layouts: Layout[] }>("/api/layouts")
      .then((r) => setLayouts(r.layouts))
      .catch(() => {});
  }, []);

  const pushToast = useCallback((message: string) => {
    const id = ++toastId.current;
    setToasts((t) => [...t, { id, message }]);
    window.setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), 4200);
  }, []);

  useEffect(() => {
    let socket: WebSocket | null = null;
    let closed = false;
    let retry: number | undefined;

    const connect = () => {
      const scheme = location.protocol === "https:" ? "wss" : "ws";
      socket = new WebSocket(`${scheme}://${location.host}/api/events`);
      socket.onopen = () => {
        setConnected(true);
        refetch();
      };
      socket.onmessage = (event) => {
        let parsed: AdminEvent;
        try {
          parsed = JSON.parse(event.data as string) as AdminEvent;
        } catch {
          return;
        }
        if (parsed.type === "status") {
          refetch();
          setQrBust((n) => n + 1);
        } else if (parsed.type === "monitor") {
          setMonitor(parsed);
        } else if (parsed.type === "toast") {
          pushToast(parsed.message);
        } else if (parsed.type === "approvalRequested") {
          setApprovals((a) => [
            ...a.filter((x) => x.requestId !== parsed.deviceId),
            { requestId: parsed.deviceId, name: parsed.name },
          ]);
        }
      };
      socket.onclose = () => {
        setConnected(false);
        if (!closed) retry = window.setTimeout(connect, 1500);
      };
    };
    connect();
    return () => {
      closed = true;
      window.clearTimeout(retry);
      socket?.close();
    };
  }, [refetch, pushToast]);

  const answerApproval = async (requestId: string, approve: boolean) => {
    setApprovals((a) => a.filter((x) => x.requestId !== requestId));
    try {
      await apiPost("/api/approval", { requestId: Number(requestId), approve });
    } catch {
      pushToast("Could not send the approval response.");
    }
  };

  if (!status) {
    return (
      <div className="admin">
        <Header connected={connected} />
        <p style={{ color: "var(--text-dim)" }}>
          Connecting to the LocalPad server on this computer...
        </p>
      </div>
    );
  }

  const activeDevice = status.devices.find((d) => d.connected) ?? null;

  return (
    <div className="admin">
      <Header connected={connected} phone={activeDevice?.name ?? null} />

      {status.warnings.map((w) => (
        <div className="warning-banner" key={w}>
          {w}
        </div>
      ))}

      {approvals.map((a) => (
        <div className="approval-banner" key={a.requestId}>
          <div style={{ flex: 1 }}>
            <strong>{a.name}</strong> wants to connect as a controller.
          </div>
          <button className="btn primary" onClick={() => answerApproval(a.requestId, true)}>
            Approve
          </button>
          <button className="btn danger" onClick={() => answerApproval(a.requestId, false)}>
            Deny
          </button>
        </div>
      ))}

      <div className="admin-grid">
        <section className="card span-5">
          <h2>Pair a phone</h2>
          <PairingCard status={status} qrBust={qrBust} onNew={() => setQrBust((n) => n + 1)} />
        </section>

        <section className="card span-7">
          <h2>Server</h2>
          <dl className="kv">
            <dt>Version</dt>
            <dd>{status.version}</dd>
            <dt>Controller URL</dt>
            <dd>{status.controllerUrl}</dd>
            <dt>Network</dt>
            <dd>{status.lanIp}</dd>
            <dt>Transport</dt>
            <dd>{status.secure ? "HTTPS with the LocalPad local CA" : "HTTP (insecure dev mode)"}</dd>
            <dt>Output backend</dt>
            <dd>{status.output.name}</dd>
            <dt>Output mode</dt>
            <dd>
              {status.output.mode}
              {status.output.dsuActive
                ? ` (DSU listening, ${status.output.dsuClients} emulator client${status.output.dsuClients === 1 ? "" : "s"})`
                : ""}
            </dd>
            <dt>Uptime</dt>
            <dd>{formatUptime(status.uptimeSecs)}</dd>
          </dl>
          <div style={{ display: "flex", gap: "0.6rem", marginTop: "1rem", flexWrap: "wrap" }}>
            <button
              className="btn danger"
              onClick={() => apiPost("/api/release").catch(() => pushToast("Release failed."))}
            >
              Release all inputs
            </button>
            <button
              className="btn"
              onClick={async () => {
                if (window.confirm("Stop the LocalPad server?")) {
                  await apiPost("/api/shutdown").catch(() => {});
                  pushToast("Server is stopping.");
                }
              }}
            >
              Stop server
            </button>
          </div>
        </section>

        <section className="card span-5">
          <h2>Phones</h2>
          {status.devices.length === 0 && (
            <p style={{ color: "var(--text-dim)", margin: 0 }}>
              No phone has paired yet. Scan the QR code with the phone camera.
            </p>
          )}
          {status.devices.map((d) => (
            <div className="device-row" key={d.deviceId}>
              <span className={`pill ${d.connected ? "live" : ""}`}>
                <span className="dot" />
                {d.connected ? "connected" : "paired"}
              </span>
              <span className="name">{d.name}</span>
              <span className="spacer" />
              {d.connected && (
                <button
                  className="btn"
                  onClick={() => apiPost("/api/disconnect", { deviceId: d.deviceId })}
                >
                  Disconnect
                </button>
              )}
              <button
                className="btn"
                onClick={() => apiPost("/api/disconnect", { deviceId: d.deviceId, forget: true })}
              >
                Forget
              </button>
            </div>
          ))}
        </section>

        <section className="card span-7">
          <h2>Input monitor</h2>
          <MonitorView monitor={monitor} hasDevice={activeDevice !== null} />
        </section>

        <section className="card span-12">
          <h2>Controller layout</h2>
          <div className="layout-grid">
            {layouts.map((layout) => (
              <button
                key={layout.id}
                className={`layout-tile ${layout.id === status.profile ? "active" : ""}`}
                onClick={() => apiPost("/api/profile", { id: layout.id }).catch(() => {})}
              >
                <div className="t-name">{layout.name}</div>
                <div className="t-output">{layout.output}</div>
              </button>
            ))}
          </div>
        </section>
      </div>

      <div className="toast-stack">
        {toasts.map((t) => (
          <div className="toast" key={t.id}>
            {t.message}
          </div>
        ))}
      </div>
    </div>
  );
}

function Header({ connected, phone }: { connected: boolean; phone?: string | null }) {
  return (
    <header className="admin-header">
      <div className="wordmark">
        <svg width="22" height="22" viewBox="0 0 32 32" aria-hidden>
          <rect width="32" height="32" rx="8" fill="var(--surface-raised)" />
          <rect x="7" y="11" width="18" height="10" rx="5" fill="none" stroke="var(--accent)" strokeWidth="2" />
          <circle cx="12" cy="16" r="1.6" fill="var(--accent)" />
          <circle cx="20" cy="16" r="1.6" fill="var(--accent)" />
        </svg>
        LocalPad
      </div>
      <span className={`pill ${connected ? "live" : "bad"}`}>
        <span className="dot" />
        {connected ? "server link" : "reconnecting"}
      </span>
      {phone && (
        <span className="pill live">
          <span className="dot" />
          {phone}
        </span>
      )}
      <span className="spacer" />
      <a href="/setup" target="_blank" rel="noreferrer">
        Phone HTTPS setup
      </a>
    </header>
  );
}

function PairingCard({
  status,
  qrBust,
  onNew,
}: {
  status: StatusResponse;
  qrBust: number;
  onNew: () => void;
}) {
  if (!status.pairing) {
    return (
      <div>
        <p style={{ color: "var(--text-dim)", marginTop: 0 }}>
          No pairing code is active. Create one to add a phone.
        </p>
        <button
          className="btn primary"
          onClick={async () => {
            await apiPost("/api/pairing/new").catch(() => {});
            onNew();
          }}
        >
          New pairing code
        </button>
      </div>
    );
  }
  return (
    <div>
      <div className="qr-wrap">
        <img src={`/api/qr.svg?v=${qrBust}`} alt="Pairing QR code" />
        <div>
          <p style={{ margin: "0 0 0.3rem", color: "var(--text-dim)", fontSize: "0.88rem" }}>
            Scan with the phone camera, or open the controller URL and type:
          </p>
          <div className="pairing-code">{status.pairing.code}</div>
          <p style={{ color: "var(--text-faint)", fontSize: "0.8rem" }}>
            Expires in {Math.max(1, Math.round(status.pairing.expiresInSecs / 60))} min. One use.
          </p>
          <button
            className="btn"
            onClick={async () => {
              await apiPost("/api/pairing/new").catch(() => {});
              onNew();
            }}
          >
            New code
          </button>
        </div>
      </div>
    </div>
  );
}

function MonitorView({
  monitor,
  hasDevice,
}: {
  monitor: MonitorSnapshot | null;
  hasDevice: boolean;
}) {
  if (!hasDevice) {
    return (
      <p style={{ color: "var(--text-dim)", margin: 0 }}>
        Live input appears here while a phone is connected.
      </p>
    );
  }
  if (!monitor) {
    return (
      <p style={{ color: "var(--text-dim)", margin: 0 }}>
        Connected. Waiting for the first input frame.
      </p>
    );
  }
  const f = monitor.frame;
  return (
    <div>
      <div className="monitor">
        <StickViz label="left" x={f.leftStick[0]} y={f.leftStick[1]} />
        <StickViz label="right" x={f.rightStick[0]} y={f.rightStick[1]} />
        <div className="monitor-buttons">
          {Object.entries(BUTTON_BITS).map(([name, bit]) => (
            <span key={name} className={`chip ${(monitor.rawButtons & bit) !== 0 ? "on" : ""}`}>
              {name.replace("dpad_", "")}
            </span>
          ))}
        </div>
      </div>
      <div className="stat-row">
        <Stat k="frames/s" v={monitor.framesPerSecond.toFixed(0)} />
        <Stat k="latency" v={monitor.latencyMs != null ? `${monitor.latencyMs.toFixed(0)} ms` : "..."} />
        <Stat k="held inputs" v={String(monitor.heldInputs)} />
        <Stat k="dropped" v={String(monitor.droppedFrames)} />
        <Stat
          k="pointer"
          v={`${f.mouseDelta[0].toFixed(0)}, ${f.mouseDelta[1].toFixed(0)}`}
        />
        <Stat k="triggers" v={`${f.triggers[0].toFixed(2)} / ${f.triggers[1].toFixed(2)}`} />
      </div>
    </div>
  );
}

function StickViz({ label, x, y }: { label: string; x: number; y: number }) {
  return (
    <div className="stick-viz">
      <div
        className="nub"
        style={{ left: `${50 + x * 38}%`, top: `${50 + y * 38}%` }}
      />
      <div className="axis-label">{label}</div>
    </div>
  );
}

function Stat({ k, v }: { k: string; v: string }) {
  return (
    <div className="stat">
      <span className="v">{v}</span>
      <span className="k">{k}</span>
    </div>
  );
}

function formatUptime(secs: number): string {
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m ${secs % 60}s`;
  return `${Math.floor(secs / 3600)}h ${Math.floor((secs % 3600) / 60)}m`;
}
