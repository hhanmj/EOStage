import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { readFileSync } from "node:fs";
import { registerHooks } from "node:module";
import { fileURLToPath } from "node:url";
import { transformSync } from "esbuild";
import { waitFor } from "@testing-library/react";
import { cleanup, fireEvent, render } from "@/test-utils/render-harness";
import { createAccountSettingsFixture } from "@/test-utils/account-settings-fixture";
import { clearTestDesktopShellBridge, registerTestDesktopShellBridge } from "@/lib/desktopShellBridge";
import { SubsetCornerBadge } from "./SubsetScissorsIcon";
import { DownloadTaskRow } from "./DownloadTaskRow";
import {
  DesktopBhoonidhiAccountPanel,
  DesktopCdseAccountPanel,
  DesktopEarthdataAccountPanel,
  DesktopUsgsAccountPanel,
} from "./DesktopAccountPanels";
import type { ApplicationTask } from "@/shell/api/types";

const hooks = registerHooks({
  load(url, context, nextLoad) {
    if (url.endsWith("/AccountsSettingsTab.tsx")) {
      const source = `const import_meta = { env: { DEV: false } };\n${readFileSync(fileURLToPath(url), "utf8").replaceAll("import.meta.env", "import_meta.env")}`;
      return {
        format: "module",
        shortCircuit: true,
        source: transformSync(source, { loader: "tsx", format: "esm" }).code,
      };
    }
    return nextLoad(url, context);
  },
});
const { AccountsSettingsTab } = await import("@/shell/components/settings/AccountsSettingsTab");
hooks.deregister();

test.afterEach(() => {
  cleanup();
  clearTestDesktopShellBridge();
});

test("AccountsSettingsTab and provider credential fields expose transparent-background portal icon buttons that open official URLs", async () => {
  const fixture = createAccountSettingsFixture();
  registerTestDesktopShellBridge(fixture.bridge);

  const view = render(
    <AccountsSettingsTab
      cdsePanel={<DesktopCdseAccountPanel />}
      earthdataPanel={<DesktopEarthdataAccountPanel />}
      bhoonidhiPanel={<DesktopBhoonidhiAccountPanel />}
      usgsPanel={<DesktopUsgsAccountPanel />}
    />,
  );

  // Every card header has a clickable portal icon button with transparent background
  for (const target of [
    "cdse",
    "earthdata",
    "bhoonidhi",
    "usgs_m2m",
    "gacos",
    "mpc",
    "opentopography",
    "gee",
  ]) {
    const btn = view.getByTestId(`account-portal-link-${target}`);
    assert.ok(btn.className.includes("bg-transparent"), `Expected bg-transparent on ${target}`);
    assert.ok(!btn.className.includes("bg-sp-accent"), `Must not use blue fill on ${target}`);
  }

  // Expand USGS, Earthdata, OpenTopography, and GEE cards directly inside AccountsSettingsTab
  const usgsCard = view.container.querySelector("#account-card-usgs_m2m") as HTMLElement;
  fireEvent.click(usgsCard.querySelector("button.cred-action-btn") as HTMLButtonElement);

  const earthdataCard = view.container.querySelector("#account-card-earthdata") as HTMLElement;
  fireEvent.click(earthdataCard.querySelector("button.cred-action-btn") as HTMLButtonElement);

  const otgCard = view.container.querySelector("#account-card-opentopography") as HTMLElement;
  fireEvent.click(otgCard.querySelector("button.cred-action-btn") as HTMLButtonElement);

  fireEvent.click(view.getByTestId("gee-account-expand-btn"));

  const geeProjectBtn = view.getByTestId("account-portal-link-gee_project_id");
  const geeSaBtn = view.getByTestId("account-portal-link-gee_service_account");
  assert.ok(geeProjectBtn.className.includes("bg-transparent"));
  assert.ok(geeSaBtn.className.includes("bg-transparent"));

  fireEvent.click(view.getByTestId("account-portal-link-usgs_token"));
  fireEvent.click(view.getByTestId("account-portal-link-opentopography_key"));
  fireEvent.click(geeProjectBtn);
  fireEvent.click(geeSaBtn);

  // Switch GEE credential mode to OAuth Token and verify oauth link updates dynamically
  fireEvent.click(view.getByTestId("gee-auth-mode-oauth"));
  const geeOauthBtn = view.getByTestId("account-portal-link-gee_oauth_token");
  assert.ok(geeOauthBtn.className.includes("bg-transparent"));
  fireEvent.click(geeOauthBtn);

  await waitFor(() => {
    assert.deepEqual(fixture.state.opened, [
      "https://ers.cr.usgs.gov/profile",
      "https://portal.opentopography.org/requestService?service=api",
      "https://console.cloud.google.com/earth-engine/configuration",
      "https://console.cloud.google.com/iam-admin/serviceaccounts",
      "https://developers.google.com/earth-engine/guides/auth",
    ]);
  });
});

test("SubsetCornerBadge and DownloadTaskRow subset scissors badge use minimalist transparent background without blue fill", () => {
  const subsetTask: ApplicationTask = {
    task_id: "task-nisar-subset-1",
    revision: 1,
    category: "scene_download",
    task_type: "nisar_subset",
    dataset: "NISAR",
    name: "NISAR L-band RSLC 子集下载",
    status: "running",
    done_bytes: 1024,
    total_bytes: 4096,
    scene_items: [],
  };

  const view = render(
    <div>
      <SubsetCornerBadge testId="catalog-scissors-badge" />
      <DownloadTaskRow task={subsetTask} />
    </div>,
  );

  const catalogBadge = view.getByTestId("catalog-scissors-badge");
  assert.ok(catalogBadge.className.includes("bg-transparent"));
  assert.ok(!catalogBadge.className.includes("bg-sp-accent"));
  assert.ok(!catalogBadge.className.includes("text-white"));

  const taskBadge = view.getByTestId("subset-scissors-badge");
  assert.ok(taskBadge.className.includes("bg-transparent"));
  assert.ok(!taskBadge.className.includes("bg-sp-accent"));
  assert.ok(!taskBadge.className.includes("text-white"));
});
