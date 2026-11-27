import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React, { useState } from "react";
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
import type { DesktopAgentBridge } from "@/shell/api/agentBridgeTypes";
import type { AgentChannelSave, AgentChannelView } from "@/lib/generated/v3-contracts";
import {
  __setEOStageAccountTestAdapter,
  subscribeEOStageAccountChanged,
  type EOStageAccountStatus,
  type EOStageBillingConfig,
  type EOStageOrder,
} from "@/shell/api/eostageAccount";
import { subscribeChannelsChanged } from "@/shell/api/agentChannelsEvents";

// tsx does not provide Vite's import.meta.env. Patch only the modules loaded here.
const scopedSource = (url: string) =>
  `import.meta.env ??= { DEV: true };\n${readFileSync(fileURLToPath(url), "utf8")}`;
const hooks = registerHooks({
  load(url, context, nextLoad) {
    if (
      url.endsWith("/DesktopActivityRail.tsx") ||
      url.endsWith("/DesktopAgentWorkbench.tsx") ||
      url.endsWith("/ModelChannelsSettingsTab.tsx")
    ) {
      return {
        format: "module",
        shortCircuit: true,
        source: transformSync(scopedSource(url), { loader: "tsx", format: "esm" }).code,
      };
    }
    return nextLoad(url, context);
  },
});
const { DesktopActivityRail } = await import("./DesktopActivityRail");
const { EOStageLoginModal } = await import("./EOStageLoginModal");
const { EOStageBillingModal } = await import("./EOStageBillingModal");
const { AgentModelPicker } = await import("./AgentModelPicker");
const { ModelChannelsSettingsTab } = await import(
  "@/shell/components/settings/ModelChannelsSettingsTab"
);
hooks.deregister();

const LOGGED_IN_ACCOUNT: EOStageAccountStatus = {
  logged_in: true,
  session_expired: false,
  email: "user@example.com",
  display_name: "demo_user",
  status: "active",
  trial_granted: true,
  total_credits: 3000,
  remaining_credits: 3000,
  trial_credits: 3000,
  account_credits: 0,
  used_credits: 0,
  initial_trial_credits: 3000,
  total_recharged_credits: 0,
  trial_expires_at: 1800000000,
  active_sessions: 1,
  proxy_base_url: "https://api.eostage.org/eostage/v1/proxy/v1",
};

const OFFICIAL_CHANNEL: AgentChannelView = {
  id: "eostage-official",
  protocol: "chat_completions",
  base_url: "https://api.eostage.org/eostage/v1/proxy/v1",
  model: "glm-5.3-flash",
  reasoning_mode: "default",
  models: [
    {
      id: "glm-5.3-flash",
      name: "GLM-5.3 Flash",
      context_window: 131072,
      max_output_tokens: 16384,
      input_types: ["text"],
      supports_tools: true,
      reasoning_levels: [
        { level: "off", value: null },
        { level: "low", value: "low" },
        { level: "medium", value: "medium" },
        { level: "high", value: "high" },
      ],
    },
  ],
  key_ready: true,
  is_default: false,
};

const CUSTOM_DEFAULT_CHANNEL: AgentChannelView = {
  id: "custom-openai",
  protocol: "responses",
  base_url: "https://api.example.invalid/v1",
  model: "gpt-5.4",
  reasoning_mode: "default",
  models: [
    {
      id: "gpt-5.4",
      name: "GPT-5.4",
      context_window: 200000,
      input_types: ["text"],
      reasoning_levels: [{ level: "low", value: "low" }],
    },
  ],
  key_ready: true,
  is_default: true,
};

test.afterEach(() => {
  cleanup();
  clearTestDesktopShellBridge();
  __setEOStageAccountTestAdapter(null);
  window.localStorage.clear();
});

test("1. Sidebar account button -> monochrome login modal -> IME guard & fallback code -> popover -> billing modal", async () => {
  let account: EOStageAccountStatus | null = null;
  __setEOStageAccountTestAdapter({
    status: async () =>
      account ?? {
        logged_in: false,
        session_expired: false,
        email: null,
        display_name: null,
        status: "logged_out",
        trial_granted: false,
        total_credits: 0,
        remaining_credits: 0,
        trial_credits: 0,
        account_credits: 0,
        used_credits: 0,
        initial_trial_credits: 0,
        total_recharged_credits: 0,
        trial_expires_at: null,
        active_sessions: 0,
        proxy_base_url: null,
      },
    loginStart: async () => ({
      session_id: "eos_login_1",
      authorize_url: "https://api.eostage.org/auth/login?state=s1",
      expires_in_secs: 300,
    }),
    loginPoll: async () => ({ status: "pending", account: null, message: null }),
    loginCompleteCode: async (code) => {
      if (code !== "eos_tkt_valid_123") {
        throw new Error("登录码无效");
      }
      account = { ...LOGGED_IN_ACCOUNT };
      return account;
    },
    billingConfig: async () => ({
      config_version: 1,
      plans_enabled: false,
      orders_enabled: false,
      redeem_enabled: true,
      credits_per_cny: 1000,
      trial_credits: 3000,
      trial_expire_days: 30,
      currency: "CNY",
      currency_symbol: "¥",
      checkout_provider: "afdian",
      checkout_notice: "内测阶段暂未开放线上充值",
      announcement: "",
      plans: [],
    }),
    ordersList: async () => [],
    usageList: async () => ({ items: [] }),
  });

  function Harness() {
    const [acct, setAcct] = useState<EOStageAccountStatus | null>(null);
    const [loginOpen, setLoginOpen] = useState(false);
    const [billingOpen, setBillingOpen] = useState(false);
    return (
      <>
        <DesktopActivityRail
          activeRoute="workbench"
          onNavigate={() => {}}
          expanded
          sidebarWidth={248}
          accountStatus={acct}
          onOpenLogin={() => setLoginOpen(true)}
          onOpenBilling={() => setBillingOpen(true)}
        />
        <EOStageLoginModal
          open={loginOpen}
          onClose={() => setLoginOpen(false)}
          onSuccess={(next) => setAcct(next)}
        />
        <EOStageBillingModal
          open={billingOpen}
          account={acct}
          onClose={() => setBillingOpen(false)}
        />
      </>
    );
  }

  const view = render(<Harness />);
  const railBtn = view.getByTestId("rail-account-btn");
  assert.ok(railBtn.textContent?.includes("登录 EOStage"));
  fireEvent.click(railBtn);

  await waitFor(() => assert.ok(view.getByTestId("eostage-login-modal")));
  await waitFor(() =>
    assert.ok(view.getByTestId("eostage-login-status-text").textContent?.includes("登录链接已就绪"))
  );
  fireEvent.click(view.getByTestId("eostage-login-toggle-fallback-btn"));
  const codeInput = view.getByTestId("eostage-login-fallback-input");

  // Chinese IME composition Enter must not submit incomplete text or trigger error
  fireEvent.keyDown(codeInput, { key: "Enter", keyCode: 229, isComposing: true });
  assert.equal(view.queryByTestId("eostage-login-fallback-error"), null);

  // Invalid submit displays inline error directly below input and clears immediately on edit
  fireEvent.change(codeInput, { target: { value: "bad-code" } });
  fireEvent.click(view.getByTestId("eostage-login-fallback-submit-btn"));
  await waitFor(() =>
    assert.ok(view.getByTestId("eostage-login-fallback-error").textContent?.includes("登录码无效"))
  );
  fireEvent.change(codeInput, { target: { value: "eos_tkt_valid_123" } });
  assert.equal(view.queryByTestId("eostage-login-fallback-error"), null);

  fireEvent.click(view.getByTestId("eostage-login-fallback-submit-btn"));
  await waitFor(() => assert.ok(view.getByTestId("rail-account-btn").textContent?.includes("user@example.com")));

  // Clicking logged-in footer opens popover, clicking 余额与账单 opens modal
  fireEvent.click(view.getByTestId("rail-account-btn"));
  await waitFor(() => assert.ok(view.getByTestId("rail-account-popover")));
  fireEvent.click(view.getByTestId("rail-account-billing-btn"));
  await waitFor(() => assert.ok(view.getByTestId("eostage-billing-modal")));
  assert.equal(view.getByTestId("billing-total-credits").textContent, "3,000");
});

test("2. Server-driven billing config toggles plans tab without client update + expands AI usage details", async () => {
  let remoteConfig: EOStageBillingConfig = {
    config_version: 1,
    plans_enabled: false,
    orders_enabled: false,
    redeem_enabled: true,
    credits_per_cny: 1000,
    trial_credits: 3000,
    trial_expire_days: 30,
    currency: "CNY",
    currency_symbol: "¥",
    checkout_provider: "afdian",
    checkout_notice: "内测暂不开启支付",
    announcement: "",
    plans: [
      {
        id: "pack_standard_19",
        title: "标准积分包",
        price_cny: 19,
        credits: 20000,
        validity_days: null,
        badge: "推荐",
        description: "20,000 永久积分",
        checkout_url: "",
        enabled: true,
      },
    ],
  };
  const createdOrders: EOStageOrder[] = [];

  __setEOStageAccountTestAdapter({
    status: async () => ({ ...LOGGED_IN_ACCOUNT }),
    billingConfig: async () => remoteConfig,
    ordersList: async () => createdOrders,
    orderCreate: async (planId) => {
      const ord: EOStageOrder = {
        order_no: "EO20261120ABCD",
        plan_id: planId,
        title: "标准积分包",
        amount_fen: 1900,
        credits: 20000,
        provider: "afdian",
        status: "pending",
        pay_url: "https://afdian.com/a/eostage?remark=EO20261120ABCD",
        created_at: 1795177800,
        paid_at: null,
        cancelled_at: null,
      };
      createdOrders.push(ord);
      return ord;
    },
    usageList: async () => ({
      items: [
        {
          id: "u1",
          request_id: "req-glm-9981",
          model: "glm-5.3-flash",
          created_at: 1795177816,
          credits: 60.616,
          quota_units: 4152,
          prompt_tokens: 14282,
          cached_tokens: 0,
          completion_tokens: 218,
          use_time_ms: 2100,
          uncached_input_credits_per_1m: 803,
          cached_input_credits_per_1m: 803,
          output_credits_per_1m: 2811,
          status: "settled",
        },
      ],
    }),
  });

  // Beta mode: plans_enabled=false hides 充值与套餐 tab
  const betaView = render(
    <EOStageBillingModal open account={LOGGED_IN_ACCOUNT} onClose={() => {}} />
  );
  await waitFor(() => assert.ok(betaView.getByTestId("billing-balance-view")));
  assert.equal(betaView.queryByTestId("billing-tab-plans"), null);

  // Open AI Usage tab & expand request detail
  fireEvent.click(betaView.getByTestId("billing-tab-usage"));
  await waitFor(() => assert.ok(betaView.getByTestId("usage-toggle-u1")));
  fireEvent.click(betaView.getByTestId("usage-toggle-u1"));
  const detail = betaView.getByTestId("usage-detail-u1");
  assert.ok(detail.textContent?.includes("14,282"));
  assert.ok(detail.textContent?.includes("218"));
  assert.ok(detail.textContent?.includes("803 Credits"));
  assert.ok(detail.textContent?.includes("req-glm-9981"));
  cleanup();

  // Remote config flipped to plans_enabled=true on server WITHOUT client code change!
  remoteConfig = { ...remoteConfig, plans_enabled: true, orders_enabled: true };
  const liveView = render(
    <EOStageBillingModal open account={LOGGED_IN_ACCOUNT} onClose={() => {}} />
  );
  const plansTab = await waitFor(() => liveView.getByTestId("billing-tab-plans"));
  fireEvent.click(plansTab);
  await waitFor(() => assert.ok(liveView.getByTestId("billing-plan-card-pack_standard_19")));
  fireEvent.click(liveView.getByTestId("billing-checkout-btn"));
  await waitFor(() => assert.ok(liveView.getByTestId("billing-orders-view")));
  assert.ok(liveView.getByTestId("billing-orders-view").textContent?.includes("EO20261120ABCD"));
});

test("3. Official channel in Model Settings uses unified ChannelModelsEditor & saves params with api_key=null", async () => {
  const savedPayloads: AgentChannelSave[] = [];
  const base = createWorkbenchTestDesktopShellBridge();
  registerTestDesktopShellBridge({
    ...base,
    agent: {
      channelsGet: async () => [OFFICIAL_CHANNEL, CUSTOM_DEFAULT_CHANNEL],
      channelsSave: async (payload) => {
        savedPayloads.push(payload);
        return [
          { ...OFFICIAL_CHANNEL, model: payload.model, models: payload.models ?? [] },
          CUSTOM_DEFAULT_CHANNEL,
        ];
      },
      channelsModels: async () => ["glm-5.3-flash"],
      channelsProbeDraft: async () => ({
        result: { ok: true, code: null, status: 200, model_count: 1 },
        models: ["glm-5.3-flash"],
      }),
    } as DesktopAgentBridge,
  });
  __setEOStageAccountTestAdapter({
    status: async () => ({ ...LOGGED_IN_ACCOUNT }),
  });

  const view = render(<ModelChannelsSettingsTab />);
  fireEvent.click(await waitFor(() => view.getByTestId("official-provider-eostage-official")));
  await waitFor(() => assert.ok(view.getByTestId("official-account-summary-bar")));

  // Base URL and API Key inputs must NOT appear for official channel
  assert.equal(view.queryByLabelText("Base URL"), null);
  assert.equal(view.queryByLabelText("API 密钥"), null);

  // Unified ChannelModelsEditor gear dialog opens and edits context_window
  const editBtn = await waitFor(() =>
    view.getByRole("button", { name: /设置模型 GLM-5\.3 Flash/ })
  );
  fireEvent.click(editBtn);
  const ctxInput = (await waitFor(() =>
    view.getByLabelText("上下文窗口")
  )) as HTMLInputElement;
  fireEvent.change(ctxInput, { target: { value: "200000" } });
  fireEvent.click(view.getByRole("button", { name: "完成" }));

  await waitFor(() => assert.equal(savedPayloads.length, 1));
  assert.equal(savedPayloads[0].id, "eostage-official");
  assert.equal(savedPayloads[0].api_key, null);
  assert.equal(savedPayloads[0].models?.[0]?.context_window, 200000);
});

test("4. AgentModelPicker always pins EOStage 官方 first above custom default channel and hides 默认模型 pseudo-row", async () => {
  const view = render(
    <AgentModelPicker
      channels={[CUSTOM_DEFAULT_CHANNEL, OFFICIAL_CHANNEL]}
      sessionId="sess-1"
      ready={true}
    />
  );

  // Trigger displays active default model name (GPT-5.4), NOT "默认模型"
  assert.equal(view.getByTestId("agent-model-picker-label").textContent, "GPT-5.4");

  fireEvent.click(view.getByTestId("agent-model-picker-trigger"));
  await waitFor(() => view.getByTestId("model-picker-models-pane"));
  assert.equal(view.queryByTestId("model-picker-channels-pane"), null);
  fireEvent.click(view.getByTestId("model-picker-active-channel-header"));
  const pane = await waitFor(() => view.getByTestId("model-picker-channels-pane"));
  assert.equal(view.queryByTestId("model-picker-default-option"), null);

  const channelButtons = Array.from(
    pane.querySelectorAll("button[data-testid^='model-picker-channel-']")
  ).map((el) => el.getAttribute("data-testid"));
  assert.deepEqual(channelButtons, [
    "model-picker-channel-eostage-official",
    "model-picker-channel-custom-openai",
  ]);
  assert.ok(pane.textContent?.includes("EOStage 官方"));
  assert.ok(pane.textContent?.includes("官方"));
});

test("5. Monochrome FieldValidationPopover anchors on invalid submit, syncs ARIA, and dismisses on Escape/input", async () => {
  __setEOStageAccountTestAdapter({
    status: async () => ({ ...LOGGED_IN_ACCOUNT }),
    loginStart: async () => ({
      session_id: "eos_login_v",
      authorize_url: "https://api.eostage.org/auth/login?state=sv",
      expires_in_secs: 300,
    }),
    loginPoll: async () => ({ status: "pending", account: null, message: null }),
    billingConfig: async () => ({
      config_version: 1,
      plans_enabled: false,
      orders_enabled: false,
      redeem_enabled: true,
      credits_per_cny: 1000,
      trial_credits: 3000,
      trial_expire_days: 30,
      currency: "CNY",
      currency_symbol: "¥",
      checkout_provider: "afdian",
      checkout_notice: "",
      announcement: "",
      plans: [],
    }),
    ordersList: async () => [],
    usageList: async () => ({ items: [] }),
  });

  const loginView = render(<EOStageLoginModal open onClose={() => {}} />);
  await waitFor(() => assert.ok(loginView.getByTestId("eostage-login-toggle-fallback-btn")));
  fireEvent.click(loginView.getByTestId("eostage-login-toggle-fallback-btn"));

  const codeInput = loginView.getByTestId("eostage-login-fallback-input") as HTMLInputElement;
  assert.equal(codeInput.getAttribute("aria-invalid"), "false");
  assert.equal(codeInput.getAttribute("aria-describedby"), null);

  // Empty submit triggers monochrome callout & sets aria-invalid + aria-describedby
  fireEvent.click(loginView.getByTestId("eostage-login-fallback-submit-btn"));
  await waitFor(() => assert.ok(loginView.getByTestId("eostage-login-fallback-error")));
  assert.equal(codeInput.getAttribute("aria-invalid"), "true");
  assert.equal(codeInput.getAttribute("aria-describedby"), "eostage-login-fallback-error");
  assert.ok(loginView.getByTestId("eostage-login-fallback-error").className.includes("sp-modern"));

  // Escape dismisses the popover without closing the modal
  fireEvent.keyDown(document, { key: "Escape" });
  await waitFor(() => assert.equal(loginView.queryByTestId("eostage-login-fallback-error"), null));
  assert.equal(codeInput.getAttribute("aria-invalid"), "false");
  assert.ok(loginView.getByTestId("eostage-login-modal"));

  // Billing redeem empty submit triggers anchored popover and clears immediately on typing
  const billingView = render(
    <EOStageBillingModal open account={LOGGED_IN_ACCOUNT} onClose={() => {}} />
  );
  const redeemInput = await waitFor(
    () => billingView.getByTestId("billing-redeem-input") as HTMLInputElement
  );
  assert.equal(redeemInput.getAttribute("aria-invalid"), "false");
  fireEvent.click(billingView.getByTestId("billing-redeem-submit-btn"));
  await waitFor(() => assert.ok(billingView.getByTestId("billing-redeem-error")));
  assert.equal(redeemInput.getAttribute("aria-invalid"), "true");
  assert.equal(redeemInput.getAttribute("aria-describedby"), "eostage-billing-redeem-error");

  fireEvent.change(redeemInput, { target: { value: "EOS-TEST-001" } });
  await waitFor(() => assert.equal(billingView.queryByTestId("billing-redeem-error"), null));
  assert.equal(redeemInput.getAttribute("aria-invalid"), "false");
});

test("6. subscribeEOStageAccountChanged fires exactly once per notification and stops after unsubscribe", async () => {
  const { notifyEOStageAccountChanged, subscribeEOStageAccountChanged } = await import(
    "@/shell/api/eostageAccount"
  );
  let calls = 0;
  const unsub = subscribeEOStageAccountChanged(() => {
    calls += 1;
  });
  notifyEOStageAccountChanged();
  assert.equal(calls, 1);
  notifyEOStageAccountChanged();
  assert.equal(calls, 2);
  unsub();
  notifyEOStageAccountChanged();
  assert.equal(calls, 2);
});

test("7. Collapsed sidebar opens unified spring Credits popover and BillingModal stays fixed-height across tabs with 60-day countdown", async () => {
  const futureExp = Math.floor(Date.now() / 1000) + 59 * 86400;
  const trialAccount: EOStageAccountStatus = {
    ...LOGGED_IN_ACCOUNT,
    trial_expires_at: futureExp,
  };
  __setEOStageAccountTestAdapter({
    status: async () => ({ ...trialAccount }),
    billingConfig: async () => ({
      config_version: 2,
      plans_enabled: false,
      orders_enabled: false,
      redeem_enabled: true,
      credits_per_cny: 1000,
      trial_credits: 3000,
      trial_expire_days: 60,
      currency: "CNY",
      currency_symbol: "¥",
      checkout_provider: "afdian",
      checkout_notice: "",
      announcement: "",
      plans: [],
    }),
    ordersList: async () => [],
    usageList: async () => ({ items: [] }),
  });

  const railView = render(
    <DesktopActivityRail
      activeRoute="workbench"
      onNavigate={() => {}}
      expanded={false}
      sidebarWidth={52}
      accountStatus={trialAccount}
    />
  );
  fireEvent.click(railView.getByTestId("rail-account-btn"));
  const popover = await waitFor(() => railView.getByTestId("rail-account-popover"));
  assert.ok(popover.className.includes("sp-spring-dropdown-surface"));
  const navEl = railView.container.querySelector("nav");
  assert.ok(navEl?.className.includes("overflow-visible"), "collapsed rail must allow account popover overflow");
  assert.ok(popover.textContent?.includes("3,000 Credits"));
  cleanup();

  const billingView = render(
    <EOStageBillingModal open account={trialAccount} onClose={() => {}} />
  );
  const modalCard = billingView.getByTestId("eostage-billing-modal").firstElementChild as HTMLElement;
  assert.ok(modalCard.className.includes("h-[min(600px,84vh)]"));
  const validity = await waitFor(() => billingView.getByTestId("billing-trial-validity"));
  assert.ok(validity.textContent?.includes("60 天"));
  assert.ok(validity.textContent?.includes("剩余"));
});

test("8. AgentModelPicker shows sync button only for official channel, syncs models inline without closing, clears on switch/close, and propagates 401 expiry", async () => {
  let syncCalls = 0;
  let shouldExpire = false;
  __setEOStageAccountTestAdapter({
    syncModels: async () => {
      syncCalls += 1;
      if (shouldExpire) {
        throw new Error("登录状态已失效，请重新登录");
      }
      return [];
    },
  });

  let channelEvents = 0;
  let accountEvents = 0;
  const unsubCh = subscribeChannelsChanged(() => {
    channelEvents += 1;
  });
  const unsubAc = subscribeEOStageAccountChanged(() => {
    accountEvents += 1;
  });

  try {
    const view = render(
      <AgentModelPicker
        channels={[CUSTOM_DEFAULT_CHANNEL, OFFICIAL_CHANNEL]}
        sessionId="sess-sync-test"
        ready={true}
      />
    );

    fireEvent.click(view.getByTestId("agent-model-picker-trigger"));
    await waitFor(() => view.getByTestId("model-picker-models-pane"));
    assert.equal(view.queryByTestId("model-picker-channels-pane"), null);
    fireEvent.click(view.getByTestId("model-picker-active-channel-header"));
    await waitFor(() => view.getByTestId("model-picker-channels-pane"));

    // Custom channel is initially selected -> sync button must NOT render
    assert.equal(view.queryByTestId("model-picker-sync-official-btn"), null);

    // Switch right pane to EOStage 官方 -> sync button appears
    fireEvent.click(view.getByTestId("model-picker-channel-eostage-official"));
    const syncBtn = await waitFor(() =>
      view.getByTestId("model-picker-sync-official-btn")
    );
    assert.equal(syncBtn.getAttribute("role"), null);

    fireEvent.click(syncBtn);
    await waitFor(() => assert.equal(syncCalls, 1));
    await waitFor(() => assert.equal(channelEvents, 1));
    const statusOk = await waitFor(() =>
      view.getByTestId("model-picker-sync-status")
    );
    assert.ok(statusOk.textContent?.includes("已同步最新官方模型"));
    assert.ok(view.queryByTestId("agent-model-picker-popover"));

    // Switching back to custom channel clears sync status & hides sync button
    fireEvent.click(view.getByTestId("model-picker-channel-custom-openai"));
    assert.equal(view.queryByTestId("model-picker-sync-official-btn"), null);
    assert.equal(view.queryByTestId("model-picker-sync-status"), null);

    // Switch back to official and simulate 401 expiry error -> emits account & channel changed events
    fireEvent.click(view.getByTestId("model-picker-channel-eostage-official"));
    shouldExpire = true;
    const syncBtn2 = await waitFor(() =>
      view.getByTestId("model-picker-sync-official-btn")
    );
    fireEvent.click(syncBtn2);
    await waitFor(() => assert.equal(syncCalls, 2));
    await waitFor(() => assert.equal(accountEvents, 1));
    await waitFor(() => assert.equal(channelEvents, 2));
    const statusErr = await waitFor(() =>
      view.getByTestId("model-picker-sync-status")
    );
    assert.ok(statusErr.textContent?.includes("失效"));

    // Closing popover clears sync status
    fireEvent.keyDown(document, { key: "Escape" });
    await waitFor(() =>
      assert.equal(view.queryByTestId("agent-model-picker-popover"), null)
    );
  } finally {
    unsubCh();
    unsubAc();
  }
});

test("9. DesktopWorkbench startup auto-sync & sidebar refresh strictly gate on logged_in and refresh=true (PRIV-REG-001)", () => {
  const wbPath = fileURLToPath(new URL("./DesktopWorkbench.tsx", import.meta.url));
  const src = readFileSync(wbPath, "utf8");
  assert.ok(src.includes("officialModelsAutoSyncedRef"), "Missing single-flight startup sync ref");
  assert.ok(src.includes("if (!st.logged_in)"), "Missing logged-out gate before auto-sync");
  assert.ok(
    src.includes("else if (refresh && !officialModelsAutoSyncedRef.current)"),
    "Startup auto-sync must gate on refresh === true so cached cold read does not sync twice"
  );
  assert.ok(
    src.includes("eostageAccountSyncModels") && src.includes("refreshingAccountRef"),
    "Sidebar account refresh must deduplicate and also sync official models when logged in"
  );
});
