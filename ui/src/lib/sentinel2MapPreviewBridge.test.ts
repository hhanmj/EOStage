import assert from "node:assert/strict";
import test from "node:test";

import {
  createTauriDesktopShellBridge,
  type TauriBridgeDependencies,
} from "./desktopShellBridgeTauri.ts";

function dependencies(
  response: unknown,
  calls: Array<[string, Record<string, unknown> | undefined]>,
): TauriBridgeDependencies {
  return {
    invoke: async (command, args) => {
      calls.push([command, args]);
      return response;
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  };
}

test("Tauri Sentinel-2 map Preview returns only frozen product identity and exact MPC Item ID", async () => {
  const calls: Array<[string, Record<string, unknown> | undefined]> = [];
  const request = {
    request_id: "s2-map-preview-1",
    batch_id: "s2-batch",
    item_id: "PROVIDER::copernicus.dataspace.odata::COLLECTION::SENTINEL-2::ITEM::165ba4c9-997c-4cea-970f-33ab9bdb8084",
  };
  const response = {
    product_key: request.item_id,
    map_item_id: "S2C_MSIL2A_20260726T125321_R138_T27VXL_20260726T221014",
  };
  const bridge = createTauriDesktopShellBridge(
    dependencies(response, calls),
    "tauri://localhost/",
    false,
  );

  assert.deepEqual(await bridge.application.sentinel2MapPreview(request), {
    ok: true,
    preview: response,
  });
  assert.deepEqual(calls, [[
    "application_sentinel2_map_preview",
    { request },
  ]]);
});

test("Tauri Sentinel-2 map Preview rejects locators and malformed Item IDs", async () => {
  const request = {
    request_id: "s2-map-preview-2",
    batch_id: "s2-batch",
    item_id: "frozen-item",
  };
  for (const response of [
    {
      product_key: "frozen-item",
      map_item_id: "S2C_MSIL2A_20260726T125321_R138_T27VXL_20260726T221014",
      href: "https://must-not-cross.example/tile",
    },
    {
      product_key: "frozen-item",
      map_item_id: "../../forged",
    },
  ]) {
    const bridge = createTauriDesktopShellBridge(
      dependencies(response, []),
      "tauri://localhost/",
      false,
    );
    assert.equal((await bridge.application.sentinel2MapPreview(request)).ok, false);
  }
});
