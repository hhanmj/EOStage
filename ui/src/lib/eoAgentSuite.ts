/**
 * eoAgentSuite.ts
 * EOStage Agent 核心工程能力的五大模块（部分工具调度、PKCE 辅助、4D 空间排序与安全校验移植/改编自
 * opengeos/GeoAgent，MIT License, Copyright (c) 2026 Open Geospatial Solutions）：
 *
 * P1. 500+ 工具防爆上下文与按需发现代理 (On-Demand Tool Discovery & Context Broker)
 *     - 结合渐进式披露 (<=12 Eager 阈值 + <=160 字单行目录) 与 opengeos/GeoAgent (`whitebox.py` / `registry.py`)
 *       的 Fast/Full 执行模式 + 四步元工具代理 (summarize_tools / search_tools / get_tool_info / load_skill_tools)。
 * P2. 免 API Key 浏览器一键登录授权 (Browser OAuth PKCE S256 & JWT Claim Helper)
 *     - 参考 opengeos/GeoAgent `geoagent/core/openai_codex.py` 与标准 OAuth PKCE S256 流程（Codex OAuth PKCE / OpenRouter PKCE /
 *       GitHub Copilot / Gemini OAuth），提供纯前端确定性 SHA-256 Base64URL PKCE 生成、回调 State 校验、
 *       JWT chatgpt_account_id 解析与过期提前量检测。
 * P3. 多因子空间优先四维景排序 (Multi-Factor 4D Spatial Scene Ranking)
 *     - 移植/改编自 opengeos/GeoAgent `geoagent/tools/stac.py`：候选池自动扩容 (>=50) + (1) 查询中心点包含 (Polygon/MultiPolygon 含洞射线法)
 *       + (2) BBox 重叠面积占比 + (3) 云量优先意图识别 + (4) 时间戳四维排序。
 * P4. 软取消同轮重规划反馈、脚本安全静态门禁与可复现实验脚本导出
 *     - 移植/改编自 opengeos/GeoAgent `geoagent/core/confirmation_hook.py` 与 `safety.py`：拒绝确认卡后返回软取消结构化重规划提示、
 *       工具长结果上下文截断、PyQGIS/GEE 危险调用静态拦截，以及零密钥可复现 Python 脚本导出。
 * P5. OlmoEarth v1.3 预计算特征分片规划、非线性反量化与观测级跨源溯源辅助
 *     - 针对 AllenAI OlmoEarth Precomputed Embeddings v1.3 (Zarr v3 Sharding `1×128×2048×2048` / 内块 `1×64×256×256` /
 *       2052B 尾部索引)，提供南半球 `EPSG:326NN` 负值北坐标 (Negative Northing) 窗口规划、Matryoshka 64D/128D
 *       零间隙 (`maximum_gap_bytes = 0`) 字节预算、`((abs(q)/127.5)**2)*sign(q)` 非线性反量化 (`NoData = -128`)、
 *       观测级低云景跨源溯源映射，以及显式附带 `OlmoEarth Artifact License` 限制声明的 Python 配方导出。
 */

import {
  REGISTERED_AGENT_TOOLS,
  type RegisteredAgentTool,
} from "./agentRegisteredTools.ts";

// ============================================================================
// P1. 500+ 工具防爆上下文与按需发现代理 (On-Demand Tool Discovery & Context Broker)
// ============================================================================

export type AgentToolContextMode = "full" | "fast";

export const TOOL_CONTEXT_MODES: readonly AgentToolContextMode[] = ["full", "fast"] as const;

/** 默认直接挂载完整 JSON Schema 的工具数量上限（超过该阈值自动切换为按需代理披露） */
export const EAGER_TOOL_SCHEMA_LIMIT = 12;

/** 单条工具摘要最大字符数（对齐 <=160 字符渐进式披露规范） */
export const MAX_TOOL_SUMMARY_CHARS = 160;

export interface ToolCatalogSummaryItem {
  tool_id: string;
  name_zh: string;
  job: string;
  effect: RegisteredAgentTool["effect"];
  summary_line: string;
}

export interface ToolBrokerMetaTool {
  tool_id: "summarize_tools" | "search_tools" | "get_tool_info" | "load_skill_tools";
  name_zh: string;
  description_zh: string;
  param_names: string[];
}

export const BROKER_META_TOOLS: readonly ToolBrokerMetaTool[] = [
  {
    tool_id: "summarize_tools",
    name_zh: "工具分类汇总目录",
    description_zh: "按作业阶段 (inspect/area/discover/obtain/verify/organize) 汇总可用工具数量与简短目录，不加载庞大参数 Schema。",
    param_names: ["job_filter"],
  },
  {
    tool_id: "search_tools",
    name_zh: "按需关键词检索工具",
    description_zh: "在数百个内置与扩展工具中按关键词检索最匹配的候选工具 ID 与单行说明（默认返回前 10 项）。",
    param_names: ["query", "job_filter", "limit"],
  },
  {
    tool_id: "get_tool_info",
    name_zh: "按需读取单工具完整 Schema",
    description_zh: "仅针对准备调用的目标工具按需拉取完整输入输出参数 Schema 与审批约束。",
    param_names: ["tool_id"],
  },
  {
    tool_id: "load_skill_tools",
    name_zh: "按需挂载技能关联工具集",
    description_zh: "将选定工具 ID 列表的完整 JSON Schema 动态挂载到当前轮次上下文中。",
    param_names: ["tool_ids"],
  },
] as const;

/**
 * 根据执行上下文模式过滤工具（移植/参考自 opengeos/GeoAgent `registry.py` `get_tools_for_context("fast" | "full")`）：
 * - "fast" 模式：仅暴露纯本地只读探测/工作区查看工具（effect === "read" 且 confirmation === "none"），避免写操作或重型外部请求。
 * - "full" 模式：暴露全部已注册工具。
 */
export function getToolsForContext(
  tools: readonly RegisteredAgentTool[] = REGISTERED_AGENT_TOOLS,
  contextMode: AgentToolContextMode = "full"
): RegisteredAgentTool[] {
  if (contextMode === "fast") {
    return tools.filter((t) => t.effect === "read" && t.confirmation === "none");
  }
  return [...tools];
}

/**
 * 将单个工具压缩为 <=160 字符的单行目录摘要（不携带完整字段表）
 */
export function formatCompactToolSummaryLine(tool: RegisteredAgentTool): string {
  const base = `[${tool.tool_id}] (${tool.job}/${tool.effect}) ${tool.name_zh}：${tool.description_zh}`;
  if (base.length <= MAX_TOOL_SUMMARY_CHARS) {
    return base;
  }
  return `${base.slice(0, MAX_TOOL_SUMMARY_CHARS - 1)}…`;
}

/**
 * 按作业分类统计并生成轻量目录（移植/参考自 opengeos/GeoAgent `whitebox.py` `summarize_whitebox_tools`）
 */
export function summarizeToolCatalog(
  tools: readonly RegisteredAgentTool[] = REGISTERED_AGENT_TOOLS
): {
  totalCount: number;
  byJob: Record<string, number>;
  byEffect: Record<string, number>;
  catalogLines: ToolCatalogSummaryItem[];
} {
  const byJob: Record<string, number> = {};
  const byEffect: Record<string, number> = {};
  const catalogLines: ToolCatalogSummaryItem[] = [];

  for (const tool of tools) {
    byJob[tool.job] = (byJob[tool.job] ?? 0) + 1;
    byEffect[tool.effect] = (byEffect[tool.effect] ?? 0) + 1;
    catalogLines.push({
      tool_id: tool.tool_id,
      name_zh: tool.name_zh,
      job: tool.job,
      effect: tool.effect,
      summary_line: formatCompactToolSummaryLine(tool),
    });
  }

  return {
    totalCount: tools.length,
    byJob,
    byEffect,
    catalogLines,
  };
}

/**
 * 按关键词与分类检索工具（移植/参考自 opengeos/GeoAgent `whitebox.py` `search_whitebox_tools`：名称精确/前缀匹配优先，其次描述匹配）
 */
export function searchRegisteredTools(
  query: string,
  options?: {
    tools?: readonly RegisteredAgentTool[];
    jobFilter?: string;
    contextMode?: AgentToolContextMode;
    limit?: number;
  }
): RegisteredAgentTool[] {
  const baseTools = getToolsForContext(
    options?.tools ?? REGISTERED_AGENT_TOOLS,
    options?.contextMode ?? "full"
  );
  const jobFilter = options?.jobFilter?.trim().toLowerCase() ?? "";
  const limit = Math.max(1, Math.min(50, options?.limit ?? 10));
  const q = query.trim().toLowerCase();

  const filteredByJob = jobFilter
    ? baseTools.filter((t) => t.job.toLowerCase() === jobFilter)
    : baseTools;

  if (!q) {
    return filteredByJob.slice(0, limit);
  }

  const scored = filteredByJob
    .map((tool) => {
      const idLower = tool.tool_id.toLowerCase();
      const nameLower = tool.name_zh.toLowerCase();
      const descLower = tool.description_zh.toLowerCase();
      const cmdLower = tool.target_command.toLowerCase();

      let score = 0;
      if (idLower === q || nameLower === q) score += 100;
      else if (idLower.startsWith(q) || nameLower.startsWith(q)) score += 60;
      else if (idLower.includes(q) || nameLower.includes(q)) score += 40;
      if (cmdLower.includes(q)) score += 20;
      if (descLower.includes(q)) score += 15;
      if (tool.job.toLowerCase().includes(q)) score += 10;

      return { tool, score };
    })
    .filter((item) => item.score > 0)
    .sort((a, b) => b.score - a.score || a.tool.tool_id.localeCompare(b.tool.tool_id));

  return scored.slice(0, limit).map((item) => item.tool);
}

/**
 * 按需获取单个工具完整 Schema 详情（移植/参考自 opengeos/GeoAgent `whitebox.py` `get_whitebox_tool_info`）
 */
export function getRegisteredToolInfo(
  toolId: string,
  tools: readonly RegisteredAgentTool[] = REGISTERED_AGENT_TOOLS
): RegisteredAgentTool | null {
  const target = toolId.trim();
  return tools.find((t) => t.tool_id === target) ?? null;
}

export interface BrokeredToolScopeOptions {
  contextMode?: AgentToolContextMode;
  eagerLimit?: number;
  loadedToolIds?: readonly string[];
  simulatedExtensionToolCount?: number;
}

export interface BrokeredToolScopeResult {
  contextMode: AgentToolContextMode;
  brokerActive: boolean;
  totalAvailableTools: number;
  eagerSchemaTools: RegisteredAgentTool[];
  deferredSummaryCatalog: ToolCatalogSummaryItem[];
  metaTools: readonly ToolBrokerMetaTool[];
  fullSchemaTokensEst: number;
  brokeredTokensEst: number;
  savedTokensPct: number;
}

function estimateToolSchemaTokens(tool: RegisteredAgentTool): number {
  const paramChars = tool.paramFields.reduce(
    (acc, f) => acc + f.name.length + f.type.length + f.description_zh.length + 28,
    0
  );
  const baseChars =
    tool.tool_id.length +
    tool.name_zh.length +
    tool.description_zh.length +
    tool.target_command.length +
    tool.input_ref.length +
    tool.output_ref.length +
    120;
  return Math.ceil((baseChars + paramChars) * 0.45);
}

/**
 * 构建渐进式披露 / Broker 工具挂载视图：
 * - 当可用工具总数 <= eagerLimit 时，直接挂载完整 Schema；
 * - 当可用工具总数 > eagerLimit（如 16 个内置工具或扩展至 500+ 工具）时，启用 Broker 按需披露：
 *   仅将核心只读探测工具 + 显式通过 `load_skill_tools` 激活的工具挂载完整 Schema，
 *   其余工具仅保留单行摘要或通过 4 个 Broker 元工具按需检索，防止撑爆上下文窗口。
 */
export function buildBrokeredAgentTools(
  tools: readonly RegisteredAgentTool[] = REGISTERED_AGENT_TOOLS,
  options: BrokeredToolScopeOptions = {}
): BrokeredToolScopeResult {
  const contextMode = options.contextMode ?? "full";
  const eagerLimit = Math.max(1, options.eagerLimit ?? EAGER_TOOL_SCHEMA_LIMIT);
  const loadedSet = new Set(options.loadedToolIds ?? []);
  const scopedTools = getToolsForContext(tools, contextMode);

  const extraSimulatedCount = Math.max(0, options.simulatedExtensionToolCount ?? 0);
  const totalAvailableTools = scopedTools.length + extraSimulatedCount;
  const brokerActive = totalAvailableTools > eagerLimit;

  let eagerSchemaTools: RegisteredAgentTool[];
  let deferredTools: RegisteredAgentTool[];

  if (!brokerActive) {
    eagerSchemaTools = [...scopedTools];
    deferredTools = [];
  } else {
    // 优先挂载已显式按需加载的工具 + Tier-0 基础探测工具，总数受控于 eagerLimit
    const explicitlyLoaded = scopedTools.filter((t) => loadedSet.has(t.tool_id));
    const remainingSlots = Math.max(0, Math.min(4, eagerLimit - explicitlyLoaded.length));
    const tier0Seeds = scopedTools
      .filter((t) => !loadedSet.has(t.tool_id) && t.effect === "read" && t.confirmation === "none")
      .slice(0, remainingSlots);
    const eagerIds = new Set([...explicitlyLoaded, ...tier0Seeds].map((t) => t.tool_id));
    eagerSchemaTools = scopedTools.filter((t) => eagerIds.has(t.tool_id));
    deferredTools = scopedTools.filter((t) => !eagerIds.has(t.tool_id));
  }

  const deferredSummaryCatalog = deferredTools.map((t) => ({
    tool_id: t.tool_id,
    name_zh: t.name_zh,
    job: t.job,
    effect: t.effect,
    summary_line: formatCompactToolSummaryLine(t),
  }));

  const baselineAllToolsTokens = tools.reduce(
    (sum, t) => sum + estimateToolSchemaTokens(t),
    0
  );
  const scopedFullTokens = scopedTools.reduce(
    (sum, t) => sum + estimateToolSchemaTokens(t),
    0
  );
  // 每个扩展工具若全量注入 JSON Schema 约占 180 tokens
  const fullSchemaTokensEst = baselineAllToolsTokens + extraSimulatedCount * 180;

  const eagerTokens = eagerSchemaTools.reduce((sum, t) => sum + estimateToolSchemaTokens(t), 0);
  const metaTokens = brokerActive ? BROKER_META_TOOLS.length * 48 : 0;
  // 当延迟工具超过 eagerLimit 时，仅向提示词注入分类汇总与至多 eagerLimit 条单行索引，其余通过 search_tools 按需发现
  const inlineSummaryCount = Math.min(deferredSummaryCatalog.length, eagerLimit);
  const summaryTokens = inlineSummaryCount * 24;
  const brokeredTokensEst = brokerActive
    ? eagerTokens + metaTokens + summaryTokens
    : scopedFullTokens;

  const savedTokensPct =
    fullSchemaTokensEst > 0 && brokeredTokensEst < fullSchemaTokensEst
      ? Number((((fullSchemaTokensEst - brokeredTokensEst) / fullSchemaTokensEst) * 100).toFixed(1))
      : 0;

  return {
    contextMode,
    brokerActive,
    totalAvailableTools,
    eagerSchemaTools,
    deferredSummaryCatalog,
    metaTools: brokerActive ? BROKER_META_TOOLS : [],
    fullSchemaTokensEst,
    brokeredTokensEst,
    savedTokensPct,
  };
}

// ============================================================================
// P2. 免 API Key 浏览器一键登录授权 (Browser OAuth PKCE S256 & JWT Claim Helper)
// ============================================================================

export interface OAuthPkceProviderPreset {
  id:
    | "openai_codex_pkce"
    | "claude_oauth_pkce"
    | "openrouter_pkce"
    | "github_copilot_device"
    | "kimi_code_device"
    | "google_gemini_pkce";
  name_zh: string;
  protocol: "responses" | "chat_completions" | "google_gemini" | "anthropic_messages";
  base_url: string;
  default_model: string;
  authorize_url: string;
  token_url: string;
  client_id: string;
  redirect_uri: string;
  scopes: string;
  extra_params?: Record<string, string>;
  description_zh: string;
}

/**
 * 免 API Key 浏览器 OAuth PKCE / Device Flow 授权预设目录
 */
export const OAUTH_PKCE_PROVIDERS: readonly OAuthPkceProviderPreset[] = [
  {
    id: "openai_codex_pkce",
    name_zh: "OpenAI Codex / ChatGPT (PKCE)",
    protocol: "responses",
    base_url: "https://chatgpt.com/backend-api/codex",
    default_model: "",
    authorize_url: "https://auth.openai.com/oauth/authorize",
    token_url: "https://auth.openai.com/oauth/token",
    client_id: "app_EMoamEEZ73f0CkXaXp7hrann",
    redirect_uri: "http://localhost:1455/auth/callback",
    scopes: "openid profile email offline_access",
    extra_params: {
      id_token_add_organizations: "true",
      codex_cli_simplified_flow: "true",
      originator: "codex_cli_rs",
    },
    description_zh: "使用浏览器登录 OpenAI / ChatGPT 账号，通过本地 1455 端口回环完成 OAuth 2.0 S256 PKCE 免密授权。",
  },
  {
    id: "claude_oauth_pkce",
    name_zh: "Claude / Anthropic (PKCE)",
    protocol: "anthropic_messages",
    base_url: "https://api.anthropic.com",
    default_model: "",
    authorize_url: "https://claude.ai/oauth/authorize",
    token_url: "https://platform.claude.com/v1/oauth/token",
    client_id: "9d1c250a-e61b-44d9-88ed-5944d1962f5e",
    redirect_uri: "http://localhost:54545/callback",
    scopes: "user:profile user:inference user:sessions:claude_code",
    extra_params: {
      code: "true",
    },
    description_zh: "通过浏览器登录 Claude 账号，支持本地 54545 端口回环或复制 code#state 完成官方订阅授权。",
  },
  {
    id: "openrouter_pkce",
    name_zh: "OpenRouter 浏览器授权 (PKCE)",
    protocol: "chat_completions",
    base_url: "https://openrouter.ai/api/v1",
    default_model: "",
    authorize_url: "https://openrouter.ai/auth",
    token_url: "https://openrouter.ai/api/v1/auth/keys",
    client_id: "eostage-desktop",
    redirect_uri: "http://localhost:1455/auth/callback",
    scopes: "chat",
    description_zh: "通过浏览器一键完成 OpenRouter S256 PKCE 授权并由后端自动换取专属密钥，无需手动复制主密钥。",
  },
  {
    id: "github_copilot_device",
    name_zh: "GitHub Copilot 浏览器设备码授权 (Device Flow)",
    protocol: "chat_completions",
    base_url: "https://api.githubcopilot.com",
    default_model: "",
    authorize_url: "https://github.com/login/device",
    token_url: "https://github.com/login/oauth/access_token",
    client_id: "Iv1.b507a08c87ecfe98",
    redirect_uri: "http://localhost:1455/auth/callback",
    scopes: "read:user",
    description_zh: "请求 GitHub 官方设备验证码 (user_code)，在浏览器输入验证码后自动轮询绑定 Copilot 渠道。",
  },
  {
    id: "kimi_code_device",
    name_zh: "Kimi Code 浏览器设备码授权 (Device Flow)",
    protocol: "chat_completions",
    base_url: "https://api.kimi.com/coding/v1",
    default_model: "",
    authorize_url: "https://www.kimi.com/code/oauth",
    token_url: "https://auth.kimi.com/api/oauth/token",
    client_id: "17e5f671-d194-4dfb-9706-5516cb48c098",
    redirect_uri: "http://localhost:1455/auth/callback",
    scopes: "coding",
    description_zh: "通过 Kimi Code 官方设备授权码登录并自动续期访问令牌。",
  },
  {
    id: "google_gemini_pkce",
    name_zh: "Google Gemini OAuth 凭据接入",
    protocol: "google_gemini",
    base_url: "https://generativelanguage.googleapis.com/v1beta",
    default_model: "",
    authorize_url: "https://accounts.google.com/o/oauth2/v2/auth",
    token_url: "https://oauth2.googleapis.com/token",
    client_id: "681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com",
    redirect_uri: "http://localhost:1455/auth/callback",
    scopes: "https://www.googleapis.com/auth/cloud-platform openid email",
    description_zh: "通过浏览器 OAuth 或粘贴已授权的 OAuth 2.0 访问令牌接入 Google Gemini。",
  },
] as const;

export const JWT_OPENAI_AUTH_CLAIM = "https://api.openai.com/auth";

/**
 * 纯 TypeScript 标准 SHA-256 实现（保证在任何浏览器、JSDOM 测试环境与非 HTTPS 上下文中均可同步生成 RFC 7636 S256 PKCE Challenge）
 */
function sha256Bytes(ascii: string): Uint8Array {
  const K = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
  ];

  const rotr = (n: number, x: number) => (x >>> n) | (x << (32 - n));
  const bytes: number[] = [];
  for (let i = 0; i < ascii.length; i++) {
    bytes.push(ascii.charCodeAt(i) & 0xff);
  }

  const bitLen = bytes.length * 8;
  bytes.push(0x80);
  while ((bytes.length % 64) !== 56) {
    bytes.push(0);
  }
  const hi = Math.floor(bitLen / 0x100000000);
  const lo = bitLen >>> 0;
  bytes.push((hi >>> 24) & 0xff, (hi >>> 16) & 0xff, (hi >>> 8) & 0xff, hi & 0xff);
  bytes.push((lo >>> 24) & 0xff, (lo >>> 16) & 0xff, (lo >>> 8) & 0xff, lo & 0xff);

  let h0 = 0x6a09e667;
  let h1 = 0xbb67ae85;
  let h2 = 0x3c6ef372;
  let h3 = 0xa54ff53a;
  let h4 = 0x510e527f;
  let h5 = 0x9b05688c;
  let h6 = 0x1f83d9ab;
  let h7 = 0x5be0cd19;

  const w = new Int32Array(64);
  for (let offset = 0; offset < bytes.length; offset += 64) {
    for (let i = 0; i < 16; i++) {
      const j = offset + i * 4;
      w[i] =
        ((bytes[j] << 24) |
          (bytes[j + 1] << 16) |
          (bytes[j + 2] << 8) |
          bytes[j + 3]) |
        0;
    }
    for (let i = 16; i < 64; i++) {
      const w15 = w[i - 15];
      const w2 = w[i - 2];
      const s0 = rotr(7, w15) ^ rotr(18, w15) ^ (w15 >>> 3);
      const s1 = rotr(17, w2) ^ rotr(19, w2) ^ (w2 >>> 10);
      w[i] = (w[i - 16] + s0 + w[i - 7] + s1) | 0;
    }

    let a = h0;
    let b = h1;
    let c = h2;
    let d = h3;
    let e = h4;
    let f = h5;
    let g = h6;
    let h = h7;

    for (let i = 0; i < 64; i++) {
      const S1 = rotr(6, e) ^ rotr(11, e) ^ rotr(25, e);
      const ch = (e & f) ^ (~e & g);
      const temp1 = (h + S1 + ch + K[i] + w[i]) | 0;
      const S0 = rotr(2, a) ^ rotr(13, a) ^ rotr(22, a);
      const maj = (a & b) ^ (a & c) ^ (b & c);
      const temp2 = (S0 + maj) | 0;

      h = g;
      g = f;
      f = e;
      e = (d + temp1) | 0;
      d = c;
      c = b;
      b = a;
      a = (temp1 + temp2) | 0;
    }

    h0 = (h0 + a) | 0;
    h1 = (h1 + b) | 0;
    h2 = (h2 + c) | 0;
    h3 = (h3 + d) | 0;
    h4 = (h4 + e) | 0;
    h5 = (h5 + f) | 0;
    h6 = (h6 + g) | 0;
    h7 = (h7 + h) | 0;
  }

  const out = new Uint8Array(32);
  const words = [h0, h1, h2, h3, h4, h5, h6, h7];
  for (let i = 0; i < 8; i++) {
    out[i * 4] = (words[i] >>> 24) & 0xff;
    out[i * 4 + 1] = (words[i] >>> 16) & 0xff;
    out[i * 4 + 2] = (words[i] >>> 8) & 0xff;
    out[i * 4 + 3] = words[i] & 0xff;
  }
  return out;
}

const BASE64_CHARS = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

export function base64UrlEncodeBytes(bytes: Uint8Array): string {
  let result = "";
  const len = bytes.length;
  for (let i = 0; i < len; i += 3) {
    const b0 = bytes[i];
    const b1 = i + 1 < len ? bytes[i + 1] : 0;
    const b2 = i + 2 < len ? bytes[i + 2] : 0;
    const triplet = (b0 << 16) | (b1 << 8) | b2;
    result += BASE64_CHARS[(triplet >>> 18) & 0x3f];
    result += BASE64_CHARS[(triplet >>> 12) & 0x3f];
    result += i + 1 < len ? BASE64_CHARS[(triplet >>> 6) & 0x3f] : "";
    result += i + 2 < len ? BASE64_CHARS[triplet & 0x3f] : "";
  }
  return result.replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "");
}

function randomRandomBytes(length: number): Uint8Array {
  const bytes = new Uint8Array(length);
  if (typeof globalThis !== "undefined" && globalThis.crypto?.getRandomValues) {
    globalThis.crypto.getRandomValues(bytes);
    return bytes;
  }
  for (let i = 0; i < length; i++) {
    bytes[i] = Math.floor(Math.random() * 256);
  }
  return bytes;
}

export interface PkcePair {
  codeVerifier: string;
  codeChallenge: string;
  method: "S256";
}

/**
 * 生成 RFC 7636 S256 PKCE (code_verifier, code_challenge) 对（参考 opengeos/GeoAgent `generate_pkce_pair`，MIT）
 */
export function generatePkcePair(customVerifier?: string): PkcePair {
  const codeVerifier =
    customVerifier && customVerifier.trim().length >= 43
      ? customVerifier.trim()
      : base64UrlEncodeBytes(randomRandomBytes(32));
  const digest = sha256Bytes(codeVerifier);
  const codeChallenge = base64UrlEncodeBytes(digest);
  return {
    codeVerifier,
    codeChallenge,
    method: "S256",
  };
}

export function generateOAuthState(): string {
  return base64UrlEncodeBytes(randomRandomBytes(16));
}

export interface BuildOAuthAuthorizeUrlInput {
  provider: OAuthPkceProviderPreset;
  codeChallenge: string;
  state: string;
  redirectUriOverride?: string;
}

/**
 * 构建浏览器 OAuth PKCE S256 授权跳转地址（参考 opengeos/GeoAgent `build_authorize_url` & OpenRouter PKCE，MIT）
 */
export function buildOAuthAuthorizationUrl(input: BuildOAuthAuthorizeUrlInput): string {
  const { provider, codeChallenge, state, redirectUriOverride } = input;
  const redirectUri = redirectUriOverride?.trim() || provider.redirect_uri;

  if (provider.id === "openrouter_pkce") {
    const params = new URLSearchParams({
      callback_url: redirectUri,
      code_challenge: codeChallenge,
      code_challenge_method: "S256",
      state,
    });
    return `${provider.authorize_url}?${params.toString()}`;
  }

  const params = new URLSearchParams({
    response_type: "code",
    client_id: provider.client_id,
    redirect_uri: redirectUri,
    scope: provider.scopes,
    code_challenge: codeChallenge,
    code_challenge_method: "S256",
    state,
    ...(provider.extra_params ?? {}),
  });
  return `${provider.authorize_url}?${params.toString()}`;
}

export interface ExtractedOAuthCallback {
  ok: boolean;
  code?: string;
  state?: string;
  error?: string;
}

/**
 * 解析并校验浏览器 OAuth 回调 URL 或授权码（参考 opengeos/GeoAgent `_extract_authorization_code`，MIT）
 */
export function extractOAuthAuthorizationCode(
  rawCallbackInput: string,
  expectedState?: string
): ExtractedOAuthCallback {
  const trimmed = rawCallbackInput.trim();
  if (!trimmed) {
    return { ok: false, error: "未输入回调链接或授权码" };
  }

  // 若直接粘贴了纯 code 或 code#state 且不含 URL 特征
  if (!trimmed.includes("://") && !trimmed.includes("?") && !trimmed.includes("=")) {
    const hashIdx = trimmed.indexOf("#");
    if (hashIdx >= 0) {
      const code = trimmed.slice(0, hashIdx).trim();
      const state = trimmed.slice(hashIdx + 1).trim();
      if (!code) {
        return { ok: false, error: "授权码不能为空" };
      }
      if (expectedState && (!state || state !== expectedState)) {
        return { ok: false, error: "OAuth state 校验不匹配或缺失，已拒绝潜在伪造回调" };
      }
      return { ok: true, code, state: state || undefined };
    }
    return { ok: true, code: trimmed };
  }

  try {
    const qIndex = trimmed.indexOf("?");
    const normalizedInput =
      qIndex >= 0
        ? `${trimmed.slice(0, qIndex + 1)}${trimmed.slice(qIndex + 1).replace(/\?/g, "&")}`
        : trimmed;
    const url = normalizedInput.includes("://")
      ? new URL(normalizedInput)
      : new URL(
          normalizedInput.startsWith("?")
            ? `http://localhost${normalizedInput}`
            : `http://localhost?${normalizedInput}`
        );
    const err = url.searchParams.get("error");
    if (err) {
      const desc = url.searchParams.get("error_description") || err;
      return { ok: false, error: `OAuth 授权失败: ${desc}` };
    }
    const state = url.searchParams.get("state") || undefined;
    if (expectedState && (!state || state !== expectedState)) {
      return { ok: false, error: "OAuth state 校验不匹配或缺失，已拒绝潜在伪造回调" };
    }
    const code = url.searchParams.get("code")?.trim();
    if (!code) {
      return { ok: false, error: "回调地址中缺少 authorization code 参数" };
    }
    return { ok: true, code, state };
  } catch {
    return { ok: false, error: "无法解析回调 URL 格式" };
  }
}

/**
 * 解码 JWT Payload（参考 opengeos/GeoAgent `decode_jwt_payload`，MIT，支持 Bearer 前缀剥离与 UTF-8 安全解码）
 */
export function decodeJwtPayload(token: string): Record<string, unknown> {
  const normalized = token.trim().replace(/^Bearer\s+/i, "").trim();
  const parts = normalized.split(".");
  if (parts.length < 2) return {};
  try {
    const base64 = parts[1].replace(/-/g, "+").replace(/_/g, "/");
    const padded = base64 + "=".repeat((4 - (base64.length % 4)) % 4);
    let jsonStr = "";
    if (typeof atob === "function") {
      const bin = atob(padded);
      const bytes = new Uint8Array(bin.length);
      for (let i = 0; i < bin.length; i++) {
        bytes[i] = bin.charCodeAt(i) & 0xff;
      }
      jsonStr =
        typeof TextDecoder !== "undefined"
          ? new TextDecoder("utf-8").decode(bytes)
          : bin;
    }
    if (!jsonStr) return {};
    const parsed = JSON.parse(jsonStr);
    return parsed && typeof parsed === "object" && !Array.isArray(parsed)
      ? (parsed as Record<string, unknown>)
      : {};
  } catch {
    return {};
  }
}

/**
 * 从 OAuth Token 响应或 JWT 中提取 ChatGPT Account ID（参考 opengeos/GeoAgent `extract_chatgpt_account_id`，MIT）
 */
export function extractChatGptAccountId(
  tokenOrPayload: string | Record<string, unknown>
): string | null {
  if (typeof tokenOrPayload === "string") {
    const claims = decodeJwtPayload(tokenOrPayload);
    return extractChatGptAccountId(claims);
  }

  const flatClaim = tokenOrPayload[`${JWT_OPENAI_AUTH_CLAIM}.chatgpt_account_id`];
  if (typeof flatClaim === "string" && flatClaim.trim()) {
    return flatClaim.trim();
  }

  const direct = tokenOrPayload.chatgpt_account_id ?? tokenOrPayload.account_id;
  if (typeof direct === "string" && direct.trim()) {
    return direct.trim();
  }

  const authClaim = tokenOrPayload[JWT_OPENAI_AUTH_CLAIM];
  if (authClaim && typeof authClaim === "object") {
    const nested = (authClaim as Record<string, unknown>).chatgpt_account_id;
    if (typeof nested === "string" && nested.trim()) {
      return nested.trim();
    }
  }

  for (const key of ["access_token", "id_token"] as const) {
    const rawJwt = tokenOrPayload[key];
    if (typeof rawJwt === "string" && rawJwt.includes(".")) {
      const claims = decodeJwtPayload(rawJwt);
      const fromClaims = extractChatGptAccountId(claims);
      if (fromClaims) return fromClaims;
    }
  }

  return null;
}

/**
 * 判断 OAuth 凭据是否缺失或即将过期（默认提前 300 秒刷新，参考 opengeos/GeoAgent `token_expires_soon`，MIT）
 */
export function isOAuthTokenExpiringSoon(
  expiresAtSec: number | string | null | undefined,
  nowSec: number = Math.floor(Date.now() / 1000),
  skewSec: number = 300
): boolean {
  if (expiresAtSec === null || expiresAtSec === undefined || expiresAtSec === "") {
    return true;
  }
  const expiry = typeof expiresAtSec === "number" ? expiresAtSec : Number(expiresAtSec);
  if (!Number.isFinite(expiry) || expiry <= 0) {
    return true;
  }
  return expiry <= nowSec + skewSec;
}

// ============================================================================
// P3. 多因子空间优先四维景排序 (Multi-Factor 4D Spatial Scene Ranking)
// ============================================================================

export type LonLatTuple = [number, number];
export type GeoBbox = [number, number, number, number]; // [west, south, east, north]

export interface GeoPolygonLike {
  type: "Polygon" | "MultiPolygon";
  coordinates: unknown;
}

export interface RankableSpatialScene {
  id: string;
  datetime: string;
  cloud_cover?: number | null;
  bbox?: GeoBbox | null;
  geometry?: GeoPolygonLike | null;
  properties?: Record<string, unknown> | null;
  [key: string]: unknown;
}

export interface RankedSpatialScene<T extends RankableSpatialScene = RankableSpatialScene> {
  scene: T;
  contains_query_center: boolean;
  bbox_overlap_ratio: number;
  effective_cloud_cover: number;
}

const LOW_CLOUD_PHRASES = [
  "cloud-free",
  "cloud free",
  "cloudfree",
  "least cloudy",
  "lowest cloud",
  "low cloud",
  "clear sky",
  "minimal cloud",
  "无云",
  "少云",
  "晴空",
  "云量低",
  "最低云量",
] as const;

/**
 * 识别自然语言请求是否包含“低云量 / 无云优先”意图（移植自 opengeos/GeoAgent `_is_low_cloud_request`，MIT）
 */
export function isLowCloudRequest(queryText?: string | null, maxCloudCover?: number | null): boolean {
  if (typeof maxCloudCover === "number" && Number.isFinite(maxCloudCover)) {
    return true;
  }
  if (!queryText) return false;
  const normalized = queryText.toLowerCase();
  return LOW_CLOUD_PHRASES.some((phrase) => normalized.includes(phrase));
}

/**
 * 计算候选池扩容大小（参考 opengeos/GeoAgent `search_stac`，MIT：当开启空间/低云优选时，将候选池放大至 >= 50 景再重排截断）
 */
export function computeCandidatePoolLimit(
  requestedLimit: number,
  cloudOrSpatialAware: boolean,
  minPoolSize: number = 50
): number {
  const safeLimit = Math.max(1, Math.floor(requestedLimit));
  if (!cloudOrSpatialAware) return safeLimit;
  return Math.max(safeLimit, minPoolSize);
}

/**
 * 2D 射线法判断点是否在闭合环内（含边界检测，移植自 opengeos/GeoAgent `_point_in_ring`，MIT）
 */
export function pointInRing(point: LonLatTuple, ring: readonly LonLatTuple[]): boolean {
  if (!ring || ring.length < 3) return false;
  const [x, y] = point;
  let inside = false;

  for (let i = 0; i < ring.length; i++) {
    const [x1, y1] = ring[i];
    const [x2, y2] = ring[(i + 1) % ring.length];

    if (x === x1 && y === y1) return true;

    const intersects =
      y1 > y !== y2 > y &&
      x <= ((x2 - x1) * (y - y1)) / (y2 - y1 || 1e-12) + x1;
    if (intersects) {
      inside = !inside;
    }
  }
  return inside;
}

/**
 * 判断 GeoJSON Polygon / MultiPolygon（支持内部孔洞 Holes）是否包含目标点（移植自 opengeos/GeoAgent `_geometry_contains_point`，MIT）
 */
export function geometryContainsPoint(
  geometry: GeoPolygonLike | null | undefined,
  point: LonLatTuple | null | undefined
): boolean {
  if (!geometry || !point) return false;

  if (geometry.type === "Polygon") {
    const rings = geometry.coordinates as LonLatTuple[][];
    if (!Array.isArray(rings) || rings.length === 0) return false;
    if (!pointInRing(point, rings[0])) return false;
    for (let i = 1; i < rings.length; i++) {
      if (pointInRing(point, rings[i])) return false;
    }
    return true;
  }

  if (geometry.type === "MultiPolygon") {
    const polygons = geometry.coordinates as LonLatTuple[][][];
    if (!Array.isArray(polygons)) return false;
    for (const rings of polygons) {
      if (Array.isArray(rings) && rings.length > 0 && pointInRing(point, rings[0])) {
        let inHole = false;
        for (let i = 1; i < rings.length; i++) {
          if (pointInRing(point, rings[i])) {
            inHole = true;
            break;
          }
        }
        if (!inHole) return true;
      }
    }
  }

  return false;
}

function bboxContainsPoint(bbox: GeoBbox | null | undefined, point: LonLatTuple | null | undefined): boolean {
  if (!bbox || bbox.length !== 4 || !point) return false;
  const [west, south, east, north] = bbox;
  const [lon, lat] = point;
  return west <= lon && lon <= east && south <= lat && lat <= north;
}

function bboxArea(bbox: GeoBbox | null | undefined): number {
  if (!bbox || bbox.length !== 4) return 0;
  return Math.max(0, bbox[2] - bbox[0]) * Math.max(0, bbox[3] - bbox[1]);
}

function bboxIntersectionArea(a: GeoBbox | null | undefined, b: GeoBbox | null | undefined): number {
  if (!a || a.length !== 4 || !b || b.length !== 4) return 0;
  const west = Math.max(a[0], b[0]);
  const south = Math.max(a[1], b[1]);
  const east = Math.min(a[2], b[2]);
  const north = Math.min(a[3], b[3]);
  if (east <= west || north <= south) return 0;
  return (east - west) * (north - south);
}

/**
 * 计算候选景 BBox 对查询 AOI BBox 的覆盖面积占比（intersection_area / query_area，保留 6 位小数）
 */
export function computeBboxOverlapRatio(
  itemBbox: GeoBbox | null | undefined,
  queryBbox: GeoBbox | null | undefined
): number {
  const qArea = bboxArea(queryBbox);
  if (qArea <= 0) return 0;
  const inter = bboxIntersectionArea(itemBbox, queryBbox);
  return Number(Math.min(1, inter / qArea).toFixed(6));
}

function extractSceneCloudCover(scene: RankableSpatialScene): number {
  if (typeof scene.cloud_cover === "number" && Number.isFinite(scene.cloud_cover)) {
    return scene.cloud_cover;
  }
  const props = scene.properties;
  if (props && typeof props === "object") {
    for (const key of ["eo:cloud_cover", "s2:cloudy_pixel_percentage", "cloud_cover"] as const) {
      const raw = props[key];
      const num = typeof raw === "number" ? raw : typeof raw === "string" && raw.trim() ? Number(raw) : NaN;
      if (Number.isFinite(num)) {
        return num;
      }
    }
  }
  return Number.POSITIVE_INFINITY;
}

export interface SpatialRankOptions {
  queryBbox?: GeoBbox | null;
  queryCenter?: LonLatTuple | null;
  preferLowCloud?: boolean;
  limit?: number;
}

/**
 * 四维空间优先景排序（移植/改编自 opengeos/GeoAgent `geoagent/tools/stac.py`，MIT）：
 * 排序优先级：
 *   1. `contains_query_center`（真实足迹 Polygon/MultiPolygon 或 BBox 是否覆盖 AOI 中心点，避免边缘擦边假阳性景排第一）
 *   2. `bbox_overlap_ratio`（与查询 AOI 的重叠面积占比，降序）
 *   3. 当 `preferLowCloud` 为 true（默认）时，`cloud_cover`（云量升序，无云量排后）优先于 `datetime`；
 *      当 `preferLowCloud` 为 false 时，最新 `datetime` 优先于 `cloud_cover`。
 */
export function rankScenesBySpatialFit<T extends RankableSpatialScene>(
  scenes: readonly T[],
  options: SpatialRankOptions = {}
): RankedSpatialScene<T>[] {
  const queryBbox = options.queryBbox ?? null;
  const preferLowCloud = options.preferLowCloud ?? true;
  const queryCenter: LonLatTuple | null =
    options.queryCenter ??
    (queryBbox && queryBbox.length === 4
      ? [(queryBbox[0] + queryBbox[2]) / 2, (queryBbox[1] + queryBbox[3]) / 2]
      : null);

  const enriched: RankedSpatialScene<T>[] = scenes.map((scene) => {
    const containsCenter = scene.geometry
      ? geometryContainsPoint(scene.geometry, queryCenter)
      : bboxContainsPoint(scene.bbox, queryCenter);
    const overlapRatio = computeBboxOverlapRatio(scene.bbox, queryBbox);
    const effectiveCloud = extractSceneCloudCover(scene);

    return {
      scene,
      contains_query_center: containsCenter,
      bbox_overlap_ratio: overlapRatio,
      effective_cloud_cover: effectiveCloud,
    };
  });

  enriched.sort((a, b) => {
    if (a.contains_query_center !== b.contains_query_center) {
      return a.contains_query_center ? -1 : 1;
    }
    if (b.bbox_overlap_ratio !== a.bbox_overlap_ratio) {
      return b.bbox_overlap_ratio - a.bbox_overlap_ratio;
    }
    if (!preferLowCloud) {
      const dtCmp = b.scene.datetime.localeCompare(a.scene.datetime);
      if (dtCmp !== 0) return dtCmp;
    }
    if (a.effective_cloud_cover !== b.effective_cloud_cover) {
      return a.effective_cloud_cover - b.effective_cloud_cover;
    }
    return b.scene.datetime.localeCompare(a.scene.datetime);
  });

  if (typeof options.limit === "number" && options.limit > 0) {
    return enriched.slice(0, options.limit);
  }
  return enriched;
}

// ============================================================================
// P4. 软取消同轮重规划反馈、脚本安全静态门禁与可复现实验脚本导出
// ============================================================================

export interface SoftCancelToolFeedbackInput {
  toolId: string;
  planId?: string;
  reasonZh?: string;
}

export interface SoftCancelToolFeedbackResult {
  status: "cancelled_by_user";
  tool_id: string;
  plan_id: string | null;
  cancel_tool_message: string;
  replan_hint_zh: string;
}

/**
 * 构建软取消重规划反馈（移植/改编自 opengeos/GeoAgent `geoagent/core/confirmation_hook.py`，MIT）：
 * 当用户在确认卡点击「拒绝 / 取消」时，不抛出致命异常中断对话，而是向 Agent 返回结构化软取消信号，
 * 引导模型在同一轮次内调整过滤条件或询问用户修正参数。
 */
export function buildSoftCancelToolFeedback(
  input: SoftCancelToolFeedbackInput
): SoftCancelToolFeedbackResult {
  const reason = input.reasonZh?.trim() || "用户在确认卡中拒绝了本次执行计划";
  return {
    status: "cancelled_by_user",
    tool_id: input.toolId,
    plan_id: input.planId ?? null,
    cancel_tool_message: `Action '${input.toolId}' was cancelled by the user (${reason}). Do not retry the exact same parameters; ask for updated constraints or propose an alternative read-only step.`,
    replan_hint_zh: `已取消「${input.toolId}」执行（${reason}）。Agent 将保留当前会话上下文并等待您调整空间范围、云量或时间条件后重新规划。`,
  };
}

/**
 * 压缩超大工具返回结果（移植/改编自 opengeos/GeoAgent `geoagent/core/safety.py` `_compact_tool_result`，MIT），防止海量景列表撑爆上下文
 */
export function compactToolResultForContext(
  value: unknown,
  maxListItems: number = 20,
  maxStringChars: number = 20_000
): unknown {
  if (typeof value === "string") {
    if (value.length <= maxStringChars) return value;
    return `${value.slice(0, maxStringChars)}... [truncated ${value.length - maxStringChars} chars]`;
  }
  if (Array.isArray(value)) {
    const sliced = value
      .slice(0, maxListItems)
      .map((item) => compactToolResultForContext(item, maxListItems, maxStringChars));
    if (value.length > maxListItems) {
      sliced.push(`... [${value.length - maxListItems} more items truncated]`);
    }
    return sliced;
  }
  if (value && typeof value === "object") {
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(value as Record<string, unknown>)) {
      out[k] = compactToolResultForContext(v, maxListItems, maxStringChars);
    }
    return out;
  }
  return value;
}

const FORBIDDEN_IMPORTS = new Set([
  "os",
  "subprocess",
  "sys",
  "socket",
  "http",
  "urllib",
  "requests",
  "shutil",
  "pickle",
  "ctypes",
  "multiprocessing",
  "pathlib",
  "importlib",
  "builtins",
  "pty",
  "marshal",
]);

const ALLOWED_PYQGIS_TOP_MODULES = new Set(["qgis", "PyQt5", "PyQt6", "math"]);
const ALLOWED_GEE_TOP_MODULES = new Set(["ee", "math"]);

const FORBIDDEN_CALLS = new Set([
  "eval",
  "exec",
  "compile",
  "open",
  "__import__",
  "input",
  "getattr",
  "setattr",
  "delattr",
  "globals",
  "locals",
  "vars",
]);

const FORBIDDEN_DUNDER_NAMES = new Set([
  "__builtins__",
  "__dict__",
  "__globals__",
  "__subclasses__",
  "__class__",
  "__bases__",
  "__mro__",
  "__code__",
]);

const FORBIDDEN_GEE_SYNC_CALLS = new Set([
  "getInfo",
  "computeValue",
  "reduceRegion",
  "reduceRegions",
  "Download",
  "Export",
]);

export interface GeoScriptSafetyReport {
  safe: boolean;
  violations: string[];
}

/**
 * 地理处理脚本静态安全校验器（移植/改编自 opengeos/GeoAgent `_validate_pyqgis_script` 与 `_validate_gee_python_snippet`，MIT）
 */
export function validateGeoScriptSafety(
  code: string,
  mode: "pyqgis" | "gee" = "pyqgis"
): GeoScriptSafetyReport {
  const violations: string[] = [];
  const trimmed = code.trim();
  if (!trimmed) {
    return { safe: false, violations: ["脚本内容不能为空"] };
  }

  const allowedTop = mode === "pyqgis" ? ALLOWED_PYQGIS_TOP_MODULES : ALLOWED_GEE_TOP_MODULES;

  // 检查所有 import a, b 与 from x import y 语句（支持单行多模块逗号分隔与相对导入拦截）
  const lines = trimmed.split(/\r?\n/);
  for (const rawLine of lines) {
    const line = rawLine.replace(/#.*$/, "").trim();
    if (!line) continue;

    const fromMatch = /^from\s+(\.*[a-zA-Z0-9_.]*)\s+import\s+(.+)$/.exec(line);
    if (fromMatch) {
      const modSpec = fromMatch[1];
      if (modSpec.startsWith(".")) {
        violations.push(`禁止使用相对路径导入: ${modSpec}`);
      } else {
        const rootMod = modSpec.split(".")[0];
        if (FORBIDDEN_IMPORTS.has(rootMod) || !allowedTop.has(rootMod)) {
          violations.push(`禁止导入系统/非白名单模块: ${rootMod}`);
        }
      }
      continue;
    }

    const impMatch = /^import\s+(.+)$/.exec(line);
    if (impMatch) {
      const specs = impMatch[1].split(",");
      for (const spec of specs) {
        const cleanMod = spec.trim().split(/\s+as\s+/)[0].trim();
        if (!cleanMod) continue;
        const rootMod = cleanMod.split(".")[0];
        if (FORBIDDEN_IMPORTS.has(rootMod) || !allowedTop.has(rootMod)) {
          violations.push(`禁止导入系统/非白名单模块: ${rootMod}`);
        }
      }
    }
  }

  // 检查危险内建调用
  for (const fnName of FORBIDDEN_CALLS) {
    const callRegex = new RegExp(`\\b${fnName}\\s*\\(`, "g");
    if (callRegex.test(trimmed)) {
      violations.push(`禁止调用高危函数: ${fnName}()`);
    }
  }

  // 检查属性或独立标识符形式的双下划线内省访问 (__builtins__, .__subclasses__, __globals__, __dict__ 等)
  const seenDunders = new Set<string>();
  const dunderAttrRegex = /\.__([a-zA-Z0-9_]+)__/g;
  let match: RegExpExecArray | null;
  while ((match = dunderAttrRegex.exec(trimmed)) !== null) {
    const dunder = `__${match[1]}__`;
    if (!seenDunders.has(dunder)) {
      seenDunders.add(dunder);
      violations.push(`禁止访问双下划线内省属性: ${dunder}`);
    }
  }
  for (const dunderName of FORBIDDEN_DUNDER_NAMES) {
    if (!seenDunders.has(dunderName) && new RegExp(`\\b${dunderName}\\b`).test(trimmed)) {
      seenDunders.add(dunderName);
      violations.push(`禁止访问双下划线内省标识符: ${dunderName}`);
    }
  }

  if (mode === "gee") {
    for (const geeCall of FORBIDDEN_GEE_SYNC_CALLS) {
      const geeRegex = new RegExp(`\\.${geeCall}\\b`, "g");
      if (geeRegex.test(trimmed)) {
        violations.push(`GEE 图层片段禁止同步阻塞或导出调用: .${geeCall}`);
      }
    }
  }

  return {
    safe: violations.length === 0,
    violations,
  };
}

export interface ReproducibleRecipeInput {
  resourceId: string;
  bbox: GeoBbox;
  timeRange: { start: string; end: string };
  maxCloudCover?: number | null;
  selectedSceneIds?: readonly string[];
  outputDir?: string;
  includeOlmoEarthCompanion?: boolean;
  olmoEarthMatryoshkaDim?: OlmoEarthMatryoshkaDim;
}

function normalizeGeoBbox(bbox: GeoBbox): {
  west: number;
  south: number;
  east: number;
  north: number;
  minLat: number;
  maxLat: number;
  centerLon: number;
  centerLat: number;
  lonSpanDeg: number;
  latSpanDeg: number;
  crossesAntimeridian: boolean;
} {
  const [rawWest, rawSouth, rawEast, rawNorth] = bbox;
  const west = Number.isFinite(rawWest) ? rawWest : 0;
  const east = Number.isFinite(rawEast) ? rawEast : west;
  const south = Number.isFinite(rawSouth) ? rawSouth : 0;
  const north = Number.isFinite(rawNorth) ? rawNorth : south;

  const minLat = Math.max(-90, Math.min(90, Math.min(south, north)));
  const maxLat = Math.max(-90, Math.min(90, Math.max(south, north)));
  const centerLat = (minLat + maxLat) / 2;
  const latSpanDeg = maxLat - minLat;

  const crossesAntimeridian = west > east && west <= 180 && east >= -180;
  let lonSpanDeg: number;
  let unwrappedCenterLon: number;
  if (crossesAntimeridian) {
    lonSpanDeg = Math.max(0, Math.min(360, (180 - west) + (east + 180)));
    unwrappedCenterLon = west + lonSpanDeg / 2;
  } else {
    lonSpanDeg = Math.max(0, Math.min(360, Math.abs(east - west)));
    unwrappedCenterLon = (west + east) / 2;
  }

  let centerLon: number;
  if (unwrappedCenterLon === 180) {
    centerLon = 180;
  } else {
    centerLon = ((((unwrappedCenterLon + 180) % 360) + 360) % 360) - 180;
    if (centerLon === -180 && unwrappedCenterLon > 0) {
      centerLon = 180;
    }
  }

  return {
    west,
    south,
    east,
    north,
    minLat,
    maxLat,
    centerLon,
    centerLat,
    lonSpanDeg,
    latSpanDeg,
    crossesAntimeridian,
  };
}

/**
 * 导出零密钥、可独立复现的 Python 检索与验证脚本（结合 opengeos/GeoAgent 可复现科研工作流思路与 EOStage 参数设计）
 */
export function exportReproduciblePythonRecipe(input: ReproducibleRecipeInput): string {
  const norm = normalizeGeoBbox(input.bbox);
  const centerLon = Number(norm.centerLon.toFixed(6));
  const centerLat = Number(norm.centerLat.toFixed(6));
  const sceneListJson = JSON.stringify(input.selectedSceneIds ?? [], null, 2);
  const cloudFilterLine =
    typeof input.maxCloudCover === "number"
      ? `    query={"eo:cloud_cover": {"lt": ${input.maxCloudCover}}},\n`
      : "";

  const baseLines = [
    "# EOStage Reproducible Geospatial Recipe (Zero-Secret Export)",
    `# Resource: ${input.resourceId}`,
    "from pystac_client import Client",
    "",
    `AOI_BBOX = [${norm.west}, ${norm.south}, ${norm.east}, ${norm.north}]`,
    `AOI_CENTER = (${centerLon}, ${centerLat})`,
    `TIME_RANGE = "${input.timeRange.start}/${input.timeRange.end}"`,
    `FROZEN_SCENE_IDS = ${sceneListJson}`,
    "",
    'catalog = Client.open("https://planetarycomputer.microsoft.com/api/stac/v1")',
    "search = catalog.search(",
    `    collections=["${input.resourceId}"],`,
    "    bbox=AOI_BBOX,",
    "    datetime=TIME_RANGE,",
    cloudFilterLine ? cloudFilterLine.trimEnd() : "    max_items=50,",
    ")",
    "items = list(search.items())",
    'print(f"Matched {len(items)} candidate scenes around center {AOI_CENTER}")',
  ];

  if (input.includeOlmoEarthCompanion) {
    baseLines.push(
      "",
      exportOlmoEarthCompanionPythonRecipe({
        bbox: input.bbox,
        year: Number(input.timeRange.start.slice(0, 4)) || 2024,
        matryoshkaDim: input.olmoEarthMatryoshkaDim ?? 64,
      })
    );
  }

  return baseLines.join("\n");
}

// ============================================================================
// P5. OlmoEarth v1.3 预计算特征分片规划、非线性反量化与观测级跨源溯源辅助
// ============================================================================

export type OlmoEarthMatryoshkaDim = 64 | 128;

export const OLMOEARTH_ARTIFACT_LICENSE_NOTICE_ZH =
  "许可证合规提示：allenai/olmoearth_precomputed 代码采用 Apache-2.0，但预计算特征与模型权重受《OlmoEarth Artifact License》约束，明文禁止用于军事、武器、监控（Surveillance）以及化石燃料与采矿提取行业（Extractive Industries）。EOStage 仅将其作为可选科学伴随配方，不替代原始 SAR/光学/DEM 物理观测。";

export const OLMOEARTH_ARTIFACT_LICENSE_NOTICE_EN =
  "License Notice: OlmoEarth precomputed embeddings and model weights are governed by the OlmoEarth Artifact License (prohibiting military, weapons, surveillance, and fossil-fuel/mining extractive industry uses). Provided by EOStage as an optional auxiliary companion recipe.";

export const OLMOEARTH_V13_SPECS = {
  version: "0.1.3",
  sourceCoopHttpsRoot: "https://data.source.coop/allenai/olmoearth-embeddings",
  sourceCoopS3Root: "s3://us-west-2.opendata.source.coop/allenai/olmoearth-embeddings",
  stacCatalogUrl:
    "https://data.source.coop/allenai/olmoearth-embeddings/v1.3/2024/collection.json",
  resolutionMeters: 10,
  outerShardShape: [1, 128, 2048, 2048] as const,
  innerChunkShape: [1, 64, 256, 256] as const,
  innerChunkGridShape: [1, 2, 8, 8] as const,
  outerShardSpanMeters: 20_480,
  innerChunkSpanMeters: 2_560,
  shardTailIndexBytes: 2_052, // 128 * 16B ([offset_u64_le, nbytes_u64_le]) + 4B crc32c
  nodataInt8: -128,
  validInt8Range: [-127, 127] as const,
  dequantScaleDivisor: 127.5,
  matryoshkaDims: [64, 128] as const,
} as const;

export interface OlmoEarthShardWindowPlan {
  utmZone: number;
  /** OlmoEarth v1.3 全球统一采用北半球 EPSG:32601..32660 编码，南半球使用负值北坐标 */
  epsgCode: string;
  hemisphere: "N" | "S";
  usesNegativeNorthing: boolean;
  centerEastingMeters: number;
  centerNorthingMeters: number;
  bboxSpanMeters: { widthMeters: number; heightMeters: number };
  matryoshkaDim: OlmoEarthMatryoshkaDim;
  channelInnerChunkSlices: 1 | 2;
  outerShardCount: number;
  spatialInnerChunkCount: number;
  selectedInnerChunkCount: number;
  tailIndexBytesTotal: number;
  estimatedTransferBytes: number;
  estimatedFull128DTransferBytes: number;
  savedBytesPctVs128D: number;
  rangeMergePolicy: {
    maximum_gap_bytes: 0;
    maximum_extra_bytes: 0;
    shard_tail_index_bytes: 2052;
  };
  licenseNoticeZh: string;
}

/**
 * 规划 OlmoEarth v1.3 UTM 分片窗口与零间隙 Range 字节预算：
 * - 严格实现 OlmoEarth 的南半球坐标约定：全球固定使用 `EPSG:32601..32660`（不切 `EPSG:327NN`），
 *   当纬度 `< 0`（含跨赤道 AOI 的南侧部分）时直接采用负值北坐标（`y < 0`），消除赤道假北偏移接缝；
 * - 支持跨国际日期变更线（`west > east`）AOI 的真实经度跨度与中心分带计算；
 * - 计算外层 Shard (`20.48 km × 20.48 km`)、内层 Chunk (`2.56 km × 2.56 km`) 与尾部 `2052B` 索引开销；
 * - 支持 Matryoshka `64D`（仅拉取通道轴第 1 个内层块 `c=0..64`）与 `128D`（拉取 `c=0..128` 两个通道块）对比。
 */
export function planOlmoEarthUtmShardWindow(
  bbox: GeoBbox,
  options?: {
    matryoshkaDim?: OlmoEarthMatryoshkaDim;
    avgCompressedInnerChunkBytes?: number;
  }
): OlmoEarthShardWindowPlan {
  const norm = normalizeGeoBbox(bbox);
  const matryoshkaDim: OlmoEarthMatryoshkaDim = options?.matryoshkaDim === 128 ? 128 : 64;
  const rawChunkBytes = options?.avgCompressedInnerChunkBytes;
  const avgChunkBytes = Math.max(
    4_096,
    Math.floor(
      typeof rawChunkBytes === "number" && Number.isFinite(rawChunkBytes)
        ? rawChunkBytes
        : 320 * 1024
    )
  );

  const normalizedLon = norm.centerLon;
  const utmZone = Math.min(60, Math.max(1, Math.floor((normalizedLon + 180) / 6) + 1));
  const epsgCode = `EPSG:326${String(utmZone).padStart(2, "0")}`;
  const hemisphere: "N" | "S" = norm.centerLat < 0 ? "S" : "N";
  // 只要 AOI 南界或中心延伸至赤道以南 (minLat < 0)，在 EPSG:326NN 下即涉及负值北坐标 (y < 0)
  const usesNegativeNorthing = norm.minLat < 0 || norm.centerLat < 0;

  const centralMeridianDeg = (utmZone - 1) * 6 - 180 + 3;
  const latRad = (norm.centerLat * Math.PI) / 180;
  const metersPerDegLat = 111_132.92 - 559.82 * Math.cos(2 * latRad);
  const metersPerDegLon = Math.max(1_000, 111_412.84 * Math.cos(latRad));

  const centerEastingMeters = Math.round(
    500_000 + (normalizedLon - centralMeridianDeg) * metersPerDegLon
  );
  // 南半球直接采用相对于赤道的负值北坐标 (y < 0)，不加 10,000,000 m 假北偏移
  const centerNorthingMeters = Math.round(norm.centerLat * metersPerDegLat);

  const widthMeters = Math.max(10, Math.round(norm.lonSpanDeg * metersPerDegLon));
  const heightMeters = Math.max(10, Math.round(norm.latSpanDeg * metersPerDegLat));

  const outerCols = Math.max(
    1,
    Math.ceil(widthMeters / OLMOEARTH_V13_SPECS.outerShardSpanMeters)
  );
  const outerRows = Math.max(
    1,
    Math.ceil(heightMeters / OLMOEARTH_V13_SPECS.outerShardSpanMeters)
  );
  const outerShardCount = outerCols * outerRows;

  const innerCols = Math.max(
    1,
    Math.ceil(widthMeters / OLMOEARTH_V13_SPECS.innerChunkSpanMeters)
  );
  const innerRows = Math.max(
    1,
    Math.ceil(heightMeters / OLMOEARTH_V13_SPECS.innerChunkSpanMeters)
  );
  const spatialInnerChunkCount = innerCols * innerRows;

  const channelInnerChunkSlices: 1 | 2 = matryoshkaDim === 64 ? 1 : 2;
  const selectedInnerChunkCount = spatialInnerChunkCount * channelInnerChunkSlices;
  const tailIndexBytesTotal = outerShardCount * OLMOEARTH_V13_SPECS.shardTailIndexBytes;

  const estimatedTransferBytes =
    selectedInnerChunkCount * avgChunkBytes + tailIndexBytesTotal;
  const estimatedFull128DTransferBytes =
    spatialInnerChunkCount * 2 * avgChunkBytes + tailIndexBytesTotal;

  const savedBytesPctVs128D =
    estimatedFull128DTransferBytes > 0 && estimatedTransferBytes < estimatedFull128DTransferBytes
      ? Number(
          (
            ((estimatedFull128DTransferBytes - estimatedTransferBytes) /
              estimatedFull128DTransferBytes) *
            100
          ).toFixed(1)
        )
      : 0;

  return {
    utmZone,
    epsgCode,
    hemisphere,
    usesNegativeNorthing,
    centerEastingMeters,
    centerNorthingMeters,
    bboxSpanMeters: { widthMeters, heightMeters },
    matryoshkaDim,
    channelInnerChunkSlices,
    outerShardCount,
    spatialInnerChunkCount,
    selectedInnerChunkCount,
    tailIndexBytesTotal,
    estimatedTransferBytes,
    estimatedFull128DTransferBytes,
    savedBytesPctVs128D,
    rangeMergePolicy: {
      maximum_gap_bytes: 0,
      maximum_extra_bytes: 0,
      shard_tail_index_bytes: 2052,
    },
    licenseNoticeZh: OLMOEARTH_ARTIFACT_LICENSE_NOTICE_ZH,
  };
}

/**
 * OlmoEarth v1.3 单值 `int8` 非线性有符号平方根反量化函数：
 * - `q === -128`（或非有限数值）严格映射为 `NaN`（`fill_value = -128`）；
 * - 有效值 `q ∈ [-127, 127]` 按官方公式 `((abs(q) / 127.5) ** 2) * sign(q)` 还原为浮点数，
 *   绝不使用错误的线性 `q / 127.5` 缩放。
 */
export function dequantizeOlmoEarthInt8(q: number): number {
  if (!Number.isFinite(q)) return Number.NaN;
  const intVal = Math.trunc(q);
  if (intVal <= OLMOEARTH_V13_SPECS.nodataInt8 || intVal < -127 || intVal > 127) {
    return Number.NaN;
  }
  if (intVal === 0) return 0;
  const normalized = Math.abs(intVal) / OLMOEARTH_V13_SPECS.dequantScaleDivisor;
  return normalized * normalized * Math.sign(intVal);
}

/**
 * 批量反量化 OlmoEarth `int8` 特征向量，并支持按 Matryoshka 前缀维度（`64` 或 `128`）截断切片
 */
export function dequantizeOlmoEarthVector(
  quantized: ArrayLike<number>,
  matryoshkaDim?: OlmoEarthMatryoshkaDim
): Float32Array {
  const safeInputLen = Math.max(0, Math.trunc(quantized?.length ?? 0));
  const targetLen =
    typeof matryoshkaDim === "number" && Number.isFinite(matryoshkaDim)
      ? Math.min(safeInputLen, Math.max(0, Math.trunc(matryoshkaDim)))
      : safeInputLen;
  const out = new Float32Array(targetLen);
  for (let i = 0; i < targetLen; i++) {
    out[i] = dequantizeOlmoEarthInt8(quantized[i]);
  }
  return out;
}

export interface OlmoEarthObservationCrosswalkInput<
  T extends RankableSpatialScene = RankableSpatialScene,
> {
  bbox: GeoBbox;
  timeRange: { start: string; end: string };
  maxCloudCover?: number;
  candidateScenes?: readonly T[];
  matryoshkaDim?: OlmoEarthMatryoshkaDim;
}

export interface OlmoEarthCrosswalkSearchRecommendation {
  resourceId:
    | "science.sentinel-2.l2a.v1"
    | "science.landsat.c2-l2.v1"
    | "science.sentinel-1.standard.v1";
  modality: "optical_10m" | "optical_30m" | "sar_10m";
  roleZh: string;
  bbox: GeoBbox;
  timeRange: { start: string; end: string };
  maxCloudCover?: number;
}

export interface OlmoEarthObservationCrosswalkResult<
  T extends RankableSpatialScene = RankableSpatialScene,
> {
  shardPlan: OlmoEarthShardWindowPlan;
  rankedLowCloudCandidates: RankedSpatialScene<T>[];
  recommendedRawSearches: OlmoEarthCrosswalkSearchRecommendation[];
  provenanceWarningZh: string;
  licenseNoticeZh: string;
}

/**
 * 构建「OlmoEarth 年度特征图块 ↔ 原始物理观测景（Sentinel-2 / Landsat / Sentinel-1）」跨源溯源映射：
 * - 由于预计算特征将 12 个月度时间窗聚合为单个 `int8` 隐向量且抹除了单景 `scene_id` 与 SAR 相位，
 *   本辅助函数为给定 AOI 同步生成分片规划、低云候选景四维排序（优先满足 `maxCloudCover` 阈值的合格低云景），
 *   以及 EOStage 权威三源检索参数建议。
 */
export function buildOlmoEarthObservationCrosswalk<
  T extends RankableSpatialScene = RankableSpatialScene,
>(
  input: OlmoEarthObservationCrosswalkInput<T>
): OlmoEarthObservationCrosswalkResult<T> {
  const maxCloud =
    typeof input.maxCloudCover === "number" && Number.isFinite(input.maxCloudCover)
      ? Math.max(0, Math.min(100, input.maxCloudCover))
      : 15;
  const shardPlan = planOlmoEarthUtmShardWindow(input.bbox, {
    matryoshkaDim: input.matryoshkaDim ?? 64,
  });
  const rankedAll = rankScenesBySpatialFit(input.candidateScenes ?? [], {
    queryBbox: input.bbox,
    preferLowCloud: true,
  });
  const qualifyingLowCloud = rankedAll.filter(
    (item) => item.effective_cloud_cover <= maxCloud
  );
  const exceedingCloud = rankedAll.filter(
    (item) => item.effective_cloud_cover > maxCloud
  );
  const rankedLowCloudCandidates = [...qualifyingLowCloud, ...exceedingCloud];

  const recommendedRawSearches: OlmoEarthCrosswalkSearchRecommendation[] = [
    {
      resourceId: "science.sentinel-2.l2a.v1",
      modality: "optical_10m",
      roleZh: "对齐 OlmoEarth 10m 光学主干（12 波段地表反射率低云原始景复核）",
      bbox: input.bbox,
      timeRange: input.timeRange,
      maxCloudCover: maxCloud,
    },
    {
      resourceId: "science.landsat.c2-l2.v1",
      modality: "optical_30m",
      roleZh: "对齐 OlmoEarth 30m 光学/热红外辅助模态（长时序辐射定标验证）",
      bbox: input.bbox,
      timeRange: input.timeRange,
      maxCloudCover: maxCloud,
    },
    {
      resourceId: "science.sentinel-1.standard.v1",
      modality: "sar_10m",
      roleZh: "对齐 OlmoEarth 10m C 波段 VV/VH 后向散射（云雨期全天候观测与干涉相位获取）",
      bbox: input.bbox,
      timeRange: input.timeRange,
    },
  ];

  return {
    shardPlan,
    rankedLowCloudCandidates,
    recommendedRawSearches,
    provenanceWarningZh:
      "科学溯源提示：OlmoEarth v1.3 预计算特征为 12 个月度窗口聚合的 int8 潜空间表征，不含逐像元原始景号 (scene_id) 与 SAR 复数相位；定量反演与 InSAR 形变解算请使用上方映射的原始物理观测数据源。",
    licenseNoticeZh: OLMOEARTH_ARTIFACT_LICENSE_NOTICE_ZH,
  };
}

/**
 * 生成带《OlmoEarth Artifact License》合规声明、南半球 `EPSG:326NN` 负北坐标转换、
 * Zarr v3 分片零间隙 (`maximum_gap_bytes=0`) 与非线性反量化的独立 Python 伴随脚本
 */
export function exportOlmoEarthCompanionPythonRecipe(input: {
  bbox: GeoBbox;
  year?: number;
  matryoshkaDim?: OlmoEarthMatryoshkaDim;
}): string {
  const rawYear =
    typeof input.year === "number" && Number.isFinite(input.year)
      ? Math.trunc(input.year)
      : 2024;
  const year = rawYear >= 1970 && rawYear <= 2100 ? rawYear : 2024;
  const dim: OlmoEarthMatryoshkaDim = input.matryoshkaDim === 128 ? 128 : 64;
  const plan = planOlmoEarthUtmShardWindow(input.bbox, { matryoshkaDim: dim });

  return [
    "# ----------------------------------------------------------------------------",
    "# EOStage x OlmoEarth v1.3 Precomputed Embedding Companion Recipe",
    `# ${OLMOEARTH_ARTIFACT_LICENSE_NOTICE_EN}`,
    `# Target UTM Grid: ${plan.epsgCode} (Hemisphere=${plan.hemisphere}, NegativeNorthing=${plan.usesNegativeNorthing})`,
    `# Shard Layout: Outer=1x128x2048x2048, Inner=1x64x256x256, TailIndex=${OLMOEARTH_V13_SPECS.shardTailIndexBytes}B, RangeGap=0B`,
    "# ----------------------------------------------------------------------------",
    "import numpy as np",
    "",
    `OLMOEARTH_STAC_URL = "${OLMOEARTH_V13_SPECS.stacCatalogUrl.replace("2024", String(year))}"`,
    `OLMOEARTH_UTM_EPSG = "${plan.epsgCode}"  # Southern Hemisphere uses negative northing in EPSG:326NN`,
    `OLMOEARTH_CENTER_UTM_M = (${plan.centerEastingMeters}, ${plan.centerNorthingMeters})`,
    `MATRYOSHKA_DIM = ${dim}  # Slice first ${dim} channels (saves ${plan.savedBytesPctVs128D}% vs full 128D)`,
    "",
    "def dequantize_olmoearth_int8(q_int8: np.ndarray) -> np.ndarray:",
    '    """Non-linear signed square-root dequantization (NoData = -128 -> NaN)."""',
    "    sliced = q_int8[:, :MATRYOSHKA_DIM, ...] if q_int8.ndim >= 4 else q_int8[:MATRYOSHKA_DIM, ...]",
    "    q = sliced.astype(np.float32)",
    "    nodata_mask = (sliced == -128)",
    "    emb = ((np.abs(q) / 127.5) ** 2) * np.sign(q)",
    "    emb[nodata_mask] = np.nan",
    "    return emb",
  ].join("\n");
}


