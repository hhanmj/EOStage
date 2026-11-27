import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { popoverPlacement, DESKTOP_TITLEBAR_HEIGHT } from "./agentPermissionLogic.ts";
import { projectErrorCard } from "./approvalLogic.ts";
import { createStartupEffect, type StartupLoadDependencies } from "./startupLoad.ts";
import * as fixtureApi from "./fixture.ts";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const SNAKE = /[a-z]+_[a-z_]+/;

function baseDeps(overrides: Partial<StartupLoadDependencies>): StartupLoadDependencies {
  return {
    listSources: async () => [],
    getSourceViews: async () => [],
    getBatchPage: async () => ({ scenes: [], rows: [] }),
    getWorkingAoi: async () => null,
    getCredentialStatus: async () => null,
    getSarDownloadPolicySettings: async () => null,
    getDemTaskSettings: async () => null,
    isDesktopHost: () => false,
    setAvailableSources: () => {},
    setSourceViews: () => {},
    setScenes: () => {},
    setRowStates: () => {},
    setWorkingGeometry: () => {},
    setWorkingAoiState: () => {},
    setAoiLabel: () => {},
    setCreds: () => {},
    setStorageRootPath: () => {},
    setSarDownloadPolicy: () => {},
    setDemConcurrency: () => {},
    ...overrides,
  };
}

const tick = () => new Promise((r) => setTimeout(r, 0));

test("W5-2f R7 返工（agnes）", async (t) => {
  await t.test("1. popoverPlacement margin：向上展开时弹层顶边离标题栏至少 8px（maxHeight = 上方空间 − 间隙 8 − 边距 8）", () => {
    const up = popoverPlacement({
      triggerTop: 400,
      triggerBottom: 432,
      viewportHeight: 600,
      desiredHeight: 500,
      topInset: DESKTOP_TITLEBAR_HEIGHT,
      margin: 8,
    });
    assert.strictEqual(up.side, "up");
    assert.strictEqual(up.maxHeight, 400 - 40 - 8 - 8);
    // 弹层底边在触发按钮上方 8px，顶边 = 400 - 8 - maxHeight = 48 = 标题栏 40 + 8
    assert.strictEqual(400 - 8 - up.maxHeight, DESKTOP_TITLEBAR_HEIGHT + 8);

    const down = popoverPlacement({
      triggerTop: 100,
      triggerBottom: 132,
      viewportHeight: 1000,
      desiredHeight: 300,
      topInset: DESKTOP_TITLEBAR_HEIGHT,
      margin: 8,
    });
    assert.strictEqual(down.side, "down");
    assert.strictEqual(down.maxHeight, 1000 - 132 - 8 - 8);
  });

  await t.test("2. projectErrorCard：禁止档、旧实际超额事件、记录失败与未知码的可见文字都不含 snake_case，原码只进详情", () => {
    const forbidden = projectErrorCard({
      code: "plan_refused_forbidden",
      status: null,
      approval: { decision: "refused_forbidden", mode: "full_access", card_reasons: [] },
    });
    assert.strictEqual(forbidden.wording, "该操作属于禁止档，任何模式都不执行");
    assert.strictEqual(forbidden.modeLine, "当前模式：完全访问");
    assert.strictEqual(forbidden.detailCode, "plan_refused_forbidden");

    const quota = projectErrorCard({
      code: "quota_exceeded_actual",
      status: null,
      approval: { decision: "auto_by_mode", mode: "within_quota", exceeded: ["max_bytes_actual"] },
    });
    assert.strictEqual(quota.wording, "实际下载量超过本次运行字节上限的 110%，Agent 建的任务已暂停");
    assert.strictEqual(quota.detailCode, "quota_exceeded_actual");

    const recordFailed = projectErrorCard({ code: "record_failed", status: "failed", approval: null });
    assert.strictEqual(recordFailed.wording, "运行记录写入失败，本次运行已终止");
    assert.strictEqual(recordFailed.detailCode, "record_failed");

    const unknown = projectErrorCard({ code: "some_unknown_code", status: null, approval: null });
    assert.strictEqual(unknown.wording, "执行出错");
    assert.strictEqual(unknown.detailCode, "some_unknown_code");

    for (const p of [forbidden, quota, recordFailed, unknown]) {
      const visible = [p.wording, p.modeLine ?? ""].join(" ");
      assert.ok(!SNAKE.test(visible), `可见文字含 snake_case：${visible}`);
    }

    const cancelled = projectErrorCard({ code: "cancelled", status: "cancelled", approval: null });
    assert.ok(!SNAKE.test(cancelled.wording));
    assert.strictEqual(cancelled.detailCode, null);
  });

  await t.test("3. 输出目录预填来源：非桌面演示模式由集中 fixture 提供存储根；没有时不调用；桌面宿主不读 fixture", async () => {
    const set1: string[] = [];
    createStartupEffect(
      baseDeps({ getFixtureStorageRoot: () => "D:\\ScenePort\\Downloads", setStorageRootPath: (p) => set1.push(p) })
    )();
    await tick();
    assert.deepStrictEqual(set1, ["D:\\ScenePort\\Downloads"]);

    const set2: string[] = [];
    createStartupEffect(baseDeps({ getFixtureStorageRoot: () => null, setStorageRootPath: (p) => set2.push(p) }))();
    await tick();
    assert.deepStrictEqual(set2, []);

    const set3: string[] = [];
    createStartupEffect(
      baseDeps({
        isDesktopHost: () => true,
        getFixtureStorageRoot: () => "D:\\ShouldNotBeUsed",
        setStorageRootPath: (p) => set3.push(p),
      })
    )();
    await tick();
    await tick();
    assert.deepStrictEqual(set3, [], "桌面宿主只用 getSarDownloadPolicySettings 的真实存储根");
  });

  await t.test("4. 演示存储根只在集中 fixture 模块，生产实现返回 null，DesktopWorkbench 经门控的 index 导出传入", () => {
    assert.strictEqual(typeof (fixtureApi as any).getFixtureStorageRoot, "function");
    const root = (fixtureApi as any).getFixtureStorageRoot();
    assert.ok(typeof root === "string" && /^[A-Za-z]:\\/.test(root), "演示存储根必须是 Windows 绝对路径");
    const prod = fs.readFileSync(path.resolve(HERE, "production.ts"), "utf-8");
    assert.match(prod, /export function getFixtureStorageRoot\(\)[^{]*\{\s*return null;/);
    const index = fs.readFileSync(path.resolve(HERE, "index.ts"), "utf-8");
    assert.match(index, /getFixtureStorageRoot = \(\) =>\s*isFixtureBackend\(\) && fixture \? fixture\.getFixtureStorageRoot\(\) : prod\.getFixtureStorageRoot\(\)/);
    const wb = fs.readFileSync(path.resolve(HERE, "../../components/desktop/DesktopWorkbench.tsx"), "utf-8");
    assert.match(wb, /getFixtureStorageRoot,/);
  });
});
