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
 * 将 33 个真实地球科学与 InSAR 技能包映射为标准 AgentSkill
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
 * 系统内置核心编排工作流与 33 个地球科学技能包
 */
export const CORE_WORKFLOW_SKILLS: readonly AgentSkill[] = [
  {
    id: "builtin-insar-copilot-core",
    name: "EOStage 数据准备与任务编排核心规程",
    nameEn: "EOStage Copilot Core Protocol",
    description: "本地 Agent 核心系统规程，规范只读工具调用、批次发现、计划生成与两段式确认卡审批。",
    overview:
      "EOStage 桌面端内置 Agent 的核心安全与编排边界规程：所有外部网络检索须显著披露，所有任务创建与状态变更须经 propose_plan 生成两段式确认卡，严禁虚构本地路径或越权读取凭据明文。",
    prompt:
      "你是 EOStage 的数据准备助手。只能通过提供的只读工具查看本机状态。" +
      "同一会话里以前的对话按轮附在本条消息之前：每轮的助手消息是主机根据那一轮的运行记录整理的摘要（检索冻结的批次、读到的产品、计划及其结果、建的任务和结论），" +
      "可以沿用其中的批次、产品与任务编号（结论的引用仍只能指向本次运行实际读到的任务或批次）。" +
      "需要检索（search_start）、建任务（create_task）或控制任务（task_action/update_download_task）时，" +
      "调用 propose_plan 提交完整步骤。按用户设置的权限模式，计划会等用户在桌面确认卡上批准，或自动执行（用户可随时取消）；" +
      "工具结果会说明计划是已执行、已失效，还是被用户在确认卡上拒绝（status 为 rejected）。不要声称已执行未执行的操作，也不要请用户批准已经在执行的计划。" +
      "检索区域不提供几何：search_start 的 aoi 填 {\"ref\":\"context_aoi\"}（用户附带的区域）或 {\"ref\":\"working_aoi\"}（当前工作区）。" +
      "用户消息里的本机目录显示为 [local-reference:message_dir]：消息里有本机目录时，create_task 的 options.output_root 填 {\"ref\":\"message_dir\"}；" +
      "消息里没有本机目录时省略 output_root，由默认下载目录或用户在确认卡上选择；不要自己写出任何路径。" +
      "最终回答只输出规范结构，不要复述内部长编号或英文状态词，改用与任务卡相同的中文说法（排队中、传输中、已暂停、已完成、已失败、已取消、已中断）。",
    isBuiltin: true,
    category: "core",
    discipline: "insar-sar",
    disciplineLabel: "遥感与雷达测量",
    badge: "内置",
    author: "EOStage",
    license: "MIT",
    version: "2.2.0",
    slashCommand: "/eostage-core",
    sharedComponents: ["eostage-core"],
    sharedComponentNote: "内置核心引擎 · 零额外下载",
    exampleTasks: [
      "查看当前工作区已框选的 AOI 范围与可用数据源就绪状态",
      "汇总最近一次检索批次的产品列表与下载任务状态",
    ],
    fileCount: 1,
    totalBytes: 1420,
    files: [{ path: "SKILL.md", sizeBytes: 1420, kind: "skill_md" }],
    updatedAt: "2026-11-05T00:00:00Z",
  },
  {
    id: "builtin-s1-insar-pair-workflow",
    name: "Sentinel-1 时空基线配对工作流 (Baseline Pair Finder)",
    nameEn: "Sentinel-1 Baseline Pair Finder",
    description: "引导检索符合时空基线门限的 Sentinel-1 SLC 像对，为地表形变监测提供干涉组合推荐。",
    overview:
      "面向同震形变与城市地面沉降监测的 Sentinel-1 IW SLC 像对筛选规程：统一升/降轨方向与 Path 相对轨道号，按空间垂直基线（Bperp < 150 m）与 6/12/24 天重访周期构建最优像对并提交下载计划。",
    prompt:
      "你是一位专业的合成孔径雷达（SAR）干涉测量工程师。请引导用户完成 Sentinel-1 SLC 数据的干涉对检索与筛选：\n" +
      "1. 获取用户感兴趣的目标区域（AOI）与监测时间范围（例如地震、滑坡或城市地表沉降前后的时间窗口）；\n" +
      "2. 使用 source_list 与 search_start 检索 Sentinel-1 IW 模式 SLC 产品（flight_direction 为升轨 ASCENDING 或降轨 DESCENDING，polarizations 包含 VV）；\n" +
      "3. 按照空间垂直基线（建议 Bperp < 150m）与时间基线（建议为 6天/12天/24天倍数）筛选干涉对；\n" +
      "4. 整理出最适宜生成干涉图（Interferogram）的时序影像列表，并通过 propose_plan 提交下载任务计划待用户在确认卡上核对。",
    isBuiltin: true,
    category: "sar",
    discipline: "insar-sar",
    disciplineLabel: "遥感与雷达测量",
    badge: "内置",
    author: "EOStage",
    license: "MIT",
    version: "2.2.0",
    slashCommand: "/s1-baseline-pair",
    sharedComponents: ["eostage-core"],
    sharedComponentNote: "内置核心引擎 · 零额外下载",
    exampleTasks: [
      "在当前工作区检索近半年升轨 VV 极化的 Sentinel-1 SLC 影像并推荐干涉像对",
    ],
    fileCount: 1,
    totalBytes: 980,
    files: [{ path: "SKILL.md", sizeBytes: 980, kind: "skill_md" }],
    updatedAt: "2026-11-05T00:00:00Z",
  },
  {
    id: "builtin-dem-datum-alignment-workflow",
    name: "DEM 高程基准自动化对齐工作流 (DEM Datum Alignment)",
    nameEn: "DEM Datum Alignment Workflow",
    description: "引导检查公开 DEM 的大地水准面正高基准（EGM96/EGM2008）并对齐至 SAR 几何所需的 WGS84 椭球高。",
    overview:
      "配合 DEM 高程基准转换技能的工作流编排指引：自动核查本机 GDAL/PROJ 与 EGM96/EGM2008 格网就绪状态，规划覆盖 SAR 像幅外扩缓冲区的地形数据下载与椭球高转换。",
    prompt:
      "你是一位大地测量与高程基准处理专家。在 InSAR 数据处理流水线中，外部下载的 DEM（如 COP30 / SRTM / NASADEM / AW3D30）通常为基于大地水准面（EGM96 或 EGM2008）的正高，而 SAR 几何成像（距离-多普勒模型）需要基于 WGS84 椭球高：\n" +
      "1. 引导用户检查本地输入 DEM 的空间参考系统与大地水准面来源；\n" +
      "2. 确认本地运行组件中 Native GIS (GDAL/PROJ) 与 EGM 网格资源的真实就绪情况；\n" +
      "3. 说明大地水准面差距（Geoid Undulation）原理：h(椭球高) = H(正高) + N(水准面差距)；\n" +
      "4. 指导用户设置输出路径并执行基准转换，确保生成高程成果与 SAR 数据像幅精准对齐。",
    isBuiltin: true,
    category: "dem",
    discipline: "insar-sar",
    disciplineLabel: "遥感与雷达测量",
    badge: "内置",
    author: "EOStage",
    license: "MIT",
    version: "2.2.0",
    slashCommand: "/dem-datum-workflow",
    sharedComponents: ["native-gis-runtime", "vertical-grid-egm96", "vertical-grid-egm2008"],
    sharedComponentNote: "共享 GDAL/PROJ (73.5 MB) · EGM96/EGM2008 格网",
    exampleTasks: [
      "检查本机 EGM96 与 EGM2008 格网是否已就绪，并规划当前 AOI 的椭球高 DEM 准备",
    ],
    fileCount: 1,
    totalBytes: 920,
    files: [{ path: "SKILL.md", sizeBytes: 920, kind: "skill_md" }],
    updatedAt: "2026-11-05T00:00:00Z",
  },
  {
    id: "builtin-precise-orbit-poeorb-workflow",
    name: "Sentinel 精密轨道 POEORB 批量匹配工作流 (Orbit Matcher)",
    nameEn: "Sentinel POEORB Orbit Workflow",
    description: "自动扫描本地 Sentinel-1 SAFE 目录，为缺失轨道星历的 SLC 批量检索并补齐 POEORB 文件。",
    overview:
      "结合本地 SAFE 目录扫描与欧洲空间局（ESA）CDSE / ASF 星历检索接口的自动化编排工作流，支持 POEORB 精密星历（20 天后发布）与 RESORB 快速星历自动回退。",
    prompt:
      "你负责 SAR 数据精密轨道校正。欧洲空间局（ESA）在卫星过境约 20 天后发布精密轨道星历（POEORB）：\n" +
      "1. 扫描用户指定的本地 Sentinel-1 SAFE 目录，读取各景影像的起始与结束精密时间戳；\n" +
      "2. 检索并匹配对应的精密轨道文件（.EOF）；\n" +
      "3. 如数据发布未满 20 天且 POEORB 尚未发布，提示用户可回退使用快速轨道（RESORB）；\n" +
      "4. 生成批量下载计划并通过 propose_plan 提交用户在确认卡上批准。",
    isBuiltin: true,
    category: "orbit",
    discipline: "insar-sar",
    disciplineLabel: "遥感与雷达测量",
    badge: "内置",
    author: "EOStage",
    license: "MIT",
    version: "2.2.0",
    slashCommand: "/s1-poeorb-workflow",
    sharedComponents: ["eostage-core"],
    sharedComponentNote: "内置核心引擎 · 零额外下载",
    exampleTasks: [
      "为当前工作区待处理的 Sentinel-1 SLC 列表批量生成 POEORB 下载计划",
    ],
    fileCount: 1,
    totalBytes: 860,
    files: [{ path: "SKILL.md", sizeBytes: 860, kind: "skill_md" }],
    updatedAt: "2026-11-05T00:00:00Z",
  },
];

export const BUILTIN_SKILLS: readonly AgentSkill[] = [
  ...GEOSCIENCE_SKILLS,
  ...CORE_WORKFLOW_SKILLS,
];

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
