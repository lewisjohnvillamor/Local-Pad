import React from "react";
import ReactDOM from "react-dom/client";
import "@fontsource-variable/outfit";
import "@fontsource/geist-mono/400.css";
import "@fontsource/geist-mono/500.css";
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
