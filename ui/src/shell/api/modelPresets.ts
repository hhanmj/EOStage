/**
 * 模型能力预设表 (Model Capabilities Presets)
 * 融合实时公开模型元数据目录 (models.dev + OpenRouter) 与 2025-2026 主流大模型家族内置规则：
 * 自动匹配上下文窗口、最大输出 tokens、工具调用、多模态视觉输入与思考推理档位。
 */

import type { AgentChannelModel, AgentReasoningLevel } from "../../lib/generated/v3-contracts.ts";
import { lookupPublicModelSpec } from "./publicModelRegistry.ts";

export interface ModelPresetRule {
  /** 匹配模式：正则 */
  pattern: RegExp;
  displayName?: string;
  context_window: number;
  max_output_tokens: number;
  supports_tools: boolean;
  supports_image: boolean;
  supports_video?: boolean;
  supports_pdf?: boolean;
  reasoning_levels?: AgentReasoningLevel[];
  /** 说明或注意事项 */
  note?: string;
}

const STANDARD_REASONING_LEVELS: AgentReasoningLevel[] = [
  { level: "off", value: null },
  { level: "low", value: "low" },
  { level: "medium", value: "medium" },
  { level: "high", value: "high" },
];

/**
 * 权威公开模型能力预设表（优先由 publicModelRegistry 实时元数据命中，离线或未命中时回退至本表）
 */
export const MODEL_PRESET_RULES: readonly ModelPresetRule[] = [
  // --- DeepSeek 系 (2025-2026: V4.1 Flash / V4 / V3.2 / R1-0528 / V3 128K+) ---
  {
    pattern: /^deepseek-flash(?:-.*)?$/i,
    displayName: "DeepSeek V4.1 Flash",
    context_window: 1048576,
    max_output_tokens: 393216,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: STANDARD_REASONING_LEVELS,
  },
  {
    pattern: /^deepseek-v4(?:-.*)?$/i,
    displayName: "DeepSeek V4",
    context_window: 1048576,
    max_output_tokens: 131072,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: STANDARD_REASONING_LEVELS,
  },
  {
    pattern: /^deepseek-v3\.[12](?:-.*)?$/i,
    displayName: "DeepSeek V3.2",
    context_window: 131072,
    max_output_tokens: 65536,
    supports_tools: true,
    supports_image: false,
    reasoning_levels: STANDARD_REASONING_LEVELS,
  },
  {
    pattern: /^(?:deepseek-r1(?:-distill.*|-0528)?|deepseek-reasoner)$/i,
    displayName: "DeepSeek R1",
    context_window: 131072,
    max_output_tokens: 65536,
    supports_tools: true,
    supports_image: false,
    reasoning_levels: [
      { level: "off", value: null },
      { level: "default", value: null },
      { level: "low", value: "low" },
      { level: "medium", value: "medium" },
      { level: "high", value: "high" },
    ],
  },
  {
    pattern: /^(?:deepseek-v3|deepseek-chat)$/i,
    displayName: "DeepSeek V3",
    context_window: 131072,
    max_output_tokens: 32768,
    supports_tools: true,
    supports_image: false,
    reasoning_levels: [],
  },
  {
    pattern: /deepseek/i,
    displayName: "DeepSeek",
    context_window: 131072,
    max_output_tokens: 32768,
    supports_tools: true,
    supports_image: false,
    reasoning_levels: [],
  },

  // --- Anthropic Claude 5 / 4.6 / 4.5 / 4 系 ---
  {
    pattern: /^claude-fable-5(?:\.\d+|-\d+)?(?:-.*)?$/i,
    displayName: "Claude Fable 5.2",
    context_window: 1000000,
    max_output_tokens: 128000,
    supports_tools: true,
    supports_image: true,
    supports_pdf: true,
    reasoning_levels: STANDARD_REASONING_LEVELS,
  },
  {
    pattern: /^claude-(?:(?:opus|sonnet|haiku)-)?5(?:-.*)?$/i,
    displayName: "Claude 5",
    context_window: 1000000,
    max_output_tokens: 128000,
    supports_tools: true,
    supports_image: true,
    supports_pdf: true,
    reasoning_levels: STANDARD_REASONING_LEVELS,
  },
  {
    pattern: /^claude-(?:opus|sonnet|haiku)-4-[56](?:-.*)?$/i,
    displayName: "Claude 4.5",
    context_window: 1000000,
    max_output_tokens: 64000,
    supports_tools: true,
    supports_image: true,
    supports_pdf: true,
    reasoning_levels: STANDARD_REASONING_LEVELS,
  },
  {
    pattern: /^claude-(?:(?:opus|sonnet)-)?4(?:-.*)?$/i,
    displayName: "Claude 4",
    context_window: 200000,
    max_output_tokens: 16384,
    supports_tools: true,
    supports_image: true,
    supports_pdf: true,
    reasoning_levels: STANDARD_REASONING_LEVELS,
  },

  // --- Anthropic Claude 3 / 3.5 / 3.7 系 ---
  {
    pattern: /^claude-3-7-sonnet(?:-\d{8}|-latest)?$/i,
    displayName: "Claude 3.7 Sonnet",
    context_window: 200000,
    max_output_tokens: 64000,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: STANDARD_REASONING_LEVELS,
  },
  {
    pattern: /^claude-3-5-sonnet(?:-\d{8}|-latest)?$/i,
    displayName: "Claude 3.5 Sonnet",
    context_window: 200000,
    max_output_tokens: 8192,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: [],
  },
  {
    pattern: /^claude-3-5-haiku(?:-\d{8}|-latest)?$/i,
    displayName: "Claude 3.5 Haiku",
    context_window: 200000,
    max_output_tokens: 8192,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: [],
  },
  {
    pattern: /^claude-3-haiku(?:-\d{8})?$/i,
    displayName: "Claude 3 Haiku",
    context_window: 200000,
    max_output_tokens: 4096,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: [],
  },
  {
    pattern: /^claude-3-opus(?:-\d{8})?$/i,
    displayName: "Claude 3 Opus",
    context_window: 200000,
    max_output_tokens: 4096,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: [],
  },
  {
    pattern: /claude/i,
    displayName: "Claude",
    context_window: 200000,
    max_output_tokens: 32768,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: [],
  },

  // --- Google Gemini 4 / 3 / 2.5 系 ---
  {
    pattern: /^gemini-[34](?:\.\d+)?(?:-.*)?$/i,
    displayName: "Gemini 3 / 4",
    context_window: 1048576,
    max_output_tokens: 65536,
    supports_tools: true,
    supports_image: true,
    supports_video: true,
    supports_pdf: true,
    reasoning_levels: STANDARD_REASONING_LEVELS,
  },
  {
    pattern: /^gemini-2\.5-(?:pro|flash)(?:-.*)?$/i,
    displayName: "Gemini 2.5",
    context_window: 1048576,
    max_output_tokens: 65536,
    supports_tools: true,
    supports_image: true,
    supports_video: true,
    supports_pdf: true,
    reasoning_levels: STANDARD_REASONING_LEVELS,
  },
  {
    pattern: /^gemini-2\.0-flash(?:-.*)?$/i,
    displayName: "Gemini 2.0 Flash",
    context_window: 1048576,
    max_output_tokens: 8192,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: STANDARD_REASONING_LEVELS,
  },
  {
    pattern: /^gemini-1\.5-pro(?:-.*)?$/i,
    displayName: "Gemini 1.5 Pro",
    context_window: 2097152,
    max_output_tokens: 8192,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: [],
  },
  {
    pattern: /^gemini-1\.5-flash(?:-.*)?$/i,
    displayName: "Gemini 1.5 Flash",
    context_window: 1048576,
    max_output_tokens: 8192,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: [],
  },
  {
    pattern: /gemini/i,
    displayName: "Gemini",
    context_window: 1048576,
    max_output_tokens: 65536,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: [],
  },

  // --- Agnes / Neohorse / 聚合多模态高速模型 ---
  {
    pattern: /^agnes(?:-.*)?$/i,
    displayName: "Agnes",
    context_window: 512000,
    max_output_tokens: 65536,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: STANDARD_REASONING_LEVELS,
  },
  {
    pattern: /^neohorse(?:-.*)?$/i,
    displayName: "Neohorse",
    context_window: 1048576,
    max_output_tokens: 65536,
    supports_tools: true,
    supports_image: true,
    supports_video: true,
    supports_pdf: true,
    reasoning_levels: STANDARD_REASONING_LEVELS,
  },

  // --- Qwen 3.8 / 3.5 / 3 / Max 系 ---
  {
    pattern: /^qwen-?3\.[5-9](?:-.*)?$/i,
    displayName: "Qwen 3.5+",
    context_window: 1000000,
    max_output_tokens: 65536,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: STANDARD_REASONING_LEVELS,
  },
  {
    pattern: /^qwen3-(?:max|coder)(?:-.*)?$/i,
    displayName: "Qwen 3 Max",
    context_window: 262144,
    max_output_tokens: 65536,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: STANDARD_REASONING_LEVELS,
  },
  {
    pattern: /^qwen-?3(?:-.*)?$/i,
    displayName: "Qwen 3",
    context_window: 131072,
    max_output_tokens: 8192,
    supports_tools: true,
    supports_image: false,
    reasoning_levels: STANDARD_REASONING_LEVELS,
  },
  {
    pattern: /^qwen-max(?:-.*)?$/i,
    displayName: "Qwen Max",
    context_window: 131072,
    max_output_tokens: 16384,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: [],
  },
  {
    pattern: /^qwen-plus(?:-.*)?$/i,
    displayName: "Qwen Plus",
    context_window: 131072,
    max_output_tokens: 16384,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: [],
  },
  {
    pattern: /^qwen-turbo(?:-.*)?$/i,
    displayName: "Qwen Turbo",
    context_window: 1000000,
    max_output_tokens: 16384,
    supports_tools: true,
    supports_image: false,
    reasoning_levels: [],
  },
  {
    pattern: /^qwen2\.5-vl(?:-.*)?$/i,
    displayName: "Qwen 2.5 VL",
    context_window: 131072,
    max_output_tokens: 8192,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: [],
  },
  {
    pattern: /^qwen2\.5(?:-.*)?$/i,
    displayName: "Qwen 2.5",
    context_window: 131072,
    max_output_tokens: 8192,
    supports_tools: true,
    supports_image: false,
    reasoning_levels: [],
  },
  {
    pattern: /qwen/i,
    displayName: "Qwen",
    context_window: 131072,
    max_output_tokens: 16384,
    supports_tools: true,
    supports_image: false,
    reasoning_levels: [],
  },

  // --- OpenAI GPT-6 / GPT-5 / GPT-4.1 / o4 / o3 / GPT-4o 系 ---
  {
    pattern: /^gpt-6(?:\.\d+)?(?:-.*)?$/i,
    displayName: "GPT-6",
    context_window: 1050000,
    max_output_tokens: 128000,
    supports_tools: true,
    supports_image: true,
    supports_pdf: true,
    reasoning_levels: STANDARD_REASONING_LEVELS,
  },
  {
    pattern: /^gpt-5(?:\.\d+)?(?:-.*)?$/i,
    displayName: "GPT-5",
    context_window: 400000,
    max_output_tokens: 128000,
    supports_tools: true,
    supports_image: true,
    supports_pdf: true,
    reasoning_levels: STANDARD_REASONING_LEVELS,
  },
  {
    pattern: /^gpt-4\.1(?:-.*)?$/i,
    displayName: "GPT-4.1",
    context_window: 1047576,
    max_output_tokens: 32768,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: [],
  },
  {
    pattern: /^o4(?:-mini)?(?:-.*)?$/i,
    displayName: "o4",
    context_window: 200000,
    max_output_tokens: 100000,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: [
      { level: "low", value: "low" },
      { level: "medium", value: "medium" },
      { level: "high", value: "high" },
    ],
  },
  {
    pattern: /^gpt-4o(?:-\d{4}-\d{2}-\d{2})?$/i,
    displayName: "GPT-4o",
    context_window: 128000,
    max_output_tokens: 16384,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: [],
  },
  {
    pattern: /^gpt-4o-mini(?:-\d{4}-\d{2}-\d{2})?$/i,
    displayName: "GPT-4o mini",
    context_window: 128000,
    max_output_tokens: 16384,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: [],
  },
  {
    pattern: /^o1(?:-\d{4}-\d{2}-\d{2})?$/i,
    displayName: "o1",
    context_window: 200000,
    max_output_tokens: 100000,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: [
      { level: "low", value: "low" },
      { level: "medium", value: "medium" },
      { level: "high", value: "high" },
    ],
  },
  {
    pattern: /^o1-preview(?:-\d{4}-\d{2}-\d{2})?$/i,
    displayName: "o1 Preview",
    context_window: 128000,
    max_output_tokens: 32768,
    supports_tools: false,
    supports_image: false,
    reasoning_levels: [
      { level: "low", value: "low" },
      { level: "medium", value: "medium" },
      { level: "high", value: "high" },
    ],
  },
  {
    pattern: /^o1-mini(?:-\d{4}-\d{2}-\d{2})?$/i,
    displayName: "o1 mini",
    context_window: 128000,
    max_output_tokens: 65536,
    supports_tools: false,
    supports_image: false,
    reasoning_levels: [
      { level: "low", value: "low" },
      { level: "medium", value: "medium" },
      { level: "high", value: "high" },
    ],
  },
  {
    pattern: /^o3-mini(?:-\d{4}-\d{2}-\d{2})?$/i,
    displayName: "o3 mini",
    context_window: 200000,
    max_output_tokens: 100000,
    supports_tools: true,
    supports_image: false,
    reasoning_levels: [
      { level: "low", value: "low" },
      { level: "medium", value: "medium" },
      { level: "high", value: "high" },
    ],
  },
  {
    pattern: /^o3(?:-\d{4}-\d{2}-\d{2})?$/i,
    displayName: "o3",
    context_window: 200000,
    max_output_tokens: 100000,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: [
      { level: "low", value: "low" },
      { level: "medium", value: "medium" },
      { level: "high", value: "high" },
    ],
  },
  {
    pattern: /^gpt-4-turbo(?:-\d{4}-\d{2}-\d{2})?$/i,
    displayName: "GPT-4 Turbo",
    context_window: 128000,
    max_output_tokens: 4096,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: [],
  },
  {
    pattern: /^gpt-4(?:-\d{4})?$/i,
    displayName: "GPT-4",
    context_window: 8192,
    max_output_tokens: 4096,
    supports_tools: true,
    supports_image: false,
    reasoning_levels: [],
  },

  // --- Zhipu GLM 5 / 4.6 / 4.5 / 4 系 ---
  {
    pattern: /^glm-5(?:\.\d+)?(?:-.*)?$/i,
    displayName: "GLM-5",
    context_window: 1000000,
    max_output_tokens: 131072,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: STANDARD_REASONING_LEVELS,
  },
  {
    pattern: /^glm-4\.[567](?:-.*)?$/i,
    displayName: "GLM-4.6",
    context_window: 200000,
    max_output_tokens: 65536,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: STANDARD_REASONING_LEVELS,
  },
  {
    pattern: /^glm-4-plus$/i,
    displayName: "GLM-4 Plus",
    context_window: 128000,
    max_output_tokens: 4096,
    supports_tools: true,
    supports_image: false,
    reasoning_levels: [],
  },
  {
    pattern: /^glm-4-flash$/i,
    displayName: "GLM-4 Flash",
    context_window: 128000,
    max_output_tokens: 4096,
    supports_tools: true,
    supports_image: false,
    reasoning_levels: [],
  },
  {
    pattern: /^glm-4v(?:-plus)?$/i,
    displayName: "GLM-4V",
    context_window: 128000,
    max_output_tokens: 4096,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: [],
  },
  {
    pattern: /glm/i,
    displayName: "GLM",
    context_window: 200000,
    max_output_tokens: 32768,
    supports_tools: true,
    supports_image: false,
    reasoning_levels: [],
  },

  // --- Moonshot Kimi K3 / K2 / v1 系 ---
  {
    pattern: /^kimi-k3(?:\.\d+)?(?:-.*)?$/i,
    displayName: "Kimi K3",
    context_window: 1048576,
    max_output_tokens: 131072,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: STANDARD_REASONING_LEVELS,
  },
  {
    pattern: /^kimi-k2(?:\.\d+)?(?:-.*)?$/i,
    displayName: "Kimi K2",
    context_window: 262144,
    max_output_tokens: 65536,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: STANDARD_REASONING_LEVELS,
  },
  {
    pattern: /^moonshot-v1-8k$/i,
    displayName: "Kimi 8K",
    context_window: 8192,
    max_output_tokens: 4096,
    supports_tools: true,
    supports_image: false,
    reasoning_levels: [],
  },
  {
    pattern: /^moonshot-v1-32k$/i,
    displayName: "Kimi 32K",
    context_window: 32768,
    max_output_tokens: 8192,
    supports_tools: true,
    supports_image: false,
    reasoning_levels: [],
  },
  {
    pattern: /^moonshot-v1-128k$/i,
    displayName: "Kimi 128K",
    context_window: 128000,
    max_output_tokens: 8192,
    supports_tools: true,
    supports_image: false,
    reasoning_levels: [],
  },
  {
    pattern: /^kimi-(?:k0|k1)(?:-.*)?$/i,
    displayName: "Kimi k-series",
    context_window: 128000,
    max_output_tokens: 8192,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: STANDARD_REASONING_LEVELS,
  },
  {
    pattern: /(?:moonshot|kimi)/i,
    displayName: "Kimi",
    context_window: 262144,
    max_output_tokens: 32768,
    supports_tools: true,
    supports_image: false,
    reasoning_levels: [],
  },

  // --- MiniMax M3 / M2 / Text-01 系 ---
  {
    pattern: /^minimax-m3(?:\.\d+)?(?:-.*)?$/i,
    displayName: "MiniMax M3",
    context_window: 1048576,
    max_output_tokens: 262144,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: STANDARD_REASONING_LEVELS,
  },
  {
    pattern: /^minimax-m2(?:\.\d+)?(?:-.*)?$/i,
    displayName: "MiniMax M2",
    context_window: 204800,
    max_output_tokens: 131072,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: STANDARD_REASONING_LEVELS,
  },
  {
    pattern: /^minimax(?:-.*)?$/i,
    displayName: "MiniMax",
    context_window: 1000000,
    max_output_tokens: 65536,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: [],
  },

  // --- xAI Grok 4 / 3 系 ---
  {
    pattern: /^grok-4\.7(?:-.*)?$/i,
    displayName: "Grok 4.7",
    context_window: 500000,
    max_output_tokens: 131072,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: STANDARD_REASONING_LEVELS,
  },
  {
    pattern: /^grok-4(?:\.\d+)?(?:-.*)?$/i,
    displayName: "Grok 4",
    context_window: 256000,
    max_output_tokens: 65536,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: STANDARD_REASONING_LEVELS,
  },
  {
    pattern: /^grok-3(?:\.\d+)?(?:-.*)?$/i,
    displayName: "Grok 3",
    context_window: 131072,
    max_output_tokens: 32768,
    supports_tools: true,
    supports_image: true,
    reasoning_levels: STANDARD_REASONING_LEVELS,
  },
];

/**
 * 现代默认兜底预设（针对未知自定义模型：Owner 2026-11-12 要求升级默认短窗口为现代 128K / 16K 默认值，且始终允许手动修改）
 */
export const CONSERVATIVE_FALLBACK_PRESET: ModelPresetRule = {
  pattern: /.*/,
  displayName: undefined,
  context_window: 128000,
  max_output_tokens: 16384,
  supports_tools: true,
  supports_image: false,
  reasoning_levels: [],
  note: "未命中公开目录时采用 128K 上下文默认配置，支持随时手动修改",
};

/**
 * 寻找与输入 ID 匹配的已知预设（优先查询已同步的公开模型库，其次匹配内置规则）
 */
export function matchSpecificPreset(
  modelId: string,
  protocol?: string,
): {
  preset: Partial<AgentChannelModel>;
  matched: boolean;
  source?: "public_api" | "builtin";
  supports_video?: boolean;
  supports_pdf?: boolean;
} {
  const trimmed = modelId.trim();
  if (!trimmed) {
    return { preset: {}, matched: false };
  }
  const publicHit = lookupPublicModelSpec(trimmed, protocol);
  if (publicHit) {
    return {
      preset: publicHit.preset,
      matched: true,
      source: "public_api",
      supports_video: publicHit.spec.supports_video,
      supports_pdf: publicHit.spec.supports_pdf,
    };
  }
  const pureId = trimmed.includes("/") ? trimmed.split("/").pop()! : trimmed;
  const matchedRule = MODEL_PRESET_RULES.find(
    (rule) => rule.pattern.test(pureId) || rule.pattern.test(trimmed),
  );
  if (!matchedRule) {
    return { preset: {}, matched: false };
  }
  return {
    preset: matchModelPreset(modelId, protocol),
    matched: true,
    source: "builtin",
    supports_video: matchedRule.supports_video,
    supports_pdf: matchedRule.supports_pdf,
  };
}

/**
 * 根据模型 ID 与渠道协议匹配预设能力配置
 */
export function matchModelPreset(
  modelId: string,
  protocol?: string,
): Partial<AgentChannelModel> {
  const trimmed = modelId.trim();
  if (!trimmed) {
    return {
      context_window: CONSERVATIVE_FALLBACK_PRESET.context_window,
      max_output_tokens: CONSERVATIVE_FALLBACK_PRESET.max_output_tokens,
      input_types: ["text"],
      reasoning_levels: [],
      supports_tools: true,
    };
  }

  const publicHit = lookupPublicModelSpec(trimmed, protocol);
  if (publicHit) {
    return publicHit.preset;
  }

  const pureId = trimmed.includes("/") ? trimmed.split("/").pop()! : trimmed;
  const matched = MODEL_PRESET_RULES.find(
    (rule) => rule.pattern.test(pureId) || rule.pattern.test(trimmed),
  );
  const preset = matched ?? CONSERVATIVE_FALLBACK_PRESET;

  let levels = preset.reasoning_levels ? [...preset.reasoning_levels] : [];
  if (levels.length > 0 && protocol === "anthropic_messages") {
    levels = [
      { level: "off", value: null },
      { level: "4000", value: "4000" },
      { level: "16000", value: "16000" },
    ];
  }

  return {
    ...(matched?.displayName ? { name: matched.displayName } : {}),
    context_window: preset.context_window,
    max_output_tokens: preset.max_output_tokens,
    input_types: preset.supports_image ? ["text", "image"] : ["text"],
    reasoning_levels: levels,
    supports_tools: preset.supports_tools,
  };
}

/**
 * 为已有的或新发现的模型套用预设（仅当对应字段未设置时才覆盖）
 */
export function applyModelPresetDefaults(
  model: AgentChannelModel,
  protocol?: string,
): AgentChannelModel {
  const preset = matchModelPreset(model.id, protocol);
  return {
    ...model,
    name: model.name || preset.name || null,
    context_window: model.context_window ?? preset.context_window ?? null,
    max_output_tokens: model.max_output_tokens ?? preset.max_output_tokens ?? null,
    input_types:
      model.input_types && model.input_types.length > 0
        ? model.input_types
        : (preset.input_types ?? ["text"]),
    reasoning_levels:
      model.reasoning_levels && model.reasoning_levels.length > 0
        ? model.reasoning_levels
        : (preset.reasoning_levels ?? []),
    supports_tools: model.supports_tools ?? preset.supports_tools ?? true,
  };
}
