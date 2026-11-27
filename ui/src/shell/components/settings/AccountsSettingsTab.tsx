import React, { useCallback, useEffect, useState } from "react";
import { useI18n, type MessageKey } from "@/i18n";
import {
  AlertCircle,
  AlertTriangle,
  CheckCircle2,
  ChevronDown,
  ExternalLink,
  Eye,
  EyeOff,
  Globe,
  KeyRound,
  Loader2,
  RefreshCw,
  Trash2,
} from "lucide-react";
import type { AccountStatus, AccountId, AccountState } from "../../api/types.ts";
import {
  ACCOUNT_METAS,
  filterRenderedAccounts,
  orderAccountsForSettings,
  type AccountMeta,
} from "../../api/accountLogic.ts";
import { getAccountStatuses, isAccountStatusesWired, isFixtureBackend } from "../../api";
import { getDesktopShellBridge } from "@/lib/desktopShellBridge";
import { queryOpenGeodata } from "@/lib/bridge";
import { formatBridgeErrorValue } from "@/lib/bridgeErrorFormat";
import { useAutoSaveDraft } from "@/components/desktop/useAutoSaveDraft";
import { AccountPortalLinkButton } from "@/components/desktop/AccountPortalLinkButton";
import {
  GEE_QUOTA_CONFIGURATION_URL,
  formatLocalizedGeeTierLabel,
  geeQuotaStatus,
  type GeeQuotaStatusResponse,
} from "@/lib/geeCloudSuiteClient";
import { useDelayedFlag, useFullTurnSpin } from "@/hooks/useLoadingPrimitives";
import { emitTopToast } from "@/components/ui/TopCenterToast";

function formatQuotaHours(hours: number): string {
  if (!Number.isFinite(hours)) return "0";
  if (Number.isInteger(hours)) return String(hours);
  const oneDec = Math.round(hours * 10) / 10;
  if (Math.abs(hours - oneDec) < 1e-4) {
    return Number.isInteger(oneDec) ? String(oneDec) : oneDec.toFixed(1);
  }
  return Number(hours.toFixed(2)).toString();
}

function isValidGeeQuotaSnapshot(val: unknown): val is GeeQuotaStatusResponse {
  return Boolean(
    val &&
      typeof val === "object" &&
      typeof (val as GeeQuotaStatusResponse).configured === "boolean" &&
      typeof (val as GeeQuotaStatusResponse).account_tier === "string" &&
      (val as GeeQuotaStatusResponse).cloud_monitoring &&
      typeof (val as GeeQuotaStatusResponse).cloud_monitoring === "object" &&
      (val as GeeQuotaStatusResponse).local_ledger &&
      typeof (val as GeeQuotaStatusResponse).local_ledger === "object"
  );
}

function openExternalUrlSafe(url: string): void {
  try {
    const bridge = getDesktopShellBridge();
    const openFn = bridge?.application?.openExternalUrl;
    if (typeof openFn === "function") {
      Promise.resolve(openFn(url)).catch(() => {
        if (typeof window !== "undefined") {
          window.open(url, "_blank", "noopener,noreferrer");
        }
      });
      return;
    }
  } catch {
    // Desktop shell bridge is unregistered in browser or unit-test environments
  }
  if (typeof window !== "undefined") {
    window.open(url, "_blank", "noopener,noreferrer");
  }
}

export interface AccountsSettingsTabProps {
  cdsePanel?: React.ReactNode;
  earthdataPanel?: React.ReactNode;
  bhoonidhiPanel?: React.ReactNode;
  usgsPanel?: React.ReactNode;
}

const GEE_SECRET_MASK = "••••••••••••••••••••••••••••••••";

const PROVIDER_CHANNEL_COVERAGE: Record<string, { count: number; summary: string }> = {
  cdse: {
    count: 3,
    summary: "Sentinel-2 (L2A / L1C)、Sentinel-3 多载荷与 Copernicus POD 轨道官方直连",
  },
  earthdata: {
    count: 12,
    summary: "Sentinel-1 全系列（含 SLC Burst）、NISAR L（9 级产品）、OPERA DISP、ARIA GUNW、ALOS-1/2、SEASAT、ASTER/NISAR DEM、IGS GIM",
  },
  bhoonidhi: {
    count: 1,
    summary: "NISAR S-Band 全系列 8 级产品 印度航天局官方归档",
  },
  usgs_m2m: {
    count: 1,
    summary: "Landsat Collection 2 Level-2 官方原始 .tar 归档包",
  },
  opentopography: {
    count: 6,
    summary: "NASADEM、AW3D30（正高/椭球高）、SRTM GL1（正高/椭球高）、SRTM GL3（需 API Key）",
  },
  gacos: {
    count: 1,
    summary: "全球对流层天顶总延迟 ZTD 大气相位屏改正产品（需通知邮箱）",
  },
  gee: {
    count: 14,
    summary: "公共 STAC 与 13 个云端数据集免账号检索 · 官方裁剪下载需配置 Project ID 与凭据",
  },
  mpc: {
    count: 7,
    summary: "HLS L30/S30、ESA WorldCover、JRC 地表水体、ALOS 镶嵌、MODIS NDVI、Sentinel-5P（另供 S2/Landsat 免密备用）",
  },
};

const OPEN_DIRECT_EXTRA_SOURCES: Array<{ id: string; name: string; count: number; detail: string }> = [
  {
    id: "aws_cop_dem",
    name: "Copernicus DEM AWS Open Data",
    count: 2,
    detail: "GLO-30 (30m) / GLO-90 (90m) 云原生 COG 公开瓦片",
  },
  {
    id: "fabdem",
    name: "FABDEM V1-2 全球裸地高程",
    count: 1,
    detail: "University of Bristol / Fathom 30m 去除林冠建筑裸地高程",
  },
  {
    id: "gem15",
    name: "Google GEM-15 全球高程",
    count: 2,
    detail: "GEM-15 DTM 裸地正高与 DSM 地表正高 15m 公开 COG 瓦片",
  },
  {
    id: "copernicus_pod",
    name: "Copernicus POD 哨兵精密轨道服务",
    count: 1,
    detail: "Sentinel-1 AUX_POEORB 精密定轨星历免密直连",
  },
  {
    id: "comet_licsar",
    name: "COMET-LiCSAR 全球干涉图门户",
    count: 1,
    detail: "英国 NERC JASMIN 公开 Sentinel-1 干涉图与相干性产品",
  },
  {
    id: "arco_era5",
    name: "ARCO-ERA5 全球大气再分析",
    count: 1,
    detail: "云原生 ERA5 小时级气象与水汽时间序列公开检索",
  },
  {
    id: "usgs_fdsn",
    name: "USGS FDSN 全球地震事件目录",
    count: 1,
    detail: "全球历史与实时发震时刻、震源机制及震中空间检索",
  },
  {
    id: "ngl_gnss",
    name: "Nevada Geodetic Lab (NGL) GNSS",
    count: 1,
    detail: "全球连续运行 GNSS 基准站三维形变时间序列公开直连",
  },
  {
    id: "global_xyz_tiles",
    name: "全球多源在线高清影像与地形底图",
    count: 1,
    detail: "聚合多组免密钥卫星、地形与行政区划在线瓦片源",
  },
  {
    id: "aws_sentinel1_grd",
    name: "Sentinel-1 GRD (AWS Open Data)",
    count: 1,
    detail: "Element 84 STAC 与 AWS S3 eu-central-1 免账号哨兵1号 GRD COG 直连",
  },
  {
    id: "noaa_aws_weather",
    name: "NOAA GFS / GOES 气象卫星 (AWS Open Data)",
    count: 1,
    detail: "AWS S3 免账号 NOAA GFS 0.25° .idx 分片子集与 GOES ABI NetCDF 观测",
  },
  {
    id: "overture_maps",
    name: "Overture Maps 全球开源矢量 (AWS Open Data)",
    count: 1,
    detail: "Overture STAC 与 AWS S3 GeoParquet 全球建筑/道路/兴趣点按区直连",
  },
];

export function AccountsSettingsTab({ cdsePanel, earthdataPanel, bhoonidhiPanel, usgsPanel }: AccountsSettingsTabProps) {
  const { locale, t } = useI18n();
  const [statuses, setStatuses] = useState<Record<string, AccountStatus>>({});
  const [expandedAccountIds, setExpandedAccountIds] = useState<Set<AccountId>>(() => {
    if (typeof window === "undefined") return new Set<AccountId>();
    const initial = new Set<AccountId>();
    const hash = window.location.hash;
    for (const candidate of ["cdse", "earthdata", "bhoonidhi", "usgs_m2m", "opentopography", "gacos"] as AccountId[]) {
      if (hash.includes(`expand=${candidate}`) || hash.includes(`scroll=${candidate}`) || hash.includes(`${candidate}-panel`)) {
        initial.add(candidate);
      }
    }
    return initial;
  });
  const [wired, setWired] = useState(true);
  const [otgKey, setOtgKey] = useState("");
  const [otgDraftVisible, setOtgDraftVisible] = useState(false);
  const [otgBusy, setOtgBusy] = useState(false);
  const [otgError, setOtgError] = useState<string | null>(null);
  const [otgRevealed, setOtgRevealed] = useState<string | null>(null);
  const [otgRevealing, setOtgRevealing] = useState(false);

  const [gacosEmail, setGacosEmail] = useState("");
  const [gacosBusy, setGacosBusy] = useState(false);
  const [gacosError, setGacosError] = useState<string | null>(null);

  const [geeExpanded, setGeeExpanded] = useState(() => {
    if (typeof window === "undefined") return false;
    return window.location.hash.includes("expand=gee") || window.location.hash.includes("scroll=gee");
  });
  const [geeConfigured, setGeeConfigured] = useState(false);
  const [geeProjectId, setGeeProjectId] = useState("");
  const [geeAuthMode, setGeeAuthMode] = useState<"service_account" | "oauth_token">("service_account");
  const [geeClientEmail, setGeeClientEmail] = useState<string | null>(null);
  const [geeSecret, setGeeSecret] = useState("");
  const [geeSecretMasked, setGeeSecretMasked] = useState(false);
  const [geeSecretRevealing, setGeeSecretRevealing] = useState(false);
  const [geeAction, setGeeAction] = useState<"save" | "verify" | "clear" | null>(null);
  const geeBusy = geeAction !== null;
  const [geeFeedback, setGeeFeedback] = useState<{ ok: boolean; message: string } | null>(null);
  const [geeQuota, setGeeQuota] = useState<GeeQuotaStatusResponse | null>(null);
  const [geeRefreshingQuota, setGeeRefreshingQuota] = useState(false);

  const showOtgRevealSpinner = useDelayedFlag(otgRevealing);
  const showOtgClearSpinner = useDelayedFlag(otgBusy);
  const showGacosClearSpinner = useDelayedFlag(gacosBusy);
  const { spinClass: geeQuotaSpinClass } = useFullTurnSpin(geeRefreshingQuota);
  const showGeeSecretSpinner = useDelayedFlag(geeSecretRevealing);
  const showGeeSaveSpinner = useDelayedFlag(geeAction === "save");
  const showGeeVerifySpinner = useDelayedFlag(geeAction === "verify");
  const showGeeClearSpinner = useDelayedFlag(geeAction === "clear");

  const loadGeeQuota = useCallback(async (forceRefresh = false, manualToast = false) => {
    if (forceRefresh) {
      setGeeRefreshingQuota(true);
    }
    try {
      const qs = await geeQuotaStatus(forceRefresh);
      if (isValidGeeQuotaSnapshot(qs)) {
        setGeeQuota(qs);
        if (manualToast) {
          emitTopToast({ text: t("accounts.gee.quotaRefreshedToast"), tone: "success" });
        }
      }
    } catch {
      // ignore in browser fixture mode
      if (manualToast) {
        emitTopToast({ text: t("accounts.gee.quotaRefreshFailedToast"), tone: "danger" });
      }
    } finally {
      if (forceRefresh) {
        setGeeRefreshingQuota(false);
      }
    }
  }, [t]);

  const loadGeeStatus = useCallback(async () => {
    try {
      const res: any = await queryOpenGeodata({ dataset: "gee_account_status" });
      if (res && res.ok) {
        const isCfg = Boolean(res.configured ?? res.gee_ready);
        setGeeConfigured(isCfg);
        setGeeSecretMasked((prev) => (isCfg ? true : prev));
        if (res.project_id) setGeeProjectId(String(res.project_id));
        if (res.auth_mode === "oauth_token" || res.auth_mode === "service_account") {
          setGeeAuthMode(res.auth_mode);
        }
        setGeeClientEmail(res.client_email ? String(res.client_email) : null);
        if (isCfg) {
          void loadGeeQuota(false);
        } else {
          setGeeQuota(null);
        }
      }
    } catch {
      // ignore in browser fixture mode
    }
  }, [loadGeeQuota]);

  const loadStatuses = useCallback(async () => {
    const isWired = isAccountStatusesWired();
    setWired(isWired);
    if (!isWired) return;
    try {
      const list = await getAccountStatuses();
      const map: Record<string, AccountStatus> = {};
      for (const item of list) map[item.account_id] = item;
      setStatuses(map);
    } catch {
      setWired(false);
    }
  }, []);

  const otgAutoSave = useAutoSaveDraft({
    draftKey: otgKey,
    complete: Boolean(otgKey.trim()),
    save: async () => {
      const credentials = getDesktopShellBridge().credentials;
      return credentials ? credentials.saveOpentopographyKey(otgKey.trim()) : { ok: false, error: t("settings.accounts.credentialUnsupported") };
    },
    onSaved: async () => {
      setOtgDraftVisible(false);
      await loadStatuses();
      window.dispatchEvent(new window.Event("eostage:credentials-changed"));
    },
  });
  const gacosAutoSave = useAutoSaveDraft({
    draftKey: gacosEmail,
    complete: Boolean(gacosEmail.trim()),
    save: async () => {
      const credentials = getDesktopShellBridge().credentials;
      return credentials ? credentials.saveGacosEmail(gacosEmail.trim()) : { ok: false, error: t("settings.accounts.credentialUnsupported") };
    },
    onSaved: async () => {
      await loadStatuses();
      window.dispatchEvent(new window.Event("eostage:credentials-changed"));
    },
  });

  useEffect(() => {
    void loadStatuses();
    void loadGeeStatus();
    const onChanged = () => {
      void loadStatuses();
      void loadGeeStatus();
    };
    window.addEventListener("eostage:credentials-changed", onChanged);
    return () => window.removeEventListener("eostage:credentials-changed", onChanged);
  }, [loadStatuses, loadGeeStatus]);

  useEffect(() => {
    if (typeof window === "undefined") return;
    const focusTargetFromHashOrEvent = (targetId?: string) => {
      const hash = window.location.hash;
      let pendingStorageId = "";
      try {
        pendingStorageId = window.sessionStorage?.getItem("eostage:pending-focus-account") || "";
        if (pendingStorageId) {
          window.sessionStorage?.removeItem("eostage:pending-focus-account");
        }
      } catch {
        // ignore storage access errors
      }
      const resolved =
        targetId ||
        pendingStorageId ||
        (["gee", "opentopography", "gacos", "cdse", "earthdata", "bhoonidhi", "usgs_m2m"].find(
          (id) => hash.includes(`scroll=${id}`) || hash.includes(`expand=${id}`),
        ) ?? "");
      if (!resolved) return;
      if (resolved === "gee") {
        setGeeExpanded(true);
      } else {
        setExpandedAccountIds((prev) => {
          const next = new Set(prev);
          next.add(resolved as AccountId);
          return next;
        });
      }
      setTimeout(() => {
        document.getElementById(`account-card-${resolved}`)?.scrollIntoView({ behavior: "instant" as ScrollBehavior, block: "center" });
      }, 150);
    };
    focusTargetFromHashOrEvent();
    const onFocusCard = (ev: Event) => {
      const detail = (ev as CustomEvent<{ accountId?: string }>).detail;
      if (detail?.accountId) focusTargetFromHashOrEvent(detail.accountId);
    };
    const onHashChange = () => focusTargetFromHashOrEvent();
    window.addEventListener("hashchange", onHashChange);
    window.addEventListener("eostage:focus-account-card", onFocusCard);
    return () => {
      window.removeEventListener("hashchange", onHashChange);
      window.removeEventListener("eostage:focus-account-card", onFocusCard);
    };
  }, []);

  const gacosStatus = statuses.gacos;
  const gacosState: AccountState = gacosStatus?.state || "credentials_missing";

  // 回显已保存的 GACOS 通知邮箱
  useEffect(() => {
    let cancelled = false;
    if (gacosState !== "credentials_missing" && !gacosEmail) {
      void getDesktopShellBridge().credentials?.readForEdit("gacos_email").then((res) => {
        if (!cancelled && res?.ok && res.value) {
          setGacosEmail(res.value);
        }
      });
    }
    return () => { cancelled = true; };
  }, [gacosState]);

  async function clearOtg() {
    setOtgBusy(true);
    setOtgError(null);
    setOtgRevealed(null);
    setOtgDraftVisible(false);
    otgAutoSave.reset();
    try {
      const credentials = getDesktopShellBridge().credentials;
      if (!credentials) throw new Error(t("settings.accounts.credentialUnsupported"));
      const result = await credentials.clearOpentopographyKey();
      if (!result.ok) {
        setOtgError(formatBridgeErrorValue(result));
        return;
      }
      setOtgKey("");
      await loadStatuses();
      window.dispatchEvent(new window.Event("eostage:credentials-changed"));
    } catch (error) {
      setOtgError(formatBridgeErrorValue(error));
    } finally {
      setOtgBusy(false);
    }
  }

  async function toggleOtgReveal() {
    if (otgKey) {
      setOtgDraftVisible((prev) => !prev);
      return;
    }
    if (otgRevealed !== null) {
      setOtgRevealed(null);
      return;
    }
    setOtgRevealing(true);
    setOtgError(null);
    try {
      const credentials = getDesktopShellBridge().credentials;
      if (!credentials) return;
      const res = await credentials.reveal("opentopography_key");
      if (res?.ok && res.value !== null) {
        setOtgRevealed(res.value);
      } else {
        setOtgError(t("accounts.fields.readFailed"));
      }
    } catch {
      setOtgError(t("accounts.fields.readFailed"));
    } finally {
      setOtgRevealing(false);
    }
  }

  async function toggleGeeSecretReveal() {
    if (!geeSecretMasked) {
      setGeeSecretMasked(true);
      return;
    }
    if (geeSecret.trim()) {
      setGeeSecretMasked(false);
      return;
    }
    setGeeSecretRevealing(true);
    setGeeFeedback(null);
    try {
      const res: any = await queryOpenGeodata({
        dataset: "gee_account_reveal",
        code: geeProjectId.trim(),
      });
      if (res && res.ok && typeof res.value === "string" && res.value.trim()) {
        setGeeSecret(res.value);
        setGeeSecretMasked(false);
      } else {
        setGeeFeedback({ ok: false, message: String(res?.error || t("accounts.fields.readFailed")) });
      }
    } catch (err) {
      setGeeFeedback({ ok: false, message: formatBridgeErrorValue(err) });
    } finally {
      setGeeSecretRevealing(false);
    }
  }

  async function clearGacos() {
    setGacosBusy(true);
    setGacosError(null);
    gacosAutoSave.reset();
    try {
      const credentials = getDesktopShellBridge().credentials;
      if (!credentials) throw new Error(t("settings.accounts.credentialUnsupported"));
      const result = await credentials.clearGacosEmail();
      if (!result.ok) {
        setGacosError(formatBridgeErrorValue(result));
        return;
      }
      setGacosEmail("");
      await loadStatuses();
      window.dispatchEvent(new window.Event("eostage:credentials-changed"));
    } catch (error) {
      setGacosError(formatBridgeErrorValue(error));
    } finally {
      setGacosBusy(false);
    }
  }

  const visibleMetas = orderAccountsForSettings(filterRenderedAccounts(ACCOUNT_METAS));
  const accountMetas = visibleMetas.filter((meta) => meta.canLogin);
  const anonymousMetas = visibleMetas.filter((meta) => !meta.canLogin);
  const panelMap: Partial<Record<AccountId, React.ReactNode>> = {
    cdse: cdsePanel,
    earthdata: earthdataPanel,
    bhoonidhi: bhoonidhiPanel,
    usgs_m2m: usgsPanel,
  };

  function renderStateBadge(state: AccountState) {
    const className = "inline-flex items-center gap-1 rounded-full px-2 py-1 text-xs";
    const stateKeys = {
      logged_in: "loggedIn",
      login_required: "loginRequired",
      credentials_missing: "credentialsMissing",
      service_unavailable: "serviceUnavailable",
      not_required: "notRequired",
    } as const;
    const label = t(`settings.accounts.states.${stateKeys[state]}`);
    switch (state) {
      case "logged_in":
      case "login_required":
        return <span className={`${className} bg-sp-success-bg text-sp-success-text`}><CheckCircle2 className="h-3 w-3" />{label}</span>;
      case "credentials_missing":
        return <span className={`${className} bg-sp-surface-muted text-sp-text-secondary`}><KeyRound className="h-3 w-3" />{label}</span>;
      case "service_unavailable":
        return <span className={`${className} bg-sp-danger-bg text-sp-danger-text`}><AlertCircle className="h-3 w-3" />{label}</span>;
      case "not_required":
        return <span className={`${className} bg-sp-surface-muted text-sp-text-secondary`}><Globe className="h-3 w-3" />{label}</span>;
      default:
        return null;
    }
  }

  function renderAccountCard(meta: AccountMeta) {
    const status = statuses[meta.id];
    const state: AccountState = meta.id === "mpc" ? "not_required" : status?.state || "credentials_missing";
    const isExpanded = expandedAccountIds.has(meta.id);
    const injectedPanel = panelMap[meta.id];
    const canExpand = Boolean(injectedPanel) || meta.id === "opentopography";

    const otgConfigured = state !== "credentials_missing";
    const isShowingOtgMask = otgConfigured && !otgKey && otgRevealed === null;
    const isOtgRevealed = otgRevealed !== null;
    const isOtgVisible = isOtgRevealed || (Boolean(otgKey) && otgDraftVisible);
    const otgDisplayValue = isOtgRevealed ? otgRevealed : otgKey;
    const canToggleOtgEye = otgConfigured || Boolean(otgKey);
    const coverage = PROVIDER_CHANNEL_COVERAGE[meta.id];

    const toggleAccountExpand = () => {
      if (!canExpand) return;
      setExpandedAccountIds((previous) => {
        const next = new Set(previous);
        if (next.has(meta.id)) next.delete(meta.id);
        else next.add(meta.id);
        return next;
      });
    };

    return (
      <div key={meta.id} id={`account-card-${meta.id}`} className="eo-settings-card-row">
        <div
          onClick={canExpand ? toggleAccountExpand : undefined}
          className={`flex items-center justify-between gap-4${
            canExpand ? " cursor-pointer select-none" : ""
          }`}
        >
          <div className="flex min-w-0 flex-1 flex-col gap-1">
            <div className="flex min-w-0 items-center gap-2 flex-wrap">
              <span className="text-[13px] font-semibold text-sp-text">{meta.name_zh}</span>
              <AccountPortalLinkButton
                target={meta.id}
                onError={meta.id === "opentopography" ? setOtgError : undefined}
              />
              {renderStateBadge(state)}
            </div>
            {coverage && (
              <p className="text-xs text-sp-text-secondary [text-wrap:pretty] leading-relaxed">
                {t(`accounts.catalog.providerSummaries.${meta.id}` as MessageKey)}
              </p>
            )}
          </div>
          {canExpand && (
            <button
              type="button"
              onClick={(event) => {
                event.stopPropagation();
                toggleAccountExpand();
              }}
              className="eo-settings-btn-secondary cred-action-btn bg-sp-accent-fill shrink-0"
            >
              {isExpanded ? t("settings.accounts.collapse") : state === "credentials_missing" ? t("settings.accounts.configure") : t("settings.accounts.manage")}
              <ChevronDown
                className={`h-3 w-3 text-sp-text-muted transition-transform duration-fast ease-sp${
                  isExpanded ? " rotate-180" : ""
                }`}
              />
            </button>
          )}
        </div>
        {isExpanded && injectedPanel && <div className="mt-2 border-t border-sp-divider pt-3">{injectedPanel}</div>}
        {isExpanded && meta.id === "opentopography" && (
          <div className="mt-2 space-y-3 border-t border-sp-divider pt-3">
            <div>
              <div className="mb-1 flex items-center justify-between gap-2 text-[13px]">
                <div className="flex items-center gap-2 flex-wrap">
                  <label htmlFor="opentopography-key" className="inline-flex items-center gap-2 text-[13px] text-sp-text-secondary">
                    <span className="whitespace-nowrap">API Key</span>
                    {otgConfigured && (
                      <span className="inline-flex items-center rounded-full bg-sp-success-bg px-2 py-1 text-xs font-normal leading-none text-sp-success-text whitespace-nowrap shrink-0">
                        {t("accounts.fields.saved")}
                      </span>
                    )}
                  </label>
                  <AccountPortalLinkButton
                    target="opentopography_key"
                    onError={setOtgError}
                  />
                </div>
              </div>
              <div className="relative">
                <input
                  id="opentopography-key"
                  aria-label="OpenTopography API Key"
                  type={isOtgVisible ? "text" : "password"}
                  value={otgDisplayValue}
                  placeholder={isShowingOtgMask ? "••••••••" : t("accounts.catalog.otgPlaceholder")}
                  onChange={(event) => {
                    if (otgRevealed !== null) setOtgRevealed(null);
                    setOtgKey(event.currentTarget.value);
                  }}
                  onBlur={() => void otgAutoSave.saveNow()}
                  disabled={otgBusy || otgRevealing}
                  readOnly={isOtgRevealed}
                  autoComplete="off"
                  className={`eo-settings-input w-full font-mono text-[13px] placeholder:font-sans disabled:opacity-50 ${
                    canToggleOtgEye ? "pr-9" : ""
                  }`}
                />
                {canToggleOtgEye && (
                  <button
                    type="button"
                    aria-label={isOtgVisible ? t("accounts.fields.hide") : t("accounts.fields.show")}
                    title={isOtgVisible ? t("accounts.fields.hide") : t("accounts.fields.show")}
                    disabled={otgBusy || otgRevealing}
                    onClick={() => void toggleOtgReveal()}
                    className="absolute right-1 top-1/2 -translate-y-1/2 rounded-sp-sm p-1 text-[13px] text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text focus:outline-none disabled:opacity-40"
                  >
                    {showOtgRevealSpinner ? (
                      <Loader2 className="h-4 w-4 animate-spin" />
                    ) : isOtgVisible ? (
                      <EyeOff className="h-4 w-4 text-sp-accent-strong" />
                    ) : (
                      <Eye className="h-4 w-4" />
                    )}
                  </button>
                )}
              </div>
            </div>
            <div className="flex items-center gap-3">
              <button
                type="button"
                aria-busy={otgBusy}
                onClick={() => void clearOtg()}
                disabled={otgBusy || otgAutoSave.state.status === "saving" || state === "credentials_missing"}
                className="inline-flex h-8 min-w-[76px] items-center justify-center gap-1 rounded-sp-sm border border-sp-line bg-sp-surface px-3 text-[13px] font-normal text-sp-danger-text transition-colors hover:bg-sp-danger-bg disabled:cursor-not-allowed disabled:opacity-40 disabled:text-sp-text-muted disabled:hover:bg-sp-surface cursor-pointer"
              >
                {showOtgClearSpinner ? <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin" /> : <Trash2 className="h-3.5 w-3.5 shrink-0" />}
                {t("settings.accounts.clear")}
              </button>
              {otgAutoSave.state.status === "saving" && <span role="status" className="text-xs text-sp-text-muted">{t("settings.accounts.savingProgress")}</span>}
              {otgAutoSave.state.status === "saved" && <span role="status" className="text-xs text-sp-text-muted">{t("settings.accounts.saved")}</span>}
            </div>
            {(otgError ?? otgAutoSave.state.error) && <p role="alert" className="break-words text-xs text-sp-danger-text">{otgError ?? otgAutoSave.state.error}</p>}
          </div>
        )}
      </div>
    );
  }

  const isGacosExpanded = expandedAccountIds.has("gacos" as AccountId);
  type GacosInContract = "gacos" extends AccountId ? true : false;
  const gacosEnabled: GacosInContract = true;
  const hasGeeSecretPresent = geeConfigured || Boolean(geeSecret.trim());
  const geeDisplaySecret = geeSecretMasked && hasGeeSecretPresent ? GEE_SECRET_MASK : geeSecret;

  return (
    <div className="eo-settings-layout">
      {/* EOStage SettingsPageHeader */}
      <div
        data-testid="settings-accounts-tour-anchor"
        className="eo-settings-page-header"
      >
        <div className="flex min-w-0 flex-1 flex-col gap-1">
          <div className="flex items-center gap-2 flex-wrap">
            <h2 className="text-[17px] font-semibold text-sp-text tracking-[-0.015em]">
              {t("settings.categories.auth")}
            </h2>
            {Boolean(import.meta.env?.DEV && isFixtureBackend()) && (
              <span className="inline-flex items-center gap-1 rounded-full border border-sp-line bg-sp-canvas px-3 py-1 text-xs text-sp-text-muted">
                <span className="h-1.5 w-1.5 rounded-full bg-sp-status-warning" />
                {t("settings.accounts.developmentFixture")}
              </span>
            )}
          </div>
        </div>
      </div>

      {!wired && <p role="status" className="text-xs text-sp-text-muted">{t("settings.accounts.statusUnavailable")}</p>}
      {/* Section 1: 需认证授权的数据源 */}
      <div className="eo-settings-section">
        <h3 className="text-[14px] font-semibold text-sp-text">{t("accounts.catalog.authSectionTitle")}</h3>

        <div className="eo-settings-card">
          <div className="eo-settings-card-content divide-y divide-sp-divider">
            {accountMetas.map(renderAccountCard)}
            {gacosEnabled && (
              <div id="account-card-gacos" className="eo-settings-card-row">
                <div
                  onClick={() =>
                    setExpandedAccountIds((previous) => {
                      const next = new Set(previous);
                      if (next.has("gacos" as AccountId)) next.delete("gacos" as AccountId);
                      else next.add("gacos" as AccountId);
                      return next;
                    })
                  }
                  className="flex items-center justify-between gap-4 cursor-pointer select-none"
                >
                  <div className="flex min-w-0 flex-1 flex-col gap-1">
                    <div className="flex min-w-0 items-center gap-2 flex-wrap">
                      <span className="text-[13px] font-semibold text-sp-text">{t("settings.accounts.gacosTitle")}</span>
                      <AccountPortalLinkButton target="gacos" onError={setGacosError} />
                      {renderStateBadge(gacosState)}
                    </div>
                    <p className="text-xs text-sp-text-secondary [text-wrap:pretty] leading-relaxed">
                      {t("accounts.catalog.providerSummaries.gacos")}
                    </p>
                  </div>
                  <button
                    type="button"
                    onClick={(event) => {
                      event.stopPropagation();
                      setExpandedAccountIds((previous) => {
                        const next = new Set(previous);
                        if (next.has("gacos" as AccountId)) next.delete("gacos" as AccountId);
                        else next.add("gacos" as AccountId);
                        return next;
                      });
                    }}
                    className="eo-settings-btn-secondary cred-action-btn bg-sp-accent-fill shrink-0"
                  >
                    {isGacosExpanded
                      ? t("settings.accounts.collapse")
                      : gacosState === "credentials_missing"
                        ? t("settings.accounts.configure")
                        : t("settings.accounts.manage")}
                    <ChevronDown
                      className={`h-3 w-3 text-sp-text-muted transition-transform duration-fast ease-sp${
                        isGacosExpanded ? " rotate-180" : ""
                      }`}
                    />
                  </button>
                </div>
                {isGacosExpanded && (
                  <div className="mt-2 space-y-3 border-t border-sp-divider pt-3">
                    <div>
                      <label htmlFor="gacos-email" className="mb-1 inline-flex items-center gap-2 text-[13px] text-sp-text-secondary">
                        <span className="whitespace-nowrap">{t("settings.accounts.notificationEmail")}</span>
                        {gacosState !== "credentials_missing" && gacosAutoSave.state.status !== "saved" && (
                          <span className="inline-flex items-center rounded-full bg-sp-success-bg px-2 py-1 text-xs font-normal leading-none text-sp-success-text whitespace-nowrap shrink-0">
                            {t("accounts.fields.saved")}
                          </span>
                        )}
                      </label>
                      <input
                        id="gacos-email"
                        aria-label={t("settings.accounts.gacosNotificationEmail")}
                        type="email"
                        placeholder="name@institution.edu"
                        value={gacosEmail}
                        onChange={(event) => setGacosEmail(event.currentTarget.value)}
                        onBlur={() => void gacosAutoSave.saveNow()}
                        disabled={gacosBusy}
                        autoComplete="email"
                        className="eo-settings-input w-full font-sans text-[13px] disabled:opacity-50"
                      />
                    </div>
                    <div className="flex items-center gap-3">
                      <button
                        type="button"
                        aria-busy={gacosBusy}
                        onClick={() => void clearGacos()}
                        disabled={gacosBusy || gacosAutoSave.state.status === "saving" || gacosState === "credentials_missing"}
                        className="inline-flex h-8 min-w-[76px] items-center justify-center gap-1 rounded-sp-sm border border-sp-line bg-sp-surface px-3 text-[13px] font-normal text-sp-danger-text transition-colors hover:bg-sp-danger-bg disabled:cursor-not-allowed disabled:opacity-40 disabled:text-sp-text-muted disabled:hover:bg-sp-surface cursor-pointer"
                      >
                        {showGacosClearSpinner ? <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin" /> : <Trash2 className="h-3.5 w-3.5 shrink-0" />}
                        {t("settings.accounts.clear")}
                      </button>
                      {gacosAutoSave.state.status === "saving" && <span role="status" className="text-xs text-sp-text-muted">{t("settings.accounts.savingProgress")}</span>}
                      {gacosAutoSave.state.status === "saved" && <span role="status" className="text-xs text-sp-text-muted">{t("settings.accounts.saved")}</span>}
                    </div>
                    {(gacosError ?? gacosAutoSave.state.error) && <p role="alert" className="break-words text-xs text-sp-danger-text">{gacosError ?? gacosAutoSave.state.error}</p>}
                  </div>
                )}
              </div>
            )}
            {/* Google Earth Engine (GEE) 账号与云项目配置 */}
            <div id="account-card-gee" className="eo-settings-card-row">
              <div
                onClick={() => setGeeExpanded((prev) => !prev)}
                className="flex items-center justify-between gap-4 cursor-pointer select-none"
              >
                <div className="flex min-w-0 flex-1 flex-col gap-1">
                  <div className="flex min-w-0 items-center gap-2 flex-wrap">
                    <span className="text-[13px] font-semibold text-sp-text">
                      Google Earth Engine (GEE)
                    </span>
                    <AccountPortalLinkButton
                      target="gee"
                      onError={(message) => setGeeFeedback({ ok: false, message })}
                    />
                    {renderStateBadge(geeConfigured ? "logged_in" : "credentials_missing")}
                    {geeConfigured && geeQuota?.configured && (
                      <span
                        data-testid="gee-quota-tier-badge"
                        className="inline-flex items-center gap-1 rounded-full border border-sp-line bg-sp-surface-muted px-2 py-1 text-xs font-semibold text-sp-text"
                      >
                        {(() => {
                          const tierText = formatLocalizedGeeTierLabel(
                            geeQuota.account_tier,
                            geeQuota.account_tier_label,
                            locale
                          );
                          return geeQuota.monthly_limit_eecu_hours != null
                            ? t("accounts.gee.tierMonthlyBadge", {
                                tier: tierText,
                                hours: formatQuotaHours(geeQuota.monthly_limit_eecu_hours),
                              })
                            : tierText;
                        })()}
                      </span>
                    )}
                    {geeConfigured && geeQuota?.configured && geeQuota.parallelism_restricted && (
                      <span
                        data-testid="gee-quota-throttle-badge"
                        className="inline-flex items-center gap-1 rounded-full bg-sp-warning-bg px-2 py-1 text-xs font-semibold text-sp-warning-text"
                      >
                        <AlertTriangle className="h-3 w-3" />
                        <span>{t("accounts.gee.throttledBadge")}</span>
                      </span>
                    )}
                  </div>
                  <p className="text-xs text-sp-text-secondary [text-wrap:pretty] leading-relaxed">
                    {t("accounts.catalog.providerSummaries.gee")}
                  </p>
                </div>
                <button
                  type="button"
                  data-testid="gee-account-expand-btn"
                  onClick={(event) => {
                    event.stopPropagation();
                    setGeeExpanded((prev) => !prev);
                  }}
                  className="eo-settings-btn-secondary cred-action-btn bg-sp-accent-fill shrink-0"
                >
                  {geeExpanded
                    ? t("settings.accounts.collapse")
                    : geeConfigured
                      ? t("settings.accounts.manage")
                      : t("settings.accounts.configure")}
                  <ChevronDown
                    className={`h-3 w-3 text-sp-text-muted transition-transform duration-fast ease-sp${
                      geeExpanded ? " rotate-180" : ""
                    }`}
                  />
                </button>
              </div>

              {geeConfigured && geeQuota?.configured && (
                <div
                  data-testid="gee-quota-ledger-panel"
                  className="mt-3 rounded-sp-card border border-sp-line bg-sp-surface-muted p-3 flex flex-col gap-2"
                >
                  <div className="flex flex-wrap items-center justify-between gap-2">
                    <div className="flex flex-wrap items-center gap-2 text-xs text-sp-text-secondary">
                      <span className="font-semibold text-sp-text">
                        {formatLocalizedGeeTierLabel(
                          geeQuota.account_tier,
                          geeQuota.account_tier_label,
                          locale
                        )}
                      </span>
                      <span>·</span>
                      <span>
                        {geeQuota.monthly_limit_eecu_hours != null
                          ? t("accounts.gee.monthlyComputeCap", {
                              hours: formatQuotaHours(geeQuota.monthly_limit_eecu_hours),
                            })
                          : t("accounts.gee.noHardMonthlyCap")}
                      </span>
                      {geeQuota.read_requests_per_minute != null && (
                        <>
                          <span>·</span>
                          <span>
                            {t("accounts.gee.readRatePerMinute", {
                              rpm: geeQuota.read_requests_per_minute,
                            })}
                          </span>
                        </>
                      )}
                      <span
                        className={`rounded-sp-sm px-2 py-0 text-xs font-semibold ${
                          geeQuota.parallelism_restricted
                            ? "bg-sp-warning-bg text-sp-warning-text"
                            : "bg-sp-success-bg text-sp-success-text"
                        }`}
                      >
                        {geeQuota.parallelism_restricted
                          ? t("accounts.gee.throttleProtecting")
                          : t("accounts.gee.unthrottledNormal")}
                      </span>
                    </div>

                    <div className="flex flex-wrap items-center gap-1">
                      <button
                        type="button"
                        data-testid="gee-refresh-quota-btn"
                        aria-busy={geeRefreshingQuota}
                        disabled={geeRefreshingQuota}
                        onClick={() => void loadGeeQuota(true, true)}
                        className="inline-flex items-center gap-1 rounded-sp-sm bg-transparent px-2 py-1 text-[13px] text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover disabled:opacity-50 transition-colors cursor-pointer shrink-0"
                      >
                        <RefreshCw className={`h-3.5 w-3.5 shrink-0 ${geeQuotaSpinClass}`} />
                        <span>{t("accounts.gee.refreshQuotaBtn")}</span>
                      </button>
                      <button
                        type="button"
                        data-testid="gee-open-quota-config-btn"
                        onClick={() => openExternalUrlSafe(GEE_QUOTA_CONFIGURATION_URL)}
                        title={t("accounts.gee.quotaConfigTitle")}
                        className="inline-flex items-center gap-1 rounded-sp-sm bg-transparent px-2 py-1 text-[13px] text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover transition-colors cursor-pointer"
                      >
                        <ExternalLink className="h-3 w-3" />
                        <span>{t("accounts.gee.quotaConfigBtn")}</span>
                      </button>
                    </div>
                  </div>

                  {geeQuota.cloud_monitoring.unlocked === true &&
                  geeQuota.cloud_monitoring.monthly_used_eecu_hours != null ? (
                    (() => {
                      const usedHoursNum = geeQuota.cloud_monitoring.monthly_used_eecu_hours;
                      const limitHoursNum = geeQuota.monthly_limit_eecu_hours;
                      if (limitHoursNum != null && limitHoursNum > 0) {
                        const remHoursNum =
                          geeQuota.cloud_monitoring.monthly_remaining_eecu_hours ??
                          Math.max(0, limitHoursNum - usedHoursNum);
                        const remPct = Math.max(0, Math.min(100, (remHoursNum / limitHoursNum) * 100));
                        const usedPct = Math.max(0, Math.min(100, (usedHoursNum / limitHoursNum) * 100));
                        const usedHours = formatQuotaHours(usedHoursNum);
                        const limitHours = formatQuotaHours(limitHoursNum);
                        const remainingHours = formatQuotaHours(remHoursNum);
                        const pct = formatQuotaHours(Number(remPct.toFixed(1)));
                        return (
                          <div
                            data-testid="gee-cloud-quota-unlocked"
                            className="rounded-sp-md border border-sp-line bg-sp-surface p-2 flex flex-col gap-2"
                          >
                            <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-sp-text">
                              <span>
                                {t("accounts.gee.unlockedRemainingSummary", {
                                  used: usedHours,
                                  limit: limitHours,
                                  remaining: remainingHours,
                                  pct,
                                })}
                              </span>
                            </div>
                            <div
                              role="progressbar"
                              data-testid="gee-cloud-quota-progressbar"
                              aria-label={t("accounts.gee.progressBarAria")}
                              aria-valuemin={0}
                              aria-valuemax={limitHoursNum}
                              aria-valuenow={usedHoursNum}
                              className="h-2 w-full overflow-hidden rounded-full bg-sp-surface-muted"
                            >
                              <div
                                data-testid="gee-cloud-quota-progress-fill"
                                style={{ width: `${usedPct}%` }}
                                className={`h-full rounded-full transition-all ${
                                  usedPct >= 85 ? "bg-sp-status-warning" : "bg-sp-accent"
                                }`}
                              />
                            </div>
                          </div>
                        );
                      }
                      return (
                        <div
                          data-testid="gee-cloud-quota-unlocked"
                          className="rounded-sp-md border border-sp-line bg-sp-surface p-2 text-xs text-sp-text"
                        >
                          {t("accounts.gee.commercialUsedSummary", {
                            used: formatQuotaHours(usedHoursNum),
                          })}
                        </div>
                      );
                    })()
                  ) : (
                    <div
                      data-testid="gee-cloud-quota-cardless-note"
                      className="rounded-sp-md border border-sp-line bg-sp-surface px-3 py-2 text-xs text-sp-text-secondary leading-snug"
                    >
                      {t("accounts.gee.cardlessSafeNote")}
                    </div>
                  )}

                  <div
                    data-testid="gee-local-usage-ledger"
                    className="flex flex-wrap items-center justify-between gap-2 pt-2 border-t border-sp-divider text-xs text-sp-text-secondary"
                  >
                    <span>
                      {t("accounts.gee.localLedgerSummary", {
                        month: geeQuota.local_ledger.month_utc,
                        total: geeQuota.local_ledger.total_requests,
                        analysis: geeQuota.local_ledger.analysis_runs,
                        downloads: geeQuota.local_ledger.chunk_downloads,
                        scripts: geeQuota.local_ledger.script_runs,
                      })}
                    </span>
                    <span className="font-mono text-sp-text">
                      {geeQuota.local_ledger.has_eecu_profile &&
                      geeQuota.local_ledger.profiled_eecu_seconds > 0
                        ? t("accounts.gee.localLedgerProfiled", {
                            eecu: geeQuota.local_ledger.profiled_eecu_seconds.toFixed(2),
                            wall: geeQuota.local_ledger.total_wall_seconds.toFixed(1),
                          })
                        : t("accounts.gee.localLedgerWallOnly", {
                            wall: geeQuota.local_ledger.total_wall_seconds.toFixed(1),
                          })}
                    </span>
                  </div>
                </div>
              )}

              {geeExpanded && (
                <div className="mt-2 space-y-3 border-t border-sp-divider pt-3">
                  <div>
                    <div className="mb-1 flex items-center gap-1 flex-wrap text-[13px]">
                      <label htmlFor="gee-project-id" className="inline-flex items-center flex-wrap gap-2 text-[13px] text-sp-text-secondary">
                        <span>Google Cloud / Earth Engine Project ID</span>
                        {geeConfigured && (
                          <span className="inline-flex items-center rounded-full bg-sp-success-bg px-2 py-1 text-xs font-normal leading-none text-sp-success-text whitespace-nowrap shrink-0 max-w-full truncate">
                            {t("accounts.fields.saved")}{geeClientEmail ? ` · ${geeClientEmail}` : ""}
                          </span>
                        )}
                      </label>
                      <AccountPortalLinkButton
                        target="gee_project_id"
                        onError={(message) => setGeeFeedback({ ok: false, message })}
                      />
                    </div>
                    <input
                      id="gee-project-id"
                      data-testid="gee-project-id"
                      aria-label="GEE Project ID"
                      type="text"
                      placeholder={t("accounts.gee.projectIdPlaceholder")}
                      value={geeProjectId}
                      onChange={(e) => setGeeProjectId(e.currentTarget.value)}
                      disabled={geeBusy}
                      className="eo-settings-input w-full font-sans text-[13px] disabled:opacity-50"
                    />
                  </div>

                  <div>
                    <div className="mb-1 flex items-center justify-between gap-2 flex-wrap">
                      <div className="flex items-center gap-1">
                        <span className="text-[13px] text-sp-text-secondary">{t("accounts.gee.credentialTypeLabel")}</span>
                        <AccountPortalLinkButton
                          target={geeAuthMode === "service_account" ? "gee_service_account" : "gee_oauth_token"}
                          onError={(message) => setGeeFeedback({ ok: false, message })}
                        />
                      </div>
                      <div className="inline-flex rounded-sp-sm bg-sp-surface-muted p-1">
                        <button
                          type="button"
                          data-testid="gee-auth-mode-sa"
                          onClick={() => setGeeAuthMode("service_account")}
                          className={`rounded-sp-xs px-3 py-1 text-[13px] cursor-pointer ${
                            geeAuthMode === "service_account"
                              ? "bg-sp-surface text-sp-text font-semibold shadow-xs"
                              : "text-sp-text-secondary hover:text-sp-text"
                          }`}
                        >
                          {t("accounts.gee.modeServiceAccount")}
                        </button>
                        <button
                          type="button"
                          data-testid="gee-auth-mode-oauth"
                          onClick={() => setGeeAuthMode("oauth_token")}
                          className={`rounded-sp-xs px-3 py-1 text-[13px] cursor-pointer ${
                            geeAuthMode === "oauth_token"
                              ? "bg-sp-surface text-sp-text font-semibold shadow-xs"
                              : "text-sp-text-secondary hover:text-sp-text"
                          }`}
                        >
                          {t("accounts.gee.modeOauthToken")}
                        </button>
                      </div>
                    </div>
                    <div className="relative">
                      <textarea
                        id="gee-secret-input"
                        data-testid="gee-secret-input"
                        aria-label={t("accounts.gee.secretInputAria")}
                        rows={3}
                        placeholder={
                          geeAuthMode === "service_account"
                            ? t("accounts.gee.saPlaceholder")
                            : t("accounts.gee.oauthPlaceholder")
                        }
                        value={geeDisplaySecret}
                        onChange={(e) => {
                          const raw = e.currentTarget.value;
                          const nextSecret = geeSecretMasked ? raw.replaceAll("•", "") : raw;
                          if (geeSecretMasked) {
                            setGeeSecretMasked(false);
                          }
                          setGeeSecret(nextSecret);
                          if (!geeProjectId.trim() && nextSecret.trimStart().startsWith("{")) {
                            try {
                              const parsed = JSON.parse(nextSecret.trim());
                              const inferredPid =
                                (typeof parsed?.project_id === "string" && parsed.project_id.trim()) ||
                                (typeof parsed?.quota_project_id === "string" && parsed.quota_project_id.trim()) ||
                                "";
                              if (inferredPid) {
                                setGeeProjectId(inferredPid);
                              }
                            } catch {
                              // Ignore partial JSON while typing
                            }
                          }
                        }}
                        disabled={geeBusy || geeSecretRevealing}
                        className={`eo-settings-input w-full py-2 font-mono text-[13px] placeholder:font-sans disabled:opacity-50 ${
                          hasGeeSecretPresent ? "pr-10" : ""
                        }`}
                      />
                      {hasGeeSecretPresent && (
                        <button
                          type="button"
                          data-testid="gee-secret-reveal-btn"
                          aria-label={geeSecretMasked ? t("accounts.fields.show") : t("accounts.fields.hide")}
                          title={geeSecretMasked ? t("accounts.fields.show") : t("accounts.fields.hide")}
                          disabled={geeBusy || geeSecretRevealing}
                          onClick={() => void toggleGeeSecretReveal()}
                          className="absolute right-2 top-2 rounded-sp-sm p-1 text-[13px] text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text focus:outline-none disabled:opacity-40 cursor-pointer"
                        >
                          {showGeeSecretSpinner ? (
                            <Loader2 className="h-4 w-4 animate-spin" />
                          ) : geeSecretMasked ? (
                            <Eye className="h-4 w-4" />
                          ) : (
                            <EyeOff className="h-4 w-4 text-sp-accent-strong" />
                          )}
                        </button>
                      )}
                    </div>
                  </div>

                  <div className="flex flex-wrap items-center gap-2">
                    <button
                      type="button"
                      data-testid="gee-account-save-btn"
                      aria-busy={geeAction === "save"}
                      disabled={geeBusy || !geeProjectId.trim() || (!geeSecret.trim() && !geeConfigured)}
                      onClick={async () => {
                        setGeeAction("save");
                        setGeeFeedback(null);
                        const secretToSave = geeSecretMasked ? geeSecret.trim() : geeSecret.trim();
                        try {
                          const res: any = await queryOpenGeodata({
                            dataset: "gee_account_save",
                            code: geeProjectId.trim(),
                            keyword: geeAuthMode,
                            sql: secretToSave,
                          });
                          if (res && res.ok) {
                            setGeeConfigured(true);
                            setGeeSecretMasked(true);
                            setGeeFeedback({
                              ok: true,
                              message:
                                locale === "zh-CN"
                                  ? String(res.message || res.message_zh || t("accounts.gee.savedDefaultMsg"))
                                  : t("accounts.gee.savedDefaultMsg"),
                            });
                            await loadGeeStatus();
                            await loadGeeQuota(true);
                            window.dispatchEvent(new window.Event("eostage:credentials-changed"));
                          } else {
                            setGeeFeedback({
                              ok: false,
                              message: String(res?.error || t("accounts.gee.saveFailedDefaultMsg")),
                            });
                          }
                        } catch (err) {
                          setGeeFeedback({ ok: false, message: formatBridgeErrorValue(err) });
                        } finally {
                          setGeeAction(null);
                        }
                      }}
                      className="cred-action-btn inline-flex h-8 min-w-[96px] items-center justify-center gap-1 px-3 text-[13px] cursor-pointer disabled:opacity-50"
                    >
                      {showGeeSaveSpinner ? <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin" /> : null}
                      <span>{t("accounts.gee.saveCredentialsBtn")}</span>
                    </button>

                    <button
                      type="button"
                      data-testid="gee-account-verify-btn"
                      aria-busy={geeAction === "verify"}
                      disabled={geeBusy || (!geeConfigured && (!geeProjectId.trim() || !geeSecret.trim()))}
                      onClick={async () => {
                        setGeeAction("verify");
                        setGeeFeedback(null);
                        try {
                          const res: any = await queryOpenGeodata({
                            dataset: "gee_account_verify",
                            code: geeProjectId.trim(),
                            keyword: geeAuthMode,
                            sql: geeSecret.trim(),
                          });
                          if (res && res.ok) {
                            setGeeFeedback({
                              ok: true,
                              message:
                                locale === "zh-CN"
                                  ? String(res.message || res.message_zh || t("accounts.gee.verifiedDefaultMsg"))
                                  : t("accounts.gee.verifiedDefaultMsg"),
                            });
                            void loadGeeQuota(true);
                          } else {
                            setGeeFeedback({
                              ok: false,
                              message: String(res?.error || t("accounts.gee.verifyFailedDefaultMsg")),
                            });
                          }
                        } catch (err) {
                          setGeeFeedback({ ok: false, message: formatBridgeErrorValue(err) });
                        } finally {
                          setGeeAction(null);
                        }
                      }}
                      className="eo-settings-btn-secondary inline-flex h-8 min-w-[92px] items-center justify-center gap-1 px-3 text-[13px] cursor-pointer disabled:opacity-50"
                    >
                      {showGeeVerifySpinner ? <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin" /> : null}
                      <span>{t("accounts.gee.testConnectionBtn")}</span>
                    </button>

                    <button
                      type="button"
                      data-testid="gee-account-clear-btn"
                      aria-busy={geeAction === "clear"}
                      disabled={geeBusy || !geeConfigured}
                      onClick={async () => {
                        setGeeAction("clear");
                        setGeeFeedback(null);
                        try {
                          const res: any = await queryOpenGeodata({ dataset: "gee_account_clear" });
                          if (res && res.ok) {
                            setGeeConfigured(false);
                            setGeeQuota(null);
                            setGeeProjectId("");
                            setGeeSecret("");
                            setGeeSecretMasked(false);
                            setGeeClientEmail(null);
                            setGeeFeedback({ ok: true, message: t("accounts.gee.clearedDefaultMsg") });
                            window.dispatchEvent(new window.Event("eostage:credentials-changed"));
                          }
                        } catch (err) {
                          setGeeFeedback({ ok: false, message: formatBridgeErrorValue(err) });
                        } finally {
                          setGeeAction(null);
                        }
                      }}
                      className="inline-flex h-8 min-w-[80px] items-center justify-center gap-1 rounded-sp-sm border border-sp-line bg-sp-surface px-3 text-[13px] font-normal text-sp-danger-text transition-colors hover:bg-sp-danger-bg disabled:cursor-not-allowed disabled:opacity-40 disabled:text-sp-text-muted disabled:hover:bg-sp-surface cursor-pointer"
                    >
                      {showGeeClearSpinner ? (
                        <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin" />
                      ) : (
                        <Trash2 className="h-3.5 w-3.5 shrink-0" />
                      )}
                      <span>{t("settings.accounts.clear")}</span>
                    </button>
                  </div>

                  {geeFeedback && (
                    <div
                      role={geeFeedback.ok ? "status" : "alert"}
                      data-testid="gee-feedback-msg"
                      className={`text-xs break-words p-2 rounded-sp-xs flex items-center gap-1 ${
                        geeFeedback.ok
                          ? "text-sp-success-text bg-sp-success-bg border border-sp-line"
                          : "text-sp-danger-text bg-sp-danger-bg border border-sp-line"
                      }`}
                    >
                      {geeFeedback.ok ? (
                        <CheckCircle2 className="h-4 w-4 shrink-0 text-sp-success-text" />
                      ) : (
                        <AlertCircle className="h-4 w-4 shrink-0 text-sp-danger-text" />
                      )}
                      <span>{geeFeedback.message}</span>
                    </div>
                  )}
                </div>
              )}
            </div>
          </div>
          <div className="eo-settings-card-beard">
            <span>{t("accounts.gee.localVaultFooterNote")}</span>
          </div>
        </div>
      </div>

      {/* Section 2: 免登录开放数据源 */}
      {anonymousMetas.length > 0 && (
        <div className="eo-settings-section">
          <h3 className="text-[14px] font-semibold text-sp-text">
            {t("accounts.catalog.openSectionTitle")}
          </h3>
          <div className="eo-settings-card">
            <div className="eo-settings-card-content divide-y divide-sp-divider">
              {anonymousMetas.map(renderAccountCard)}
              <div data-testid="open-direct-channels-summary" className="eo-settings-card-row">
                <div className="flex flex-col gap-2">
                  <div className="flex items-center gap-2 flex-wrap">
                    <span className="text-[13px] font-semibold text-sp-text">
                      {t("accounts.catalog.openExtraGroupTitle")}
                    </span>
                    {renderStateBadge("not_required")}
                  </div>
                  <div className="sp-grid-cards-sm">
                    {OPEN_DIRECT_EXTRA_SOURCES.map((src) => (
                      <div
                        key={src.id}
                        className="flex flex-col gap-1 rounded-sp-sm border border-sp-line bg-sp-canvas px-3 py-2"
                      >
                        <span className="text-[13px] font-semibold text-sp-text min-w-0">
                          {t(`accounts.catalog.openExtra.${src.id}_name` as MessageKey)}
                        </span>
                        <span className="text-xs text-sp-text-secondary [text-wrap:pretty] leading-relaxed">
                          {t(`accounts.catalog.openExtra.${src.id}_detail` as MessageKey)}
                        </span>
                      </div>
                    ))}
                  </div>
                </div>
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
