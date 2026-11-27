import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./surfaces.css";
import "./launcher/Launcher.css";
import { initTheme } from "./theme";
import { LauncherApp } from "./launcher/LauncherApp";

initTheme();

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <LauncherApp />
  </StrictMode>
);
