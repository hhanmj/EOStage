import assert from "node:assert/strict";
import test from "node:test";

import { JSDOM } from "jsdom";
import React from "react";

import { createDownloadAcquisitionDraft } from "../lib/downloadDestinationConfirmation";
import { DownloadDestinationConfirmation } from "./DownloadDestinationConfirmation";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://127.0.0.1/",
});
Object.assign(globalThis, {
  window: dom.window,
  document: dom.window.document,
  HTMLElement: dom.window.HTMLElement,
  Node: dom.window.Node,
  MutationObserver: dom.window.MutationObserver,
  getComputedStyle: dom.window.getComputedStyle,
  IS_REACT_ACT_ENVIRONMENT: true,
});
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: dom.window.navigator,
});

const { act, cleanup, fireEvent, render, screen, waitFor } = await import("@testing-library/react");
test.afterEach(() => cleanup());

function draft() {
  return createDownloadAcquisitionDraft({
    sourceBatchId: "batch-confirmation",
    sourceBatchRevision: 6,
    selectionScope: "checked",
    selectedProductIds: ["PRODUCT-A", "PRODUCT-B"],
    selectedProductCount: 2,
    acquisition: { kind: "sentinel1", frozenParameters: {} },
    destinationPrefill: "F:/downloads/default",
    layout: "sentinel1_slc",
    taskPolicy: "manual",
    concurrency: 2,
  });
}

test("default destination is reviewable and cancel performs no confirmation", async () => {
  const confirmed: unknown[] = [];
  let cancelled = 0;
  render(
    <DownloadDestinationConfirmation
      draft={draft()}
      chooseDirectory={async () => null}
      onCancel={() => { cancelled += 1; }}
      onConfirm={async (value) => { confirmed.push(value); }}
    />,
  );

  assert.ok(screen.getByRole("dialog", { name: "确认下载任务" }));
  assert.equal(screen.getByLabelText("目的目录").getAttribute("value"), "F:/downloads/default");
  assert.equal(confirmed.length, 0);

  fireEvent.click(screen.getByRole("button", { name: "取消" }));
  assert.equal(cancelled, 1);
  assert.equal(confirmed.length, 0);
});

test("directory choice and explicit confirmation produce exactly one frozen draft", async () => {
  const confirmed: unknown[] = [];
  let release: (() => void) | null = null;
  render(
    <DownloadDestinationConfirmation
      draft={draft()}
      chooseDirectory={async () => "F:/downloads/confirmed"}
      onCancel={() => undefined}
      onConfirm={(value) => new Promise<void>((resolve) => {
        confirmed.push(value);
        release = resolve;
      })}
    />,
  );

  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "选择目录" }));
  });
  assert.equal(screen.getByLabelText("目的目录").getAttribute("value"), "F:/downloads/confirmed");

  const create = screen.getByRole("button", { name: "创建下载任务" });
  fireEvent.click(create);
  await waitFor(() => {
    assert.equal(confirmed.length, 1);
    assert.equal(create.hasAttribute("disabled"), true);
  });
  fireEvent.click(create);
  assert.equal(confirmed.length, 1);

  assert.deepEqual(confirmed[0], {
    sourceBatchId: "batch-confirmation",
    sourceBatchRevision: 6,
    selectionScope: "checked",
    selectedProductIds: ["PRODUCT-A", "PRODUCT-B"],
    selectedProductCount: 2,
    acquisition: { kind: "sentinel1", frozenParameters: {} },
    outputRoot: "F:/downloads/confirmed",
    layout: "sentinel1_slc",
    taskPolicy: "manual",
    concurrency: 2,
  });
  assert.equal(Object.isFrozen(confirmed[0]), true);

  await act(async () => {
    release?.();
  });
  assert.equal(screen.queryByRole("dialog", { name: "确认下载任务" }), null);
});

test("submission failure keeps the same frozen acquisition visible for retry", async () => {
  let attempts = 0;
  render(
    <DownloadDestinationConfirmation
      draft={draft()}
      chooseDirectory={async () => null}
      onCancel={() => undefined}
      onConfirm={async () => {
        attempts += 1;
        throw new Error("Store unavailable");
      }}
    />,
  );

  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "创建下载任务" }));
  });
  assert.ok(screen.getByText("Store unavailable"));
  assert.ok(screen.getByText("2 个产品"));
  assert.equal(screen.getByLabelText("目的目录").getAttribute("value"), "F:/downloads/default");
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "创建下载任务" }));
  });
  assert.equal(attempts, 2);
});

function sentinel2Draft(safeArchiveAvailable = true) {
  return createDownloadAcquisitionDraft({
    sourceBatchId: "sentinel2-batch",
    sourceBatchRevision: 3,
    selectionScope: "exact_item",
    selectedProductIds: ["S2-A"],
    selectedProductCount: 1,
    acquisition: {
      kind: "sentinel2",
      frozenParameters: {
        delivery_form: "safe_archive",
        safe_archive_available: safeArchiveAvailable,
        analysis_bands_available: true,
        band_preset: "true_color",
        selected_band_keys: ["B02", "B03", "B04"],
      },
    },
    destinationPrefill: "F:/downloads/sentinel2",
    layout: "direct",
    taskPolicy: "manual",
    concurrency: 4,
  });
}

test("Sentinel-2 explicitly separates complete SAFE from analysis COG delivery", async () => {
  const confirmed: any[] = [];
  render(
    <DownloadDestinationConfirmation
      draft={sentinel2Draft()}
      chooseDirectory={async () => null}
      onCancel={() => undefined}
      onConfirm={async (value) => { confirmed.push(value); }}
    />,
  );

  const safe = screen.getByRole("radio", { name: /完整 SAFE 产品/ }) as HTMLInputElement;
  const analysis = screen.getByRole("radio", { name: /分析波段/ }) as HTMLInputElement;
  assert.equal(safe.checked, true);
  assert.equal(analysis.checked, false);

  fireEvent.click(analysis);
  assert.ok(screen.getByText(/分析波段将保存为独立 COG 文件集/));
  const trueColor = screen.getByRole("radio", { name: /真彩色/ }) as HTMLInputElement;
  assert.equal(trueColor.checked, true);

  fireEvent.click(screen.getByRole("radio", { name: /植被/ }));
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "创建下载任务" }));
  });
  assert.equal(confirmed.length, 1);
  assert.equal(confirmed[0].acquisition.frozenParameters.delivery_form, "analysis_bands");
  assert.equal(confirmed[0].acquisition.frozenParameters.band_preset, "vegetation");
  assert.deepEqual(confirmed[0].acquisition.frozenParameters.selected_band_keys, ["B04", "B08"]);
});

test("Sentinel-2 keeps SAFE unavailable instead of silently substituting it", async () => {
  const confirmed: any[] = [];
  render(
    <DownloadDestinationConfirmation
      draft={sentinel2Draft(false)}
      chooseDirectory={async () => null}
      onCancel={() => undefined}
      onConfirm={async (value) => { confirmed.push(value); }}
    />,
  );

  const safe = screen.getByRole("radio", { name: /完整 SAFE 产品/ }) as HTMLInputElement;
  const analysis = screen.getByRole("radio", { name: /分析波段/ }) as HTMLInputElement;
  assert.equal(safe.disabled, true);
  assert.equal(safe.checked, true);
  assert.equal(analysis.checked, false);
  assert.ok(screen.getByText(/登录 CDSE 后可用/));
  assert.equal((screen.getByRole("button", { name: "创建下载任务" }) as HTMLButtonElement).disabled, true);

  fireEvent.click(analysis);
  assert.equal(analysis.checked, true);
  fireEvent.click(screen.getByRole("radio", { name: /自定义波段/ }));
  fireEvent.click(screen.getByRole("checkbox", { name: /B02 蓝光/ }));
  fireEvent.click(screen.getByRole("checkbox", { name: /B03 绿光/ }));
  fireEvent.click(screen.getByRole("checkbox", { name: /B04 红光/ }));
  assert.equal((screen.getByRole("button", { name: "创建下载任务" }) as HTMLButtonElement).disabled, true);
  assert.equal(confirmed.length, 0);
});

function orbitDraft() {
  return createDownloadAcquisitionDraft({
    ...draft(),
    acquisition: { kind: "sentinel1", frozenParameters: { allow_companion_orbit: true } },
  });
}

test("companion Orbit is optional and follows the confirmed SLC directory", async () => {
  const confirmed: any[] = [];
  render(<DownloadDestinationConfirmation draft={orbitDraft()}
    chooseDirectory={async () => "F:/chosen/slc"} onCancel={() => undefined}
    onConfirm={async (value) => { confirmed.push(value); }} />);
  const checkbox = screen.getByRole("checkbox", { name: /同时下载精密轨道/ }) as HTMLInputElement;
  assert.equal(checkbox.checked, false);
  fireEvent.click(checkbox);
  await act(async () => { fireEvent.click(screen.getByRole("button", { name: "选择目录" })); });
  assert.ok(screen.getByText("SARscape 目录：F:/chosen/slc/Sentinel_Orbit/AUX_POEORB"));
  await act(async () => { fireEvent.click(screen.getByRole("button", { name: "创建下载任务" })); });
  assert.equal(confirmed.length, 1);
  assert.equal(confirmed[0].outputRoot, "F:/chosen/slc");
  assert.equal(confirmed[0].acquisition.frozenParameters.download_orbit, true);
  assert.deepEqual(confirmed[0].selectedProductIds, ["PRODUCT-A", "PRODUCT-B"]);
});

test("unchecked Orbit never enables a companion and in-flight changes are disabled", async () => {
  let release: (() => void) | undefined;
  const confirmed: any[] = [];
  render(<DownloadDestinationConfirmation draft={orbitDraft()}
    chooseDirectory={async () => null} onCancel={() => undefined}
    onConfirm={(value) => new Promise<void>((resolve) => { confirmed.push(value); release = resolve; })} />);
  const checkbox = screen.getByRole("checkbox") as HTMLInputElement;
  fireEvent.click(checkbox);
  fireEvent.click(checkbox);
  fireEvent.click(screen.getByRole("button", { name: "创建下载任务" }));
  await waitFor(() => assert.equal(checkbox.disabled, true));
  assert.equal(confirmed[0].acquisition.frozenParameters.download_orbit, false);
  await act(async () => { release?.(); });
});

test("Sentinel-1 confirmation surfaces frozen GACOS companion intent", () => {
  render(<DownloadDestinationConfirmation draft={createDownloadAcquisitionDraft({
    ...draft(),
    acquisition: {
      kind: "sentinel1",
      frozenParameters: {
        allow_companion_orbit: true,
        download_orbit: true,
        request_gacos: true,
      },
    },
  })} chooseDirectory={async () => null} onCancel={() => undefined} onConfirm={async () => undefined} />);
  assert.ok(screen.getByText("同时下载精密轨道"));
  assert.ok(screen.getByText("同时提交 GACOS 申请"));
});

test("other acquisitions do not expose the SLC Orbit option", () => {
  render(<DownloadDestinationConfirmation draft={createDownloadAcquisitionDraft({
    ...draft(), acquisition: { kind: "nisar_complete", frozenParameters: {} },
  })} chooseDirectory={async () => null} onCancel={() => undefined} onConfirm={async () => undefined} />);
  assert.equal(screen.queryByRole("checkbox"), null);
});

const LANDSAT_L2SP_KEYS = [
  "coastal", "blue", "green", "red", "nir08", "swir16", "swir22",
  "lwir11", "drad", "emis", "emsd", "trad", "urad", "atran", "cdist", "qa",
  "qa_pixel", "qa_radsat", "qa_aerosol", "ang", "mtl.txt", "mtl.xml",
] as const;

function landsatDraft(archiveAvailable: boolean) {
  const assets = LANDSAT_L2SP_KEYS.map((assetKey, index) => ({
    asset_key: assetKey,
    file_name: `LC09_TEST_${assetKey}.dat`,
    role: (["ang", "mtl.txt", "mtl.xml"].includes(assetKey)
      ? "metadata"
      : ["qa", "qa_pixel", "qa_radsat", "qa_aerosol"].includes(assetKey)
        ? "auxiliary"
        : "primary") as "primary" | "auxiliary" | "metadata",
    size_bytes: (index + 1) * 1_000_000,
  }));
  return createDownloadAcquisitionDraft({
    ...draft(),
    selectedProductIds: ["LANDSAT-A"],
    selectedProductCount: 1,
    acquisition: {
      kind: "landsat",
      frozenParameters: {
        delivery_form: "official_archive",
        official_archive_available: archiveAvailable,
        complete_asset_set_available: true,
        content_selection: "all",
        selected_asset_keys: assets.map((asset) => asset.asset_key),
        available_assets: assets,
        product_folder_name: "LC09_TEST_PRODUCT",
      },
    },
    layout: "direct",
  });
}

test("Landsat confirmation never silently replaces an unavailable USGS archive with the MPC file set", async () => {
  const confirmed: any[] = [];
  render(<DownloadDestinationConfirmation
    draft={landsatDraft(false)}
    chooseDirectory={async () => null}
    onCancel={() => undefined}
    onConfirm={async (value) => { confirmed.push(value); }}
  />);

  const archive = screen.getByRole("radio", { name: /官方原始归档/ }) as HTMLInputElement;
  const files = screen.getByRole("radio", { name: /展开文件集/ }) as HTMLInputElement;
  assert.equal(archive.checked, true);
  assert.equal(archive.disabled, true);
  assert.equal(files.checked, false);
  assert.equal((screen.getByRole("button", { name: "创建下载任务" }) as HTMLButtonElement).disabled, true);
  assert.ok(screen.getByText(/可先登录 USGS M2M/));
  assert.ok(screen.getByText(/实际保存到独立产品目录：.*LC09_TEST_PRODUCT/));

  fireEvent.click(files);
  assert.equal(files.checked, true);
  assert.equal(archive.checked, false);
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "创建下载任务" }));
  });
  assert.equal(confirmed.length, 1);
  assert.equal(confirmed[0].acquisition.frozenParameters.delivery_form, "complete_asset_set");
});

test("Landsat SR selection freezes 13 files and hides provider choice behind expanded delivery", async () => {
  const confirmed: any[] = [];
  render(<DownloadDestinationConfirmation
    draft={landsatDraft(true)}
    chooseDirectory={async () => null}
    onCancel={() => undefined}
    onConfirm={async (value) => { confirmed.push(value); }}
  />);

  fireEvent.click(screen.getByRole("radio", { name: /二级表面反射率/ }));
  assert.ok(screen.getByText(/已选 13 \/ 22 个文件/));
  assert.equal(screen.queryByRole("radio", { name: /官方原始归档/ }), null);
  assert.equal(screen.queryByRole("radio", { name: /展开文件集/ }), null);
  assert.ok(screen.getByText(/所选内容将按展开文件保存/));

  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "创建下载任务" }));
  });
  assert.equal(confirmed.length, 1);
  assert.equal(confirmed[0].acquisition.frozenParameters.delivery_form, "complete_asset_set");
  assert.equal(confirmed[0].acquisition.frozenParameters.content_selection, "surface_reflectance");
  assert.equal(confirmed[0].acquisition.frozenParameters.selected_asset_keys.length, 13);
});

test("Landsat custom file selection updates frozen count and size", async () => {
  const confirmed: any[] = [];
  render(<DownloadDestinationConfirmation
    draft={landsatDraft(true)}
    chooseDirectory={async () => null}
    onCancel={() => undefined}
    onConfirm={async (value) => { confirmed.push(value); }}
  />);
  fireEvent.click(screen.getByRole("radio", { name: /自定义文件/ }));
  const red = screen.getByRole("checkbox", { name: /红光 B4/ }) as HTMLInputElement;
  assert.equal(red.checked, true);
  fireEvent.click(red);
  assert.equal(red.checked, false);
  assert.ok(screen.getByText(/已选 21 \/ 22 个文件/));

  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "创建下载任务" }));
  });
  assert.equal(confirmed.length, 1);
  assert.equal(confirmed[0].acquisition.frozenParameters.selected_asset_keys.includes("red"), false);
  assert.equal(confirmed[0].acquisition.frozenParameters.selected_asset_keys.length, 21);
});

test("Landsat original archive remains the default when USGS bundle delivery is available", async () => {
  const confirmed: any[] = [];
  render(<DownloadDestinationConfirmation
    draft={landsatDraft(true)}
    chooseDirectory={async () => null}
    onCancel={() => undefined}
    onConfirm={async (value) => { confirmed.push(value); }}
  />);
  const archive = screen.getByRole("radio", { name: /官方原始归档/ }) as HTMLInputElement;
  assert.equal(archive.checked, true);
  assert.equal(archive.disabled, false);
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "创建下载任务" }));
  });
  assert.equal(confirmed.length, 1);
  assert.equal(confirmed[0].acquisition.frozenParameters.delivery_form, "official_archive");
});

test("Unified confirmation sheet supports NISAR AOI subset Range V2 preflight, manifest toggle, and blocks outside-click dismiss", async () => {
  const nisarCalls: Array<{ frequency: "A" | "B"; polarizations: string[]; outputDir: string }> = [];
  let cancelled = false;
  const nisarDraft = createDownloadAcquisitionDraft({
    sourceBatchId: "nisar-batch",
    sourceBatchRevision: 2,
    selectionScope: "checked",
    selectedProductIds: ["NISAR_L2_RSLC_001"],
    selectedProductCount: 1,
    acquisition: {
      kind: "nisar_subset",
      frozenParameters: {
        subset_aoi: { west: 117.1, south: 31.7, east: 117.3, north: 31.9 },
        frequency: "A",
        polarizations: ["HH", "HV"],
        transfer_mode: "range_v2",
      },
    },
    destinationPrefill: "F:/downloads/nisar",
    layout: "direct",
    taskPolicy: "manual",
    concurrency: 2,
  });

  const view = render(
    <DownloadDestinationConfirmation
      draft={nisarDraft}
      chooseDirectory={async () => null}
      onCancel={() => { cancelled = true; }}
      onConfirm={async () => {}}
      activeResourceId="science.nisar.l.rslc.provisional.v1"
      aoiBbox={{ west: 117.1, south: 31.7, east: 117.3, north: 31.9 }}
      selectedScenes={[
        {
          scene_id: "NISAR_L2_RSLC_001",
          platform: "NISAR",
          product_type: "RSLC",
          beam_mode: "Q",
          polarization: "HH+HV",
          acquisition_datetime: "2025-08-01T10:00:00Z",
          absolute_orbit: 100,
          relative_orbit: 12,
          path: 12,
          frame: 40,
          orbit_direction: "ASCENDING",
          has_url: true,
          file_size_remote: 10 * 1024 * 1024 * 1024,
          footprint_bbox: { west: 116.5, south: 31.0, east: 118.0, north: 32.5 },
        },
      ]}
      onConfirmNisarSubset={async (opts) => {
        nisarCalls.push(opts);
      }}
    />,
  );

  // Clicking the ModalOverlay backdrop must NOT dismiss the confirmation sheet
  const backdrop = view.container.firstElementChild as HTMLElement;
  fireEvent.click(backdrop);
  assert.equal(cancelled, false);

  assert.ok(screen.getByTestId("nisar-range-v2-preflight"));
  assert.equal(view.container.textContent?.includes("Range V2"), false, "Must not leak internal Range V2 jargon to users");
  assert.ok(view.container.textContent?.includes("HDF5 HTTP Range"));
  const estEl = screen.getByTestId("nisar-subset-estimate-summary");
  assert.ok(estEl.className.includes("text-[12px]"), "estimate summary must be 12px (never 11px)");
  assert.equal(estEl.className.includes("text-[11px]"), false);
  fireEvent.click(screen.getByTestId("toggle-scene-manifest"));
  assert.ok(screen.getByText("NISAR_L2_RSLC_001"));

  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "创建下载任务" }));
  });
  assert.deepEqual(nisarCalls, [
    { frequency: "A", polarizations: ["HH", "HV"], outputDir: "F:/downloads/nisar" },
  ]);
});

test("Full-product confirmation for NISAR / dedicated_geodata hides the duplicate AOI Subset switcher even when AOI exists", async () => {
  const confirmed: any[] = [];
  const fullDraft = createDownloadAcquisitionDraft({
    sourceBatchId: "geodata-direct-science.nisar.l.rslc.provisional.v1",
    sourceBatchRevision: 0,
    selectionScope: "exact_item",
    selectedProductIds: ["NISAR_L2_GSLC_001"],
    selectedProductCount: 1,
    acquisition: {
      kind: "generic_sar",
      frozenParameters: {
        dedicated_geodata: true,
        dataset_label: "NISAR L 波段 SAR",
        product_level: "GSLC",
        estimated_total_bytes: 4_500_000_000,
      },
    },
    destinationPrefill: "F:/downloads/nisar-gslc",
    layout: "direct",
    taskPolicy: "manual",
    concurrency: 2,
  });

  render(
    <DownloadDestinationConfirmation
      draft={fullDraft}
      chooseDirectory={async () => null}
      onCancel={() => undefined}
      onConfirm={async (val) => { confirmed.push(val); }}
      activeResourceId="science.nisar.l.rslc.provisional.v1"
      aoiBbox={{ west: 117.1, south: 31.7, east: 117.3, north: 31.9 }}
    />,
  );

  assert.equal(screen.queryByRole("radio", { name: /按当前研究区裁剪下载/ }), null);
  assert.ok(screen.getByText("NISAR L 波段 SAR"));
  assert.ok(screen.getByText(/产品级别: GSLC/));

  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "创建下载任务" }));
  });
  assert.equal(confirmed.length, 1);
  assert.equal(confirmed[0].outputRoot, "F:/downloads/nisar-gslc");
});

test("目的目录 supports manual typing and rejects temporary system /tmp paths", async () => {
  const confirmed: any[] = [];
  render(
    <DownloadDestinationConfirmation
      draft={draft()}
      chooseDirectory={async () => null}
      onCancel={() => undefined}
      onConfirm={async (val) => { confirmed.push(val); }}
    />,
  );

  const input = screen.getByLabelText("目的目录") as HTMLInputElement;
  assert.equal(input.readOnly, false);

  fireEvent.change(input, { target: { value: "/tmp/should-reject" } });
  const alert = await screen.findByRole("alert");
  assert.match(alert.textContent ?? "", /不能将系统临时目录/);
  fireEvent.click(screen.getByRole("button", { name: "创建下载任务" }));
  assert.equal(confirmed.length, 0);

  fireEvent.change(input, { target: { value: "F:/downloads/typed-path" } });
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "创建下载任务" }));
  });
  assert.equal(confirmed.length, 1);
  assert.equal(confirmed[0].outputRoot, "F:/downloads/typed-path");
});

test("Sentinel-2 L1C exposes 13 bands (incl. B10) and HLS S30 exposes band presets and custom selection", async () => {
  const confirmedL1c: any[] = [];
  const l1cView = render(
    <DownloadDestinationConfirmation
      draft={createDownloadAcquisitionDraft({
        sourceBatchId: "s2-l1c-batch",
        sourceBatchRevision: 1,
        selectionScope: "checked",
        selectedProductIds: ["S2-L1C-A"],
        acquisition: {
          kind: "sentinel2",
          frozenParameters: {
            product_level: "l1c",
            delivery_form: "analysis_bands",
            safe_archive_available: true,
            analysis_bands_available: true,
            band_preset: "custom",
            selected_band_keys: ["B02", "B10"],
          },
        },
        destinationPrefill: "F:/downloads/s2l1c",
        layout: "direct",
        taskPolicy: "manual",
        concurrency: 2,
      })}
      chooseDirectory={async () => null}
      onCancel={() => undefined}
      onConfirm={async (val) => { confirmedL1c.push(val); }}
    />,
  );

  assert.ok(screen.getByText(/已选 2 \/ 13 个波段/));
  assert.ok(screen.getByRole("checkbox", { name: /B10 卷云波段/ }));
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "创建下载任务" }));
  });
  assert.equal(confirmedL1c.length, 1);
  assert.deepEqual(confirmedL1c[0].acquisition.frozenParameters.selected_band_keys, ["B02", "B10"]);
  l1cView.unmount();

  const confirmedHls: any[] = [];
  render(
    <DownloadDestinationConfirmation
      draft={createDownloadAcquisitionDraft({
        sourceBatchId: "hls-s30-batch",
        sourceBatchRevision: 1,
        selectionScope: "checked",
        selectedProductIds: ["HLS.S30.001"],
        acquisition: {
          kind: "hls",
          frozenParameters: {
            hls_product: "S30",
            band_preset: "all_bands",
            selected_band_keys: ["B01", "B02", "B03", "B04", "B05", "B06", "B07", "B08", "B8A", "B09", "B10", "B11", "B12", "Fmask"],
          },
        },
        destinationPrefill: "F:/downloads/hls",
        layout: "direct",
        taskPolicy: "manual",
        concurrency: 2,
      })}
      chooseDirectory={async () => null}
      onCancel={() => undefined}
      onConfirm={async (val) => { confirmedHls.push(val); }}
    />,
  );
  fireEvent.click(screen.getByRole("radio", { name: /短波红外 \(SWIR\)/ }));
  assert.ok(screen.getByText(/已选 3 \/ 14 个波段/));
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "创建下载任务" }));
  });
  assert.equal(confirmedHls.length, 1);
  assert.deepEqual(confirmedHls[0].acquisition.frozenParameters.selected_band_keys, ["B12", "B11", "B04"]);
});

test("SAR companion checkboxes appear strictly on Sentinel-1 (hidden on Black Marble & non-S1 SAR) and layout uses human wording", async () => {
  const blackMarbleView = render(
    <DownloadDestinationConfirmation
      draft={createDownloadAcquisitionDraft({
        sourceBatchId: "geodata-direct-science.nasa.black-marble.v1",
        sourceBatchRevision: 0,
        selectionScope: "exact_item",
        selectedProductIds: ["VNP46A3.A2025001.h28v05"],
        acquisition: {
          kind: "generic_sar",
          frozenParameters: {
            dedicated_geodata: true,
            dataset_label: "NASA Black Marble 夜光遥感 (VNP46A)",
            resource_id: "science.nasa.black-marble.v1",
            estimated_total_bytes: 12_000_000,
          },
        },
        destinationPrefill: "F:/downloads/blackmarble",
        layout: "direct",
        taskPolicy: "manual",
        concurrency: 2,
      })}
      activeResourceId="science.nasa.black-marble.v1"
      chooseDirectory={async () => null}
      onCancel={() => undefined}
      onConfirm={async () => undefined}
      onConfirmSarCompanions={async () => undefined}
    />,
  );
  assert.equal(screen.queryByTestId("confirm-sar-companion-section"), null);
  assert.equal(screen.queryByTestId("confirm-checkbox-gacos"), null);
  assert.equal(screen.queryByTestId("confirm-checkbox-dem"), null);
  assert.equal(screen.queryByText(/伴随辅助数据/), null);
  const bmText = blackMarbleView.container.textContent ?? "";
  assert.match(bmText, /保存方式：直接保存到所选文件夹/);
  assert.doesNotMatch(bmText, /布局：direct|\bdirect\b/);
  const directBadge = screen.getByText("直接保存到所选文件夹");
  assert.ok(directBadge.getAttribute("title")?.includes("不额外新建分类子文件夹"));
  blackMarbleView.unmount();

  const nisarView = render(
    <DownloadDestinationConfirmation
      draft={createDownloadAcquisitionDraft({
        sourceBatchId: "nisar-batch",
        sourceBatchRevision: 1,
        selectionScope: "exact_item",
        selectedProductIds: ["NISAR_L2_GCOV_001"],
        acquisition: {
          kind: "generic_sar",
          frozenParameters: {
            resource_id: "science.nisar.l-band.rslc.v1",
            dataset_label: "NISAR L 波段 SAR",
          },
        },
        destinationPrefill: "F:/downloads/nisar",
        layout: "sar_dataset_processing",
        taskPolicy: "manual",
        concurrency: 2,
      })}
      activeResourceId="science.nisar.l-band.rslc.v1"
      chooseDirectory={async () => null}
      onCancel={() => undefined}
      onConfirm={async () => undefined}
      onConfirmSarCompanions={async () => undefined}
    />,
  );
  assert.equal(screen.queryByTestId("confirm-sar-companion-section"), null);
  assert.equal(screen.queryByTestId("confirm-checkbox-gacos"), null);
  assert.equal(screen.queryByTestId("confirm-checkbox-dem"), null);
  assert.match(nisarView.container.textContent ?? "", /保存方式：按产品类型自动建子目录/);
  nisarView.unmount();

  const s1View = render(
    <DownloadDestinationConfirmation
      draft={createDownloadAcquisitionDraft({
        sourceBatchId: "s1-batch",
        sourceBatchRevision: 1,
        selectionScope: "exact_item",
        selectedProductIds: ["S1A_IW_SLC_001"],
        acquisition: {
          kind: "sentinel1",
          frozenParameters: {
            allow_companion_orbit: true,
            download_orbit: true,
            allow_companion_gacos: true,
            request_gacos: false,
            request_dem: false,
          },
        },
        destinationPrefill: "F:/downloads/s1",
        layout: "sar_dataset_processing",
        taskPolicy: "manual",
        concurrency: 2,
      })}
      activeResourceId="science.sentinel-1.standard.v1"
      chooseDirectory={async () => null}
      onCancel={() => undefined}
      onConfirm={async () => undefined}
      onConfirmSarCompanions={async () => undefined}
    />,
  );
  assert.ok(screen.getByTestId("confirm-sar-companion-section"));
  const dialog = screen.getByRole("dialog");
  const orbitBox = dialog.querySelector('input[type="checkbox"]:not([data-testid])') as HTMLInputElement;
  const gacosBox = screen.getByTestId("confirm-checkbox-gacos") as HTMLInputElement;
  const demBox = screen.getByTestId("confirm-checkbox-dem") as HTMLInputElement;
  assert.ok(orbitBox && gacosBox && demBox);
  assert.equal(orbitBox.disabled, false);
  assert.equal(gacosBox.disabled, false);
  assert.equal(demBox.disabled, false);
  fireEvent.click(demBox);
  assert.equal(demBox.checked, true);
  s1View.unmount();
});
