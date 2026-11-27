import React, { useCallback, useEffect, useRef, useState } from "react";
import { Check, ChevronDown, ChevronRight, ExternalLink, Loader2, RefreshCw, X } from "lucide-react";
import { ModalOverlay } from "@/components/ui/ModalOverlay";
import { FieldValidationPopover } from "@/components/ui/FieldValidationPopover";
import { openExternalUrl } from "@/lib/bridge";
import {
  eostageAccountStatus,
  eostageBillingConfig,
  eostageBillingOrderCancel,
  eostageBillingOrderCheck,
  eostageBillingOrderCreate,
  eostageBillingOrdersList,
  eostageBillingRedeem,
  eostageUsageList,
  notifyEOStageAccountChanged,
  type EOStageAccountStatus,
  type EOStageBillingConfig,
  type EOStageOrder,
  type EOStageUsageItem,
} from "@/shell/api/eostageAccount";
import { cn } from "@/lib/utils";
import { EmptyState } from "@/components/ui/EmptyState";

export type BillingModalTab = "balance" | "plans" | "orders" | "usage";

export interface EOStageBillingModalProps {
  open: boolean;
  initialTab?: BillingModalTab;
  account: EOStageAccountStatus | null;
  onClose: () => void;
  onOpenOfficialModelSettings?: () => void;
}

function formatUnixDateTime(ts: number | null | undefined): string {
  if (!ts || !Number.isFinite(ts)) return "--";
  const d = new Date(ts * 1000);
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  return `${m}/${day} ${hh}:${mm}`;
}

function formatDateOnly(ts: number | null | undefined): string {
  if (!ts || !Number.isFinite(ts)) return "";
  const d = new Date(ts * 1000);
  const yyyy = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${yyyy}-${m}-${day}`;
}

function formatCreditsAmount(val: number): string {
  if (!Number.isFinite(val)) return "0";
  if (Math.abs(val - Math.round(val)) < 1e-6) {
    return Math.round(val).toLocaleString();
  }
  return Number(val.toFixed(4)).toLocaleString(undefined, { maximumFractionDigits: 4 });
}

function formatTrialValidityText(account: EOStageAccountStatus | null, trialDays = 60): string {
  if (account?.trial_granted === false) {
    return "本机已领取过新用户免费体验 Credits";
  }
  if (!account?.trial_expires_at) {
    return `新用户免费领取（有效期 ${trialDays} 天），优先消耗`;
  }
  const diffSecs = account.trial_expires_at - Math.floor(Date.now() / 1000);
  const dateStr = formatDateOnly(account.trial_expires_at);
  if (diffSecs <= 0) {
    return `体验 Credits 已于 ${dateStr} 到期`;
  }
  const remainingDays = Math.max(1, Math.ceil(diffSecs / 86400));
  return `有效期 ${trialDays} 天 · 剩余 ${remainingDays} 天（${dateStr} 到期），优先消耗`;
}

function formatPriceYuan(amountFen: number): string {
  return `¥${(amountFen / 100).toFixed(2)}`;
}

export function EOStageBillingModal({
  open,
  initialTab = "balance",
  account: initialAccount,
  onClose,
  onOpenOfficialModelSettings,
}: EOStageBillingModalProps) {
  const [activeTab, setActiveTab] = useState<BillingModalTab>(initialTab);
  const [account, setAccount] = useState<EOStageAccountStatus | null>(initialAccount);
  const [config, setConfig] = useState<EOStageBillingConfig | null>(null);
  const [orders, setOrders] = useState<EOStageOrder[]>([]);
  const [usageItems, setUsageItems] = useState<EOStageUsageItem[]>([]);
  const [usageError, setUsageError] = useState<string | null>(null);
  const [expandedUsageIds, setExpandedUsageIds] = useState<Record<string, boolean>>({});
  const [loading, setLoading] = useState(false);

  const [selectedPlanId, setSelectedPlanId] = useState<string | null>(null);
  const [creatingOrder, setCreatingOrder] = useState(false);
  const [orderFeedback, setOrderFeedback] = useState<string | null>(null);

  const [redeemInput, setRedeemInput] = useState("");
  const [redeeming, setRedeeming] = useState(false);
  const [redeemError, setRedeemError] = useState<string | null>(null);
  const [redeemSuccess, setRedeemSuccess] = useState<string | null>(null);
  const redeemInputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    setAccount(initialAccount);
  }, [initialAccount]);

  const initialLoggedIn = Boolean(initialAccount?.logged_in);

  const loadAll = useCallback(async () => {
    setLoading(true);
    setUsageError(null);
    try {
      const acctPromise = eostageAccountStatus(true);
      const cfgPromise = eostageBillingConfig();
      const ordersPromise = initialLoggedIn ? eostageBillingOrdersList() : null;
      const usagePromise = initialLoggedIn ? eostageUsageList(30) : null;

      const [acctRes, cfgRes] = await Promise.all([acctPromise, cfgPromise]);
      setAccount(acctRes);
      setConfig(cfgRes);
      if (acctRes.logged_in) {
        const [ordersRes, usageRes] = await Promise.allSettled([
          ordersPromise ?? eostageBillingOrdersList(),
          usagePromise ?? eostageUsageList(30),
        ]);
        if (ordersRes.status === "fulfilled") {
          setOrders(ordersRes.value);
        }
        if (usageRes.status === "fulfilled") {
          setUsageItems(usageRes.value.items);
        } else {
          setUsageError(usageRes.reason instanceof Error ? usageRes.reason.message : "用量读取失败");
        }
      }
      const firstEnabled = cfgRes.plans.find((p) => p.enabled !== false);
      setSelectedPlanId((prev) => prev || firstEnabled?.id || null);
    } catch {
      // Keep cached values cleanly on transient error
    } finally {
      setLoading(false);
    }
  }, [initialLoggedIn]);

  useEffect(() => {
    if (!open) return;
    setActiveTab(initialTab);
    setRedeemError(null);
    setRedeemSuccess(null);
    setOrderFeedback(null);
    void loadAll();
  }, [open, initialTab, loadAll]);

  useEffect(() => {
    if (!open) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, onClose]);

  if (!open) return null;

  const enabledPlans = (config?.plans ?? []).filter((p) => p.enabled !== false);
  const showPlansTab = Boolean(config?.plans_enabled && enabledPlans.length > 0);
  const showOrdersTab = Boolean(config?.orders_enabled || config?.plans_enabled || orders.length > 0);

  const tabs: { id: BillingModalTab; label: string }[] = [
    { id: "balance", label: "余额" },
    ...(showPlansTab ? [{ id: "plans" as const, label: "充值与套餐" }] : []),
    ...(showOrdersTab
      ? [{ id: "orders" as const, label: orders.length > 0 ? `订单记录 · ${orders.length}` : "订单记录" }]
      : []),
    { id: "usage", label: "AI 用量" },
  ];

  const resolvedTab: BillingModalTab = tabs.some((t) => t.id === activeTab) ? activeTab : "balance";

  const handleRedeem = async () => {
    const clean = redeemInput.trim();
    if (!clean) {
      setRedeemError("请输入 Credits 兑换码");
      redeemInputRef.current?.focus();
      return;
    }
    setRedeeming(true);
    setRedeemError(null);
    setRedeemSuccess(null);
    try {
      const res = await eostageBillingRedeem(clean);
      setAccount(res.account);
      setRedeemInput("");
      setRedeemSuccess(`已成功兑换 ${res.added_credits.toLocaleString()} Credits`);
      notifyEOStageAccountChanged();
    } catch (err) {
      setRedeemError(err instanceof Error ? err.message : "兑换码无效或已使用");
      redeemInputRef.current?.focus();
    } finally {
      setRedeeming(false);
    }
  };

  const selectedPlan = enabledPlans.find((p) => p.id === selectedPlanId) ?? enabledPlans[0] ?? null;

  const handleCheckout = async () => {
    if (!selectedPlan) return;
    setCreatingOrder(true);
    setOrderFeedback(null);
    try {
      const created = await eostageBillingOrderCreate(selectedPlan.id);
      setOrders((prev) => [created, ...prev.filter((o) => o.order_no !== created.order_no)]);
      if (created.pay_url) {
        await openExternalUrl(created.pay_url);
      }
      setActiveTab("orders");
    } catch (err) {
      setOrderFeedback(err instanceof Error ? err.message : "创建订单失败");
    } finally {
      setCreatingOrder(false);
    }
  };

  const handleOrderCheck = async (orderNo: string) => {
    try {
      const updated = await eostageBillingOrderCheck(orderNo);
      setOrders((prev) => prev.map((o) => (o.order_no === orderNo ? updated : o)));
      if (updated.status === "paid") {
        void loadAll();
        notifyEOStageAccountChanged();
      }
    } catch (err) {
      setOrderFeedback(err instanceof Error ? err.message : "查询订单失败");
    }
  };

  const handleOrderCancel = async (orderNo: string) => {
    try {
      const updated = await eostageBillingOrderCancel(orderNo);
      setOrders((prev) => prev.map((o) => (o.order_no === orderNo ? updated : o)));
    } catch (err) {
      setOrderFeedback(err instanceof Error ? err.message : "取消订单失败");
    }
  };

  return (
    <ModalOverlay
      role="dialog"
      aria-modal={true}
      aria-label="余额与账单"
      data-testid="eostage-billing-modal"
      closeOnBackdropClick={true}
      onClose={onClose}
    >
      <div
        className="flex h-[min(600px,84vh)] w-full max-w-[min(680px,92vw)] flex-col rounded-modal border border-sp-line bg-sp-card shadow-modal overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-start justify-between gap-3 px-6 pt-5 pb-3">
          <div className="space-y-1">
            <h2 className="text-base font-semibold text-sp-text">余额与账单</h2>
            {account?.email && (
              <p className="text-xs text-sp-text-secondary">{account.email}</p>
            )}
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="关闭余额与账单"
            className="flex h-7 w-7 shrink-0 items-center justify-center rounded-sp-sm text-[13px] text-sp-text-muted hover:bg-sp-hover hover:text-sp-text cursor-pointer"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="flex items-center justify-between gap-2 border-b border-sp-divider px-6 pb-3">
          <div className="flex flex-wrap items-center gap-1">
            {tabs.map((tab) => {
              const active = resolvedTab === tab.id;
              return (
                <button
                  key={tab.id}
                  type="button"
                  data-testid={`billing-tab-${tab.id}`}
                  onClick={() => setActiveTab(tab.id)}
                  className={cn(
                    "h-8 rounded-sp-sm px-3 text-[13px] transition-colors cursor-pointer",
                    active
                      ? "bg-sp-surface-muted font-semibold text-sp-text"
                      : "text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text"
                  )}
                >
                  {tab.label}
                </button>
              );
            })}
          </div>
          <button
            type="button"
            data-testid="billing-refresh-btn"
            aria-label="刷新账单状态"
            disabled={loading}
            onClick={() => void loadAll()}
            className="flex h-7 w-7 shrink-0 items-center justify-center rounded-sp-sm text-[13px] text-sp-text-muted hover:bg-sp-hover hover:text-sp-text cursor-pointer disabled:opacity-50"
          >
            <RefreshCw className={cn("h-3.5 w-3.5", loading && "animate-spin")} />
          </button>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto px-6 py-5 space-y-4">
          {resolvedTab === "balance" && (
            <div data-testid="billing-balance-view" className="space-y-4">
              <div className="grid grid-cols-2 gap-4 rounded-sp-md border border-sp-line bg-sp-surface p-4">
                <div className="space-y-1">
                  <div className="text-xs text-sp-text-muted">可用 Credits</div>
                  <div
                    data-testid="billing-total-credits"
                    className="text-2xl font-semibold tabular-nums text-sp-text"
                  >
                    {(account?.total_credits ?? 0).toLocaleString()}
                  </div>
                </div>
                <div className="space-y-1">
                  <div className="text-xs text-sp-text-muted">已使用</div>
                  <div className="text-2xl font-semibold tabular-nums text-sp-text-secondary">
                    {(account?.used_credits ?? 0).toLocaleString()}
                  </div>
                </div>
              </div>

              <div className="rounded-sp-md border border-sp-line bg-sp-surface divide-y divide-sp-divider">
                <div className="flex items-center justify-between gap-2 px-4 py-3">
                  <div className="space-y-1">
                    <div className="text-[13px] font-semibold text-sp-text">体验 Credits</div>
                    <div data-testid="billing-trial-validity" className="text-xs text-sp-text-muted">
                      {formatTrialValidityText(account, config?.trial_expire_days ?? 60)}
                    </div>
                  </div>
                  <div className="text-[13px] font-semibold tabular-nums text-sp-text">
                    {(account?.trial_credits ?? 0).toLocaleString()} Credits
                  </div>
                </div>
                <div className="flex items-center justify-between gap-2 px-4 py-3">
                  <div className="space-y-1">
                    <div className="text-[13px] font-semibold text-sp-text">账户永久 Credits</div>
                    <div className="text-xs text-sp-text-muted">兑换或充值所得 Credits 永不过期</div>
                  </div>
                  <div className="text-[13px] font-semibold tabular-nums text-sp-text">
                    {(account?.account_credits ?? 0).toLocaleString()} Credits
                  </div>
                </div>
              </div>

              <p className="text-xs text-sp-text-muted leading-relaxed">
                1,000 Credits = ¥1，仅在使用官方托管模型时按实际消耗扣除；本机数据检索下载、GIS 处理及自定义 API Key 渠道均免费。
              </p>

              {config?.announcement && (
                <div className="rounded-sp-sm border border-sp-line bg-sp-surface-muted px-3 py-2 text-xs text-sp-text-secondary leading-relaxed">
                  {config.announcement}
                </div>
              )}

              {config?.redeem_enabled !== false && (
                <div className="rounded-sp-md border border-sp-line bg-sp-surface p-4 space-y-2">
                  <label htmlFor="eostage-redeem-input" className="block text-[13px] font-semibold text-sp-text">
                    使用兑换码
                  </label>
                  <div className="flex items-center gap-2">
                    <input
                      ref={redeemInputRef}
                      id="eostage-redeem-input"
                      data-testid="billing-redeem-input"
                      type="text"
                      value={redeemInput}
                      disabled={redeeming}
                      aria-invalid={redeemError ? "true" : "false"}
                      aria-describedby={redeemError ? "eostage-billing-redeem-error" : undefined}
                      placeholder="输入 EOS-... 兑换码"
                      onChange={(e) => {
                        setRedeemInput(e.target.value);
                        if (redeemError) setRedeemError(null);
                      }}
                      onKeyDown={(e) => {
                        if (
                          e.nativeEvent.isComposing ||
                          (e.nativeEvent as unknown as { keyCode?: number }).keyCode === 229
                        ) {
                          return;
                        }
                        if (e.key === "Enter") {
                          e.preventDefault();
                          void handleRedeem();
                        }
                      }}
                      className={cn(
                        "eo-settings-control-input h-9 flex-1 font-mono",
                        redeemError && "border-sp-text"
                      )}
                    />
                    <button
                      type="button"
                      data-testid="billing-redeem-submit-btn"
                      disabled={redeeming}
                      onClick={() => void handleRedeem()}
                      className="eo-settings-btn-secondary shrink-0"
                    >
                      {redeeming ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : null}
                      <span>兑换 Credits</span>
                    </button>
                  </div>
                  <FieldValidationPopover
                    id="eostage-billing-redeem-error"
                    testId="billing-redeem-error"
                    message={redeemError}
                    anchorRef={redeemInputRef}
                    onDismiss={() => setRedeemError(null)}
                  />
                  {redeemSuccess && (
                    <p className="text-xs text-sp-text" role="status">
                      {redeemSuccess}
                    </p>
                  )}
                </div>
              )}

              {!config?.plans_enabled && config?.checkout_notice && (
                <p className="text-xs text-sp-text-muted">{config.checkout_notice}</p>
              )}

              {onOpenOfficialModelSettings && (
                <div className="flex items-center justify-between rounded-sp-md border border-sp-line bg-sp-surface px-4 py-3">
                  <div className="space-y-1">
                    <div className="text-[13px] font-semibold text-sp-text">也可以使用自己的模型渠道</div>
                    <div className="text-xs text-sp-text-muted">自有渠道不扣 EOStage Credits</div>
                  </div>
                  <button
                    type="button"
                    onClick={() => {
                      onClose();
                      onOpenOfficialModelSettings();
                    }}
                    className="eo-settings-btn-secondary shrink-0"
                  >
                    <span>打开模型设置</span>
                  </button>
                </div>
              )}
            </div>
          )}

          {resolvedTab === "plans" && showPlansTab && (
            <div data-testid="billing-plans-view" className="space-y-4">
              <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
                {enabledPlans.map((plan) => {
                  const selected = selectedPlan?.id === plan.id;
                  return (
                    <button
                      key={plan.id}
                      type="button"
                      data-testid={`billing-plan-card-${plan.id}`}
                      onClick={() => setSelectedPlanId(plan.id)}
                      className={cn(
                        "flex flex-col items-start gap-2 rounded-sp-md border p-4 text-left text-[13px] transition-colors cursor-pointer",
                        selected
                          ? "border-sp-text bg-sp-surface-muted"
                          : "border-sp-line bg-sp-surface hover:bg-sp-hover"
                      )}
                    >
                      <div className="flex w-full items-center justify-between gap-2">
                        <span className="font-semibold text-sp-text">{plan.title}</span>
                        {plan.badge && <span className="eo-settings-badge">{plan.badge}</span>}
                      </div>
                      <div className="text-xl font-semibold tabular-nums text-sp-text">
                        {config?.currency_symbol ?? "¥"}
                        {plan.price_cny}
                      </div>
                      <div className="text-xs tabular-nums text-sp-text-secondary">
                        {plan.credits.toLocaleString()} Credits
                      </div>
                      {plan.description && (
                        <div className="text-xs text-sp-text-muted leading-relaxed">{plan.description}</div>
                      )}
                    </button>
                  );
                })}
              </div>

              {config?.checkout_notice && (
                <p className="text-xs text-sp-text-muted leading-relaxed">{config.checkout_notice}</p>
              )}

              {orderFeedback && (
                <p className="text-xs text-sp-danger-text" role="alert">
                  {orderFeedback}
                </p>
              )}

              {selectedPlan && (
                <div className="flex items-center justify-between border-t border-sp-divider pt-3">
                  <div className="text-[13px] text-sp-text">
                    本次支付{" "}
                    <span className="font-semibold tabular-nums">
                      {config?.currency_symbol ?? "¥"}
                      {selectedPlan.price_cny}
                    </span>
                  </div>
                  <button
                    type="button"
                    data-testid="billing-checkout-btn"
                    disabled={creatingOrder}
                    onClick={() => void handleCheckout()}
                    className="eo-settings-btn-primary"
                  >
                    {creatingOrder ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    ) : (
                      <ExternalLink className="h-3.5 w-3.5" />
                    )}
                    <span>立即创建订单并支付</span>
                  </button>
                </div>
              )}
            </div>
          )}

          {resolvedTab === "orders" && showOrdersTab && (
            <div data-testid="billing-orders-view" className="space-y-3">
              {orders.length === 0 ? (
                <EmptyState
                  kind="first-use"
                  size="row"
                  title="暂无订单记录"
                  className="rounded-sp-md border border-dashed border-sp-line bg-sp-surface px-4 py-4"
                />
              ) : (
                <div className="divide-y divide-sp-divider rounded-sp-md border border-sp-line bg-sp-surface">
                  {orders.map((order) => {
                    const statusLabel =
                      order.status === "paid"
                        ? "已到账"
                        : order.status === "cancelled"
                          ? "已取消"
                          : "待支付";
                    return (
                      <div key={order.order_no} className="p-4 space-y-2">
                        <div className="flex items-start justify-between gap-2">
                          <div className="space-y-1">
                            <div className="text-[13px] font-semibold text-sp-text">
                              {order.title} · {order.credits.toLocaleString()} Credits
                            </div>
                            <div className="text-xs text-sp-text-muted">
                              {formatUnixDateTime(order.created_at)}
                            </div>
                            <div className="font-mono text-xs text-sp-text-muted">{order.order_no}</div>
                          </div>
                          <div className="text-right space-y-1">
                            <div className="text-xs text-sp-text-secondary">{statusLabel}</div>
                            <div className="text-[13px] font-semibold tabular-nums text-sp-text">
                              {formatPriceYuan(order.amount_fen)}
                            </div>
                          </div>
                        </div>
                        {order.status === "pending" && (
                          <div className="flex items-center gap-2 pt-1">
                            {order.pay_url && (
                              <button
                                type="button"
                                onClick={() => void openExternalUrl(order.pay_url)}
                                className="eo-settings-btn-secondary"
                              >
                                <ExternalLink className="h-3.5 w-3.5" />
                                <span>继续支付</span>
                              </button>
                            )}
                            <button
                              type="button"
                              onClick={() => void handleOrderCheck(order.order_no)}
                              className="eo-settings-btn-secondary"
                            >
                              <span>查询付款</span>
                            </button>
                            <button
                              type="button"
                              onClick={() => void handleOrderCancel(order.order_no)}
                              className="text-[13px] text-sp-text-muted hover:text-sp-text px-2 cursor-pointer"
                            >
                              取消订单
                            </button>
                          </div>
                        )}
                      </div>
                    );
                  })}
                </div>
              )}
            </div>
          )}

          {resolvedTab === "usage" && (
            <div data-testid="billing-usage-view" className="space-y-3">
              {usageError && (
                <p className="text-xs text-sp-danger-text" role="alert">
                  {usageError}
                </p>
              )}
              {usageItems.length === 0 ? (
                <EmptyState
                  kind="first-use"
                  size="row"
                  title={loading ? "正在加载官方模型用量记录…" : "暂无官方模型扣费记录"}
                  className="rounded-sp-md border border-dashed border-sp-line bg-sp-surface px-4 py-4"
                />
              ) : (
                <div className="divide-y divide-sp-divider rounded-sp-md border border-sp-line bg-sp-surface">
                  {usageItems.map((item) => {
                    const isOpen = Boolean(expandedUsageIds[item.id]);
                    return (
                      <div key={item.id} className="p-4 space-y-2">
                        <div className="flex items-start justify-between gap-2">
                          <div className="space-y-1 min-w-0">
                            <div className="text-[13px] font-semibold text-sp-text truncate">{item.model}</div>
                            <div className="text-xs text-sp-text-muted">
                              {formatUnixDateTime(item.created_at)}
                            </div>
                          </div>
                          <div className="text-right shrink-0 space-y-1">
                            <div className="text-[13px] font-semibold tabular-nums text-sp-text">
                              {formatCreditsAmount(item.credits)} Credits
                            </div>
                            <div className="text-xs text-sp-text-muted">已结算</div>
                          </div>
                        </div>

                        <button
                          type="button"
                          data-testid={`usage-toggle-${item.id}`}
                          onClick={() =>
                            setExpandedUsageIds((prev) => ({ ...prev, [item.id]: !isOpen }))
                          }
                          className="flex items-center gap-1 text-[13px] text-sp-text-secondary hover:text-sp-text cursor-pointer"
                        >
                          {isOpen ? <ChevronDown className="h-3.5 w-3.5" /> : <ChevronRight className="h-3.5 w-3.5" />}
                          <span>计费明细</span>
                        </button>

                        {isOpen && (
                          <div
                            data-testid={`usage-detail-${item.id}`}
                            className="space-y-2 border-t border-sp-divider pt-2 text-xs text-sp-text-secondary"
                          >
                            <div className="grid grid-cols-3 gap-2">
                              <div>输入 token</div>
                              <div className="tabular-nums text-sp-text">{item.prompt_tokens.toLocaleString()}</div>
                              <div className="text-right tabular-nums">
                                其中缓存 {item.cached_tokens.toLocaleString()}
                              </div>
                            </div>
                            <div className="grid grid-cols-3 gap-2">
                              <div>输出 token</div>
                              <div className="tabular-nums text-sp-text">{item.completion_tokens.toLocaleString()}</div>
                              <div className="text-right tabular-nums">
                                {item.use_time_ms > 0 ? `耗时 ${(item.use_time_ms / 1000).toFixed(1)}s` : ""}
                              </div>
                            </div>
                            {item.uncached_input_credits_per_1m !== null && (
                              <div className="text-xs text-sp-text-muted">
                                每百万 token：未缓存输入 {item.uncached_input_credits_per_1m.toLocaleString()} Credits
                                · 缓存输入 {(item.cached_input_credits_per_1m ?? item.uncached_input_credits_per_1m).toLocaleString()} Credits
                                · 输出 {(item.output_credits_per_1m ?? 0).toLocaleString()} Credits
                              </div>
                            )}
                            {item.request_id && (
                              <div className="font-mono text-xs text-sp-text-muted truncate" title={item.request_id}>
                                {item.request_id}
                              </div>
                            )}
                          </div>
                        )}
                      </div>
                    );
                  })}
                </div>
              )}
            </div>
          )}
        </div>

        <div className="flex items-center justify-end border-t border-sp-divider px-6 py-3">
          <button
            type="button"
            onClick={onClose}
            className="eo-settings-btn-secondary"
          >
            <Check className="h-3.5 w-3.5" />
            <span>完成</span>
          </button>
        </div>
      </div>
    </ModalOverlay>
  );
}
