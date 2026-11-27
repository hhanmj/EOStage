import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { cleanup, fireEvent, render } from "../../test-utils/render-harness";
import { CatalogResultRow, ProductName } from "./CatalogResultRow";
import {
  canonicalAgentDatasetShortName,
  detectNisarSceneReleaseTier,
} from "../../shell/api/agentSessionWorkspace";
import type { SceneRow } from "../../lib/bridge";

const scene: SceneRow = {
  scene_id: "S1A_IW_SLC__1SDV_20240408T100132_20240408T100159_053341_067676_8A12",
  file_name: "S1A_IW_SLC__1SDV_20240408T100132_20240408T100159_053341_067676_8A12.SAFE",
  platform: "Sentinel-1A",
  product_type: "SLC",
  beam_mode: "IW",
  polarization: "VV+VH",
  acquisition_datetime: "2024-04-08T10:01:32Z",
  absolute_orbit: 53341,
  relative_orbit: 69,
  path: 69,
  frame: 450,
  orbit_direction: "ascending",
  file_size_remote: 1_500_000_000,
  has_url: true,
};

test.afterEach(() => cleanup());

test("full product name inserts break opportunities after underscores without truncating", () => {
  const view = render(<ProductName value={scene.file_name!} />);
  assert.equal(view.container.textContent, scene.file_name);
  assert.equal(view.container.querySelectorAll("wbr").length, scene.file_name!.split("_").length - 1);
});

test("dot-delimited HLS product name inserts break opportunities after dots and wraps safely", () => {
  const hlsId = "HLS.S30.T51SUA.2025165T024529.v2.0";
  const view = render(<ProductName value={hlsId} />);
  assert.equal(view.container.textContent, hlsId);
  assert.equal(view.container.querySelectorAll("wbr").length, hlsId.split(".").length - 1);
  cleanup();

  const hlsScene: SceneRow = {
    ...scene,
    scene_id: hlsId,
    file_name: hlsId,
    platform: "Sentinel-2B",
    product_type: "HLS_S30",
  };
  const rowView = render(<CatalogResultRow scene={hlsScene} family="optical" />);
  const nameEl = rowView.getByTestId("product-name");
  assert.equal(nameEl.textContent, hlsId);
  assert.equal(nameEl.className.includes("truncate"), false);
  assert.equal(nameEl.className.includes("[overflow-wrap:anywhere]"), true);
});

test("row hover calls the map highlight callback and preserves the download action", () => {
  let hovered: string | null = null;
  let downloaded: string | null = null;
  const view = render(<CatalogResultRow
    scene={scene}
    family="sar"
    onHoverChange={(id) => { hovered = id; }}
    onRowAction={(id, action) => { if (action === "download") downloaded = id; }}
  />);
  const row = view.container.querySelector("[data-scene-id]")!;
  fireEvent.mouseEnter(row);
  assert.equal(hovered, scene.scene_id);
  fireEvent.mouseLeave(row);
  assert.equal(hovered, null);
  assert.equal(view.getByTestId("product-name").textContent, scene.file_name);
  assert.equal(view.getByTestId("product-name").className.includes("truncate"), false);
  assert.ok(view.getByText("Path", { exact: false }).textContent?.includes("69"));
  assert.ok(view.getByText("Frame", { exact: false }).textContent?.includes("450"));
  assert.ok(view.getByText("VV+VH"));
  fireEvent.click(view.getByRole("button", { name: "下载" }));
  assert.equal(downloaded, scene.scene_id);
});

test("missing SAR metadata is omitted rather than fabricated", () => {
  const sparse: SceneRow = {
    ...scene,
    relative_orbit: null,
    path: null,
    frame: null,
    polarization: "",
    orbit_direction: "",
    file_size_remote: null,
  };
  const view = render(<CatalogResultRow scene={sparse} family="sar" />);
  assert.equal(view.queryByText("Path", { exact: false }), null);
  assert.equal(view.queryByText("Frame", { exact: false }), null);
  assert.equal(view.queryByText("--"), null);
});

test("row info button shows hover summary popover and copies full metadata without hijacking selection", async () => {
  let selectedId: string | null = null;
  let copiedText = "";
  const originalClipboard = navigator.clipboard;
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: {
      writeText: async (txt: string) => {
        copiedText = txt;
      },
    },
  });

  try {
    const view = render(
      <CatalogResultRow
        scene={{
          ...scene,
          footprint_bbox: { west: 117.1, south: 31.6, east: 117.9, north: 32.2, crs: "EPSG:4326" },
        }}
        family="sar"
        onSelectScene={(id) => { selectedId = id; }}
      />,
    );
    const infoBtn = view.getByTestId("row-info-copy");
    assert.equal(view.queryByTestId("row-pin-swipe-left"), null);
    assert.equal(view.queryByTestId("row-pin-swipe-right"), null);

    fireEvent.mouseEnter(infoBtn);
    const popover = view.getByTestId("row-info-popover");
    assert.ok(popover.textContent?.includes(scene.scene_id));
    assert.ok(popover.textContent?.includes("WGS84"));
    assert.equal(popover.textContent?.includes("同幅覆盖区"), false);

    fireEvent.click(infoBtn);
    await new Promise((r) => setTimeout(r, 10));
    assert.ok(copiedText.includes(scene.scene_id));
    assert.ok(copiedText.includes("Product ID:"));
    assert.ok(copiedText.includes("Track: 69"));
    assert.ok(copiedText.includes("Frame: 450"));
    assert.ok(copiedText.includes("Acquisition UTC:"));
    assert.ok(copiedText.includes("BBox (WGS84):"));
    assert.equal(copiedText.includes("PROVIDER::"), false);
    assert.equal(copiedText.includes("同幅覆盖区"), false);
    assert.equal(selectedId, null);

    fireEvent.mouseLeave(infoBtn);
    assert.equal(view.queryByTestId("row-info-popover") === null, true);
  } finally {
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: originalClipboard,
    });
  }
});

test("optical eligible row renders AOI clip button and forwards scene_id", () => {
  let clippedId: string | null = null;
  const opticalScene: SceneRow = {
    ...scene,
    scene_id: "S2A_OPT_ROW_1",
    file_name: "S2A_MSIL2A_20260101T030000_N0511_R032_T50RKU_20260101T070000.SAFE",
    platform: "Sentinel-2A",
    product_type: "S2MSI2A",
    provider: "microsoft.planetary-computer",
    native_metadata: { logical_dataset: "optical.sentinel-2.l2a" },
  };
  const view = render(
    <CatalogResultRow
      scene={opticalScene}
      family="optical"
      canOpticalAoiClip
      onOpticalAoiClip={(id) => {
        clippedId = id;
      }}
    />,
  );
  const nameEl = view.getByTestId("product-name");
  assert.equal(nameEl.className.includes("min-w-[180px]"), true);
  assert.equal(nameEl.className.includes("truncate"), false);
  const clipBtn = view.getByTestId("btn-optical-aoi-clip-row");
  fireEvent.click(clipBtn);
  assert.equal(clippedId, "S2A_OPT_ROW_1");
});

test("uppercase DESCENDING and ASCENDING orbit_direction render localized descending/ascending badges", () => {
  const descView = render(
    <CatalogResultRow scene={{ ...scene, orbit_direction: "DESCENDING" }} family="sar" />,
  );
  assert.ok(descView.getByText(/降轨|Descending/));
  cleanup();

  const ascView = render(
    <CatalogResultRow scene={{ ...scene, orbit_direction: "ASCENDING" }} family="sar" />,
  );
  assert.ok(ascView.getByText(/升轨|Ascending/));
});

test("row preview toggle button fires onTogglePinPreview for eligible previews and hides when unavailable", () => {
  let toggledId: string | null = null;
  let selectedId: string | null = null;
  const view = render(
    <CatalogResultRow
      scene={{
        ...scene,
        footprint_bbox: { west: 80, south: 31, east: 83, north: 34, crs: "EPSG:4326" },
        assets: [{ role: "thumbnail", url: "https://bhoonidhi.nrsc.gov.in/thumb.jpg", file_name: "thumb.jpg", media_type: "image/jpeg", size_bytes: null }],
      }}
      family="sar"
      isPreviewPinned={false}
      onSelectScene={(id) => { selectedId = id; }}
      onTogglePinPreview={(sc) => { toggledId = sc.scene_id; }}
    />,
  );
  const btn = view.getByTestId("row-preview-toggle");
  assert.equal(btn.getAttribute("aria-pressed"), "false");
  fireEvent.click(btn);
  assert.equal(toggledId, scene.scene_id);
  assert.equal(selectedId, null);
  cleanup();

  // No thumbnail/browse/tile authority -> preview toggle must not appear
  const noPreviewView = render(
    <CatalogResultRow
      scene={{
        ...scene,
        footprint_bbox: { west: 80, south: 31, east: 83, north: 34, crs: "EPSG:4326" },
        assets: [],
      }}
      family="sar"
      batchId="b-no-preview"
      onTogglePinPreview={() => {}}
    />,
  );
  assert.equal(noPreviewView.queryByTestId("row-preview-toggle") === null, true);
  cleanup();

  // Blocked external host -> preview toggle must not appear
  const blockedView = render(
    <CatalogResultRow
      scene={{
        ...scene,
        footprint_bbox: { west: 80, south: 31, east: 83, north: 34, crs: "EPSG:4326" },
        assets: [{ role: "thumbnail", url: "https://example.invalid/thumb.jpg", file_name: "thumb.jpg", media_type: "image/jpeg", size_bytes: null }],
      }}
      family="sar"
      onTogglePinPreview={() => {}}
    />,
  );
  assert.equal(blockedView.queryByTestId("row-preview-toggle") === null, true);
});

test("Item 1: NISAR Validated granule with _L1_PR_ name renders official Validated badge, never Provisional or V1", () => {
  const nisarValidated: SceneRow = {
    ...scene,
    scene_id: "NISAR_L1_PR_RSLC_001_055_A_001_20261101",
    file_name: "NISAR_L1_PR_RSLC_001_055_A_001_20261101.h5",
    platform: "NISAR",
    product_type: "RSLC",
    native_metadata: {
      release_tier: "validated",
      collection_concept_id: "C3622236985-ASF",
      collection_short_name: "NISAR_L1_RSLC_V1",
    },
  };
  const validView = render(<CatalogResultRow scene={nisarValidated} family="sar" />);
  const validBadge = validView.getByTestId("row-nisar-tier-badge");
  assert.equal(validBadge.textContent?.trim(), "Validated");
  assert.equal(validBadge.textContent?.includes("V1"), false);
  assert.equal(detectNisarSceneReleaseTier(nisarValidated), "validated");
  assert.equal(
    canonicalAgentDatasetShortName("science.nisar.l.rslc.v1", [nisarValidated]),
    "NISAR RSLC (Validated)",
  );
  cleanup();

  // Validated by collection_concept_id alone even when granule contains _L2_PR_
  const nisarValidatedByCollection: SceneRow = {
    ...nisarValidated,
    scene_id: "NISAR_L2_PR_GSLC_001_055_A_001_20261101",
    file_name: "NISAR_L2_PR_GSLC_001_055_A_001_20261101.h5",
    product_type: "GSLC",
    native_metadata: {
      collection_concept_id: "C3622244601-ASF",
      collection_short_name: "NISAR_L2_GSLC_V1",
    },
  };
  assert.equal(detectNisarSceneReleaseTier(nisarValidatedByCollection), "validated");

  const nisarProvisional: SceneRow = {
    ...nisarValidated,
    scene_id: "NISAR_L1_PR_RSLC_002_055_A_001_20261101",
    native_metadata: {
      release_tier: "provisional",
      collection_concept_id: "C2853145197-ASF",
      collection_short_name: "NISAR_L1_RSLC_PROVISIONAL_V1",
    },
  };
  const provView = render(<CatalogResultRow scene={nisarProvisional} family="sar" />);
  assert.equal(provView.getByTestId("row-nisar-tier-badge").textContent?.trim(), "Provisional");
  assert.equal(detectNisarSceneReleaseTier(nisarProvisional), "provisional");
  assert.equal(
    canonicalAgentDatasetShortName("science.nisar.l.rslc.v1", [nisarProvisional]),
    "NISAR RSLC (Provisional)",
  );
});

test("Item 10: failed row displays real sanitized failure cause and guidance instead of bare DL005 box", () => {
  const view = render(
    <CatalogResultRow
      scene={scene}
      family="sar"
      rowState={{
        item_id: scene.scene_id,
        task_id: "t-fail-1",
        task_status: "failed",
        item_status: "failed",
        actions: [],
      }}
      task={{
        task_id: "t-fail-1",
        revision: 2,
        category: "scene_download",
        name: "Sentinel-1",
        status: "failed",
        items_count: 1,
        failure: {
          codes: ["DL005"],
          message_zh: "远端服务器返回 HTTP 503（服务暂时不可用），重试后仍未恢复。",
          failed_items: 1,
          total_items: 1,
        },
      }}
      isFailureExpanded
    />,
  );

  const causeEl = view.getByTestId("row-failure-cause");
  assert.ok(causeEl.textContent?.includes("HTTP 503"));
  const guidanceEl = view.getByTestId("row-failure-guidance");
  assert.ok(guidanceEl.textContent?.includes("建议操作"));
  assert.ok(guidanceEl.textContent?.includes("繁忙或触发限流"));
  const diagEl = view.getByTestId("row-failure-diagnostic-ref");
  assert.ok(diagEl.textContent?.includes("诊断参考：DL005"));
  assert.equal(view.queryByText("错误码:") === null, true);
});

test("Bhoonidhi NISAR S-SAR row displays authoritative Product ID, Track/Frame/DH/RSLC, and copies clean English metadata", async () => {
  let copiedText = "";
  const originalClipboard = navigator.clipboard;
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: {
      writeText: async (txt: string) => {
        copiedText = txt;
      },
    },
  });

  const itemId = "NISAR_S1_UR_RSLC_035_026_A_018_3700_DHNA_A_20261106T234845_20261106T234921_P00500_M_F_I_001";
  const bhoonidhiRow: SceneRow = {
    scene_id: `PROVIDER::isro.nrsc.bhoonidhi::COLLECTION::NISAR_SSAR_RSLC::ITEM::${itemId}`,
    product_key: `PROVIDER::isro.nrsc.bhoonidhi::COLLECTION::NISAR_SSAR_RSLC::ITEM::${itemId}`,
    product_id: `PROVIDER::isro.nrsc.bhoonidhi::COLLECTION::NISAR_SSAR_RSLC::ITEM::${itemId}`,
    file_name: "2026310234512_26_18",
    provider: "isro.nrsc.bhoonidhi",
    dataset: "sar.nisar.s.ssar.rslc.v1",
    platform: "NISAR",
    instrument: "SSAR",
    processing_level: "RSLC",
    product_type: "RSLC",
    beam_mode: "W",
    polarization: "DH",
    orbit_direction: "ascending",
    relative_orbit: 26,
    path: 26,
    frame: 18,
    absolute_orbit: 6944,
    acquisition_datetime: "2026-11-06T23:48:45Z",
    footprint_bbox: { west: 79.897268, south: 31.275294, east: 83.177631, north: 34.071074, crs: "EPSG:4326" },
    has_url: false,
    sar_product: {
      product_key: `PROVIDER::isro.nrsc.bhoonidhi::COLLECTION::NISAR_SSAR_RSLC::ITEM::${itemId}`,
      provider_product_key: `PROVIDER::isro.nrsc.bhoonidhi::COLLECTION::NISAR_SSAR_RSLC::ITEM::${itemId}`,
      provider_product_id: itemId,
      canonical_product_key: null,
      identity_rule_id: "isro.nrsc.bhoonidhi.nisar.s.ssar.rslc.identity.v1",
      product_id: itemId,
      scene_id: "2026310234512_26_18",
      provider: "isro.nrsc.bhoonidhi",
      dataset: "sar.nisar.s.ssar.rslc.v1",
      platform: "NISAR",
      instrument: "SSAR",
      processing_level: "RSLC",
      product_family: "NISAR_SSAR_RSLC",
      beam_mode: "W",
      beam_swath: null,
      polarization: "DH",
      flight_direction: "ASCENDING",
      look_direction: "LEFT",
      relative_orbit: 26,
      absolute_orbit: 6944,
      frame: 18,
      asf_frame: null,
      path: 26,
      acquisition_start: "2026-11-06T23:48:45Z",
      acquisition_stop: "2026-11-06T23:49:21Z",
      footprint_geojson: null,
      footprint_bbox: { west: 79.897268, south: 31.275294, east: 83.177631, north: 34.071074, crs: "EPSG:4326" },
      download_url: null,
      file_name: `${itemId}.h5`,
      file_size_remote: null,
      checksum: null,
      local_path: null,
      native_metadata: {
        logical_dataset: "sar.nisar.s.ssar.rslc.v1",
        properties: {
          Track: 26,
          Frame: 18,
          Polarization: "DH",
          Mode: "W",
          Scene_Identifier: "2026310234512_26_18",
          Imaging_Orbit: 6944,
        },
      },
      burst: null,
      assets: [],
    },
  };

  try {
    const view = render(<CatalogResultRow scene={bhoonidhiRow} family="sar" />);
    assert.equal(view.getByTestId("product-name").textContent, itemId);
    assert.ok(view.getByText("2026-11-06 23:48 UTC"));
    assert.ok(view.getByText("Track", { exact: false }).textContent?.includes("26"));
    assert.ok(view.getByText("Frame", { exact: false }).textContent?.includes("18"));
    assert.ok(view.getByText("DH"));
    assert.ok(view.getByText("RSLC"));

    const infoBtn = view.getByTestId("row-info-copy");
    fireEvent.mouseEnter(infoBtn);
    const popover = view.getByTestId("row-info-popover");
    assert.ok(popover.textContent?.includes("产品 ID"));
    assert.ok(popover.textContent?.includes("2026310234512_26_18"));
    assert.equal(popover.textContent?.includes("同幅覆盖区"), false);

    fireEvent.click(infoBtn);
    await new Promise((r) => setTimeout(r, 10));
    assert.match(copiedText, new RegExp(`^Product ID: ${itemId}$`, "m"));
    assert.match(copiedText, /^Scene ID: 2026310234512_26_18$/m);
    assert.match(copiedText, /^Source: NISAR S-SAR RSLC \(ISRO Bhoonidhi\)$/m);
    assert.match(copiedText, /^Track: 26$/m);
    assert.match(copiedText, /^Frame: 18$/m);
    assert.match(copiedText, /^Polarization: DH$/m);
    assert.match(copiedText, /^Acquisition UTC: 2026-11-06 23:48:45 ~ 23:49:21 UTC$/m);
    assert.equal(copiedText.includes("PROVIDER::"), false);
    assert.equal(copiedText.includes("同幅覆盖区"), false);
  } finally {
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: originalClipboard });
  }
});

test("non-SAR rows (GEE and earthquake) classify cleanly even when family defaults to sar", async () => {
  const geeRow: SceneRow = {
    scene_id: "gee-composite::COPERNICUS/S2_SR_HARMONIZED",
    product_key: "gee-composite::COPERNICUS/S2_SR_HARMONIZED",
    product_id: "gee-composite::COPERNICUS/S2_SR_HARMONIZED",
    dataset: "science.gee.catalog.v1",
    provider: "Google Earth Engine",
    file_name: "Sentinel-2 L2A · AOI 云端合成 (8 景)",
    platform: "COPERNICUS/S2_SR_HARMONIZED",
    product_type: "GEE 合成 (10m)",
    beam_mode: "",
    polarization: "",
    acquisition_datetime: "2026-11-20",
    absolute_orbit: null,
    relative_orbit: null,
    path: null,
    frame: null,
    orbit_direction: "",
    has_url: true,
    native_metadata: {
      gee_asset_id: "COPERNICUS/S2_SR_HARMONIZED",
      is_gee_composite: true,
      native_scale_m: 10,
      matched_scenes: 8,
      total_scenes: 12,
    },
  };
  const view = render(<CatalogResultRow scene={geeRow} family="sar" />);
  assert.ok(view.getByText("有效 8/12 景"));
  assert.equal(view.queryByText(/升轨|降轨/) === null, true);
  assert.equal(view.queryByText("-") === null, true);
  cleanup();

  const eqRow: SceneRow = {
    scene_id: "ci40001234",
    product_key: "ci40001234",
    product_id: "ci40001234",
    dataset: "event.usgs.earthquake.v1",
    provider: "USGS",
    file_name: "M 4.2 - 12km SE of Los Angeles, CA",
    platform: "USGS Earthquake",
    product_type: "Mw 4.2",
    beam_mode: "",
    polarization: "",
    acquisition_datetime: "2026-11-10T08:15:30Z",
    absolute_orbit: null,
    relative_orbit: null,
    path: null,
    frame: null,
    orbit_direction: "",
    has_url: false,
    native_metadata: { magnitude: 4.2, depth_km: 11.4, place: "12km SE of Los Angeles, CA" },
  };
  const eqView = render(<CatalogResultRow scene={eqRow} family="sar" />);
  assert.ok(eqView.getByText("2026-11-10 08:15 UTC"));
  assert.ok(eqView.getByText("Mw 4.2"));
  assert.equal(eqView.queryByText(/升轨|降轨/) === null, true);
});

test("optical row ignores fallback sar/undefined family and omits fake Unknown/Path/Frame while keeping cloud cover", () => {
  const s2Scene: SceneRow = {
    scene_id: "PROVIDER::copernicus.dataspace.odata::COLLECTION::SENTINEL-2::ITEM::S2B_TEST_CLOUD",
    product_key: "PROVIDER::copernicus.dataspace.odata::COLLECTION::SENTINEL-2::ITEM::S2B_TEST_CLOUD",
    product_id: "S2B_MSIL2A_20261124T030031_N0513_R032_T50SNB_20261124T062550.SAFE",
    provider: "copernicus.dataspace.odata",
    dataset: "optical.sentinel-2.l2a",
    file_name: "S2B_MSIL2A_20261124T030031_N0513_R032_T50SNB_20261124T062550.SAFE",
    platform: "Sentinel-2B",
    product_type: "L2A",
    processing_level: "L2A",
    beam_mode: "",
    polarization: "",
    acquisition_datetime: "2026-11-24T03:00:31Z",
    absolute_orbit: null,
    relative_orbit: null,
    path: null,
    frame: null,
    orbit_direction: "",
    has_url: false,
    native_metadata: {
      logical_dataset: "optical.sentinel-2.l2a",
      properties: { cloud_cover: 18.5 },
    },
  };

  for (const passedFamily of [undefined, "sar"] as const) {
    const view = render(<CatalogResultRow scene={s2Scene} family={passedFamily} />);
    assert.ok(view.getByText(/云量\s*18\.5%/));
    assert.equal(view.queryByText(/Unknown/) === null, true);
    assert.equal(view.queryByText(/Path\s*未知/) === null, true);
    assert.equal(view.queryByText(/Frame\s*未知/) === null, true);
    cleanup();
  }
});

test("clicking GEE preview invokes onPreviewScene exclusively without triggering onTogglePinPreview", () => {
  const geeScene: SceneRow = {
    scene_id: "gee-scene::COPERNICUS/S2_SR_HARMONIZED/20261120",
    product_key: "gee-scene::COPERNICUS/S2_SR_HARMONIZED/20261120",
    product_id: "gee-scene::COPERNICUS/S2_SR_HARMONIZED/20261120",
    dataset: "science.gee.catalog.v1",
    provider: "Google Earth Engine",
    file_name: "S2_SR_20261120",
    platform: "COPERNICUS/S2_SR_HARMONIZED",
    product_type: "GEE 景",
    beam_mode: "",
    polarization: "",
    acquisition_datetime: "2026-11-20T03:00:00Z",
    absolute_orbit: null,
    relative_orbit: null,
    path: null,
    frame: null,
    orbit_direction: "",
    has_url: true,
    native_metadata: { gee_asset_id: "COPERNICUS/S2_SR_HARMONIZED" },
  };

  let previewCalls = 0;
  let pinCalls = 0;
  const view = render(
    <CatalogResultRow
      scene={geeScene}
      family="optical"
      onPreviewScene={() => { previewCalls += 1; }}
      onTogglePinPreview={() => { pinCalls += 1; }}
    />,
  );
  fireEvent.click(view.getByTestId("row-gee-preview"));
  assert.equal(previewCalls, 1);
  assert.equal(pinCalls, 0);
});
