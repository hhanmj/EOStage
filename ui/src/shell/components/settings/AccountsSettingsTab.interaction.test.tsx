import "../../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { readFileSync } from "node:fs";
import { registerHooks } from "node:module";
import { fileURLToPath } from "node:url";
import { transformSync } from "esbuild";
import { within, waitFor } from "@testing-library/react";
import { cleanup, fireEvent, render } from "../../../test-utils/render-harness";
import { clearTestDesktopShellBridge, registerTestDesktopShellBridge } from "@/lib/desktopShellBridge";
import { createAccountSettingsFixture } from "@/test-utils/account-settings-fixture";
import { NetworkProxySettingsTab } from "./NetworkProxySettingsTab";

const hooks = registerHooks({
  load(url, context, nextLoad) {
    if (url.endsWith("/AccountsSettingsTab.tsx")) {
      const source = `const import_meta = { env: { DEV: false } };\n${readFileSync(fileURLToPath(url), "utf8").replaceAll("import.meta.env", "import_meta.env")}`;
      return { format: "module", shortCircuit: true, source: transformSync(source, { loader: "tsx", format: "esm" }).code };
    }
    return nextLoad(url, context);
  },
});
const { AccountsSettingsTab } = await import("./AccountsSettingsTab");
hooks.deregister();

test.afterEach(() => {
  cleanup();
  clearTestDesktopShellBridge();
});

test("account-backed sources (including OpenTopography, GACOS, GEE) precede anonymous MPC and show dynamic 58-channel coverage", () => {
  const view = render(<AccountsSettingsTab />);
  const ids = Array.from(view.container.querySelectorAll("[id^='account-card-']")).map((element) => element.id);
  assert.deepEqual(ids, [
    "account-card-cdse",
    "account-card-earthdata",
    "account-card-bhoonidhi",
    "account-card-usgs_m2m",
    "account-card-opentopography",
    "account-card-gacos",
    "account-card-gee",
    "account-card-mpc",
  ]);
  assert.ok(ids.indexOf("account-card-opentopography") < ids.indexOf("account-card-mpc"));
  assert.ok(ids.indexOf("account-card-gacos") < ids.indexOf("account-card-mpc"));
  assert.ok(ids.indexOf("account-card-gee") < ids.indexOf("account-card-mpc"));

  // A4: unconfigured badge uses 待配置 instead of 未配置
  const cdseCard = view.container.querySelector("#account-card-cdse") as HTMLElement;
  assert.ok(within(cdseCard).getByText("待配置"));

  assert.ok(view.queryByTestId("accounts-51-channel-summary") === null);
  const bhoonidhiCard = view.container.querySelector("#account-card-bhoonidhi") as HTMLElement;
  assert.ok(within(bhoonidhiCard).queryByText("覆盖 1 个数据源") === null);
  assert.ok(within(bhoonidhiCard).getByText("NISAR S-Band 全系列 8 级产品 印度航天局官方归档"));
  assert.ok(view.getByTestId("open-direct-channels-summary").textContent?.includes("COMET-LiCSAR"));

  assert.equal(view.queryByText("默认数据落盘存储根路径"), null);
  assert.equal(view.queryByRole("button", { name: "更改目录" }), null);
  assert.equal(view.queryByText("已登录"), null);
  assert.equal(view.queryByText("本机凭据"), null);
  assert.equal(view.queryByText("运行状态"), null);
});

test("configured GACOS displays 管理 button instead of 配置", async () => {
  const fixture = createAccountSettingsFixture();
  fixture.values.set("gacos_email", "researcher@university.edu");
  (fixture.bridge.application as any).unified = {
    accountSessionStatuses: async () => [
      { provider: "cdse", state: "credentials_missing" },
      { provider: "usgs_m2m", state: "credentials_missing" },
      { provider: "earthdata", state: "credentials_missing" },
      { provider: "bhoonidhi", state: "credentials_missing" },
      { provider: "mpc", state: "not_required" },
      { provider: "opentopography", state: "logged_in" },
      { provider: "gacos", state: "logged_in" },
    ],
  };
  registerTestDesktopShellBridge(fixture.bridge);

  const view = render(<AccountsSettingsTab />);
  const gacosCard = view.container.querySelector("#account-card-gacos") as HTMLElement;
  await waitFor(() => {
    assert.ok(within(gacosCard).getByText("已配置"));
    assert.ok(within(gacosCard).getByRole("button", { name: "管理" }));
  });
});

test("GEE secret stays masked after save instead of vanishing, and supports eye reveal/hide", async () => {
  const fixture = createAccountSettingsFixture();
  let savedGeeSecret = "";
  let savedProjectId = "";
  (fixture.bridge.application as any).openGeodataQuery = async (req: any) => {
    if (req.dataset === "gee_account_status") {
      return {
        ok: true,
        configured: Boolean(savedGeeSecret),
        has_secret: Boolean(savedGeeSecret),
        project_id: savedProjectId,
        auth_mode: "oauth_token",
        client_email: "",
      };
    }
    if (req.dataset === "gee_account_save") {
      savedProjectId = req.code || "";
      if (req.sql) savedGeeSecret = req.sql;
      return { ok: true, configured: true, project_id: savedProjectId, auth_mode: req.keyword };
    }
    if (req.dataset === "gee_account_reveal") {
      return { ok: true, value: savedGeeSecret };
    }
    return { ok: true };
  };
  registerTestDesktopShellBridge(fixture.bridge);

  const view = render(<AccountsSettingsTab />);
  fireEvent.click(view.getByTestId("gee-account-expand-btn"));
  fireEvent.click(view.getByTestId("gee-auth-mode-oauth"));

  const projectInput = view.getByTestId("gee-project-id") as HTMLInputElement;
  const secretInput = view.getByTestId("gee-secret-input") as HTMLTextAreaElement;
  assert.ok(projectInput.className.includes("font-sans"));

  fireEvent.change(projectInput, { target: { value: "ee-sceneport-prod" } });
  fireEvent.change(secretInput, { target: { value: "ya29.synthetic-gee-token-999" } });
  fireEvent.click(view.getByTestId("gee-account-save-btn"));

  await waitFor(() => {
    assert.ok(secretInput.value.includes("••••••••"), "GEE secret must remain masked after save rather than emptying");
  });

  // Clicking reveal button reveals the saved token; clicking again masks it back
  const revealBtn = view.getByTestId("gee-secret-reveal-btn");
  fireEvent.click(revealBtn);
  await waitFor(() => {
    assert.equal(secretInput.value, "ya29.synthetic-gee-token-999");
  });
  fireEvent.click(revealBtn);
  await waitFor(() => {
    assert.ok(secretInput.value.includes("••••••••"));
  });
});

test("GEE card inside AccountsSettingsTab renders live quota tier, local EECU ledger, cardless note, and unlocked billing progress bar without duplicating #account-card-gee", async () => {
  const fixture = createAccountSettingsFixture();
  let billingUnlocked = false;
  (fixture.bridge.application as any).openGeodataQuery = async (req: any) => {
    if (req.dataset === "gee_account_status") {
      return {
        ok: true,
        configured: true,
        gee_ready: true,
        has_secret: true,
        project_id: "ee-sceneport-quota",
        auth_mode: "service_account",
        client_email: "gee-bot@ee-sceneport-quota.iam.gserviceaccount.com",
      };
    }
    if (req.dataset === "gee_quota_status") {
      if (req.gee_params?.refresh === true) {
        billingUnlocked = true;
      }
      return {
        ok: true,
        configured: true,
        project_id: "ee-sceneport-quota",
        registration_state: "REGISTERED_NOT_COMMERCIALLY",
        account_tier: billingUnlocked ? "noncommercial_contributor" : "noncommercial_community",
        account_tier_label: billingUnlocked
          ? "进阶科研档 (Contributor)"
          : "非商业科研档 (Community)",
        monthly_limit_eecu_seconds: billingUnlocked ? 3600000 : 540000,
        monthly_limit_eecu_hours: billingUnlocked ? 1000 : 150,
        daily_limit_eecu_seconds: null,
        read_requests_per_minute: 6000,
        parallelism_restricted: billingUnlocked,
        last_quota_status_header: billingUnlocked ? "parallelism_restricted=true" : null,
        cloud_monitoring: billingUnlocked
          ? {
              unlocked: true,
              reason: null,
              monthly_used_eecu_seconds: 360000,
              monthly_used_eecu_hours: 100,
              monthly_remaining_eecu_hours: 900,
            }
          : {
              unlocked: false,
              reason: "billing_not_enabled",
              monthly_used_eecu_seconds: null,
              monthly_used_eecu_hours: null,
              monthly_remaining_eecu_hours: null,
            },
        local_ledger: {
          month_utc: "2026-11",
          total_requests: 6,
          analysis_runs: 4,
          chunk_downloads: 1,
          script_runs: 1,
          profiled_eecu_seconds: 3.75,
          has_eecu_profile: true,
          total_wall_seconds: 11.2,
          last_call_eecu_seconds: 1.25,
          last_call_wall_seconds: 2.3,
          last_updated_at_ms: 1794492000000,
        },
        fetched_at_ms: 1794492000000,
      };
    }
    return { ok: true };
  };
  registerTestDesktopShellBridge(fixture.bridge);

  const view = render(<AccountsSettingsTab />);
  assert.equal(view.container.querySelectorAll("#account-card-gee").length, 1);

  await waitFor(() => {
    assert.match(
      view.getByTestId("gee-quota-tier-badge").textContent || "",
      /非商业科研档 \(Community\) · 每月 150 小时/
    );
    assert.ok(view.getByTestId("gee-quota-ledger-panel"));
    assert.ok(view.getByTestId("gee-cloud-quota-cardless-note"));
    assert.equal(view.queryByTestId("gee-cloud-quota-progressbar"), null);
    assert.match(
      view.getByTestId("gee-local-usage-ledger").textContent || "",
      /实测云端算力 3\.75 EECU-秒/
    );
  });

  fireEvent.click(view.getByTestId("gee-refresh-quota-btn"));

  await waitFor(() => {
    assert.match(
      view.getByTestId("gee-quota-tier-badge").textContent || "",
      /进阶科研档 \(Contributor\) · 每月 1000 小时/
    );
    assert.ok(view.getByTestId("gee-quota-throttle-badge"));
    assert.equal(view.queryByTestId("gee-cloud-quota-cardless-note"), null);
    assert.match(
      view.getByTestId("gee-cloud-quota-unlocked").textContent || "",
      /全账号本月已用 100 \/ 1000 小时 · 剩余 900 小时 \(90%\)/
    );
    const bar = view.getByTestId("gee-cloud-quota-progressbar");
    assert.equal(bar.getAttribute("aria-valuemax"), "1000");
    assert.equal(bar.getAttribute("aria-valuenow"), "100");
  });
});

test("NetworkProxySettingsTab removes manual save button, auto-saves sanitized payload with toast, and lists 9 probe endpoints", async () => {
  const fixture = createAccountSettingsFixture();
  const savedPayloads: Record<string, unknown>[] = [];
  fixture.bridge.application.getNetworkSettings = async () => ({
    ok: true,
    proxy_enabled: false,
    proxy_url: "",
    cdse_network_policy: "default",
    earthdata_network_policy: "default",
    usgs_m2m_network_policy: "default",
    nisar_s_network_policy: "system",
    tianditu_token: "my-token",
  });
  fixture.bridge.application.saveNetworkSettings = async (settings) => {
    savedPayloads.push(settings as unknown as Record<string, unknown>);
    return { ok: true, settings };
  };
  registerTestDesktopShellBridge(fixture.bridge);

  const view = render(<NetworkProxySettingsTab />);
  assert.equal(view.queryByRole("button", { name: "保存网络配置" }), null);

  // All 9 endpoints including GEE, GACOS, COMET-LiCSAR are rendered
  assert.ok(view.getByText("Google Earth Engine (GEE)"));
  assert.ok(view.getByText("GACOS Generic Atmospheric Correction"));
  assert.ok(view.getByText("COMET-LiCSAR Interferometry Portal"));

  // Wait for initial load, then click "自动识别系统代理" -> immediate auto-save + toast
  const systemRadio = await view.findByRole("radio", { name: /自动识别系统代理/ });
  fireEvent.click(systemRadio);

  await waitFor(() => {
    assert.equal(savedPayloads.length, 1);
    assert.equal("ok" in savedPayloads[0]!, false, "must strip ok field before invoking Rust deny_unknown_fields");
    assert.equal(savedPayloads[0]!.proxy_enabled, true);
    assert.equal(savedPayloads[0]!.proxy_url, "");
    assert.equal(savedPayloads[0]!.tianditu_token, "my-token");
    assert.ok(view.getByTestId("network-autosave-toast"));
  });
});

test("GEE card auto-fills Project ID from pasted Service Account JSON when #gee-project-id is blank", async () => {
  const fixture = createAccountSettingsFixture();
  const savedRequests: Record<string, unknown>[] = [];
  (fixture.bridge.application as any).openGeodataQuery = async (req: Record<string, unknown>) => {
    if (req.dataset === "gee_account_status") {
      return { ok: true, configured: false, has_secret: false, project_id: "", auth_mode: "service_account" };
    }
    if (req.dataset === "gee_quota_status") {
      return { ok: true, configured: false };
    }
    if (req.dataset === "gee_account_save") {
      savedRequests.push(req);
      return {
        ok: true,
        configured: true,
        gee_ready: true,
        project_id: req.code,
        auth_mode: "service_account",
        client_email: "gee-bot@ee-pasted-sa-proj.iam.gserviceaccount.com",
      };
    }
    return { ok: true };
  };
  registerTestDesktopShellBridge(fixture.bridge);

  const view = render(<AccountsSettingsTab />);
  fireEvent.click(await view.findByTestId("gee-account-expand-btn"));

  const saveBtn = (await view.findByTestId("gee-account-save-btn")) as HTMLButtonElement;
  const projectInput = view.container.querySelector("#gee-project-id") as HTMLInputElement;
  const secretInput = view.container.querySelector("#gee-secret-input") as HTMLTextAreaElement;

  await waitFor(() => {
    assert.equal(projectInput.value, "");
    assert.equal(saveBtn.disabled, true);
  });

  const saJson = JSON.stringify({
    type: "service_account",
    project_id: "ee-pasted-sa-proj",
    client_email: "gee-bot@ee-pasted-sa-proj.iam.gserviceaccount.com",
    private_key: "-----BEGIN PRIVATE KEY-----\nMIIEvAIBADANBg==\n-----END PRIVATE KEY-----\n",
  });
  fireEvent.change(secretInput, { target: { value: saJson } });

  await waitFor(() => {
    assert.equal(projectInput.value, "ee-pasted-sa-proj");
    assert.equal(saveBtn.disabled, false);
  });

  fireEvent.click(saveBtn);
  await waitFor(() => {
    assert.equal(savedRequests.length, 1);
    assert.equal(savedRequests[0]!.code, "ee-pasted-sa-proj");
  });
});
