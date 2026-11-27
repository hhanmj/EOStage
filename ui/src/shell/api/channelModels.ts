import { t } from "../../i18n/index.ts";
import { applyModelPresetDefaults } from "./modelPresets.ts";
import type {
  AgentChannelModel,
  AgentChannelView,
  AgentModelSelection,
  AgentReasoningLevel,
} from "../../lib/generated/v3-contracts.ts";

export function channelModels(
  channel: Pick<AgentChannelView, "model"> & { models?: AgentChannelModel[] | null },
): AgentChannelModel[] {
  const models = (channel.models ?? []).filter((model) => model.id.trim());
  const defaultId = channel.model.trim();
  if (!defaultId) {
    return models.map((model) => ({ ...model }));
  }
  return models.some((model) => model.id === defaultId)
    ? models.map((model) => ({ ...model }))
    : [{ id: defaultId, input_types: ["text"], reasoning_levels: [] }, ...models.map((model) => ({ ...model }))];
}

export function normalizeChannelModels(
  defaultModelId: string,
  models: readonly AgentChannelModel[],
): AgentChannelModel[] {
  const seen = new Set<string>();
  const normalized = models.flatMap((model) => {
    const id = model.id.trim();
    if (!id || seen.has(id)) return [];
    seen.add(id);
    const name = model.name?.trim();
    const protocol = model.protocol?.trim();
    const reasoningMapping = model.reasoning_param_mapping?.trim();
    const reasoningLevels = (model.reasoning_levels ?? []).flatMap((level) => {
      const name = level.level.trim();
      if (!name) return [];
      return [{ level: name, value: name === "off" ? null : level.value?.trim() || null }];
    });
    const inputTypes = (["text", "image", "video", "pdf"] as const).filter(
      (kind) => kind === "text" || Boolean(model.input_types?.includes(kind)),
    );
    return [{
      id,
      ...(name ? { name } : {}),
      ...(protocol ? { protocol } : {}),
      ...(model.context_window ? { context_window: model.context_window } : {}),
      ...(model.max_output_tokens ? { max_output_tokens: model.max_output_tokens } : {}),
      input_types: inputTypes,
      reasoning_levels: reasoningLevels,
      ...(model.supports_tools !== undefined ? { supports_tools: model.supports_tools } : {}),
      ...(model.supports_structured_output !== undefined
        ? { supports_structured_output: model.supports_structured_output }
        : {}),
      ...(model.supports_web_search !== undefined
        ? { supports_web_search: model.supports_web_search }
        : {}),
      ...(model.supports_mid_turn_system !== undefined
        ? { supports_mid_turn_system: model.supports_mid_turn_system }
        : {}),
      ...(typeof model.is_free === "boolean" ? { is_free: model.is_free } : {}),
      ...(reasoningMapping ? { reasoning_param_mapping: reasoningMapping } : {}),
    }];
  });
  return channelModels({ model: defaultModelId.trim(), models: normalized });
}

export function newChannelModel(id = "", protocol?: string): AgentChannelModel {
  return applyModelPresetDefaults({ id, input_types: ["text"], reasoning_levels: [] }, protocol);
}

export function mergeChannelModels(
  models: readonly AgentChannelModel[],
  ids: readonly string[],
  protocol?: string,
): AgentChannelModel[] {
  const merged = models.map((model) => ({ ...model }));
  for (const id of ids) {
    const trimmed = id.trim();
    if (trimmed && !merged.some((model) => model.id === trimmed)) {
      merged.push(newChannelModel(trimmed, protocol));
    }
  }
  return merged;
}

export function moveChannelModel(
  models: readonly AgentChannelModel[],
  index: number,
  offset: -1 | 1,
): AgentChannelModel[] {
  const target = index + offset;
  if (index < 0 || target < 0 || index >= models.length || target >= models.length) {
    return models.map((model) => ({ ...model }));
  }
  const moved = models.map((model) => ({ ...model }));
  [moved[index], moved[target]] = [moved[target], moved[index]];
  return moved;
}

export function reasoningLevelPresets(protocol: string): AgentReasoningLevel[] {
  return protocol === "anthropic_messages"
    ? [
        { level: "off", value: null },
        { level: "4000", value: "4000" },
        { level: "16000", value: "16000" },
      ]
    : [
        { level: "off", value: null },
        { level: "low", value: "low" },
        { level: "medium", value: "medium" },
        { level: "high", value: "high" },
      ];
}

export const OFFICIAL_CHANNEL_ID = "eostage-official";
const LEGACY_OFFICIAL_CHANNEL_ID = "eostage-plan";

export function isOfficialChannelId(channelId: string | null | undefined): boolean {
  const clean = (channelId ?? "").trim();
  return clean === OFFICIAL_CHANNEL_ID || clean === LEGACY_OFFICIAL_CHANNEL_ID;
}

export function channelDisplayName(channel: Pick<AgentChannelView, "id">): string {
  return isOfficialChannelId(channel.id) ? t("settings.channels.officialProviderName") : channel.id;
}

export interface ChannelModelGroup {
  channel: AgentChannelView;
  models: AgentChannelModel[];
}

export function groupChannelModels(channels: readonly AgentChannelView[]): ChannelModelGroup[] {
  const rank = (channel: AgentChannelView): number => {
    if (isOfficialChannelId(channel.id)) return 0;
    if (channel.is_default) return 1;
    return 2;
  };
  const sorted = [...channels].sort((a, b) => rank(a) - rank(b));
  return sorted
    .filter((channel) => channel.key_ready)
    .map((channel) => ({ channel, models: channelModels(channel) }));
}

export function filterEnabledChannelGroups(
  channels: readonly AgentChannelView[],
  disabledProviders: Readonly<Record<string, boolean>>,
  disabledModelsByChannel: Readonly<Record<string, Readonly<Record<string, boolean>>>>,
): ChannelModelGroup[] {
  return groupChannelModels(channels)
    .filter((group) => !disabledProviders[group.channel.id])
    .map((group) => {
      const disabledMap = disabledModelsByChannel[group.channel.id] ?? {};
      const models = group.models.filter((model) => !disabledMap[model.id]);
      return { channel: group.channel, models };
    })
    .filter((group) => group.models.length > 0);
}

export function resolveModelSelection(
  channels: readonly AgentChannelView[],
  selection: AgentModelSelection | null | undefined,
): AgentModelSelection | null {
  if (!selection) return null;
  const channel = channels.find((item) => item.id === selection.channel_id && item.key_ready);
  const model = channel && channelModels(channel).find((item) => item.id === selection.model_id);
  if (!model) return null;
  const level = selection.reasoning_level;
  if (level && !model.reasoning_levels?.some((item) => item.level === level)) return null;
  return {
    channel_id: channel.id,
    model_id: model.id,
    ...(level ? { reasoning_level: level } : {}),
  };
}

export function buildModelSelection(
  channels: readonly AgentChannelView[],
  channelId: string,
  modelId: string,
  reasoningLevel?: string | null,
): AgentModelSelection | null {
  return resolveModelSelection(channels, {
    channel_id: channelId,
    model_id: modelId,
    reasoning_level: reasoningLevel ?? null,
  });
}

export function modelSelectionLabel(
  channels: readonly AgentChannelView[],
  selection: AgentModelSelection | null | undefined,
): string {
  const resolved = resolveModelSelection(channels, selection);
  if (!resolved) {
    const activeMeta = resolveActiveModelMeta(channels, null);
    return activeMeta.modelLabel || t("shared.channelModels.selectModel");
  }
  const group = groupChannelModels(channels).find((item) => item.channel.id === resolved.channel_id);
  const model = group?.models.find((item) => item.id === resolved.model_id);
  return `${model?.name?.trim() || model?.id || resolved.model_id}${resolved.reasoning_level ? ` · ${resolved.reasoning_level}` : ""}`;
}

export function resolveActiveModelMeta(
  channels: readonly AgentChannelView[],
  selection: AgentModelSelection | null | undefined,
): { modelLabel?: string; contextWindow?: number; isFree?: boolean; model?: AgentChannelModel } {
  const groups = groupChannelModels(channels);
  if (groups.length === 0) return {};
  const resolved = resolveModelSelection(channels, selection);
  const targetGroup = resolved
    ? groups.find((g) => g.channel.id === resolved.channel_id)
    : groups.find((g) => g.channel.is_default) ?? groups[0];
  if (!targetGroup) return {};
  const targetModelId = resolved ? resolved.model_id : targetGroup.channel.model;
  const modelObj =
    targetGroup.models.find((m) => m.id === targetModelId) ?? targetGroup.models[0];
  const modelLabel = (modelObj?.name?.trim() || modelObj?.id || targetModelId || "").trim() || undefined;
  const rawCtx = modelObj?.context_window;
  const contextWindow =
    typeof rawCtx === "number" && Number.isFinite(rawCtx) && rawCtx > 0 ? rawCtx : undefined;
  return {
    modelLabel,
    contextWindow,
    isFree: isOfficialChannelId(targetGroup.channel.id) && modelObj?.is_free === true,
    model: modelObj,
  };
}

export function parseStoredModelSelection(value: string | null): AgentModelSelection | null {
  if (!value) return null;
  try {
    const parsed = JSON.parse(value) as Partial<AgentModelSelection>;
    if (typeof parsed.channel_id !== "string" || typeof parsed.model_id !== "string") return null;
    return {
      channel_id: parsed.channel_id,
      model_id: parsed.model_id,
      reasoning_level: typeof parsed.reasoning_level === "string" ? parsed.reasoning_level : null,
    };
  } catch {
    return null;
  }
}


/**
 * Adapted from opengeos/GeoLibre (apps/geolibre-desktop/src/lib/assistant/model-discovery.ts, commit dc92b6c)
 * License: MIT, Copyright (c) 2026 Qiusheng Wu
 */
export const KEY_DISCOVERY_SETTLE_MS = 500;

const NON_CHAT_TOOL_MODEL_PATTERN =
  /(?:^|[-_/:.])(audio|realtime|live|transcribe|tts|whisper|dall-e|image|search|embedding|embeddings|babbage|davinci|moderation|instruct|computer-use|deep-research|chat-latest|omni-moderation|text-embedding|veo|imagen|robotics|aqa|rerank)(?:$|[-_/:.\d])/i;

const MODEL_DATE_SNAPSHOT_SUFFIX = /-(20\d{2}-\d{2}-\d{2}|20\d{6})$/;

export function isToolCapableChatModel(id: string): boolean {
  const trimmed = id.trim();
  if (!trimmed) return false;
  return !NON_CHAT_TOOL_MODEL_PATTERN.test(trimmed);
}

export function stripModelDateSnapshot(id: string): string | null {
  const trimmed = id.trim();
  const match = MODEL_DATE_SNAPSHOT_SUFFIX.exec(trimmed);
  if (!match) return null;
  const base = trimmed.slice(0, trimmed.length - match[0].length).trim();
  return base || null;
}

export interface FilterDiscoveredModelsOptions {
  hideNonChat?: boolean;
  collapseSnapshots?: boolean;
}

export function filterDiscoveredModels(
  rawIds: readonly string[],
  options: FilterDiscoveredModelsOptions = {},
): string[] {
  const hideNonChat = options.hideNonChat ?? true;
  const collapseSnapshots = options.collapseSnapshots ?? true;

  const deduped: string[] = [];
  const seen = new Set<string>();
  for (const raw of rawIds) {
    const trimmed = raw.trim();
    if (!trimmed || seen.has(trimmed)) continue;
    seen.add(trimmed);
    if (hideNonChat && !isToolCapableChatModel(trimmed)) continue;
    deduped.push(trimmed);
  }

  if (!collapseSnapshots) {
    return deduped;
  }

  const presentSet = new Set(deduped);
  return deduped.filter((id) => {
    const base = stripModelDateSnapshot(id);
    if (base && presentSet.has(base)) {
      return false;
    }
    return true;
  });
}
