import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { failureReasonLine } from "./taskCardLogic.ts";
import { assertLocalizedSource } from "./i18nSourceTestUtils.test.ts";
import type { ApplicationTask } from "./types.ts";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const uiDir = path.resolve(__dirname, "../../..");
const uiSrcDir = path.resolve(uiDir, "src");

/**
 * 递归扫描目录下所有指定扩展名的文件
 */
function walkFiles(dir: string, extensions: string[]): string[] {
  const result: string[] = [];
  const entries = fs.readdirSync(dir, { withFileTypes: true });
  for (const entry of entries) {
    const fullPath = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      result.push(...walkFiles(fullPath, extensions));
    } else if (entry.isFile() && extensions.some((ext) => entry.name.endsWith(ext))) {
      result.push(fullPath);
    }
  }
  return result;
}

test("W4 Slice 12-3 - HTML Title ScenePort & Failure Reason Line Suite", async (t) => {
  // --------------------------------------------------------------------------
  // 项 5: HTML / 文档标题测试
  // --------------------------------------------------------------------------
  await t.test("1. ui/index.html 与 ui/tauri.html 的 <title> 恰为 ScenePort", () => {
    const indexPath = path.join(uiDir, "index.html");
    const tauriPath = path.join(uiDir, "tauri.html");

    const indexHtml = fs.readFileSync(indexPath, "utf8");
    const tauriHtml = fs.readFileSync(tauriPath, "utf8");

    const indexMatch = indexHtml.match(/<title>(.*?)<\/title>/i);
    const tauriMatch = tauriHtml.match(/<title>(.*?)<\/title>/i);

    assert.ok(indexMatch, "ui/index.html must contain a <title> tag");
    assert.ok(tauriMatch, "ui/tauri.html must contain a <title> tag");

    assert.strictEqual(
      indexMatch[1].trim(),
      "EOStage",
      "ui/index.html <title> must be exactly EOStage"
    );
    assert.strictEqual(
      tauriMatch[1].trim(),
      "EOStage",
      "ui/tauri.html <title> must be exactly EOStage"
    );
  });

  await t.test("2. ui/src 业务源码断言：全目录无任何 document.title 写入", () => {
    // 递归检索 ui/src 下全部业务源码文件（排除 .test.ts 与 .test.tsx 文件）
    const srcFiles = walkFiles(uiSrcDir, [".ts", ".tsx"]).filter(
      (file) => !file.endsWith(".test.ts") && !file.endsWith(".test.tsx")
    );
    assert.ok(srcFiles.length > 0, "must find source files in ui/src");

    const violations: { file: string; line: number; text: string }[] = [];
    const docTitleAssignRegex = /\bdocument\.title\s*=/;

    for (const file of srcFiles) {
      const content = fs.readFileSync(file, "utf8");
      if (docTitleAssignRegex.test(content)) {
        const lines = content.split("\n");
        lines.forEach((line, idx) => {
          if (docTitleAssignRegex.test(line)) {
            violations.push({
              file: path.relative(uiDir, file),
              line: idx + 1,
              text: line.trim(),
            });
          }
        });
      }
    }

    assert.deepStrictEqual(
      violations,
      [],
      `Found forbidden document.title assignment in ui/src:\n${JSON.stringify(violations, null, 2)}`
    );
  });

  // --------------------------------------------------------------------------
  // 项 6: failureReasonLine 纯函数行为测试
  // --------------------------------------------------------------------------
  await t.test("3. (a) 已知码映射：model_timeout 映射为模型服务响应超时", () => {
    const task: Pick<ApplicationTask, "error"> = {
      error: {
        code: "model_timeout",
        message_zh: "模型服务响应超时",
        retryable: true,
        needs_account: null,
        needs_user_confirmation: false,
      },
    };
    const res = failureReasonLine(task);
    assert.strictEqual(res.primary, "模型服务响应超时");
    assert.strictEqual(res.secondary, null, "message_zh matches primary, secondary should be null");
  });

  await t.test("4. (b) 未知码原样：SOME_FUTURE_UNKNOWN_CODE 原样呈现", () => {
    const task: Pick<ApplicationTask, "error"> = {
      error: {
        code: "SOME_FUTURE_UNKNOWN_CODE",
        message_zh: "",
        retryable: false,
        needs_account: null,
        needs_user_confirmation: false,
      },
    };
    const res = failureReasonLine(task);
    assert.strictEqual(res.primary, "SOME_FUTURE_UNKNOWN_CODE");
    assert.strictEqual(res.secondary, null);
  });

  await t.test("5. (c) 无 error（null / undefined / 空对象 / 空白串）显示「失败原因未返回」", () => {
    assert.deepStrictEqual(failureReasonLine({ error: null }), {
      primary: "失败原因未返回",
      secondary: null,
    });
    assert.deepStrictEqual(failureReasonLine({ error: undefined }), {
      primary: "失败原因未返回",
      secondary: null,
    });
    assert.deepStrictEqual(failureReasonLine({ error: {} as any }), {
      primary: "失败原因未返回",
      secondary: null,
    });
    assert.deepStrictEqual(failureReasonLine({ error: "   " }), {
      primary: "失败原因未返回",
      secondary: null,
    });
    assert.deepStrictEqual(failureReasonLine(null), {
      primary: "失败原因未返回",
      secondary: null,
    });
  });

  await t.test("6. (d) code + message_zh 均存在且不同时 secondary 呈现", () => {
    const task: Pick<ApplicationTask, "error"> = {
      error: {
        code: "model_timeout",
        message_zh: "上游网关连接超时 30 秒",
        retryable: true,
        needs_account: null,
        needs_user_confirmation: false,
      },
    };
    const res = failureReasonLine(task);
    assert.strictEqual(res.primary, "模型服务响应超时");
    assert.strictEqual(res.secondary, "上游网关连接超时 30 秒");
  });

  await t.test("7. (e) 字符串 error 原样呈现", () => {
    const task: Pick<ApplicationTask, "error"> = {
      error: "网络连接重置：ECONNRESET",
    };
    const res = failureReasonLine(task);
    assert.strictEqual(res.primary, "网络连接重置：ECONNRESET");
    assert.strictEqual(res.secondary, null);
  });

  await t.test("8. TypedError 无 code 但 message_zh 非空", () => {
    const task: Pick<ApplicationTask, "error"> = {
      error: {
        code: "",
        message_zh: "文件系统写入权限不足",
        retryable: false,
        needs_account: null,
        needs_user_confirmation: false,
      },
    };
    const res = failureReasonLine(task);
    assert.strictEqual(res.primary, "文件系统写入权限不足");
    assert.strictEqual(res.secondary, null);
  });

  // --------------------------------------------------------------------------
  // 项 6: 源码级断言：TaskDetailView 对 failed 任务恒渲染失败原因行
  // --------------------------------------------------------------------------
  await t.test("9. DesktopDownloadDrawer 源码断言：TaskDetailView 对 failed 任务恒渲染失败原因行", () => {
    const drawerPath = path.join(uiSrcDir, "components/desktop/DesktopDownloadDrawer.tsx");
    const drawerSource = fs.readFileSync(drawerPath, "utf8");

    // 必须调用 failureReasonLine
    assert.match(
      drawerSource,
      /failureReasonLine\(/,
      "DesktopDownloadDrawer must call failureReasonLine in TaskDetailView"
    );

    // The localized detail view keeps the failure-reason label.
    assertLocalizedSource(drawerSource, "downloads.detail.failureReason", "失败原因");

    // TaskDetailView 不得继续使用旧的仅依赖 task.error 的条件渲染：`{task.error && (\s*<div className="pt-2 border-t border-sp-divider">\s*<div[^>]*>错误原因`
    assert.doesNotMatch(
      drawerSource,
      /\{\s*task\.error\s*&&\s*\(\s*<div[^>]*>\s*<div[^>]*>\s*错误原因/,
      "DesktopDownloadDrawer must not conditionally render error section based solely on task.error"
    );
  });
});
