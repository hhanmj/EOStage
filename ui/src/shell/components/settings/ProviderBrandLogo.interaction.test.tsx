import "../../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { cleanup, render } from "../../../test-utils/render-harness";
import {
  ProviderBrandLogo,
  resolveProviderBrandKey,
} from "./ProviderBrandLogo";

test.afterEach(() => {
  cleanup();
});

test("resolveProviderBrandKey prioritizes Google Gemini even with /v1beta/openai baseUrl", () => {
  assert.equal(
    resolveProviderBrandKey(
      "google-gemini",
      "https://generativelanguage.googleapis.com/v1beta/openai",
      "gemini-3.1-pro"
    ),
    "google-gemini"
  );
  assert.equal(
    resolveProviderBrandKey(
      "my-gemini-channel",
      "https://generativelanguage.googleapis.com/v1beta/openai",
      "gpt-fallback-not-used"
    ),
    "google-gemini"
  );
  assert.equal(
    resolveProviderBrandKey(
      "custom-alias",
      "https://generativelanguage.googleapis.com/v1beta/openai",
      ""
    ),
    "google-gemini"
  );
  assert.equal(
    resolveProviderBrandKey(
      "openai-codex",
      "https://api.openai.com/v1",
      "gpt-5"
    ),
    "openai"
  );
  assert.equal(
    resolveProviderBrandKey(
      "anthropic-claude",
      "https://api.anthropic.com/v1",
      "claude-sonnet-4-5"
    ),
    "claude"
  );
  assert.equal(
    resolveProviderBrandKey(
      "qwen-relay",
      "https://dashscope.aliyuncs.com/compatible-mode/v1",
      "qwen3-max"
    ),
    "aliyun-bailian"
  );
});

test("ProviderBrandLogo renders data-brand='google-gemini' SVG for Gemini OpenAI-compat endpoint", () => {
  const { container } = render(
    <ProviderBrandLogo
      provider="google-gemini"
      baseUrl="https://generativelanguage.googleapis.com/v1beta/openai"
      modelHint="gemini-2.5-pro"
    />
  );
  const svg = container.querySelector("svg[data-brand='google-gemini']");
  assert.ok(svg, "should render Google Gemini sparkle SVG instead of OpenAI");
  assert.equal(container.querySelector("svg[data-brand='openai']"), null);
});

test("custom relay channel with glm or gemini model hint always renders custom cube icon instead of Zhipu/Gemini brand", () => {
  assert.equal(
    resolveProviderBrandKey("tokenh", "https://relay.example.invalid/v1", "glm-5.3-flash"),
    "custom"
  );
  assert.equal(
    resolveProviderBrandKey("Vellum", "https://api.vellum.invalid/v1", "gemini-2.5-pro"),
    "custom"
  );
  const { container } = render(
    <ProviderBrandLogo
      provider="tokenh"
      baseUrl="https://relay.example.invalid/v1"
      modelHint="glm-5.3-flash"
    />
  );
  assert.ok(container.querySelector("svg[data-brand='custom']"));
  assert.equal(container.querySelector("svg[data-brand='zhipu-glm']"), null);
});
