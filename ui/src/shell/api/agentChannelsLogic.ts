/**
 * Agent 模型渠道纯逻辑模块
 * 无外部 IO 依赖，方便单元测试与状态编排
 */

import type {
  AgentChannelView,
  AgentChannelSave,
  AgentChannelTest,
  AgentReasoningMode,
} from "./agentBridgeTypes.ts";
import type { AgentChannelModel } from "../../lib/generated/v3-contracts.ts";
import { channelModels, normalizeChannelModels } from "./channelModels.ts";
import { t } from "../../i18n/index.ts";
import { formatErrorWithCode } from "./errorExplanations.ts";

export type ChannelProtocolValue =
  | "chat_completions"
  | "responses"
  | "anthropic_messages"
  | "google_gemini";

export interface ChannelProtocolOption {
  value: ChannelProtocolValue;
  label_zh: string;
}

export const CHANNEL_PROTOCOLS: readonly ChannelProtocolOption[] = [
  { value: "anthropic_messages", label_zh: "Anthropic Messages (/v1/messages)" },
  { value: "chat_completions", label_zh: "Chat Completions (/chat/completions)" },
  { value: "google_gemini", label_zh: "Google Gemini API (/v1beta)" },
  { value: "responses", label_zh: "Responses (/responses)" },
] as const;

export interface ChannelDraft {
  id: string;
  protocol: ChannelProtocolValue;
  base_url: string;
  model: string;
  models: AgentChannelModel[];
  reasoning_mode: AgentReasoningMode;
  apiKeyInput: string;
  key_ready: boolean;
  is_default: boolean;
}

export function newChannelDraft(id?: string): ChannelDraft {
  const generatedId = id || `chan-${Date.now().toString(36)}`;
  return {
    id: generatedId,
    protocol: "chat_completions",
    base_url: "",
    model: "",
    models: [],
    reasoning_mode: "off",
    apiKeyInput: "",
    key_ready: false,
    is_default: false,
  };
}

export function channelDraftFromView(view: AgentChannelView): ChannelDraft {
  return {
    id: view.id,
    protocol: (view.protocol as ChannelProtocolValue) || "chat_completions",
    base_url: view.base_url,
    model: view.model,
    models: channelModels(view),
    reasoning_mode: view.reasoning_mode ?? "off",
    apiKeyInput: "",
    key_ready: view.key_ready,
    is_default: view.is_default,
  };
}

export function buildChannelSavePayload(draft: {
  id: string;
  protocol: string;
  base_url: string;
  model: string;
  models?: AgentChannelModel[];
  reasoning_mode?: string | null;
  apiKeyInput?: string | null;
  [key: string]: unknown;
}): AgentChannelSave {
  const id = draft.id?.trim();
  if (!id) {
    throw new Error(t("agent.channels.idRequired"));
  }
  if (id.length > 64) {
    throw new Error(t("agent.channels.idTooLong"));
  }
  if (!/^[a-zA-Z0-9_-]+$/.test(id)) {
    throw new Error(t("agent.channels.idInvalid"));
  }

  const protocol = draft.protocol?.trim();
  const validProtocols = CHANNEL_PROTOCOLS.map((p) => p.value);
  if (!validProtocols.includes(protocol as ChannelProtocolValue)) {
    throw new Error(t("agent.channels.unsupportedProtocol"));
  }

  const baseUrl = draft.base_url?.trim();
  if (!baseUrl) {
    throw new Error(t("agent.channels.baseUrlRequired"));
  }

  const rawModels = draft.models;
  const nonEmptyFirstModel = rawModels?.find((m) => m.id?.trim())?.id.trim() ?? "";
  let model = draft.model?.trim() ?? "";
  if (!model && nonEmptyFirstModel) {
    model = nonEmptyFirstModel;
  }
  const allowEmpty = Boolean((draft as any).allowEmptyModel) && (!rawModels || rawModels.length === 0);
  if (!model && !allowEmpty) {
    throw new Error(t("agent.channels.modelRequired"));
  }

  const reasoningMode = (draft as any).reasoning_mode;
  if (reasoningMode !== "default" && reasoningMode !== "off") {
    throw new Error(t("agent.channels.unsupportedReasoning"));
  }

  const payload: AgentChannelSave = {
    id,
    protocol,
    base_url: baseUrl,
    model,
    ...(rawModels ? { models: normalizeChannelModels(model, rawModels) } : {}),
    reasoning_mode: reasoningMode,
    api_key: null,
  };

  const key = draft.apiKeyInput?.trim();
  if (key) {
    payload.api_key = key;
  } else {
    delete (payload as any).api_key;
  }

  return payload;
}

export interface ChannelViewDisplay {
  id: string;
  protocol: string;
  protocol_zh: string;
  base_url: string;
  model: string;
  reasoning_mode: AgentReasoningMode;
  reasoning_mode_zh: string;
  key_ready: boolean;
  key_ready_zh: string;
  is_default: boolean;
  is_default_zh: string;
}

export function projectChannelView(view: AgentChannelView): ChannelViewDisplay {
  const protocolObj = CHANNEL_PROTOCOLS.find((p) => p.value === view.protocol);
  const protocol_zh = protocolObj ? protocolObj.label_zh : view.protocol;
  const key_ready_zh = view.key_ready ? t("agent.channels.keyReady") : t("agent.channels.keyMissing");
  const is_default_zh = view.is_default ? t("agent.channels.default") : t("agent.channels.alternate");
  const reasoning_mode: AgentReasoningMode = view.reasoning_mode === "default" ? "default" : "off";
  const reasoning_mode_zh = reasoning_mode === "default" ? t("agent.channels.reasoningDefault") : t("agent.channels.reasoningOff");

  return {
    id: view.id,
    protocol: view.protocol,
    protocol_zh,
    base_url: view.base_url,
    model: view.model,
    reasoning_mode,
    reasoning_mode_zh,
    key_ready: view.key_ready,
    key_ready_zh,
    is_default: view.is_default,
    is_default_zh,
  };
}

export function reasoningOffRefusedNote(refused: boolean | null | undefined): string | null {
  return refused === true ? t("agent.channels.reasoningOffRefused") : null;
}

export function formatChannelTestResult(result: AgentChannelTest): string {
  if (result.ok) {
    if (typeof result.model_count === "number") {
      return t("agent.channels.testModels", { count: result.model_count });
    }
    return t("agent.channels.testSuccess");
  }

  if (result.status === 401 || result.status === 403) return t("agent.channels.authFailed", { status: result.status });
  if (result.status === 404) return t("agent.channels.notFound");
  if (result.status === 408 || result.code === "model_timeout") return t("agent.channels.timeout");
  if (!result.status && result.code === "model_transient") return t("agent.channels.networkUnavailable");
  const parts: string[] = [];
  if (result.code) {
    parts.push(formatErrorWithCode(result.code));
  } else {
    parts.push(t("agent.channels.connectionFailed"));
  }

  if (result.status) {
    parts.push(`(HTTP ${result.status})`);
  }

  return parts.join(" ");
}

export interface ChannelModelsFetchResult {
  models: string[];
  fallbackToManual: boolean;
  message_zh?: string;
}

export async function fetchChannelModels(
  fetcher: (channelId: string) => Promise<string[]>,
  channelId: string,
  timeoutMs: number = 8000
): Promise<ChannelModelsFetchResult> {
  let timer: ReturnType<typeof setTimeout> | undefined;

  const timeoutPromise = new Promise<ChannelModelsFetchResult>((resolve) => {
    timer = setTimeout(() => {
      resolve({
        models: [],
        fallbackToManual: true,
        message_zh: t("agent.channels.modelsTimeout"),
      });
    }, timeoutMs);
  });

  const requestPromise = (async () => {
    try {
      const models = await fetcher(channelId);
      if (timer) clearTimeout(timer);
      return {
        models: Array.isArray(models) ? models : [],
        fallbackToManual: false,
        message_zh:
          models && models.length > 0
            ? t("agent.channels.modelsLoaded", { count: models.length })
            : t("agent.channels.modelsEmpty"),
      };
    } catch (err: any) {
      if (timer) clearTimeout(timer);
      return {
        models: [],
        fallbackToManual: true,
        message_zh: t("agent.channels.modelsFailed", { error: err?.message || t("agent.channels.networkError") }),
      };
    }
  })();

  return Promise.race([requestPromise, timeoutPromise]);
}

export function baseUrlNeedsV1Hint(
  url: string | null | undefined,
  protocol?: string | null
): boolean {
  if (protocol === "google_gemini") return false;
  const trimmed = typeof url === "string" ? url.trim() : "";
  if (!trimmed) return false;
  return !/(\/v1|\/v1beta|\/openai)\/?$/i.test(trimmed);
}

export function appendV1ToBaseUrl(url: string): string {
  const trimmed = url.trim();
  if (!trimmed) return trimmed;
  if (/\/v1\/?$/i.test(trimmed)) {
    return trimmed;
  }
  return `${trimmed.replace(/\/+$/, "")}/v1`;
}

/** 渠道卡小标签：主机记下该渠道拒绝过「关闭思考」（AgentChannelView.reasoning_switch_unsupported）。 */
export function reasoningSwitchUnsupportedZh(view: { reasoning_switch_unsupported?: boolean }): string | null {
  return view?.reasoning_switch_unsupported === true ? t("agent.channels.reasoningSwitchUnsupported") : null;
}

export type ChannelAuthKind = "official" | "subscription" | "api";

export interface ChannelAuthMeta {
  channel_id: string;
  auth_kind: ChannelAuthKind;
  oauth_provider?: string | null;
  account_id?: string | null;
}

export interface SubscriptionQuotaWindow {
  used_percent: number;
  remaining_percent: number;
  window_minutes?: number | null;
  reset_at_secs?: number | null;
}

export interface SubscriptionChannelQuota {
  channel_id: string;
  oauth_provider: string;
  plan_type?: string | null;
  account_id?: string | null;
  primary_window?: SubscriptionQuotaWindow | null;
  secondary_window?: SubscriptionQuotaWindow | null;
  reset_credits_available?: number | null;
  credits_has_credits?: boolean | null;
  credits_unlimited?: boolean | null;
  credits_balance?: string | null;
  fetched_at_ms: number;
}

/**
 * Classifies whether a channel uses Official credits, ChatGPT/Codex subscription OAuth,
 * or standard API key billing. Prefers authoritative Rust secret metadata when available
 * (so renamed Codex channels keep `"subscription"` and OpenRouter PKCE stays `"api"`),
 * with an offline structural heuristic for fixtures and non-Tauri hosts.
 */
export function resolveChannelAuthKind(
  channel: { id?: string | null; base_url?: string | null },
  authMetaMap?: Record<string, ChannelAuthMeta>
): ChannelAuthKind {
  const id = (channel.id || "").trim();
  const idLower = id.toLowerCase();
  if (idLower === "eostage-official") {
    return "official";
  }
  const meta = id && authMetaMap ? authMetaMap[id] : undefined;
  if (meta?.auth_kind === "official" || meta?.auth_kind === "subscription" || meta?.auth_kind === "api") {
    return meta.auth_kind;
  }
  const urlLower = (channel.base_url || "").toLowerCase();
  if (urlLower.includes("chatgpt.com/backend-api")) {
    return "subscription";
  }
  if (idLower.includes("openrouter")) {
    return "api";
  }
  if (
    idLower.includes("codex") ||
    idLower.includes("chatgpt") ||
    idLower.includes("claude-oauth") ||
    idLower.includes("claude-code") ||
    idLower.includes("github-copilot")
  ) {
    return "subscription";
  }
  return "api";
}

export async function fetchChannelAuthMetaMap(): Promise<Record<string, ChannelAuthMeta>> {
  if (typeof window === "undefined") return {};
  const internals = (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ as
    | { invoke?: unknown }
    | undefined;
  if (!internals || typeof internals.invoke !== "function") return {};
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    const list = await invoke<ChannelAuthMeta[]>("agent_oauth_channels_meta");
    if (!Array.isArray(list)) return {};
    const map: Record<string, ChannelAuthMeta> = {};
    for (const item of list) {
      if (item?.channel_id) {
        map[item.channel_id] = item;
      }
    }
    return map;
  } catch {
    return {};
  }
}
