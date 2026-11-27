import React, { useState, useEffect, useCallback, useRef, useMemo } from "react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";
import {
  Cpu,
  Plus,
  Activity,
  CheckCircle2,
  AlertCircle,
  Loader2,
  Check,
  Info,
  ChevronDown,
  Clock,
  MessageSquare,
  RefreshCw,
  ArrowLeft,
  Box,
  Eye,
  EyeOff,
  Globe,
  MoreHorizontal,
  Pencil,
  X,
  ExternalLink,
  Copy,
} from "lucide-react";
import { getDesktopShellBridge } from "@/lib/desktopShellBridge.ts";
import { isTauriDesktopHost, openExternalUrl } from "@/lib/bridge.ts";
import { isFixtureBackend } from "../../api";
import { syncPublicModelRegistry } from "../../api/publicModelRegistry.ts";
import { HoverHintLabel } from "@/components/desktop/HoverHintLabel.tsx";
import {
  CHANNEL_PROTOCOLS,
  newChannelDraft,
  channelDraftFromView,
  buildChannelSavePayload,
  formatChannelTestResult,
  baseUrlNeedsV1Hint,
  appendV1ToBaseUrl,
  resolveChannelAuthKind,
  type ChannelAuthMeta,
  type ChannelProtocolValue,
  type ChannelDraft,
  type SubscriptionChannelQuota,
  type SubscriptionQuotaWindow,
} from "../../api/agentChannelsLogic.ts";
import type { AgentChannelModel } from "@/lib/generated/v3-contracts.ts";
import type { AgentChannelView, AgentChannelTest } from "../../api/agentBridgeTypes.ts";
import { notifyChannelsChanged } from "../../api/agentChannelsEvents.ts";
import { ChannelDeleteControl } from "./ChannelDeleteControl.tsx";
import { ChannelListItem } from "./ChannelListItem.tsx";
import { ChannelModelsEditor, ChannelModelsFetchDialog } from "./ChannelModelsEditor.tsx";
import { EmptyState } from "@/components/ui/EmptyState";
import { EmptyStateWireframe } from "@/components/ui/EmptyStateVisuals";
import { ProviderBrandLogo } from "./ProviderBrandLogo.tsx";
import { ModalOverlay } from "@/components/ui/ModalOverlay";
import { OptimisticInlineStatus, type OptimisticSyncState } from "@/components/ui/OptimisticInlineStatus.tsx";
import { SessionRowSkeletons } from "@/components/ui/Skeleton.tsx";
import { emitTopToast } from "@/components/ui/TopCenterToast.tsx";
import { useDelayedFlag, useFullTurnSpin } from "@/hooks/useLoadingPrimitives.ts";
import {
  hydrateDisabledPreferencesFromBackend,
  loadDisabledProviders,
  persistProviderEnabled,
} from "@/lib/channelPreferences.ts";
import {
  filterDiscoveredModels,
  channelDisplayName,
  isOfficialChannelId,
  OFFICIAL_CHANNEL_ID,
} from "../../api/channelModels.ts";
import {
  eostageAccountLogout,
  eostageAccountStatus,
  eostageAccountSyncModels,
  notifyEOStageAccountChanged,
  subscribeEOStageAccountChanged,
  type EOStageAccountStatus,
} from "../../api/eostageAccount.ts";
import { EOStageLoginModal } from "@/components/desktop/EOStageLoginModal.tsx";
import { EOStageBillingModal } from "@/components/desktop/EOStageBillingModal.tsx";
import {
  clearChannelHealth,
  getChannelHealthMap,
  resolveChannelHealthDotState,
  setChannelHealth,
  subscribeChannelHealth,
  type ChannelHealthEntry,
} from "../../api/channelHealth.ts";
import {
  OAUTH_PKCE_PROVIDERS,
  generatePkcePair,
  generateOAuthState,
  buildOAuthAuthorizationUrl,
  extractOAuthAuthorizationCode,
  decodeJwtPayload,
  extractChatGptAccountId,
  isOAuthTokenExpiringSoon,
  type OAuthPkceProviderPreset,
} from "@/lib/eoAgentSuite.ts";

interface ProviderCatalogPreset {
  id: string;
  name: string;
  subtitle: string;
  category: "eostage" | "other";
  badge?: string;
  protocol: ChannelProtocolValue;
  base_url: string;
  models: AgentChannelModel[];
}

const PROVIDER_CATALOG_PRESETS: readonly ProviderCatalogPreset[] = [
  {
    id: "deepseek",
    name: "DeepSeek",
    subtitle: "DeepSeek 官方 API 兼容端点（支持拉取可用模型列表）",
    category: "other",
    protocol: "chat_completions",
    base_url: "https://api.deepseek.com/v1",
    models: [],
  },
  {
    id: "moonshot-kimi",
    name: "Kimi",
    subtitle: "Moonshot / Kimi 官方 API 兼容端点",
    category: "other",
    protocol: "chat_completions",
    base_url: "https://api.moonshot.cn/v1",
    models: [],
  },
  {
    id: "minimax",
    name: "MiniMax",
    subtitle: "MiniMax 官方 OpenAI 兼容 API 端点",
    category: "other",
    protocol: "chat_completions",
    base_url: "https://api.minimax.chat/v1",
    models: [],
  },
  {
    id: "zhipu-glm",
    name: "智谱 GLM",
    subtitle: "智谱开放平台 OpenAI 兼容 API 端点",
    category: "other",
    protocol: "chat_completions",
    base_url: "https://open.bigmodel.cn/api/paas/v4",
    models: [],
  },
  {
    id: "aliyun-bailian",
    name: "阿里云百炼",
    subtitle: "通义千问 DashScope OpenAI 兼容 API 端点",
    category: "other",
    protocol: "chat_completions",
    base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    models: [],
  },
  {
    id: "openrouter",
    name: "OpenRouter",
    subtitle: "聚合多模型统一网关（填入 Key 后自动拉取最新模型目录）",
    category: "other",
    protocol: "chat_completions",
    base_url: "https://openrouter.ai/api/v1",
    models: [],
  },
  {
    id: "openai",
    name: "OpenAI",
    subtitle: "OpenAI 官方 Responses / Chat Completions 端点",
    category: "other",
    protocol: "responses",
    base_url: "https://api.openai.com/v1",
    models: [],
  },
  {
    id: "anthropic",
    name: "Anthropic",
    subtitle: "Claude 原生 Messages API 端点",
    category: "other",
    protocol: "anthropic_messages",
    base_url: "https://api.anthropic.com",
    models: [],
  },
  {
    id: "google-gemini",
    name: "Gemini",
    subtitle: "Google Gemini 原生 Generative Language API 端点 (/v1beta)",
    category: "other",
    protocol: "google_gemini",
    base_url: "https://generativelanguage.googleapis.com/v1beta",
    models: [],
  },
  {
    id: "xai-grok",
    name: "xAI Grok",
    subtitle: "xAI 官方 OpenAI 兼容 API 端点",
    category: "other",
    protocol: "chat_completions",
    base_url: "https://api.x.ai/v1",
    models: [],
  },
] as const;

const LAST_VIEWED_CHANNEL_STORAGE_KEY = "eostage.modelSettings.lastChannelId";

function readLastViewedChannelId(): string | null {
  try {
    const val = window.localStorage.getItem(LAST_VIEWED_CHANNEL_STORAGE_KEY);
    return val && val.trim() ? val.trim() : null;
  } catch {
    return null;
  }
}

function writeLastViewedChannelId(channelId: string): void {
  try {
    if (channelId.trim()) {
      window.localStorage.setItem(LAST_VIEWED_CHANNEL_STORAGE_KEY, channelId.trim());
    }
  } catch {
    // Ignore restricted storage environments
  }
}

function clearLastViewedChannelId(): void {
  try {
    window.localStorage.removeItem(LAST_VIEWED_CHANNEL_STORAGE_KEY);
  } catch {
    // Ignore
  }
}

const __fixture_DEV_CHANNELS: AgentChannelView[] = [
  {
    id: "chan-fixture-demo",
    protocol: "chat_completions",
    base_url: "https://api.openai.com/v1",
    model: "gpt-5.4",
    reasoning_mode: "default",
    key_ready: true,
    is_default: true,
  },
  {
    id: "chan-fixture-legacy",
    protocol: "anthropic_messages",
    base_url: "https://api.anthropic.com",
    model: "claude-sonnet-5-5",
    key_ready: false,
    is_default: false,
  },
];

interface OAuthPkceStartResult {
  session_id: string;
  provider_id: string;
  authorize_url: string;
  redirect_uri: string;
  state: string;
  expires_in_secs: number;
}

interface OAuthPkceCompleteResult {
  status: "pending" | "authorized" | "expired" | "error" | string;
  channel_id?: string | null;
  message?: string | null;
}

interface OAuthDeviceStartResult {
  session_id: string;
  provider_id: string;
  user_code: string;
  verification_uri: string;
  interval_secs: number;
  expires_in_secs: number;
}

interface OAuthDevicePollResult {
  status: "pending" | "authorized" | "expired" | "error" | string;
  channel_id?: string | null;
  account_label?: string | null;
  message?: string | null;
}

const SUPPORTED_OAUTH_PKCE_PROVIDER_IDS = new Set<OAuthPkceProviderPreset["id"]>([
  "openai_codex_pkce",
  "claude_oauth_pkce",
  "github_copilot_device",
  "openrouter_pkce",
]);

const SUPPORTED_OAUTH_PKCE_PROVIDERS: readonly OAuthPkceProviderPreset[] =
  OAUTH_PKCE_PROVIDERS.filter((provider) =>
    SUPPORTED_OAUTH_PKCE_PROVIDER_IDS.has(provider.id)
  );

function hasNativeTauriInvoke(): boolean {
  if (typeof window === "undefined") return false;
  const internals = (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ as
    | { invoke?: unknown }
    | undefined;
  return Boolean(internals && typeof internals.invoke === "function");
}

function extractCommandErrorMessage(err: unknown): string {
  if (typeof err === "string") return err.trim();
  if (err && typeof err === "object") {
    const rec = err as Record<string, unknown>;
    const ctx = typeof rec.context === "string" ? rec.context.trim() : "";
    if (ctx) return ctx;
    const msg = typeof rec.message === "string" ? rec.message.trim() : "";
    if (msg) return msg;
  }
  return "";
}

type QuotaTranslateKey =
  | "settings.channels.quotaResetSoon"
  | "settings.channels.quotaDurationDaysHours"
  | "settings.channels.quotaDurationHoursMinutes"
  | "settings.channels.quotaDurationMinutes"
  | "settings.channels.quotaResetsIn";

function formatQuotaResetRemaining(
  resetAtSecs: number | null | undefined,
  t: (key: QuotaTranslateKey, vars?: Record<string, string | number>) => string
): string | null {
  if (typeof resetAtSecs !== "number" || !Number.isFinite(resetAtSecs) || resetAtSecs <= 0) {
    return null;
  }
  const diffSecs = Math.max(0, Math.floor(resetAtSecs - Date.now() / 1000));
  if (diffSecs < 60) {
    return t("settings.channels.quotaResetSoon");
  }
  const totalMinutes = Math.floor(diffSecs / 60);
  const days = Math.floor(totalMinutes / (60 * 24));
  const hours = Math.floor((totalMinutes % (60 * 24)) / 60);
  const minutes = totalMinutes % 60;
  const duration =
    days > 0
      ? t("settings.channels.quotaDurationDaysHours", { days, hours })
      : hours > 0
        ? t("settings.channels.quotaDurationHoursMinutes", { hours, minutes })
        : t("settings.channels.quotaDurationMinutes", { minutes });
  return t("settings.channels.quotaResetsIn", { duration });
}

async function invokeTauriCommand<T>(
  command: string,
  args?: Record<string, unknown>
): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(command, args);
}

async function openOAuthBrowserUrl(url: string): Promise<{ ok: boolean; error?: string }> {
  if (hasNativeTauriInvoke()) {
    try {
      await invokeTauriCommand("agent_oauth_open_browser", { url });
      return { ok: true };
    } catch {
      // fall through to openExternalUrl
    }
  }
  return openExternalUrl(url);
}

const quietButtonClass =
  "eo-settings-btn-secondary disabled:opacity-50 outline-none";
const inputClass =
  "eo-settings-control-input disabled:cursor-not-allowed disabled:opacity-60";

/**
 * 统一样式的局部下拉选择组件，替代原生 <select>
 */
interface ProtocolSelectProps {
  value: ChannelProtocolValue;
  onChange: (value: ChannelProtocolValue) => void;
  disabled?: boolean;
}

function ProtocolSelect({ value, onChange, disabled }: ProtocolSelectProps) {
  const [isOpen, setIsOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const { t } = useI18n();

  const selectedOption = useMemo(
    () => CHANNEL_PROTOCOLS.find((p) => p.value === value) || CHANNEL_PROTOCOLS[0],
    [value]
  );

  useEffect(() => {
    if (!isOpen) return;
    const handleClickOutside = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setIsOpen(false);
      }
    };
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setIsOpen(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [isOpen]);

  return (
    <div ref={containerRef} className="relative w-full">
      <button
        type="button"
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={isOpen}
        aria-label={t("settings.channels.protocol")}
        onClick={() => !disabled && setIsOpen((prev) => !prev)}
        className={cn(
          "eo-settings-control-input flex items-center justify-between text-[13px] cursor-pointer",
          "hover:bg-sp-hover disabled:cursor-not-allowed disabled:opacity-60"
        )}
      >
        <span className="truncate">{selectedOption.label_zh}</span>
        <ChevronDown
          className={cn(
            "h-3.5 w-3.5 shrink-0 text-sp-text-muted transition-transform duration-fast",
            isOpen && "rotate-180"
          )}
        />
      </button>

      {isOpen && (
        <div
          role="listbox"
          aria-label={t("settings.channels.protocol")}
          className="absolute right-0 top-full z-30 mt-1 max-h-60 w-full overflow-auto rounded-sp-md border border-sp-line bg-sp-card p-1 shadow-modal text-[13px]"
        >
          {CHANNEL_PROTOCOLS.map((protocol) => {
            const isSelected = protocol.value === value;
            return (
              <div
                key={protocol.value}
                role="option"
                aria-selected={isSelected}
                tabIndex={0}
                onClick={() => {
                  onChange(protocol.value);
                  setIsOpen(false);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    onChange(protocol.value);
                    setIsOpen(false);
                  }
                }}
                className={cn(
                  "flex items-center justify-between rounded-sp-sm px-2 py-1 cursor-pointer transition-colors outline-none",
                  isSelected
                    ? "bg-sp-surface-muted font-semibold text-sp-text"
                    : "text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text"
                )}
              >
                <span>{protocol.label_zh}</span>
                {isSelected && <Check className="h-3.5 w-3.5 text-sp-text" />}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

export interface ModelChannelsSettingsTabProps {
  tourOpenAddProviderSignal?: number;
}

export function ModelChannelsSettingsTab({
  tourOpenAddProviderSignal = 0,
}: ModelChannelsSettingsTabProps = {}) {
  const { t } = useI18n();
  const isTauri = isTauriDesktopHost();
  const isDevFixture = Boolean((import.meta.env?.DEV ?? true) && (isFixtureBackend() || !isTauri));

  const [channels, setChannels] = useState<AgentChannelView[]>([]);
  const [channelAuthMetaMap, setChannelAuthMetaMap] = useState<Record<string, ChannelAuthMeta>>({});
  const channelAuthMetaMapRef = useRef<Record<string, ChannelAuthMeta>>({});
  channelAuthMetaMapRef.current = channelAuthMetaMap;
  const [quotaMap, setQuotaMap] = useState<Record<string, SubscriptionChannelQuota>>({});
  const quotaMapRef = useRef<Record<string, SubscriptionChannelQuota>>({});
  quotaMapRef.current = quotaMap;
  const [quotaLoadingId, setQuotaLoadingId] = useState<string | null>(null);
  const [quotaErrorMap, setQuotaErrorMap] = useState<Record<string, string | null>>({});
  const [activeChannelId, setActiveChannelId] = useState<string>("");
  const [isNewDraft, setIsNewDraft] = useState<boolean>(false);
  const [isAddingProviderCatalog, setIsAddingProviderCatalog] = useState<boolean>(false);
  const [providerEnabled, setProviderEnabled] = useState<boolean>(true);
  const [disabledProviders, setDisabledProviders] = useState<Record<string, boolean>>(() =>
    loadDisabledProviders()
  );
  const [showApiKey, setShowApiKey] = useState<boolean>(false);
  const [moreMenuOpen, setMoreMenuOpen] = useState<boolean>(false);
  const [draft, setDraft] = useState<ChannelDraft>(() => newChannelDraft());
  const [apiKeyInput, setApiKeyInput] = useState<string>("");

  const [modelsList, setModelsList] = useState<string[]>([]);
  const [modelsError, setModelsError] = useState<string | null>(null);
  const [fetchDialogOpen, setFetchDialogOpen] = useState(false);
  const [healthMap, setHealthMap] = useState<Record<string, ChannelHealthEntry>>(() =>
    getChannelHealthMap()
  );
  const fetchModelsButtonRef = useRef<HTMLButtonElement>(null);
  const testToastTimerRef = useRef<number | null>(null);
  const noticeTimerRef = useRef<number | null>(null);
  const loadedChannelKeyRef = useRef<string>("");
  const keyLoadGenRef = useRef<number>(0);
  const moreMenuContainerRef = useRef<HTMLDivElement | null>(null);
  const autoSaveTimerRef = useRef<number | null>(null);
  const autoSaveClearTimerRef = useRef<number | null>(null);
  const autoSaveGenRef = useRef<number>(0);
  const lastAutoSavePayloadSigRef = useRef<string | null>(null);
  const isNewDraftRef = useRef<boolean>(isNewDraft);
  isNewDraftRef.current = isNewDraft;
  const draftRef = useRef<ChannelDraft>(draft);
  draftRef.current = draft;
  const apiKeyInputRef = useRef<string>(apiKeyInput);
  apiKeyInputRef.current = apiKeyInput;
  const [autoSaveSync, setAutoSaveSync] = useState<{
    state: OptimisticSyncState;
    error?: string | null;
    retry?: () => void;
  }>({ state: "idle" });

  useEffect(() => {
    return subscribeChannelHealth(() => {
      setHealthMap(getChannelHealthMap());
    });
  }, []);

  useEffect(() => {
    if (!moreMenuOpen) return;
    const handleOutside = (event: MouseEvent) => {
      if (
        moreMenuContainerRef.current &&
        event.target instanceof Node &&
        !moreMenuContainerRef.current.contains(event.target)
      ) {
        setMoreMenuOpen(false);
      }
    };
    const handleEsc = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setMoreMenuOpen(false);
      }
    };
    document.addEventListener("mousedown", handleOutside);
    document.addEventListener("keydown", handleEsc);
    return () => {
      document.removeEventListener("mousedown", handleOutside);
      document.removeEventListener("keydown", handleEsc);
    };
  }, [moreMenuOpen]);

  const clearTestToastTimer = useCallback(() => {
    if (testToastTimerRef.current !== null) {
      window.clearTimeout(testToastTimerRef.current);
      testToastTimerRef.current = null;
    }
  }, []);
  const clearNoticeTimer = useCallback(() => {
    if (noticeTimerRef.current !== null) {
      window.clearTimeout(noticeTimerRef.current);
      noticeTimerRef.current = null;
    }
  }, []);

  const clearAutoSaveTimers = useCallback(() => {
    if (autoSaveTimerRef.current !== null) {
      window.clearTimeout(autoSaveTimerRef.current);
      autoSaveTimerRef.current = null;
    }
    if (autoSaveClearTimerRef.current !== null) {
      window.clearTimeout(autoSaveClearTimerRef.current);
      autoSaveClearTimerRef.current = null;
    }
  }, []);

  const resetAutoSaveState = useCallback(() => {
    clearAutoSaveTimers();
    ++autoSaveGenRef.current;
    lastAutoSavePayloadSigRef.current = null;
    setAutoSaveSync({ state: "idle" });
  }, [clearAutoSaveTimers]);

  useEffect(() => {
    return () => {
      clearTestToastTimer();
      clearNoticeTimer();
      clearAutoSaveTimers();
    };
  }, [clearTestToastTimer, clearNoticeTimer, clearAutoSaveTimers]);

  const probeGeneration = useRef(0);
  const invalidateProbe = useCallback(() => {
    probeGeneration.current++;
    clearTestToastTimer();
    setTesting(false);
    setLoadingModels(false);
    setTestResult(null);
    setTestSnippet(null);
    setTestedModelLabel(null);
    setModelsList([]);
    setModelsError(null);
    setFetchDialogOpen(false);
  }, [clearTestToastTimer]);

  const [initialLoading, setInitialLoading] = useState<boolean>(true);
  const [saving, setSaving] = useState<boolean>(false);
  const [testing, setTesting] = useState<boolean>(false);
  const [testingModelId, setTestingModelId] = useState<string | null>(null);
  const [loadingModels, setLoadingModels] = useState<boolean>(false);
  const [settingDefault, setSettingDefault] = useState<boolean>(false);
  const [deleting, setDeleting] = useState<boolean>(false);

  const { spinClass: providersRefreshSpinClass } = useFullTurnSpin(initialLoading);
  const showChannelsSkeleton = useDelayedFlag(initialLoading);
  const showSavingSpinner = useDelayedFlag(saving);
  const showProviderTestSpinner = useDelayedFlag(testing && testingModelId === null);
  const handledTourSignalRef = useRef(0);

  useEffect(() => {
    if (tourOpenAddProviderSignal <= 0) {
      handledTourSignalRef.current = 0;
      return;
    }
    if (!initialLoading && handledTourSignalRef.current !== tourOpenAddProviderSignal) {
      handledTourSignalRef.current = tourOpenAddProviderSignal;
      setIsAddingProviderCatalog(true);
    }
  }, [tourOpenAddProviderSignal, initialLoading]);

  const [providerSwitchSync, setProviderSwitchSync] = useState<{
    state: OptimisticSyncState;
    error?: string | null;
    retry?: () => void;
  }>({ state: "idle" });
  const providerSwitchGenRef = useRef(0);

  const [testResult, setTestResult] = useState<AgentChannelTest | null>(null);
  const [testSnippet, setTestSnippet] = useState<string | null>(null);
  const [testedModelLabel, setTestedModelLabel] = useState<string | null>(null);
  const [globalNotice, setGlobalNotice] = useState<{
    type: "success" | "error" | "info";
    text: string;
  } | null>(null);

  const [renameDialogOpen, setRenameDialogOpen] = useState<boolean>(false);
  const [renameInput, setRenameInput] = useState<string>("");
  const [renameError, setRenameError] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<boolean>(false);
  const showRenamingSpinner = useDelayedFlag(renaming);

  // 免 API Key 浏览器一键授权 (OAuth PKCE S256) 状态（归入「添加供应商」界面）
  const [pkceProviderId, setPkceProviderId] =
    useState<OAuthPkceProviderPreset["id"]>("openai_codex_pkce");
  const [pkceSession, setPkceSession] = useState<{
    codeVerifier: string;
    codeChallenge: string;
    state: string;
    authorizeUrl: string;
    mode?: "pkce" | "device";
    nativePkceSessionId?: string;
    nativeDeviceSessionId?: string;
    userCode?: string;
    expiresAtMs?: number;
  } | null>(null);
  const [pkceCallbackInput, setPkceCallbackInput] = useState("");
  const [pkceStatusText, setPkceStatusText] = useState<string | null>(null);
  const [pkceStatusOk, setPkceStatusOk] = useState<boolean>(true);
  const [copiedDeviceCode, setCopiedDeviceCode] = useState<boolean>(false);

  // EOStage 官方账号与统一官方渠道状态
  const [selectedOfficialLoggedOut, setSelectedOfficialLoggedOut] = useState<boolean>(false);
  const [officialAccount, setOfficialAccount] = useState<EOStageAccountStatus | null>(null);
  const [officialLoginOpen, setOfficialLoginOpen] = useState<boolean>(false);
  const [officialBillingOpen, setOfficialBillingOpen] = useState<boolean>(false);
  const [syncingOfficialModels, setSyncingOfficialModels] = useState<boolean>(false);

  const helperPollTimerRef = useRef<number | null>(null);
  const pkcePollInFlightRef = useRef<boolean>(false);
  const loadChannelsRef = useRef<(() => Promise<void>) | null>(null);

  const stopHelperPolling = useCallback(() => {
    if (helperPollTimerRef.current !== null) {
      window.clearInterval(helperPollTimerRef.current);
      helperPollTimerRef.current = null;
    }
    pkcePollInFlightRef.current = false;
  }, []);

  useEffect(() => {
    return () => {
      stopHelperPolling();
    };
  }, [stopHelperPolling]);

  const showNoticeRef = useRef<((type: "success" | "error" | "info", text: string) => void) | null>(null);

  const refreshChannelsAfterOAuth = useCallback(async () => {
    if (loadChannelsRef.current) {
      await loadChannelsRef.current();
    }
    notifyChannelsChanged();
  }, []);

  const selectedPkceProvider = useMemo(
    () =>
      SUPPORTED_OAUTH_PKCE_PROVIDERS.find((p) => p.id === pkceProviderId) ??
      SUPPORTED_OAUTH_PKCE_PROVIDERS[0],
    [pkceProviderId]
  );

  const getPresetDisplayName = useCallback(
    (preset: ProviderCatalogPreset): string => {
      if (preset.id === "zhipu-glm") return t("settings.channels.presetZhipuGlm");
      if (preset.id === "aliyun-bailian") return t("settings.channels.presetAliyunBailian");
      return preset.name;
    },
    [t]
  );

  const getPkceProviderCopy = useCallback(
    (provider: OAuthPkceProviderPreset): { name: string; description: string } => {
      switch (provider.id) {
        case "openai_codex_pkce":
          return {
            name: t("settings.channels.pkceProviders.openai_codex_pkce.name"),
            description: t("settings.channels.pkceProviders.openai_codex_pkce.description"),
          };
        case "claude_oauth_pkce":
          return {
            name: t("settings.channels.pkceProviders.claude_oauth_pkce.name"),
            description: t("settings.channels.pkceProviders.claude_oauth_pkce.description"),
          };
        case "openrouter_pkce":
          return {
            name: t("settings.channels.pkceProviders.openrouter_pkce.name"),
            description: t("settings.channels.pkceProviders.openrouter_pkce.description"),
          };
        case "github_copilot_device":
          return {
            name: t("settings.channels.pkceProviders.github_copilot_device.name"),
            description: t("settings.channels.pkceProviders.github_copilot_device.description"),
          };
        default:
          return { name: provider.name_zh, description: provider.description_zh };
      }
    },
    [t]
  );

  const prepareNewPkceChannelDraft = useCallback(
    (provider: OAuthPkceProviderPreset) => {
      const baseId = provider.id.replace(/_/g, "-");
      let nextId = baseId;
      let suffix = 2;
      while (channels.some((c) => c.id === nextId)) {
        nextId = `${baseId}-${suffix}`;
        suffix += 1;
      }
      const presetModel = provider.default_model;
      const nextDraft: ChannelDraft = {
        id: nextId,
        protocol: provider.protocol,
        base_url: provider.base_url,
        model: presetModel || "",
        models: presetModel
          ? [{ id: presetModel, input_types: ["text"], reasoning_levels: [] }]
          : [],
        reasoning_mode: "default",
        key_ready: false,
        is_default: channels.length === 0,
        apiKeyInput: "",
      };
      resetAutoSaveState();
      ++keyLoadGenRef.current;
      loadedChannelKeyRef.current = "";
      setShowApiKey(false);
      setSelectedOfficialLoggedOut(false);
      setIsNewDraft(true);
      setActiveChannelId(nextId);
      setDraft(nextDraft);
      initialDraftRef.current = { draft: nextDraft, apiKey: "" };
      setApiKeyInput("");
      setTestResult(null);
      setTestSnippet(null);
      setTestedModelLabel(null);
      return nextDraft;
    },
    [channels, resetAutoSaveState]
  );

  const startPkceFlowForProvider = useCallback(
    (targetProvider: OAuthPkceProviderPreset) => {
      stopHelperPolling();
      setCopiedDeviceCode(false);

      if (isTauri && hasNativeTauriInvoke()) {
        // GitHub Copilot 走原生 Rust RFC 8628 设备码授权
        if (targetProvider.id === "github_copilot_device") {
          invalidateProbe();
          void (async () => {
            try {
              const res = await invokeTauriCommand<OAuthDeviceStartResult>(
                "agent_oauth_device_start",
                { providerId: "github_copilot" }
              );
              setPkceSession({
                mode: "device",
                codeVerifier: "",
                codeChallenge: res.user_code,
                userCode: res.user_code,
                state: res.session_id,
                authorizeUrl: res.verification_uri,
                nativeDeviceSessionId: res.session_id,
                expiresAtMs: Date.now() + (res.expires_in_secs || 900) * 1000,
              });
              setPkceStatusOk(true);
              setPkceStatusText(t("settings.channels.deviceCopilotStarted"));
              void openOAuthBrowserUrl(res.verification_uri);
              const pollMs = Math.max(res.interval_secs || 5, 5) * 1000;
              helperPollTimerRef.current = window.setInterval(() => {
                if (pkcePollInFlightRef.current) return;
                pkcePollInFlightRef.current = true;
                void (async () => {
                  try {
                    const done = await invokeTauriCommand<OAuthDevicePollResult>(
                      "agent_oauth_device_poll",
                      { sessionId: res.session_id }
                    );
                    if (done.status === "authorized") {
                      stopHelperPolling();
                      setPkceStatusOk(true);
                      setPkceStatusText(
                        done.message ||
                          t("settings.channels.pkceAuthorizedChannel", {
                            channel: done.channel_id || "github-copilot",
                          })
                      );
                      showNoticeRef.current?.(
                        "success",
                        done.message || t("settings.channels.pkceSavedToast")
                      );
                      if (done.channel_id) {
                        setQuotaMap((prev) => {
                          const next = { ...prev };
                          delete next[done.channel_id!];
                          return next;
                        });
                      }
                      await refreshChannelsAfterOAuth();
                    } else if (done.status === "expired" || done.status === "error") {
                      stopHelperPolling();
                      setPkceStatusOk(false);
                      setPkceStatusText(done.message || t("settings.channels.pkceFailedOrExpired"));
                    }
                  } catch (err: unknown) {
                    stopHelperPolling();
                    setPkceStatusOk(false);
                    setPkceStatusText(
                      extractCommandErrorMessage(err) || t("settings.channels.pkceFailedOrExpired")
                    );
                  } finally {
                    pkcePollInFlightRef.current = false;
                  }
                })();
              }, pollMs);
            } catch (err: unknown) {
              setPkceStatusOk(false);
              setPkceStatusText(
                extractCommandErrorMessage(err) || t("settings.channels.deviceStartFailed")
              );
            }
          })();
          return;
        }

        // OpenAI Codex / ChatGPT、Claude Pro/Max 与 OpenRouter 统一走原生 Rust S256 PKCE + 本地回环监听 + 后端换取令牌
        if (
          targetProvider.id === "openai_codex_pkce" ||
          targetProvider.id === "claude_oauth_pkce" ||
          targetProvider.id === "openrouter_pkce"
        ) {
          const nativeProviderId =
            targetProvider.id === "openai_codex_pkce"
              ? "openai_codex"
              : targetProvider.id === "claude_oauth_pkce"
                ? "claude_code"
                : "openrouter";
          invalidateProbe();
          void (async () => {
            try {
              const res = await invokeTauriCommand<OAuthPkceStartResult>("agent_oauth_pkce_start", {
                providerId: nativeProviderId,
              });
              setPkceSession({
                mode: "pkce",
                codeVerifier: "",
                codeChallenge: "S256 (Rust Loopback)",
                state: res.state,
                authorizeUrl: res.authorize_url,
                nativePkceSessionId: res.session_id,
                expiresAtMs: Date.now() + (res.expires_in_secs || 600) * 1000,
              });
              setPkceStatusOk(true);
              setPkceStatusText(
                nativeProviderId === "openai_codex"
                  ? t("settings.channels.pkceNativeCodexStarted")
                  : nativeProviderId === "claude_code"
                    ? t("settings.channels.pkceNativeClaudeStarted")
                    : t("settings.channels.pkceNativeOpenRouterStarted")
              );
              void openOAuthBrowserUrl(res.authorize_url);
              helperPollTimerRef.current = window.setInterval(() => {
                if (pkcePollInFlightRef.current) return;
                pkcePollInFlightRef.current = true;
                void (async () => {
                  try {
                    const done = await invokeTauriCommand<OAuthPkceCompleteResult>(
                      "agent_oauth_pkce_complete",
                      { sessionId: res.session_id, callbackInput: null }
                    );
                    if (done.status === "authorized") {
                      stopHelperPolling();
                      setPkceStatusOk(true);
                      setPkceStatusText(
                        done.message ||
                          t("settings.channels.pkceAuthorizedChannel", {
                            channel: done.channel_id || nativeProviderId,
                          })
                      );
                      showNoticeRef.current?.(
                        "success",
                        done.message || t("settings.channels.pkceSavedToast")
                      );
                      if (done.channel_id) {
                        setQuotaMap((prev) => {
                          const next = { ...prev };
                          delete next[done.channel_id!];
                          return next;
                        });
                      }
                      await refreshChannelsAfterOAuth();
                    } else if (done.status === "expired" || done.status === "error") {
                      stopHelperPolling();
                      setPkceStatusOk(false);
                      setPkceStatusText(done.message || t("settings.channels.pkceFailedOrExpired"));
                    }
                  } catch (err: unknown) {
                    stopHelperPolling();
                    setPkceStatusOk(false);
                    setPkceStatusText(
                      extractCommandErrorMessage(err) || t("settings.channels.pkceFailedOrExpired")
                    );
                  } finally {
                    pkcePollInFlightRef.current = false;
                  }
                })();
              }, 1000);
            } catch (err: unknown) {
              setPkceStatusOk(false);
              setPkceStatusText(
                extractCommandErrorMessage(err) || t("settings.channels.pkceStartFailed")
              );
            }
          })();
          return;
        }
      }

      if (targetProvider.id === "github_copilot_device") {
        const previewCode = "DEMO-CODE";
        const previewState = generateOAuthState();
        setPkceSession({
          mode: "device",
          codeVerifier: "",
          codeChallenge: previewCode,
          userCode: previewCode,
          state: previewState,
          authorizeUrl: targetProvider.authorize_url || "https://github.com/login/device",
        });
        invalidateProbe();
        prepareNewPkceChannelDraft(targetProvider);
        setPkceStatusOk(true);
        setPkceStatusText(t("settings.channels.deviceCopilotStarted"));
        return;
      }

      const pair = generatePkcePair();
      const state = generateOAuthState();
      const authorizeUrl = buildOAuthAuthorizationUrl({
        provider: targetProvider,
        codeChallenge: pair.codeChallenge,
        state,
      });
      setPkceSession({
        mode: "pkce",
        codeVerifier: pair.codeVerifier,
        codeChallenge: pair.codeChallenge,
        state,
        authorizeUrl,
      });
      invalidateProbe();
      const nextDraft = prepareNewPkceChannelDraft(targetProvider);
      setPkceStatusOk(true);
      setPkceStatusText(
        t("settings.channels.pkceLinkGenerated", {
          id: nextDraft.id,
          challenge: pair.codeChallenge.slice(0, 12),
        })
      );
    },
    [
      invalidateProbe,
      isTauri,
      prepareNewPkceChannelDraft,
      refreshChannelsAfterOAuth,
      stopHelperPolling,
      t,
    ]
  );

  const handleGeneratePkceAuthUrl = useCallback(() => {
    startPkceFlowForProvider(selectedPkceProvider);
  }, [selectedPkceProvider, startPkceFlowForProvider]);

  const verifyPkceCallbackValue = useCallback(
    (rawInput: string, options?: { autoOpenChannelConfig?: boolean }) => {
      const raw = rawInput.trim();
      if (!raw) {
        setPkceStatusOk(false);
        setPkceStatusText(t("settings.channels.pkcePastePrompt"));
        return;
      }

      const bearerTrimmed = raw.replace(/^Bearer\s+/i, "").trim();
      const looksLikeJwt =
        !/\s/.test(bearerTrimmed) &&
        !bearerTrimmed.includes("://") &&
        !bearerTrimmed.includes("?") &&
        /^[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+$/.test(bearerTrimmed);
      if (raw.startsWith("{") || looksLikeJwt) {
        try {
          const parsed = raw.startsWith("{")
            ? (JSON.parse(raw) as Record<string, unknown>)
            : raw;
          const accountId = extractChatGptAccountId(parsed);
          const tokenVal =
            typeof parsed === "string"
              ? parsed.replace(/^Bearer\s+/i, "").trim()
              : typeof parsed.access_token === "string"
                ? parsed.access_token.replace(/^Bearer\s+/i, "").trim()
                : raw;
          const jwtPayload = decodeJwtPayload(tokenVal);
          const rawExp =
            typeof parsed === "object" && parsed !== null && "expires_at" in parsed
              ? (parsed.expires_at as number | string | null | undefined)
              : (jwtPayload?.exp as number | string | null | undefined);
          const expiringWarning =
            rawExp !== undefined && isOAuthTokenExpiringSoon(rawExp)
              ? t("settings.channels.pkceExpiringSuffix")
              : "";
          const targetJwtDraft =
            !isNewDraft || draft.base_url !== selectedPkceProvider.base_url
              ? prepareNewPkceChannelDraft(selectedPkceProvider)
              : draft;
          initialDraftRef.current = { draft: targetJwtDraft, apiKey: tokenVal };
          setApiKeyInput(tokenVal);
          setPkceStatusOk(true);
          setPkceStatusText(
            accountId
              ? t("settings.channels.pkceJwtAccountParsed", {
                  accountId,
                  warning: expiringWarning,
                })
              : t("settings.channels.pkceTokenParsed", {
                  warning: expiringWarning,
                })
          );
          if (options?.autoOpenChannelConfig) {
            setIsAddingProviderCatalog(false);
          }
          return;
        } catch {
          // 继续按回调 URL 解析
        }
      }

      if (isTauri && hasNativeTauriInvoke() && pkceSession?.nativePkceSessionId) {
        const nativeSessionId = pkceSession.nativePkceSessionId;
        void (async () => {
          try {
            const res = await invokeTauriCommand<OAuthPkceCompleteResult>(
              "agent_oauth_pkce_complete",
              { sessionId: nativeSessionId, callbackInput: raw }
            );
            if (res.status === "authorized") {
              stopHelperPolling();
              setPkceStatusOk(true);
              setPkceStatusText(
                res.message ||
                  t("settings.channels.pkceExchangedSaved", {
                    channel: res.channel_id || "oauth",
                  })
              );
              showNoticeRef.current?.(
                "success",
                res.message || t("settings.channels.pkceBrowserSavedToast")
              );
              if (res.channel_id) {
                setQuotaMap((prev) => {
                  const next = { ...prev };
                  delete next[res.channel_id!];
                  return next;
                });
              }
              await refreshChannelsAfterOAuth();
            } else if (res.status === "pending") {
              setPkceStatusOk(true);
              setPkceStatusText(res.message || t("settings.channels.pkceNativeCodexStarted"));
            } else {
              stopHelperPolling();
              setPkceStatusOk(false);
              setPkceStatusText(res.message || t("settings.channels.pkceCallbackIncomplete"));
            }
          } catch (err: unknown) {
            stopHelperPolling();
            setPkceStatusOk(false);
            setPkceStatusText(
              extractCommandErrorMessage(err) || t("settings.channels.pkceCallbackFailed")
            );
          }
        })();
        return;
      }

      const res = extractOAuthAuthorizationCode(raw, pkceSession?.state);
      if (!res.ok || !res.code) {
        setPkceStatusOk(false);
        setPkceStatusText(res.error || t("settings.channels.pkceCallbackParseFailed"));
        return;
      }

      const targetDraft =
        !isNewDraft || draft.base_url !== selectedPkceProvider.base_url
          ? prepareNewPkceChannelDraft(selectedPkceProvider)
          : draft;

      // 绝不将一次性 authorization code 写入 apiKeyInput；仅在成功兑换为真实访问令牌后写入
      setPkceStatusOk(true);
      setPkceStatusText(
        t("settings.channels.pkceCodeExtracted", {
          code: res.code.slice(0, 10),
        })
      );

      if (pkceSession?.codeVerifier && selectedPkceProvider.token_url && typeof fetch === "function") {
        const body = new URLSearchParams({
          grant_type: "authorization_code",
          client_id: selectedPkceProvider.client_id,
          code: res.code,
          redirect_uri: selectedPkceProvider.redirect_uri,
          code_verifier: pkceSession.codeVerifier,
        });
        void fetch(selectedPkceProvider.token_url, {
          method: "POST",
          headers: {
            "Content-Type": "application/x-www-form-urlencoded",
            Accept: "application/json",
          },
          body: body.toString(),
        })
          .then(async (resp) => {
            if (!resp.ok) return null;
            return (await resp.json()) as Record<string, unknown>;
          })
          .then((tokenData) => {
            const exchangedToken =
              tokenData && typeof tokenData.access_token === "string"
                ? tokenData.access_token.trim()
                : tokenData && typeof tokenData.key === "string"
                  ? tokenData.key.trim()
                  : "";
            if (exchangedToken) {
              setApiKeyInput(exchangedToken);
              const accountId = tokenData ? extractChatGptAccountId(tokenData) : null;
              setPkceStatusOk(true);
              setPkceStatusText(
                accountId
                  ? t("settings.channels.pkceExchangeAccountOk", {
                      id: targetDraft.id,
                      accountId,
                    })
                  : t("settings.channels.pkceExchangeOk", { id: targetDraft.id })
              );
              if (options?.autoOpenChannelConfig) {
                setIsAddingProviderCatalog(false);
              }
            } else {
              setPkceStatusOk(false);
              setPkceStatusText(t("settings.channels.pkceExchangeMissingToken"));
            }
          })
          .catch(() => {
            setPkceStatusOk(false);
            setPkceStatusText(t("settings.channels.pkceExchangeCorsRestricted"));
          });
      } else {
        setPkceStatusOk(false);
        setPkceStatusText(t("settings.channels.pkceMissingSession"));
      }
    },
    [
      draft,
      isNewDraft,
      isTauri,
      pkceSession,
      prepareNewPkceChannelDraft,
      refreshChannelsAfterOAuth,
      selectedPkceProvider,
      stopHelperPolling,
      t,
    ]
  );

  const handleVerifyPkceCallback = useCallback(() => {
    verifyPkceCallbackValue(pkceCallbackInput);
  }, [pkceCallbackInput, verifyPkceCallbackValue]);

  useEffect(() => {
    if (typeof window === "undefined") return;
    const onOAuthCallback = (event: Event) => {
      const custom = event as CustomEvent<{ url?: string }>;
      const callbackUrl = custom.detail?.url?.trim();
      if (!callbackUrl) return;
      setPkceCallbackInput(callbackUrl);
      verifyPkceCallbackValue(callbackUrl, { autoOpenChannelConfig: true });
    };
    window.addEventListener("eostage:oauth-callback", onOAuthCallback);
    return () => {
      window.removeEventListener("eostage:oauth-callback", onOAuthCallback);
    };
  }, [verifyPkceCallbackValue]);

  const showNotice = useCallback(
    (type: "success" | "error" | "info", text: string) => {
      clearNoticeTimer();
      setGlobalNotice({ type, text });
      const durationMs = type === "error" ? 7000 : 4500;
      noticeTimerRef.current = window.setTimeout(() => {
        noticeTimerRef.current = null;
        setGlobalNotice(null);
      }, durationMs);
      return () => clearNoticeTimer();
    },
    [clearNoticeTimer]
  );
  showNoticeRef.current = showNotice;

  const showTestHud = useCallback(
    (result: AgentChannelTest, snippet: string | null, modelLabel: string | null) => {
      clearTestToastTimer();
      setTestResult(result);
      setTestSnippet(snippet);
      setTestedModelLabel(modelLabel);
      const durationMs = result.ok ? 4500 : 7000;
      testToastTimerRef.current = window.setTimeout(() => {
        testToastTimerRef.current = null;
        setTestResult(null);
        setTestSnippet(null);
        setTestedModelLabel(null);
      }, durationMs);
    },
    [clearTestToastTimer]
  );

  const initialDraftRef = useRef<{ draft: ChannelDraft; apiKey: string } | null>(null);
  const [pendingUnsavedAction, setPendingUnsavedAction] = useState<(() => void) | null>(null);
  const unsavedDialogReturnFocusRef = useRef<HTMLElement | null>(null);

  const hasUnsavedChannelEdits = useMemo(() => {
    if (!initialDraftRef.current) return false;
    if (apiKeyInput !== initialDraftRef.current.apiKey) return true;
    return JSON.stringify(draft) !== JSON.stringify(initialDraftRef.current.draft);
  }, [draft, apiKeyInput]);

  const closeUnsavedDialog = useCallback(() => {
    setPendingUnsavedAction(null);
    const prev = unsavedDialogReturnFocusRef.current;
    if (prev && prev.isConnected) {
      prev.focus();
    }
  }, []);

  useEffect(() => {
    if (!pendingUnsavedAction || typeof document === "undefined") return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        closeUnsavedDialog();
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [pendingUnsavedAction, closeUnsavedDialog]);

  useEffect(() => {
    if (!renameDialogOpen || typeof document === "undefined") return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !renaming) {
        e.preventDefault();
        setRenameDialogOpen(false);
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [renameDialogOpen, renaming]);

  const checkUnsavedAndProceed = useCallback(
    (action: () => void) => {
      if (isNewDraft && hasUnsavedChannelEdits) {
        unsavedDialogReturnFocusRef.current =
          typeof document !== "undefined" && document.activeElement instanceof HTMLElement
            ? document.activeElement
            : null;
        setPendingUnsavedAction(() => action);
        return;
      }
      action();
    },
    [isNewDraft, hasUnsavedChannelEdits]
  );

  const loadSavedChannelKey = useCallback(
    (channelId: string, keyReady: boolean) => {
      const gen = ++keyLoadGenRef.current;
      loadedChannelKeyRef.current = "";
      if (!isTauri || !keyReady || !channelId || isOfficialChannelId(channelId)) {
        return;
      }
      try {
        const bridge = getDesktopShellBridge();
        const readKeyFn = bridge.credentials?.readAgentChannelKey;
        if (typeof readKeyFn !== "function") return;
        void readKeyFn
          .call(bridge.credentials, channelId)
          .then((res) => {
            if (gen !== keyLoadGenRef.current) return;
            const savedKey = res?.ok && typeof res.value === "string" ? res.value : "";
            loadedChannelKeyRef.current = savedKey;
            if (initialDraftRef.current) {
              initialDraftRef.current.apiKey = savedKey;
            }
            setApiKeyInput(savedKey);
          })
          .catch(() => {
            // Keep input empty if credential read fails
          });
      } catch {
        // Ignore bridge lookup errors in non-desktop environments
      }
    },
    [isTauri]
  );

  const fetchChannelQuota = useCallback(
    async (channelId: string) => {
      if (!channelId || !isTauri || !hasNativeTauriInvoke()) {
        return;
      }
      setQuotaLoadingId(channelId);
      setQuotaErrorMap((prev) => ({ ...prev, [channelId]: null }));
      try {
        const res = await invokeTauriCommand<SubscriptionChannelQuota>(
          "agent_oauth_channel_quota",
          { channelId }
        );
        setQuotaMap((prev) => ({ ...prev, [channelId]: res }));
      } catch (err: unknown) {
        setQuotaErrorMap((prev) => ({
          ...prev,
          [channelId]:
            extractCommandErrorMessage(err) || t("settings.channels.quotaUnavailable"),
        }));
      } finally {
        setQuotaLoadingId((curr) => (curr === channelId ? null : curr));
      }
    },
    [isTauri, t]
  );

  const selectChannel = useCallback(
    (
      channel: AgentChannelView,
      options?: { preserveLoadedKey?: string; skipRemember?: boolean }
    ) => {
      resetAutoSaveState();
      invalidateProbe();
      const d = channelDraftFromView(channel);
      const authKind = resolveChannelAuthKind(channel, channelAuthMetaMapRef.current);
      setShowApiKey(false);
      setSelectedOfficialLoggedOut(false);
      setActiveChannelId(channel.id);
      if (!options?.skipRemember) {
        writeLastViewedChannelId(
          isOfficialChannelId(channel.id) ? OFFICIAL_CHANNEL_ID : channel.id
        );
      }
      setProviderEnabled(loadDisabledProviders()[channel.id] !== true);
      setDisabledProviders(loadDisabledProviders());
      setProviderSwitchSync({ state: "idle" });
      setIsNewDraft(false);
      setIsAddingProviderCatalog(false);
      setDraft(d);
      if (authKind === "subscription") {
        ++keyLoadGenRef.current;
        loadedChannelKeyRef.current = "";
        initialDraftRef.current = { draft: d, apiKey: "" };
        setApiKeyInput("");
        if (isTauri && hasNativeTauriInvoke() && channel.key_ready && !quotaMapRef.current[channel.id]) {
          void fetchChannelQuota(channel.id);
        }
      } else if (options?.preserveLoadedKey !== undefined) {
        ++keyLoadGenRef.current;
        loadedChannelKeyRef.current = options.preserveLoadedKey;
        initialDraftRef.current = { draft: d, apiKey: options.preserveLoadedKey };
        setApiKeyInput(options.preserveLoadedKey);
      } else {
        initialDraftRef.current = { draft: d, apiKey: "" };
        setApiKeyInput("");
        loadSavedChannelKey(channel.id, Boolean(channel.key_ready));
      }
      setTestResult(null);
      setTestSnippet(null);
      setTestedModelLabel(null);
      setModelsList([]);
      setModelsError(null);
      setFetchDialogOpen(false);
    },
    [fetchChannelQuota, invalidateProbe, isTauri, loadSavedChannelKey, resetAutoSaveState]
  );

  const handleSelectChannel = useCallback(
    (channel: AgentChannelView) => {
      if (
        activeChannelId === channel.id &&
        !isNewDraft &&
        !isAddingProviderCatalog &&
        !selectedOfficialLoggedOut
      ) {
        return;
      }
      checkUnsavedAndProceed(() => selectChannel(channel));
    },
    [
      activeChannelId,
      isNewDraft,
      isAddingProviderCatalog,
      selectedOfficialLoggedOut,
      checkUnsavedAndProceed,
      selectChannel,
    ]
  );

  const handleApplyProviderPreset = useCallback(
    (preset: ProviderCatalogPreset) => {
      checkUnsavedAndProceed(() => {
        invalidateProbe();
        setSelectedOfficialLoggedOut(false);
        const existing = channels.find((c) => c.id === preset.id);
        if (existing) {
          selectChannel(existing);
          return;
        }
        const resolvedModels = preset.models.map((m) => ({ ...m }));
        const presetDraft: ChannelDraft = {
          id: preset.id,
          protocol: preset.protocol,
          base_url: preset.base_url,
          model: resolvedModels[0]?.id || "",
          models: resolvedModels,
          reasoning_mode: "default",
          key_ready: false,
          is_default: channels.length === 0,
          apiKeyInput: "",
        };
        resetAutoSaveState();
        ++keyLoadGenRef.current;
        loadedChannelKeyRef.current = "";
        setShowApiKey(false);
        setIsNewDraft(true);
        setIsAddingProviderCatalog(false);
        setActiveChannelId(presetDraft.id);
        setDraft(presetDraft);
        initialDraftRef.current = { draft: presetDraft, apiKey: "" };
        setApiKeyInput("");
        setTestResult(null);
        setTestSnippet(null);
        setTestedModelLabel(null);
        setModelsList([]);
        setModelsError(null);
        setFetchDialogOpen(false);
      });
    },
    [channels, checkUnsavedAndProceed, invalidateProbe, resetAutoSaveState, selectChannel]
  );

  const loadChannels = useCallback(async () => {
    if (!isTauri) {
      if (
        (import.meta.env?.DEV ?? true) &&
        typeof window !== "undefined" &&
        window.location.hash.includes("channels-view=empty")
      ) {
        setChannels([]);
        setActiveChannelId("");
        setIsNewDraft(false);
      } else if (isDevFixture) {
        setChannels(__fixture_DEV_CHANNELS);
        if (__fixture_DEV_CHANNELS.length > 0) selectChannel(__fixture_DEV_CHANNELS[0]);
      }
      setInitialLoading(false);
      return;
    }

    try {
      setInitialLoading(true);
      const bridge = getDesktopShellBridge();
      if (!bridge.agent) throw new Error(t("settings.channels.bridgeUnavailable"));
      const [list, acct, , authMetaList] = await Promise.all([
        bridge.agent.channelsGet(),
        eostageAccountStatus(false).catch(() => null),
        hydrateDisabledPreferencesFromBackend(),
        hasNativeTauriInvoke()
          ? invokeTauriCommand<ChannelAuthMeta[]>("agent_oauth_channels_meta").catch(() => [])
          : Promise.resolve([] as ChannelAuthMeta[]),
      ]);
      const nextAuthMetaMap: Record<string, ChannelAuthMeta> = {};
      if (Array.isArray(authMetaList)) {
        for (const item of authMetaList) {
          if (item?.channel_id) {
            nextAuthMetaMap[item.channel_id] = item;
          }
        }
      }
      channelAuthMetaMapRef.current = nextAuthMetaMap;
      setChannelAuthMetaMap(nextAuthMetaMap);
      setOfficialAccount(acct);
      setChannels(list);
      setDisabledProviders(loadDisabledProviders());
      const official = list.find((channel) => isOfficialChannelId(channel.id)) ?? null;
      const rememberedId = readLastViewedChannelId();
      const rememberedCustom =
        rememberedId && !isOfficialChannelId(rememberedId)
          ? list.find((channel) => channel.id === rememberedId) ?? null
          : null;
      if (rememberedId && isOfficialChannelId(rememberedId)) {
        if (official) {
          selectChannel(official, { skipRemember: true });
        } else {
          setSelectedOfficialLoggedOut(true);
          setActiveChannelId("");
          setIsNewDraft(false);
        }
      } else if (rememberedCustom) {
        selectChannel(rememberedCustom, { skipRemember: true });
      } else if (official) {
        selectChannel(official, { skipRemember: true });
      } else {
        setSelectedOfficialLoggedOut(list.length > 0);
        setActiveChannelId("");
        setIsNewDraft(false);
      }
    } catch (err: any) {
      showNotice(
        "error",
        t("settings.channels.loadFailed", {
          error: err?.message || t("settings.channels.backendError"),
        })
      );
    } finally {
      setInitialLoading(false);
    }
  }, [isTauri, isDevFixture, selectChannel, showNotice, t]);

  loadChannelsRef.current = loadChannels;

  useEffect(() => {
    void loadChannels();
  }, [loadChannels]);

  useEffect(() => {
    const syncOfficial = () => {
      void eostageAccountStatus(false)
        .then(setOfficialAccount)
        .catch(() => {});
      void loadChannels();
    };
    return subscribeEOStageAccountChanged(syncOfficial);
  }, [loadChannels]);

  const handleStartCreate = () => {
    checkUnsavedAndProceed(() => {
      resetAutoSaveState();
      invalidateProbe();
      const newChannel = newChannelDraft();
      ++keyLoadGenRef.current;
      loadedChannelKeyRef.current = "";
      setShowApiKey(false);
      setSelectedOfficialLoggedOut(false);
      setIsNewDraft(true);
      setIsAddingProviderCatalog(false);
      setActiveChannelId(newChannel.id);
      setDraft(newChannel);
      initialDraftRef.current = { draft: newChannel, apiKey: "" };
      setApiKeyInput("");
      setTestResult(null);
      setTestSnippet(null);
      setTestedModelLabel(null);
      setModelsList([]);
      setModelsError(null);
      setFetchDialogOpen(false);
    });
  };

  const handleCancelDraft = () => {
    checkUnsavedAndProceed(() => {
      resetAutoSaveState();
      invalidateProbe();
      setSelectedOfficialLoggedOut(false);
      setIsNewDraft(false);
      setIsAddingProviderCatalog(false);
      if (channels.length > 0) {
        const next =
          channels.find((c) => isOfficialChannelId(c.id)) ||
          channels[0];
        selectChannel(next);
      } else {
        setActiveChannelId("");
      }
    });
  };

  const prepareChannelSavePayload = useCallback(
    (targetDraft: ChannelDraft, targetKey: string, isDraftChannel: boolean) => {
      const isOfficial = !isDraftChannel && isOfficialChannelId(targetDraft.id);
      const isUnchangedLoadedKey =
        isOfficial ||
        (!isDraftChannel &&
          Boolean(targetDraft.key_ready) &&
          loadedChannelKeyRef.current.length > 0 &&
          targetKey === loadedChannelKeyRef.current);
      const canAllowEmptyModel =
        targetDraft.models.length === 0 &&
        !targetDraft.model.trim() &&
        (Boolean(targetDraft.key_ready) ||
          targetKey.trim().length > 0 ||
          targetDraft.id.includes("oauth") ||
          targetDraft.id.includes("cli") ||
          targetDraft.id.includes("pkce") ||
          targetDraft.id.includes("codex") ||
          targetDraft.id.includes("gemini"));
      const payload = buildChannelSavePayload({
        ...targetDraft,
        allowEmptyModel: canAllowEmptyModel,
        apiKeyInput: isUnchangedLoadedKey ? "" : targetKey,
      });
      if (isUnchangedLoadedKey) {
        payload.api_key = null;
      }
      return payload;
    },
    []
  );

  const performAutoSave = useCallback(
    async (targetDraft: ChannelDraft, targetKey: string) => {
      if (!isTauri || isNewDraftRef.current || !targetDraft.id.trim()) return;
      if (!isOfficialChannelId(targetDraft.id) && !targetDraft.base_url.trim()) return;
      if (targetDraft.models.some((m) => !m.id.trim())) return;
      const payload = prepareChannelSavePayload(targetDraft, targetKey, false);
      const payloadSig = JSON.stringify(payload);
      if (lastAutoSavePayloadSigRef.current === payloadSig) return;
      clearAutoSaveTimers();
      lastAutoSavePayloadSigRef.current = payloadSig;
      const gen = ++autoSaveGenRef.current;
      setAutoSaveSync({ state: "saving" });
      try {
        const bridge = getDesktopShellBridge();
        if (!bridge.agent) throw new Error(t("settings.channels.bridgeUnavailable"));
        const refreshed = await bridge.agent.channelsSave(payload);
        if (gen !== autoSaveGenRef.current) return;
        clearChannelHealth(payload.id);
        setChannels(refreshed);
        if (targetKey.trim()) {
          loadedChannelKeyRef.current = targetKey.trim();
        }
        const savedChannel = refreshed.find((channel) => channel.id === payload.id);
        if (savedChannel && draftRef.current.id === payload.id) {
          const syncedDraft = channelDraftFromView(savedChannel);
          initialDraftRef.current = { draft: syncedDraft, apiKey: loadedChannelKeyRef.current };
          lastAutoSavePayloadSigRef.current = JSON.stringify(
            prepareChannelSavePayload(syncedDraft, loadedChannelKeyRef.current, false)
          );
          setDraft((prev) =>
            prev.id === payload.id ? { ...prev, key_ready: syncedDraft.key_ready } : prev
          );
        }
        notifyChannelsChanged();
        setAutoSaveSync({ state: "saved" });
        autoSaveClearTimerRef.current = window.setTimeout(() => {
          if (gen === autoSaveGenRef.current) {
            setAutoSaveSync((cur) => (cur.state === "saved" ? { state: "idle" } : cur));
          }
        }, 1800);
      } catch (err: any) {
        if (gen !== autoSaveGenRef.current) return;
        lastAutoSavePayloadSigRef.current = null;
        setAutoSaveSync({
          state: "error",
          error: err?.message || t("settings.channels.saveFailed"),
          retry: () => {
            void performAutoSave(draftRef.current, apiKeyInputRef.current);
          },
        });
      }
    },
    [clearAutoSaveTimers, isTauri, prepareChannelSavePayload, t]
  );

  const scheduleAutoSave = useCallback(
    (targetDraft: ChannelDraft, targetKey: string, delayMs = 500) => {
      if (isNewDraftRef.current || !targetDraft.id.trim()) return;
      if (targetDraft.models.some((m) => !m.id.trim())) return;
      clearAutoSaveTimers();
      if (delayMs <= 0) {
        void performAutoSave(targetDraft, targetKey);
        return;
      }
      autoSaveTimerRef.current = window.setTimeout(() => {
        autoSaveTimerRef.current = null;
        void performAutoSave(targetDraft, targetKey);
      }, delayMs);
    },
    [clearAutoSaveTimers, performAutoSave]
  );

  const updateModels = (models: AgentChannelModel[]) => {
    invalidateProbe();
    const nextDraft: ChannelDraft = {
      ...draftRef.current,
      models,
      model: models[0]?.id.trim() ?? "",
    };
    draftRef.current = nextDraft;
    setDraft(nextDraft);
    if (!isNewDraftRef.current && !models.some((m) => !m.id.trim())) {
      scheduleAutoSave(nextDraft, apiKeyInputRef.current, 0);
    }
  };

  const handleSaveConfig = async () => {
    if (!isTauri) {
      showNotice("info", t("settings.channels.demoWriteUnsupported"));
      return;
    }
    try {
      clearAutoSaveTimers();
      ++autoSaveGenRef.current;
      setSaving(true);
      const payload = prepareChannelSavePayload(draft, apiKeyInput, isNewDraft);
      const bridge = getDesktopShellBridge();
      if (!bridge.agent) throw new Error(t("settings.channels.bridgeUnavailable"));
      const refreshed = await bridge.agent.channelsSave(payload);
      clearChannelHealth(payload.id);
      setChannels(refreshed);
      const effectiveSavedKey = apiKeyInput.trim() ? apiKeyInput.trim() : loadedChannelKeyRef.current;
      const savedChannel = refreshed.find((channel) => channel.id === payload.id);
      if (savedChannel) {
        selectChannel(
          savedChannel,
          effectiveSavedKey ? { preserveLoadedKey: effectiveSavedKey } : undefined
        );
      } else {
        setIsNewDraft(false);
      }
      notifyChannelsChanged();
      showNotice("success", t("settings.channels.saveSucceeded"));
    } catch (err: any) {
      showNotice("error", err?.message || t("settings.channels.saveFailed"));
    } finally {
      setSaving(false);
    }
  };

  const handleSetDefault = async () => {
    if (!draft.id || isNewDraft) return;
    if (!isTauri) {
      showNotice("info", t("settings.channels.demoDefaultUnsupported"));
      return;
    }
    try {
      resetAutoSaveState();
      setSettingDefault(true);
      const bridge = getDesktopShellBridge();
      if (!bridge.agent) throw new Error(t("settings.channels.bridgeUnavailable"));
      const refreshed = await bridge.agent.channelsSetDefault(draft.id);
      setChannels(refreshed);
      const preservedKey = apiKeyInput || loadedChannelKeyRef.current;
      const current = refreshed.find((channel) => channel.id === draft.id);
      if (current) {
        selectChannel(current, preservedKey ? { preserveLoadedKey: preservedKey } : undefined);
      }
      notifyChannelsChanged();
      showNotice("success", t("settings.channels.defaultSucceeded"));
    } catch (err: any) {
      showNotice(
        "error",
        t("settings.channels.defaultFailed", {
          error: err?.message || t("settings.channels.networkError"),
        })
      );
    } finally {
      setSettingDefault(false);
    }
  };

  const handleConfirmDelete = async () => {
    if (!draft.id || isNewDraft) return;
    if (!isTauri) {
      showNotice("info", t("settings.channels.demoDeleteUnsupported"));
      return;
    }
    try {
      resetAutoSaveState();
      setDeleting(true);
      const bridge = getDesktopShellBridge();
      if (!bridge.agent) throw new Error(t("settings.channels.bridgeUnavailable"));
      const refreshed = await bridge.agent.channelsDelete(draft.id);
      if (readLastViewedChannelId() === draft.id) {
        clearLastViewedChannelId();
      }
      setChannels(refreshed);
      if (refreshed.length > 0)
        selectChannel(
          refreshed.find((channel) => isOfficialChannelId(channel.id)) || refreshed[0]
        );
      else {
        ++keyLoadGenRef.current;
        loadedChannelKeyRef.current = "";
        initialDraftRef.current = null;
        setApiKeyInput("");
        setActiveChannelId("");
        setIsNewDraft(false);
      }
      notifyChannelsChanged();
      showNotice("success", t("settings.channels.deleteSucceeded"));
    } catch (err: any) {
      showNotice("error", err?.message || t("settings.channels.deleteFailed"));
    } finally {
      setDeleting(false);
    }
  };

  const handleConfirmRename = async () => {
    if (!draft.id || isNewDraft) return;
    const oldId = draft.id.trim();
    const nextId = renameInput.trim();
    if (!nextId) {
      setRenameError(t("agent.channels.idInvalid"));
      return;
    }
    if (nextId.length > 64 || !/^[a-zA-Z0-9_-]+$/.test(nextId)) {
      setRenameError(t("agent.channels.idInvalid"));
      return;
    }
    if (nextId === oldId) {
      setRenameDialogOpen(false);
      return;
    }
    if (channels.some((c) => c.id === nextId)) {
      setRenameError(t("settings.channels.renameDuplicateError"));
      return;
    }
    resetAutoSaveState();
    if (!isTauri) {
      const updated = channels.map((c) => (c.id === oldId ? { ...c, id: nextId } : c));
      setChannels(updated);
      setRenameDialogOpen(false);
      const target = updated.find((c) => c.id === nextId);
      if (target) selectChannel(target);
      showNotice("success", t("settings.channels.renameSuccess", { id: nextId }));
      return;
    }
    const migrateStoredComposerChannelRef = (fromId: string, toId: string) => {
      try {
        const storageKey = "eostage.agent.modelSelection";
        const raw = window.localStorage.getItem(storageKey);
        if (!raw) return;
        const parsed = JSON.parse(raw) as Record<string, unknown>;
        if (parsed && typeof parsed === "object" && parsed.channel_id === fromId) {
          window.localStorage.setItem(
            storageKey,
            JSON.stringify({ ...parsed, channel_id: toId })
          );
        }
      } catch {
        // Storage is optional.
      }
    };
    try {
      setRenaming(true);
      setRenameError(null);
      const bridge = getDesktopShellBridge();
      if (!bridge.agent) throw new Error(t("settings.channels.bridgeUnavailable"));

      const trimmedTypedKey = apiKeyInput.trim();
      const hasExplicitKeyEdit =
        Boolean(trimmedTypedKey) && apiKeyInput !== loadedChannelKeyRef.current;

      let updatedList: AgentChannelView[];
      let preservedUiKey: string | undefined;

      if (typeof bridge.agent.channelsRename === "function") {
        // EOStage-style atomic in-process rename: Keychain secret is migrated
        // inside Rust without round-tripping plaintext through the UI or risking
        // delete-after-empty-save data loss.
        const initialSnap = initialDraftRef.current?.draft;
        const hasMetadataEdits =
          Boolean(initialSnap) &&
          JSON.stringify(initialSnap) !== JSON.stringify(draft);
        if (hasMetadataEdits) {
          await bridge.agent.channelsSave(
            buildChannelSavePayload({
              ...draft,
              id: oldId,
              apiKeyInput: "",
            })
          );
        }
        updatedList = await bridge.agent.channelsRename(
          oldId,
          nextId,
          hasExplicitKeyEdit ? trimmedTypedKey : null
        );
        const existingUiKey = hasExplicitKeyEdit
          ? trimmedTypedKey
          : loadedChannelKeyRef.current;
        preservedUiKey = existingUiKey || undefined;
      } else {
        // Legacy fallback when `channelsRename` is unavailable: fail closed and
        // NEVER overwrite with an empty key or delete `oldId` if `key_ready`
        // cannot be read back!
        let migratedKey = trimmedTypedKey || loadedChannelKeyRef.current;
        if (
          !migratedKey &&
          draft.key_ready &&
          typeof bridge.credentials?.readAgentChannelKey === "function"
        ) {
          try {
            const keyRes = await bridge.credentials.readAgentChannelKey(oldId);
            if (keyRes?.ok && typeof keyRes.value === "string") {
              migratedKey = keyRes.value.trim();
            }
          } catch {
            migratedKey = "";
          }
        }
        if (draft.key_ready && !migratedKey) {
          setRenameError(t("settings.channels.renameKeyUnreadable"));
          return;
        }
        const savePayload = buildChannelSavePayload({
          ...draft,
          id: nextId,
          apiKeyInput: migratedKey,
        });
        updatedList = await bridge.agent.channelsSave(savePayload);
        if (draft.is_default) {
          updatedList = await bridge.agent.channelsSetDefault(nextId);
        }
        updatedList = await bridge.agent.channelsDelete(oldId);
        preservedUiKey = migratedKey || undefined;
      }

      migrateStoredComposerChannelRef(oldId, nextId);
      setChannelAuthMetaMap((prev) => {
        if (!prev[oldId]) return prev;
        const nextMap = { ...prev };
        nextMap[nextId] = { ...nextMap[oldId], channel_id: nextId };
        delete nextMap[oldId];
        channelAuthMetaMapRef.current = nextMap;
        return nextMap;
      });
      setQuotaMap((prev) => {
        if (!prev[oldId]) return prev;
        const nextMap = { ...prev };
        nextMap[nextId] = { ...nextMap[oldId], channel_id: nextId };
        delete nextMap[oldId];
        return nextMap;
      });
      setChannels(updatedList);
      setRenameDialogOpen(false);
      const renamedView = updatedList.find((c) => c.id === nextId);
      if (renamedView) {
        selectChannel(
          renamedView,
          preservedUiKey ? { preserveLoadedKey: preservedUiKey } : undefined
        );
      }
      notifyChannelsChanged();
      showNotice("success", t("settings.channels.renameSuccess", { id: nextId }));
    } catch (err: any) {
      setRenameError(err?.message || t("settings.channels.saveFailed"));
    } finally {
      setRenaming(false);
    }
  };

  /**
   * 真实测试连接：支持指定单个模型测试（行右侧插头）或渠道默认模型测试（右上角 ... 菜单）
   */
  const handleTestConnection = async (targetModelId?: string) => {
    const explicitModelId = typeof targetModelId === "string" ? targetModelId.trim() : "";
    const defaultModelId = draft.model.trim() || draft.models[0]?.id?.trim() || "";
    const testModel = explicitModelId || defaultModelId || null;
    const matchedModelObj = testModel
      ? draft.models.find((m) => m.id.trim() === testModel)
      : undefined;
    const displayModelName = matchedModelObj?.name?.trim()
      ? matchedModelObj.name.trim() !== testModel
        ? `${matchedModelObj.name.trim()} (${testModel})`
        : matchedModelObj.name.trim()
      : testModel;
    const resolvedModelLabel = displayModelName
      ? explicitModelId
        ? displayModelName
        : t("settings.channels.defaultPrefixLabel", { model: displayModelName })
      : t("settings.channels.channelDefault");

    if (!isTauri) {
      showTestHud(
        {
          ok: true,
          code: null,
          status: 200,
          model_count: 5,
          latency_ms: 128,
          message: "pong (fixture)",
        },
        "pong (fixture)",
        resolvedModelLabel
      );
      return;
    }
    const generation = ++probeGeneration.current;
    setLoadingModels(false);
    try {
      setTesting(true);
      setTestingModelId(explicitModelId || null);
      clearTestToastTimer();
      setTestResult(null);
      setTestSnippet(null);
      setTestedModelLabel(resolvedModelLabel);
      const bridge = getDesktopShellBridge();
      if (!bridge.agent) throw new Error(t("settings.channels.bridgeUnavailable"));

      const testFn =
        typeof bridge.agent.channelsTestDraft === "function"
          ? bridge.agent.channelsTestDraft.bind(bridge.agent)
          : null;

      const fallbackStoredId =
        isNewDraft && !apiKeyInput && draft.key_ready
          ? (channels.find((c) => c.base_url === draft.base_url && c.key_ready)?.id ?? null)
          : null;
      const effectiveChannelId = isNewDraft ? fallbackStoredId : draft.id;
      const probeApiKey =
        !isNewDraft && draft.key_ready && apiKeyInput === loadedChannelKeyRef.current
          ? null
          : apiKeyInput || null;

      const effectiveProtocol = matchedModelObj?.protocol?.trim() || draft.protocol;

      if (testFn) {
        const result = await testFn({
          id: effectiveChannelId,
          protocol: effectiveProtocol,
          base_url: draft.base_url,
          api_key: probeApiKey,
          model: testModel,
        });
        if (generation !== probeGeneration.current) return;
        if (effectiveChannelId) {
          setChannelHealth(
            effectiveChannelId,
            result.ok ? "ok" : "error",
            result.message || undefined
          );
        }
        showTestHud(result, result.message || null, resolvedModelLabel);
      } else {
        const { result } = await bridge.agent.channelsProbeDraft({
          id: effectiveChannelId,
          protocol: effectiveProtocol,
          base_url: draft.base_url,
          api_key: probeApiKey,
          model: testModel,
        });
        if (generation !== probeGeneration.current) return;
        if (effectiveChannelId) {
          setChannelHealth(
            effectiveChannelId,
            result.ok ? "ok" : "error",
            result.message || undefined
          );
        }
        showTestHud(result, result.message || null, resolvedModelLabel);
      }
    } catch (err: any) {
      if (generation !== probeGeneration.current) return;
      if (!isNewDraft && draft.id) {
        setChannelHealth(draft.id, "error", err?.message || undefined);
      }
      showTestHud(
        {
          ok: false,
          code: "AGENT_CHANNEL_INVALID",
          status: null,
          model_count: null,
          latency_ms: null,
          message: err?.message || null,
        },
        err?.message || null,
        resolvedModelLabel
      );
    } finally {
      if (generation === probeGeneration.current) {
        setTesting(false);
        setTestingModelId(null);
      }
    }
  };

  const closeFetchDialog = useCallback(() => setFetchDialogOpen(false), []);
  const discoveredChatModels = useMemo(() => filterDiscoveredModels(modelsList), [modelsList]);

  const handleFetchModels = async () => {
    setFetchDialogOpen(true);
    setModelsError(null);
    if (!isTauri) {
      setModelsList(["gpt-5.4", "gpt-5.3-codex", "gpt-6.1-terra"]);
      return;
    }
    const generation = ++probeGeneration.current;
    setTesting(false);
    try {
      setLoadingModels(true);
      const bridge = getDesktopShellBridge();
      if (!bridge.agent) throw new Error(t("settings.channels.bridgeUnavailable"));
      const probeApiKey =
        !isNewDraft && draft.key_ready && apiKeyInput === loadedChannelKeyRef.current
          ? null
          : apiKeyInput || null;
      const { result, models } = await bridge.agent.channelsProbeDraft({
        id: isNewDraft ? null : draft.id,
        protocol: draft.protocol,
        base_url: draft.base_url,
        api_key: probeApiKey,
        model: draft.model || null,
      });
      if (generation !== probeGeneration.current) return;
      if (!isNewDraft && draft.id) {
        setChannelHealth(
          draft.id,
          result.ok ? "ok" : "error",
          result.message || undefined
        );
      }
      setModelsList(models);
      if (!result.ok)
        setModelsError(t("settings.channels.fetchFailed", { error: formatChannelTestResult(result) }));
      else if (models.length === 0) setModelsError(t("settings.channels.noModelsFetched"));
    } catch (err: any) {
      if (generation !== probeGeneration.current) return;
      if (!isNewDraft && draft.id) {
        setChannelHealth(draft.id, "error", err?.message || undefined);
      }
      setModelsList([]);
      setModelsError(err?.message || t("settings.channels.fetchModelsFailed"));
    } finally {
      if (generation === probeGeneration.current) setLoadingModels(false);
    }
  };

  const officialChannel = useMemo(
    () => channels.find((c) => isOfficialChannelId(c.id)) ?? null,
    [channels]
  );
  const isOfficialSelected = !isNewDraft && isOfficialChannelId(draft.id) && !selectedOfficialLoggedOut;
  const hasSelectedChannel = (isNewDraft || Boolean(activeChannelId)) && !selectedOfficialLoggedOut;

  const handleSelectOfficialChannel = () => {
    checkUnsavedAndProceed(() => {
      resetAutoSaveState();
      invalidateProbe();
      setIsAddingProviderCatalog(false);
      writeLastViewedChannelId(OFFICIAL_CHANNEL_ID);
      if (officialChannel) {
        selectChannel(officialChannel);
        return;
      }
      setSelectedOfficialLoggedOut(true);
      setIsNewDraft(false);
      setActiveChannelId("");
      setTestResult(null);
      setTestSnippet(null);
      setTestedModelLabel(null);
    });
  };

  const handleSyncOfficialModels = async () => {
    if (!isTauri) {
      showNotice("info", t("settings.channels.demoWriteUnsupported"));
      return;
    }
    try {
      setSyncingOfficialModels(true);
      await eostageAccountSyncModels();
      await loadChannels();
      notifyChannelsChanged();
      showNotice("success", t("settings.channels.syncedOfficialToast"));
    } catch (err: any) {
      showNotice("error", err?.message || t("settings.channels.syncOfficialFailed"));
    } finally {
      setSyncingOfficialModels(false);
    }
  };

  const otherProviderPresets = useMemo(
    () => PROVIDER_CATALOG_PRESETS.filter((p) => p.category === "other"),
    []
  );

  const selectedAuthKind = resolveChannelAuthKind(draft, channelAuthMetaMap);
  const activeQuota = draft.id ? quotaMap[draft.id] : undefined;
  const activeQuotaError = draft.id ? quotaErrorMap[draft.id] : null;
  const isQuotaLoading = Boolean(draft.id && quotaLoadingId === draft.id);

  const handleReauthorizeSubscription = () => {
    const providerMeta = channelAuthMetaMap[draft.id]?.oauth_provider;
    const presetId: OAuthPkceProviderPreset["id"] =
      providerMeta === "openrouter"
        ? "openrouter_pkce"
        : providerMeta === "claude_code"
          ? "claude_oauth_pkce"
          : providerMeta === "github_copilot"
            ? "github_copilot_device"
            : draft.id.startsWith("claude")
              ? "claude_oauth_pkce"
              : draft.id.startsWith("github-copilot") || draft.base_url.includes("githubcopilot.com")
                ? "github_copilot_device"
                : "openai_codex_pkce";
    const preset =
      SUPPORTED_OAUTH_PKCE_PROVIDERS.find((p) => p.id === presetId) ??
      SUPPORTED_OAUTH_PKCE_PROVIDERS[0];
    setIsAddingProviderCatalog(true);
    setPkceProviderId(preset.id);
    setPkceCallbackInput("");
    startPkceFlowForProvider(preset);
  };

  if (!isTauri && !isDevFixture) {
    return (
      <div className="flex h-72 flex-col items-center justify-center gap-3 rounded-card bg-sp-card p-8 text-center text-sp-text-secondary">
        <Cpu className="h-6 w-6" />
        <span className="text-sm">{t("settings.channels.desktopOnly")}</span>
      </div>
    );
  }

  return (
    <div className="space-y-4 max-w-5xl mx-auto">
      {/* 顶部标题栏 (对齐图 1 & 图 2：左侧标题+副标题，右侧刷新按钮 + 黑色「+ 添加供应商」胶囊，保留 sticky top-0 z-10 bg-sp-surface) */}
      <div
        data-testid="settings-models-tour-anchor"
        className="sticky top-0 z-10 bg-sp-surface eo-settings-page-header pb-2"
      >
        <div className="flex min-w-0 flex-1 flex-col gap-1">
          <h2 className="text-lg font-semibold text-sp-text">
            {t("settings.categories.models")}
          </h2>
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            data-testid="refresh-providers-btn"
            onClick={() =>
              void loadChannels().then(() =>
                emitTopToast({ text: t("settings.channels.providersRefreshed"), tone: "success" })
              )
            }
            title={t("settings.channels.refreshProviders")}
            aria-label={t("settings.channels.refreshProviders")}
            aria-busy={initialLoading}
            className={quietButtonClass}
          >
            <RefreshCw className={cn("h-3.5 w-3.5", providersRefreshSpinClass)} />
          </button>
          <button
            type="button"
            data-testid="toggle-oauth-pkce-helper"
            onClick={() => setIsAddingProviderCatalog(true)}
            className="eo-settings-btn-primary"
          >
            <Plus className="h-3.5 w-3.5" />
            <span>{t("settings.channels.addProvider")}</span>
          </button>
        </div>
      </div>

      {/* 主体统一大卡片（统一交互布局 图 1 & 图 2：左栏收纳 EOStage 官方与多自定义供应商，右栏紧耦合展示 Base URL + API 格式 + API Key + 模型列表） */}
      <div className="grid grid-cols-1 md:grid-cols-12 rounded-card border border-sp-line bg-sp-card min-h-[540px] overflow-x-clip">
        {/* 左侧供应商收纳栏（固定在滚动视口顶部，支持无限新增供应商垂直滚动收纳，绝不留左栏大片空白） */}
        <div className="md:col-span-4 border-b md:border-b-0 md:border-r border-sp-line p-3 space-y-4 overflow-y-auto md:sticky md:top-0 md:self-start max-h-[calc(100vh-150px)]">
          {/* 第一组：EOStage 官方通道 */}
          <div className="space-y-1" data-testid="provider-group-eostage">
            <div className="px-2 py-1 text-xs font-semibold text-sp-text-muted">
              EOStage
            </div>
            <div className="space-y-1">
              {(() => {
                const isOfficialActive =
                  !isAddingProviderCatalog &&
                  (selectedOfficialLoggedOut ||
                    (!isNewDraft && Boolean(officialChannel) && activeChannelId === officialChannel?.id));
                const officialId = officialChannel?.id || OFFICIAL_CHANNEL_ID;
                const officialEnabled = disabledProviders[officialId] !== true;
                const officialHealth =
                  officialChannel || officialAccount?.logged_in
                    ? resolveChannelHealthDotState(
                        {
                          id: officialId,
                          key_ready: Boolean(
                            officialChannel?.key_ready || officialAccount?.logged_in
                          ),
                        },
                        healthMap[officialId],
                        officialEnabled
                      )
                    : "missing_key";
                return (
                  <button
                    type="button"
                    data-testid="official-provider-eostage-official"
                    onClick={handleSelectOfficialChannel}
                    className={cn(
                      "eo-settings-channel-pill w-full justify-between text-[13px]",
                      isOfficialActive && "eo-settings-channel-pill-active"
                    )}
                  >
                    <div className="flex min-w-0 items-center gap-2">
                      <ProviderBrandLogo provider="eostage" className="h-4 w-4 shrink-0" />
                      <span className="truncate font-semibold text-sp-text">
                        {t("settings.channels.officialProviderName")}
                      </span>
                    </div>
                    <div className="flex items-center gap-2 shrink-0">
                      {officialChannel?.is_default && (
                        <span className="eo-settings-badge shrink-0">
                          {t("settings.channels.default")}
                        </span>
                      )}
                      {officialChannel || officialAccount?.logged_in ? (
                        <span
                          data-testid="official-channel-health-dot"
                          data-health-state={officialHealth}
                          className={cn(
                            "h-2 w-2 rounded-full shrink-0",
                            officialHealth === "ok"
                              ? "bg-sp-status-success"
                              : officialHealth === "error"
                                ? "bg-sp-status-danger"
                                : "bg-sp-text-disabled"
                          )}
                        />
                      ) : (
                        <span className="eo-settings-badge shrink-0">
                          {t("settings.channels.notLoggedIn")}
                        </span>
                      )}
                    </div>
                  </button>
                );
              })()}
            </div>
          </div>

          {/* 第二组：自定义供应商列表 */}
          <div className="space-y-1" data-testid="provider-group-custom">
            <div className="flex items-center justify-between px-2 py-1">
              <span className="text-xs text-sp-text-muted">
                {t("settings.channels.customProviders")}
              </span>
              {!isNewDraft && (
                <button
                  type="button"
                  onClick={handleStartCreate}
                  aria-label={t("settings.channels.newChannel")}
                  title={t("settings.channels.newChannel")}
                  className="flex h-6 w-6 items-center justify-center rounded-sp-sm text-[13px] text-sp-text hover:bg-sp-hover transition-colors cursor-pointer"
                >
                  <Plus className="h-3.5 w-3.5" />
                </button>
              )}
            </div>

            {initialLoading ? (
              showChannelsSkeleton ? (
                <SessionRowSkeletons rows={4} />
              ) : (
                <div className="h-28" aria-hidden="true" />
              )
            ) : channels.filter((c) => !isOfficialChannelId(c.id) && c.id !== "eostage-plan" && c.id !== "eostage-api").length === 0 && !isNewDraft ? null : (
              <div className="space-y-1">
                {channels
                  .filter((c) => !isOfficialChannelId(c.id) && c.id !== "eostage-plan" && c.id !== "eostage-api")
                  .map((channel) => {
                    const isSelected =
                      !isAddingProviderCatalog &&
                      !selectedOfficialLoggedOut &&
                      !isNewDraft &&
                      activeChannelId === channel.id;
                    return (
                      <div key={channel.id}>
                        <ChannelListItem
                          channel={channel}
                          isSelected={isSelected}
                          enabled={disabledProviders[channel.id] !== true}
                          authKind={resolveChannelAuthKind(channel, channelAuthMetaMap)}
                          healthState={resolveChannelHealthDotState(
                            channel,
                            healthMap[channel.id],
                            disabledProviders[channel.id] !== true
                          )}
                          onSelect={() => handleSelectChannel(channel)}
                        />
                      </div>
                    );
                  })}

                {isNewDraft && (
                  <div>
                    <ChannelListItem
                      channel={{ ...draft, models: draft.models } as AgentChannelView}
                      isSelected={!isAddingProviderCatalog && !selectedOfficialLoggedOut}
                      enabled={providerEnabled}
                      healthState={resolveChannelHealthDotState(
                        { id: draft.id, key_ready: draft.key_ready || apiKeyInput.trim().length > 0 },
                        healthMap[draft.id],
                        providerEnabled
                      )}
                      onSelect={() => {
                        setSelectedOfficialLoggedOut(false);
                        setIsAddingProviderCatalog(false);
                      }}
                    />
                  </div>
                )}
              </div>
            )}
          </div>
        </div>

        {/* 右侧主详情面板：添加供应商目录视图 vs 供应商紧耦合配置详情视图 */}
        <div className="md:col-span-8 p-5 flex flex-col justify-between">
          {isAddingProviderCatalog ? (
            <div data-testid="add-provider-catalog-view" className="space-y-5">
              {/* 顶部返回栏：← 添加供应商 */}
              <div className="flex items-center justify-between gap-3 border-b border-sp-divider pb-3">
                <button
                  type="button"
                  data-testid="back-from-add-provider-btn"
                  onClick={() => setIsAddingProviderCatalog(false)}
                  className="flex shrink-0 whitespace-nowrap items-center gap-2 rounded-sp-sm px-2 py-1 text-sm font-semibold text-sp-text hover:bg-sp-hover transition-colors cursor-pointer"
                >
                  <ArrowLeft className="h-4 w-4 shrink-0" />
                  <span>{t("settings.channels.addProvider")}</span>
                </button>
              </div>

              {/* 1. 官方渠道预设与自定义供应商（品牌 Logo + 无广告简介 + 防折行「添加」按钮） */}
              <div className="space-y-2">
                <div className="text-xs font-semibold text-sp-text-muted">
                  {t("settings.channels.officialAndCustom")}
                </div>
                <div className="sp-grid-cards-sm">
                  <button
                    type="button"
                    data-testid="create-custom-provider-card"
                    onClick={handleStartCreate}
                    className="flex min-h-12 items-center justify-center gap-2 rounded-sp-md border border-dashed border-sp-line bg-sp-card px-3 py-3 text-[13px] font-semibold text-sp-text hover:bg-sp-hover transition-colors cursor-pointer"
                  >
                    <Plus className="h-4 w-4 shrink-0 text-sp-text-secondary" />
                    <span className="truncate">{t("settings.channels.createCustomProvider")}</span>
                  </button>

                  {otherProviderPresets.map((preset) => (
                    <button
                      key={preset.id}
                      type="button"
                      data-testid={`preset-provider-card-${preset.id}`}
                      onClick={() => handleApplyProviderPreset(preset)}
                      className="flex min-h-12 items-center justify-between gap-2 rounded-sp-md border border-sp-line bg-sp-card px-3 py-3 text-left hover:bg-sp-hover transition-colors cursor-pointer"
                    >
                      <div className="flex min-w-0 items-center gap-2">
                        <ProviderBrandLogo
                          provider={preset.id}
                          baseUrl={preset.base_url}
                        />
                        <span className="truncate text-[13px] font-semibold text-sp-text">
                          {getPresetDisplayName(preset)}
                        </span>
                      </div>
                      <span className="eo-settings-badge shrink-0 whitespace-nowrap">
                        {t("settings.channels.add")}
                      </span>
                    </button>
                  ))}
                </div>
              </div>

              {/* 2. 免 Key 浏览器授权 (OAuth PKCE) */}
              <div
                data-testid="oauth-pkce-helper-panel"
                className="rounded-sp-md border border-sp-line bg-sp-surface p-4 space-y-3"
              >
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0 flex-1 space-y-1">
                    <div className="text-sm font-semibold text-sp-text">
                      {t("settings.channels.pkceTitle")}
                    </div>
                    <p className="text-xs text-sp-text-muted">
                      {t("settings.channels.pkceSubtitle")}
                    </p>
                  </div>
                  <button
                    type="button"
                    data-testid="generate-pkce-auth-url-btn"
                    onClick={handleGeneratePkceAuthUrl}
                    className="eo-settings-btn-primary shrink-0 whitespace-nowrap"
                  >
                    {t("settings.channels.pkceGenerateAndOpen")}
                  </button>
                </div>

                <div className="grid grid-cols-1 gap-2 xl:grid-cols-2">
                  {SUPPORTED_OAUTH_PKCE_PROVIDERS.map((provider) => (
                    <button
                      key={provider.id}
                      type="button"
                      data-testid={`pkce-provider-${provider.id}`}
                      onClick={() => {
                        stopHelperPolling();
                        setPkceProviderId(provider.id);
                        setPkceSession(null);
                        setPkceStatusText(null);
                        setPkceCallbackInput("");
                      }}
                      className={cn(
                        "eo-settings-channel-pill w-full min-w-0 overflow-hidden !whitespace-normal justify-between gap-2 text-left text-[13px] min-h-[36px] h-auto py-2 px-3",
                        pkceProviderId === provider.id && "eo-settings-channel-pill-active"
                      )}
                    >
                      <div className="flex min-w-0 flex-1 items-center gap-2">
                        <ProviderBrandLogo
                          provider={provider.id}
                          baseUrl={provider.base_url}
                          className="h-3.5 w-3.5 shrink-0"
                        />
                        <span className="min-w-0 whitespace-normal leading-snug break-words">
                          {getPkceProviderCopy(provider).name}
                        </span>
                      </div>
                      {pkceProviderId === provider.id && (
                        <Check className="h-3.5 w-3.5 shrink-0 text-sp-text" />
                      )}
                    </button>
                  ))}
                </div>

                <p className="text-xs text-sp-text-secondary">
                  {getPkceProviderCopy(selectedPkceProvider).description}
                </p>

                {pkceSession && (
                  <div className="rounded-sp-sm border border-sp-line bg-sp-surface-muted p-3 space-y-2">
                    <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-sp-text-muted">
                      {pkceSession.mode === "device" ? (
                        <span className="flex flex-wrap items-center gap-2">
                          <span>{t("settings.channels.deviceCodeLabel")}:</span>
                          <code
                            data-testid="device-user-code-display"
                            className="font-mono font-semibold text-sp-text select-all"
                          >
                            {pkceSession.userCode || pkceSession.codeChallenge}
                          </code>
                          <button
                            type="button"
                            data-testid="copy-device-code-btn"
                            onClick={() => {
                              const codeVal = pkceSession.userCode || pkceSession.codeChallenge;
                              if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
                                void navigator.clipboard.writeText(codeVal);
                              }
                              setCopiedDeviceCode(true);
                              window.setTimeout(() => setCopiedDeviceCode(false), 1800);
                            }}
                            className={cn(quietButtonClass, "shrink-0 whitespace-nowrap")}
                          >
                            <Copy className="mr-1 inline h-3 w-3" />
                            {copiedDeviceCode
                              ? t("settings.channels.deviceCodeCopied")
                              : t("settings.channels.deviceCopyCode")}
                          </button>
                        </span>
                      ) : (
                        <span>
                          S256 Challenge:{" "}
                          <code
                            data-testid="device-user-code-display"
                            className="font-mono font-semibold text-sp-text"
                          >
                            {pkceSession.codeChallenge}
                          </code>
                        </span>
                      )}
                      <div className="flex flex-wrap items-center gap-2">
                        {pkceSession.mode !== "device" && (
                          <span>
                            State:{" "}
                            <code className="font-mono text-sp-text">
                              {pkceSession.state}
                            </code>
                          </span>
                        )}
                        <button
                          type="button"
                          data-testid="open-pkce-browser-btn"
                          onClick={() => {
                            const isNativePkceProvider =
                              selectedPkceProvider.id === "openai_codex_pkce" ||
                              selectedPkceProvider.id === "claude_oauth_pkce" ||
                              selectedPkceProvider.id === "openrouter_pkce";
                            const isNativeDeviceProvider =
                              selectedPkceProvider.id === "github_copilot_device";
                            const needsFreshNativeSession =
                              isTauri &&
                              hasNativeTauriInvoke() &&
                              ((isNativePkceProvider &&
                                (!pkceSession.nativePkceSessionId ||
                                  helperPollTimerRef.current === null ||
                                  (pkceSession.expiresAtMs !== undefined &&
                                    Date.now() >= pkceSession.expiresAtMs - 10_000))) ||
                                (isNativeDeviceProvider &&
                                  (!pkceSession.nativeDeviceSessionId ||
                                    helperPollTimerRef.current === null ||
                                    (pkceSession.expiresAtMs !== undefined &&
                                      Date.now() >= pkceSession.expiresAtMs - 10_000))));
                            if (needsFreshNativeSession) {
                              handleGeneratePkceAuthUrl();
                              return;
                            }
                            void (async () => {
                              const res = await openOAuthBrowserUrl(pkceSession.authorizeUrl);
                              if (res.ok) {
                                setPkceStatusOk(true);
                                setPkceStatusText(
                                  pkceSession.mode === "device"
                                    ? t("settings.channels.devicePendingHint")
                                    : pkceSession.nativePkceSessionId
                                      ? t("settings.channels.pkceOpenBrowserStartedNative")
                                      : t("settings.channels.pkceOpenBrowserStartedManual")
                                );
                              } else {
                                setPkceStatusOk(false);
                                setPkceStatusText(
                                  res.error || t("settings.channels.pkceOpenBrowserFailed")
                                );
                              }
                            })();
                          }}
                          className={cn(quietButtonClass, "shrink-0 whitespace-nowrap")}
                        >
                          <ExternalLink className="mr-1 inline h-3 w-3" />
                          {t("settings.channels.pkceOpenBrowser")}
                        </button>
                      </div>
                    </div>
                    <div
                      data-testid="pkce-authorize-url-display"
                      className="font-mono text-xs text-sp-text break-all select-all"
                    >
                      {pkceSession.authorizeUrl}
                    </div>
                  </div>
                )}

                {selectedPkceProvider.id !== "github_copilot_device" && (
                  <div className="flex items-center gap-2">
                    <input
                      type="text"
                      data-testid="pkce-callback-input"
                      value={pkceCallbackInput}
                      onChange={(e) => setPkceCallbackInput(e.target.value)}
                      placeholder={t("settings.channels.pkceCallbackPlaceholder")}
                      className={cn(inputClass, "font-mono")}
                    />
                    <button
                      type="button"
                      data-testid="verify-pkce-callback-btn"
                      onClick={handleVerifyPkceCallback}
                      className={cn(quietButtonClass, "shrink-0 whitespace-nowrap")}
                    >
                      {t("settings.channels.pkceVerifyCallback")}
                    </button>
                  </div>
                )}

                {pkceStatusText && (
                  <div
                    data-testid="pkce-status-banner"
                    className={cn(
                      "flex items-center justify-between gap-2 rounded-sp-sm px-3 py-2 text-xs",
                      pkceStatusOk
                        ? "bg-sp-success-bg text-sp-success-text"
                        : "bg-sp-danger-bg text-sp-danger-text"
                    )}
                  >
                    <span>{pkceStatusText}</span>
                    {pkceStatusOk && (
                      <button
                        type="button"
                        onClick={() => setIsAddingProviderCatalog(false)}
                        className={cn(quietButtonClass, "shrink-0 whitespace-nowrap")}
                      >
                        {t("settings.channels.pkceEnterConfig")}
                      </button>
                    )}
                  </div>
                )}
              </div>
            </div>
          ) : selectedOfficialLoggedOut ? (
            <div
              data-testid="official-eostage-panel"
              className="flex h-full flex-col justify-between space-y-5"
            >
              <div className="space-y-4">
                <div className="flex items-center justify-between border-b border-sp-divider pb-3">
                  <div className="flex items-center gap-2">
                    <ProviderBrandLogo provider="eostage" />
                    <h4 className="text-sm font-semibold text-sp-text">
                      {t("settings.channels.officialProviderName")}
                    </h4>
                  </div>
                </div>

                <div className="rounded-sp-md border border-sp-line bg-sp-surface p-5 space-y-4">
                  <p className="text-xs text-sp-text-secondary leading-relaxed">
                    {t("settings.channels.officialSignInDesc")}
                  </p>
                  <div className="flex flex-wrap items-center gap-2">
                    <button
                      type="button"
                      data-testid="official-panel-login-btn"
                      onClick={() => setOfficialLoginOpen(true)}
                      className="eo-settings-btn-primary"
                    >
                      <span>{t("settings.channels.signInOfficial")}</span>
                    </button>
                  </div>
                </div>
              </div>
            </div>
          ) : hasSelectedChannel ? (
            <div
              data-testid={isOfficialSelected ? "official-eostage-panel" : undefined}
              className="flex h-full flex-col justify-between space-y-5"
            >
              <div className="space-y-4">
                {/* 供应商面板顶部标题行 */}
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-2">
                    <ProviderBrandLogo
                      provider={isOfficialSelected ? "eostage" : draft.id}
                      baseUrl={draft.base_url}
                      modelHint={[draft.model, ...draft.models.map((m) => m.id)]
                        .filter(Boolean)
                        .join(" ")}
                    />
                    <h4 className="text-sm font-semibold text-sp-text">
                      {isOfficialSelected
                        ? t("settings.channels.officialProviderName")
                        : isNewDraft
                          ? draft.id || t("settings.channels.addModelChannel")
                          : draft.id}
                    </h4>
                    {isNewDraft ? (
                      <span className="eo-settings-badge">
                        {t("settings.channels.draftChannel")}
                      </span>
                    ) : (
                      <>
                        {!isOfficialSelected && (
                          <span
                            data-testid="channel-detail-auth-kind-badge"
                            data-auth-kind={selectedAuthKind}
                            className="eo-settings-badge"
                          >
                            {selectedAuthKind === "subscription"
                              ? t("settings.channels.authKindSubscription")
                              : t("settings.channels.authKindApi")}
                          </span>
                        )}
                        {draft.is_default && (
                          <span className="eo-settings-badge">
                            {t("settings.channels.default")}
                          </span>
                        )}
                      </>
                    )}
                  </div>

                  <div ref={moreMenuContainerRef} className="relative flex items-center gap-2">
                    {isOfficialSelected && officialAccount?.logged_in && (
                      <span
                        data-testid="official-panel-credits-pill"
                        className="eo-settings-badge tabular-nums select-text"
                      >
                        {(officialAccount.total_credits ?? 0).toLocaleString()} Credits
                      </span>
                    )}
                    {isNewDraft && (
                      <button
                        type="button"
                        onClick={handleCancelDraft}
                        className="text-[13px] text-sp-text hover:underline cursor-pointer"
                      >
                        {t("settings.channels.cancelDraft")}
                      </button>
                    )}
                    <OptimisticInlineStatus
                      state={autoSaveSync.state !== "idle" ? autoSaveSync.state : providerSwitchSync.state}
                      errorMessage={autoSaveSync.state === "error" ? autoSaveSync.error : providerSwitchSync.error}
                      onRetry={autoSaveSync.state === "error" ? autoSaveSync.retry : providerSwitchSync.retry}
                      testId={autoSaveSync.state !== "idle" ? "channel-autosave-status" : "provider-switch-sync-status"}
                    />
                    <button
                      type="button"
                      role="switch"
                      aria-checked={providerEnabled}
                      aria-label={t("settings.channels.enableProviderAria")}
                      onClick={() => {
                        const prevVal = providerEnabled;
                        const nextVal = !prevVal;
                        setProviderEnabled(nextVal);
                        if (draft.id) {
                          setDisabledProviders((cur) => {
                            const nextMap = { ...cur };
                            if (nextVal) delete nextMap[draft.id];
                            else nextMap[draft.id] = true;
                            return nextMap;
                          });
                        }
                        if (isNewDraft || !draft.id) return;
                        const runToggle = async (targetEnabled: boolean) => {
                          const gen = ++providerSwitchGenRef.current;
                          setProviderSwitchSync({ state: "saving" });
                          try {
                            await persistProviderEnabled(draft.id, targetEnabled);
                            if (gen !== providerSwitchGenRef.current) return;
                            setDisabledProviders(loadDisabledProviders());
                            setProviderSwitchSync({ state: "saved" });
                            window.setTimeout(() => {
                              if (gen === providerSwitchGenRef.current) {
                                setProviderSwitchSync((cur) => (cur.state === "saved" ? { state: "idle" } : cur));
                              }
                            }, 1600);
                          } catch (err: any) {
                            if (gen !== providerSwitchGenRef.current) return;
                            setProviderEnabled(prevVal);
                            setDisabledProviders(loadDisabledProviders());
                            setProviderSwitchSync({
                              state: "error",
                              error: err?.message || t("settings.channels.toggleSaveFailed"),
                              retry: () => {
                                setProviderEnabled(targetEnabled);
                                setDisabledProviders((cur) => {
                                  const nextMap = { ...cur };
                                  if (targetEnabled) delete nextMap[draft.id];
                                  else nextMap[draft.id] = true;
                                  return nextMap;
                                });
                                void runToggle(targetEnabled);
                              },
                            });
                          }
                        };
                        void runToggle(nextVal);
                      }}
                      className="eo-settings-switch"
                    >
                      <span className="eo-settings-switch-thumb" />
                    </button>
                    <button
                      type="button"
                      data-testid="provider-more-actions-btn"
                      aria-label={t("settings.channels.moreProviderActions")}
                      title={t("settings.channels.moreActionsTitle")}
                      aria-busy={testing && testingModelId === null}
                      onClick={() => setMoreMenuOpen((prev) => !prev)}
                      className="flex h-7 w-7 items-center justify-center rounded-sp-sm text-[13px] text-sp-text hover:bg-sp-hover active:scale-95 transition-[colors,transform] cursor-pointer"
                    >
                      {testing && testingModelId === null && showProviderTestSpinner ? (
                        <Loader2 data-testid="provider-test-connection-spinner" className="h-4 w-4 animate-spin" />
                      ) : (
                        <MoreHorizontal className="h-4 w-4" />
                      )}
                    </button>
                    {moreMenuOpen && (
                      <div
                        data-testid="provider-more-actions-menu"
                        className="absolute right-0 top-8 z-20 min-w-40 rounded-sp-md border border-sp-line bg-sp-card p-1 shadow-modal space-y-1"
                      >
                        <button
                          type="button"
                          data-testid="provider-menu-test-connection-btn"
                          aria-label={t("settings.channels.testEndpoint")}
                          disabled={testing || saving || deleting || renaming}
                          onClick={() => {
                            setMoreMenuOpen(false);
                            void handleTestConnection();
                          }}
                          className="flex w-full items-center gap-2 rounded-sp-sm px-2 py-1 text-[13px] text-sp-text hover:bg-sp-hover cursor-pointer disabled:opacity-50"
                        >
                          <Activity className="h-3.5 w-3.5 text-sp-text-secondary" />
                          <span>{t("settings.channels.testConnection")}</span>
                        </button>
                        {isOfficialSelected && (
                          <>
                            <button
                              type="button"
                              onClick={() => {
                                setMoreMenuOpen(false);
                                void handleSyncOfficialModels();
                              }}
                              className="flex w-full items-center gap-2 rounded-sp-sm px-2 py-1 text-[13px] text-sp-text hover:bg-sp-hover cursor-pointer"
                            >
                              <RefreshCw className="h-3.5 w-3.5 text-sp-text-secondary" />
                              <span>{t("settings.channels.syncOfficialModels")}</span>
                            </button>
                            <button
                              type="button"
                              onClick={() => {
                                setMoreMenuOpen(false);
                                setOfficialBillingOpen(true);
                              }}
                              className="flex w-full items-center gap-2 rounded-sp-sm px-2 py-1 text-[13px] text-sp-text hover:bg-sp-hover cursor-pointer"
                            >
                              <Globe className="h-3.5 w-3.5 text-sp-text-secondary" />
                              <span>{t("settings.channels.balanceAndBilling")}</span>
                            </button>
                          </>
                        )}
                        {!isNewDraft && !isOfficialSelected && (
                          <button
                            type="button"
                            data-testid="provider-menu-rename-btn"
                            aria-label={t("settings.channels.renameChannel")}
                            disabled={saving || deleting || renaming}
                            onClick={() => {
                              setMoreMenuOpen(false);
                              setRenameInput(draft.id);
                              setRenameError(null);
                              setRenameDialogOpen(true);
                            }}
                            className="flex w-full items-center gap-2 rounded-sp-sm px-2 py-1 text-[13px] text-sp-text hover:bg-sp-hover cursor-pointer disabled:opacity-50"
                          >
                            <Pencil className="h-3.5 w-3.5 text-sp-text-secondary" />
                            <span>{t("settings.channels.renameChannel")}</span>
                          </button>
                        )}
                        {!isNewDraft && !draft.is_default && (
                          <button
                            type="button"
                            data-testid="provider-menu-set-default-btn"
                            aria-label={t("settings.channels.setCurrentDefault")}
                            disabled={settingDefault || saving || deleting || renaming}
                            onClick={() => {
                              setMoreMenuOpen(false);
                              void handleSetDefault();
                            }}
                            className="flex w-full items-center gap-2 rounded-sp-sm px-2 py-1 text-[13px] text-sp-text hover:bg-sp-hover cursor-pointer disabled:opacity-50"
                          >
                            <Check className="h-3.5 w-3.5 text-sp-text-secondary" />
                            <span>{t("settings.channels.setDefault")}</span>
                          </button>
                        )}
                        {!isNewDraft && isOfficialSelected && (
                          <div className="pt-1 border-t border-sp-divider">
                            <button
                              type="button"
                              data-testid="official-menu-logout-btn"
                              onClick={() => {
                                setMoreMenuOpen(false);
                                void eostageAccountLogout().then(() => {
                                  notifyEOStageAccountChanged();
                                  notifyChannelsChanged();
                                  setSelectedOfficialLoggedOut(true);
                                  setActiveChannelId("");
                                });
                              }}
                              className="flex w-full items-center gap-2 rounded-sp-sm px-2 py-1 text-[13px] text-sp-danger-text hover:bg-sp-danger-bg cursor-pointer"
                            >
                              <span>{t("settings.channels.signOutOfficial")}</span>
                            </button>
                          </div>
                        )}
                        {!isNewDraft && !isOfficialSelected && (
                          <div className="pt-1 border-t border-sp-divider">
                            <ChannelDeleteControl
                              variant="menu-item"
                              onConfirmDelete={() => {
                                setMoreMenuOpen(false);
                                void handleConfirmDelete();
                              }}
                              deleting={deleting}
                              disabled={saving || testing || settingDefault || renaming}
                            />
                          </div>
                        )}
                      </div>
                    )}
                  </div>
                </div>

                {/* 紧耦合配置表单：官方渠道隐藏 Base URL 与密钥输入框，展示账号摘要并复用完全一致的模型编辑器 */}
                <div className="space-y-3">
                  {isOfficialSelected ? (
                    <div
                      data-testid="official-account-summary-bar"
                      className="flex flex-wrap items-center justify-between gap-2 rounded-sp-md border border-sp-line bg-sp-surface px-4 py-3"
                    >
                      <div className="text-[13px] font-semibold text-sp-text">
                        {officialAccount?.email || t("settings.channels.signedInDefaultTitle")}
                      </div>
                      <button
                        type="button"
                        data-testid="official-open-billing-btn"
                        onClick={() => setOfficialBillingOpen(true)}
                        className="eo-settings-btn-secondary shrink-0"
                      >
                        <span>{t("settings.channels.viewBalanceUsage")}</span>
                      </button>
                    </div>
                  ) : (
                    <>
                  <div
                    className={cn("space-y-1", !isNewDraft && "sr-only")}
                    aria-hidden={!isNewDraft || undefined}
                  >
                    <div className="flex items-center gap-1">
                      <label htmlFor={`channel-name-${draft.id}`} className="text-[13px] text-sp-text-secondary">
                        {t("settings.channels.name")}
                      </label>
                      {isNewDraft && (
                        <HoverHintLabel hint={t("settings.channels.nameHint")} focusable>
                          <Info
                            className="h-3.5 w-3.5 text-sp-text-muted"
                            aria-label={t("settings.channels.nameRules")}
                          />
                        </HoverHintLabel>
                      )}
                    </div>
                    <input
                      id={`channel-name-${draft.id}`}
                      aria-label={t("settings.channels.name")}
                      type="text"
                      value={draft.id}
                      tabIndex={!isNewDraft ? -1 : undefined}
                      disabled={!isNewDraft || saving}
                      onChange={(event) => {
                        invalidateProbe();
                        const nextDraft = { ...draftRef.current, id: event.target.value };
                        draftRef.current = nextDraft;
                        setDraft(nextDraft);
                      }}
                      className={cn(inputClass, "h-9")}
                    />
                  </div>

                  <div className="space-y-1">
                    <div className="flex items-center gap-1">
                      <label htmlFor={`channel-url-${draft.id}`} className="text-[13px] text-sp-text-secondary">
                        {t("settings.channels.baseUrl")}
                      </label>
                      <HoverHintLabel hint={t("settings.channels.baseUrlHint")} focusable>
                        <Info
                          className="h-3.5 w-3.5 text-sp-text-muted"
                          aria-label={t("settings.channels.baseUrlHintLabel")}
                        />
                      </HoverHintLabel>
                    </div>
                    <input
                      id={`channel-url-${draft.id}`}
                      aria-label={t("settings.channels.baseUrl")}
                      type="text"
                      placeholder="https://api.example.com/v1"
                      value={draft.base_url}
                      disabled={saving}
                      onChange={(event) => {
                        invalidateProbe();
                        const nextDraft = { ...draftRef.current, base_url: event.target.value };
                        draftRef.current = nextDraft;
                        setDraft(nextDraft);
                        if (!isNewDraftRef.current) {
                          scheduleAutoSave(nextDraft, apiKeyInputRef.current, 500);
                        }
                      }}
                      onBlur={() => {
                        if (
                          !isNewDraftRef.current &&
                          initialDraftRef.current &&
                          draftRef.current.base_url !== initialDraftRef.current.draft.base_url
                        ) {
                          scheduleAutoSave(draftRef.current, apiKeyInputRef.current, 0);
                        }
                      }}
                      className={cn(inputClass, "h-9 font-mono")}
                    />
                  </div>

                  <div className="space-y-1">
                    <div className="flex items-center gap-1">
                      <label className="text-[13px] text-sp-text-secondary">
                        {t("settings.channels.protocol")}
                      </label>
                    </div>
                    <ProtocolSelect
                      value={draft.protocol}
                      disabled={saving}
                      onChange={(newProtocol) => {
                        invalidateProbe();
                        const nextDraft = { ...draftRef.current, protocol: newProtocol };
                        draftRef.current = nextDraft;
                        setDraft(nextDraft);
                        if (!isNewDraftRef.current) {
                          scheduleAutoSave(nextDraft, apiKeyInputRef.current, 0);
                        }
                      }}
                    />
                  </div>

                  {!isNewDraft && selectedAuthKind === "subscription" ? (
                    <div
                      data-testid="subscription-quota-card"
                      className="rounded-sp-md border border-sp-line bg-sp-surface-muted/40 p-4 space-y-3"
                    >
                      <div className="flex flex-wrap items-center justify-between gap-2">
                        <div className="flex flex-wrap items-center gap-2">
                          <span className="text-[13px] font-semibold text-sp-text">
                            {t("settings.channels.subscriptionQuotaTitle")}
                          </span>
                          {draft.key_ready && (
                            <span className="eo-settings-badge">
                              {t("settings.channels.subscriptionCredSaved")}
                            </span>
                          )}
                          {activeQuota?.plan_type && (
                            <span
                              data-testid="subscription-plan-badge"
                              className="eo-settings-badge uppercase"
                            >
                              {activeQuota.plan_type}
                            </span>
                          )}
                        </div>
                        <div className="flex items-center gap-2">
                          <button
                            type="button"
                            data-testid="subscription-quota-refresh-btn"
                            onClick={() => void fetchChannelQuota(draft.id)}
                            disabled={isQuotaLoading}
                            className={quietButtonClass}
                          >
                            <RefreshCw
                              className={cn(
                                "mr-1 inline h-3.5 w-3.5",
                                isQuotaLoading && "animate-spin"
                              )}
                            />
                            {t("settings.channels.refreshQuota")}
                          </button>
                          <button
                            type="button"
                            data-testid="subscription-reauthorize-btn"
                            onClick={handleReauthorizeSubscription}
                            className={quietButtonClass}
                          >
                            {t("settings.channels.reauthorize")}
                          </button>
                        </div>
                      </div>

                      {isQuotaLoading && !activeQuota ? (
                        <div className="flex items-center gap-2 text-xs text-sp-text-muted">
                          <Loader2 className="h-3.5 w-3.5 animate-spin" />
                          <span>{t("settings.channels.quotaLoading")}</span>
                        </div>
                      ) : activeQuota ? (
                        <div className="space-y-3">
                          {(
                            [
                              {
                                key: "primary",
                                label:
                                  activeQuota.oauth_provider === "github_copilot"
                                    ? t("settings.channels.quotaPremiumWindow")
                                    : t("settings.channels.quota5hWindow"),
                                win: activeQuota.primary_window,
                              },
                              {
                                key: "secondary",
                                label:
                                  activeQuota.oauth_provider === "github_copilot"
                                    ? t("settings.channels.quotaChatWindow")
                                    : t("settings.channels.quotaWeeklyWindow"),
                                win: activeQuota.secondary_window,
                              },
                            ] as Array<{
                              key: string;
                              label: string;
                              win: SubscriptionQuotaWindow | null | undefined;
                            }>
                          )
                            .filter((row): row is { key: string; label: string; win: SubscriptionQuotaWindow } =>
                              Boolean(row.win)
                            )
                            .map((row) => {
                              const pct = Math.max(0, Math.min(100, row.win.remaining_percent));
                              const resetText = formatQuotaResetRemaining(row.win.reset_at_secs, t);
                              return (
                                <div
                                  key={row.key}
                                  data-testid={`subscription-quota-${row.key}`}
                                  className="space-y-1"
                                >
                                  <div className="flex items-center justify-between gap-2 text-xs">
                                    <span className="text-sp-text-secondary">
                                      {row.label}
                                    </span>
                                    <div className="flex items-center gap-2 tabular-nums">
                                      <span className="font-semibold text-sp-text">
                                        {t("settings.channels.quotaRemaining", { percent: pct })}
                                      </span>
                                      {resetText && (
                                        <span className="text-sp-text-muted">{resetText}</span>
                                      )}
                                    </div>
                                  </div>
                                  <div className="h-1.5 w-full overflow-hidden rounded-full bg-sp-surface-muted">
                                    <div
                                      className={cn(
                                        "h-full rounded-full transition-all",
                                        pct <= 15 ? "bg-sp-status-danger" : "bg-sp-accent"
                                      )}
                                      style={{ width: `${pct}%` }}
                                    />
                                  </div>
                                </div>
                              );
                            })}

                          {(activeQuota.credits_unlimited ||
                            activeQuota.credits_balance ||
                            activeQuota.reset_credits_available != null) && (
                            <div className="flex flex-wrap items-center gap-3 pt-1 text-xs text-sp-text-secondary">
                              {(activeQuota.credits_unlimited || activeQuota.credits_balance) && (
                                <span data-testid="subscription-credits-balance" className="tabular-nums">
                                  {t("settings.channels.quotaBalance")}:{" "}
                                  <strong className="font-semibold text-sp-text">
                                    {activeQuota.credits_unlimited
                                      ? t("settings.channels.quotaUnlimited")
                                      : `$${activeQuota.credits_balance}`}
                                  </strong>
                                </span>
                              )}
                              {activeQuota.reset_credits_available != null && (
                                <span data-testid="subscription-reset-credits" className="tabular-nums">
                                  {t("settings.channels.quotaResetCredits", {
                                    count: activeQuota.reset_credits_available,
                                  })}
                                </span>
                              )}
                            </div>
                          )}
                        </div>
                      ) : activeQuotaError ? (
                        <p role="alert" className="text-xs text-sp-status-danger">
                          {activeQuotaError}
                        </p>
                      ) : (
                        <p className="text-xs text-sp-text-muted">
                          {t("settings.channels.quotaUnavailable")}
                        </p>
                      )}

                      {activeQuota && activeQuotaError && (
                        <p role="alert" className="text-xs text-sp-status-danger">
                          {activeQuotaError}
                        </p>
                      )}
                    </div>
                  ) : (
                    <div className="space-y-1">
                      <div className="flex items-center gap-1">
                        <label htmlFor={`channel-key-${draft.id}`} className="text-[13px] text-sp-text-secondary">
                          {t("settings.channels.apiKey")}
                        </label>
                        <HoverHintLabel hint={t("settings.channels.apiKeyHint")} focusable>
                          <Info
                            className="h-3.5 w-3.5 text-sp-text-muted"
                            aria-label={t("settings.channels.apiKeyHintLabel")}
                          />
                        </HoverHintLabel>
                        {draft.key_ready && (
                          <span className="ml-auto eo-settings-badge">
                            {t("settings.accounts.saved")}
                          </span>
                        )}
                      </div>
                      <div className="relative">
                        <input
                          id={`channel-key-${draft.id}`}
                          aria-label={t("settings.channels.apiKey")}
                          type={showApiKey ? "text" : "password"}
                          autoComplete="new-password"
                          placeholder="sk-..."
                          value={apiKeyInput}
                          disabled={saving}
                          onChange={(event) => {
                            invalidateProbe();
                            const nextKey = event.target.value;
                            apiKeyInputRef.current = nextKey;
                            setApiKeyInput(nextKey);
                            if (!isNewDraftRef.current) {
                              scheduleAutoSave(draftRef.current, nextKey, 500);
                            }
                          }}
                          onBlur={() => {
                            if (
                              !isNewDraftRef.current &&
                              initialDraftRef.current &&
                              apiKeyInputRef.current !== initialDraftRef.current.apiKey
                            ) {
                              scheduleAutoSave(draftRef.current, apiKeyInputRef.current, 0);
                            }
                          }}
                          style={{ paddingRight: "2.5rem" }}
                          className={cn(inputClass, "h-9 pr-10 font-mono")}
                        />
                        <button
                          type="button"
                          onClick={() => setShowApiKey((prev) => !prev)}
                          aria-label={
                            showApiKey
                              ? t("settings.channels.hideApiKey")
                              : t("settings.channels.showApiKey")
                          }
                          className="absolute right-2 top-1/2 -translate-y-1/2 rounded-sp-sm p-1 text-[13px] text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text cursor-pointer"
                        >
                          {showApiKey ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
                        </button>
                      </div>
                    </div>
                  )}
                    </>
                  )}

                  {/* 自动发现提示（DSH 识别渠道模型实时提示条） */}
                  {discoveredChatModels.length > 0 && !fetchDialogOpen && !isOfficialSelected && (
                    <div className="flex items-center justify-between gap-2 rounded-sp-sm border border-sp-line bg-sp-surface-muted px-3 py-2 text-xs text-sp-text-secondary">
                      <span>
                        {t("settings.channels.autoDiscoveredReady", {
                          count: discoveredChatModels.length,
                        })}
                      </span>
                      <button
                        type="button"
                        onClick={() => setFetchDialogOpen(true)}
                        className={quietButtonClass}
                      >
                        {t("settings.channels.applyDiscovered")}
                      </button>
                    </div>
                  )}

                  {/* 模型列表（统一复用 ChannelModelsEditor） */}
                  <ChannelModelsEditor
                    channelId={!isNewDraft ? draft.id : undefined}
                    models={draft.models}
                    protocol={draft.protocol}
                    disabled={saving}
                    fetching={isOfficialSelected ? syncingOfficialModels : loadingModels}
                    discoveredCount={isOfficialSelected ? 0 : discoveredChatModels.length}
                    fetchButtonLabel={
                      isOfficialSelected ? t("settings.channels.syncOfficialModels") : undefined
                    }
                    onFetch={() => {
                      if (isOfficialSelected) {
                        void handleSyncOfficialModels();
                      } else {
                        void handleFetchModels();
                      }
                    }}
                    onChange={updateModels}
                    onTestModel={(modelId) => void handleTestConnection(modelId)}
                    testingModelId={testing ? testingModelId : null}
                    fetchButtonRef={fetchModelsButtonRef}
                  />
                </div>
              </div>

              {/* 右侧面板底部：仅新建草稿时保留「取消 / 保存」操作栏，已有渠道随改随存 */}
              {isNewDraft && (
                <div className="flex flex-wrap items-center justify-between gap-2 pt-4 border-t border-sp-divider">
                  <button
                    type="button"
                    onClick={handleCancelDraft}
                    className={quietButtonClass}
                  >
                    {t("settings.channels.cancel")}
                  </button>
                  <button
                    type="button"
                    onClick={() => void handleSaveConfig()}
                    disabled={saving || deleting}
                    aria-busy={saving}
                    className="eo-settings-btn-primary min-w-[74px] justify-center disabled:opacity-50"
                    aria-label={t("settings.channels.saveConfig")}
                  >
                    {saving && showSavingSpinner ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin shrink-0" />
                    ) : (
                      <Check className="h-3.5 w-3.5 shrink-0" />
                    )}
                    <span>{t("settings.channels.save")}</span>
                  </button>
                </div>
              )}
            </div>
          ) : (
            <div className="flex min-h-64 items-center justify-center p-4">
              <EmptyState
                kind="first-use"
                size="page"
                visual={<EmptyStateWireframe variant="channel-setup" showAccentAction />}
                title={
                  channels.length === 0
                    ? t("settings.channels.empty")
                    : t("settings.channels.selectChannelPrompt")
                }
                description={t("settings.channels.noProviderSelectedDesc")}
              />
            </div>
          )}
        </div>
      </div>

      {/* 获取可用模型的浮层 (DSH 识别渠道模型勾选器) */}
      <ChannelModelsFetchDialog
        open={fetchDialogOpen}
        models={draft.models}
        ids={modelsList}
        loading={loadingModels}
        error={modelsError}
        onCancel={closeFetchDialog}
        onMerge={updateModels}
        triggerRef={fetchModelsButtonRef}
      />

      {pendingUnsavedAction && (
        <ModalOverlay
          role="dialog"
          aria-modal={true}
          aria-label={t("settings.channels.unsavedChanges")}
          data-testid="channel-unsaved-confirm-dialog"
          closeOnBackdropClick={false}
        >
          <div
            className="w-full max-w-sm rounded-modal border border-sp-line bg-sp-surface dark:bg-sp-card p-5 shadow-modal space-y-4"
            onClick={(e) => e.stopPropagation()}
          >
            <h3 className="text-sm font-semibold text-sp-text">
              {t("settings.channels.discardChannelTitle")}
            </h3>
            <p className="text-[13px] text-sp-text-secondary leading-relaxed">
              {t("settings.channels.unsavedChanges")}
            </p>
            <div className="flex items-center justify-end gap-2 pt-2">
              <button
                type="button"
                autoFocus
                onClick={closeUnsavedDialog}
                className={quietButtonClass}
              >
                {t("settings.channels.cancel")}
              </button>
              <button
                type="button"
                data-testid="confirm-discard-channel-edits-btn"
                onClick={() => {
                  const fn = pendingUnsavedAction;
                  setPendingUnsavedAction(null);
                  fn();
                }}
                className="inline-flex h-8 items-center rounded-sp-sm bg-sp-danger-strong px-3 text-[13px] font-semibold text-white hover:opacity-90 cursor-pointer outline-none focus-visible:ring-1 focus-visible:ring-sp-accent"
              >
                {t("settings.channels.discardChanges")}
              </button>
            </div>
          </div>
        </ModalOverlay>
      )}

      {renameDialogOpen && (
        <ModalOverlay
          role="dialog"
          aria-modal={true}
          aria-label={t("settings.channels.renameChannel")}
          data-testid="channel-rename-dialog"
          closeOnBackdropClick={false}
        >
          <div
            className="w-full max-w-sm rounded-modal border border-sp-line bg-sp-surface dark:bg-sp-card p-5 shadow-modal space-y-4"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-center justify-between gap-2">
              <div className="space-y-1">
                <h3 className="text-sm font-semibold text-sp-text">
                  {t("settings.channels.renameChannel")}
                </h3>
                <p className="text-xs text-sp-text-secondary leading-relaxed">
                  {t("settings.channels.renameChannelDesc")}
                </p>
              </div>
              <button
                type="button"
                disabled={renaming}
                aria-label={t("settings.channels.cancel")}
                onClick={() => setRenameDialogOpen(false)}
                className="flex h-7 w-7 shrink-0 items-center justify-center rounded-sp-sm text-sp-text-muted hover:bg-sp-hover hover:text-sp-text cursor-pointer disabled:opacity-40"
              >
                <X className="h-4 w-4" />
              </button>
            </div>
            <div className="space-y-2">
              <input
                type="text"
                autoFocus
                aria-label={t("settings.channels.renameNewNameAria")}
                data-testid="channel-rename-input"
                value={renameInput}
                disabled={renaming}
                onChange={(e) => {
                  setRenameInput(e.target.value);
                  if (renameError) setRenameError(null);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    void handleConfirmRename();
                  } else if (e.key === "Escape" && !renaming) {
                    e.preventDefault();
                    e.stopPropagation();
                    setRenameDialogOpen(false);
                  }
                }}
                placeholder={t("settings.channels.renamePlaceholder")}
                className={cn(inputClass, "h-9")}
              />
              {renameError && (
                <p className="text-xs text-sp-danger-text">{renameError}</p>
              )}
            </div>
            <div className="flex items-center justify-end gap-2 pt-1">
              <button
                type="button"
                disabled={renaming}
                onClick={() => setRenameDialogOpen(false)}
                className={quietButtonClass}
              >
                {t("settings.channels.cancel")}
              </button>
              <button
                type="button"
                data-testid="channel-rename-confirm-btn"
                disabled={renaming || !renameInput.trim()}
                aria-busy={renaming}
                onClick={() => void handleConfirmRename()}
                className="eo-settings-btn-primary min-w-[96px] justify-center disabled:opacity-50"
              >
                {renaming && showRenamingSpinner ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin shrink-0" />
                ) : (
                  <Check className="h-3.5 w-3.5 shrink-0" />
                )}
                <span>{t("settings.channels.renameConfirm")}</span>
              </button>
            </div>
          </div>
        </ModalOverlay>
      )}

      {(testResult || globalNotice) && (
        <div className="fixed bottom-6 right-6 z-50 flex w-full max-w-md flex-col gap-2 pointer-events-none">
          {testResult && (
            <div
              role="status"
              aria-live="polite"
              data-testid="channel-speed-test-hud"
              className="pointer-events-auto rounded-modal border border-sp-line bg-sp-surface dark:bg-sp-card p-4 shadow-modal space-y-2"
            >
              <div className="flex items-start justify-between gap-3">
                <div
                  className={cn(
                    "flex items-center gap-2 text-[13px] font-semibold",
                    testResult.ok ? "text-sp-success-text" : "text-sp-danger-text"
                  )}
                >
                  {testResult.ok ? (
                    <CheckCircle2 className="h-4 w-4 shrink-0" />
                  ) : (
                    <AlertCircle className="h-4 w-4 shrink-0" />
                  )}
                  <span>{formatChannelTestResult(testResult)}</span>
                </div>
                <button
                  type="button"
                  data-testid="close-speed-test-hud-btn"
                  aria-label={t("settings.channels.closeSpeedTestHud")}
                  onClick={() => {
                    clearTestToastTimer();
                    setTestResult(null);
                    setTestSnippet(null);
                    setTestedModelLabel(null);
                  }}
                  className="flex h-6 w-6 shrink-0 items-center justify-center rounded-sp-sm text-[13px] text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text cursor-pointer"
                >
                  <X className="h-3.5 w-3.5" />
                </button>
              </div>

              {(testedModelLabel ||
                typeof testResult.status === "number" ||
                typeof testResult.latency_ms === "number") && (
                <div className="flex flex-wrap items-center gap-2 text-xs text-sp-text-secondary">
                  {testedModelLabel && (
                    <span data-testid="speed-test-hud-model" className="eo-settings-badge font-mono">
                      {testedModelLabel}
                    </span>
                  )}
                  {typeof testResult.status === "number" && (
                    <span className="eo-settings-badge font-mono">
                      HTTP {testResult.status}
                    </span>
                  )}
                  {typeof testResult.latency_ms === "number" && (
                    <span className="inline-flex items-center gap-1 font-mono text-sp-text-secondary">
                      <Clock className="h-3.5 w-3.5" />
                      {t("settings.channels.testLatency", {
                        ms: String(testResult.latency_ms),
                      })}
                    </span>
                  )}
                </div>
              )}

              {testSnippet && (
                <div className="flex items-start gap-2 rounded-sp-sm bg-sp-surface-muted p-2 text-xs font-mono text-sp-text break-words">
                  <MessageSquare className="h-3.5 w-3.5 mt-1 shrink-0 text-sp-text-muted" />
                  <span>{t("settings.channels.testResponse", { text: testSnippet })}</span>
                </div>
              )}

              {!testResult.ok && baseUrlNeedsV1Hint(draft.base_url, draft.protocol) && (
                <div className="flex items-center justify-between gap-2 pt-1 border-t border-sp-divider text-[13px] text-sp-text-secondary">
                  <span>{t("settings.channels.missingV1")}</span>
                  <button
                    type="button"
                    onClick={() => {
                      invalidateProbe();
                      const nextDraft = {
                        ...draftRef.current,
                        base_url: appendV1ToBaseUrl(draftRef.current.base_url),
                      };
                      draftRef.current = nextDraft;
                      setDraft(nextDraft);
                      if (!isNewDraftRef.current) {
                        scheduleAutoSave(nextDraft, apiKeyInputRef.current, 0);
                      }
                      showNotice("info", t("settings.channels.appendedV1"));
                    }}
                    aria-label={t("settings.channels.appendV1")}
                    className={quietButtonClass}
                  >
                    {t("settings.channels.appendV1")}
                  </button>
                </div>
              )}
            </div>
          )}

          {globalNotice && (
            <div
              role="status"
              aria-live="polite"
              data-testid="channel-floating-notice"
              className={cn(
                "pointer-events-auto flex items-center justify-between gap-2 rounded-sp-md border border-sp-line px-3 py-2 text-[13px] shadow-overlay",
                globalNotice.type === "success" && "bg-sp-success-bg text-sp-success-text",
                globalNotice.type === "error" && "bg-sp-danger-bg text-sp-danger-text",
                globalNotice.type === "info" && "bg-sp-surface dark:bg-sp-card text-sp-text"
              )}
            >
              <div className="flex items-center gap-2">
                {globalNotice.type === "success" ? (
                  <CheckCircle2 className="h-4 w-4 shrink-0" />
                ) : (
                  <AlertCircle className="h-4 w-4 shrink-0" />
                )}
                <span>{globalNotice.text}</span>
              </div>
              <button
                type="button"
                aria-label={t("settings.channels.closeNotice")}
                onClick={() => {
                  clearNoticeTimer();
                  setGlobalNotice(null);
                }}
                className="flex h-6 w-6 shrink-0 items-center justify-center rounded-sp-sm text-[13px] text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text cursor-pointer"
              >
                <X className="h-3.5 w-3.5" />
              </button>
            </div>
          )}
        </div>
      )}

      <EOStageLoginModal
        open={officialLoginOpen}
        onClose={() => setOfficialLoginOpen(false)}
        onSuccess={(acct) => {
          setOfficialAccount(acct);
          setSelectedOfficialLoggedOut(false);
          void loadChannels().then(() => {
            setActiveChannelId(OFFICIAL_CHANNEL_ID);
          });
        }}
      />

      <EOStageBillingModal
        open={officialBillingOpen}
        initialTab="balance"
        account={officialAccount}
        onClose={() => setOfficialBillingOpen(false)}
      />
    </div>
  );
}

