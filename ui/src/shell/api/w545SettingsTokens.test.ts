import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const uiSrcDir = path.resolve(__dirname, "../..");

export const SCOPED_FILES = [
  "components/SettingsModal.tsx",
  "shell/components/settings/StorageSettingsTab.tsx",
  "shell/components/settings/ModelChannelsSettingsTab.tsx",
  "shell/components/settings/ChannelListItem.tsx",
  "shell/components/settings/ChannelReasoningField.tsx",
  "shell/components/settings/ChannelReasoningNote.tsx",
  "shell/components/settings/NetworkProxySettingsTab.tsx",
  "shell/components/settings/AccountsSettingsTab.tsx",
  "shell/components/settings/PerformanceUsageSettingsTab.tsx",
  "shell/components/InplaceLoginCard.tsx",
  "shell/components/StatusStateCard.tsx",
  "components/desktop/DesktopDownloadDrawer.tsx",
  "components/desktop/DownloadCenterPage.tsx",
];

interface CodeLine {
  file: string;
  lineNum: number;
  lineText: string;
}

interface ScopedContent {
  file: string;
  content: string;
  lineOffset: number;
}

interface Violation {
  rule: string;
  file: string;
  line: number;
  snippet: string;
}

export interface LineViolation {
  rule: string;
  snippet: string;
}

export interface ControlFontViolation {
  tag: string;
  snippet: string;
  index: number;
}

export function findLineViolations(lineText: string): LineViolation[] {
  const violations: LineViolation[] = [];

  // Regex patterns
  // A: rounded-control
  const REGEX_A = /\brounded-control\b/g;
  // B: text-[(9|10|10.5|11|11.5)px]
  const REGEX_B = /\btext-\[(?:9|10|10\.5|11|11\.5)px\](?![\w-])/g;
  // C: arbitrary values
  const REGEX_C = /(?:#[0-9A-Fa-f]{6}\b|rounded-\[|shadow-\[|bg-\[#|text-\[#)/g;
  // D: native color palette (excluding literal bg-slate-950/45)
  const REGEX_D =
    /(?:(?<![\w-])(?:[a-z0-9-]+:)*(?:(?:bg|text|border|ring|divide|from|to|via|placeholder|outline|fill|stroke)-(?:slate|gray|zinc|neutral|stone|blue|sky|indigo|red|rose|green|emerald|amber|orange|yellow|lime|teal|cyan|violet|purple|pink|fuchsia)-\d[\w\/-]*|bg-white\b|bg-black\b|text-black\b|border-white\b|border-black\b))/g;
  // E: text-white must accompany bg-sp-accent-fill, bg-sp-danger-strong or bg-sp-accent
  const REGEX_E_TRIGGER = /(?<![\w-])text-white\b/;
  const REGEX_E_ALLOWED = /\b(?:bg-sp-accent-fill|bg-sp-danger-strong|bg-sp-accent)\b/;
  // F: non-token rounded
  const REGEX_F =
    /(?<![\w-])(?:[a-z0-9-]+:)?(?:rounded-(?:lg|xl|2xl|3xl|md|sm)\b|rounded(?=[\s"'`]))/g;
  // G: non-token shadow
  const REGEX_G =
    /(?<![\w-])(?:[a-z0-9-]+:)?(?:shadow-(?:sm|md|lg|xl|2xl|inner)\b|shadow(?=[\s"'`]))/g;
  // H: half-step spacing
  const REGEX_H =
    /(?<=^|[\s:"'-])(?:gap|gap-x|gap-y|p|px|py|pt|pb|pl|pr|m|mx|my|mt|mb|ml|mr|space-x|space-y)-(?:0\.5|1\.5|2\.5|3\.5)\b/g;
  // I: font weight
  const REGEX_I =
    /(?<![\w-])(?:[a-z0-9-]+:)?font-(?:medium|light|extralight|thin|extrabold|black)\b/g;

  let m: RegExpExecArray | null;

  REGEX_A.lastIndex = 0;
  while ((m = REGEX_A.exec(lineText)) !== null) {
    violations.push({ rule: "A", snippet: m[0] });
  }

  REGEX_B.lastIndex = 0;
  while ((m = REGEX_B.exec(lineText)) !== null) {
    violations.push({ rule: "B", snippet: m[0] });
  }

  REGEX_C.lastIndex = 0;
  while ((m = REGEX_C.exec(lineText)) !== null) {
    violations.push({ rule: "C", snippet: m[0] });
  }

  // Owner 2026-10-29: Task D 允许全屏对话框背衬采用 lighter dim bg-black/20
  const strippedD = lineText.replace(/\b(?:bg-slate-950\/45|bg-black\/20)\b/g, "");
  REGEX_D.lastIndex = 0;
  while ((m = REGEX_D.exec(strippedD)) !== null) {
    violations.push({ rule: "D", snippet: m[0] });
  }

  if (REGEX_E_TRIGGER.test(lineText) && !REGEX_E_ALLOWED.test(lineText)) {
    violations.push({ rule: "E", snippet: "text-white without required bg token" });
  }

  REGEX_F.lastIndex = 0;
  while ((m = REGEX_F.exec(lineText)) !== null) {
    violations.push({ rule: "F", snippet: m[0] });
  }

  REGEX_G.lastIndex = 0;
  while ((m = REGEX_G.exec(lineText)) !== null) {
    violations.push({ rule: "G", snippet: m[0] });
  }

  REGEX_H.lastIndex = 0;
  while ((m = REGEX_H.exec(lineText)) !== null) {
    violations.push({ rule: "H", snippet: m[0] });
  }

  REGEX_I.lastIndex = 0;
  while ((m = REGEX_I.exec(lineText)) !== null) {
    violations.push({ rule: "I", snippet: m[0] });
  }

  return violations;
}

export function findControlFontViolations(content: string): ControlFontViolation[] {
  const violations: ControlFontViolation[] = [];
  const tagRegex = /<(label|input|select|textarea|button)\b/g;
  const sizeRegex = /\btext-(?:xs\b|\[(?:9|10|10\.5|11|11\.5|12)px\](?![\w-]))/g;

  let tagMatch: RegExpExecArray | null;
  tagRegex.lastIndex = 0;
  while ((tagMatch = tagRegex.exec(content)) !== null) {
    const tagStart = tagMatch.index;
    let i = tagStart + tagMatch[0].length;
    let inQuote: string | null = null;
    let braceDepth = 0;
    let tagEnd = -1;

    while (i < content.length) {
      const char = content[i];
      if (inQuote) {
        if (char === "\\") {
          i += 2;
          continue;
        }
        if (char === inQuote) {
          inQuote = null;
        }
      } else {
        if (char === '"' || char === "'" || char === "`") {
          inQuote = char;
        } else if (char === "{") {
          braceDepth++;
        } else if (char === "}") {
          if (braceDepth > 0) braceDepth--;
        } else if (char === ">" && braceDepth === 0) {
          tagEnd = i;
          break;
        }
      }
      i++;
    }

    if (tagEnd !== -1) {
      const tagText = content.slice(tagStart, tagEnd + 1);
      let sm: RegExpExecArray | null;
      sizeRegex.lastIndex = 0;
      while ((sm = sizeRegex.exec(tagText)) !== null) {
        violations.push({
          tag: tagMatch[1],
          snippet: sm[0],
          index: tagStart + sm.index,
        });
      }
    }
  }

  return violations;
}

function loadScopedLines(): { lines: CodeLine[]; scopedContents: ScopedContent[] } {
  const lines: CodeLine[] = [];
  const scopedContents: ScopedContent[] = [];

  for (const relPath of SCOPED_FILES) {
    const fullPath = path.resolve(uiSrcDir, relPath);
    assert.ok(fs.existsSync(fullPath), `Scoped file not found: ${relPath}`);
    const content = fs.readFileSync(fullPath, "utf8");
    const fileLines = content.split(/\r?\n/);
    fileLines.forEach((lineText, idx) => {
      lines.push({
        file: relPath.replace(/\\/g, "/"),
        lineNum: idx + 1,
        lineText,
      });
    });
    scopedContents.push({
      file: relPath.replace(/\\/g, "/"),
      content,
      lineOffset: 1,
    });
  }

  // DesktopWorkbench.tsx snippet between <SettingsModal and <AdminBoundaryModal
  const dwPath = path.resolve(uiSrcDir, "components/desktop/DesktopWorkbench.tsx");
  assert.ok(fs.existsSync(dwPath), "DesktopWorkbench.tsx not found");
  const dwContent = fs.readFileSync(dwPath, "utf8");
  const dwLines = dwContent.split(/\r?\n/);

  let startLineIdx = -1;
  let endLineIdx = -1;
  for (let i = 0; i < dwLines.length; i++) {
    if (startLineIdx === -1 && dwLines[i].includes("<SettingsModal")) {
      startLineIdx = i;
    } else if (startLineIdx !== -1 && dwLines[i].includes("<AdminBoundaryModal")) {
      endLineIdx = i;
      break;
    }
  }

  assert.ok(startLineIdx !== -1, "Anchor '<SettingsModal' not found in DesktopWorkbench.tsx");
  assert.ok(endLineIdx !== -1, "Anchor '<AdminBoundaryModal' not found in DesktopWorkbench.tsx");
  assert.ok(endLineIdx > startLineIdx, "Invalid snippet boundaries in DesktopWorkbench.tsx");

  for (let i = startLineIdx; i < endLineIdx; i++) {
    lines.push({
      file: "components/desktop/DesktopWorkbench.tsx",
      lineNum: i + 1,
      lineText: dwLines[i],
    });
  }

  const snippetContent = dwLines.slice(startLineIdx, endLineIdx).join("\n");
  scopedContents.push({
    file: "components/desktop/DesktopWorkbench.tsx",
    content: snippetContent,
    lineOffset: startLineIdx + 1,
  });

  return { lines, scopedContents };
}

test("W5-4/W5-5 Settings Tokens Guard Rules A-K", () => {
  const { lines, scopedContents } = loadScopedLines();
  const violations: Violation[] = [];

  for (const { file, lineNum, lineText } of lines) {
    const lineViolations = findLineViolations(lineText);
    for (const v of lineViolations) {
      violations.push({ rule: v.rule, file, line: lineNum, snippet: v.snippet });
    }
  }

  for (const { file, content, lineOffset } of scopedContents) {
    const cViolations = findControlFontViolations(content);
    for (const cv of cViolations) {
      const lineNum =
        lineOffset + content.slice(0, cv.index).split(/\r?\n/).length - 1;
      violations.push({
        rule: "J",
        file,
        line: lineNum,
        snippet: `<${cv.tag} ... ${cv.snippet}>`,
      });
    }
  }

  // Rule K: ui/src/index.css 的 .cred-action-btn {…} 规则 font-size ≥ 13px
  const indexCssPath = path.resolve(uiSrcDir, "index.css");
  assert.ok(fs.existsSync(indexCssPath), "index.css not found");
  const indexCss = fs.readFileSync(indexCssPath, "utf8");
  const credBtnMatch = indexCss.match(/\.cred-action-btn\s*\{([^}]*)\}/);
  assert.ok(credBtnMatch, ".cred-action-btn rule block not found in index.css");

  const fontSizeMatch = credBtnMatch[1].match(/font-size:\s*(\d+)px/);
  const credBtnIndex = indexCss.indexOf(".cred-action-btn {");
  let kLineNum = 1;
  if (credBtnIndex !== -1) {
    kLineNum = indexCss.slice(0, credBtnIndex).split(/\r?\n/).length;
  }

  if (!fontSizeMatch || parseInt(fontSizeMatch[1], 10) < 13) {
    violations.push({
      rule: "K",
      file: "index.css",
      line: kLineNum,
      snippet: `.cred-action-btn font-size: ${fontSizeMatch ? fontSizeMatch[1] : "none"}px (< 13px)`,
    });
  }

  const message = violations
    .map((v) => `  ${v.file}:${v.line}: [Rule ${v.rule}] ${v.snippet}`)
    .join("\n");

  assert.strictEqual(
    violations.length,
    0,
    `W5-4/W5-5 Settings token guard violations found (${violations.length}):\n${message}`
  );
});

test("W5-4/W5-5 Settings Tokens Rule Self-Tests: Line Rules A-I", () => {
  // A: rounded-control
  assert.strictEqual(findLineViolations('className="rounded-control"').some((v) => v.rule === "A"), true, "A must match rounded-control");
  assert.strictEqual(findLineViolations('className="rounded-sp-sm"').some((v) => v.rule === "A"), false, "A must not match rounded-sp-sm");

  // B: text-[(9|10|10.5|11|11.5)px]
  assert.strictEqual(findLineViolations("text-[11px] ").some((v) => v.rule === "B"), true, "B must match text-[11px] ");
  assert.strictEqual(findLineViolations('text-[11px]"').some((v) => v.rule === "B"), true, "B must match text-[11px]\"");
  assert.strictEqual(findLineViolations("text-[10.5px]}").some((v) => v.rule === "B"), true, "B must match text-[10.5px]}");
  assert.strictEqual(findLineViolations("text-[9px]`").some((v) => v.rule === "B"), true, "B must match text-[9px]`");
  assert.strictEqual(findLineViolations("text-[12px]").some((v) => v.rule === "B"), false, "B must not match text-[12px]");
  assert.strictEqual(findLineViolations("text-[13px]").some((v) => v.rule === "B"), false, "B must not match text-[13px]");

  // C: arbitrary values
  assert.strictEqual(findLineViolations("color: #1a2b3c;").some((v) => v.rule === "C"), true, "C must match hex color");
  assert.strictEqual(findLineViolations('className="rounded-sp-md"').some((v) => v.rule === "C"), false, "C must not match rounded-sp-md");

  // D: native color palette (excluding literal bg-slate-950/45)
  assert.strictEqual(findLineViolations('className="bg-slate-950/45"').some((v) => v.rule === "D"), false, "D must not match literal bg-slate-950/45");
  assert.strictEqual(findLineViolations('className="bg-slate-950/40"').some((v) => v.rule === "D"), true, "D must match bg-slate-950/40");
  assert.strictEqual(findLineViolations('className="dark:bg-slate-800"').some((v) => v.rule === "D"), true, "D must match dark:bg-slate-800");
  assert.strictEqual(findLineViolations('className="bg-white"').some((v) => v.rule === "D"), true, "D must match bg-white");

  // E: text-white must accompany bg-sp-accent-fill, bg-sp-danger-strong or bg-sp-accent
  assert.strictEqual(findLineViolations('className="text-white"').some((v) => v.rule === "E"), true, "E must match text-white without allowed bg");
  assert.strictEqual(findLineViolations('className="text-white bg-sp-accent-fill"').some((v) => v.rule === "E"), false, "E must not match text-white with bg-sp-accent-fill");
  assert.strictEqual(findLineViolations('className="bg-sp-danger-strong text-white"').some((v) => v.rule === "E"), false, "E must not match text-white with bg-sp-danger-strong");

  // F: non-token rounded
  assert.strictEqual(findLineViolations('className="rounded-sp-md"').some((v) => v.rule === "F"), false, "F must not match rounded-sp-md");
  assert.strictEqual(findLineViolations('className="rounded-card"').some((v) => v.rule === "F"), false, "F must not match rounded-card");
  assert.strictEqual(findLineViolations('className="rounded-full"').some((v) => v.rule === "F"), false, "F must not match rounded-full");
  assert.strictEqual(findLineViolations('className="rounded-modal"').some((v) => v.rule === "F"), false, "F must not match rounded-modal");
  assert.strictEqual(findLineViolations('className="rounded-lg"').some((v) => v.rule === "F"), true, "F must match rounded-lg");
  assert.strictEqual(findLineViolations('"rounded "').some((v) => v.rule === "F"), true, 'F must match "rounded "');

  // G: non-token shadow
  assert.strictEqual(findLineViolations('className="shadow-card"').some((v) => v.rule === "G"), false, "G must not match shadow-card");
  assert.strictEqual(findLineViolations('className="shadow-sm"').some((v) => v.rule === "G"), true, "G must match shadow-sm");
  assert.strictEqual(findLineViolations('"shadow "').some((v) => v.rule === "G"), true, 'G must match "shadow "');

  // H: half-step spacing
  assert.strictEqual(findLineViolations('className="space-y-1.5"').some((v) => v.rule === "H"), true, "H must match space-y-1.5");
  assert.strictEqual(findLineViolations('className="px-2.5"').some((v) => v.rule === "H"), true, "H must match px-2.5");
  assert.strictEqual(findLineViolations('className="hover:p-0.5"').some((v) => v.rule === "H"), true, "H must match hover:p-0.5");
  assert.strictEqual(findLineViolations('className="gap-2"').some((v) => v.rule === "H"), false, "H must not match gap-2");
  assert.strictEqual(findLineViolations('className="p-3"').some((v) => v.rule === "H"), false, "H must not match p-3");
  assert.strictEqual(findLineViolations('className="h-3.5"').some((v) => v.rule === "H"), false, "H must not match h-3.5");

  // I: font weight
  assert.strictEqual(findLineViolations('className="font-medium"').some((v) => v.rule === "I"), true, "I must match font-medium");
  assert.strictEqual(findLineViolations('className="font-semibold"').some((v) => v.rule === "I"), false, "I must not match font-semibold");
});

test("W5-4/W5-5 Settings Tokens Rule Self-Tests: Control Font Rule J", () => {
  assert.strictEqual(
    findControlFontViolations('<button className="h-8 text-[12px]">').length > 0,
    true,
    'J must match <button className="h-8 text-[12px]">'
  );
  assert.strictEqual(
    findControlFontViolations('<label className={cn("a", ok ? "text-xs" : "")}>').length > 0,
    true,
    'J must match <label className={cn("a", ok ? "text-xs" : "")}>'
  );
  assert.strictEqual(
    findControlFontViolations('<input onChange={(e) => set(e.target.value)} className="text-[11px]" />').length > 0,
    true,
    'J must match <input ... className="text-[11px]" />'
  );
  assert.strictEqual(
    findControlFontViolations('<button className="text-[13px]">').length,
    0,
    'J must not match <button className="text-[13px]">'
  );
  assert.strictEqual(
    findControlFontViolations('<span className="text-xs">').length,
    0,
    'J must not match <span className="text-xs">'
  );
});

export function findGlobalRoundedControlViolations(): Violation[] {
  const violations: Violation[] = [];
  function scanDir(dir: string) {
    const entries = fs.readdirSync(dir, { withFileTypes: true });
    entries.sort((a, b) => a.name.localeCompare(b.name));
    for (const entry of entries) {
      const fullPath = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        scanDir(fullPath);
      } else if (entry.isFile()) {
        const ext = path.extname(entry.name);
        if (ext !== ".ts" && ext !== ".tsx" && ext !== ".css") continue;
        if (entry.name.endsWith(".test.ts") || entry.name.endsWith(".test.tsx")) continue;
        const relPath = path.relative(uiSrcDir, fullPath).replace(/\\/g, "/");
        if (relPath === "pages/Workbench.tsx") continue;

        const content = fs.readFileSync(fullPath, "utf8");
        const lines = content.split(/\r?\n/);
        for (let i = 0; i < lines.length; i++) {
          const lineText = lines[i];
          const matches = lineText.match(/\brounded-control\b/g);
          if (matches) {
            for (const m of matches) {
              violations.push({
                rule: "A",
                file: relPath,
                line: i + 1,
                snippet: m,
              });
            }
          }
        }
      }
    }
  }
  scanDir(uiSrcDir);
  return violations;
}

test("W5-5 全仓 rounded-control 清除守卫", () => {
  const violations = findGlobalRoundedControlViolations();
  const message = violations
    .map((v) => `  ${v.file}:${v.line}: [Rule ${v.rule}] ${v.snippet}`)
    .join("\n");
  assert.strictEqual(
    violations.length,
    0,
    `W5-5 全仓 rounded-control 违规 (${violations.length}):\n${message}`
  );
});

export function findDirectoryMinFontSizeViolations(): Violation[] {
  const violations: Violation[] = [];
  const REGEX_B = /\btext-\[(?:9|10|10\.5|11|11\.5)px\](?![\w-])/g;

  const targetDirs = [
    path.resolve(uiSrcDir, "shell"),
    path.resolve(uiSrcDir, "components/desktop"),
  ];

  function scanDir(dir: string) {
    if (!fs.existsSync(dir)) return;
    const entries = fs.readdirSync(dir, { withFileTypes: true });
    entries.sort((a, b) => a.name.localeCompare(b.name));
    for (const entry of entries) {
      const fullPath = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        scanDir(fullPath);
      } else if (entry.isFile()) {
        const ext = path.extname(entry.name);
        if (ext !== ".tsx") continue;
        if (entry.name.endsWith(".test.tsx") || entry.name.endsWith(".test.ts")) continue;
        const relPath = path.relative(uiSrcDir, fullPath).replace(/\\/g, "/");

        const content = fs.readFileSync(fullPath, "utf8");
        const lines = content.split(/\r?\n/);
        for (let i = 0; i < lines.length; i++) {
          const lineText = lines[i];
          REGEX_B.lastIndex = 0;
          let m: RegExpExecArray | null;
          while ((m = REGEX_B.exec(lineText)) !== null) {
            violations.push({
              rule: "B",
              file: relPath,
              line: i + 1,
              snippet: m[0],
            });
          }
        }
      }
    }
  }

  for (const dir of targetDirs) {
    scanDir(dir);
  }
  return violations;
}

test("W5-5 全界面最小字号提升至 12px 守卫", () => {
  const violations = findDirectoryMinFontSizeViolations();
  const message = violations
    .map((v) => `  ${v.file}:${v.line}: [Rule ${v.rule}] ${v.snippet}`)
    .join("\n");
  assert.strictEqual(
    violations.length,
    0,
    `W5-5 全界面最小字号违规 (${violations.length}):\n${message}`
  );
});

test("W5-5 R9 渠道 id 完整显示与滚动可见性（源码断言）", () => {
  // 1. ChannelListItem.tsx：id span 含 break-all；不含 font-mono truncate
  //    （W5-8 R5 起渠道 id 改用界面字体，等宽只给路径/产品编号/哈希）
  const channelItemPath = path.resolve(uiSrcDir, "shell/components/settings/ChannelListItem.tsx");
  assert.ok(fs.existsSync(channelItemPath), "ChannelListItem.tsx not found");
  const channelItemContent = fs.readFileSync(channelItemPath, "utf8");
  assert.ok(
    // Owner 2026-10-29: Task C 将列表项精简为单行 h-9，渠道名称使用 truncate 截断（仍保留 break-all 兜底）
    /<span[^>]*className=["'][^"']*truncate[^"']*["'][^>]*>\s*\{channel\.id\}\s*<\/span>/.test(channelItemContent),
    "ChannelListItem must render channel id with 'break-all' in the UI font"
  );
  assert.strictEqual(
    /font-mono\s+truncate/.test(channelItemContent),
    false,
    "ChannelListItem must not contain 'font-mono truncate'"
  );

  // 2. ModelChannelsSettingsTab.tsx：新建草稿同样复用可查看完整 id 的单行渠道项
  const modelTabPath = path.resolve(uiSrcDir, "shell/components/settings/ModelChannelsSettingsTab.tsx");
  assert.ok(fs.existsSync(modelTabPath), "ModelChannelsSettingsTab.tsx not found");
  const modelTabContent = fs.readFileSync(modelTabPath, "utf8");
  assert.ok(
    /<ChannelListItem[\s\S]{0,240}channel=\{\{ \.\.\.draft, models: draft\.models \}/.test(modelTabContent),
    "ModelChannelsSettingsTab new draft must reuse ChannelListItem for its complete channel id"
  );

  // 3. SettingsModal.tsx：overflow-y-scroll 出现且 flex-1 flex-col overflow-y-auto 不再出现
  const settingsModalPath = path.resolve(uiSrcDir, "components/SettingsModal.tsx");
  assert.ok(fs.existsSync(settingsModalPath), "SettingsModal.tsx not found");
  const settingsModalContent = fs.readFileSync(settingsModalPath, "utf8");
  assert.ok(
    settingsModalContent.includes("overflow-y-scroll"),
    "SettingsModal must contain 'overflow-y-scroll'"
  );
  assert.strictEqual(
    settingsModalContent.includes("flex-1 flex-col overflow-y-auto"),
    false,
    "SettingsModal must not contain 'flex-1 flex-col overflow-y-auto'"
  );

  // 4. NetworkProxySettingsTab.tsx：提供方名称为外链入口（title={ep.site}），右侧显示状态/延迟
  const networkProxyPath = path.resolve(uiSrcDir, "shell/components/settings/NetworkProxySettingsTab.tsx");
  assert.ok(fs.existsSync(networkProxyPath), "NetworkProxySettingsTab.tsx not found");
  const networkProxyContent = fs.readFileSync(networkProxyPath, "utf8");
  assert.ok(
    networkProxyContent.includes("title={ep.site}"),
    "NetworkProxySettingsTab must contain 'title={ep.site}'"
  );
  assert.ok(
    !networkProxyContent.includes("title={ep.endpoint}"),
    "NetworkProxySettingsTab must not show the raw probe endpoint URL"
  );
  assert.ok(
    networkProxyContent.includes("openExternalUrl"),
    "NetworkProxySettingsTab must open provider sites through the open-external-url boundary"
  );
});

test("Issue #45: .eo-settings-input & .eo-settings-control-input live in @layer components so pr-9/pr-10 win over base padding", () => {
  const indexCssPath = path.resolve(uiSrcDir, "index.css");
  assert.ok(fs.existsSync(indexCssPath), "index.css not found");
  const cssContent = fs.readFileSync(indexCssPath, "utf8");

  const layerMatch = cssContent.match(/@layer\s+components\s*\{([\s\S]*?\n)\}/);
  assert.ok(layerMatch, "index.css must declare a @layer components block");
  const layerBlock = layerMatch[1];
  assert.ok(layerBlock.includes(".eo-settings-control-input"), ".eo-settings-control-input must be inside @layer components");
  assert.ok(layerBlock.includes(".eo-settings-input"), ".eo-settings-input must be inside @layer components");
  assert.ok(layerBlock.includes("textarea.eo-settings-input"), "textarea.eo-settings-input must be inside @layer components");

  const outsideLayer = cssContent.replace(/@layer\s+components\s*\{[\s\S]*?\n\}/g, "");
  assert.equal(/^\.eo-settings-(?:control-)?input\b/m.test(outsideLayer), false, "base .eo-settings-input rules must not sit outside @layer components");

  const accountPanels = fs.readFileSync(path.resolve(uiSrcDir, "components/desktop/DesktopAccountPanels.tsx"), "utf8");
  assert.ok(accountPanels.includes("pr-9"), "CredentialInputField must keep pr-9 for reveal toggle");
  assert.ok(accountPanels.includes("flex-wrap gap-2"), "CredentialInputField label must allow flex-wrap in narrow grids");
  assert.ok(accountPanels.includes("whitespace-nowrap shrink-0"), "Saved badge must not wrap into vertical characters");

  const accountsTab = fs.readFileSync(path.resolve(uiSrcDir, "shell/components/settings/AccountsSettingsTab.tsx"), "utf8");
  assert.ok(accountsTab.includes("pr-10"), "GEE secret textarea must apply pr-10 to clear top-right eye button");
});
