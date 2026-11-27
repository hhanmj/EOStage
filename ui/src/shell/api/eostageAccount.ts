import { isTauriDesktopHost, openExternalUrl } from "@/lib/bridge";

export const EOStageAccountChangedEvent = "eostage:eostage-account-changed";

export interface EOStageAccountStatus {
  logged_in: boolean;
  session_expired: boolean;
  email: string | null;
  display_name: string | null;
  status: string;
  trial_granted: boolean;
  total_credits: number;
  remaining_credits: number;
  trial_credits: number;
  account_credits: number;
  used_credits: number;
  initial_trial_credits: number;
  total_recharged_credits: number;
  trial_expires_at: number | null;
  active_sessions: number;
  proxy_base_url: string | null;
}

export interface EOStageLoginStartResult {
  session_id: string;
  authorize_url: string;
  expires_in_secs: number;
}

export interface EOStageLoginPollResult {
  status: "pending" | "authorized" | "expired" | "error" | string;
  account: EOStageAccountStatus | null;
  message: string | null;
}

export interface EOStagePlan {
  id: string;
  title: string;
  price_cny: number;
  credits: number;
  validity_days: number | null;
  badge: string;
  description: string;
  checkout_url: string;
  enabled: boolean;
}

export interface EOStageBillingConfig {
  config_version: number;
  plans_enabled: boolean;
  orders_enabled: boolean;
  redeem_enabled: boolean;
  credits_per_cny: number;
  trial_credits: number;
  trial_expire_days: number;
  currency: string;
  currency_symbol: string;
  checkout_provider: string;
  checkout_notice: string;
  announcement: string;
  plans: EOStagePlan[];
}

export interface EOStageOrder {
  order_no: string;
  plan_id: string;
  title: string;
  amount_fen: number;
  credits: number;
  provider: string;
  status: "pending" | "paid" | "cancelled" | string;
  pay_url: string;
  created_at: number;
  paid_at: number | null;
  cancelled_at: number | null;
}

export interface EOStageRedeemResult {
  added_credits: number;
  account: EOStageAccountStatus;
}

export interface EOStageUsageItem {
  id: string;
  request_id: string;
  model: string;
  created_at: number;
  credits: number;
  quota_units: number;
  prompt_tokens: number;
  cached_tokens: number;
  completion_tokens: number;
  use_time_ms: number;
  uncached_input_credits_per_1m: number | null;
  cached_input_credits_per_1m: number | null;
  output_credits_per_1m: number | null;
  status: string;
}

export interface EOStageUsageListResult {
  items: EOStageUsageItem[];
}

export interface EOStageAccountAdapter {
  status?: (refresh?: boolean) => Promise<EOStageAccountStatus>;
  loginStart?: () => Promise<EOStageLoginStartResult>;
  loginPoll?: (sessionId: string) => Promise<EOStageLoginPollResult>;
  loginCompleteCode?: (code: string) => Promise<EOStageAccountStatus>;
  logout?: () => Promise<EOStageAccountStatus>;
  syncModels?: () => Promise<unknown[]>;
  billingConfig?: () => Promise<EOStageBillingConfig>;
  redeem?: (code: string) => Promise<EOStageRedeemResult>;
  ordersList?: () => Promise<EOStageOrder[]>;
  orderCreate?: (planId: string) => Promise<EOStageOrder>;
  orderCheck?: (orderNo: string) => Promise<EOStageOrder>;
  orderCancel?: (orderNo: string) => Promise<EOStageOrder>;
  usageList?: (limit?: number) => Promise<EOStageUsageListResult>;
}

const DEFAULT_LOGGED_OUT_STATUS: EOStageAccountStatus = {
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
};

const DEFAULT_BILLING_CONFIG: EOStageBillingConfig = {
  config_version: 1,
  plans_enabled: false,
  orders_enabled: false,
  redeem_enabled: true,
  credits_per_cny: 1000,
  trial_credits: 3000,
  trial_expire_days: 60,
  currency: "CNY",
  currency_symbol: "¥",
  checkout_provider: "afdian",
  checkout_notice: "内测阶段暂未开放线上充值；如持有官方兑换码，可在下方直接兑换 Credits 享用。",
  announcement: "",
  plans: [],
};

let testAdapter: EOStageAccountAdapter | null = null;
const accountListeners = new Set<() => void>();

export function __setEOStageAccountTestAdapter(adapter: EOStageAccountAdapter | null): void {
  testAdapter = adapter;
}

export function subscribeEOStageAccountChanged(listener: () => void): () => void {
  accountListeners.add(listener);
  return () => {
    accountListeners.delete(listener);
  };
}

export function notifyEOStageAccountChanged(): void {
  for (const listener of [...accountListeners]) {
    try {
      listener();
    } catch {
      // isolate listener errors
    }
  }
  try {
    if (typeof window !== "undefined" && typeof window.dispatchEvent === "function") {
      const EventCtor =
        typeof window.CustomEvent === "function" ? window.CustomEvent : CustomEvent;
      window.dispatchEvent(new EventCtor(EOStageAccountChangedEvent));
    }
  } catch {
    // ignore environments where CustomEvent dispatch differs
  }
}

async function invokeNative<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(command, args);
}

export async function openEOStageBrowserUrl(url: string): Promise<void> {
  const hasNativeInvoke =
    typeof window !== "undefined" &&
    typeof (window as unknown as { __TAURI_INTERNALS__?: { invoke?: unknown } })
      .__TAURI_INTERNALS__?.invoke === "function";
  if (isTauriDesktopHost() || hasNativeInvoke) {
    try {
      await invokeNative("agent_oauth_open_browser", { url });
      return;
    } catch {
      // fall through to bridge openExternalUrl
    }
  }
  await openExternalUrl(url);
}

function extractMessage(err: unknown, fallback: string): string {
  if (typeof err === "string" && err.trim()) return err;
  if (err && typeof err === "object") {
    const rec = err as Record<string, unknown>;
    if (typeof rec.message === "string" && rec.message.trim()) return rec.message;
  }
  return fallback;
}

export async function eostageAccountStatus(refresh = false): Promise<EOStageAccountStatus> {
  if (testAdapter?.status) {
    return testAdapter.status(refresh);
  }
  if (!isTauriDesktopHost()) {
    return { ...DEFAULT_LOGGED_OUT_STATUS };
  }
  return invokeNative<EOStageAccountStatus>("eostage_account_status", { refresh });
}

export async function eostageAccountLoginStart(): Promise<EOStageLoginStartResult> {
  if (testAdapter?.loginStart) {
    return testAdapter.loginStart();
  }
  if (!isTauriDesktopHost()) {
    throw new Error("请在桌面客户端内完成账号登录");
  }
  try {
    return await invokeNative<EOStageLoginStartResult>("eostage_account_login_start");
  } catch (err) {
    throw new Error(extractMessage(err, "启动登录流程失败"));
  }
}

export async function eostageAccountLoginPoll(sessionId: string): Promise<EOStageLoginPollResult> {
  if (testAdapter?.loginPoll) {
    return testAdapter.loginPoll(sessionId);
  }
  if (!isTauriDesktopHost()) {
    return { status: "expired", account: null, message: "非桌面环境" };
  }
  try {
    return await invokeNative<EOStageLoginPollResult>("eostage_account_login_poll", {
      sessionId,
      session_id: sessionId,
    });
  } catch (err) {
    throw new Error(extractMessage(err, "轮询登录状态失败"));
  }
}

export async function eostageAccountLoginCompleteCode(code: string): Promise<EOStageAccountStatus> {
  if (testAdapter?.loginCompleteCode) {
    return testAdapter.loginCompleteCode(code);
  }
  if (!isTauriDesktopHost()) {
    throw new Error("请在桌面客户端内完成账号登录");
  }
  try {
    return await invokeNative<EOStageAccountStatus>("eostage_account_login_complete_code", {
      code,
    });
  } catch (err) {
    throw new Error(extractMessage(err, "登录码验证失败"));
  }
}

export async function eostageAccountLogout(): Promise<EOStageAccountStatus> {
  if (testAdapter?.logout) {
    return testAdapter.logout();
  }
  if (!isTauriDesktopHost()) {
    return { ...DEFAULT_LOGGED_OUT_STATUS };
  }
  return invokeNative<EOStageAccountStatus>("eostage_account_logout");
}

export async function eostageAccountSyncModels(): Promise<unknown[]> {
  if (testAdapter?.syncModels) {
    return testAdapter.syncModels();
  }
  if (!isTauriDesktopHost()) {
    return [];
  }
  try {
    return await invokeNative<unknown[]>("eostage_account_sync_models");
  } catch (err) {
    throw new Error(extractMessage(err, "同步官方模型目录失败"));
  }
}

export async function eostageBillingConfig(): Promise<EOStageBillingConfig> {
  if (testAdapter?.billingConfig) {
    return testAdapter.billingConfig();
  }
  if (!isTauriDesktopHost()) {
    return { ...DEFAULT_BILLING_CONFIG };
  }
  try {
    return await invokeNative<EOStageBillingConfig>("eostage_billing_config");
  } catch {
    return { ...DEFAULT_BILLING_CONFIG };
  }
}

export async function eostageBillingRedeem(code: string): Promise<EOStageRedeemResult> {
  if (testAdapter?.redeem) {
    return testAdapter.redeem(code);
  }
  if (!isTauriDesktopHost()) {
    throw new Error("请在桌面客户端内完成 Credits 兑换");
  }
  try {
    return await invokeNative<EOStageRedeemResult>("eostage_billing_redeem", { code });
  } catch (err) {
    throw new Error(extractMessage(err, "兑换失败"));
  }
}

export async function eostageBillingOrdersList(): Promise<EOStageOrder[]> {
  if (testAdapter?.ordersList) {
    return testAdapter.ordersList();
  }
  if (!isTauriDesktopHost()) {
    return [];
  }
  try {
    return await invokeNative<EOStageOrder[]>("eostage_billing_orders_list");
  } catch (err) {
    throw new Error(extractMessage(err, "获取订单记录失败"));
  }
}

export async function eostageBillingOrderCreate(planId: string): Promise<EOStageOrder> {
  if (testAdapter?.orderCreate) {
    return testAdapter.orderCreate(planId);
  }
  if (!isTauriDesktopHost()) {
    throw new Error("请在桌面客户端内创建订单");
  }
  try {
    return await invokeNative<EOStageOrder>("eostage_billing_order_create", {
      planId,
      plan_id: planId,
    });
  } catch (err) {
    throw new Error(extractMessage(err, "创建订单失败"));
  }
}

export async function eostageBillingOrderCheck(orderNo: string): Promise<EOStageOrder> {
  if (testAdapter?.orderCheck) {
    return testAdapter.orderCheck(orderNo);
  }
  if (!isTauriDesktopHost()) {
    throw new Error("请在桌面客户端内查询订单");
  }
  try {
    return await invokeNative<EOStageOrder>("eostage_billing_order_check", {
      orderNo,
      order_no: orderNo,
    });
  } catch (err) {
    throw new Error(extractMessage(err, "查询订单失败"));
  }
}

export async function eostageBillingOrderCancel(orderNo: string): Promise<EOStageOrder> {
  if (testAdapter?.orderCancel) {
    return testAdapter.orderCancel(orderNo);
  }
  if (!isTauriDesktopHost()) {
    throw new Error("请在桌面客户端内取消订单");
  }
  try {
    return await invokeNative<EOStageOrder>("eostage_billing_order_cancel", {
      orderNo,
      order_no: orderNo,
    });
  } catch (err) {
    throw new Error(extractMessage(err, "取消订单失败"));
  }
}

export async function eostageUsageList(limit = 30): Promise<EOStageUsageListResult> {
  if (testAdapter?.usageList) {
    return testAdapter.usageList(limit);
  }
  if (!isTauriDesktopHost()) {
    return { items: [] };
  }
  try {
    return await invokeNative<EOStageUsageListResult>("eostage_usage_list", { limit });
  } catch (err) {
    throw new Error(extractMessage(err, "获取用量明细失败"));
  }
}
