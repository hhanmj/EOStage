import React, { useState, useEffect, useCallback } from "react";
import {
  FolderOpen,
  Save,
  Check,
  AlertCircle,
  Loader2,
  Trash2,
  RefreshCw,
  ShieldCheck,
} from "lucide-react";
import { useI18n, type MessageKey } from "@/i18n";
import { cn } from "@/lib/utils";
import { getApplicationUsageStatistics } from "@/lib/bridge";
import { isTauriDesktopHost } from "@/lib/desktopShellBridge";
import { formatByteSize } from "@/lib/formatByteSize";
import {
  formatStoragePanelInfo,
  type StorageLiveMetrics,
} from "../../api/settingsStorageLogic.ts";
import { useDelayedFlag, useFullTurnSpin } from "@/hooks/useLoadingPrimitives";
import { emitTopToast } from "@/components/ui/TopCenterToast";
import {
  isTemporarySystemDownloadPath,
  TEMPORARY_DOWNLOAD_PATH_ERROR_ZH,
} from "@/lib/storagePathValidation";

export interface CacheCategorySummary {
  id: string;
  label_zh: string;
  description_zh: string;
  location: string;
  size_bytes: number;
  cap_bytes: number;
  entry_count: number;
  max_entries: number;
  ttl_seconds: number;
  clearable: boolean;
}

export interface CacheSummaryResponse {
  ok: boolean;
  cache_dir: string;
  sqlite_path: string;
  sqlite_size_bytes: number;
  sqlite_wal_bytes: number;
  total_size_bytes: number;
  total_cap_bytes: number;
  categories: CacheCategorySummary[];
  protected_notice_zh: string;
}

export interface ClearCachesResponse {
  ok: boolean;
  cleared_categories: string[];
  reclaimed_bytes: number;
  removed_entries: number;
  vacuumed: boolean;
  summary: CacheSummaryResponse;
}

export interface StorageSettingsTabProps {
  storageRootPath?: string | null;
  onChangeStoragePath?: () => void;
  onSaveStoragePath?: (newPath: string) => Promise<boolean | void>;
  busy?: boolean;
  onGetCacheSummary?: () => Promise<CacheSummaryResponse | null>;
  onClearDisposableCaches?: (
    categories?: string[] | null
  ) => Promise<ClearCachesResponse | null>;
}

const DEFAULT_PROTECTED_NOTICE_ZH =
  "清理缓存仅删除可重建的检索、边界、目录与临时预览缓存，不会影响已下载影像、下载任务记录、工作区设置或账号凭据。";

function hasNativeTauriInvoke(): boolean {
  if (typeof window === "undefined") return false;
  const internals = (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ as
    | { invoke?: unknown }
    | undefined;
  return Boolean(internals && typeof internals.invoke === "function");
}

async function invokeNativeTauriCommand<T>(
  command: string,
  args?: Record<string, unknown>
): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(command, args);
}

function formatTtlLabel(seconds: number, t: (key: MessageKey, values?: Record<string, string | number>) => string): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return "--";
  if (seconds >= 86400 && seconds % 86400 === 0) {
    return t("settings.storage.cache.ttlDays", { value: seconds / 86400 });
  }
  if (seconds >= 3600 && seconds % 3600 === 0) {
    return t("settings.storage.cache.ttlHours", { value: seconds / 3600 });
  }
  if (seconds >= 86400) {
    return t("settings.storage.cache.ttlDays", { value: Math.round(seconds / 86400) });
  }
  if (seconds >= 3600) {
    return t("settings.storage.cache.ttlHours", { value: Math.round(seconds / 3600) });
  }
  return t("settings.storage.cache.ttlMinutes", { value: Math.max(1, Math.round(seconds / 60)) });
}

export const StorageSettingsTab: React.FC<StorageSettingsTabProps> = ({
  storageRootPath,
  onChangeStoragePath,
  onSaveStoragePath,
  busy = false,
  onGetCacheSummary,
  onClearDisposableCaches,
}) => {
  const { t, locale } = useI18n();
  const [liveMetrics, setLiveMetrics] = useState<StorageLiveMetrics | null>(null);
  const info = formatStoragePanelInfo(storageRootPath, liveMetrics);
  const [textInput, setTextInput] = useState(storageRootPath || "");
  const [validationError, setValidationError] = useState<string | null>(null);
  const [savingText, setSavingText] = useState(false);
  const [savedSuccess, setSavedSuccess] = useState(false);
  const showSavingSpinner = useDelayedFlag(savingText);

  const canQueryCache = Boolean(onGetCacheSummary || hasNativeTauriInvoke());
  const canClearCache = Boolean(onClearDisposableCaches || hasNativeTauriInvoke());

  const [cacheSummary, setCacheSummary] = useState<CacheSummaryResponse | null>(null);
  const [cacheLoading, setCacheLoading] = useState(false);
  const [cacheError, setCacheError] = useState<string | null>(null);
  const [confirmClearOpen, setConfirmClearOpen] = useState(false);
  const [clearingCache, setClearingCache] = useState(false);
  const [clearFeedback, setClearFeedback] = useState<string | null>(null);

  const { spinClass: cacheRefreshSpinClass } = useFullTurnSpin(cacheLoading);
  const showClearingSpinner = useDelayedFlag(clearingCache);

  useEffect(() => {
    setTextInput(storageRootPath || "");
    if (storageRootPath?.trim()) {
      setValidationError(null);
    }
  }, [storageRootPath]);

  useEffect(() => {
    if (!isTauriDesktopHost()) return;
    let cancelled = false;
    void getApplicationUsageStatistics()
      .then((stats) => {
        if (cancelled || !stats) return;
        setLiveMetrics({
          dbSizeBytes: stats.sqlite_size_bytes,
          diskFreeBytes: stats.disk_free_bytes ?? null,
        });
      })
      .catch(() => {
        // 保持 "--" 优雅降级
      });
    return () => {
      cancelled = true;
    };
  }, [storageRootPath]);

  const loadCacheSummary = useCallback(async () => {
    if (!canQueryCache) return;
    setCacheLoading(true);
    setCacheError(null);
    try {
      const result = onGetCacheSummary
        ? await onGetCacheSummary()
        : await invokeNativeTauriCommand<CacheSummaryResponse>("get_cache_summary");
      if (result && result.ok !== false) {
        setCacheSummary(result);
        setLiveMetrics((prev) => ({
          dbSizeBytes:
            typeof result.sqlite_size_bytes === "number"
              ? result.sqlite_size_bytes
              : prev?.dbSizeBytes ?? null,
          diskFreeBytes: prev?.diskFreeBytes ?? null,
        }));
      } else {
        setCacheError(t("settings.storage.cache.readFailed"));
      }
    } catch (err: any) {
      setCacheError(err?.message_zh || err?.message || t("settings.storage.cache.readFailedFallback"));
    } finally {
      setCacheLoading(false);
    }
  }, [canQueryCache, onGetCacheSummary, t]);

  useEffect(() => {
    void loadCacheSummary();
  }, [loadCacheSummary]);

  const handleSave = async () => {
    const trimmed = textInput.trim();
    if (!trimmed) {
      setValidationError(t("settings.storage.emptyPath"));
      return;
    }
    if (isTemporarySystemDownloadPath(trimmed)) {
      setValidationError(t("settings.storage.temporaryPathError"));
      return;
    }
    setValidationError(null);
    if (!onSaveStoragePath) return;

    setSavingText(true);
    try {
      const ok = await onSaveStoragePath(trimmed);
      if (ok === true) {
        setSavedSuccess(true);
        emitTopToast({ text: t("settings.storage.saved"), tone: "success" });
        setTimeout(() => setSavedSuccess(false), 2000);
      } else {
        setValidationError(t("settings.storage.saveFailed"));
      }
    } catch (err: any) {
      setValidationError(err?.message_zh || err?.message || t("settings.storage.saveFailedShort"));
    } finally {
      setSavingText(false);
    }
  };

  const handleConfirmClearCache = async () => {
    if (!canClearCache || clearingCache) return;
    setClearingCache(true);
    setCacheError(null);
    setClearFeedback(null);
    try {
      const res = onClearDisposableCaches
        ? await onClearDisposableCaches(null)
        : await invokeNativeTauriCommand<ClearCachesResponse>("clear_disposable_caches", {
            categories: null,
          });
      if (!res || res.ok === false) {
        setCacheError(t("settings.storage.cache.clearIncomplete"));
        return;
      }
      setConfirmClearOpen(false);
      if (res.summary) {
        setCacheSummary(res.summary);
        setLiveMetrics((prev) => ({
          dbSizeBytes:
            typeof res.summary.sqlite_size_bytes === "number"
              ? res.summary.sqlite_size_bytes
              : prev?.dbSizeBytes ?? null,
          diskFreeBytes: prev?.diskFreeBytes ?? null,
        }));
      }
      const reclaimedLabel = formatByteSize(res.reclaimed_bytes, "0 B");
      const summaryLine =
        res.removed_entries > 0 || res.reclaimed_bytes > 0
          ? t("settings.storage.cache.clearSummaryReclaimed", {
              reclaimed: reclaimedLabel,
              removed: res.removed_entries,
              vacuumed: res.vacuumed ? t("settings.storage.cache.vacuumedSuffix") : "",
            })
          : t("settings.storage.cache.clearSummaryEmpty");
      setClearFeedback(summaryLine);
      emitTopToast({
        text:
          res.removed_entries > 0 || res.reclaimed_bytes > 0
            ? t("settings.storage.cache.toastReclaimed", { reclaimed: reclaimedLabel })
            : t("settings.storage.cache.toastUpToDate"),
        tone: "success",
      });
    } catch (err: any) {
      setCacheError(err?.message_zh || err?.message || t("settings.storage.cache.clearFailedFallback"));
    } finally {
      setClearingCache(false);
    }
  };

  const cacheUsagePercent =
    cacheSummary && cacheSummary.total_cap_bytes > 0
      ? Math.min(
          100,
          Math.round((cacheSummary.total_size_bytes / cacheSummary.total_cap_bytes) * 100)
        )
      : 0;
  const protectedNotice =
    !cacheSummary?.protected_notice_zh ||
    cacheSummary.protected_notice_zh.trim() === DEFAULT_PROTECTED_NOTICE_ZH ||
    locale !== "zh-CN"
      ? t("settings.storage.cache.defaultProtectedNotice")
      : cacheSummary.protected_notice_zh;

  return (
    <div className="space-y-4">
      <div className="eo-settings-card">
        <div className="eo-settings-card-content">
          {/* 行 1：当前路径与选择目录 */}
          <div className="eo-settings-card-row">
            <div className="flex flex-wrap items-center justify-between gap-4">
              <div className="flex min-w-0 flex-1 flex-col gap-1">
                <h4 className="text-[13px] font-normal text-sp-text">{t("settings.storage.title")}</h4>
                <div
                  title={storageRootPath?.trim() || undefined}
                  className="font-mono text-xs text-sp-text-secondary [overflow-wrap:anywhere] select-all"
                >
                  {storageRootPath?.trim()
                    ? storageRootPath
                        .trim()
                        .split(/([/\\])/)
                        .map((seg, i) => (
                          <React.Fragment key={i}>
                            {seg}
                            {(seg === "/" || seg === "\\") && <wbr />}
                          </React.Fragment>
                        ))
                    : t("settings.storage.unconfiguredPath")}
                </div>
              </div>
              {onChangeStoragePath && (
                <button
                  type="button"
                  onClick={onChangeStoragePath}
                  disabled={busy || savingText}
                  className="eo-settings-btn-secondary shrink-0 disabled:opacity-50"
                >
                  <FolderOpen className="h-3.5 w-3.5" />
                  <span>{t("settings.storage.pickLocation")}</span>
                </button>
              )}
            </div>
          </div>

          {/* 行 2：自定义路径输入与保存 */}
          {onSaveStoragePath && (
            <div className="eo-settings-card-row border-t border-sp-divider">
              <div className="text-[12.5px] text-sp-text-secondary">
                {t("settings.storage.customPath")}
              </div>
              <div className="flex items-center gap-2">
                <input
                  type="text"
                  value={textInput}
                  onChange={(e) => {
                    setTextInput(e.target.value);
                    setValidationError(null);
                    setSavedSuccess(false);
                  }}
                  disabled={busy || savingText}
                  placeholder={t("settings.storage.pathPlaceholder")}
                  className="eo-settings-input flex-1 font-mono"
                />
                <button
                  type="button"
                  onClick={() => void handleSave()}
                  disabled={busy || savingText || textInput.trim() === (storageRootPath || "").trim()}
                  aria-busy={savingText}
                  className="eo-settings-btn-primary h-8 min-w-[92px] justify-center shrink-0 disabled:opacity-50"
                >
                  {showSavingSpinner ? (
                    <>
                      <Loader2 className="h-3.5 w-3.5 animate-spin shrink-0" />
                      <span>{t("settings.storage.savePath")}</span>
                    </>
                  ) : savedSuccess ? (
                    <>
                      <Check className="h-3.5 w-3.5 text-sp-status-success shrink-0 sp-check-morph-in" />
                      <span>{t("settings.storage.saved")}</span>
                    </>
                  ) : (
                    <>
                      <Save className="h-3.5 w-3.5 shrink-0" />
                      <span>{t("settings.storage.savePath")}</span>
                    </>
                  )}
                </button>
              </div>
            </div>
          )}

          {validationError && (
            <div className="mx-3 mb-2 flex items-center gap-2 rounded-sp-sm bg-sp-danger-bg/40 border border-sp-danger-text/20 p-3 text-xs text-sp-danger-text">
              <AlertCircle className="h-4 w-4 shrink-0" />
              <span>{validationError}</span>
            </div>
          )}
        </div>

        {/* Concentric SettingsCardBeard */}
        <div className="eo-settings-card-beard">
          <span>
            {t("settings.storage.databaseSize")} <strong className="font-mono font-semibold text-sp-text">{info.dbSize}</strong>
          </span>
          <span>
            {t("settings.storage.diskFree")} <strong className="font-mono font-semibold text-sp-text">{info.diskFree}</strong>
          </span>
        </div>
      </div>

      {/* 卡片 2：缓存管理（可重建检索/行政区边界/目录缩略图缓存统计与安全一键清理） */}
      <div data-testid="storage-cache-card" className="eo-settings-card">
        <div className="eo-settings-card-content">
          <div className="eo-settings-card-row">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <div className="flex min-w-0 flex-1 flex-col gap-1">
                <h4 className="text-[13px] font-semibold text-sp-text">{t("settings.storage.cache.title")}</h4>
              </div>
              <div className="flex items-center gap-2 shrink-0">
                <button
                  type="button"
                  data-testid="cache-refresh-btn"
                  onClick={() => void loadCacheSummary()}
                  disabled={!canQueryCache || cacheLoading || clearingCache}
                  aria-label={t("settings.storage.cache.refreshAria")}
                  title={t("settings.storage.cache.refreshAria")}
                  className="eo-settings-btn-secondary disabled:opacity-50"
                >
                  <RefreshCw className={cn("h-3.5 w-3.5", cacheRefreshSpinClass)} />
                  <span>{t("settings.storage.cache.refresh")}</span>
                </button>
                <button
                  type="button"
                  data-testid="cache-clear-btn"
                  onClick={() => {
                    setClearFeedback(null);
                    setCacheError(null);
                    setConfirmClearOpen(true);
                  }}
                  disabled={!canClearCache || clearingCache || confirmClearOpen}
                  className="eo-settings-btn-secondary disabled:opacity-50"
                >
                  <Trash2 className="h-3.5 w-3.5" />
                  <span>{t("settings.storage.cache.clear")}</span>
                </button>
              </div>
            </div>
          </div>

          {!canQueryCache ? (
            <div className="eo-settings-card-row border-t border-sp-divider">
              <p className="text-xs text-sp-text-muted">
                {t("settings.storage.cache.desktopOnlyHint")}
              </p>
            </div>
          ) : cacheSummary ? (
            <>
              <div className="eo-settings-card-row border-t border-sp-divider space-y-2">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <div className="min-w-0 flex-1 space-y-1">
                    <div className="text-xs text-sp-text-muted">{t("settings.storage.cache.dirLabel")}</div>
                    <div
                      data-testid="cache-dir-path"
                      className="font-mono text-xs text-sp-text-secondary break-all"
                    >
                      {cacheSummary.cache_dir || "--"}
                    </div>
                  </div>
                  <div className="flex items-center gap-2 shrink-0">
                    <span
                      data-testid="cache-total-usage"
                      className="font-mono text-xs font-semibold text-sp-text"
                    >
                      {formatByteSize(cacheSummary.total_size_bytes, "0 B")} /{" "}
                      {formatByteSize(cacheSummary.total_cap_bytes, "--")}
                    </span>
                    <span className="eo-settings-badge font-mono">
                      {cacheUsagePercent}%
                    </span>
                  </div>
                </div>

                <div className="h-2 w-full rounded-full bg-sp-surface-muted overflow-hidden">
                  <div
                    className="h-full rounded-full bg-sp-accent-fill transition-[width] duration-base ease-sp"
                    style={{ width: `${cacheUsagePercent}%` }}
                  />
                </div>
              </div>

              {cacheSummary.categories.length > 0 && (
                <div
                  data-testid="cache-category-list"
                  className="eo-settings-card-row border-t border-sp-divider space-y-2"
                >
                  {cacheSummary.categories.map((cat) => {
                    const cacheLabel =
                      cat.id === "search_cache"
                        ? t("settings.storage.cache.categories.search_cache.label")
                        : cat.id === "admin_boundary_cache"
                          ? t("settings.storage.cache.categories.admin_boundary_cache.label")
                          : cat.id === "catalog_and_preview_cache"
                            ? t("settings.storage.cache.categories.catalog_and_preview_cache.label")
                            : cat.label_zh;
                    const cacheDesc =
                      cat.id === "search_cache"
                        ? t("settings.storage.cache.categories.search_cache.description")
                        : cat.id === "admin_boundary_cache"
                          ? t("settings.storage.cache.categories.admin_boundary_cache.description")
                          : cat.id === "catalog_and_preview_cache"
                            ? t("settings.storage.cache.categories.catalog_and_preview_cache.description")
                            : cat.description_zh;
                    return (
                    <div
                      key={cat.id}
                      data-testid={`cache-category-${cat.id}`}
                      className="flex flex-col gap-1 rounded-sp-sm border border-sp-line bg-sp-surface-muted px-3 py-2"
                    >
                      <div className="flex flex-wrap items-center justify-between gap-2">
                        <span className="text-[13px] font-semibold text-sp-text">
                          {cacheLabel}
                        </span>
                        <span className="font-mono text-xs font-semibold text-sp-text">
                          {formatByteSize(cat.size_bytes, "0 B")} /{" "}
                          {formatByteSize(cat.cap_bytes, "--")}
                        </span>
                      </div>
                      <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-sp-text-muted">
                        <span className="min-w-0 flex-1">{cacheDesc}</span>
                        <span className="shrink-0 font-mono text-sp-text-secondary">
                          {t("settings.storage.cache.itemsTtl", {
                            entryCount: cat.entry_count,
                            maxEntries: cat.max_entries,
                            ttl: formatTtlLabel(cat.ttl_seconds, t),
                          })}
                        </span>
                      </div>
                    </div>
                    );
                  })}
                </div>
              )}
            </>
          ) : cacheLoading ? (
            <div className="eo-settings-card-row border-t border-sp-divider">
              <div className="flex items-center gap-2 text-xs text-sp-text-muted">
                <Loader2 className="h-3.5 w-3.5 animate-spin shrink-0" />
                <span>{t("settings.storage.cache.scanning")}</span>
              </div>
            </div>
          ) : null}

          {confirmClearOpen && (
            <div
              data-testid="cache-clear-confirm-panel"
              role="region"
              aria-label={t("settings.storage.cache.confirmRegionAria")}
              className="mx-3 mb-3 rounded-sp-sm border border-sp-line bg-sp-surface-muted p-3 space-y-2"
            >
              <div className="text-[13px] font-semibold text-sp-text">
                {t("settings.storage.cache.confirmTitle")}
              </div>
              <div className="flex items-center justify-end gap-2 pt-1">
                <button
                  type="button"
                  data-testid="cache-clear-cancel-btn"
                  disabled={clearingCache}
                  onClick={() => setConfirmClearOpen(false)}
                  className="eo-settings-btn-secondary disabled:opacity-50"
                >
                  {t("settings.storage.cache.cancel")}
                </button>
                <button
                  type="button"
                  data-testid="cache-clear-confirm-btn"
                  disabled={clearingCache}
                  aria-busy={clearingCache}
                  onClick={() => void handleConfirmClearCache()}
                  className="inline-flex h-8 items-center gap-2 rounded-sp-sm bg-sp-danger-strong px-3 text-[13px] font-semibold text-white hover:opacity-90 disabled:opacity-50 cursor-pointer"
                >
                  {showClearingSpinner ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin shrink-0" />
                  ) : (
                    <Trash2 className="h-3.5 w-3.5 shrink-0" />
                  )}
                  <span>{t("settings.storage.cache.confirmClear")}</span>
                </button>
              </div>
            </div>
          )}

          {clearFeedback && (
            <div
              data-testid="cache-clear-feedback"
              role="status"
              className="mx-3 mb-3 flex items-center gap-2 rounded-sp-sm bg-sp-success-bg border border-sp-line px-3 py-2 text-xs text-sp-success-text"
            >
              <Check className="h-4 w-4 shrink-0" />
              <span>{clearFeedback}</span>
            </div>
          )}

          {cacheError && (
            <div
              data-testid="cache-error-banner"
              role="alert"
              className="mx-3 mb-3 flex items-center justify-between gap-2 rounded-sp-sm bg-sp-danger-bg/40 border border-sp-danger-text/20 p-3 text-xs text-sp-danger-text"
            >
              <div className="flex items-center gap-2 min-w-0">
                <AlertCircle className="h-4 w-4 shrink-0" />
                <span>{cacheError}</span>
              </div>
              <button
                type="button"
                onClick={() => void loadCacheSummary()}
                disabled={cacheLoading || clearingCache}
                className="eo-settings-btn-secondary shrink-0 disabled:opacity-50"
              >
                {t("settings.storage.cache.retry")}
              </button>
            </div>
          )}
        </div>

        <div className="eo-settings-card-beard">
          <span
            data-testid="cache-protected-notice"
            className="flex items-center gap-2 text-sp-text-secondary"
          >
            <ShieldCheck className="h-3.5 w-3.5 shrink-0 text-sp-text-muted" />
            <span>{protectedNotice}</span>
          </span>
        </div>
      </div>
    </div>
  );
};
