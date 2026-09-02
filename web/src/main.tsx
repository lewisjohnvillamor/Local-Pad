import React from "react";
import ReactDOM from "react-dom/client";
import "@fontsource-variable/outfit";
// Latin-only subsets: the UI has no Cyrillic or Vietnamese copy, and the
// extra subsets would bloat the embedded bundle.
import "@fontsource/geist-mono/latin-400.css";
import "@fontsource/geist-mono/latin-500.css";
import "./styles.css";
import { AdminApp } from "./admin/AdminApp";
import { ControllerApp } from "./controller/ControllerApp";
import { SetupApp } from "./setup/SetupApp";

function route(): React.ReactElement {
  const path = window.location.pathname;
  if (path.startsWith("/controller")) return <ControllerApp />;
  if (path.startsWith("/setup")) return <SetupApp />;
  return <AdminApp />;
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>{route()}</React.StrictMode>
);
