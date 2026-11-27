import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { render, cleanup, fireEvent } from "../../test-utils/render-harness";
import { ModalOverlay } from "../ui/ModalOverlay";
import { DesktopTitlebar } from "./DesktopTitlebar";
import { ModelChannelsSettingsTab } from "../../shell/components/settings/ModelChannelsSettingsTab";
import { formatSignedBboxSummary, formatSignedLat, formatSignedLon } from "@/lib/geoCoords";
import { altShortcut, historyNavShortcut, isMacPlatform, modShortcut } from "@/lib/platformShortcuts";
import { buildSceneSummaryRows } from "@/lib/catalogProductInfo";
import { buildManualConditionTags } from "./manualSearchConditions";

test.afterEach(() => {
  cleanup();
});

test("Audit Remediation 2026-11-18 regression guards", async (suite) => {
  await suite.test("1. geoCoords formats eastern/northern and western/southern coordinates accurately", () => {
    assert.equal(formatSignedLon(117.2272), "117.2272°E");
    assert.equal(formatSignedLon(-73.9857), "73.9857°W");
    assert.equal(formatSignedLat(31.8206), "31.8206°N");
    assert.equal(formatSignedLat(-33.8688), "33.8688°S");
    assert.equal(
      formatSignedBboxSummary({ west: 116.6814, south: 30.9486, east: 117.9688, north: 32.5423 }),
      "116.6814°E · 30.9486°N · 117.9688°E · 32.5423°N",
    );

    const rows = buildSceneSummaryRows({
      scene_id: "LC08_L2SP_120038_20261018_02_T2",
      footprint_bbox: { west: 116.6814, south: 30.9486, east: 117.9688, north: 32.5423, crs: "EPSG:4326" },
    } as never);
    const bboxRow = rows.find((r) => r.label === "WGS84 范围");
    assert.ok(bboxRow);
    assert.match(bboxRow.value, /116\.6814°E ~ 117\.9688°E · 30\.9486°N ~ 32\.5423°N/);
  });

  await suite.test("2. ModalOverlay portals out of .cds-scroll-fade-y into .sp-modern container", () => {
    const view = render(
      <div className="sp-modern" data-testid="modern-root">
        <div className="cds-scroll-fade-y" data-testid="masked-ancestor">
          <ModalOverlay data-testid="portaled-modal">
            <div>预览内容</div>
          </ModalOverlay>
        </div>
      </div>,
    );
    const modernRoot = view.getByTestId("modern-root");
    const maskedAncestor = view.getByTestId("masked-ancestor");
    const modal = view.getByTestId("portaled-modal");
    assert.equal(maskedAncestor.contains(modal), false, "modal must escape .cds-scroll-fade-y");
    assert.equal(modal.parentElement, modernRoot, "modal must portal directly into .sp-modern");
  });

  await suite.test("3. platformShortcuts and DesktopTitlebar adapt between JSDOM/Windows and MacIntel", () => {
    const desc = Object.getOwnPropertyDescriptor(window.navigator, "platform");
    try {
      Object.defineProperty(window.navigator, "platform", { configurable: true, value: "MacIntel" });
      assert.equal(isMacPlatform(), true);
      assert.equal(modShortcut("K"), "⌘K");
      assert.equal(altShortcut("↑"), "⌥↑");
      assert.equal(historyNavShortcut("back"), "⌘[");

      const macView = render(<DesktopTitlebar sidebarExpanded />);
      assert.ok(macView.getByTestId("macos-traffic-close"));
      assert.ok(macView.getByTestId("macos-traffic-minimize"));
      assert.ok(macView.getByTestId("macos-traffic-maximize"));
      cleanup();

      Object.defineProperty(window.navigator, "platform", { configurable: true, value: "Win32" });
      assert.equal(isMacPlatform(), false);
      assert.equal(modShortcut("K"), "Ctrl+K");
      const winView = render(<DesktopTitlebar sidebarExpanded />);
      assert.equal(winView.queryByTestId("macos-traffic-close"), null);
    } finally {
      if (desc) {
        Object.defineProperty(window.navigator, "platform", desc);
      } else {
        Object.defineProperty(window.navigator, "platform", { configurable: true, value: "" });
      }
    }
  });

  await suite.test("4. buildManualConditionTags formats spacecraft array cleanly without truncating raw underscores", () => {
    const tags = buildManualConditionTags({
      request_id: "req-1",
      resource_id: "science.landsat.c2-l2.v1",
      aoi: { type: "Polygon", coordinates: [[[117, 31], [118, 31], [118, 32], [117, 32], [117, 31]]] },
      aoi_name: "合肥市",
      time_range: { start: "2026-08-01", end: "2026-11-01" },
      filters: {
        spacecraft: ["landsat_4", "landsat_5", "landsat_7", "landsat_8", "landsat_9"],
        cloud_cover_lte: 20,
      },
    } as never);
    const platformTag = tags.find((t) => t.id === "optical:platform");
    assert.ok(platformTag);
    assert.equal(platformTag.text, "Landsat 4、Landsat 5 等 5 颗");
    const cloudTag = tags.find((t) => t.id.startsWith("optical:cloud"));
    assert.ok(cloudTag);
    assert.equal(cloudTag.text, "最大云量 ≤ 20%");
  });

  await suite.test("5. ModelChannelsSettingsTab OAuth PKCE provider pills wrap cleanly inside 2-col grid without nowrap overflow", async () => {
    const view = render(<ModelChannelsSettingsTab />);
    fireEvent.click(view.getByTestId("toggle-oauth-pkce-helper"));
    const pkceBtns = view.getAllByTestId(/^pkce-provider-/);
    assert.equal(pkceBtns.length, 4);
    for (const btn of pkceBtns) {
      assert.ok(btn.className.includes("min-w-0"), `expected min-w-0 on ${btn.getAttribute("data-testid")}`);
      assert.ok(btn.className.includes("overflow-hidden"), `expected overflow-hidden on ${btn.getAttribute("data-testid")}`);
      assert.ok(btn.className.includes("!whitespace-normal"), `expected !whitespace-normal on ${btn.getAttribute("data-testid")}`);
    }
  });
});
