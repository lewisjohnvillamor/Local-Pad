import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { ControllerSession } from "./session";
import { LayoutView } from "./LayoutView";
import type { LayoutSummary } from "../protocol/messages";

function requestFullscreen() {
  const el = document.documentElement;
  if (!document.fullscreenElement && el.requestFullscreen) {
    el.requestFullscreen().catch(() => {});
  }
}

export function ControllerApp() {
  const session = useMemo(() => new ControllerSession(), []);
  const view = useSyncExternalStore(session.subscribe, session.getView);
  const [isPortrait, setIsPortrait] = useState(
    window.innerHeight >= window.innerWidth
  );
  const [sheet, setSheet] = useState<null | "layouts" | "settings">(null);
  const wentFullscreen = useRef(false);

  useEffect(() => {
    session.begin();
    const onVisibility = () => session.handleVisibilityChange();
    const onPageHide = () => session.handlePageHide();
    const onResize = () => setIsPortrait(window.innerHeight >= window.innerWidth);
    document.addEventListener("visibilitychange", onVisibility);
    window.addEventListener("pagehide", onPageHide);
    window.addEventListener("resize", onResize);
    return () => {
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("pagehide", onPageHide);
      window.removeEventListener("resize", onResize);
      session.destroy();
    };
  }, [session]);

  if (view.phase !== "connected") {
    return <ConnectScreen session={session} phase={view.phase} error={view.error} />;
  }

  const layout = view.layout!;
  const wrongOrientation =
    (layout.orientation === "landscape" && isPortrait) ||
    (layout.orientation === "portrait" && !isPortrait);

  const needsMotion =
    layout.controls.some((c) => c.type === "motion") && !view.motionOn;

  return (
    <div
      className="controller-shell"
      onPointerDown={() => {
        // Hide the browser chrome on the first real touch; must come from
        // a user gesture, so it cannot happen at connect time.
        if (!wentFullscreen.current) {
          wentFullscreen.current = true;
          requestFullscreen();
        }
      }}
    >
      <div className="controller-topbar">
        <span className="title">{layout.name}</span>
        <span className={`pill ${view.paused ? "bad" : "live"}`}>
          <span className="dot" />
          {view.paused
            ? "paused"
            : view.latencyMs != null
              ? `${view.latencyMs.toFixed(0)} ms`
              : "linked"}
        </span>
        <span className="spacer" />
        <button
          className="btn"
          aria-label="Switch layout"
          onClick={() => setSheet(sheet === "layouts" ? null : "layouts")}
        >
          Layouts
        </button>
        <button
          className="btn icon"
          aria-label="Controller settings"
          onClick={() => setSheet(sheet === "settings" ? null : "settings")}
        >
          <GearIcon />
        </button>
        <button className="btn" onClick={() => session.releaseEverything()}>
          Release
        </button>
      </div>
      <div className="play-area control-surface">
        <LayoutView layout={layout} session={session} motionOn={view.motionOn} />
      </div>

      {sheet === "layouts" && (
        <Sheet title="Layouts" onClose={() => setSheet(null)}>
          <div className="sheet-grid">
            {view.layouts.map((entry: LayoutSummary) => (
              <button
                key={entry.id}
                className={`layout-tile ${entry.id === layout.id ? "active" : ""}`}
                onClick={() => {
                  session.requestLayout(entry.id);
                  setSheet(null);
                }}
              >
                <div className="t-name">{entry.name}</div>
                <div className="t-output">{entry.output}</div>
              </button>
            ))}
          </div>
        </Sheet>
      )}

      {sheet === "settings" && (
        <Sheet title="Settings" onClose={() => setSheet(null)}>
          <SettingsPanel session={session} />
        </Sheet>
      )}

      {needsMotion && (
        <div className="overlay" style={{ background: "rgba(15, 18, 17, 0.94)" }}>
          <div className="inner">
            <h1>Motion controls</h1>
            <p>
              This layout uses the gyroscope. The browser will ask for
              permission once.
            </p>
            <button className="btn primary" onClick={() => void session.enableMotion()}>
              Enable motion
            </button>
            <button
              className="btn"
              onClick={() => setSheet("layouts")}
            >
              Pick another layout
            </button>
            {view.error && <p style={{ color: "var(--danger)" }}>{view.error}</p>}
          </div>
        </div>
      )}

      {wrongOrientation && (
        <div className="overlay">
          <div className="inner" style={{ textAlign: "center" }}>
            <h1>Rotate the phone</h1>
            <p>
              The {layout.name} layout is designed for{" "}
              {layout.orientation === "landscape" ? "landscape" : "portrait"}.
            </p>
          </div>
        </div>
      )}

      {view.paused && (
        <div className="overlay">
          <div className="inner" style={{ textAlign: "center" }}>
            <h1>Paused</h1>
            <p>All inputs were released while the browser was in the background.</p>
          </div>
        </div>
      )}
    </div>
  );
}

function GearIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" aria-hidden>
      <path
        d="M12 15.5a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7Zm7.4-2.6.1-.9-.1-.9 2-1.6-2-3.4-2.4 1a7.6 7.6 0 0 0-1.6-.9L15 3.5h-4l-.4 2.7c-.6.2-1.1.5-1.6.9l-2.4-1-2 3.4 2 1.6-.1.9.1.9-2 1.6 2 3.4 2.4-1c.5.4 1 .7 1.6.9l.4 2.7h4l.4-2.7c.6-.2 1.1-.5 1.6-.9l2.4 1 2-3.4-2-1.6Z"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function Sheet({
  title,
  onClose,
  children,
}: {
  title: string;
  onClose: () => void;
  children: React.ReactNode;
}) {
  return (
    <div className="sheet-backdrop" onClick={onClose}>
      <div className="sheet" onClick={(e) => e.stopPropagation()}>
        <div className="sheet-head">
          <h2>{title}</h2>
          <button className="btn" onClick={onClose}>
            Done
          </button>
        </div>
        {children}
      </div>
    </div>
  );
}

function SettingsPanel({ session }: { session: ControllerSession }) {
  const view = useSyncExternalStore(session.subscribe, session.getView);
  const s = view.settings;
  return (
    <div className="settings-panel">
      <label className="setting-row">
        <span>Pointer speed</span>
        <input
          type="range"
          min={0.4}
          max={3}
          step={0.1}
          value={s.pointerSpeed}
          onChange={(e) => session.updateSettings({ pointerSpeed: Number(e.target.value) })}
        />
        <span className="mono">{s.pointerSpeed.toFixed(1)}x</span>
      </label>
      <label className="setting-row">
        <span>Scroll speed</span>
        <input
          type="range"
          min={0.4}
          max={3}
          step={0.1}
          value={s.scrollSpeed}
          onChange={(e) => session.updateSettings({ scrollSpeed: Number(e.target.value) })}
        />
        <span className="mono">{s.scrollSpeed.toFixed(1)}x</span>
      </label>
      <label className="setting-row toggle">
        <span>Invert scroll direction</span>
        <input
          type="checkbox"
          checked={!s.naturalScroll}
          onChange={(e) => session.updateSettings({ naturalScroll: !e.target.checked })}
        />
      </label>
      <label className="setting-row toggle">
        <span>Vibration on press</span>
        <input
          type="checkbox"
          checked={s.haptics}
          onChange={(e) => session.updateSettings({ haptics: e.target.checked })}
        />
      </label>
    </div>
  );
}

function ConnectScreen({
  session,
  phase,
  error,
}: {
  session: ControllerSession;
  phase: string;
  error: string | null;
}) {
  const [code, setCode] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);
  const isIos = /iPhone|iPad/.test(navigator.userAgent);

  const statusLine: Record<string, string> = {
    "needs-pairing": "not paired",
    pairing: "pairing...",
    connecting: "connecting...",
    ended: "disconnected",
  };

  return (
    <div className="overlay">
      <div className="inner">
        <h1>LocalPad controller</h1>
        <p>
          Scan the QR code on the computer, or type the six digit code shown
          in the dashboard.
        </p>
        {(phase === "needs-pairing" || phase === "ended") && (
          <>
            <div className="field">
              <label htmlFor="pair-code">Pairing code</label>
              <input
                id="pair-code"
                ref={inputRef}
                inputMode="numeric"
                autoComplete="one-time-code"
                placeholder="000-000"
                value={code}
                onChange={(e) => setCode(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && code.trim()) void session.pair(code.trim());
                }}
              />
              {error && <span className="error">{error}</span>}
            </div>
            <button
              className="btn primary"
              disabled={!code.trim()}
              onClick={() => {
                requestFullscreen();
                void session.pair(code.trim());
              }}
            >
              Connect
            </button>
          </>
        )}
        <div className="connect-status">{statusLine[phase] ?? phase}</div>
        {isIos && (
          <p style={{ fontSize: "0.82rem" }}>
            Tip: add this page to the Home Screen (Share, then Add to Home
            Screen) for a full-screen controller without browser bars.
          </p>
        )}
        {!window.isSecureContext && (
          <p style={{ fontSize: "0.82rem" }}>
            This page is not on HTTPS, so motion layouts are unavailable. See
            the <a href="/setup">setup guide</a>.
          </p>
        )}
      </div>
    </div>
  );
}
