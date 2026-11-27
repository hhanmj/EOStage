/**
 * challenger_m3_1_empirical_stress.interaction.test.tsx
 * 
 * Empirical Adversarial Challenger Test Suite for Milestone 3 UI Deliverables:
 * - Multi-provider switching and parameter propagation
 * - Custom GEE asset ID input validation, event handling and edge cases
 * - Failover suggestion banner conditions and zero silent swaps
 * - Category counts and registry integrity
 */

import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { render, cleanup, fireEvent, act } from "@testing-library/react";

import { DataSourceCatalogModal } from "./DataSourceCatalogModal";
import {
  ManualConditionsPanel,
  MULTI_PROVIDER_CONFIGS,
  getActiveDatasetProvider,
  setActiveDatasetProvider,
  isMultiProviderDataset,
  getWorkbenchToolOptions,
  updateWorkbenchToolOptions,
  subscribeWorkbenchToolOptions,
  type WorkbenchToolOptionsSnapshot,
} from "./ManualConditionsPanel";
import {
  buildCatalogItems,
  filterCatalogItems,
  computeCategoryCounts,
  clearRecentSources,
  getRecentSourceIds,
  CATALOG_METADATA_REGISTRY,
  type CatalogCategoryKey,
} from "../../lib/dataSourceCatalogLogic";
import type { SourceDescriptor } from "@/shell/api/types";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const uiSrcDir = path.resolve(__dirname, "../..");

const mockDescriptors: SourceDescriptor[] = [
  {
    resource_id: "science.sentinel-1.standard.v1",
    group_id: "sentinel1",
    display_name_zh: "Sentinel-1",
    family: "sar",
    search_mode: "sar",
    filter_schema_id: null,
    filter_schema_revision: null,
    result_cap: 1000,
    accounts: ["asf.earthdata"],
    network_default: "direct",
    availability: "available",
    availability_reason_zh: null,
  },
  {
    resource_id: "science.sentinel-2.l2a.v1",
    group_id: "sentinel2",
    display_name_zh: "Sentinel-2 MSI",
    family: "optical",
    search_mode: "catalogue_intent",
    filter_schema_id: "copernicus.s2.v1",
    filter_schema_revision: 1,
    result_cap: 500,
    accounts: ["copernicus.cdse"],
    network_default: "direct",
    availability: "available",
    availability_reason_zh: null,
  },
  {
    resource_id: "science.landsat.c2-l2.v1",
    group_id: "landsat",
    display_name_zh: "Landsat 8-9 C2-L2",
    family: "optical",
    search_mode: "catalogue_intent",
    filter_schema_id: "usgs.landsat.v1",
    filter_schema_revision: 1,
    result_cap: 500,
    accounts: ["usgs.m2m"],
    network_default: "direct",
    availability: "available",
    availability_reason_zh: null,
  },
];

test.afterEach(() => {
  cleanup();
  clearRecentSources();
  // Reset selected providers to canonical defaults
  updateWorkbenchToolOptions({
    selectedProviders: {
      "science.sentinel-2.l2a.v1": "cdse",
      "science.sentinel-2.l1c.v1": "cdse",
      "science.landsat.c2-l2.v1": "mpc",
      "dem.cop30.v1": "aws",
      "COP30": "aws",
    },
    geeAssetId: "COPERNICUS/S2_SR_HARMONIZED",
  });
});

test("Challenger M3-1 Suite 1: Multi-Provider Switching, State Reactivity & Parameter Propagation", async (suite) => {
  await suite.test("1.1 Cycle Sentinel-2 providers (cdse -> mpc -> gee -> cdse) and verify geeAssetId propagation", () => {
    const s2Id = "science.sentinel-2.l2a.v1";
    assert.strictEqual(isMultiProviderDataset(s2Id), true);

    // Initial default is CDSE
    assert.strictEqual(getActiveDatasetProvider(s2Id), "cdse");

    // Track listener notifications
    let notificationCount = 0;
    const unsubscribe = subscribeWorkbenchToolOptions(() => {
      notificationCount += 1;
    });

    // Switch to MPC
    setActiveDatasetProvider(s2Id, "mpc");
    assert.strictEqual(getActiveDatasetProvider(s2Id), "mpc");
    assert.strictEqual(notificationCount, 1);

    // Switch to GEE: must automatically update defaultGeeAssetId
    setActiveDatasetProvider(s2Id, "gee");
    assert.strictEqual(getActiveDatasetProvider(s2Id), "gee");
    assert.strictEqual(notificationCount, 2);
    assert.strictEqual(getWorkbenchToolOptions().geeAssetId, "COPERNICUS/S2_SR_HARMONIZED");

    // Switch back to CDSE: provider is cdse, notification fired
    setActiveDatasetProvider(s2Id, "cdse");
    assert.strictEqual(getActiveDatasetProvider(s2Id), "cdse");
    assert.strictEqual(notificationCount, 3);

    unsubscribe();
  });

  await suite.test("1.2 Cycle Landsat C2-L2 providers (mpc -> usgs -> gee -> mpc) and verify geeAssetId", () => {
    const landsatId = "science.landsat.c2-l2.v1";
    assert.strictEqual(isMultiProviderDataset(landsatId), true);

    // Initial default is MPC
    assert.strictEqual(getActiveDatasetProvider(landsatId), "mpc");

    // Switch to USGS
    setActiveDatasetProvider(landsatId, "usgs");
    assert.strictEqual(getActiveDatasetProvider(landsatId), "usgs");

    // Switch to GEE
    setActiveDatasetProvider(landsatId, "gee");
    assert.strictEqual(getActiveDatasetProvider(landsatId), "gee");
    assert.strictEqual(getWorkbenchToolOptions().geeAssetId, "LANDSAT/LC09/C02/T1_L2");

    // Switch back to MPC
    setActiveDatasetProvider(landsatId, "mpc");
    assert.strictEqual(getActiveDatasetProvider(landsatId), "mpc");
  });

  await suite.test("1.3 Cycle Copernicus DEM providers (aws -> opentopography -> gee -> aws) and verify geeAssetId", () => {
    const demCode = "COP30";
    assert.strictEqual(isMultiProviderDataset(demCode), true);

    // Initial default is AWS
    assert.strictEqual(getActiveDatasetProvider(demCode), "aws");

    // Switch to OpenTopography
    setActiveDatasetProvider(demCode, "opentopography");
    assert.strictEqual(getActiveDatasetProvider(demCode), "opentopography");

    // Switch to GEE
    setActiveDatasetProvider(demCode, "gee");
    assert.strictEqual(getActiveDatasetProvider(demCode), "gee");
    assert.strictEqual(getWorkbenchToolOptions().geeAssetId, "COPERNICUS/DEM/GLO30");

    // Switch back to AWS
    setActiveDatasetProvider(demCode, "aws");
    assert.strictEqual(getActiveDatasetProvider(demCode), "aws");
  });

  await suite.test("1.4 Adversarial input to setActiveDatasetProvider: unknown resourceId and unsupported providerId fail-closed", () => {
    // 1. Unknown resourceId
    const unknownId = "science.nonexistent.satellite.v1";
    assert.strictEqual(isMultiProviderDataset(unknownId), false);
    assert.strictEqual(getActiveDatasetProvider(unknownId), "default");
    
    // Setting provider for unknown resourceId must be safely ignored
    setActiveDatasetProvider(unknownId, "some_provider");
    assert.strictEqual(getActiveDatasetProvider(unknownId), "default");

    // 2. Unsupported/invalid providerId for a known dataset
    const s2Id = "science.sentinel-2.l2a.v1";
    setActiveDatasetProvider(s2Id, "malicious_fake_provider");
    // getActiveDatasetProvider must fallback safely to cfg.options[0].id ("cdse")
    assert.strictEqual(getActiveDatasetProvider(s2Id), "cdse");
  });

  await suite.test("1.5 ManualConditionsPanel segmented buttons interaction for Sentinel-2, Landsat, and DEM", () => {
    const dummyProps = {
      scientificMode: "sar" as const,
      onResetParams: () => {},
      availableSources: mockDescriptors,
      activeResourceId: "science.sentinel-2.l2a.v1",
      onSelectSource: () => {},
      productType: "MSIL2A",
      onProductTypeChange: () => {},
      workingAoi: null,
      workingGeometry: null,
      aoiLabel: "全球视图",
      onReplaceAoi: async () => {},
      onImportVectorSuccess: () => {},
      onPreviewAdminGeometry: () => {},
      startDate: null,
      endDate: null,
      onStartDateChange: () => {},
      onEndDateChange: () => {},
      orbitDirection: "all",
      onOrbitDirectionChange: () => {},
      relativeOrbit: null,
      onRelativeOrbitChange: () => {},
      frame: null,
      onFrameChange: () => {},
      path: null,
      onPathChange: () => {},
      polarizations: [],
      onPolarizationsChange: () => {},
      onSearchStart: () => {},
      searching: false,
      searchError: null,
      searchProgress: null,
      onCancelSearch: () => {},
      onResetFilters: () => {},
      displayedBatch: null,
      batchRevision: 0,
      activeBatchItemCount: 0,
      checkedSceneIds: new Set<string>(),
      onCheckScene: () => {},
      onCheckAllMatching: () => {},
      onClearCheckedScenes: () => {},
      onRowDownload: async () => {},
      onBatchDownload: async () => {},
      rowTaskStates: {},
      isTaskActive: () => false,
      onRetryRowTask: async () => {},
      onRowAction: () => {},
      taskErrorExplanation: () => null,
      onOpenTaskOutput: () => {},
    };

    const { getByTestId, rerender } = render(<ManualConditionsPanel {...dummyProps} />);

    // Sentinel-2 options rendered
    const cdseOpt = getByTestId("provider-option-cdse");
    const mpcOpt = getByTestId("provider-option-mpc");
    const geeOpt = getByTestId("provider-option-gee");
    assert.ok(cdseOpt);
    assert.ok(mpcOpt);
    assert.ok(geeOpt);

    // Initial cdse is checked
    assert.strictEqual(cdseOpt.getAttribute("aria-checked"), "true");
    assert.strictEqual(mpcOpt.getAttribute("aria-checked"), "false");

    // Click MPC option
    act(() => {
      fireEvent.click(mpcOpt);
    });
    assert.strictEqual(getActiveDatasetProvider("science.sentinel-2.l2a.v1"), "mpc");

    // Click GEE option
    act(() => {
      fireEvent.click(geeOpt);
    });
    assert.strictEqual(getActiveDatasetProvider("science.sentinel-2.l2a.v1"), "gee");
    assert.strictEqual(getWorkbenchToolOptions().geeAssetId, "COPERNICUS/S2_SR_HARMONIZED");

    // Rerender with Landsat
    rerender(
      <ManualConditionsPanel
        {...dummyProps}
        activeResourceId="science.landsat.c2-l2.v1"
        productType="C2_L2"
      />
    );

    const landsatMpc = getByTestId("provider-option-mpc");
    const landsatUsgs = getByTestId("provider-option-usgs");
    const landsatGee = getByTestId("provider-option-gee");
    assert.ok(landsatMpc);
    assert.ok(landsatUsgs);
    assert.ok(landsatGee);

    act(() => {
      fireEvent.click(landsatUsgs);
    });
    assert.strictEqual(getActiveDatasetProvider("science.landsat.c2-l2.v1"), "usgs");

    act(() => {
      fireEvent.click(landsatGee);
    });
    assert.strictEqual(getActiveDatasetProvider("science.landsat.c2-l2.v1"), "gee");
    assert.strictEqual(getWorkbenchToolOptions().geeAssetId, "LANDSAT/LC09/C02/T1_L2");
  });
});

test("Challenger M3-1 Suite 2: Custom GEE Asset ID Validation, Edge Cases & Pre-Configured Collections", async (suite) => {
  await suite.test("2.1 Empty and whitespace-only GEE Asset ID inputs disable submit button", () => {
    let chosenSource = "";
    let closed = false;

    const { getByTestId } = render(
      <DataSourceCatalogModal
        isOpen={true}
        onClose={() => { closed = true; }}
        onSelectSource={(id) => { chosenSource = id; }}
        availableSources={mockDescriptors}
        activeResourceId="science.sentinel-2.l2a.v1"
      />
    );

    // Switch to GEE category
    act(() => {
      fireEvent.click(getByTestId("catalog-nav-gee"));
    });

    const input = getByTestId("gee-custom-asset-id-input") as HTMLInputElement;
    const submitBtn = getByTestId("gee-custom-asset-id-submit-btn") as HTMLButtonElement;

    // Test with pure spaces
    act(() => {
      fireEvent.change(input, { target: { value: "    " } });
    });
    assert.strictEqual(submitBtn.disabled, true, "Submit button must be disabled for pure whitespace");

    // Test with tabs and newlines
    act(() => {
      fireEvent.change(input, { target: { value: "\t\n  \r\n" } });
    });
    assert.strictEqual(submitBtn.disabled, true, "Submit button must be disabled for whitespace and newlines");

    // Clicking a disabled button directly should not trigger submit
    act(() => {
      fireEvent.click(submitBtn);
    });
    assert.strictEqual(chosenSource, "");
    assert.strictEqual(closed, false);
  });

  await suite.test("2.2 Whitespace-surrounded valid Asset ID is automatically trimmed on submit", () => {
    let chosenSource = "";
    let closed = false;

    const { getByTestId } = render(
      <DataSourceCatalogModal
        isOpen={true}
        onClose={() => { closed = true; }}
        onSelectSource={(id) => { chosenSource = id; }}
        availableSources={mockDescriptors}
        activeResourceId="science.sentinel-2.l2a.v1"
      />
    );

    act(() => {
      fireEvent.click(getByTestId("catalog-nav-gee"));
    });

    const input = getByTestId("gee-custom-asset-id-input") as HTMLInputElement;
    const submitBtn = getByTestId("gee-custom-asset-id-submit-btn") as HTMLButtonElement;

    const untrimmedAsset = "   projects/my-gcp-project/assets/custom_sar_mosaic   ";
    act(() => {
      fireEvent.change(input, { target: { value: untrimmedAsset } });
    });
    assert.strictEqual(submitBtn.disabled, false);

    act(() => {
      fireEvent.click(submitBtn);
    });

    assert.strictEqual(chosenSource, "science.gee.catalog.v1");
    assert.strictEqual(closed, true);
    assert.strictEqual(
      getWorkbenchToolOptions().geeAssetId,
      "projects/my-gcp-project/assets/custom_sar_mosaic"
    );
  });

  await suite.test("2.3 Enter key submits valid Asset ID when input is non-empty", () => {
    let chosenSource = "";
    let closed = false;

    const { getByTestId } = render(
      <DataSourceCatalogModal
        isOpen={true}
        onClose={() => { closed = true; }}
        onSelectSource={(id) => { chosenSource = id; }}
        availableSources={mockDescriptors}
        activeResourceId="science.sentinel-2.l2a.v1"
      />
    );

    act(() => {
      fireEvent.click(getByTestId("catalog-nav-gee"));
    });

    const input = getByTestId("gee-custom-asset-id-input") as HTMLInputElement;

    act(() => {
      fireEvent.change(input, { target: { value: "users/researcher/inundation_extent" } });
      fireEvent.keyDown(input, { key: "Enter" });
    });

    assert.strictEqual(chosenSource, "science.gee.catalog.v1");
    assert.strictEqual(closed, true);
    assert.strictEqual(
      getWorkbenchToolOptions().geeAssetId,
      "users/researcher/inundation_extent"
    );
  });

  await suite.test("2.4 Enter key in custom input does NOT bubble to modal listener on whitespace", () => {
    // When the user enters empty whitespace in the custom asset ID input and hits Enter:
    // With e.stopPropagation() and e.preventDefault() in place, the Enter key does NOT bubble
    // up to the modal's global keydown listener. The modal stays open and no item is selected.
    let chosenSource = "";
    let closed = false;

    const { getByTestId } = render(
      <DataSourceCatalogModal
        isOpen={true}
        onClose={() => { closed = true; }}
        onSelectSource={(id) => { chosenSource = id; }}
        availableSources={mockDescriptors}
        activeResourceId="science.sentinel-2.l2a.v1"
      />
    );

    act(() => {
      fireEvent.click(getByTestId("catalog-nav-gee"));
    });

    const input = getByTestId("gee-custom-asset-id-input") as HTMLInputElement;

    // Set whitespace
    act(() => {
      fireEvent.change(input, { target: { value: "   " } });
      fireEvent.keyDown(input, { key: "Enter" });
    });

    // Empirically observes that modal was NOT closed because bubbling was stopped
    assert.strictEqual(closed, false, "Modal must remain open when Enter is pressed with whitespace");
    assert.strictEqual(chosenSource, "", "No source must be selected when Enter is pressed with whitespace");
  });

  await suite.test("2.5 Selecting pre-configured GEE collections routes to science.gee.catalog.v1 with registered geeAssetId", () => {
    let chosenSource = "";
    let closed = false;

    const { getByTestId, container, getByText } = render(
      <DataSourceCatalogModal
        isOpen={true}
        onClose={() => { closed = true; }}
        onSelectSource={(id) => { chosenSource = id; }}
        availableSources={mockDescriptors}
        activeResourceId="science.sentinel-2.l2a.v1"
      />
    );

    act(() => {
      fireEvent.click(getByTestId("catalog-nav-gee"));
    });

    // Select Dynamic World
    const dynamicWorldRow = container.querySelector(
      '[data-resource-id="science.gee.dynamic-world.v1"]'
    ) as HTMLElement;
    assert.ok(dynamicWorldRow, "science.gee.dynamic-world.v1 row must exist");

    act(() => {
      fireEvent.click(dynamicWorldRow);
    });

    // Click "使用此数据源"
    const useBtn = getByText("使用此数据源");
    act(() => {
      fireEvent.click(useBtn);
    });

    assert.strictEqual(chosenSource, "science.gee.catalog.v1");
    assert.strictEqual(closed, true);
    assert.strictEqual(getWorkbenchToolOptions().geeAssetId, "GOOGLE/DYNAMICWORLD/V1");
  });
});

test("Challenger M3-1 Suite 3: Failover Suggestion Banner, Zero Silent Swaps & Source Wiring", async (suite) => {
  await suite.test("3.1 DesktopWorkbench source inspection: DEM failover banner, #6.1 silent optical fallback & catch safety", () => {
    const workbenchPath = path.resolve(uiSrcDir, "components/desktop/DesktopWorkbench.tsx");
    const src = fs.readFileSync(workbenchPath, "utf-8");

    // 1. Verify failover banner markup and testids (retained for DEM COP30)
    assert.ok(
      src.includes('data-testid="failover-notice-banner"'),
      "DesktopWorkbench must render data-testid='failover-notice-banner'"
    );
    assert.ok(
      src.includes('data-testid={`failover-btn-${opt.provider}`}'),
      "DesktopWorkbench must render interactive provider switch buttons"
    );

    // 2-3. Provenance (Owner 2026-11-14 #6.1): Optical failover suggestion banners
    // (Sentinel-2 CDSE & Landsat MPC/USGS) were intentionally replaced by silent
    // backend/internal optical fallback (trySilentOpticalFallback) without user-facing
    // optical provider prompts.
    assert.ok(
      src.includes("trySilentOpticalFallback"),
      "DesktopWorkbench must use silent optical fallback (trySilentOpticalFallback) per #6.1"
    );

    // 4. Verify user-directed transition in handleFailoverSwitch (for retained DEM banner)
    assert.ok(
      src.includes("setActiveDatasetProvider(effectiveMultiId, targetProvider)"),
      "handleFailoverSwitch must explicitly set the new provider"
    );
    assert.ok(
      src.includes("setFailoverBanner(null)"),
      "handleFailoverSwitch must dismiss the banner"
    );
    assert.ok(
      src.includes("void handleTriggerSearch()"),
      "handleFailoverSwitch must re-trigger search with the new provider"
    );

    // 5. Verify no direct setActiveDatasetProvider inside catch blocks
    const catchBlocks = src.match(/catch\s*\([^)]*\)\s*\{[^}]*\}/g) || [];
    for (const cb of catchBlocks) {
      assert.strictEqual(
        cb.includes("setActiveDatasetProvider"),
        false,
        "catch blocks must not directly call setActiveDatasetProvider"
      );
    }
  });

  await suite.test("3.2 Multi-provider selection in DesktopWorkbench: recorded provenance for #6.1 silent backend selection", () => {
    // Provenance (Owner 2026-11-14 #6.1): User-facing multi-provider bar
    // (workbench-multi-provider-bar) was intentionally removed in favor of
    // silent backend provider selection; users do not pick upstream providers directly.
    const workbenchPath = path.resolve(uiSrcDir, "components/desktop/DesktopWorkbench.tsx");
    const src = fs.readFileSync(workbenchPath, "utf-8");

    assert.ok(
      !src.includes('data-testid="workbench-multi-provider-bar"'),
      "DesktopWorkbench must not re-introduce removed workbench-multi-provider-bar (#6.1)"
    );
  });
});

test("Challenger M3-1 Suite 4: Registry Integrity, Category Counts & Metadata Completeness", async (suite) => {
  await suite.test("4.1 Verify all registered GEE collections and category counts", () => {
    const allItems = buildCatalogItems();
    const counts = computeCategoryCounts(allItems, []);

    // GEE category must have registered items
    assert.ok(counts["gee-catalog"] >= 10, `gee-catalog should have at least 10 items, got ${counts["gee-catalog"]}`);

    const geeItems = filterCatalogItems(allItems, { category: "gee-catalog" });
    assert.strictEqual(geeItems.length, counts["gee-catalog"]);

    // Key collections must exist in registry
    const expectedCollections = [
      "science.gee.catalog.v1",
      "science.gee.s2-sr.v1",
      "science.gee.s1-grd.v1",
      "science.gee.landsat9.v1",
      "science.gee.landsat8.v1",
      "science.gee.dynamic-world.v1",
      "science.gee.worldcover.v1",
      "science.gee.gsw.v1",
      "science.gee.dem-glo30.v1",
      "science.gee.nasadem.v1",
      "science.gee.gem-15.v1",
      "science.gee.modis-ndvi.v1",
      "science.gee.era5-land.v1",
      "science.gee.s5p-no2.v1",
    ];

    for (const resId of expectedCollections) {
      const item = allItems.find((i) => i.resourceId === resId);
      assert.ok(item, `Registered catalog item ${resId} must exist`);
      assert.strictEqual(item.status, "available");
      assert.strictEqual(item.disabled, false);
      if (resId !== "science.gee.catalog.v1") {
        assert.ok(item.geeAssetId && item.geeAssetId.length > 0, `${resId} must have valid geeAssetId`);
      }
    }
  });
});
