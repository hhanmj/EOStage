import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { cleanup, fireEvent, render } from "../../test-utils/render-harness";
import { waitFor } from "@testing-library/react";
import { registerHooks } from "node:module";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { transformSync } from "esbuild";
import {
  clearTestDesktopShellBridge,
  registerTestDesktopShellBridge,
} from "@/lib/desktopShellBridge";
import { createWorkbenchTestDesktopShellBridge } from "../../test-utils/workbenchDesktopBridge";
import { I18nProvider } from "@/i18n";

const hooks = registerHooks({
  load(url, context, nextLoad) {
    if (url.endsWith("/DesktopAgentWorkbench.tsx")) {
      const source = `import.meta.env ??= { DEV: true };\n${readFileSync(fileURLToPath(url), "utf8")}`;
      return {
        format: "module",
        shortCircuit: true,
        source: transformSync(source, { loader: "tsx", format: "esm" }).code,
      };
    }
    return nextLoad(url, context);
  },
});
const { DesktopActivityRail } = await import("./DesktopActivityRail");
hooks.deregister();

test.afterEach(() => {
  cleanup();
  clearTestDesktopShellBridge();
  window.localStorage.clear();
});

test("AboutEOStageModal opens from ? button in both expanded and collapsed rail, keeps privacy-compliant explicit update check, and preserves onboarding replay", async () => {
  let updateChecks = 0;
  let replayClicks = 0;
  let welcomeClicks = 0;
  const openedUrls: string[] = [];
  const base = createWorkbenchTestDesktopShellBridge();
  registerTestDesktopShellBridge({
    ...base,
    application: {
      ...base.application,
      checkAppUpdate: async () => {
        updateChecks += 1;
        return {
          current_version: "2.2.0-beta.0",
          latest_version: "v2.3.0",
          has_update: true,
          release_url: "https://github.com/hhanmj/ScenePort/releases/tag/v2.3.0",
          release_notes: "New features",
          checked_at: "2026-11-22T09:00:00Z",
          status: "update_available",
          message_zh: "发现新版本 v2.3.0（当前 2.2.0-beta.0），可前往下载安装。",
        };
      },
      openExternalUrl: async (url: string) => {
        openedUrls.push(url);
        return { ok: true, url };
      },
    },
  });

  // 1. Expanded rail -> ? button opens About modal, update check is NOT called automatically
  const view = render(
    <I18nProvider>
      <DesktopActivityRail
        activeRoute="chat"
        onNavigate={() => {}}
        sidebarCollapsed={false}
        onReplayOnboarding={() => {
          replayClicks += 1;
        }}
        onReplayWelcome={() => {
          welcomeClicks += 1;
        }}
      />
    </I18nProvider>
  );

  const helpBtn = view.getByRole("button", { name: "快捷键与帮助" });
  fireEvent.click(helpBtn);
  const modal = await waitFor(() => view.getByTestId("about-eostage-modal"));
  assert.ok(modal);
  assert.equal(updateChecks, 0, "PRIV-REG-001: opening About modal must NOT auto-check updates");

  // Onboarding replay button immediately visible and functional (run_full_e2e_audit M0)
  const replayBtn = view.getByTestId("rail-replay-onboarding-btn");
  assert.ok(replayBtn);

  // Explicit check update
  fireEvent.click(view.getByTestId("about-check-update-btn"));
  await waitFor(() => assert.equal(updateChecks, 1));
  assert.match(view.getByTestId("about-update-status").textContent || "", /v2\.3\.0/);

  // Shortcuts toggle
  assert.equal(view.queryByTestId("about-shortcuts-list"), null);
  fireEvent.click(view.getByTestId("about-toggle-shortcuts-btn"));
  assert.ok(view.getByTestId("about-shortcuts-list"));

  // Official horizontal brand logo is rendered cleanly without legacy InSAR-only tagline
  const iconImg = view.getByTestId("about-real-app-icon") as HTMLImageElement;
  assert.ok(iconImg.getAttribute("src")?.includes("/app-logo."));
  assert.ok(!modal.textContent?.includes("开源遥感与 InSAR 智能工作台"));

  // Legal & Contact QR views + back button
  fireEvent.click(view.getByTestId("about-open-terms-btn"));
  assert.ok(view.getByTestId("about-terms-view"));
  fireEvent.click(view.getByTestId("about-back-btn"));
  fireEvent.click(view.getByTestId("about-open-privacy-btn"));
  assert.ok(view.getByTestId("about-privacy-view"));
  fireEvent.click(view.getByTestId("about-back-btn"));

  fireEvent.click(view.getByTestId("about-contact-btn"));
  const contactView = view.getByTestId("about-contact-view");
  const qrImg = contactView.querySelector("img");
  assert.ok(qrImg?.getAttribute("src")?.includes("/contact/wechat_story.jpg"));
  fireEvent.click(view.getByTestId("about-back-btn"));

  // First 3 cards have no external URLs yet and do not trigger openExternalUrl
  assert.ok(!modal.textContent?.includes("github.com/hhanmj"));
  fireEvent.click(view.getByTestId("about-author-btn"));
  fireEvent.click(view.getByTestId("about-github-btn"));
  fireEvent.click(view.getByTestId("about-open-feedback-btn"));
  assert.equal(openedUrls.length, 0);

  // Welcome page replay button sits beside onboarding replay and invokes handler
  const welcomeBtn = view.getByTestId("rail-replay-welcome-btn");
  assert.ok(welcomeBtn);
  fireEvent.click(welcomeBtn);
  assert.equal(welcomeClicks, 1);
  assert.equal(view.queryByTestId("about-eostage-modal"), null);

  // Re-open and test Replay onboarding closes modal and invokes handler
  fireEvent.click(helpBtn);
  await waitFor(() => assert.ok(view.getByTestId("about-eostage-modal")));
  fireEvent.click(view.getByTestId("rail-replay-onboarding-btn"));
  assert.equal(replayClicks, 1);
  assert.equal(view.queryByTestId("about-eostage-modal"), null);
  view.unmount();

  // 2. Collapsed rail -> ? button is STILL present and opens About modal cleanly
  const collapsedView = render(
    <I18nProvider>
      <DesktopActivityRail
        activeRoute="chat"
        onNavigate={() => {}}
        sidebarCollapsed={true}
        onReplayOnboarding={() => {}}
      />
    </I18nProvider>
  );
  const collapsedHelpBtn = collapsedView.getByRole("button", { name: "快捷键与帮助" });
  fireEvent.click(collapsedHelpBtn);
  await waitFor(() => assert.ok(collapsedView.getByTestId("about-eostage-modal")));
});
