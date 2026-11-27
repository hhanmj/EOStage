import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React, { useState } from "react";
import { act } from "react";
import { cleanup, fireEvent, render } from "../../test-utils/render-harness";
import { GlobalCommandPalette } from "./GlobalCommandPalette";
import { BUILTIN_SKILLS } from "../../lib/agentSkills";
import type { AgentSessionSummary } from "../../lib/generated/v3-contracts";
import type { ApplicationTask } from "../../shell/api/types";

test.afterEach(() => {
  cleanup();
});

function makeSession(overrides: Partial<AgentSessionSummary> = {}): AgentSessionSummary {
  return {
    session_id: "sess-1",
    title: "合肥市 Sentinel-1 滑坡筛查",
    created_at: "2026-11-15T00:00:00Z",
    updated_at: "2026-11-15T01:00:00Z",
    run_ids: ["run-1"],
    permission_mode: "confirm_changes",
    pinned_at: null,
    archived_at: null,
    workspace_id: null,
    last_run_status: "failed",
    ...overrides,
  };
}

function makeTask(overrides: Partial<ApplicationTask> = {}): ApplicationTask {
  return {
    task_id: "task-1",
    name: "Copernicus DEM 30m 下载",
    status: "completed",
    provider: "copernicus",
    category: "dem",
    dataset: "COP-DEM GLO-30",
    progress: 100,
    total_files: 4,
    completed_files: 4,
    total_bytes: 1024000,
    completed_bytes: 1024000,
    output_dir: "/data/eostage/dem/hefei",
    created_at: "2026-11-15T00:30:00Z",
    updated_at: "2026-11-15T00:45:00Z",
    ...overrides,
  } as ApplicationTask;
}

test("GlobalCommandPalette toggles on Ctrl/Cmd+K, enforces fixed 16px modal geometry, and closes on Esc", async () => {
  const view = render(<GlobalCommandPalette enableGlobalShortcut />);
  assert.equal(view.queryByTestId("global-command-palette"), null);

  await act(async () => {
    window.dispatchEvent(
      new window.KeyboardEvent("keydown", { key: "k", ctrlKey: true, bubbles: true })
    );
  });

  const dialog = view.getByTestId("global-command-palette");
  assert.ok(dialog.className.includes("w-[min(780px,92vw)]"));
  assert.ok(dialog.className.includes("h-[min(560px,80vh)]"));
  assert.ok(dialog.className.includes("rounded-[16px]"));

  const input = view.getByTestId("command-palette-search-input");
  fireEvent.keyDown(input, { key: "Escape", bubbles: true });
  assert.equal(view.queryByTestId("global-command-palette"), null);

  await act(async () => {
    window.dispatchEvent(
      new window.KeyboardEvent("keydown", { key: "k", metaKey: true, bubbles: true })
    );
  });
  assert.ok(view.getByTestId("global-command-palette"));
});

test("GlobalCommandPalette exposes all 5 tabs, 30+ catalog data sources, workbench tools, sessions, tasks, and local paths", () => {
  const routes: string[] = [];
  const scientificCalls: Array<{ mode: string; preset?: string }> = [];
  const settingsCalls: string[] = [];
  const selectedSources: string[] = [];
  const openedSessions: string[] = [];
  const openedTasks: string[] = [];
  const openedPaths: string[] = [];

  const view = render(
    <GlobalCommandPalette
      open
      sessions={[makeSession()]}
      tasks={[makeTask()]}
      skills={BUILTIN_SKILLS.slice(0, 2)}
      localPaths={[
        {
          id: "art-1",
          title: "合肥 SBAS 相干图成果",
          path: "/data/eostage/results/hefei_coh.tif",
          source: "InSAR 后处理",
        },
      ]}
      onNavigate={(route) => routes.push(route)}
      onOpenScientificMode={(mode, preset) => scientificCalls.push({ mode, preset })}
      onOpenSettingsCategory={(cat) => settingsCalls.push(cat)}
      onSelectDataSource={(rid) => selectedSources.push(rid)}
      onOpenSession={(sid) => openedSessions.push(sid)}
      onOpenTask={(tid) => openedTasks.push(tid)}
      onOpenLocalPath={(path) => openedPaths.push(path)}
    />
  );

  // Check all 5 tabs render
  for (const key of ["all", "workbench", "catalog", "sessions_tasks", "artifacts"]) {
    assert.ok(view.getByTestId(`command-palette-tab-${key}`));
  }

  // Catalog tab includes 30+ data sources + skills
  fireEvent.click(view.getByTestId("command-palette-tab-catalog"));
  const sourceButtons = view
    .getByTestId("command-palette-results")
    .querySelectorAll('[data-testid^="command-palette-item-cmd-source-"]');
  assert.ok(
    sourceButtons.length >= 30,
    `expected >= 30 catalog data sources in palette, got ${sourceButtons.length}`
  );

  // Filter for Sentinel-1 and select it
  const searchInput = view.getByTestId("command-palette-search-input");
  fireEvent.change(searchInput, { target: { value: "Sentinel-1" } });
  const firstS1 = view
    .getByTestId("command-palette-results")
    .querySelector('[data-testid^="command-palette-item-cmd-source-"]') as HTMLButtonElement;
  assert.ok(firstS1, "expected Sentinel-1 data source match");
  fireEvent.click(firstS1);
  assert.equal(selectedSources.length, 1);

  // Workbench tab routes DEM 转换 & 运行环境与工具
  fireEvent.change(searchInput, { target: { value: "DEM 转换" } });
  fireEvent.click(view.getByTestId("command-palette-tab-workbench"));
  fireEvent.click(view.getByTestId("command-palette-item-cmd-dem-convert"));
  assert.deepEqual(scientificCalls[0], { mode: "convert", preset: undefined });

  fireEvent.change(searchInput, { target: { value: "运行环境与工具" } });
  fireEvent.click(view.getByTestId("command-palette-item-cmd-settings-runtime_tools"));
  assert.deepEqual(settingsCalls, ["runtime_tools"]);

  // Sessions & tasks tab
  fireEvent.change(searchInput, { target: { value: "" } });
  fireEvent.click(view.getByTestId("command-palette-tab-sessions_tasks"));
  const sessionItem = view.getByTestId("command-palette-item-cmd-session-sess-1");
  assert.match(sessionItem.textContent ?? "", /执行失败/);
  fireEvent.click(sessionItem);
  assert.deepEqual(openedSessions, ["sess-1"]);

  fireEvent.click(view.getByTestId("command-palette-item-cmd-task-task-1"));
  assert.deepEqual(openedTasks, ["task-1"]);

  // Artifacts tab merges localPaths + task.output_dir
  fireEvent.click(view.getByTestId("command-palette-tab-artifacts"));
  fireEvent.click(view.getByTestId("command-palette-item-cmd-artifact-art-1"));
  fireEvent.click(view.getByTestId("command-palette-item-cmd-artifact-dir-task-1"));
  assert.deepEqual(openedPaths, [
    "/data/eostage/results/hefei_coh.tif",
    "/data/eostage/dem/hefei",
  ]);
  assert.equal(routes.length, 0);
});

test("GlobalCommandPalette supports ArrowUp/ArrowDown/Enter keyboard selection, closes on execute, and shows empty state", () => {
  const triggered: string[] = [];
  function ControlledHarness() {
    const [open, setOpen] = useState(true);
    return (
      <>
        <span data-testid="palette-open-state">{open ? "open" : "closed"}</span>
        <GlobalCommandPalette
          open={open}
          onOpenChange={setOpen}
          onOpenScientificMode={(mode, preset) => triggered.push(`${mode}:${preset ?? ""}`)}
          onNavigate={(route) => triggered.push(`route:${route}`)}
        />
      </>
    );
  }

  const view = render(<ControlledHarness />);
  const input = view.getByTestId("command-palette-search-input");
  fireEvent.click(view.getByTestId("command-palette-tab-workbench"));
  fireEvent.change(input, { target: { value: "DEM" } });

  const items = Array.from(
    view.getByTestId("command-palette-results").querySelectorAll('[role="option"]')
  );
  assert.ok(items.length >= 2, "expected multiple DEM commands");
  assert.equal(items[0].getAttribute("data-active"), "true");

  fireEvent.keyDown(input, { key: "ArrowDown", bubbles: true });
  assert.equal(items[0].getAttribute("data-active"), "false");
  assert.equal(items[1].getAttribute("data-active"), "true");

  fireEvent.keyDown(input, { key: "ArrowUp", bubbles: true });
  assert.equal(items[0].getAttribute("data-active"), "true");

  fireEvent.keyDown(input, { key: "ArrowDown", bubbles: true });
  fireEvent.keyDown(input, { key: "Enter", bubbles: true });

  assert.equal(triggered.length, 1);
  assert.equal(view.getByTestId("palette-open-state").textContent, "closed");
});

test("GlobalCommandPalette renders zero-jitter empty state on unmatched query", () => {
  const view = render(<GlobalCommandPalette open />);
  const input = view.getByTestId("command-palette-search-input");
  fireEvent.change(input, { target: { value: "不存在的指令xyz999" } });
  const empty = view.getByTestId("command-palette-empty");
  assert.match(empty.textContent ?? "", /未找到匹配.*不存在的指令xyz999.*命令/);
  const dialog = view.getByTestId("global-command-palette");
  assert.ok(dialog.className.includes("h-[min(560px,80vh)]"));
});

test("GlobalCommandPalette mounts spring entrance marker, tab glider pill, and stretchy result highlight layer", () => {
  const view = render(<GlobalCommandPalette open />);
  const dialog = view.getByTestId("global-command-palette");
  assert.equal(dialog.getAttribute("data-spring-palette"), "true");
  assert.ok(view.getByTestId("command-palette-tab-highlight"));
  assert.ok(view.getByTestId("command-palette-spring-highlight"));
  const springRows = view.getByTestId("command-palette-results").querySelectorAll("[data-palette-spring-row]");
  assert.ok(springRows.length >= 8);
});
