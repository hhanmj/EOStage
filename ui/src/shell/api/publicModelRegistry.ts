/**
 * 公开大模型元数据同步服务 (Public Model Metadata Registry)
 *
 * 接入 GitHub 开源与主流网关社区通用的公开模型参数接口：
 * 1. https://models.dev/api.json（开源社区 sst/models.dev 维护，NewAPI / OpenCode / Lobe 等生态广泛采用，覆盖 230+ 供应商）
 * 2. https://openrouter.ai/api/v1/models（OpenRouter 实时公开模型目录，无需密钥）
 *
 * 自动提取上下文窗口 (context_window)、最大输出 (max_output_tokens)、工具调用 (supports_tools)、
 * 多模态输入 (text / image / video / pdf) 与推理能力 (reasoning)，本地缓存 24 小时。
 */

import type { AgentChannelModel, AgentReasoningLevel } from "../../lib/generated/v3-contracts.ts";
import { isTauriDesktopHost } from "../../lib/desktopShellBridge.ts";

export interface PublicModelSpec {
  id: string;
  provider?: string;
  name: string;
  context_window: number;
  max_output_tokens: number;
  supports_tools: boolean;
  supports_image: boolean;
  supports_video?: boolean;
  supports_pdf?: boolean;
  reasoning: boolean;
}

interface CachedRegistryPayload {
  version: 1;
  updatedAt: number;
  sourceSummary: string;
  items: PublicModelSpec[];
}

export interface PublicModelRegistryState {
  status: "idle" | "syncing" | "ready" | "error";
  count: number;
  updatedAt: number | null;
  sourceSummary: string | null;
  error: string | null;
}

const STORAGE_KEY = "eostage.public_model_registry.v1";
const CACHE_TTL_MS = 24 * 60 * 60 * 1000;
const MODELS_DEV_URL = "https://models.dev/api.json";
const OPENROUTER_MODELS_URL = "https://openrouter.ai/api/v1/models";

let exactIndex = new Map<string, PublicModelSpec>();
let slugIndex = new Map<string, PublicModelSpec>();
let catalogList: PublicModelSpec[] = [];
let hydratedFromStorage = false;
let inflightSync: Promise<PublicModelRegistryState> | null = null;
const listeners = new Set<() => void>();

let registryState: PublicModelRegistryState = {
  status: "idle",
  count: 0,
  updatedAt: null,
  sourceSummary: null,
  error: null,
};

function notifyListeners(): void {
  for (const listener of listeners) {
    try {
      listener();
    } catch {
      // ignore listener errors
    }
  }
}

function normalizeModelKey(raw: string): string {
  return raw.trim().toLowerCase();
}

function bareModelSlug(raw: string): string {
  const normalized = normalizeModelKey(raw);
  const afterSlash = normalized.includes("/") ? normalized.split("/").pop()! : normalized;
  return afterSlash.includes(":") ? afterSlash.split(":")[0]! : afterSlash;
}

function rebuildIndices(items: PublicModelSpec[]): void {
  const nextExact = new Map<string, PublicModelSpec>();
  const nextSlug = new Map<string, PublicModelSpec>();
  const deduped: PublicModelSpec[] = [];

  for (const item of items) {
    if (!item.id || item.context_window <= 0) continue;
    const fullKey = normalizeModelKey(item.id);
    const existing = nextExact.get(fullKey);
    if (!existing || item.context_window > existing.context_window) {
      nextExact.set(fullKey, item);
    }
    const slug = bareModelSlug(item.id);
    const existingSlug = nextSlug.get(slug);
    if (
      !existingSlug ||
      item.context_window > existingSlug.context_window ||
      (item.context_window === existingSlug.context_window &&
        item.max_output_tokens > existingSlug.max_output_tokens)
    ) {
      nextSlug.set(slug, item);
    }
  }

  for (const spec of nextExact.values()) {
    deduped.push(spec);
  }
  deduped.sort((a, b) => b.context_window - a.context_window || a.id.localeCompare(b.id));

  exactIndex = nextExact;
  slugIndex = nextSlug;
  catalogList = deduped;
}

function readStorageCache(): CachedRegistryPayload | null {
  if (typeof window === "undefined" || !window.localStorage) return null;
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as CachedRegistryPayload;
    if (parsed?.version !== 1 || !Array.isArray(parsed.items)) return null;
    return parsed;
  } catch {
    return null;
  }
}

function writeStorageCache(payload: CachedRegistryPayload): void {
  if (typeof window === "undefined" || !window.localStorage) return;
  try {
    // 仅缓存核心检索字段并控制在合理体积内，确保秒级加载且不触碰 localStorage 配额上限
    const compactItems = payload.items.slice(0, 2200);
    window.localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({ ...payload, items: compactItems }),
    );
  } catch {
    // ignore quota errors
  }
}

export function hydratePublicModelRegistryFromCache(): PublicModelRegistryState {
  if (hydratedFromStorage) return registryState;
  hydratedFromStorage = true;
  const cached = readStorageCache();
  if (cached && cached.items.length > 0) {
    rebuildIndices(cached.items);
    registryState = {
      status: "ready",
      count: catalogList.length,
      updatedAt: cached.updatedAt,
      sourceSummary: cached.sourceSummary,
      error: null,
    };
  }
  return registryState;
}

function parseModelsDevPayload(payload: unknown): PublicModelSpec[] {
  if (!payload || typeof payload !== "object") return [];
  const specs: PublicModelSpec[] = [];
  for (const [providerId, providerVal] of Object.entries(payload as Record<string, unknown>)) {
    if (!providerVal || typeof providerVal !== "object") continue;
    const pObj = providerVal as Record<string, unknown>;
    const providerName = typeof pObj.name === "string" ? pObj.name : providerId;
    const modelsObj = pObj.models;
    if (!modelsObj || typeof modelsObj !== "object") continue;

    for (const [modelKey, modelVal] of Object.entries(modelsObj as Record<string, unknown>)) {
      if (!modelVal || typeof modelVal !== "object") continue;
      const m = modelVal as Record<string, unknown>;
      const limit = (m.limit && typeof m.limit === "object" ? m.limit : {}) as Record<string, unknown>;
      const contextWindow = Number(limit.context ?? 0);
      if (!Number.isFinite(contextWindow) || contextWindow < 1024) continue;
      const rawOutput = Number(limit.output ?? 0);
      const maxOutput =
        Number.isFinite(rawOutput) && rawOutput >= 256
          ? rawOutput
          : Math.min(65536, Math.max(8192, Math.floor(contextWindow / 8)));
      const modalities = (
        m.modalities && typeof m.modalities === "object" ? m.modalities : {}
      ) as Record<string, unknown>;
      const inputs = Array.isArray(modalities.input)
        ? modalities.input.map((v) => String(v).toLowerCase())
        : ["text"];
      const id = typeof m.id === "string" && m.id.trim() ? m.id.trim() : modelKey.trim();
      if (!id) continue;
      const name = typeof m.name === "string" && m.name.trim() ? m.name.trim() : id;

      specs.push({
        id: id.includes("/") ? id : `${providerId}/${id}`,
        provider: providerName,
        name,
        context_window: contextWindow,
        max_output_tokens: maxOutput,
        supports_tools: m.tool_call !== false,
        supports_image: inputs.includes("image"),
        supports_video: inputs.includes("video"),
        supports_pdf: inputs.includes("pdf"),
        reasoning: Boolean(m.reasoning),
      });
    }
  }
  return specs;
}

function parseOpenRouterPayload(payload: unknown): PublicModelSpec[] {
  if (!payload || typeof payload !== "object") return [];
  const data = (payload as Record<string, unknown>).data;
  if (!Array.isArray(data)) return [];
  const specs: PublicModelSpec[] = [];
  for (const entry of data) {
    if (!entry || typeof entry !== "object") continue;
    const m = entry as Record<string, unknown>;
    const id = typeof m.id === "string" ? m.id.trim() : "";
    if (!id) continue;
    const topProvider = (
      m.top_provider && typeof m.top_provider === "object" ? m.top_provider : {}
    ) as Record<string, unknown>;
    const contextWindow = Number(m.context_length ?? topProvider.context_length ?? 0);
    if (!Number.isFinite(contextWindow) || contextWindow < 1024) continue;
    const rawMaxOut = Number(topProvider.max_completion_tokens ?? 0);
    const maxOutput =
      Number.isFinite(rawMaxOut) && rawMaxOut >= 256
        ? rawMaxOut
        : Math.min(65536, Math.max(8192, Math.floor(contextWindow / 8)));
    const arch = (
      m.architecture && typeof m.architecture === "object" ? m.architecture : {}
    ) as Record<string, unknown>;
    const inputMods = Array.isArray(arch.input_modalities)
      ? arch.input_modalities.map((v) => String(v).toLowerCase())
      : ["text"];
    const supportedParams = Array.isArray(m.supported_parameters)
      ? m.supported_parameters.map((v) => String(v).toLowerCase())
      : [];
    const rawName = typeof m.name === "string" && m.name.trim() ? m.name.trim() : id;
    const cleanName = rawName.includes(": ") ? rawName.split(": ").slice(1).join(": ").trim() : rawName;

    specs.push({
      id,
      provider: id.includes("/") ? id.split("/")[0] : "OpenRouter",
      name: cleanName || id,
      context_window: contextWindow,
      max_output_tokens: maxOutput,
      supports_tools: supportedParams.length === 0 || supportedParams.includes("tools"),
      supports_image: inputMods.includes("image"),
      supports_video: inputMods.includes("video"),
      supports_pdf: inputMods.includes("file") || inputMods.includes("pdf"),
      reasoning:
        supportedParams.includes("reasoning") ||
        supportedParams.includes("include_reasoning"),
    });
  }
  return specs;
}

function hasNativeTauriInvoke(): boolean {
  if (typeof window === "undefined") return false;
  const internals = (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ as
    | { invoke?: unknown }
    | undefined;
  return Boolean(internals && typeof internals.invoke === "function");
}

async function fetchJsonWithTimeout(url: string, timeoutMs = 8000): Promise<unknown> {
  if (typeof fetch !== "function") throw new Error("fetch unavailable");
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    const resp = await fetch(url, {
      method: "GET",
      headers: { Accept: "application/json" },
      signal: controller.signal,
    });
    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
    return await resp.json();
  } finally {
    clearTimeout(timer);
  }
}

export async function syncPublicModelRegistry(force = false): Promise<PublicModelRegistryState> {
  hydratePublicModelRegistryFromCache();
  if (
    !force &&
    registryState.status === "ready" &&
    registryState.updatedAt &&
    Date.now() - registryState.updatedAt < CACHE_TTL_MS &&
    catalogList.length > 0
  ) {
    return registryState;
  }
  if (inflightSync) return inflightSync;

  registryState = {
    ...registryState,
    status: "syncing",
    error: null,
  };
  notifyListeners();

  inflightSync = (async () => {
    try {
      if (isTauriDesktopHost() && hasNativeTauriInvoke()) {
        try {
          const { invoke } = await import("@tauri-apps/api/core");
          const nativeRes = await invoke<{
            items?: Array<Omit<PublicModelSpec, "provider"> & { provider?: string | null }>;
            source_summary?: string;
          }>("agent_sync_public_model_registry");
          if (nativeRes && Array.isArray(nativeRes.items) && nativeRes.items.length > 0) {
            const normalized: PublicModelSpec[] = nativeRes.items.map((item) => ({
              ...item,
              provider: item.provider ?? undefined,
            }));
            rebuildIndices(normalized);
            const updatedAt = Date.now();
            const sourceSummary = nativeRes.source_summary?.trim() || "models.dev + OpenRouter";
            writeStorageCache({
              version: 1,
              updatedAt,
              sourceSummary,
              items: catalogList,
            });
            registryState = {
              status: "ready",
              count: catalogList.length,
              updatedAt,
              sourceSummary,
              error: null,
            };
            notifyListeners();
            return registryState;
          }
        } catch {
          // Fall back to direct fetchJsonWithTimeout when native sync is unavailable
        }
      }

      const [modelsDevRes, openRouterRes] = await Promise.allSettled([
        fetchJsonWithTimeout(MODELS_DEV_URL, 9000),
        fetchJsonWithTimeout(OPENROUTER_MODELS_URL, 8000),
      ]);

      const merged: PublicModelSpec[] = [];
      const sources: string[] = [];

      if (modelsDevRes.status === "fulfilled") {
        const parsed = parseModelsDevPayload(modelsDevRes.value);
        if (parsed.length > 0) {
          merged.push(...parsed);
          sources.push("models.dev");
        }
      }
      if (openRouterRes.status === "fulfilled") {
        const parsed = parseOpenRouterPayload(openRouterRes.value);
        if (parsed.length > 0) {
          merged.push(...parsed);
          sources.push("OpenRouter");
        }
      }

      if (merged.length === 0) {
        throw new Error("公开模型元数据服务暂不可达，当前使用内置模型库");
      }

      rebuildIndices(merged);
      const updatedAt = Date.now();
      const sourceSummary = sources.join(" + ");
      writeStorageCache({
        version: 1,
        updatedAt,
        sourceSummary,
        items: catalogList,
      });
      registryState = {
        status: "ready",
        count: catalogList.length,
        updatedAt,
        sourceSummary,
        error: null,
      };
      notifyListeners();
      return registryState;
    } catch (err: any) {
      registryState = {
        ...registryState,
        status: catalogList.length > 0 ? "ready" : "error",
        error: err?.message || "同步公开模型库失败",
      };
      notifyListeners();
      return registryState;
    } finally {
      inflightSync = null;
    }
  })();

  return inflightSync;
}

export function getPublicModelRegistryState(): PublicModelRegistryState {
  hydratePublicModelRegistryFromCache();
  return registryState;
}

export function subscribePublicModelRegistry(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function buildReasoningLevels(reasoning: boolean, protocol?: string): AgentReasoningLevel[] {
  if (!reasoning) return [];
  if (protocol === "anthropic_messages") {
    return [
      { level: "off", value: null },
      { level: "4000", value: "4000" },
      { level: "16000", value: "16000" },
    ];
  }
  return [
    { level: "off", value: null },
    { level: "low", value: "low" },
    { level: "medium", value: "medium" },
    { level: "high", value: "high" },
  ];
}

export function lookupPublicModelSpec(
  modelId: string,
  protocol?: string,
): { spec: PublicModelSpec; preset: Partial<AgentChannelModel> } | null {
  hydratePublicModelRegistryFromCache();
  const trimmed = modelId.trim();
  if (!trimmed || catalogList.length === 0) return null;

  const fullKey = normalizeModelKey(trimmed);
  const slug = bareModelSlug(trimmed);
  const found = exactIndex.get(fullKey) ?? slugIndex.get(slug);
  if (!found) return null;

  return {
    spec: found,
    preset: {
      name: found.name,
      context_window: found.context_window,
      max_output_tokens: found.max_output_tokens,
      supports_tools: found.supports_tools,
      input_types: found.supports_image ? ["text", "image"] : ["text"],
      reasoning_levels: buildReasoningLevels(found.reasoning, protocol),
    },
  };
}

export function searchPublicModels(query: string, limit = 8): PublicModelSpec[] {
  hydratePublicModelRegistryFromCache();
  if (catalogList.length === 0) return [];
  const q = normalizeModelKey(query);
  const seenSlugs = new Set<string>();
  const results: PublicModelSpec[] = [];

  for (const spec of catalogList) {
    const slug = bareModelSlug(spec.id);
    if (seenSlugs.has(slug)) continue;
    if (
      !q ||
      slug.includes(q) ||
      normalizeModelKey(spec.id).includes(q) ||
      normalizeModelKey(spec.name).includes(q)
    ) {
      seenSlugs.add(slug);
      results.push(spec);
      if (results.length >= limit) break;
    }
  }
  return results;
}

/** 仅供单测注入或清空公开模型目录快照 */
export function setPublicModelRegistryForTest(specs: PublicModelSpec[] | null): void {
  hydratedFromStorage = true;
  if (!specs) {
    exactIndex = new Map();
    slugIndex = new Map();
    catalogList = [];
    registryState = {
      status: "idle",
      count: 0,
      updatedAt: null,
      sourceSummary: null,
      error: null,
    };
    return;
  }
  rebuildIndices(specs);
  registryState = {
    status: "ready",
    count: catalogList.length,
    updatedAt: Date.now(),
    sourceSummary: "test",
    error: null,
  };
}
