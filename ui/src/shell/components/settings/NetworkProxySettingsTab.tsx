import React, { useState, useEffect, useRef, useCallback } from "react";
import {
  CheckCircle2,
  AlertCircle,
  Loader2,
  Activity,
  ExternalLink,
} from "lucide-react";
import { t, useI18n, type MessageKey } from "@/i18n";
import type { GlobalProxyMode } from "../../api/types.ts";
import {
  PROXY_MODE_OPTIONS,
  validateProxyUrl,
  TEST_ENDPOINTS,
  projectProxyMode,
  proxyFieldsForMode,
  PROBE_ERROR_REASONS,
} from "../../api/networkSettingsLogic.ts";
import {
  getNetworkSettings,
  saveNetworkSettings,
  probeProviderConnectivity,
  openExternalUrl,
  type NetworkSettings,
  type NetworkSettingsInput,
  type ProviderConnectivityOutcome,
} from "@/lib/bridge.ts";
import { isFixtureBackend } from "../../api";
import { useDelayedFlag } from "@/hooks/useLoadingPrimitives";

type ProbeStatus = {
  results: ProviderConnectivityOutcome["results"];
  tested_at: string;
};

type SaveToast = {
  kind: "success" | "error";
  message: string;
} | null;

export function NetworkProxySettingsTab() {
  const { locale } = useI18n();
  const [globalMode, setGlobalMode] = useState<GlobalProxyMode>("direct");
  const [manualHost, setManualHost] = useState("127.0.0.1");
  const [manualPort, setManualPort] = useState("7890");

  // 保存从后端实际读取到的完整网络配置对象，避免保存时冲掉其他非本页负责的持久化字段
  const [loadedRawSettings, setLoadedRawSettings] = useState<NetworkSettings | null>(null);
  const loadedRawSettingsRef = useRef<NetworkSettings | null>(null);
  const lastSavedSignatureRef = useRef<string>("");
  const debounceTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const toastTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const [loadError, setLoadError] = useState<string | null>(null);
  const [validationError, setValidationError] = useState<string | null>(null);
  const [saveSuccessMsg, setSaveSuccessMsg] = useState<string | null>(null);
  const [toast, setToast] = useState<SaveToast>(null);
  const [isSaving, setIsSaving] = useState(false);

  const [testingConnection, setTestingConnection] = useState(false);
  const [probeStatus, setProbeStatus] = useState<ProbeStatus | null>(null);
  const showSavingSpinner = useDelayedFlag(isSaving);
  const showTestingSpinner = useDelayedFlag(testingConnection);

  const showToast = useCallback((kind: "success" | "error", message: string) => {
    if (toastTimerRef.current) clearTimeout(toastTimerRef.current);
    setToast({ kind, message });
    toastTimerRef.current = setTimeout(() => {
      setToast(null);
    }, kind === "error" ? 5000 : 3000);
  }, []);

  useEffect(() => {
    return () => {
      if (debounceTimerRef.current) clearTimeout(debounceTimerRef.current);
      if (toastTimerRef.current) clearTimeout(toastTimerRef.current);
    };
  }, []);

  // 初始化加载已有设置（三档投影：direct / auto / manual）
  useEffect(() => {
    void (async () => {
      try {
        const net = await getNetworkSettings();
        if (net && net.ok) {
          setLoadedRawSettings(net);
          loadedRawSettingsRef.current = net;
          const mode = projectProxyMode(net);
          setGlobalMode(mode);
          let resolvedHost = "127.0.0.1";
          let resolvedPort = "7890";
          if (mode === "manual") {
            try {
              const urlCandidate = net.proxy_url.startsWith("http://") || net.proxy_url.startsWith("https://")
                ? net.proxy_url
                : `http://${net.proxy_url}`;
              const url = new URL(urlCandidate);
              resolvedHost = url.hostname || "127.0.0.1";
              resolvedPort = url.port || "7890";
              setManualHost(resolvedHost);
              setManualPort(resolvedPort);
            } catch (_e) {
              // 保留默认
            }
          }
          const initialFields = proxyFieldsForMode(mode, { host: resolvedHost, port: resolvedPort });
          lastSavedSignatureRef.current = `${initialFields.proxy_enabled}|${initialFields.proxy_url}`;
        } else {
          setLoadError(t("settings.network.loadFailed"));
        }
      } catch (err: any) {
        setLoadError(t("settings.network.loadError", { error: err?.message || t("settings.channels.backendError") }));
      }
    })();
  }, []);

  const persistNetworkMode = useCallback(
    async (targetMode: GlobalProxyMode, hostVal: string, portVal: string, options?: { reportInvalid?: boolean }) => {
      if (targetMode === "manual") {
        const fullUrl = `http://${hostVal.trim()}:${portVal.trim()}`;
        const check = validateProxyUrl(fullUrl);
        if (!check.ok) {
          if (options?.reportInvalid) {
            const msg = check.error_zh || t("settings.network.invalidProxy");
            setValidationError(msg);
            showToast("error", msg);
          }
          return;
        }
      }

      const { proxy_enabled, proxy_url } = proxyFieldsForMode(targetMode, {
        host: hostVal,
        port: portVal,
      });
      const signature = `${proxy_enabled}|${proxy_url}`;
      if (signature === lastSavedSignatureRef.current && loadedRawSettingsRef.current) {
        setValidationError(null);
        return;
      }

      let baseline = loadedRawSettingsRef.current;
      if (!baseline) {
        const fresh = await getNetworkSettings();
        if (fresh && fresh.ok) {
          baseline = fresh;
          loadedRawSettingsRef.current = fresh;
          setLoadedRawSettings(fresh);
        }
      }
      if (!baseline) {
        const msg = t("settings.network.missingBaseline");
        setValidationError(msg);
        showToast("error", msg);
        return;
      }

      setValidationError(null);
      setIsSaving(true);
      try {
        // 严格构造 7 字段载荷，绝不混入 ok 等额外字段（修复 Rust deny_unknown_fields 保存报错）
        const payload: NetworkSettingsInput = {
          proxy_enabled,
          proxy_url,
          cache_enabled: baseline.cache_enabled,
          cache_dir: baseline.cache_dir,
          cache_limit_mb: baseline.cache_limit_mb,
          tianditu_token: baseline.tianditu_token,
          asf_ssl_verify: baseline.asf_ssl_verify,
        };

        const saved = await saveNetworkSettings(payload);
        if (!saved.ok) {
          const errMsg = t("settings.network.saveFailed", { error: saved.error });
          setValidationError(errMsg);
          showToast("error", errMsg);
          return;
        }

        loadedRawSettingsRef.current = saved;
        setLoadedRawSettings(saved);
        lastSavedSignatureRef.current = signature;
        setLoadError(null);
        const okMsg = t("settings.network.autoSaved");
        setSaveSuccessMsg(okMsg);
        showToast("success", okMsg);
      } catch (err: any) {
        const errMsg = t("settings.network.saveFailed", { error: err?.message || t("settings.channels.backendError") });
        setValidationError(errMsg);
        showToast("error", errMsg);
      } finally {
        setIsSaving(false);
      }
    },
    [showToast],
  );

  const handleSelectMode = (nextMode: GlobalProxyMode) => {
    if (debounceTimerRef.current) {
      clearTimeout(debounceTimerRef.current);
      debounceTimerRef.current = null;
    }
    setGlobalMode(nextMode);
    setValidationError(null);
    void persistNetworkMode(nextMode, manualHost, manualPort, { reportInvalid: nextMode === "manual" });
  };

  const scheduleManualAutoSave = (nextHost: string, nextPort: string) => {
    if (debounceTimerRef.current) clearTimeout(debounceTimerRef.current);
    const fullUrl = `http://${nextHost.trim()}:${nextPort.trim()}`;
    if (!validateProxyUrl(fullUrl).ok) return;
    debounceTimerRef.current = setTimeout(() => {
      void persistNetworkMode("manual", nextHost, nextPort, { reportInvalid: false });
    }, 550);
  };

  const handleManualBlur = () => {
    if (debounceTimerRef.current) {
      clearTimeout(debounceTimerRef.current);
      debounceTimerRef.current = null;
    }
    if (globalMode === "manual") {
      void persistNetworkMode("manual", manualHost, manualPort, { reportInvalid: true });
    }
  };

  const handleRunConnectionTest = async () => {
    setTestingConnection(true);
    setProbeStatus(null);
    try {
      const outcome = await probeProviderConnectivity();
      if (outcome.ok) {
        setProbeStatus({
          results: outcome.results,
          tested_at: new Date().toLocaleTimeString(locale, { hour12: false }),
        });
      } else {
        setValidationError(t("settings.network.probeFailed", { error: outcome.error }));
      }
    } catch (err: any) {
      setValidationError(t("settings.network.probeFailed", { error: err?.message || t("settings.network.probeServiceError") }));
    } finally {
      setTestingConnection(false);
    }
  };

  const handleOpenProviderSite = async (site: string) => {
    const result = await openExternalUrl(site);
    if (!result.ok) setValidationError(t("settings.network.providerSiteFailed"));
  };

  return (
    <div className="eo-settings-layout">
      {/* EOStage SettingsPageHeader */}
      <div className="eo-settings-page-header">
        <div className="flex min-w-0 flex-1 flex-col gap-1">
          <div className="flex items-center gap-2 flex-wrap">
            <h2 className="text-[17px] font-semibold text-sp-text tracking-[-0.015em]">
              {t("settings.network.title")}
            </h2>
            {Boolean(import.meta.env?.DEV && isFixtureBackend()) && (
              <span className="inline-flex items-center gap-1 rounded-full border border-sp-line bg-sp-canvas px-3 py-1 text-xs text-sp-text-muted">
                <span className="h-1.5 w-1.5 rounded-full bg-sp-status-warning" />
                <span>{t("settings.accounts.developmentFixture")}</span>
              </span>
            )}
          </div>
        </div>
      </div>

      {loadError && (
        <div className="flex items-center gap-2 rounded-sp-md border border-sp-line bg-sp-warning-bg p-3 text-xs text-sp-warning-text">
          <AlertCircle className="h-4 w-4 shrink-0" />
          <span>{loadError}</span>
        </div>
      )}

      {/* 1. 全局模式选择 (点击即自动保存生效，无需手动保存按钮) */}
      <div className="eo-settings-section">
        <h3 className="text-[14px] font-semibold text-sp-text">{t("settings.network.modeTitle")}</h3>

        <div className="eo-settings-card">
          <div className="eo-settings-card-content">
            <div className="eo-settings-card-row">
              <div className="flex flex-wrap items-center justify-between gap-4">
                <h4 className="text-[13px] font-normal text-sp-text">{t("settings.network.outboundModeTitle")}</h4>

                <div className="eo-segmented-track">
                  {PROXY_MODE_OPTIONS.map((opt) => (
                    <label
                      key={opt.value}
                      data-active={globalMode === opt.value}
                      className="eo-segmented-pill h-[26px] px-3 text-[13px] cursor-pointer select-none"
                    >
                      <input
                        type="radio"
                        name="global_proxy_mode"
                        value={opt.value}
                        checked={globalMode === opt.value}
                        onChange={() => handleSelectMode(opt.value)}
                        className="sr-only"
                      />
                      <span>{t(`settings.network.modes.${opt.value}` as MessageKey)}</span>
                    </label>
                  ))}
                </div>
              </div>
            </div>

            {/* 手动 HTTP 代理配置区 */}
            {globalMode === "manual" && (
              <div className="eo-settings-card-row border-t border-sp-divider">
                <div className="text-[12.5px] font-semibold text-sp-text">
                  {t("settings.network.manualTitle")}
                </div>
                <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
                  <div className="sm:col-span-2 space-y-1">
                    <label htmlFor="network-proxy-host" className="block text-[13px] text-sp-text-secondary">
                      {t("settings.network.host")}
                    </label>
                    <input
                      id="network-proxy-host"
                      type="text"
                      placeholder={t("settings.network.hostPlaceholder")}
                      value={manualHost}
                      onChange={(e) => {
                        const next = e.target.value;
                        setManualHost(next);
                        setValidationError(null);
                        scheduleManualAutoSave(next, manualPort);
                      }}
                      onBlur={handleManualBlur}
                      className="eo-settings-input w-full font-mono text-[13px] placeholder:font-sans"
                    />
                  </div>
                  <div className="space-y-1">
                    <label htmlFor="network-proxy-port" className="block text-[13px] text-sp-text-secondary">
                      {t("settings.network.port")}
                    </label>
                    <input
                      id="network-proxy-port"
                      type="text"
                      placeholder={t("settings.network.portPlaceholder")}
                      value={manualPort}
                      onChange={(e) => {
                        const next = e.target.value;
                        setManualPort(next);
                        setValidationError(null);
                        scheduleManualAutoSave(manualHost, next);
                      }}
                      onBlur={handleManualBlur}
                      className="eo-settings-input w-full font-mono text-[13px] placeholder:font-sans"
                    />
                  </div>
                </div>
              </div>
            )}
          </div>

          {/* Concentric SettingsCardBeard: 自动保存状态提示（静默就地反馈，不使用底部浮动成功弹窗） */}
          <div className="eo-settings-card-beard">
            <div>
              {showSavingSpinner ? (
                <span role="status" className="text-xs text-sp-text-secondary flex items-center gap-1">
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  {t("settings.network.saving")}
                </span>
              ) : toast && toast.kind !== "error" ? (
                <span
                  role="status"
                  data-testid="network-autosave-toast"
                  className="text-xs text-sp-success-text flex items-center gap-1"
                >
                  <CheckCircle2 className="h-3.5 w-3.5 shrink-0" />
                  <span>{toast.message}</span>
                </span>
              ) : validationError ? (
                <span role="alert" className="text-xs text-sp-danger-text flex items-center gap-1">
                  <AlertCircle className="h-3.5 w-3.5" />
                  {validationError}
                </span>
              ) : saveSuccessMsg ? (
                <span role="status" className="text-xs text-sp-success-text flex items-center gap-1">
                  <CheckCircle2 className="h-3.5 w-3.5" />
                  {saveSuccessMsg}
                </span>
              ) : null}
            </div>
          </div>
        </div>
      </div>

      {/* 2. 真实连通性探测区域（覆盖 9 大遥感与云端平台） */}
      <div className="eo-settings-section">
        <div className="flex items-center justify-between gap-4">
          <div className="flex flex-col gap-1">
            <h3 className="text-[14px] font-semibold text-sp-text">{t("settings.network.probeTitle")}</h3>
            <p className="text-[13px] text-sp-text-secondary">{t("settings.network.probeDescription")}</p>
          </div>
          <button
            type="button"
            onClick={handleRunConnectionTest}
            disabled={testingConnection}
            aria-busy={testingConnection}
            className="eo-settings-btn-secondary min-w-[116px] justify-center shrink-0 disabled:opacity-50"
          >
            {showTestingSpinner ? (
              <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin" />
            ) : (
              <Activity className="h-3.5 w-3.5 shrink-0" />
            )}
            <span>{t("settings.network.testConnection")}</span>
          </button>
        </div>

        <div className="eo-settings-card">
          <div
            className={`eo-settings-card-content divide-y divide-sp-divider transition-opacity duration-200 ${
              testingConnection && probeStatus ? "opacity-60" : ""
            }`}
          >
            {TEST_ENDPOINTS.map((ep) => {
              const result = probeStatus?.results.find((r) => r.id === ep.id);
              const reason =
                result && !result.ok
                  ? result.error_code in PROBE_ERROR_REASONS
                    ? t(`settings.network.probeReasons.${result.error_code}` as MessageKey)
                    : t("settings.network.genericProbeFailure", { code: result.error_code })
                  : undefined;
              return (
                <div
                  key={ep.id}
                  data-testid={`network-probe-row-${ep.id}`}
                  className="flex items-center justify-between gap-3 px-3 py-2 text-[12.5px]"
                >
                  <a
                    href={ep.site}
                    onClick={(event) => {
                      event.preventDefault();
                      void handleOpenProviderSite(ep.site);
                    }}
                    className="flex min-w-0 items-center gap-1 text-sp-text hover:text-sp-accent-strong cursor-pointer"
                    title={ep.site}
                  >
                    <span className="truncate">{ep.name}</span>
                    <ExternalLink className="h-3 w-3 shrink-0 opacity-60" />
                  </a>
                  <div className="flex shrink-0 items-center gap-2 font-mono text-xs">
                    {!probeStatus && (
                      <span className="text-sp-text-muted">{t("settings.network.notProbed")}</span>
                    )}
                    {probeStatus && !result && (
                      <span className="text-sp-text-muted">—</span>
                    )}
                    {probeStatus && result && (result.ok ? (
                      <>
                        <span className="text-sp-success-text">
                          {result.latency_ms} ms
                        </span>
                        <CheckCircle2
                          className="h-3.5 w-3.5 text-sp-success-text"
                          aria-label={t("settings.network.reachable")}
                        />
                      </>
                    ) : (
                      <span
                        className="inline-flex items-center gap-1 text-sp-danger-text"
                        title={reason}
                      >
                        <AlertCircle className="h-3.5 w-3.5" aria-label={t("settings.network.unreachable")} />
                        {t("settings.network.unreachable")}
                      </span>
                    ))}
                  </div>
                </div>
              );
            })}
          </div>
          {probeStatus && (
            <div className="eo-settings-card-beard">
              <span className="font-mono text-xs">
                {t("settings.network.probeTime", { time: probeStatus.tested_at })}
              </span>
            </div>
          )}
        </div>
      </div>

      {toast && toast.kind === "error" && (
        <div
          role="alert"
          data-testid="network-autosave-toast"
          className="fixed bottom-6 right-6 z-50 flex max-w-sm items-center gap-2 rounded-sp-md border border-sp-line bg-sp-danger-bg px-3 py-2 text-[13px] text-sp-danger-text shadow-card"
        >
          <AlertCircle className="h-4 w-4 shrink-0 text-sp-danger-text" />
          <span>{toast.message}</span>
        </div>
      )}
    </div>
  );
}
