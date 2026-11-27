import assert from "node:assert/strict";
import test from "node:test";
import type { AgentChannelView } from "../../lib/generated/v3-contracts.ts";
import {
  buildModelSelection,
  channelModels,
  filterEnabledChannelGroups,
  groupChannelModels,
  mergeChannelModels,
  modelSelectionLabel,
  moveChannelModel,
  normalizeChannelModels,
  parseStoredModelSelection,
  reasoningLevelPresets,
  resolveActiveModelMeta,
  resolveModelSelection, filterDiscoveredModels,
} from "./channelModels.ts";

const channel: AgentChannelView = {
  id: "main",
  protocol: "chat_completions",
  base_url: "https://example.invalid/v1",
  model: "gpt-default",
  models: [
    { id: "gpt-default", name: "GPT", input_types: ["text"], reasoning_levels: [] },
    {
      id: "o3",
      input_types: ["text", "image"],
      context_window: 200000,
      reasoning_levels: [{ level: "low", value: "low" }, { level: "high", value: "high" }],
    },
  ],
  key_ready: true,
  is_default: true,
};

test("merge preserves model settings and appends only new nonblank ids", () => {
  const merged = mergeChannelModels(channel.models!, ["o3", "new-model", "", "new-model"]);
  assert.equal(merged.length, 3);
  assert.equal(merged[1].context_window, 200000);
  assert.deepEqual(merged[2], {
    id: "new-model",
    name: null,
    input_types: ["text"],
    reasoning_levels: [],
    // Round A1 (modelPresets.ts CONSERVATIVE_FALLBACK_PRESET): upgraded from 32K/4K to modern 128K/16K defaults
    context_window: 128000,
    max_output_tokens: 16384,
    supports_tools: true,
  });
});

test("legacy channel projects its default model and moving keeps order bounds", () => {
  assert.deepEqual(channelModels({ model: "legacy", models: [] }).map((m) => m.id), ["legacy"]);
  assert.deepEqual(channelModels({ model: "legacy" }).map((m) => m.id), ["legacy"]);
  assert.deepEqual(moveChannelModel(channelModels(channel), 0, -1).map((m) => m.id), ["gpt-default", "o3"]);
  assert.deepEqual(moveChannelModel(channelModels(channel), 1, -1).map((m) => m.id), ["o3", "gpt-default"]);
});

test("presets match provider reasoning payload conventions", () => {
  assert.deepEqual(reasoningLevelPresets("chat_completions").map((l) => [l.level, l.value]), [
    ["off", null], ["low", "low"], ["medium", "medium"], ["high", "high"],
  ]);
  assert.deepEqual(reasoningLevelPresets("anthropic_messages").map((l) => [l.level, l.value]), [
    ["off", null], ["4000", "4000"], ["16000", "16000"],
  ]);
});

test("picker groups only ready channels and resolves exact models and levels", () => {
  const unavailable = { ...channel, id: "no-key", key_ready: false };
  assert.deepEqual(groupChannelModels([unavailable, channel]).map((g) => g.channel.id), ["main"]);
  assert.deepEqual(buildModelSelection([channel], "main", "o3", "high"), {
    channel_id: "main", model_id: "o3", reasoning_level: "high",
  });
  assert.equal(resolveModelSelection([channel], { channel_id: "main", model_id: "o3", reasoning_level: "maximum" }), null);
  assert.equal(resolveModelSelection([channel], { channel_id: "main", model_id: "removed" }), null);
  assert.equal(resolveModelSelection([unavailable], { channel_id: "no-key", model_id: "o3" }), null);
  assert.equal(modelSelectionLabel([channel], null), "GPT");
  assert.equal(modelSelectionLabel([channel], { channel_id: "main", model_id: "o3", reasoning_level: "low" }), "o3 · low");

  const official: AgentChannelView = {
    id: "eostage-official",
    protocol: "chat_completions",
    base_url: "https://api.eostage.org/eostage/v1/proxy/v1",
    model: "glm-5.3-flash",
    models: [{ id: "glm-5.3-flash", name: "GLM-5.3 Flash", input_types: ["text"], reasoning_levels: [] }],
    key_ready: true,
    is_default: false,
  };
  assert.deepEqual(
    groupChannelModels([channel, official]).map((g) => g.channel.id),
    ["eostage-official", "main"]
  );
});

test("normalization trims ids, keeps default selectable, and forces off to null", () => {
  const normalized = normalizeChannelModels(" gpt-default ", [
    { ...channel.models![0], id: " gpt-default ", reasoning_levels: [{ level: "off", value: "off" }, { level: "high", value: " high " }] },
    { id: "", input_types: ["text"], reasoning_levels: [] },
    { id: "gpt-default", input_types: ["text"], reasoning_levels: [] },
  ]);
  assert.equal(normalized.length, 1);
  assert.equal(normalized[0].id, "gpt-default");
  assert.deepEqual(normalized[0].reasoning_levels, [{ level: "off", value: null }, { level: "high", value: "high" }]);
  assert.deepEqual(normalizeChannelModels("gpt-default", []).map((model) => model.id), ["gpt-default"]);
});

test("stored selection is parsed defensively", () => {
  assert.deepEqual(parseStoredModelSelection('{"channel_id":"main","model_id":"o3","reasoning_level":"low"}'), {
    channel_id: "main", model_id: "o3", reasoning_level: "low",
  });
  assert.equal(parseStoredModelSelection("not-json"), null);
  assert.equal(parseStoredModelSelection('{"channel_id":"main"}'), null);
});


test("filterDiscoveredModels strips non-chat/non-tool models and collapses dated snapshots only when base exists", () => {
  const raw = [
    "gpt-4o",
    "gpt-4o-2024-08-06",
    "gpt-4o-audio-preview",
    "gpt-4o-realtime-preview",
    "text-embedding-3-small",
    "dall-e-3",
    "whisper-1",
    "tts-1-hd",
    "omni-moderation-latest",
    "claude-3-5-sonnet-20241022",
    "gemini-2.5-pro",
    "imagen-3.0-generate-002",
    "veo-2.0-generate-001",
  ];
  const filtered = filterDiscoveredModels(raw);
  assert.deepEqual(filtered, [
    "gpt-4o",
    "claude-3-5-sonnet-20241022",
    "gemini-2.5-pro",
  ]);

  const unfiltered = filterDiscoveredModels(raw, { hideNonChat: false, collapseSnapshots: false });
  assert.equal(unfiltered.length, raw.length);
});

test("empty-model channels omit blank placeholder rows and normalize preserves video/pdf/protocol/capabilities", () => {
  assert.deepEqual(channelModels({ model: "", models: [] }), []);
  assert.deepEqual(
    channelModels({
      model: "",
      models: [{ id: "gemini-2.5-flash", input_types: ["text", "video", "pdf"], reasoning_levels: [] }],
    }).map((m) => m.id),
    ["gemini-2.5-flash"],
  );

  const normalized = normalizeChannelModels("", [
    {
      id: "egg-alpha-003",
      protocol: "responses",
      input_types: ["text", "image", "video", "pdf"],
      reasoning_levels: [{ level: "medium", value: "medium" }],
      supports_tools: true,
      supports_structured_output: true,
      supports_web_search: true,
      supports_mid_turn_system: false,
      reasoning_param_mapping: '{"reasoning":{"effort":"{value}"}}',
    },
  ]);
  assert.equal(normalized.length, 1);
  assert.equal(normalized[0].protocol, "responses");
  assert.deepEqual(normalized[0].input_types, ["text", "image", "video", "pdf"]);
  assert.equal(normalized[0].supports_web_search, true);
  assert.equal(normalized[0].supports_mid_turn_system, false);
  assert.equal(normalized[0].reasoning_param_mapping, '{"reasoning":{"effort":"{value}"}}');
});

test("filterEnabledChannelGroups excludes disabled providers, disabled models, and empty channels", () => {
  const emptyOauth: AgentChannelView = {
    id: "codex-empty",
    protocol: "responses",
    base_url: "https://chatgpt.com/backend-api/codex",
    model: "",
    models: [],
    key_ready: true,
    is_default: false,
  };
  const filtered = filterEnabledChannelGroups(
    [channel, emptyOauth],
    {},
    { main: { "gpt-default": true } },
  );
  assert.equal(filtered.length, 1);
  assert.equal(filtered[0].channel.id, "main");
  assert.deepEqual(filtered[0].models.map((m) => m.id), ["o3"]);

  const noneLeft = filterEnabledChannelGroups([channel], { main: true }, {});
  assert.equal(noneLeft.length, 0);
});

test("normalizeChannelModels preserves is_free and resolveActiveModelMeta surfaces isFree", () => {
  const normalized = normalizeChannelModels("", [
    {
      id: "neohorse-1.5-flash-preview",
      name: "NeoHorse 1.5 Flash Preview",
      is_free: true,
    },
    {
      id: "glm-5.3-flash",
      name: "GLM-5.3 Flash",
      is_free: false,
    },
  ]);
  assert.equal(normalized.length, 2);
  assert.equal(normalized[0].is_free, true);
  assert.equal(normalized[1].is_free, false);

  const officialChannel: AgentChannelView = {
    id: "eostage-official",
    protocol: "chat_completions",
    base_url: "https://api.eostage.org/eostage/v1",
    model: "neohorse-1.5-flash-preview",
    models: normalized,
    key_ready: true,
    is_default: true,
  };
  const freeMeta = resolveActiveModelMeta([officialChannel], {
    channel_id: "eostage-official",
    model_id: "neohorse-1.5-flash-preview",
  });
  assert.equal(freeMeta.isFree, true);
  assert.equal(freeMeta.model?.id, "neohorse-1.5-flash-preview");

  const paidMeta = resolveActiveModelMeta([officialChannel], {
    channel_id: "eostage-official",
    model_id: "glm-5.3-flash",
  });
  assert.equal(paidMeta.isFree, false);

  const customSpoofChannel: AgentChannelView = {
    id: "custom-openai",
    protocol: "chat_completions",
    base_url: "https://api.openai.com/v1",
    model: "gpt-spoof-free",
    models: [{ id: "gpt-spoof-free", name: "Spoof", is_free: true }],
    key_ready: true,
    is_default: false,
  };
  const customMeta = resolveActiveModelMeta([customSpoofChannel], {
    channel_id: "custom-openai",
    model_id: "gpt-spoof-free",
  });
  assert.equal(customMeta.isFree, false);
});
