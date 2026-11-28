/**
 * agentSkills.ts
 * 地球科学、GIS 与 InSAR 技能包定义（含 SKILL.md 规程、references/ 参考规范、scripts/ 转换脚本、共享本机组件依赖）
 */

import {
  GEOSCIENCE_SKILL_PACKAGES,
  type GeoscienceSkillPackage,
  type SkillFileEntry,
} from "./geoscienceSkillsCatalog";

export type { GeoscienceSkillPackage, SkillFileEntry };

export type SkillDiscipline =
  | "insar-sar"
  | "gis-spatial"
  | "earth-env"
  | "skill-engineering"
  | "custom";

export interface AgentSkill {
  id: string;
  name: string;
  nameEn?: string;
  description: string;
  overview?: string;
  prompt: string;
  isBuiltin: boolean;
  category: "core" | "sar" | "dem" | "orbit" | "gis" | "earth" | "custom";
  discipline?: SkillDiscipline;
  disciplineLabel?: string;
  badge?: "内置" | "官方" | "自定义";
  sourceLabel?: string;
  author?: string;
  license?: string;
  version?: string;
  slashCommand?: string;
  sharedComponents?: string[];
  sharedComponentNote?: string;
  workbenchTool?: string | null;
  exampleTasks?: string[];
  fileCount?: number;
  totalBytes?: number;
  files?: SkillFileEntry[];
  bundledFileContents?: Record<string, string>;
  tags?: string[];
  updatedAt: string;
}

export const AGENT_SKILLS_STORAGE_KEY = "eostage.agent.customSkills.v1";
export const DISABLED_SKILLS_STORAGE_KEY = "eostage.agent.disabledSkills.v1";

/**
 * 将 29 个真实地球科学与 InSAR 技能包映射为标准 AgentSkill
 */
export const GEOSCIENCE_SKILLS: readonly AgentSkill[] = GEOSCIENCE_SKILL_PACKAGES.map(
  (pkg): AgentSkill => ({
    id: pkg.id,
    name: pkg.nameZh,
    nameEn: pkg.nameEn,
    description: pkg.shortDescriptionZh,
    overview: pkg.overviewZh,
    prompt: pkg.skillMd,
    isBuiltin: true,
    category:
      pkg.discipline === "insar-sar"
        ? "sar"
        : pkg.discipline === "gis-spatial"
          ? "gis"
          : pkg.discipline === "earth-env"
            ? "earth"
            : "core",
    discipline: pkg.discipline,
    disciplineLabel: pkg.disciplineLabel,
    badge: pkg.badge,
    author: pkg.author,
    license: pkg.license,
    version: pkg.version,
    slashCommand: pkg.slashCommand,
    sharedComponents: pkg.sharedComponents,
    sharedComponentNote: pkg.sharedComponentNote,
    workbenchTool: pkg.workbenchTool,
    exampleTasks: pkg.exampleTasks,
    fileCount: pkg.fileCount,
    totalBytes: pkg.totalBytes,
    files: pkg.files,
    bundledFileContents: pkg.bundledFileContents,
    updatedAt: "2026-11-05T00:00:00Z",
  }),
);

/**
 * 全部内置地球科学与 InSAR 技能（由 ui/public/skills 权威包生成；核心对话行为规则已下沉至 Rust SYSTEM_PROMPT）
 */
export const BUILTIN_SKILLS: readonly AgentSkill[] = [...GEOSCIENCE_SKILLS];

export function loadDisabledSkillIds(): Set<string> {
  if (typeof window === "undefined" || !window.localStorage) return new Set();
  try {
    const raw = window.localStorage.getItem(DISABLED_SKILLS_STORAGE_KEY);
    if (!raw) return new Set();
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return new Set();
    return new Set(parsed.filter((x) => typeof x === "string"));
  } catch {
    return new Set();
  }
}

export function saveDisabledSkillIds(ids: Iterable<string>): void {
  if (typeof window === "undefined" || !window.localStorage) return;
  try {
    window.localStorage.setItem(
      DISABLED_SKILLS_STORAGE_KEY,
      JSON.stringify(Array.from(ids)),
    );
  } catch {
    // ignore
  }
}

export function toggleSkillEnabled(skillId: string, enable: boolean): Set<string> {
  const current = loadDisabledSkillIds();
  if (enable) {
    current.delete(skillId);
  } else {
    current.add(skillId);
  }
  saveDisabledSkillIds(current);
  return current;
}

export function loadCustomSkills(): AgentSkill[] {
  if (typeof window === "undefined" || !window.localStorage) return [];
  try {
    const raw = window.localStorage.getItem(AGENT_SKILLS_STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(
      (item) =>
        item &&
        typeof item.id === "string" &&
        typeof item.name === "string" &&
        typeof item.prompt === "string" &&
        !item.isBuiltin,
    );
  } catch {
    return [];
  }
}

export function saveCustomSkills(skills: AgentSkill[]): void {
  if (typeof window === "undefined" || !window.localStorage) return;
  try {
    const filtered = skills.filter((s) => !s.isBuiltin);
    window.localStorage.setItem(AGENT_SKILLS_STORAGE_KEY, JSON.stringify(filtered));
  } catch {
    // 忽略 localStorage 配额错误
  }
}

export function getAllSkills(): AgentSkill[] {
  const custom = loadCustomSkills();
  return [...BUILTIN_SKILLS, ...custom];
}

export function getEnabledAgentSkills(): AgentSkill[] {
  const disabled = loadDisabledSkillIds();
  return getAllSkills().filter((skill) => !disabled.has(skill.id));
}

export function formatSkillInjectionPrompt(skill: AgentSkill): string {
  const cmd = skill.slashCommand ? ` (${skill.slashCommand})` : "";
  return `[工作流/技能: ${skill.name}${cmd}]\n${skill.prompt.trim()}`;
}

export function exportCustomSkillsJson(): string {
  const custom = loadCustomSkills();
  return JSON.stringify(
    {
      schema: "eostage.skills.v1",
      exportedAt: new Date().toISOString(),
      skills: custom,
    },
    null,
    2,
  );
}

/**
 * 解析标准 SKILL.md (含 YAML Frontmatter --- name / description / license ---)
 */
export function parseSkillMarkdownFile(
  markdownText: string,
  fallbackFileName = "SKILL.md",
): AgentSkill {
  const trimmed = markdownText.trim();
  let name = "";
  let description = "";
  let license = "MIT";
  let author = "User";
  let body = trimmed;

  if (trimmed.startsWith("---")) {
    const endIdx = trimmed.indexOf("\n---", 3);
    if (endIdx !== -1) {
      const fm = trimmed.slice(3, endIdx);
      body = trimmed.slice(endIdx + 4).trim();
      for (const rawLine of fm.split(/\r?\n/)) {
        const line = rawLine.trim();
        if (line.startsWith("name:")) {
          name = line.slice(5).trim().replace(/^["']|["']$/g, "");
        } else if (line.startsWith("description:")) {
          description = line.slice(12).trim().replace(/^["']|["']$/g, "");
        } else if (line.startsWith("license:")) {
          license = line.slice(8).trim().replace(/^["']|["']$/g, "");
        } else if (line.startsWith("author:")) {
          author = line.slice(7).trim().replace(/^["']|["']$/g, "");
        }
      }
    }
  }

  if (!name) {
    const h1Match = body.match(/^#\s+(.+)$/m);
    name = h1Match ? h1Match[1].trim() : fallbackFileName.replace(/\.md$/i, "");
  }
  if (!description) {
    const firstPara = body
      .split(/\r?\n\r?\n/)
      .map((p) => p.trim())
      .find((p) => p && !p.startsWith("#") && !p.startsWith("<!--"));
    description = firstPara ? firstPara.slice(0, 140) : name;
  }

  const slug = name
    .toLowerCase()
    .replace(/[^a-z0-9\u4e00-\u9fa5-_]+/g, "-")
    .replace(/^-+|-+$/g, "");
  const id = `custom-${slug || Date.now()}`;
  const sizeBytes = new TextEncoder().encode(markdownText).length;

  return {
    id,
    name,
    description,
    overview: description,
    prompt: markdownText,
    isBuiltin: false,
    category: "custom",
    discipline: "custom",
    disciplineLabel: "本地导入技能",
    badge: "自定义",
    author,
    license,
    version: "local",
    slashCommand: `/${slug || "custom-skill"}`,
    sharedComponents: ["eostage-core"],
    sharedComponentNote: "本地导入 SKILL.md 技能包",
    exampleTasks: [`使用 /${slug || "custom-skill"} 执行 ${name} 分析`],
    fileCount: 1,
    totalBytes: sizeBytes,
    files: [{ path: "SKILL.md", sizeBytes, kind: "skill_md" }],
    bundledFileContents: { "SKILL.md": markdownText },
    updatedAt: new Date().toISOString(),
  };
}

export function importCustomSkillsFromJson(inputText: string): {
  importedCount: number;
  skills: AgentSkill[];
} {
  const trimmed = inputText.trim();
  // 支持直接粘贴 SKILL.md (以 --- 或 # 开头)
  if (trimmed.startsWith("---") || (trimmed.startsWith("#") && !trimmed.startsWith("{"))) {
    const mdSkill = parseSkillMarkdownFile(trimmed);
    const existing = loadCustomSkills().filter((s) => s.id !== mdSkill.id);
    const merged = [...existing, mdSkill];
    saveCustomSkills(merged);
    return {
      importedCount: 1,
      skills: getAllSkills(),
    };
  }

  const parsed = JSON.parse(trimmed);
  const rawList: any[] = Array.isArray(parsed)
    ? parsed
    : Array.isArray(parsed?.skills)
      ? parsed.skills
      : Array.isArray(parsed?.entries)
        ? parsed.entries
        : parsed && typeof parsed === "object"
          ? [parsed]
          : [];

  const nowIso = new Date().toISOString();
  const normalized: AgentSkill[] = [];

  for (const item of rawList) {
    if (!item || typeof item !== "object") continue;
    const rawSkill = item.skill && typeof item.skill === "object" ? item.skill : item;
    const name = String(rawSkill.name || rawSkill.name_zh || rawSkill.name_en || "").trim();
    const description = String(
      rawSkill.description || rawSkill.description_zh || rawSkill.description_en || "",
    ).trim();
    const prompt = String(
      rawSkill.prompt || rawSkill.prompt_template || rawSkill.skillMd || "",
    ).trim();
    if (!name || !prompt) continue;
    const id = String(rawSkill.id || `custom-skill-${Date.now()}-${normalized.length}`).trim();
    const finalId = id.startsWith("builtin-") ? `custom-${id}` : id;
    const sizeBytes = new TextEncoder().encode(prompt).length;
    normalized.push({
      id: finalId,
      name,
      description: description || name,
      overview: String(rawSkill.overview || rawSkill.overviewZh || description || name),
      prompt,
      isBuiltin: false,
      category: "custom",
      discipline: "custom",
      disciplineLabel: "自定义技能",
      badge: "自定义",
      author: String(rawSkill.author || "User"),
      license: String(rawSkill.license || "MIT"),
      version: String(rawSkill.version || "1.0.0"),
      slashCommand: String(rawSkill.slashCommand || `/${finalId}`),
      sharedComponents: Array.isArray(rawSkill.sharedComponents)
        ? rawSkill.sharedComponents
        : ["eostage-core"],
      sharedComponentNote: String(rawSkill.sharedComponentNote || "自定义技能包"),
      exampleTasks: Array.isArray(rawSkill.exampleTasks) ? rawSkill.exampleTasks : [],
      fileCount: 1,
      totalBytes: sizeBytes,
      files: [{ path: "SKILL.md", sizeBytes, kind: "skill_md" }],
      bundledFileContents: { "SKILL.md": prompt },
      updatedAt: nowIso,
    });
  }

  if (normalized.length === 0) {
    throw new Error("未找到有效的技能定义（支持 SKILL.md 文本或包含 name 与 prompt 的 JSON）");
  }

  const existing = loadCustomSkills();
  const byId = new Map<string, AgentSkill>();
  for (const s of existing) {
    byId.set(s.id, s);
  }
  for (const s of normalized) {
    byId.set(s.id, s);
  }
  const merged = Array.from(byId.values());
  saveCustomSkills(merged);
  return {
    importedCount: normalized.length,
    skills: getAllSkills(),
  };
}

export const getAgentSkills = getAllSkills;
export const SKILLS_STORAGE_KEY = AGENT_SKILLS_STORAGE_KEY;
