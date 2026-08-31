import { useState } from "react";

type Platform = "ios" | "android";

export function SetupApp() {
  const [platform, setPlatform] = useState<Platform>(
    /iPhone|iPad/.test(navigator.userAgent) ? "ios" : "android"
  );

  return (
    <div className="setup">
      <h1>Phone HTTPS setup</h1>
      <p style={{ color: "var(--text-dim)", margin: 0 }}>
        Browsers only allow gyroscope access on secure pages. LocalPad runs a
        private certificate authority that never leaves this computer; trust
        it once on the phone and motion layouts work from then on.
      </p>

      <div className="card">
        <h2>Step 1: download the certificate</h2>
        <p style={{ color: "var(--text-dim)", marginTop: 0 }}>
          This is the public certificate only. The private key stays on the
          computer and is never served.
        </p>
        <a className="btn primary" href="/setup/localpad-ca.crt" download>
          Download localpad-ca.crt
        </a>
      </div>

      <div className="card">
        <h2>Step 2: trust it on the phone</h2>
        <div className="tabs" style={{ marginBottom: "0.9rem" }}>
          <button
            className={`btn ${platform === "ios" ? "active" : ""}`}
            onClick={() => setPlatform("ios")}
          >
            iPhone
          </button>
          <button
            className={`btn ${platform === "android" ? "active" : ""}`}
            onClick={() => setPlatform("android")}
          >
            Android
          </button>
        </div>
        {platform === "ios" ? (
          <ol>
            <li>Open the downloaded file; iOS shows "Profile Downloaded".</li>
            <li>Settings, General, VPN and Device Management, install the LocalPad profile.</li>
            <li>Settings, General, About, Certificate Trust Settings, enable full trust for "LocalPad Local CA".</li>
            <li>Return to the controller page and reload.</li>
          </ol>
        ) : (
          <ol>
            <li>Open the downloaded file, or go to Settings and search "CA certificate".</li>
            <li>Choose "Install anyway" and pick the downloaded localpad-ca.crt.</li>
            <li>Chrome trusts it right away; reload the controller page.</li>
          </ol>
        )}
      </div>

      <div className="card">
        <h2>What this certificate can and cannot do</h2>
        <ol>
          <li>It only vouches for this LocalPad server on your own network.</li>
          <li>Remove it any time from the same settings screen.</li>
          <li>If you skip this step, everything except motion still works over HTTPS after accepting the browser warning once.</li>
        </ol>
      </div>

      <p>
        <a href="/controller">Back to the controller</a>
      </p>
    </div>
  );
}
