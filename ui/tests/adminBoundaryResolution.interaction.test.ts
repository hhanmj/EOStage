import assert from "node:assert/strict";
import test from "node:test";

import { resolveAdminBoundary, type AdminBoundary } from "../src/lib/bridge.ts";
import {
  clearTestDesktopShellBridge,
  registerTestDesktopShellBridge,
} from "../src/lib/desktopShellBridge.ts";
import { createTauriDesktopShellBridge } from "../src/lib/desktopShellBridgeTauri.ts";

for (const context of [undefined, "administrative_boundary.resolve.provider_unavailable"] as const) {
  test(`selected AOI resolution preserves the typed provider failure ${context === undefined ? "without" : "with"} context`, async () => {
    const failure = Object.freeze({
      ok: false as const,
      error: "全球行政边界服务暂时不可用，请稍后重试",
      code: "BOUNDARY_PROVIDER_UNAVAILABLE",
      ...(context === undefined ? {} : { context }),
    });
    const requests: unknown[] = [];
    const bridge = createTauriDesktopShellBridge({
      invoke: async (command, args) => {
        assert.equal(command, "application_admin_boundary_resolve");
        requests.push(args);
        return failure;
      },
      getCurrentWindow: () => ({
        minimize: async () => undefined,
        toggleMaximize: async () => undefined,
        close: async () => undefined,
        innerSize: async () => ({ width: 100, height: 100 }),
        startResizeDragging: async () => undefined,
      }),
      open: async () => null,
    }, "tauri://localhost/", false);
    registerTestDesktopShellBridge(bridge);
    const candidate: AdminBoundary = {
      label: "California",
      bbox: { west: -124, east: -114, south: 32, north: 42, crs: "EPSG:4326" },
      provider: "nominatim",
      provider_object_id: "relation:123",
    };
    const candidateBefore = structuredClone(candidate);
    try {
      // ApiError has optional context/diagnostic fields. applicationError materializes
      // absent values as undefined; keep full equality, including the original code/message.
      assert.deepEqual(await resolveAdminBoundary(candidate), {
        ok: false,
        error: "全球行政边界服务暂时不可用，请稍后重试",
        code: "BOUNDARY_PROVIDER_UNAVAILABLE",
        context,
        diagnostic: undefined,
      });
      assert.deepEqual(requests, [{
        request: { provider: "nominatim", provider_object_id: "relation:123" },
      }]);
      assert.deepEqual(candidate, candidateBefore, "a failed resolve cannot replace the selected AOI");
    } finally {
      clearTestDesktopShellBridge();
    }
  });
}
