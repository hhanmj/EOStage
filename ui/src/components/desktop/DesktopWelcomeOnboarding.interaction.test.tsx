import "../../test-utils/setup-dom.ts";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { cleanup, fireEvent, render } from "@testing-library/react";

const { I18nProvider, LOCALE_STORAGE_KEY } = await import("../../i18n/index.ts");
const { DesktopWelcomeOnboarding, SmoothEoPhoto } = await import("./DesktopWelcomeOnboarding.tsx");
const { STAGE1_DECK_LAYERS } = await import("./onboarding/onboardingAtlasData.ts");

test.afterEach(() => {
  cleanup();
});

test("STAGE1_DECK_LAYERS uses only bundled sprite sheets when atlas CDN is unconfigured", () => {
  assert.equal(STAGE1_DECK_LAYERS.length, 1);
  assert.ok(STAGE1_DECK_LAYERS[0].every((url) => url.startsWith("/onboarding-stage1/")));
});

test("DesktopWelcomeOnboarding renders recognizable unconfigured pack fallback without emitting /atlas-cdn 404 URLs", () => {
  window.localStorage.setItem(LOCALE_STORAGE_KEY, "en");
  const view = render(
    <I18nProvider>
      <DesktopWelcomeOnboarding open={true} onClose={() => {}} />
    </I18nProvider>
  );

  fireEvent.click(view.getByTestId("welcome-onboarding-continue-btn"));
  fireEvent.click(view.getByTestId("welcome-continent-toggle"));
  fireEvent.click(view.getByTestId("welcome-country-pick-CHN"));

  const heroCard = view.getByTestId("welcome-atlas-hero-card");
  const fallback = view.getByTestId("welcome-atlas-photo-fallback");
  assert.equal(fallback.getAttribute("data-fallback-reason"), "pack_unconfigured");
  assert.ok((fallback.textContent || "").includes("VITE_EO_ATLAS_CDN_BASE_URL"));
  const imgs = Array.from(heroCard.querySelectorAll("img"));
  assert.ok(imgs.every((img) => !(img.getAttribute("src") || "").includes("/atlas-cdn/")));
});

test("SmoothEoPhoto exposes load_error fallback and working retry action on image failure", () => {
  const view = render(<SmoothEoPhoto src="https://cdn.example.com/eo/tile.webp" alt="Tile" locale="en" />);
  const img = view.getByAltText("Tile");
  fireEvent.error(img);

  const fallback = view.getByTestId("welcome-atlas-photo-fallback");
  assert.equal(fallback.getAttribute("data-fallback-reason"), "load_error");
  const retryBtn = view.getByTestId("welcome-atlas-photo-retry");
  fireEvent.click(retryBtn);

  assert.equal(view.queryByTestId("welcome-atlas-photo-fallback"), null);
  assert.ok(view.getByAltText("Tile"));
});
