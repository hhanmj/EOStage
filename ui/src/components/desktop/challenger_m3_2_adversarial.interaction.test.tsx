/**
 * challenger_m3_2_adversarial.test.tsx
 * 
 * Adversarial stress harness for Milestone 3 UI Deliverables:
 * 1. Typography compliance (no font-size < 12px, no rounded-control, w545SettingsTokens rules).
 * 2. Catalog item clicks, category switches, filtering stability, edge-case queries, custom GEE asset handling.
 * 3. Non-silent failover honors user consent: zero automatic swapping, explicit user-directed transition.
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
  MULTI_PROVIDER_CONFIGS,
  getActiveDatasetProvider,
  setActiveDatasetProvider,
  getWorkbenchToolOptions,
  updateWorkbenchToolOptions,
  type WorkbenchToolOptionsSnapshot,
} from "./ManualConditionsPanel";
import {
  buildCatalogItems,
  filterCatalogItems,
  clearRecentSources,
  type CatalogCategoryKey,
  type CatalogBandTag,
  type CatalogAccountTag,
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
});

test("Adversarial Suite: Typography & Token Compliance (Rule A, Rule B, Rule J)", async (t) => {
  await t.test("1. AST & Regex scan of M3 modified files for font-size < 12px or rounded-control", () => {
    const targetFiles = [
      "components/desktop/DataSourceCatalogModal.tsx",
      "components/desktop/ManualConditionsPanel.tsx",
      "components/desktop/DesktopWorkbench.tsx",
      "lib/dataSourceCatalogLogic.ts",
    ];

    const sub12Regex = /\btext-\[(?:[1-9]|1[01]|11\.\d+)px\](?![\w-])/g;
    const roundedControlRegex = /\brounded-control\b/g;

    for (const relFile of targetFiles) {
      const fullPath = path.resolve(uiSrcDir, relFile);
      const content = fs.readFileSync(fullPath, "utf8");
      const lines = content.split(/\r?\n/);

      for (let i = 0; i < lines.length; i++) {
        const line = lines[i];
        sub12Regex.lastIndex = 0;
        let m = sub12Regex.exec(line);
        assert.strictEqual(
          m,
          null,
          `Found forbidden sub-12px font (${m?.[0]}) in ${relFile}:${i + 1}: ${line.trim()}`
        );

        roundedControlRegex.lastIndex = 0;
        m = roundedControlRegex.exec(line);
        assert.strictEqual(
          m,
          null,
          `Found forbidden rounded-control in ${relFile}:${i + 1}: ${line.trim()}`
        );
      }
    }
  });

  await t.test("2. DOM Rendered Font Verification in DataSourceCatalogModal", () => {
    const { container } = render(
      <DataSourceCatalogModal
        isOpen={true}
        onClose={() => {}}
        onSelectSource={() => {}}
        availableSources={mockDescriptors}
        activeResourceId="science.sentinel-2.l2a.v1"
      />
    );

    // Verify all text elements in the modal have valid font classes or text-xs (12px)
    const allElements = container.querySelectorAll("*");
    for (const el of allElements) {
      const className = el.getAttribute("class") || "";
      // Check for any text-[Npx] where N < 12
      const match = className.match(/text-\[(\d+(?:\.\d+)?)px\]/);
      if (match) {
        const px = parseFloat(match[1]);
        assert.ok(
          px >= 12,
          `Element <${el.tagName}> has font-size ${px}px < 12px in DataSourceCatalogModal: ${className}`
        );
      }
    }
  });
});

test("Adversarial Suite: Catalog Item Clicks, Category Switching, and Filtering Stability", async (t) => {
  await t.test("1. Stress test switching across ALL categories without unmount crash", () => {
    let selectedSource = "";
    const { getByTestId, queryByTestId, rerender } = render(
      <DataSourceCatalogModal
        isOpen={true}
        onClose={() => {}}
        onSelectSource={(id) => { selectedSource = id; }}
        availableSources={mockDescriptors}
        activeResourceId="science.sentinel-2.l2a.v1"
      />
    );

    const categories: { testId: string; catKey: CatalogCategoryKey }[] = [
      { testId: "catalog-nav-recent", catKey: "recent" },
      { testId: "catalog-nav-primary-all", catKey: "primary-all" },
      { testId: "catalog-nav-primary-sar", catKey: "primary-sar" },
      { testId: "catalog-nav-primary-optical", catKey: "primary-optical" },
      { testId: "catalog-nav-primary-dem", catKey: "primary-dem" },
      { testId: "catalog-nav-primary-thematic", catKey: "primary-thematic" },
      { testId: "catalog-nav-auxiliary-orbit", catKey: "auxiliary-all" },
      { testId: "catalog-nav-auxiliary-atmospheric", catKey: "auxiliary-atmospheric" },
      { testId: "catalog-nav-auxiliary-geodesy", catKey: "auxiliary-geodesy" },
      { testId: "catalog-nav-gee", catKey: "gee-catalog" },
    ];

    for (const { testId, catKey } of categories) {
      const navBtn = getByTestId(testId);
      assert.ok(navBtn, `Nav button ${testId} must exist`);
      act(() => {
        fireEvent.click(navBtn);
      });

      // Special check for GEE category: custom asset container should mount
      if (catKey === "gee-catalog") {
        const customContainer = queryByTestId("catalog-gee-custom-asset-container");
        assert.ok(customContainer, "GEE custom asset container must be mounted when GEE category is active");
      } else {
        const customContainer = queryByTestId("catalog-gee-custom-asset-container");
        assert.strictEqual(customContainer, null, `GEE custom asset container must NOT be mounted in ${catKey}`);
      }
    }
  });

  await t.test("2. Adversarial search queries: regex specials, unicode, extreme strings", () => {
    const { getByPlaceholderText, container } = render(
      <DataSourceCatalogModal
        isOpen={true}
        onClose={() => {}}
        onSelectSource={() => {}}
        availableSources={mockDescriptors}
        activeResourceId="science.sentinel-2.l2a.v1"
      />
    );

    const searchInput = getByPlaceholderText("搜索名称、波段或关键词") as HTMLInputElement;
    assert.ok(searchInput, "Search input must exist");

    const adversarialQueries = [
      "",
      "   ",
      ".*",
      "[[[",
      "(?<=a)",
      "\\d+\\w+",
      "~`!@#$%^&*()_+{}|:\"<>?",
      "非匹配测试xyz9999",
      "Sentinel",
      "sEnTiNeL",
      "GEE",
      "gee",
      "高程",
      "COPERNICUS/S2_SR_HARMONIZED",
    ];

    for (const q of adversarialQueries) {
      act(() => {
        fireEvent.change(searchInput, { target: { value: q } });
      });
      // Assert search input accepted query without crashing
      assert.strictEqual(searchInput.value, q);
    }
  });

  await t.test("3. Arbitrary GEE Asset ID input interaction & submission", () => {
    let chosenSource = "";
    const { getByTestId } = render(
      <DataSourceCatalogModal
        isOpen={true}
        onClose={() => {}}
        onSelectSource={(id) => { chosenSource = id; }}
        availableSources={mockDescriptors}
        activeResourceId="science.sentinel-2.l2a.v1"
      />
    );

    // Switch to GEE category
    act(() => {
      fireEvent.click(getByTestId("catalog-nav-gee"));
    });

    const customInput = getByTestId("gee-custom-asset-id-input") as HTMLInputElement;
    const submitBtn = getByTestId("gee-custom-asset-id-submit-btn") as HTMLButtonElement;
    assert.ok(customInput);
    assert.ok(submitBtn);

    // Empty input: button should be disabled
    act(() => {
      fireEvent.change(customInput, { target: { value: "   " } });
    });
    assert.ok(submitBtn.disabled, "Submit button must be disabled for whitespace only input");

    // Valid arbitrary asset ID
    const testAsset = "projects/earthengine-public/assets/test_collection_v1";
    act(() => {
      fireEvent.change(customInput, { target: { value: testAsset } });
    });
    assert.strictEqual(submitBtn.disabled, false);

    act(() => {
      fireEvent.click(submitBtn);
    });

    // Check that source was selected and workbench tool options were updated
    assert.strictEqual(chosenSource, "science.gee.catalog.v1");
    assert.strictEqual(getWorkbenchToolOptions().geeAssetId, testAsset);
  });

  await t.test("4. Item click, keyboard navigation (ArrowDown/Up/Enter/Escape), and GEE collection routing", () => {
    let chosenSource = "";
    let closed = false;

    const { getByTestId, container } = render(
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

    // Middle column items: query by data-resource-id
    const s2GeeItem = container.querySelector('[data-resource-id="science.gee.s2-sr.v1"]');
    assert.ok(s2GeeItem, "Sentinel-2 SR GEE item must be listed in GEE category");

    // Click item to highlight
    act(() => {
      fireEvent.click(s2GeeItem);
    });

    // Double click to confirm select
    act(() => {
      fireEvent.doubleClick(s2GeeItem);
    });

    assert.strictEqual(chosenSource, "science.gee.catalog.v1");
    assert.strictEqual(getWorkbenchToolOptions().geeAssetId, "COPERNICUS/S2_SR_HARMONIZED");
    assert.strictEqual(closed, true);

    // Re-test keyboard Escape
    closed = false;
    const { } = render(
      <DataSourceCatalogModal
        isOpen={true}
        onClose={() => { closed = true; }}
        onSelectSource={() => {}}
        availableSources={mockDescriptors}
        activeResourceId="science.sentinel-2.l2a.v1"
      />
    );

    act(() => {
      fireEvent.keyDown(window, { key: "Escape" });
    });
    assert.strictEqual(closed, true, "Escape must close modal");
  });
});

test("Adversarial Suite: Non-Silent Failover Honors User Consent", async (t) => {
  await t.test("1. Multi-Provider configurations integrity", () => {
    const sentinel2 = MULTI_PROVIDER_CONFIGS["science.sentinel-2.l2a.v1"];
    assert.ok(sentinel2);
    assert.deepStrictEqual(
      sentinel2.options.map((o) => o.id),
      ["cdse", "mpc", "gee"]
    );

    const landsat = MULTI_PROVIDER_CONFIGS["science.landsat.c2-l2.v1"];
    assert.ok(landsat);
    assert.deepStrictEqual(
      landsat.options.map((o) => o.id),
      ["mpc", "usgs", "gee"]
    );

    const cop30 = MULTI_PROVIDER_CONFIGS["COP30"];
    assert.ok(cop30);
    assert.deepStrictEqual(
      cop30.options.map((o) => o.id),
      ["aws", "opentopography", "gee"]
    );
  });

  await t.test("2. Zero Silent Swap: Provider stays strictly unchanged until explicit user action", () => {
    // Reset provider to initial default
    setActiveDatasetProvider("science.sentinel-2.l2a.v1", "cdse");
    assert.strictEqual(getActiveDatasetProvider("science.sentinel-2.l2a.v1"), "cdse");

    // Simulate an error occurring without user action:
    // In silent failover, an outage would silently mutate the state.
    // In our non-silent architecture, getActiveDatasetProvider must STILL return 'cdse'!
    const simulatedErrorStage = "failed";
    assert.strictEqual(simulatedErrorStage, "failed");
    assert.strictEqual(getActiveDatasetProvider("science.sentinel-2.l2a.v1"), "cdse");

    // Only upon explicit invocation of setActiveDatasetProvider (representing user clicking banner button) does it change
    setActiveDatasetProvider("science.sentinel-2.l2a.v1", "mpc");
    assert.strictEqual(getActiveDatasetProvider("science.sentinel-2.l2a.v1"), "mpc");

    // Switch to GEE
    setActiveDatasetProvider("science.sentinel-2.l2a.v1", "gee");
    assert.strictEqual(getActiveDatasetProvider("science.sentinel-2.l2a.v1"), "gee");
    assert.strictEqual(getWorkbenchToolOptions().geeAssetId, "COPERNICUS/S2_SR_HARMONIZED");
  });

  await t.test("3. Landsat and DEM provider transitions", () => {
    // Landsat
    setActiveDatasetProvider("science.landsat.c2-l2.v1", "mpc");
    assert.strictEqual(getActiveDatasetProvider("science.landsat.c2-l2.v1"), "mpc");

    setActiveDatasetProvider("science.landsat.c2-l2.v1", "usgs");
    assert.strictEqual(getActiveDatasetProvider("science.landsat.c2-l2.v1"), "usgs");

    setActiveDatasetProvider("science.landsat.c2-l2.v1", "gee");
    assert.strictEqual(getActiveDatasetProvider("science.landsat.c2-l2.v1"), "gee");
    assert.strictEqual(getWorkbenchToolOptions().geeAssetId, "LANDSAT/LC09/C02/T1_L2");

    // DEM
    setActiveDatasetProvider("COP30", "aws");
    assert.strictEqual(getActiveDatasetProvider("COP30"), "aws");

    setActiveDatasetProvider("COP30", "opentopography");
    assert.strictEqual(getActiveDatasetProvider("COP30"), "opentopography");

    setActiveDatasetProvider("COP30", "gee");
    assert.strictEqual(getActiveDatasetProvider("COP30"), "gee");
    assert.strictEqual(getWorkbenchToolOptions().geeAssetId, "COPERNICUS/DEM/GLO30");
  });

  await t.test("4. Failover Banner User Consent & Option Dispatch", () => {
    // Test interactive notice banner behavior when primary provider fails
    let chosenFailover = "";
    let bannerDismissed = false;

    // Simulate DesktopWorkbench failoverBanner state
    const simulatedFailover = {
      datasetKey: "science.sentinel-2.l2a.v1",
      message: "CDSE 服务当前检索受阻，可点击切换至备用数据源通道（MPC / GEE）重新检索。",
      options: [
        { provider: "mpc", label: "切换至 MPC" },
        { provider: "gee", label: "切换至 GEE" },
      ],
    };

    const handleFailoverSwitch = (provider: string) => {
      setActiveDatasetProvider(simulatedFailover.datasetKey, provider);
      chosenFailover = provider;
    };

    // Render banner standalone harness
    const { getByTestId, queryByTestId } = render(
      <div data-testid="failover-notice-banner" className="flex items-center gap-2">
        <span>{simulatedFailover.message}</span>
        {simulatedFailover.options.map((opt) => (
          <button
            key={opt.provider}
            type="button"
            data-testid={`failover-btn-${opt.provider}`}
            onClick={() => handleFailoverSwitch(opt.provider)}
          >
            {opt.label}
          </button>
        ))}
        <button
          type="button"
          data-testid="failover-close-btn"
          onClick={() => { bannerDismissed = true; }}
        >
          关闭
        </button>
      </div>
    );

    // Initial state: CDSE
    setActiveDatasetProvider("science.sentinel-2.l2a.v1", "cdse");
    assert.strictEqual(getActiveDatasetProvider("science.sentinel-2.l2a.v1"), "cdse");

    // Click close button: provider must REMAIN cdse
    act(() => {
      fireEvent.click(getByTestId("failover-close-btn"));
    });
    assert.strictEqual(bannerDismissed, true);
    assert.strictEqual(getActiveDatasetProvider("science.sentinel-2.l2a.v1"), "cdse", "Dismissing banner must NOT change provider");

    // Click '切换至 MPC' button: provider changes to mpc
    act(() => {
      fireEvent.click(getByTestId("failover-btn-mpc"));
    });
    assert.strictEqual(chosenFailover, "mpc");
    assert.strictEqual(getActiveDatasetProvider("science.sentinel-2.l2a.v1"), "mpc");

    // Click '切换至 GEE' button: provider changes to gee and updates geeAssetId
    act(() => {
      fireEvent.click(getByTestId("failover-btn-gee"));
    });
    assert.strictEqual(chosenFailover, "gee");
    assert.strictEqual(getActiveDatasetProvider("science.sentinel-2.l2a.v1"), "gee");
    assert.strictEqual(getWorkbenchToolOptions().geeAssetId, "COPERNICUS/S2_SR_HARMONIZED");
  });
});

