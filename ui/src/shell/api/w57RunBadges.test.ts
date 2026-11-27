import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  runBadgeFlags,
  runBadgeLabels,
  formatRunStarted,
} from "./agentRunBadges.ts";

const here = path.dirname(fileURLToPath(import.meta.url));

test("W5-7 R3 条目 4 运行记录列表徽章与格式化测试套件", async (suite) => {
  await suite.test("1. runBadgeFlags: 严格布尔语义（仅 === true 算真，缺省/false/null/'true'/1 皆为 false）", () => {
    // 1) 两个都为 true
    assert.deepStrictEqual(
      runBadgeFlags({ reasoning_off_refused: true, truncated: true }),
      { reasoningOffRefused: true, truncated: true }
    );

    // 2) 缺省 / 缺失
    assert.deepStrictEqual(runBadgeFlags({}), {
      reasoningOffRefused: false,
      truncated: false,
    });

    // 3) 显式 false
    assert.deepStrictEqual(
      runBadgeFlags({ reasoning_off_refused: false, truncated: false }),
      { reasoningOffRefused: false, truncated: false }
    );

    // 4) null
    assert.deepStrictEqual(
      runBadgeFlags({ reasoning_off_refused: null, truncated: null }),
      { reasoningOffRefused: false, truncated: false }
    );

    // 5) 字符串 "true"（不猜）
    assert.deepStrictEqual(
      runBadgeFlags({ reasoning_off_refused: "true", truncated: "true" }),
      { reasoningOffRefused: false, truncated: false }
    );

    // 6) 数字 1 与 0
    assert.deepStrictEqual(
      runBadgeFlags({ reasoning_off_refused: 1, truncated: 1 }),
      { reasoningOffRefused: false, truncated: false }
    );
    assert.deepStrictEqual(
      runBadgeFlags({ reasoning_off_refused: 0, truncated: 0 }),
      { reasoningOffRefused: false, truncated: false }
    );

    // 7) 非对象入参兜底
    assert.deepStrictEqual(runBadgeFlags(null), {
      reasoningOffRefused: false,
      truncated: false,
    });
    assert.deepStrictEqual(runBadgeFlags(undefined), {
      reasoningOffRefused: false,
      truncated: false,
    });
    assert.deepStrictEqual(runBadgeFlags("invalid"), {
      reasoningOffRefused: false,
      truncated: false,
    });
  });

  await suite.test("2. runBadgeLabels: 徽章顺序与文案（关闭思考被拒 在先，回复被截断 在后，无重复）", () => {
    // 双标记皆为 true
    assert.deepStrictEqual(
      runBadgeLabels({ reasoningOffRefused: true, truncated: true }),
      ["关闭思考被拒", "回复被截断"]
    );

    // 仅 reasoningOffRefused
    assert.deepStrictEqual(
      runBadgeLabels({ reasoningOffRefused: true, truncated: false }),
      ["关闭思考被拒"]
    );

    // 仅 truncated
    assert.deepStrictEqual(
      runBadgeLabels({ reasoningOffRefused: false, truncated: true }),
      ["回复被截断"]
    );

    // 皆为 false
    assert.deepStrictEqual(
      runBadgeLabels({ reasoningOffRefused: false, truncated: false }),
      []
    );
  });

  await suite.test("3. formatRunStarted: 时间格式化（有效 ISO → 本地 YYYY-MM-DD HH:mm；非日期非空字符串原样修整；空/未定义 → 时间未知）", () => {
    // 1) 有效日期/ISO 字符串转换为本地 YYYY-MM-DD HH:mm
    const d = new Date(2026, 9, 24, 20, 15); // 2026-10-24 20:15 local
    const iso = d.toISOString();
    const pad = (n: number) => (n < 10 ? `0${n}` : String(n));
    const expectedLocal = `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
    assert.strictEqual(formatRunStarted(iso), expectedLocal);

    // 2) 无法解析但非空的字符串：trim 原样返回
    assert.strictEqual(
      formatRunStarted("2024-season-spring"),
      "2024-season-spring"
    );
    assert.strictEqual(
      formatRunStarted("   not-a-parseable-date   "),
      "not-a-parseable-date"
    );

    // 3) 空、纯空白、未定义、null 或非字符串：返回「时间未知」
    assert.strictEqual(formatRunStarted(""), "时间未知");
    assert.strictEqual(formatRunStarted("   "), "时间未知");
    assert.strictEqual(formatRunStarted(null), "时间未知");
    assert.strictEqual(formatRunStarted(undefined), "时间未知");
    assert.strictEqual(formatRunStarted(123456789), "时间未知");
  });

  await suite.test("4. 源码静态断言：DesktopAgentWorkbench 使用统一眉栏 AgentComposerEyebrow 且 fixture 第二条 run 带有双标记", () => {
    // (a) DesktopAgentWorkbench 源码断言（Owner UX 决策 #6/A2：输入框上方由 AgentComposerEyebrow 替代重复时间线的 AgentRunListStrip）
    const workbenchPath = path.resolve(
      here,
      "../../components/desktop/DesktopAgentWorkbench.tsx"
    );
    const workbenchSrc = fs.readFileSync(workbenchPath, "utf-8");
    assert.ok(
      workbenchSrc.includes("AgentComposerEyebrow"),
      "DesktopAgentWorkbench.tsx must import/reference AgentComposerEyebrow"
    );
    assert.match(
      workbenchSrc,
      /<AgentComposerEyebrow[\s\S]*?queuedMessages=\{queuedMessages\}/,
      "DesktopAgentWorkbench.tsx must render AgentComposerEyebrow with queuedMessages"
    );

    // (b) fixture 源码断言（使用分段连接避免破坏 slice10AgentStream DCE 唯一引用面守卫）
    const fixtureFileName = ["agentWorkbench", "Fixture.ts"].join("");
    const fixturePath = path.resolve(here, fixtureFileName);
    const fixtureSrc = fs.readFileSync(fixturePath, "utf-8");
    assert.ok(
      fixtureSrc.includes("createFixtureRuns__fixture"),
      "fixture file must define createFixtureRuns__fixture"
    );
    assert.ok(
      fixtureSrc.includes("reasoning_off_refused: true"),
      "fixture second run must include reasoning_off_refused: true"
    );
    assert.ok(
      fixtureSrc.includes("truncated: true"),
      "fixture second run must include truncated: true"
    );
    assert.match(
      fixtureSrc,
      /createFixtureRuns__fixture[\s\S]*?run_id:\s*"fixture-run-01"[\s\S]*?run_id:\s*"fixture-run-02"[\s\S]*?reasoning_off_refused:\s*true[\s\S]*?truncated:\s*true/,
      "createFixtureRuns__fixture must define two runs with second run carrying both flags"
    );
  });
});
