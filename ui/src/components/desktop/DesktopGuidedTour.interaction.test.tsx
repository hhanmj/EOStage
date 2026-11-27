import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { cleanup, fireEvent, render } from "../../test-utils/render-harness";
import { I18nProvider, LOCALE_STORAGE_KEY } from "@/i18n";
import {
  DesktopGuidedTour,
  GuidedTourPromptBanner,
  DESKTOP_GUIDED_TOUR_STEPS,
  type GuidedTourStepConfig,
} from "./DesktopGuidedTour";

test.afterEach(() => {
  cleanup();
  window.localStorage.clear();
});

function MockWorkspaceAnchors() {
  const box = (left: number, top: number, width: number, height: number): React.CSSProperties => ({
    position: "fixed",
    left,
    top,
    width,
    height,
  });
  return (
    <div className="sp-modern">
      <div data-testid="agent-home-composer-shell" style={box(320, 260, 560, 140)} />
      <div data-testid="admin-boundary-modal" style={box(420, 160, 480, 420)} />
      <div data-column="search" style={box(60, 48, 320, 620)} />
      <div data-testid="catalog-sidebar-nav" style={box(180, 120, 196, 480)} />
      <div data-testid="download-center-tour-anchor" style={box(250, 48, 900, 96)} />
      <div data-testid="add-provider-catalog-view" style={box(620, 110, 580, 540)} />
      <div data-testid="settings-accounts-tour-anchor" style={box(280, 48, 860, 64)} />
    </div>
  );
}

test("DesktopGuidedTour steps through all 7 EOStage features, orchestrates panels/modals/settings, and supports keyboard & segment navigation", () => {
  const enteredSteps: GuidedTourStepConfig[] = [];
  const finishCalls: Array<{ stayOnCurrent?: boolean; restoreRoute?: boolean } | undefined> = [];

  const view = render(
    <I18nProvider>
      <MockWorkspaceAnchors />
      <DesktopGuidedTour
        open={false}
        onStepEnter={(step) => enteredSteps.push(step)}
        onFinish={(opts) => finishCalls.push(opts)}
      />
    </I18nProvider>
  );
  assert.equal(view.queryByTestId("desktop-guided-tour"), null);

  view.rerender(
    <I18nProvider>
      <MockWorkspaceAnchors />
      <DesktopGuidedTour
        open={true}
        onStepEnter={(step) => enteredSteps.push(step)}
        onFinish={(opts) => finishCalls.push(opts)}
      />
    </I18nProvider>
  );

  // Step 1: AI Composer
  assert.ok(view.getByTestId("desktop-guided-tour") !== null);
  assert.equal(view.getByTestId("guided-tour-step-counter").textContent?.trim(), "1 / 7");
  assert.match(view.getByTestId("guided-tour-step-title").textContent || "", /多源遥感数据/);
  assert.equal(view.queryByTestId("guided-tour-prev-btn"), null);

  // Next -> Step 2 (AdminBoundaryModal opened from dual AOI entry points)
  fireEvent.click(view.getByTestId("guided-tour-next-btn"));
  assert.equal(view.getByTestId("guided-tour-step-counter").textContent?.trim(), "2 / 7");
  assert.equal(enteredSteps.at(-1)?.openAdminModal, true);
  assert.match(view.getByTestId("guided-tour-step-title").textContent || "", /行政区/);

  // Keyboard ArrowRight -> Step 3 (Map Workbench with Search panel expanded)
  fireEvent.keyDown(window, { key: "ArrowRight" });
  assert.equal(view.getByTestId("guided-tour-step-counter").textContent?.trim(), "3 / 7");
  assert.equal(enteredSteps.at(-1)?.route, "workbench");
  assert.equal(enteredSteps.at(-1)?.expandSearchPanel, true);

  // Next -> Step 4 (Data Catalog modal opened with accurate registry counts)
  fireEvent.click(view.getByTestId("guided-tour-next-btn"));
  assert.equal(view.getByTestId("guided-tour-step-counter").textContent?.trim(), "4 / 7");
  assert.equal(enteredSteps.at(-1)?.openCatalog, true);
  const step4CardText = view.getByTestId("guided-tour-card").textContent || "";
  assert.match(step4CardText, /53 个卫星源 \+ 14 个 GEE 专题/);
  assert.match(step4CardText, /共收录 67 项/);
  assert.match(step4CardText, /26 个免账号直连/);
  assert.equal(step4CardText.includes("59 个"), false);

  // ArrowLeft returns cleanly to Step 3
  fireEvent.keyDown(window, { key: "ArrowLeft" });
  assert.equal(view.getByTestId("guided-tour-step-counter").textContent?.trim(), "3 / 7");

  // Jump via segmented progress bar directly to Step 6 (Model Settings + Add Provider Catalog)
  fireEvent.click(view.getByTestId("guided-tour-dot-5"));
  assert.equal(view.getByTestId("guided-tour-step-counter").textContent?.trim(), "6 / 7");
  assert.equal(enteredSteps.at(-1)?.route, "settings");
  assert.equal(enteredSteps.at(-1)?.settingsCategory, "models");
  assert.equal(enteredSteps.at(-1)?.openAddProviderCatalog, true);
  assert.match(view.getByTestId("guided-tour-step-title").textContent || "", /多平台预设/);

  // Next -> Step 7 (Data Service Accounts), shows stayHere and finish CTA
  fireEvent.click(view.getByTestId("guided-tour-next-btn"));
  assert.equal(view.getByTestId("guided-tour-step-counter").textContent?.trim(), "7 / 7");
  assert.equal(enteredSteps.at(-1)?.route, "settings");
  assert.equal(enteredSteps.at(-1)?.settingsCategory, "auth");
  assert.ok(view.getByTestId("guided-tour-stay-btn") !== null);

  // Clicking Configure here invokes onFinish({ stayOnCurrent: true })
  fireEvent.click(view.getByTestId("guided-tour-stay-btn"));
  assert.equal(finishCalls.length, 1);
  assert.deepEqual(finishCalls[0], { stayOnCurrent: true });

  // Escape dismisses and asks to restore caller route
  fireEvent.keyDown(window, { key: "Escape" });
  assert.equal(finishCalls.length, 2);
  assert.deepEqual(finishCalls[1], { restoreRoute: true });

  assert.equal(DESKTOP_GUIDED_TOUR_STEPS.length, 7);
});

test("GuidedTourPromptBanner starts or dismisses cleanly without blocking overlay", () => {
  let starts = 0;
  let dismisses = 0;

  const view = render(
    <I18nProvider>
      <div className="sp-modern" />
      <GuidedTourPromptBanner
        open={true}
        onStartTour={() => {
          starts += 1;
        }}
        onDismiss={() => {
          dismisses += 1;
        }}
      />
    </I18nProvider>
  );

  const banner = view.getByTestId("guided-tour-prompt-banner");
  assert.match(banner.textContent || "", /快速熟悉 EOStage/);
  fireEvent.click(view.getByTestId("guided-tour-prompt-start-btn"));
  assert.equal(starts, 1);
  fireEvent.click(view.getByTestId("guided-tour-prompt-dismiss-btn"));
  assert.equal(dismisses, 1);
});

test("DesktopGuidedTour renders English and Japanese copy cleanly without hardcoded Chinese", () => {
  window.localStorage.setItem(LOCALE_STORAGE_KEY, "en");
  const enView = render(
    <I18nProvider>
      <MockWorkspaceAnchors />
      <DesktopGuidedTour open={true} onFinish={() => {}} />
    </I18nProvider>
  );
  assert.match(enView.getByTestId("guided-tour-step-title").textContent || "", /plain language/);
  assert.equal(enView.getByTestId("guided-tour-next-btn").textContent?.trim(), "Next");
  cleanup();

  window.localStorage.setItem(LOCALE_STORAGE_KEY, "ja");
  const jaView = render(
    <I18nProvider>
      <MockWorkspaceAnchors />
      <DesktopGuidedTour open={true} onFinish={() => {}} />
    </I18nProvider>
  );
  assert.match(jaView.getByTestId("guided-tour-step-title").textContent || "", /自然な言葉で/);
  assert.equal(jaView.getByTestId("guided-tour-next-btn").textContent?.trim(), "次へ");
});
