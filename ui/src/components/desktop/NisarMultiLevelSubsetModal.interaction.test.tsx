import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { waitFor } from "@testing-library/react";
import { cleanup, fireEvent, render } from "../../test-utils/render-harness";
import type { SceneRow } from "../../shell/api/types";
import {
  extractNisarSubsetDownloadUrl,
  NisarMultiLevelSubsetModal,
} from "./NisarMultiLevelSubsetModal";

test.afterEach(() => cleanup());

function makeNisarScene(index: number, overrides: Partial<SceneRow> = {}): SceneRow {
  const sceneId = `NISAR_S4_ST_RSLC_2026_${String(index).padStart(2, "0")}`;
  return {
    product_key: sceneId,
    product_id: sceneId,
    scene_id: `PROVIDER::bhoonidhi::ITEM::${sceneId}`,
    provider: "ISRO Bhoonidhi",
    dataset: "nisar-s-rslc",
    file_name: `${sceneId}.h5`,
    platform: "NISAR",
    product_type: "RSLC",
    beam_mode: "ST",
    polarization: "RH+RV",
    acquisition_datetime: "2026-10-07T13:29:48Z",
    absolute_orbit: 100 + index,
    relative_orbit: 98,
    path: 98,
    frame: 70 + index,
    orbit_direction: "DESCENDING",
    file_size_remote: 2_600_000_000,
    footprint_bbox: { west: 81.5, south: 32.3, east: 83.6, north: 33.2, crs: "EPSG:4326" },
    footprint_geojson: null,
    has_url: true,
    download_url: "",
    assets: [
      {
        role: "thumbnail",
        url: `https://bhoonidhi.nrsc.gov.in/img/JPG/${sceneId}.jpg`,
        file_name: `${sceneId}.jpg`,
        media_type: "image/jpeg",
        size_bytes: null,
      },
    ],
    native_metadata: {
      bbox: [81.5, 32.3, 83.6, 33.2],
    },
    ...overrides,
  };
}

test("NisarMultiLevelSubsetModal renders all 28 selected scene names visibly without flex collapse", () => {
  const scenes = Array.from({ length: 28 }, (_, i) => makeNisarScene(i + 1));
  const view = render(
    <NisarMultiLevelSubsetModal
      productLevel="RSLC"
      band="S"
      scenes={scenes}
      aoiBbox={{ west: 81.73, south: 32.53, east: 83.46, north: 33.02, crs: "EPSG:4326" }}
      aoiLabel="地图绘制矩形"
      initialOutputRoot="/Users/demo/Downloads/EOStage"
      rslcSelections={[{ frequency: "A", polarizations: ["RH", "RV"] }]}
      onClose={() => {}}
      onSubmitted={() => {}}
    />,
  );

  const list = view.getByTestId("nisar-multilevel-scene-list");
  assert.ok(!list.className.includes("flex flex-col"), "Scroll list must not flex-shrink 28 rows to zero height");
  assert.equal(list.children.length, 28);
  assert.ok(list.textContent?.includes("NISAR_S4_ST_RSLC_2026_01.h5"));
  assert.ok(list.textContent?.includes("NISAR_S4_ST_RSLC_2026_28.h5"));
  // No broken probe button when Bhoonidhi S-Band scenes only carry .jpg quicklooks and no RSLC batch preflight callback
  assert.equal(view.queryByTestId("nisar-multilevel-preflight-btn"), null);
});

test("extractNisarSubsetDownloadUrl ignores quicklook .jpg thumbnails and accepts genuine .h5 URLs", () => {
  const thumbOnly = makeNisarScene(1);
  assert.equal(extractNisarSubsetDownloadUrl(thumbOnly), "");

  const withH5 = makeNisarScene(2, {
    assets: [
      {
        role: "thumbnail",
        url: "https://bhoonidhi.nrsc.gov.in/img/JPG/preview.jpg",
        file_name: "preview.jpg",
        media_type: "image/jpeg",
        size_bytes: null,
      },
      {
        role: "data",
        url: "https://datapool.asf.alaska.edu/NISAR/NISAR_L1_PR_RSLC.h5",
        file_name: "NISAR_L1_PR_RSLC.h5",
        media_type: "application/x-hdf5",
        size_bytes: 2_000_000_000,
      },
    ],
  });
  assert.equal(
    extractNisarSubsetDownloadUrl(withH5),
    "https://datapool.asf.alaska.edu/NISAR/NISAR_L1_PR_RSLC.h5",
  );
});

test("NisarMultiLevelSubsetModal runs RSLC V2 preflight summary and formats structured error objects without [object Object]", async () => {
  const scenes = [makeNisarScene(1), makeNisarScene(2)];
  const view = render(
    <NisarMultiLevelSubsetModal
      productLevel="RSLC"
      band="S"
      scenes={scenes}
      aoiBbox={{ west: 81.73, south: 32.53, east: 83.46, north: 33.02, crs: "EPSG:4326" }}
      aoiLabel="研究区"
      initialOutputRoot="/Users/demo/Downloads/EOStage"
      rslcSelections={[{ frequency: "A", polarizations: ["RH", "RV"] }]}
      onPreflightRslcBatch={async () => ({
        eligibleCount: 2,
        excludedCount: 0,
        commonFrequencies: ["Frequency A"],
        commonPolarizations: ["RH", "RV"],
      })}
      onSubmitRslcBatch={async () => {
        throw {
          code: "NISAR_SUBSET_REJECTED",
          error: "当前账户尚未完成 ISRO Bhoonidhi 会话登录",
          context: "请先在设置中测试连接",
        };
      }}
      onClose={() => {}}
      onSubmitted={() => {}}
    />,
  );

  const preflightBtn = view.getByTestId("nisar-multilevel-preflight-btn");
  assert.ok(preflightBtn.textContent?.includes("校验研究区与波段可用性"));
  fireEvent.click(preflightBtn);
  const summaryEl = await waitFor(() => view.getByTestId("nisar-multilevel-rslc-preflight-info"));
  assert.ok(summaryEl.textContent?.includes("2/2 景"));

  fireEvent.click(view.getByTestId("nisar-multilevel-submit-btn"));
  await waitFor(() => {
    const text = view.getByTestId("nisar-multilevel-subset-modal").textContent ?? "";
    assert.ok(text.includes("ISRO Bhoonidhi"), `Expected formatted error message, got: ${text}`);
    assert.doesNotMatch(text, /\[object Object\]/);
  });
});

test("Item 4: NisarMultiLevelSubsetModal map disables legacy Leaflet attribution, places legend bottom-left, and provides bottom-right circular i popover", () => {
  const scenes = [
    makeNisarScene(1, {
      footprint_geojson: {
        type: "Polygon",
        coordinates: [[[81.5, 32.3], [83.6, 32.3], [83.6, 33.2], [81.5, 33.2], [81.5, 32.3]]],
      },
    }),
  ];
  const view = render(
    <NisarMultiLevelSubsetModal
      productLevel="RSLC"
      band="L"
      scenes={scenes}
      aoiBbox={{ west: 81.73, south: 32.53, east: 83.46, north: 33.02, crs: "EPSG:4326" }}
      aoiLabel="研究区"
      initialOutputRoot="/data/eostage-test-output"
      onClose={() => {}}
      onSubmitted={() => {}}
    />,
  );

  // Zero legacy bottom-right Leaflet attribution bar
  assert.equal(view.container.querySelector(".leaflet-control-attribution"), null);

  // Legend pill sits at bottom-left, not bottom-right
  const legend = view.getByTestId("aoi-draw-map-legend");
  assert.ok(legend.className.includes("bottom-3"));
  assert.ok(legend.className.includes("left-3"));
  assert.equal(legend.className.includes("right-3"), false);

  // Circular i button in bottom-right toggles attribution popover
  const attrBtn = view.getByTestId("aoi-draw-map-attribution-btn");
  assert.ok(attrBtn.className.includes("rounded-full"));
  assert.equal(attrBtn.getAttribute("aria-expanded"), "false");
  assert.equal(view.queryByTestId("aoi-draw-map-attribution-popover"), null);

  fireEvent.click(attrBtn);
  assert.equal(attrBtn.getAttribute("aria-expanded"), "true");
  const popover = view.getByTestId("aoi-draw-map-attribution-popover");
  assert.ok(popover.textContent?.includes("Leaflet"));

  fireEvent.click(attrBtn);
  assert.equal(attrBtn.getAttribute("aria-expanded"), "false");
  assert.equal(view.queryByTestId("aoi-draw-map-attribution-popover"), null);
});
