import React, { useState, useEffect, useCallback } from "react";
import {
  ArrowLeft,
  Sun,
  Moon,
  BarChart3,
  RefreshCw,
} from "lucide-react";
import {
  SETTINGS_GROUPS,
  LAST_SETTINGS_CATEGORY_STORAGE_KEY,
  getSettingsCategoryLabel,
  normalizeSettingsCategory,
  readLastSettingsCategory,
  rememberSettingsCategory,
} from "@/shell/api/settingsEntries";
import { cn } from "@/lib/utils";
import { LOCALES, useI18n, type Locale } from "@/i18n";
import { AccountsSettingsTab } from "@/shell/components/settings/AccountsSettingsTab";
import { NetworkProxySettingsTab } from "@/shell/components/settings/NetworkProxySettingsTab";
import { ModelChannelsSettingsTab } from "@/shell/components/settings/ModelChannelsSettingsTab";
import { PerformanceUsageSettingsTab } from "@/shell/components/settings/PerformanceUsageSettingsTab";
import { WorkStepsSettingsTab } from "@/shell/components/settings/WorkStepsSettingsTab";
import { DiagnosticsSettingsSection } from "@/shell/components/settings/DiagnosticsSettingsSection";
import {
  getBusySendPrefs,
  setBusySendPrefs,
  subscribeBusySendPrefs,
  type BusySendMode,
} from "@/lib/performanceAndUsagePrefs";
import {
  RuntimeToolsSettingsSection,
} from "@/components/desktop/PluginsAndSkillsPage";

export {
  LAST_SETTINGS_CATEGORY_STORAGE_KEY,
  readLastSettingsCategory,
  rememberSettingsCategory,
};
import { markSettingsModalOpen } from "@/shell/api/settingsModalLayer";
import {
  getApplicationUsageStatistics,
  type ApplicationUsageStatisticsResponse,
} from "@/lib/bridge";
import {
  UsageAnalyticsDashboard,
  type UsageAnalyticsDashboardStats,
} from "@/components/settings/UsageAnalyticsDashboard";
import { TransferEngineSettingsSection } from "@/components/settings/TransferEngineSettingsSection";
import { emitTopToast } from "@/components/ui/TopCenterToast";
import { UsageStatsSkeleton } from "@/components/ui/Skeleton";
import { EmptyState } from "@/components/ui/EmptyState";
import { EmptyStateIcon } from "@/components/ui/EmptyStateVisuals";
import { useDelayedFlag, useFullTurnSpin } from "@/hooks/useLoadingPrimitives";


export const MIN_SHELL_FONT_SIZE = 12;
export const MAX_SHELL_FONT_SIZE = 22;
export const DEFAULT_SHELL_FONT_SIZE = 14;
export const SHELL_FONT_SIZE_STORAGE_KEY = "eostage.shell.fontSize";

export const SHELL_FONT_SIZES = [
  { px: 13, labelKey: "settings.dialog.fontSmall" },
  { px: 14, labelKey: "settings.dialog.fontStandard" },
  { px: 15, labelKey: "settings.dialog.fontLarge" },
  { px: 16, labelKey: "settings.dialog.fontExtraLarge" },
] as const;

export function applyGlobalFontSize(px: number, persist = false): number {
  const numeric = Number.isFinite(px) ? Math.round(px) : DEFAULT_SHELL_FONT_SIZE;
  const clamped = Math.min(MAX_SHELL_FONT_SIZE, Math.max(MIN_SHELL_FONT_SIZE, numeric));
  const scale = Number((clamped / DEFAULT_SHELL_FONT_SIZE).toFixed(4));
  const rootRemPx = Number((scale * 16).toFixed(2));

  if (typeof document !== "undefined" && document.documentElement) {
    const root = document.documentElement;
    root.style.setProperty("--sp-font-size", `${clamped}px`);
    root.style.setProperty("--sp-font-scale", String(scale));
    root.style.fontSize = `${rootRemPx}px`;
    root.dataset.shellFontSize = String(clamped);

    document.querySelectorAll<HTMLElement>(".sp-modern").forEach((el) => {
      el.style.setProperty("--sp-font-size", `${clamped}px`);
      el.style.setProperty("--sp-font-scale", String(scale));
    });
  }

  if (persist && typeof window !== "undefined") {
    try {
      window.localStorage?.setItem(SHELL_FONT_SIZE_STORAGE_KEY, String(clamped));
    } catch {
      // ignore storage quota errors
    }
  }

  return clamped;
}

export function restoreGlobalFontSizeFromStorage(): number {
  if (typeof window === "undefined") return DEFAULT_SHELL_FONT_SIZE;
  try {
    const raw = window.localStorage?.getItem(SHELL_FONT_SIZE_STORAGE_KEY);
    if (!raw) return DEFAULT_SHELL_FONT_SIZE;
    const parsed = Number.parseInt(raw.trim(), 10);
    if (!Number.isNaN(parsed) && parsed >= MIN_SHELL_FONT_SIZE && parsed <= MAX_SHELL_FONT_SIZE) {
      return applyGlobalFontSize(parsed, false);
    }
  } catch {
    // ignore storage access errors
  }
  return DEFAULT_SHELL_FONT_SIZE;
}

if (typeof window !== "undefined" && typeof document !== "undefined") {
  restoreGlobalFontSizeFromStorage();
}

export type SettingsCategory =
  | "appearance"
  | "auth"
  | "network_proxy"
  | "models"
  | "performance_usage"
  | "work_steps"
  | "storage"
  | "network"
  | "plugins_skills"
  | "runtime_tools"
  | "usage_stats";

export interface SettingsModalProps {
  open: boolean;
  onClose: () => void;
  variant?: "overlay" | "page";
  initialCategory?: SettingsCategory;
  category?: SettingsCategory;
  onCategoryChange?: (category: SettingsCategory) => void;
  hideNav?: boolean;
  // 外观设置（暗黑主题与外壳字号）
  dark?: boolean;
  onToggleDark?: () => void;
  fontSize?: number;
  onFontSizeChange?: (px: number) => void;
  appVersion?: string | null;

  // 简要状态信息
  cdseAuthenticated?: boolean;
  cdseSaved?: boolean;
  earthdataAuthenticated?: boolean;
  earthdataLabel?: string;
  storageRootPath?: string;

  // 真实功能子组件注入 (展开后真实可操作)
  cdsePanel?: React.ReactNode;
  earthdataPanel?: React.ReactNode;
  bhoonidhiPanel?: React.ReactNode;
  usgsPanel?: React.ReactNode;
  storagePanel?: React.ReactNode;
  concurrencyPanel?: React.ReactNode;
  onSyncDemConcurrency?: (limit: number) => void | Promise<boolean | void>;
  tourOpenAddProviderSignal?: number;

  onChangeStoragePath?: () => void;
}

export function SettingsModal({
  open,
  onClose,
  variant = "overlay",
  initialCategory,
  category,
  onCategoryChange,
  hideNav = false,
  dark = false,
  onToggleDark,
  fontSize = 14,
  onFontSizeChange,
  appVersion,
  cdsePanel,
  earthdataPanel,
  bhoonidhiPanel,
  usgsPanel,
  storagePanel,
  concurrencyPanel,
  onSyncDemConcurrency,
  tourOpenAddProviderSignal,
}: SettingsModalProps) {
  const { locale, setLocale, t } = useI18n();
  const [internalCategory, setInternalCategory] = useState<SettingsCategory>(() =>
    initialCategory ? normalizeSettingsCategory(initialCategory) : readLastSettingsCategory()
  );
  const rawCategory = category !== undefined ? category : internalCategory;
  const activeCategory: SettingsCategory = normalizeSettingsCategory(rawCategory);

  const [fontSizeInput, setFontSizeInput] = useState<string>(() => String(fontSize));

  useEffect(() => {
    setFontSizeInput(String(fontSize));
    applyGlobalFontSize(fontSize, false);
  }, [fontSize]);

  const commitFontSize = (raw: string) => {
    const trimmed = raw.trim();
    const val = Number.parseInt(trimmed, 10);
    if (!Number.isNaN(val) && String(val) === trimmed && val >= MIN_SHELL_FONT_SIZE && val <= MAX_SHELL_FONT_SIZE) {
      setFontSizeInput(String(val));
      applyGlobalFontSize(val, true);
      if (val !== fontSize) {
        onFontSizeChange?.(val);
      }
    } else {
      setFontSizeInput(String(fontSize));
    }
  };

  const parsedFontSizeInput = Number.parseInt(fontSizeInput.trim(), 10);
  const currentResolvedFontSize =
    !Number.isNaN(parsedFontSizeInput) &&
    parsedFontSizeInput >= MIN_SHELL_FONT_SIZE &&
    parsedFontSizeInput <= MAX_SHELL_FONT_SIZE
      ? parsedFontSizeInput
      : fontSize;

  const handleSelectCategory = (cat: SettingsCategory) => {
    const normalized = normalizeSettingsCategory(cat);
    rememberSettingsCategory(normalized);
    onCategoryChange?.(normalized);
    if (category === undefined) {
      setInternalCategory(normalized);
    }
  };

  useEffect(() => {
    if (initialCategory) {
      setInternalCategory(normalizeSettingsCategory(initialCategory));
    }
  }, [initialCategory]);

  const [usageStats, setUsageStats] =
    useState<ApplicationUsageStatisticsResponse | null>(null);
  const [usageStatsLoading, setUsageStatsLoading] = useState(false);
  const [usageStatsError, setUsageStatsError] = useState(false);
  const { spinClass: usageRefreshSpinClass } = useFullTurnSpin(usageStatsLoading);
  const showUsageStatsSkeleton = useDelayedFlag(usageStatsLoading && !usageStats);

  const loadUsageStats = useCallback(async (manual = false) => {
    setUsageStatsLoading(true);
    setUsageStatsError(false);
    try {
      const res = await getApplicationUsageStatistics();
      if (res) {
        setUsageStats(res);
        if (manual) {
          emitTopToast({ text: t("settings.usageStats.toastUpdated"), tone: "success" });
        }
      }
    } catch {
      setUsageStatsError(true);
      if (manual) {
        emitTopToast({ text: t("settings.usageStats.toastRefreshFailed"), tone: "danger" });
      }
    } finally {
      setUsageStatsLoading(false);
    }
  }, [t]);

  useEffect(() => {
    if (open && activeCategory === "usage_stats") {
      void loadUsageStats();
    }
  }, [open, activeCategory, loadUsageStats]);

  const formatUsageBytes = (bytes: number) => {
    if (!bytes || bytes <= 0) return "0 B";
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1048576) return `${(bytes / 1024).toFixed(1)} KB`;
    if (bytes < 1073741824) return `${(bytes / 1048576).toFixed(2)} MB`;
    return `${(bytes / 1073741824).toFixed(2)} GB`;
  };

  // 冒泡阶段监听：嵌套的确认/子对话框先消费 Esc 时会 preventDefault；输入法组字中的 Esc 属于输入法。
  // 打开期间登记层级，地图浮层与下载抽屉的 Esc 处理据此让位。
  useEffect(() => {
    if (!open) return;
    const releaseLayer = markSettingsModalOpen();
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented || e.isComposing) return;
      // 标记已消费，后注册的下层 Esc 处理（如下载抽屉）据此跳过
      e.preventDefault();
      onClose();
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
      releaseLayer();
    };
  }, [open, onClose]);

  if (!open) return null;

  const isPage = variant === "page";
  const isWideCategory =
    activeCategory === "models" ||
    activeCategory === "runtime_tools" ||
    activeCategory === "usage_stats";

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={t("settings.dialog.label")}
      className={
        isPage
          ? "flex h-full w-full min-w-0 bg-sp-surface select-none"
          : "fixed inset-x-0 bottom-0 top-10 z-[2500] flex bg-sp-canvas select-none"
      }
    >
      {/* 左侧导航栏（220px 侧栏 + 30px 紧凑导航行） */}
      {!hideNav && (
        <aside
          className={
            isPage
              ? "flex h-full w-56 shrink-0 flex-col border-r border-sp-line bg-sp-canvas px-3 py-3 overflow-y-auto"
              : "flex h-full w-60 shrink-0 flex-col border-r border-sp-line bg-sp-canvas p-4 overflow-y-auto"
          }
        >
          {/* 返回工作区按钮 */}
          {!isPage && (
            <button
              type="button"
              onClick={onClose}
              aria-label={t("shell.nav.backToWorkspace")}
              className="mb-3 flex h-[30px] w-full items-center gap-2 rounded-sp-sm px-3 text-[13px] font-normal text-sp-text hover:bg-sp-hover transition-colors duration-fast ease-sp whitespace-nowrap cursor-pointer select-none"
            >
              <ArrowLeft className="h-3.5 w-3.5 shrink-0" />
              <span className="truncate">{t("shell.nav.backToWorkspace")}</span>
            </button>
          )}

          <nav className="flex flex-col gap-3" data-testid="settings-grouped-nav">
            {SETTINGS_GROUPS.map((group) => (
              <div key={group.key} data-settings-group={group.key} className="flex flex-col gap-1">
                <div className="px-3 py-1 text-xs font-semibold text-sp-text-muted whitespace-nowrap select-none">
                  {t(group.labelKey)}
                </div>
                {group.items.map((cat) => {
                  const Icon = cat.icon;
                  const isActive = activeCategory === cat.key;
                  const catLabel = getSettingsCategoryLabel(cat);
                  return (
                    <button
                      key={cat.key}
                      type="button"
                      data-testid={`settings-nav-${cat.key}`}
                      aria-current={isActive ? "page" : undefined}
                      onClick={() => handleSelectCategory(cat.key)}
                      title={catLabel}
                      className={cn(
                        "flex h-[30px] w-full items-center gap-2 rounded-sp-sm px-3 text-[13px] transition-colors duration-fast ease-sp whitespace-nowrap cursor-pointer select-none",
                        isActive
                          ? "bg-sp-selected font-semibold text-sp-text"
                          : "text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text"
                      )}
                    >
                      <Icon className="h-3.5 w-3.5 shrink-0" />
                      <span className="truncate">{catLabel}</span>
                    </button>
                  );
                })}
              </div>
            ))}
          </nav>
        </aside>
      )}

      {/* 右侧主内容区 */}
      <main
        className={
          isPage
            ? "flex-1 overflow-y-scroll bg-sp-surface"
            : "flex-1 overflow-y-scroll bg-sp-surface"
        }
      >
        <div
          className={cn(
            "mx-auto w-full",
            isWideCategory ? "max-w-5xl px-6 py-6" : isPage ? "max-w-3xl px-8 py-6" : "max-w-3xl px-8 py-8"
          )}
        >
          {activeCategory === "appearance" && (
            <div className="eo-settings-layout" data-testid="appearance-merged-settings">
              {/* EOStage SettingsPageHeader */}
              <div className="eo-settings-page-header">
                <h2 className="text-[17px] font-semibold text-sp-text tracking-[-0.015em]">
                  {t("settings.dialog.appearance")}
                </h2>
                {appVersion && (
                  <span className="font-mono text-xs text-sp-text-muted">
                    {t("settings.dialog.version", { version: appVersion })}
                  </span>
                )}
              </div>

              {/* Section 1: 界面主题与排版 (SettingsSection -> SettingsCard -> SettingsCardContent -> SettingsCardRow -> SettingsCardBeard) */}
              <div className="eo-settings-section">
                <h3 className="text-[14px] font-semibold text-sp-text">{t("settings.dialog.appearanceSectionTitle")}</h3>

                <div className="eo-settings-card">
                  <div className="eo-settings-card-content">
                    {/* 主题行 */}
                    <div className="eo-settings-card-row">
                      <div className="flex flex-wrap items-center justify-between gap-4">
                        <div className="flex min-w-0 flex-col gap-1">
                          <h4 className="text-[13px] font-normal text-sp-text">{t("settings.dialog.theme")}</h4>
                          <div className="text-[12.5px] text-sp-text-secondary">{t("settings.dialog.themeDescription")}</div>
                        </div>
                        <div className="eo-segmented-track" role="group" aria-label={t("settings.dialog.themeSwitch")}>
                          <button
                            type="button"
                            onClick={() => {
                              if (dark && onToggleDark) onToggleDark();
                            }}
                            aria-pressed={!dark}
                            data-active={!dark}
                            className="eo-segmented-pill h-[26px] gap-1 px-3 cursor-pointer"
                          >
                            <Sun className="h-3.5 w-3.5" />
                            <span>{t("settings.dialog.light")}</span>
                          </button>
                          <button
                            type="button"
                            onClick={() => {
                              if (!dark && onToggleDark) onToggleDark();
                            }}
                            aria-pressed={dark}
                            data-active={dark}
                            className="eo-segmented-pill h-[26px] gap-1 px-3 cursor-pointer"
                          >
                            <Moon className="h-3.5 w-3.5" />
                            <span>{t("settings.dialog.dark")}</span>
                          </button>
                        </div>
                      </div>
                    </div>

                    {/* 语言行 */}
                    <div className="eo-settings-card-row">
                      <div className="flex flex-wrap items-center justify-between gap-4">
                        <div className="flex min-w-0 flex-col gap-1">
                          <h4 className="text-[13px] font-normal text-sp-text">{t("settings.language.label")}</h4>
                          <div className="text-[12.5px] text-sp-text-secondary">{t("settings.dialog.languageDescription")}</div>
                        </div>
                        <div className="eo-segmented-track" role="group" aria-label={t("settings.language.label")}>
                          {LOCALES.map((language: Locale) => (
                            <button
                              key={language}
                              type="button"
                              onClick={() => setLocale(language)}
                              aria-pressed={locale === language}
                              data-active={locale === language}
                              className="eo-segmented-pill h-[26px] justify-center whitespace-nowrap px-3 cursor-pointer"
                            >
                              {t(`common.locale.${language}`)}
                            </button>
                          ))}
                        </div>
                      </div>
                    </div>

                    {/* 字号行（支持 小/标准/大/特大 预设与步进输入框即时全局缩放） */}
                    <div className="eo-settings-card-row">
                      <div className="flex flex-wrap items-center justify-between gap-4">
                        <div className="flex min-w-0 flex-col gap-1">
                          <h4 className="text-[13px] font-normal text-sp-text" title={t("settings.dialog.fontSizeHint")}>
                            {t("settings.dialog.fontSize")}
                          </h4>
                          <div className="text-[12.5px] text-sp-text-secondary">
                            {t("settings.dialog.fontSizeDescription", { min: MIN_SHELL_FONT_SIZE, max: MAX_SHELL_FONT_SIZE })}
                          </div>
                        </div>
                        <div className="flex flex-wrap items-center gap-2">
                          <div
                            className="eo-segmented-track"
                            role="group"
                            aria-label={t("settings.dialog.fontSize")}
                            data-testid="font-size-preset-group"
                          >
                            {SHELL_FONT_SIZES.map((preset) => {
                              const isActivePreset = currentResolvedFontSize === preset.px;
                              return (
                                <button
                                  key={preset.px}
                                  type="button"
                                  data-testid={`font-size-preset-${preset.px}`}
                                  aria-pressed={isActivePreset}
                                  data-active={isActivePreset}
                                  onClick={() => commitFontSize(String(preset.px))}
                                  className="eo-segmented-pill h-[26px] justify-center whitespace-nowrap px-3 text-[13px] cursor-pointer"
                                >
                                  <span>{t(preset.labelKey)}</span>
                                </button>
                              );
                            })}
                          </div>
                          <div className="eo-settings-stepper">
                            <button
                              type="button"
                              disabled={currentResolvedFontSize <= MIN_SHELL_FONT_SIZE}
                              onClick={() => commitFontSize(String(Math.max(MIN_SHELL_FONT_SIZE, currentResolvedFontSize - 1)))}
                              aria-label={t("settings.dialog.decreaseFontSize")}
                              className="flex h-[24px] w-[26px] items-center justify-center rounded-sp-sm text-[13px] text-sp-text hover:bg-sp-hover disabled:opacity-40 cursor-pointer"
                            >
                              −
                            </button>
                            <input
                              type="number"
                              min={MIN_SHELL_FONT_SIZE}
                              max={MAX_SHELL_FONT_SIZE}
                              step={1}
                              value={fontSizeInput}
                              onChange={(e) => setFontSizeInput(e.target.value)}
                              onBlur={() => commitFontSize(fontSizeInput)}
                              onKeyDown={(e) => {
                                if (e.key === "Enter") {
                                  commitFontSize(fontSizeInput);
                                } else if (e.key === "Escape") {
                                  setFontSizeInput(String(fontSize));
                                }
                              }}
                              aria-label={t("settings.dialog.fontSize")}
                              className="min-w-0 flex-1 bg-transparent text-center font-mono text-[13px] text-sp-text tabular-nums outline-none [appearance:textfield]"
                            />
                            <button
                              type="button"
                              disabled={currentResolvedFontSize >= MAX_SHELL_FONT_SIZE}
                              onClick={() => commitFontSize(String(Math.min(MAX_SHELL_FONT_SIZE, currentResolvedFontSize + 1)))}
                              aria-label={t("settings.dialog.increaseFontSize")}
                              className="flex h-[24px] w-[26px] items-center justify-center rounded-sp-sm text-[13px] text-sp-text hover:bg-sp-hover disabled:opacity-40 cursor-pointer"
                            >
                              +
                            </button>
                          </div>
                        </div>
                      </div>
                    </div>
                  </div>
                </div>
              </div>

              {/* Section 2：工作步骤显示 */}
              <WorkStepsSettingsTab />

              {/* Section 3：性能与用量展示偏好 */}
              <PerformanceUsageSettingsTab />

              {/* Section 4：回复进行中发送新消息默认方式（排队等待 / 立即引导） */}
              <BusySendSettingsSection />

              {/* Section 5：异常诊断与日志上报（默认关闭，仅脱敏记录） */}
              <DiagnosticsSettingsSection />
            </div>
          )}

          {activeCategory === "auth" && (
            <AccountsSettingsTab
              cdsePanel={cdsePanel}
              earthdataPanel={earthdataPanel}
              bhoonidhiPanel={bhoonidhiPanel}
              usgsPanel={usgsPanel}
            />
          )}

          {activeCategory === "network_proxy" && (
            <NetworkProxySettingsTab />
          )}

          {activeCategory === "models" && (
            <ModelChannelsSettingsTab tourOpenAddProviderSignal={tourOpenAddProviderSignal} />
          )}

          {activeCategory === "runtime_tools" && (
            <div className="eo-settings-layout" data-testid="runtime-tools-settings-tab">
              <div className="eo-settings-page-header">
                <h2 className="text-[17px] font-semibold text-sp-text tracking-[-0.015em]">
                  {t("settings.categories.runtimeTools")}
                </h2>
              </div>
              <div className="rounded-card border border-sp-line bg-sp-card overflow-hidden p-5">
                <RuntimeToolsSettingsSection />
              </div>
            </div>
          )}

          {activeCategory === "usage_stats" && (
            <div className="eo-settings-layout" data-testid="usage-stats-settings-tab">
              <div className="eo-settings-page-header">
                <h2 className="text-[17px] font-semibold text-sp-text tracking-[-0.015em]">
                  {t("settings.categories.usageStats")}
                </h2>
                <button
                  type="button"
                  data-testid="refresh-usage-stats-btn"
                  onClick={() => void loadUsageStats(true)}
                  disabled={usageStatsLoading}
                  aria-busy={usageStatsLoading}
                  className="eo-settings-btn-secondary disabled:opacity-50"
                >
                  <RefreshCw
                    className={cn(
                      "h-3.5 w-3.5",
                      usageRefreshSpinClass,
                    )}
                    strokeWidth={1.75}
                  />
                  <span>{t("settings.usageStats.refresh")}</span>
                </button>
              </div>

              {/* 核心真实指标卡片：首次等待 <=300ms 预留布局不闪白，>300ms 用骨架屏，失败原地退回 */}
              {!usageStats && usageStatsLoading ? (
                showUsageStatsSkeleton ? (
                  <UsageStatsSkeleton />
                ) : (
                  <div className="min-h-[300px]" aria-hidden="true" />
                )
              ) : !usageStats && usageStatsError ? (
                <div className="rounded-card border border-sp-line bg-sp-card p-6 shadow-card">
                  <EmptyState
                    kind="failed"
                    size="page"
                    visual={<EmptyStateIcon name="alert" size={48} />}
                    title={t("settings.usageStats.loadFailedTitle")}
                    description={t("settings.usageStats.loadFailedDescription")}
                    primaryAction={{
                      label: t("settings.usageStats.retry"),
                      onClick: () => void loadUsageStats(true),
                      loading: usageStatsLoading,
                    }}
                  />
                </div>
              ) : (
                <>
              <div
                data-testid="usage-stats-summary-cards"
                className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-4"
              >
                <div
                  data-testid="usage-stat-sessions"
                  style={{ "--row-index": 0 } as React.CSSProperties}
                  className="sp-stagger-fade-row rounded-card border border-sp-line bg-sp-card p-4 shadow-card"
                >
                  <div className="text-xs text-sp-text-muted">{t("settings.usageStats.sessionsTitle")}</div>
                  <div className="mt-1 text-xl font-semibold font-mono text-sp-text">
                    {usageStats ? usageStats.session_count : 0}
                  </div>
                  <div className="mt-1 text-xs text-sp-text-secondary">
                    {t("settings.usageStats.sessionsMeta", {
                      workspaces: usageStats ? usageStats.workspace_count : 0,
                      plugins: usageStats ? usageStats.installed_plugins_count : 0,
                    })}
                  </div>
                </div>

                <div
                  data-testid="usage-stat-runs"
                  className="rounded-card border border-sp-line bg-sp-card p-4 shadow-card"
                >
                  <div className="text-xs text-sp-text-muted">{t("settings.usageStats.runsTitle")}</div>
                  <div className="mt-1 text-xl font-semibold font-mono text-sp-text">
                    {usageStats ? usageStats.run_count : 0}
                  </div>
                  <div className="mt-1 text-xs text-sp-text-secondary">
                    {t("settings.usageStats.runsMeta", {
                      tools: usageStats ? usageStats.tool_call_count : 0,
                    })}
                  </div>
                </div>

                <div
                  data-testid="usage-stat-tokens"
                  className="rounded-card border border-sp-line bg-sp-card p-4 shadow-card"
                >
                  <div className="text-xs text-sp-text-muted">{t("settings.usageStats.tokensTitle")}</div>
                  <div className="mt-1 text-xl font-semibold font-mono text-sp-text">
                    {(usageStats?.total_tokens ?? 0).toLocaleString()}
                  </div>
                  <div className="mt-1 text-xs text-sp-text-secondary">
                    {t("settings.usageStats.tokensMeta", {
                      prompt: (usageStats?.prompt_tokens ?? 0).toLocaleString(),
                      completion: (usageStats?.completion_tokens ?? 0).toLocaleString(),
                      reasoning: (usageStats?.reasoning_tokens ?? 0).toLocaleString(),
                    })}
                  </div>
                </div>

                <div
                  data-testid="usage-stat-downloads"
                  className="rounded-card border border-sp-line bg-sp-card p-4 shadow-card"
                >
                  <div className="text-xs text-sp-text-muted">{t("settings.usageStats.downloadsTitle")}</div>
                  <div className="mt-1 text-xl font-semibold font-mono text-sp-text">
                    {formatUsageBytes(usageStats?.download_transferred_bytes ?? 0)}
                  </div>
                  <div className="mt-1 text-xs text-sp-text-secondary">
                    {t("settings.usageStats.downloadsMeta", {
                      total: usageStats?.download_total_tasks ?? 0,
                      completed: usageStats?.download_completed_tasks ?? 0,
                      sqlite: formatUsageBytes(usageStats?.sqlite_size_bytes ?? 0),
                    })}
                  </div>
                </div>
              </div>

              {/* 可视化 SVG 趋势图、模型占比环形图、工具/数据源明细与 GEE 状态 */}
              <UsageAnalyticsDashboard
                stats={(usageStats as UsageAnalyticsDashboardStats | null) ?? null}
                loading={usageStatsLoading}
                formatBytes={(bytes) => formatUsageBytes(bytes ?? 0)}
              />

              {/* 按模型 / 渠道分组的真实调用与 Token 明细表 */}
              <div
                data-testid="usage-stats-model-table"
                className="rounded-card border border-sp-line bg-sp-card p-5 shadow-card space-y-3"
              >
                <div className="flex items-center justify-between gap-2 flex-wrap">
                  <div className="text-sm font-semibold text-sp-text">
                    {t("settings.usageStats.tableTitle")}
                  </div>
                  <span className="font-mono text-xs text-sp-text-muted">
                    {t("settings.usageStats.cachedTokens", {
                      tokens: (usageStats?.cached_tokens ?? 0).toLocaleString(),
                    })}
                  </span>
                </div>
                <div className="overflow-x-auto rounded-sp-sm border border-sp-line bg-sp-surface">
                  <table className="w-full text-left text-xs border-collapse">
                    <thead>
                      <tr className="border-b border-sp-line bg-sp-card text-sp-text-secondary font-semibold">
                        <th className="py-2 px-3">{t("settings.usageStats.colRoute")}</th>
                        <th className="py-2 px-3 text-right">{t("settings.usageStats.colRuns")}</th>
                        <th className="py-2 px-3 text-right">{t("settings.usageStats.colPrompt")}</th>
                        <th className="py-2 px-3 text-right">{t("settings.usageStats.colCompletion")}</th>
                        <th className="py-2 px-3 text-right">{t("settings.usageStats.colReasoning")}</th>
                        <th className="py-2 px-3 text-right">{t("settings.usageStats.colCached")}</th>
                        <th className="py-2 px-3 text-right">{t("settings.usageStats.colTotal")}</th>
                      </tr>
                    </thead>
                    <tbody>
                      {usageStats?.model_breakdown && usageStats.model_breakdown.length > 0 ? (
                        usageStats.model_breakdown.map((m) => (
                          <tr
                            key={m.route}
                            data-testid={`usage-model-row-${m.route}`}
                            className="border-b border-sp-line/50 last:border-b-0"
                          >
                            <td className="py-2 px-3 font-mono font-semibold text-sp-text">
                              {m.route}
                            </td>
                            <td className="py-2 px-3 text-right font-mono text-sp-text-secondary">
                              {m.run_count}
                            </td>
                            <td className="py-2 px-3 text-right font-mono text-sp-text-secondary">
                              {m.prompt_tokens.toLocaleString()}
                            </td>
                            <td className="py-2 px-3 text-right font-mono text-sp-text-secondary">
                              {m.completion_tokens.toLocaleString()}
                            </td>
                            <td className="py-2 px-3 text-right font-mono text-sp-text-secondary">
                              {m.reasoning_tokens.toLocaleString()}
                            </td>
                            <td className="py-2 px-3 text-right font-mono text-sp-text-secondary">
                              {m.cached_tokens.toLocaleString()}
                            </td>
                            <td className="py-2 px-3 text-right font-mono font-semibold text-sp-text">
                              {m.total_tokens.toLocaleString()}
                            </td>
                          </tr>
                        ))
                      ) : (
                        <tr>
                          <td
                            colSpan={7}
                            className="py-4 px-3 text-center text-sp-text-muted"
                          >
                            {t("settings.usageStats.tableEmpty")}
                          </td>
                        </tr>
                      )}
                    </tbody>
                  </table>
                </div>
              </div>
                </>
              )}
            </div>
          )}

          {activeCategory === "storage" && (
            <div className="eo-settings-layout">
              <div className="eo-settings-page-header">
                <h2 className="text-[17px] font-semibold text-sp-text tracking-[-0.015em]">
                  {t("settings.dialog.storageTitle")}
                </h2>
              </div>
              <div className="eo-settings-section">
                {storagePanel}
              </div>
            </div>
          )}

          {activeCategory === "network" && (
            <div className="eo-settings-layout">
              <div className="eo-settings-page-header">
                <h2 className="text-[17px] font-semibold text-sp-text tracking-[-0.015em]">
                  {t("settings.dialog.transferTitle")}
                </h2>
              </div>
              {concurrencyPanel && (
                <div className="eo-settings-section">
                  {concurrencyPanel}
                </div>
              )}
              <TransferEngineSettingsSection onSyncDemConcurrency={onSyncDemConcurrency} />
            </div>
          )}


        </div>
      </main>
    </div>
  );
}

function BusySendSettingsSection() {
  const { t } = useI18n();
  const [busySendMode, setBusySendMode] = useState<BusySendMode>(
    () => getBusySendPrefs().busySendMode
  );

  useEffect(() => {
    return subscribeBusySendPrefs((next) => {
      setBusySendMode(next.busySendMode);
    });
  }, []);

  const handleSelect = (mode: BusySendMode) => {
    const next = setBusySendPrefs({ busySendMode: mode });
    setBusySendMode(next.busySendMode);
  };

  return (
    <div className="eo-settings-section" data-testid="busy-send-settings-section">
      <h3 className="text-[14px] font-semibold text-sp-text">
        {t("settings.busySend.title")}
      </h3>

      <div className="eo-settings-card">
        <div className="eo-settings-card-content">
          <div className="eo-settings-card-row">
            <div className="flex flex-wrap items-center justify-between gap-4">
              <div className="text-[12.5px] text-sp-text-secondary">
                {t("settings.busySend.description")}
              </div>

              <div
                role="group"
                aria-label={t("settings.busySend.title")}
                className="eo-segmented-track shrink-0"
              >
                <button
                  type="button"
                  data-testid="busy-send-mode-queue"
                  aria-pressed={busySendMode === "queue"}
                  data-active={busySendMode === "queue"}
                  onClick={() => handleSelect("queue")}
                  className="eo-segmented-pill h-[26px] justify-center whitespace-nowrap px-3 cursor-pointer"
                >
                  {t("settings.busySend.queue")}
                </button>
                <button
                  type="button"
                  data-testid="busy-send-mode-steer"
                  aria-pressed={busySendMode === "steer"}
                  data-active={busySendMode === "steer"}
                  onClick={() => handleSelect("steer")}
                  className="eo-segmented-pill h-[26px] justify-center whitespace-nowrap px-3 cursor-pointer"
                >
                  {t("settings.busySend.steer")}
                </button>
              </div>
            </div>
          </div>
        </div>

        <div className="eo-settings-card-beard">
          {busySendMode === "queue" ? (
            <span>
              <strong className="text-sp-text font-semibold">
                {t("settings.busySend.queueSummaryTitle")}
              </strong>
              {t("settings.busySend.queueSummaryBody")}
            </span>
          ) : (
            <span>
              <strong className="text-sp-text font-semibold">
                {t("settings.busySend.steerSummaryTitle")}
              </strong>
              {t("settings.busySend.steerSummaryBody")}
            </span>
          )}
        </div>
      </div>
    </div>
  );
}

