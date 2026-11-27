import React from "react";
import ReactDOM from "react-dom/client";
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import "leaflet/dist/leaflet.css";
import "./index.css";
import App from "./App";
import { I18nProvider } from "./i18n";
import {
  registerTauriDesktopShellBridge,
} from "./lib/desktopShellBridge.ts";
import {
  createTauriDesktopShellBridge,
  isAllowedTauriOrigin,
} from "./lib/desktopShellBridgeTauri.ts";
import {
  configureUiSmoke,
  installUiSmokeObservers,
  markUiSmoke,
  uiSmokeStartupHash,
} from "./lib/uiSmoke.ts";
import { runUiSmokeAoiVertical } from "./lib/uiSmokeDriver.ts";

const origin = typeof window === "undefined" ? "" : window.location.origin;
const development = Boolean(import.meta.env.DEV) || origin === "http://127.0.0.1:5173";
const bridge = createTauriDesktopShellBridge(undefined, origin, development);

async function runUiSmokeServiceChecks(): Promise<void> {
  const readiness = await bridge.application.getReadiness();
  if (readiness.ok) {
    markUiSmoke("application_service_ready", readiness.status === "store_ready");
  } else if (readiness.code === "V3_APPLICATION_UNAVAILABLE") {
    markUiSmoke("v3_application_unavailable_seen", true);
  }
}

if (!registerTauriDesktopShellBridge(
  bridge,
  origin,
  (candidate) => isAllowedTauriOrigin(candidate, development),
)) {
  throw new Error("Tauri desktop shell origin was not accepted");
}

async function bootstrap(): Promise<void> {
  if (typeof window !== "undefined" && (origin.startsWith("tauri://") || origin.includes("tauri.localhost"))) {
    try {
      const first = await tauriInvoke<string>("ui_smoke_phase");
      if (first === "__cdp_idle__" || first.startsWith("{")) {
        const handleRaw = (raw: string) => {
          if (!raw || !raw.startsWith("{")) return;
          const req = JSON.parse(raw) as {
            id: number;
            cmd: string;
            args?: Record<string, unknown>;
          };
          void (async () => {
            try {
              const value = await tauriInvoke(req.cmd, req.args);
              await tauriInvoke("ui_smoke_checkpoint", {
                step: `__eostage_cdp_reply__:${JSON.stringify({ id: req.id, ok: true, value: value ?? null })}`,
              });
            } catch (error) {
              const errPayload =
                error instanceof Error
                  ? { message: error.message, name: error.name }
                  : error;
              await tauriInvoke("ui_smoke_checkpoint", {
                step: `__eostage_cdp_reply__:${JSON.stringify({ id: req.id, ok: false, error: errPayload })}`,
              });
            }
          })();
        };
        handleRaw(first);
        while (true) {
          try {
            const raw = await tauriInvoke<string>("ui_smoke_phase");
            handleRaw(raw);
          } catch {
            // Keep long-polling while CDP bridge is active
          }
        }
      }
    } catch {
      // Normal desktop startup (UI_SMOKE_NOT_ENABLED) continues below.
    }
  }

  let uiSmokeEnabled = false;
  try {
    uiSmokeEnabled = await tauriInvoke<boolean>("ui_smoke_status");
  } catch {
    // Normal desktop startup keeps the internal UI smoke route inactive.
  }
  configureUiSmoke(
    uiSmokeEnabled,
    (report) => tauriInvoke("ui_smoke_report", { report }).then(() => undefined),
  );
  const removeUiSmokeObservers = installUiSmokeObservers();
  if (uiSmokeEnabled) {
    // Before the first render, so the shell resolves its initial route from
    // this hash instead of the persisted last route.
    const smokeHash = uiSmokeStartupHash(window.location.hash);
    if (smokeHash) window.history.replaceState(window.history.state, "", smokeHash);
  }

  ReactDOM.createRoot(document.getElementById("root")!).render(
    <React.StrictMode>
      <I18nProvider>
        <App />
      </I18nProvider>
    </React.StrictMode>,
  );

  if (uiSmokeEnabled) {
    window.requestAnimationFrame(() => {
      void runUiSmokeServiceChecks();
    });
    void runUiSmokeAoiVertical();
  }

  window.addEventListener("beforeunload", removeUiSmokeObservers, { once: true });
}

void bootstrap();
