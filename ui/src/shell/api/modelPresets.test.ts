import test from "node:test";
import assert from "node:assert/strict";
import {
  matchModelPreset,
  matchSpecificPreset,
  applyModelPresetDefaults,
  CONSERVATIVE_FALLBACK_PRESET,
} from "./modelPresets.ts";
import { setPublicModelRegistryForTest } from "./publicModelRegistry.ts";

// Provenance (2026-11-12 Owner Request #10): DeepSeek V3/R1 official context upgraded to 128K (131072),
// flagship 2026 families added, and live public model registry lookup takes precedence when synced.
test("matchModelPreset: DeepSeek V3 and R1 match current 128K context and reasoning", () => {
  const v3 = matchModelPreset("deepseek-v3");
  assert.equal(v3.name, "DeepSeek V3");
  assert.equal(v3.context_window, 131072);
  assert.equal(v3.max_output_tokens, 32768);
  assert.equal(v3.supports_tools, true);
  assert.deepEqual(v3.input_types, ["text"]);
  assert.equal(v3.reasoning_levels?.length, 0);

  const r1 = matchModelPreset("deepseek-r1");
  assert.equal(r1.name, "DeepSeek R1");
  assert.equal(r1.context_window, 131072);
  assert.equal(r1.max_output_tokens, 65536);
  assert.equal(r1.supports_tools, true);
  assert.equal(r1.reasoning_levels?.length, 5);

  const r1Distill = matchModelPreset("deepseek-r1-distill-qwen-32b");
  assert.equal(r1Distill.name, "DeepSeek R1");
  assert.equal(r1Distill.reasoning_levels?.length, 5);
});

test("matchModelPreset: Claude 4 and Gemini 2.5 and Qwen 3 match", () => {
  const claude4 = matchModelPreset("claude-sonnet-4-20250501");
  assert.equal(claude4.name, "Claude 4");
  assert.equal(claude4.context_window, 200000);
  assert.equal(claude4.max_output_tokens, 16384);
  assert.equal(claude4.supports_tools, true);
  assert.deepEqual(claude4.input_types, ["text", "image"]);
  assert.equal(claude4.reasoning_levels?.length, 4);

  const gemini25 = matchModelPreset("gemini-2.5-pro");
  assert.equal(gemini25.name, "Gemini 2.5");
  assert.equal(gemini25.context_window, 1048576);
  assert.equal(gemini25.max_output_tokens, 65536);
  assert.equal(gemini25.supports_tools, true);
  assert.deepEqual(gemini25.input_types, ["text", "image"]);
  assert.equal(gemini25.reasoning_levels?.length, 4);

  const qwen3 = matchModelPreset("qwen3-72b");
  assert.equal(qwen3.name, "Qwen 3");
  assert.equal(qwen3.context_window, 131072);
  assert.equal(qwen3.max_output_tokens, 8192);
  assert.equal(qwen3.supports_tools, true);
  assert.equal(qwen3.reasoning_levels?.length, 4);
});

test("matchModelPreset: OpenAI gpt-4o, o1, o3-mini match correctly", () => {
  const gpt4o = matchModelPreset("gpt-4o");
  assert.equal(gpt4o.name, "GPT-4o");
  assert.equal(gpt4o.context_window, 128000);
  assert.equal(gpt4o.max_output_tokens, 16384);
  assert.equal(gpt4o.supports_tools, true);
  assert.deepEqual(gpt4o.input_types, ["text", "image"]);

  const o1 = matchModelPreset("o1-preview");
  assert.equal(o1.name, "o1 Preview");
  assert.equal(o1.context_window, 128000);
  assert.equal(o1.supports_tools, false);
  assert.equal(o1.reasoning_levels?.length, 3);

  const o3 = matchModelPreset("o3-mini");
  assert.equal(o3.name, "o3 mini");
  assert.equal(o3.context_window, 200000);
  assert.equal(o3.max_output_tokens, 100000);
  assert.equal(o3.supports_tools, true);
});

test("matchModelPreset: Claude 3.5 and 3.7 sonnet match Anthropic reasoning", () => {
  const claude35 = matchModelPreset("claude-3-5-sonnet-20241022", "anthropic_messages");
  assert.equal(claude35.name, "Claude 3.5 Sonnet");
  assert.equal(claude35.context_window, 200000);
  assert.equal(claude35.max_output_tokens, 8192);
  assert.deepEqual(claude35.input_types, ["text", "image"]);
  assert.equal(claude35.reasoning_levels?.length, 0);

  const claude37 = matchModelPreset("claude-3-7-sonnet-20250219", "anthropic_messages");
  assert.equal(claude37.name, "Claude 3.7 Sonnet");
  assert.equal(claude37.context_window, 200000);
  assert.equal(claude37.reasoning_levels?.length, 3);
  assert.equal(claude37.reasoning_levels?.[1].level, "4000");
});

test("matchModelPreset: Qwen, GLM, Kimi, and Gemini families match", () => {
  const qwenVl = matchModelPreset("qwen2.5-vl-72b-instruct");
  assert.equal(qwenVl.context_window, 131072);
  assert.deepEqual(qwenVl.input_types, ["text", "image"]);

  const glm4 = matchModelPreset("glm-4-plus");
  assert.equal(glm4.context_window, 128000);
  assert.equal(glm4.max_output_tokens, 4096);

  const kimi = matchModelPreset("moonshot-v1-32k");
  assert.equal(kimi.context_window, 32768);

  const gemini = matchModelPreset("gemini-1.5-pro");
  assert.equal(gemini.context_window, 2097152);
  assert.deepEqual(gemini.input_types, ["text", "image"]);
});

test("matchSpecificPreset distinguishes known rules from unknown fallbacks and prefers synced public registry", () => {
  const specificDeepSeek = matchSpecificPreset("deepseek-reasoner");
  assert.equal(specificDeepSeek.matched, true);
  assert.equal(specificDeepSeek.preset.name, "DeepSeek R1");
  assert.equal(specificDeepSeek.preset.context_window, 131072);

  const gpt5 = matchSpecificPreset("gpt-5.4");
  assert.equal(gpt5.matched, true);
  assert.equal(gpt5.preset.context_window, 400000);

  const unknown = matchSpecificPreset("my-custom-unrecognized-model");
  assert.equal(unknown.matched, false);

  try {
    setPublicModelRegistryForTest([
      {
        id: "custom-org/live-api-model-v2",
        provider: "CustomOrg",
        name: "Live API Model V2",
        context_window: 524288,
        max_output_tokens: 65536,
        supports_tools: true,
        supports_image: true,
        supports_pdf: true,
        reasoning: true,
      },
    ]);
    const liveHit = matchSpecificPreset("live-api-model-v2");
    assert.equal(liveHit.matched, true);
    assert.equal(liveHit.source, "public_api");
    assert.equal(liveHit.preset.name, "Live API Model V2");
    assert.equal(liveHit.preset.context_window, 524288);
    assert.equal(liveHit.preset.max_output_tokens, 65536);
    assert.equal(liveHit.supports_pdf, true);
  } finally {
    setPublicModelRegistryForTest(null);
  }
});

test("matchModelPreset: unknown model uses conservative fallback", () => {
  const custom = matchModelPreset("my-unknown-finetune-model");
  assert.equal(custom.context_window, CONSERVATIVE_FALLBACK_PRESET.context_window);
  assert.equal(custom.max_output_tokens, CONSERVATIVE_FALLBACK_PRESET.max_output_tokens);
  assert.equal(custom.supports_tools, true);
  assert.deepEqual(custom.input_types, ["text"]);
});

test("applyModelPresetDefaults preserves user manual overrides", () => {
  const userModel = {
    id: "gpt-4o",
    name: "Custom GPT-4o",
    context_window: 64000,
    max_output_tokens: 2048,
    input_types: ["text" as const],
    reasoning_levels: [],
    supports_tools: false,
  };
  const filled = applyModelPresetDefaults(userModel);
  assert.equal(filled.name, "Custom GPT-4o");
  assert.equal(filled.context_window, 64000);
  assert.equal(filled.max_output_tokens, 2048);
  assert.deepEqual(filled.input_types, ["text"]);
  assert.equal(filled.supports_tools, false);
});
