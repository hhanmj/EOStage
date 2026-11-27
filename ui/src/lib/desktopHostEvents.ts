/**
 * Safe Tauri & test host event subscription helper.
 * Uses `@tauri-apps/api/event` when running inside Tauri / macos_cdp_bridge,
 * and also bridges `CustomEvent("eostage-host:" + eventName)` on `window` for JSDOM tests.
 */

import { listen as tauriListen } from "@tauri-apps/api/event";

export type DesktopHostEventName =
  | "agent:stream-event"
  | "agent:live-delta"
  | "agent:workspace-sync"
  | "workbench:aoi-changed"
  | "workbench:search-updated"
  | "workbench:file-drop";

export type WorkbenchFileDropEvent = {
  kind: "hover" | "over" | "drop" | "cancel";
  paths: string[];
  position?: { x: number; y: number } | null;
  scaleFactor?: number;
};

function hasTauriInternals(): boolean {
  if (typeof window === "undefined") return false;
  const win = window as unknown as Record<string, unknown>;
  return Boolean(win.__TAURI_INTERNALS__ && typeof (win.__TAURI_INTERNALS__ as Record<string, unknown>).invoke === "function");
}

export function subscribeDesktopHostEvent<T = Record<string, unknown>>(
  eventName: DesktopHostEventName,
  handler: (payload: T) => void
): () => void {
  if (typeof window === "undefined") {
    return () => {};
  }

  let disposed = false;
  let unlistenTauri: (() => void) | null = null;

  const customEventName = `eostage-host:${eventName}`;
  const domListener = (ev: Event) => {
    if (disposed) return;
    const custom = ev as CustomEvent<T>;
    handler(custom.detail);
  };
  window.addEventListener(customEventName, domListener);

  if (hasTauriInternals()) {
    tauriListen<T>(eventName, (event) => {
      if (disposed) return;
      handler(event.payload);
    })
      .then((fn) => {
        if (disposed) {
          try {
            void Promise.resolve(fn()).catch(() => {});
          } catch {
            // ignore unlisten failure after unmount
          }
        } else {
          unlistenTauri = fn;
        }
      })
      .catch(() => {
        // Non-fatal in partial bridge environments; streamPoll fallback remains active.
      });
  }

  return () => {
    disposed = true;
    window.removeEventListener(customEventName, domListener);
    if (unlistenTauri) {
      try {
        void Promise.resolve(unlistenTauri()).catch(() => {});
      } catch {
        // ignore unlisten failure
      }
      unlistenTauri = null;
    }
  };
}

export function emitLocalDesktopHostEvent<T = Record<string, unknown>>(
  eventName: DesktopHostEventName,
  payload: T
): void {
  if (typeof window === "undefined") return;
  const EventCtor =
    (window as unknown as { CustomEvent?: typeof CustomEvent }).CustomEvent ?? CustomEvent;
  window.dispatchEvent(new EventCtor(`eostage-host:${eventName}`, { detail: payload }));
}
