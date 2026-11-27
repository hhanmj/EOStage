import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

import { formatOrbitTerms, ORBIT_LEGEND_LABELS_EN } from "./orbitTerms.ts";
import { projectConfirmCard } from "./agentConversationLogic.ts";
import { groupSessionsByDay } from "./agentHistoryLogic.ts";
import type { StreamConfirmCardItem, AgentSessionSummary } from "../../lib/generated/v3-contracts.ts";

test("W5-2 P0-1 Specification: Orbit Terms in English in the New Shell", async (t) => {
  await t.test("formatOrbitTerms: behaviour test cases", () => {
    assert.strictEqual(formatOrbitTerms("SLC · 降轨"), "SLC · Descending");
    assert.strictEqual(formatOrbitTerms("SLC · 升轨 (IW)"), "SLC · Ascending (IW)");
    assert.strictEqual(formatOrbitTerms("升降轨都要"), "Ascending/Descending都要");
    assert.strictEqual(
      formatOrbitTerms("ALOS-2 PALSAR-2 升轨数据检索"),
      "ALOS-2 PALSAR-2 Ascending数据检索"
    );
    assert.strictEqual(formatOrbitTerms("DESCENDING"), "Descending");
    assert.strictEqual(formatOrbitTerms("大小未知"), "大小未知");
    assert.strictEqual(formatOrbitTerms(""), "");
  });

  await t.test("projectConfirmCard: maps orbit terms in conditionBadges", () => {
    const item: StreamConfirmCardItem = {
      kind: "confirm_card",
      seq: 1,
      run_id: "run-test-orbit",
      plan_id: "plan-orbit-01",
      plan_hash: "hash-orbit-01",
      snapshot: {
        source: "Sentinel-1",
        range: {
          aoi_name: "大同盆地",
          area_km2: 120,
          time_range: { start: "2024-04-01", end: "2024-04-30" },
          product: "SLC · 降轨",
        },
        estimated_bytes: 1024,
        output_dir: "/data",
      },
    };

    const projected = projectConfirmCard(item, null);
    assert.ok(
      projected.conditionBadges.includes("SLC · Descending"),
      "conditionBadges must include 'SLC · Descending'"
    );
    const hasChineseDesc = projected.conditionBadges.some((b) => b.includes("降轨"));
    assert.strictEqual(hasChineseDesc, false, "conditionBadges must not contain '降轨'");
  });

  await t.test("groupSessionsByDay: preserves original orbit terms in history item title (W5-8 L4 P2-7)", () => {
    const now = new Date("2026-10-25T12:00:00Z");
    const session: AgentSessionSummary = {
      session_id: "sess-orbit-01",
      title: "台湾花莲 ALOS-2 升轨数据检索",
      created_at: "2026-10-25T10:00:00Z",
      updated_at: "2026-10-25T10:00:00Z",
      run_ids: [],
    };

    const groups = groupSessionsByDay([session], now);
    assert.strictEqual(groups.length, 1);
    assert.strictEqual(
      groups[0].items[0].title,
      "台湾花莲 ALOS-2 升轨数据检索"
    );
  });

  await t.test("source assertions for WorkbenchMap.tsx, DesktopAgentWorkbench.tsx, DesktopWorkbench.tsx and Workbench.tsx", () => {
    const workbenchMapContent = fs.readFileSync(
      path.resolve(__dirname, "../../components/WorkbenchMap.tsx"),
      "utf-8"
    );
    const desktopAgentWorkbenchContent = fs.readFileSync(
      path.resolve(__dirname, "../../components/desktop/DesktopAgentWorkbench.tsx"),
      "utf-8"
    );
    const desktopWorkbenchContent = fs.readFileSync(
      path.resolve(__dirname, "../../components/desktop/DesktopWorkbench.tsx"),
      "utf-8"
    );

    assert.match(
      workbenchMapContent,
      /\borbitLabels\b/,
      "WorkbenchMap.tsx must declare and use optional orbitLabels prop"
    );
    assert.match(
      desktopAgentWorkbenchContent,
      /ORBIT_LEGEND_LABELS_EN/,
      "DesktopAgentWorkbench.tsx must pass ORBIT_LEGEND_LABELS_EN"
    );
    assert.match(
      desktopWorkbenchContent,
      /ORBIT_LEGEND_LABELS_EN/,
      "DesktopWorkbench.tsx must pass ORBIT_LEGEND_LABELS_EN"
    );
  });
});
