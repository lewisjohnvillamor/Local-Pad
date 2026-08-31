import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { ControllerSession } from "./session";
import { LayoutView } from "./LayoutView";

export function ControllerApp() {
  const session = useMemo(() => new ControllerSession(), []);
  const view = useSyncExternalStore(session.subscribe, session.getView);
  const [isPortrait, setIsPortrait] = useState(
    window.innerHeight >= window.innerWidth
  );

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
    <div className="controller-shell">
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
        {view.motionOn && (
          <button className="btn" onClick={() => session.recenter()}>
            Recenter
          </button>
        )}
        <button className="btn" onClick={() => session.releaseEverything()}>
          Release
        </button>
      </div>
      <div className="play-area control-surface">
        <LayoutView layout={layout} session={session} motionOn={view.motionOn} />
      </div>

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

  const statusLine: Record<string, string> = {
    "needs-pairing": "not paired",
    pairing: "pairing...",
    connecting: "connecting...",
    "waiting-approval": "waiting for approval on the computer",
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
              onClick={() => void session.pair(code.trim())}
            >
              Connect
            </button>
          </>
        )}
        <div className="connect-status">{statusLine[phase] ?? phase}</div>
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
