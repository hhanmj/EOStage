import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ArrowLeft,
  Check,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  Compass,
  FolderOpen,
  Globe,
  Maximize2,
  Search,
  X,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { detectSystemLocale, type Locale, useI18n } from "@/i18n";
import {
  AtlasEarthCanvas,
  type GlobeClickPick,
  type GlobeLayoutMetrics,
  type HoverTargetInfo,
} from "./onboarding/AtlasEarthCanvas";
import {
  ATLAS_COUNTRIES_BY_ISO3,
  ATLAS_COUNTRY_LIST,
  type AtlasCountryEntry,
  CONTINENT_TABS,
  type ContinentId,
  type CountrySummaryRecord,
  type EoModalityKey,
  GLOBE_ASSET_URLS,
  searchAtlasTargets,
} from "./onboarding/onboardingAtlasData";
import { Stage1MosaicCanvas } from "./onboarding/Stage1MosaicCanvas";

const WELCOME_LOCALES: ReadonlyArray<{ code: Locale; nativeName: string }> = [
  { code: "zh-CN", nativeName: "简体中文" },
  { code: "en", nativeName: "English" },
  { code: "ja", nativeName: "日本語" },
];

export const WELCOME_ONBOARDING_STORAGE_KEY = "eostage.onboarding.welcomeCompleted";

export interface DesktopWelcomeOnboardingProps {
  open: boolean;
  onClose: () => void;
  /** User-configured cache directory from NetworkSettings.cache_dir (empty means default app data directory). */
  cacheDir?: string | null;
  /** Real application data cache directory resolved from get_cache_summary.cache_dir. */
  defaultCacheDir?: string | null;
  /** Deprecated legacy prop kept optional for compatibility. */
  installRootDir?: string | null;
  /** Pick and persist a custom cache directory via saveNetworkSettings({ cache_dir }). */
  onPickCacheDir?: () => Promise<void> | void;
  /** Reset cache directory back to the default app data directory (cache_dir = ""). */
  onResetCacheDir?: () => Promise<void> | void;
  /** Fallback if parent opens settings directly. */
  onOpenSettingsCategory?: (category: "storage" | "network_proxy" | "appearance") => void;
}

interface OnboardingCopy {
  dialogLabel: string;
  skip: string;
  stage1Tagline: string;
  languageTitle: string;
  languageAutoDesc: (name: string) => string;
  languageManualDesc: string;
  cacheDefaultDesc: string;
  cacheCustomDesc: string;
  changeBtn: string;
  resetDefaultBtn: string;
  continueBtn: string;
  enterAppShort: string;
  backToSetup: string;
  backToGlobe: string;
  continentMenuBtn: (count: number) => string;
  continentFilterPlaceholder: string;
  searchPlaceholder: string;
  noSearchResults: string;
  prevModality: string;
  nextModality: string;
  expandView: string;
  closeFullBleed: string;
}

const ONBOARDING_COPY: Record<Locale, OnboardingCopy> = {
  "zh-CN": {
    dialogLabel: "EOStage 欢迎引导",
    skip: "跳过",
    stage1Tagline: "多源遥感检索、空间预览与智能下载的一站式地球观测工作台。",
    languageTitle: "界面语言",
    languageAutoDesc: (name) => `默认跟随系统（已识别 ${name}）`,
    languageManualDesc: "已手动选择",
    cacheDefaultDesc: "数据缓存默认保存在应用数据目录",
    cacheCustomDesc: "已设为自定义数据缓存目录",
    changeBtn: "更改…",
    resetDefaultBtn: "恢复默认",
    continueBtn: "探索 3D 地球",
    enterAppShort: "进入软件",
    backToSetup: "返回设置",
    backToGlobe: "返回地球",
    continentMenuBtn: (n) => `按大洲挑选 · ${n} 国`,
    continentFilterPlaceholder: "筛选国家或代码…",
    searchPlaceholder: "搜索国家或经典地标…",
    noSearchResults: "未找到匹配的国家或遥感地点",
    prevModality: "上一数据类型",
    nextModality: "下一数据类型",
    expandView: "全屏查看高清影像",
    closeFullBleed: "关闭全屏",
  },
  en: {
    dialogLabel: "EOStage Welcome",
    skip: "Skip",
    stage1Tagline: "An all-in-one Earth observation workbench for multi-source search, spatial preview, and smart downloads.",
    languageTitle: "Interface Language",
    languageAutoDesc: (name) => `Follows OS automatically (${name})`,
    languageManualDesc: "Manual preference",
    cacheDefaultDesc: "Dataset cache defaults to your app data folder",
    cacheCustomDesc: "Custom dataset cache folder",
    changeBtn: "Change…",
    resetDefaultBtn: "Reset",
    continueBtn: "Explore 3D Earth",
    enterAppShort: "Launch App",
    backToSetup: "Setup",
    backToGlobe: "Globe",
    continentMenuBtn: (n) => `Continents · ${n}`,
    continentFilterPlaceholder: "Filter countries…",
    searchPlaceholder: "Search a country or EO site",
    noSearchResults: "No matching country or site",
    prevModality: "Previous data layer",
    nextModality: "Next data layer",
    expandView: "Open full-screen HD view",
    closeFullBleed: "Close full-screen",
  },
  ja: {
    dialogLabel: "EOStage ようこそ",
    skip: "スキップ",
    stage1Tagline: "多源衛星データの検索・空間プレビュー・スマートダウンロードを統合した地球観測ワークベンチ。",
    languageTitle: "表示言語",
    languageAutoDesc: (name) => `システム言語を自動認識（${name}）`,
    languageManualDesc: "手動選択",
    cacheDefaultDesc: "既定ではアプリデータフォルダにキャッシュを保存",
    cacheCustomDesc: "カスタムキャッシュ保存先",
    changeBtn: "変更…",
    resetDefaultBtn: "既定に戻す",
    continueBtn: "3D地球を探索",
    enterAppShort: "アプリへ",
    backToSetup: "初期設定",
    backToGlobe: "地球へ戻る",
    continentMenuBtn: (n) => `地域別に選ぶ · ${n}`,
    continentFilterPlaceholder: "国名・コードで絞り込み…",
    searchPlaceholder: "国・地域を検索…",
    noSearchResults: "一致する国・観測地がありません",
    prevModality: "前のデータ種別",
    nextModality: "次のデータ種別",
    expandView: "全画面で高精細画像を開く",
    closeFullBleed: "閉じる",
  },
};

function formatFriendlyCachePath(
  cacheDir?: string | null,
  defaultCacheDir?: string | null,
  installRootDir?: string | null
): {
  isDefault: boolean;
  displayPath: string;
  fullPath: string;
} {
  const custom = (cacheDir || "").trim();
  const defaultClean = (defaultCacheDir || "").trim();
  const installClean = (installRootDir || "").trim();
  const raw = custom || defaultClean || installClean;

  if (!raw) {
    return {
      isDefault: !custom,
      displayPath: "EOStage › cache",
      fullPath: "./cache",
    };
  }

  const normalized = raw.replace(/\\/g, "/");
  const segments = normalized.split("/").filter(Boolean);
  const tail =
    segments.length <= 3 ? segments.join(" › ") : `… › ${segments.slice(-2).join(" › ")}`;
  const useResolvedDirect = Boolean(custom || defaultClean);
  return {
    isDefault: !custom,
    displayPath: useResolvedDirect ? tail : `${tail} › cache`,
    fullPath: useResolvedDirect
      ? raw
      : `${raw}${raw.endsWith("/") || raw.endsWith("\\") ? "" : "/"}cache`,
  };
}

interface CountrySelectionState {
  country: AtlasCountryEntry;
  initialPlaceIndex: number;
}

const ATLAS_PHOTO_FALLBACK_COPY: Record<
  Locale,
  {
    packUnconfigured: string;
    loadFailed: string;
    retry: string;
  }
> = {
  "zh-CN": {
    packUnconfigured: "可选高清图包未配置（配置 VITE_EO_ATLAS_CDN_BASE_URL 可加载实景）",
    loadFailed: "遥感预览图加载失败",
    retry: "重试加载",
  },
  en: {
    packUnconfigured: "HD atlas pack not configured (set VITE_EO_ATLAS_CDN_BASE_URL to enable)",
    loadFailed: "Satellite preview failed to load",
    retry: "Retry",
  },
  ja: {
    packUnconfigured: "高解像度画像パック未設定（VITE_EO_ATLAS_CDN_BASE_URL で有効化）",
    loadFailed: "衛星プレビューの読み込みに失敗しました",
    retry: "再試行",
  },
};

/** Smooth-fading HD satellite image with honest unconfigured / load-error fallback */
export function SmoothEoPhoto({
  src,
  alt,
  locale,
  isActive = true,
  eager = false,
  className,
  onReady,
}: {
  src?: string;
  alt: string;
  locale: Locale;
  isActive?: boolean;
  eager?: boolean;
  className?: string;
  onReady?: () => void;
}) {
  const imgRef = useRef<HTMLImageElement | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [failed, setFailed] = useState(false);
  const [retryKey, setRetryKey] = useState(0);
  const fallbackCopy = ATLAS_PHOTO_FALLBACK_COPY[locale];

  useEffect(() => {
    setFailed(false);
    setRetryKey(0);
    if (!src) {
      setLoaded(false);
      if (isActive) {
        onReady?.();
      }
      return;
    }
    const el = imgRef.current;
    if (el && el.complete && el.naturalWidth > 0) {
      setLoaded(true);
      onReady?.();
    } else {
      setLoaded(false);
    }
  }, [src, isActive, onReady]);

  if (!isActive) {
    return <div aria-hidden="true" className={cn("bg-[#0b162a]", className)} />;
  }

  if (!src || failed) {
    const reason = !src ? "pack_unconfigured" : "load_error";
    return (
      <div
        data-testid="welcome-atlas-photo-fallback"
        data-fallback-reason={reason}
        className={cn(
          "relative overflow-hidden bg-[radial-gradient(circle_at_30%_25%,rgba(37,99,235,0.28),transparent_60%),linear-gradient(135deg,#0c1b35,#060f1f)]",
          className
        )}
      >
        <img
          src={GLOBE_ASSET_URLS.day}
          alt=""
          aria-hidden="true"
          className="h-full w-full object-cover opacity-28 saturate-75"
        />
        <div
          aria-hidden="true"
          className="pointer-events-none absolute inset-0 bg-[#040b18]/45"
        />
        <div className="pointer-events-none absolute inset-x-4 top-4 flex items-start justify-end gap-2">
          <span className="rounded-md border border-white/15 bg-black/45 px-2.5 py-1 text-xs font-medium text-white/80 backdrop-blur-md">
            {reason === "pack_unconfigured" ? fallbackCopy.packUnconfigured : fallbackCopy.loadFailed}
          </span>
          {reason === "load_error" && (
            <button
              type="button"
              data-testid="welcome-atlas-photo-retry"
              onClick={(e) => {
                e.stopPropagation();
                setFailed(false);
                setLoaded(false);
                setRetryKey((k) => k + 1);
              }}
              className="pointer-events-auto rounded-md border border-white/25 bg-black/60 px-2.5 py-1 text-xs font-medium text-white backdrop-blur-md transition-colors hover:bg-black/80 cursor-pointer"
            >
              {fallbackCopy.retry}
            </button>
          )}
        </div>
      </div>
    );
  }

  return (
    <img
      key={`${src}#${retryKey}`}
      ref={imgRef}
      src={src}
      alt={alt}
      loading={eager ? "eager" : "lazy"}
      decoding="async"
      onLoad={() => {
        setLoaded(true);
        onReady?.();
      }}
      onError={() => {
        setFailed(true);
        setLoaded(false);
      }}
      className={cn(
        "transition-opacity duration-500 ease-out",
        loaded ? "opacity-100" : "opacity-0",
        className
      )}
    />
  );
}

/** Interactive bottom dot indicator strip for switching EO data modalities on the active place */
function ModalityDotsBar({
  modalities,
  activeModality,
  locale,
  onSelect,
  testId = "welcome-modality-dots",
  className,
}: {
  modalities: readonly AtlasCountryEntry["places"][number]["modalities"][number][];
  activeModality: AtlasCountryEntry["places"][number]["modalities"][number];
  locale: Locale;
  onSelect: (key: EoModalityKey) => void;
  testId?: string;
  className?: string;
}) {
  if (modalities.length <= 1) return null;
  return (
    <div
      data-testid={testId}
      onClick={(e) => e.stopPropagation()}
      className={cn("pointer-events-auto flex items-center gap-2", className)}
    >
      {modalities.map((m) => {
        const active = m.key === activeModality.key;
        return (
          <button
            key={m.key}
            type="button"
            data-testid={`${testId === "welcome-modality-dots" ? "welcome-modality-dot" : "welcome-fullbleed-dot"}-${m.key}`}
            aria-label={m.label[locale]}
            aria-pressed={active}
            title={`${m.label[locale]} · ${m.badge}`}
            onClick={(e) => {
              e.stopPropagation();
              onSelect(m.key);
            }}
            className="group/dot relative flex h-5 items-center justify-center px-0.5 cursor-pointer"
          >
            <span
              className="pointer-events-none absolute bottom-full mb-2 whitespace-nowrap rounded-md border border-white/15 bg-[#060d1c]/92 px-2 py-0.5 text-xs font-medium text-white opacity-0 shadow-lg backdrop-blur-md transition-opacity duration-150 group-hover/dot:opacity-100 group-focus-visible/dot:opacity-100"
            >
              {m.label[locale]}
            </span>
            <span
              className={cn(
                "block h-1.5 rounded-full transition-all duration-300",
                active
                  ? "w-5 bg-white shadow-[0_0_10px_rgba(255,255,255,0.5)]"
                  : "w-1.5 bg-white/45 group-hover/dot:scale-125 group-hover/dot:bg-white/90"
              )}
            />
          </button>
        );
      })}
    </div>
  );
}

export function DesktopWelcomeOnboarding({
  open,
  onClose,
  cacheDir,
  defaultCacheDir,
  installRootDir,
  onPickCacheDir,
  onResetCacheDir,
  onOpenSettingsCategory,
}: DesktopWelcomeOnboardingProps) {
  const { locale, setLocale } = useI18n();
  const [stage, setStage] = useState<"welcome" | "atlas">("welcome");
  const [globeReady, setGlobeReady] = useState(false);
  const [countries, setCountries] = useState<readonly CountrySummaryRecord[]>([]);
  const [globeMetrics, setGlobeMetrics] = useState<GlobeLayoutMetrics>(() => {
    const w = typeof window !== "undefined" ? window.innerWidth : 1280;
    const h = typeof window !== "undefined" ? window.innerHeight : 800;
    return {
      globeCx: w < 720 ? w * 0.5 : w * 0.5 - 120,
      globeCy: h * 0.5,
      globeR: Math.min(h * 0.34, 280),
      cssW: w,
      cssH: h,
    };
  });

  // Page 2: Atlas Explorer states (imperative ref-driven hover label keeps WebGL pointermove at 60fps)
  const hoverLabelRef = useRef<HTMLDivElement | null>(null);
  const hoverTextRef = useRef<HTMLDivElement | null>(null);
  const fullBleedOpenRef = useRef(false);
  const [shownCountry, setShownCountry] = useState<CountrySelectionState | null>(null);
  const [outgoingCountry, setOutgoingCountry] = useState<CountrySelectionState | null>(null);
  const [panelClosing, setPanelClosing] = useState(false);
  const [placeIndex, setPlaceIndex] = useState(0);
  const [preferredModalityKey, setPreferredModalityKey] = useState<EoModalityKey>("optical");
  const [activePhotoReady, setActivePhotoReady] = useState(false);

  const [fullBleedOpen, setFullBleedOpen] = useState(false);
  fullBleedOpenRef.current = fullBleedOpen;
  const [fullBleedClosing, setFullBleedClosing] = useState(false);

  const handleHoverChange = useCallback((info: HoverTargetInfo | null) => {
    const box = hoverLabelRef.current;
    const labelEl = hoverTextRef.current;
    if (!box || !labelEl) return;
    if (!info || fullBleedOpenRef.current) {
      box.style.opacity = "0";
      box.dataset.hoverKind = "none";
      return;
    }
    const maxW = typeof window !== "undefined" ? window.innerWidth : 1280;
    const clampedX = Math.min(info.screenX + 18, Math.max(16, maxW - 220));
    const clampedY = Math.max(48, info.screenY - 34);
    box.style.transform = `translate3d(${clampedX}px, ${clampedY}px, 0)`;
    box.style.opacity = info.kind === "country" ? "1" : "0.6";
    box.dataset.hoverKind = info.kind;
    labelEl.textContent = info.name;
  }, []);

  useEffect(() => {
    if (fullBleedOpen) {
      handleHoverChange(null);
    }
  }, [fullBleedOpen, handleHoverChange]);

  // Navigation controls
  const [langMenuOpen, setLangMenuOpen] = useState(false);
  const [continentDrawerOpen, setContinentDrawerOpen] = useState(false);
  const [activeContinent, setActiveContinent] = useState<ContinentId>("asia");
  const [continentFilter, setContinentFilter] = useState("");
  const [searchQuery, setSearchQuery] = useState("");
  const [searchFocused, setSearchFocused] = useState(false);
  const [flyToRequest, setFlyToRequest] = useState<{ lat: number; lon: number; seq: number } | null>(null);

  const langMenuRef = useRef<HTMLDivElement | null>(null);
  const searchWrapRef = useRef<HTMLDivElement | null>(null);
  const continentWrapRef = useRef<HTMLDivElement | null>(null);

  const flySeqRef = useRef(0);
  const shownRef = useRef<CountrySelectionState | null>(null);
  const switchTimerRef = useRef<number | null>(null);
  const closeTimerRef = useRef<number | null>(null);
  const fullBleedTimerRef = useRef<number | null>(null);

  const detectedSystemLocale = useMemo(() => detectSystemLocale(), []);
  const detectedLocaleItem =
    WELCOME_LOCALES.find((item) => item.code === detectedSystemLocale) ?? WELCOME_LOCALES[0];
  const currentLangItem = WELCOME_LOCALES.find((item) => item.code === locale) ?? WELCOME_LOCALES[0];
  const copy = ONBOARDING_COPY[locale] ?? ONBOARDING_COPY["zh-CN"];

  const searchHits = useMemo(
    () => searchAtlasTargets(searchQuery, countries, locale, 8),
    [searchQuery, countries, locale]
  );

  const continentCountries = useMemo(() => {
    const q = continentFilter.trim().toLowerCase();
    return ATLAS_COUNTRY_LIST.filter((c) => {
      if (q) {
        return `${c.iso3} ${c.name["zh-CN"]} ${c.name.en} ${c.name.ja}`.toLowerCase().includes(q);
      }
      return c.continent === activeContinent;
    });
  }, [activeContinent, continentFilter]);

  // Reset cleanly on dialog open & clean up pending animation timers on close/unmount
  useEffect(() => {
    if (open) {
      setStage("welcome");
      setGlobeReady(false);
      setLangMenuOpen(false);
      setContinentDrawerOpen(false);
      setContinentFilter("");
      setShownCountry(null);
      setOutgoingCountry(null);
      setPanelClosing(false);
      setFullBleedOpen(false);
      setFullBleedClosing(false);
      setSearchQuery("");
      setSearchFocused(false);
      handleHoverChange(null);
      shownRef.current = null;
    }
    return () => {
      if (switchTimerRef.current !== null) {
        window.clearTimeout(switchTimerRef.current);
        switchTimerRef.current = null;
      }
      if (closeTimerRef.current !== null) {
        window.clearTimeout(closeTimerRef.current);
        closeTimerRef.current = null;
      }
      if (fullBleedTimerRef.current !== null) {
        window.clearTimeout(fullBleedTimerRef.current);
        fullBleedTimerRef.current = null;
      }
    };
  }, [open]);

  // Reset image readiness when switching places or modalities inside a country panel
  useEffect(() => {
    setActivePhotoReady(false);
  }, [placeIndex, preferredModalityKey, shownCountry?.country.iso3]);

  // Deferred Stage-2 WebGL Earth texture warm-cache after Stage-1 intro settles (sequential so frame rate stays locked)
  useEffect(() => {
    if (!open || stage !== "welcome") return;
    let cancelled = false;
    let pendingTimer: number | null = null;

    const waitMs = (ms: number): Promise<void> =>
      new Promise((resolve) => {
        pendingTimer = window.setTimeout(() => {
          pendingTimer = null;
          resolve();
        }, ms);
      });

    void (async () => {
      await waitMs(3800);
      for (const asset of [
        GLOBE_ASSET_URLS.day,
        GLOBE_ASSET_URLS.night,
        GLOBE_ASSET_URLS.specular,
        GLOBE_ASSET_URLS.clouds,
        GLOBE_ASSET_URLS.topology,
      ]) {
        if (cancelled) return;
        const img = new Image();
        img.decoding = "async";
        img.src = asset;
        await waitMs(350);
      }
      if (!cancelled) {
        void fetch(GLOBE_ASSET_URLS.countries).catch(() => {});
      }
    })();

    return () => {
      cancelled = true;
      if (pendingTimer !== null) {
        window.clearTimeout(pendingTimer);
      }
    };
  }, [open, stage]);

  const handleFinish = useCallback(() => {
    try {
      if (typeof window !== "undefined") {
        window.localStorage.setItem(WELCOME_ONBOARDING_STORAGE_KEY, "1");
      }
    } catch {
      // ignore storage errors
    }
    onClose();
  }, [onClose]);

  const enterAtlasStage = useCallback(() => {
    handleHoverChange(null);
    setLangMenuOpen(false);
    setGlobeReady(false);
    setStage("atlas");
  }, [handleHoverChange]);

  const handleOpenCountry = useCallback(
    (iso3: string, options?: { lat?: number; lon?: number; placeIndex?: number }) => {
      const countryEntry = ATLAS_COUNTRIES_BY_ISO3[iso3];
      if (!countryEntry) return;

      const pIdx = Math.max(0, Math.min(countryEntry.places.length - 1, options?.placeIndex ?? 0));
      const targetPlace = countryEntry.places[pIdx];
      const globeSummary = countries.find((c) => c.iso3 === iso3);

      const flyLat = options?.lat ?? globeSummary?.labelLat ?? targetPlace?.lat ?? 0;
      const flyLon = options?.lon ?? globeSummary?.labelLon ?? targetPlace?.lon ?? 0;
      flySeqRef.current += 1;
      setFlyToRequest({ lat: flyLat, lon: flyLon, seq: flySeqRef.current });

      setContinentDrawerOpen(false);
      setContinentFilter("");
      setSearchQuery("");
      setSearchFocused(false);
      setActivePhotoReady(false);

      if (closeTimerRef.current !== null) {
        window.clearTimeout(closeTimerRef.current);
        closeTimerRef.current = null;
      }

      const next: CountrySelectionState = {
        country: countryEntry,
        initialPlaceIndex: pIdx,
      };
      const prev = shownRef.current;

      if (prev && prev.country.iso3 !== countryEntry.iso3) {
        if (switchTimerRef.current !== null) {
          window.clearTimeout(switchTimerRef.current);
        }
        setOutgoingCountry(prev);
        switchTimerRef.current = window.setTimeout(() => {
          setOutgoingCountry(null);
          switchTimerRef.current = null;
        }, 580);
      } else {
        setOutgoingCountry(null);
      }

      shownRef.current = next;
      setPanelClosing(false);
      setShownCountry(next);
      setPlaceIndex(pIdx);
    },
    [countries]
  );

  const handleGlobeSelectLocation = useCallback(
    (pick: GlobeClickPick) => {
      if (stage !== "atlas" || !pick.iso3) return;
      handleOpenCountry(pick.iso3, { lat: pick.lat, lon: pick.lon });
    },
    [stage, handleOpenCountry]
  );

  const handleCloseCountry = useCallback(() => {
    if (!shownCountry) return;
    if (closeTimerRef.current !== null) {
      window.clearTimeout(closeTimerRef.current);
    }
    setPanelClosing(true);
    closeTimerRef.current = window.setTimeout(() => {
      closeTimerRef.current = null;
      setShownCountry(null);
      setOutgoingCountry(null);
      setPanelClosing(false);
      shownRef.current = null;
    }, 430);
  }, [shownCountry]);

  const handleCloseFullBleed = useCallback(() => {
    if (!fullBleedOpen) return;
    if (fullBleedTimerRef.current !== null) {
      window.clearTimeout(fullBleedTimerRef.current);
    }
    setFullBleedClosing(true);
    fullBleedTimerRef.current = window.setTimeout(() => {
      fullBleedTimerRef.current = null;
      setFullBleedOpen(false);
      setFullBleedClosing(false);
    }, 350);
  }, [fullBleedOpen]);

  const currentPlace = shownCountry
    ? shownCountry.country.places[placeIndex % shownCountry.country.places.length] ??
      shownCountry.country.places[0]
    : null;

  const activeModality = currentPlace
    ? currentPlace.modalities.find((m) => m.key === preferredModalityKey) ?? currentPlace.modalities[0]
    : null;

  const activeModalityIndex =
    currentPlace && activeModality
      ? Math.max(0, currentPlace.modalities.findIndex((m) => m.key === activeModality.key))
      : 0;

  // Cloudflare-friendly single next-dot prefetch ONLY after the active HD image has finished loading
  useEffect(() => {
    if (!activePhotoReady || !currentPlace || currentPlace.modalities.length <= 1) return;
    const nextMod =
      currentPlace.modalities[(activeModalityIndex + 1) % currentPlace.modalities.length];
    if (!nextMod?.src) return;
    const img = new Image();
    img.src = nextMod.src;
  }, [activePhotoReady, currentPlace, activeModalityIndex]);

  // Multi-level Esc + Arrow keyboard navigation
  useEffect(() => {
    if (!open) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        if (langMenuOpen) {
          setLangMenuOpen(false);
          return;
        }
        if (fullBleedOpen) {
          handleCloseFullBleed();
          return;
        }
        if (shownCountry) {
          handleCloseCountry();
          return;
        }
        if (continentDrawerOpen) {
          setContinentDrawerOpen(false);
          setContinentFilter("");
          return;
        }
        if (searchFocused || searchQuery) {
          setSearchQuery("");
          setSearchFocused(false);
          return;
        }
        if (stage === "atlas") {
          handleHoverChange(null);
          setStage("welcome");
          return;
        }
        handleFinish();
        return;
      }

      if (e.key === "Enter" && stage === "welcome" && !langMenuOpen) {
        const activeTag = (document.activeElement as HTMLElement | null)?.tagName;
        if (activeTag !== "BUTTON" && activeTag !== "INPUT") {
          e.preventDefault();
          enterAtlasStage();
          return;
        }
      }

      if (stage === "atlas" && shownCountry && currentPlace) {
        const activeTag = (document.activeElement as HTMLElement | null)?.tagName;
        if (activeTag === "INPUT" || activeTag === "TEXTAREA") {
          return;
        }
        const totalPlaces = shownCountry.country.places.length;
        if (e.key === "ArrowLeft") {
          if (e.shiftKey && totalPlaces > 1) {
            setPlaceIndex((i) => (i - 1 + totalPlaces) % totalPlaces);
          } else if (currentPlace.modalities.length > 1) {
            const prevIdx =
              (activeModalityIndex - 1 + currentPlace.modalities.length) %
              currentPlace.modalities.length;
            setPreferredModalityKey(currentPlace.modalities[prevIdx].key);
          }
        } else if (e.key === "ArrowRight") {
          if (e.shiftKey && totalPlaces > 1) {
            setPlaceIndex((i) => (i + 1) % totalPlaces);
          } else if (currentPlace.modalities.length > 1) {
            const nextIdx = (activeModalityIndex + 1) % currentPlace.modalities.length;
            setPreferredModalityKey(currentPlace.modalities[nextIdx].key);
          }
        }
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [
    open,
    stage,
    langMenuOpen,
    fullBleedOpen,
    shownCountry,
    currentPlace,
    activeModalityIndex,
    continentDrawerOpen,
    searchFocused,
    searchQuery,
    handleCloseFullBleed,
    handleCloseCountry,
    enterAtlasStage,
    handleFinish,
  ]);

  useEffect(() => {
    if (!open) return;
    const onMouseDown = (e: MouseEvent) => {
      if (langMenuOpen && langMenuRef.current && !langMenuRef.current.contains(e.target as Node)) {
        setLangMenuOpen(false);
      }
      if (searchFocused && searchWrapRef.current && !searchWrapRef.current.contains(e.target as Node)) {
        setSearchFocused(false);
      }
      if (
        continentDrawerOpen &&
        continentWrapRef.current &&
        !continentWrapRef.current.contains(e.target as Node)
      ) {
        setContinentDrawerOpen(false);
        setContinentFilter("");
      }
    };
    document.addEventListener("mousedown", onMouseDown);
    return () => document.removeEventListener("mousedown", onMouseDown);
  }, [open, langMenuOpen, searchFocused, continentDrawerOpen]);

  if (!open) return null;

  const cachePath = formatFriendlyCachePath(cacheDir, defaultCacheDir, installRootDir);
  const isUsingSystemLocale = locale === detectedSystemLocale;
  const isStage1 = stage === "welcome";

  // Dynamic E 🌐 Stage wordmark proportions kept cleanly symmetric around sphere limb across all window sizes
  const wordFontSize = Math.round(Math.max(40, globeMetrics.globeR * 0.58));
  const tuckFactor = 0.94;
  const leftWordRight = globeMetrics.cssW - (globeMetrics.globeCx - globeMetrics.globeR * tuckFactor);
  const rightWordLeft = globeMetrics.globeCx + globeMetrics.globeR * tuckFactor;

  return (
    <div
      data-testid="desktop-welcome-onboarding"
      role="dialog"
      aria-modal="true"
      aria-label={copy.dialogLabel}
      className="fixed inset-0 z-[2600] overflow-hidden select-none text-white touch-none"
      style={{
        background: isStage1
          ? "radial-gradient(120% 108% at 50% 44%, #0a1a30 0%, #05101f 46%, #020812 78%, #01040c 100%)"
          : "radial-gradient(100% 100% at 50% 48%, #04070e 0%, #010205 42%, #000000 74%, #000000 100%)",
      }}
    >
      {/* ===================================================================
          STAGE 1: HARDWARE-ACCELERATED 60FPS CANVAS PULLBACK + BREATHING MOSAIC
         =================================================================== */}
      {isStage1 && (
        <>
          <Stage1MosaicCanvas active={open && isStage1} />

          <div data-testid="welcome-stage1-foreground" className="pointer-events-none fixed inset-0 z-30">
            {/* Top-right Skip */}
            <button
              type="button"
              data-testid="welcome-onboarding-skip-btn"
              onClick={handleFinish}
              className="eo-stage1-chrome-in pointer-events-auto fixed right-6 top-5 flex h-8 items-center justify-center rounded-[8px] border border-white/20 bg-black/48 px-3.5 text-xs font-medium text-white/90 backdrop-blur-sm transition-colors hover:bg-black/70 hover:text-white cursor-pointer"
            >
              {copy.skip}
            </button>

            {/* Vertically Centered Quiet Setup Column */}
            <div className="fixed inset-0 flex flex-col items-center justify-center px-5">
              {/* Official C-2 Icon + Title + Tagline (softly materializes first) */}
              <div className="eo-stage1-brand-emerge flex flex-col items-center text-center mb-6">
                <div className="flex h-[60px] w-[60px] items-center justify-center overflow-hidden rounded-[15px] bg-white/[0.84] p-1.5 shadow-[0_16px_40px_rgba(0,0,0,0.48),inset_0_1px_0_rgba(255,255,255,1)] backdrop-blur-sm">
                  <img
                    src="/app-symbol-256.png"
                    alt="EOStage"
                    data-testid="welcome-real-app-icon"
                    className="h-full w-full object-contain"
                  />
                </div>
                <h1 className="mt-3 text-[32px] font-semibold leading-tight tracking-tight text-white drop-shadow-[0_2px_20px_rgba(0,0,0,0.75)]">
                  EOStage
                </h1>
                <p className="mt-1.5 max-w-[460px] text-[13px] leading-relaxed text-white/85 drop-shadow-[0_1px_14px_rgba(0,0,0,0.8)]">
                  {copy.stage1Tagline}
                </p>
              </div>

              {/* Translucent Frosted-Glass Setup Cards (softly fade in after brand) */}
              <div className="eo-stage1-glass-rise pointer-events-auto flex w-full max-w-[520px] flex-col gap-3">
                {/* Card 1: Interface Language */}
                <div
                  ref={langMenuRef}
                  data-testid="welcome-card-language"
                  className="relative flex items-center justify-between gap-3 rounded-[16px] border border-white/65 bg-white/[0.70] px-4 py-3.5 text-slate-900 shadow-[0_22px_54px_rgba(0,0,0,0.42),inset_0_1px_0_rgba(255,255,255,0.88)] backdrop-blur-[14px] backdrop-saturate-[1.6]"
                >
                  <div className="flex min-w-0 items-center gap-3">
                    <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-slate-900/[0.06] text-slate-700">
                      <Globe className="h-4 w-4" />
                    </div>
                    <div className="min-w-0">
                      <div className="text-[13px] font-semibold leading-tight text-slate-900">
                        {copy.languageTitle}
                      </div>
                      <div className="mt-0.5 truncate text-xs text-slate-600">
                        {isUsingSystemLocale
                          ? copy.languageAutoDesc(detectedLocaleItem.nativeName)
                          : copy.languageManualDesc}
                      </div>
                    </div>
                  </div>
                  <button
                    type="button"
                    data-testid="welcome-lang-trigger"
                    aria-haspopup="listbox"
                    aria-expanded={langMenuOpen}
                    onClick={() => setLangMenuOpen((v) => !v)}
                    className="flex h-8 shrink-0 items-center gap-1.5 rounded-[9px] border border-slate-900/12 bg-white/72 px-3 text-xs font-medium text-slate-800 shadow-xs transition-colors hover:bg-white/95 cursor-pointer"
                  >
                    <span>{currentLangItem.nativeName}</span>
                    <ChevronDown className="h-3.5 w-3.5 text-slate-500" />
                  </button>

                  {langMenuOpen && (
                    <div
                      role="listbox"
                      data-testid="welcome-lang-menu"
                      className="absolute bottom-full right-0 z-50 mb-2 w-40 rounded-xl border border-white/75 bg-white/[0.84] p-1.5 shadow-2xl backdrop-blur-[14px] backdrop-saturate-150"
                    >
                      {WELCOME_LOCALES.map((item) => {
                        const active = item.code === locale;
                        return (
                          <button
                            key={item.code}
                            type="button"
                            role="option"
                            aria-selected={active}
                            data-testid={`welcome-lang-option-${item.code}`}
                            onClick={() => {
                              setLocale(item.code);
                              setLangMenuOpen(false);
                            }}
                            className={cn(
                              "flex h-8 w-full items-center justify-between rounded-[7px] px-2.5 text-left text-xs text-slate-800 transition-colors hover:bg-slate-100 cursor-pointer",
                              active && "bg-slate-100 font-semibold"
                            )}
                          >
                            <span>{item.nativeName}</span>
                            {active && <Check className="h-3.5 w-3.5 text-slate-900" />}
                          </button>
                        );
                      })}
                    </div>
                  )}
                </div>

                {/* Card 2: Cache Directory (Install Dir default) */}
                <div
                  data-testid="welcome-card-cache"
                  className="flex items-center justify-between gap-3 rounded-[16px] border border-white/65 bg-white/[0.70] px-4 py-3.5 text-slate-900 shadow-[0_22px_54px_rgba(0,0,0,0.42),inset_0_1px_0_rgba(255,255,255,0.88)] backdrop-blur-[14px] backdrop-saturate-[1.6]"
                >
                  <div className="flex min-w-0 items-center gap-3">
                    <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-slate-900/[0.06] text-slate-700">
                      <FolderOpen className="h-4 w-4" />
                    </div>
                    <div className="min-w-0">
                      <div
                        data-testid="welcome-cache-path-display"
                        title={cachePath.fullPath}
                        className="truncate text-[13px] font-semibold leading-tight text-slate-900"
                      >
                        {cachePath.displayPath}
                      </div>
                      <div className="mt-0.5 truncate text-xs text-slate-600">
                        {cachePath.isDefault ? copy.cacheDefaultDesc : copy.cacheCustomDesc}
                      </div>
                    </div>
                  </div>
                  <div className="flex shrink-0 items-center gap-1.5">
                    {!cachePath.isDefault && onResetCacheDir && (
                      <button
                        type="button"
                        data-testid="welcome-reset-cache-btn"
                        onClick={() => void onResetCacheDir()}
                        className="flex h-8 items-center justify-center rounded-[8px] px-2.5 text-xs text-slate-600 transition-colors hover:bg-slate-100 hover:text-slate-900 cursor-pointer"
                      >
                        {copy.resetDefaultBtn}
                      </button>
                    )}
                    <button
                      type="button"
                      data-testid="welcome-pick-cache-btn"
                      onClick={() => {
                        if (onPickCacheDir) {
                          void onPickCacheDir();
                        } else if (onOpenSettingsCategory) {
                          handleFinish();
                          onOpenSettingsCategory("storage");
                        }
                      }}
                      className="flex h-8 items-center justify-center rounded-[9px] border border-slate-900/12 bg-white/72 px-3 text-xs font-medium text-slate-800 shadow-xs transition-colors hover:bg-white/95 cursor-pointer"
                    >
                      {copy.changeBtn}
                    </button>
                  </div>
                </div>
              </div>
            </div>

            {/* Bottom-right Continue -> Stage 2 Interactive 3D Earth */}
            <button
              type="button"
              data-testid="welcome-onboarding-continue-btn"
              onClick={enterAtlasStage}
              className="eo-stage1-chrome-in pointer-events-auto fixed bottom-6 right-6 flex h-9 items-center gap-2 rounded-[10px] border border-white/25 bg-[#121826]/78 px-4 text-xs font-semibold text-white shadow-[0_14px_34px_rgba(0,0,0,0.48)] backdrop-blur-sm transition-colors hover:bg-[#1e273c]/92 cursor-pointer"
            >
              <span>{copy.continueBtn}</span>
            </button>
          </div>
        </>
      )}

      {/* ===================================================================
          STAGE 2: PURE INTERACTIVE 3D EARTH EXPLORER (176 COUNTRIES HD)
         =================================================================== */}
      {!isStage1 && (
        <>
          {/* 5% Fractal-Noise Grain Layer on dark space backdrop only (eliminates 8-bit banding) */}
          <div
            aria-hidden="true"
            className="pointer-events-none fixed inset-0 z-[1] opacity-[0.05]"
            style={{
              backgroundImage:
                "url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='140' height='140'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='0.85' numOctaves='2' stitchTiles='stitch'/%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23n)'/%3E%3C/svg%3E\")",
            }}
          />
          {/* Occluded E 🌐 Stage Wordmark behind z-10 sphere (strictly consistent lowercase 'tage') */}
          <div
            aria-hidden="true"
            data-testid="welcome-atlas-wordmark"
            className={cn(
              "pointer-events-none fixed inset-0 z-[5] transition-all duration-[850ms] ease-[cubic-bezier(0.16,1,0.3,1)]",
              globeReady && !shownCountry ? "opacity-100 translate-y-0" : "opacity-0 translate-y-3"
            )}
          >
            <div className="hidden md:block relative h-full w-full">
              <span
                className="absolute font-semibold tracking-tight text-[#eaf1ff] leading-[0.9]"
                style={{
                  top: "50%",
                  transform: "translateY(-50%)",
                  right: `${leftWordRight}px`,
                  fontSize: `${wordFontSize}px`,
                  textShadow: "0 4px 60px rgba(3, 16, 40, 0.52)",
                }}
              >
                E
              </span>
              <span
                className="absolute font-semibold tracking-tight text-[#eaf1ff] leading-[0.9]"
                style={{
                  top: "50%",
                  transform: "translateY(-50%)",
                  left: `${rightWordLeft}px`,
                  fontSize: `${wordFontSize}px`,
                  textShadow: "0 4px 60px rgba(3, 16, 40, 0.52)",
                }}
              >
                Stage
              </span>
            </div>
            <div className="md:hidden absolute inset-x-0 top-[11%] text-center text-[32px] font-semibold tracking-[0.14em] text-[#eaf1ff]/88">
              EOStage
            </div>
          </div>

          <AtlasEarthCanvas
            locale={locale}
            panelOpen={Boolean(shownCountry)}
            selectedCountryIso3={shownCountry?.country.iso3 ?? null}
            flyToRequest={flyToRequest}
            onReady={(loadedCountries) => {
              setCountries(loadedCountries);
              setGlobeReady(true);
            }}
            onLayoutMetrics={setGlobeMetrics}
            onHoverChange={handleHoverChange}
            onSelectLocation={handleGlobeSelectLocation}
          />

          {/* Soft vignette & Stage-2 entrance veil */}
          <div
            aria-hidden="true"
            className="pointer-events-none fixed inset-0 z-20"
            style={{ boxShadow: "inset 0 0 180px 40px rgba(0, 0, 0, 0.55)" }}
          />
          {!globeReady && (
            <div
              aria-hidden="true"
              className="eo-globe-pulse pointer-events-none fixed bottom-6 left-6 z-20 h-2.5 w-2.5 rounded-full bg-white/80"
            />
          )}
          <div
            aria-hidden="true"
            className={cn(
              "pointer-events-none fixed inset-0 z-[60] bg-[#000000] transition-opacity duration-[950ms] ease-out delay-100",
              globeReady ? "opacity-0" : "opacity-100"
            )}
          />

          {/* Always-Active Hover Country / Ocean Cursor Trailing Label (z-35; works even with country panel open) */}
          <div
            ref={hoverLabelRef}
            aria-hidden="true"
            data-testid="welcome-atlas-hover-label"
            data-hover-kind="none"
            className="pointer-events-none fixed left-0 top-0 z-[35] transition-opacity duration-200"
            style={{ opacity: 0 }}
          >
            <div
              ref={hoverTextRef}
              className="whitespace-nowrap font-semibold uppercase tracking-[0.1em] text-white"
              style={{
                fontSize: "clamp(0.95rem, 1.9vw, 1.5rem)",
                textShadow: "0 2px 22px rgba(0, 0, 0, 0.65)",
              }}
            />
          </div>

          {/* Stage-2 Idle Chrome (Back to Setup, 7-Continent 176 Picker, Minimal Search, Enter App) */}
          {!shownCountry && (
            <div className="pointer-events-none fixed inset-0 z-40">
              <div
                ref={continentWrapRef}
                className="pointer-events-auto fixed left-6 top-6 flex flex-wrap items-center gap-2.5"
              >
                <button
                  type="button"
                  data-testid="welcome-atlas-back-setup-btn"
                  onClick={() => {
                    handleHoverChange(null);
                    setContinentDrawerOpen(false);
                    setContinentFilter("");
                    setStage("welcome");
                  }}
                  className="flex h-8 items-center gap-1.5 rounded-full border border-white/20 bg-[#060d1a]/75 px-3 text-xs font-medium tracking-wide text-white/85 backdrop-blur-md transition-colors hover:bg-white/15 hover:text-white cursor-pointer"
                >
                  <ArrowLeft className="h-3.5 w-3.5" />
                  <span>{copy.backToSetup}</span>
                </button>

                <button
                  type="button"
                  data-testid="welcome-continent-toggle"
                  onClick={() => {
                    setContinentDrawerOpen((v) => {
                      if (v) {
                        setContinentFilter("");
                      }
                      return !v;
                    });
                  }}
                  aria-expanded={continentDrawerOpen}
                  className={cn(
                    "flex h-8 items-center gap-1.5 rounded-full border px-3.5 text-xs font-medium backdrop-blur-md transition-colors cursor-pointer",
                    continentDrawerOpen
                      ? "border-white/55 bg-white text-[#071124]"
                      : "border-white/20 bg-[#060d1a]/75 text-white/90 hover:border-white/40 hover:bg-white/12"
                  )}
                >
                  <Compass className="h-3.5 w-3.5" />
                  <span>{copy.continentMenuBtn(ATLAS_COUNTRY_LIST.length)}</span>
                </button>

                {continentDrawerOpen && (
                  <div
                    data-testid="welcome-continent-drawer"
                    className="eo-globe-fade fixed left-6 top-16 z-50 flex max-h-[68vh] w-[min(92vw,420px)] flex-col overflow-hidden rounded-2xl border border-white/12 bg-[#060d1a]/92 p-3 shadow-[0_24px_60px_rgba(0,0,0,0.85)] backdrop-blur-xl"
                  >
                    <div className="mb-2.5 flex items-center gap-2 border-b border-white/15 pb-2">
                      <Search className="h-3.5 w-3.5 text-white/55" />
                      <input
                        type="text"
                        data-testid="welcome-continent-filter-input"
                        value={continentFilter}
                        onChange={(e) => setContinentFilter(e.target.value)}
                        placeholder={copy.continentFilterPlaceholder}
                        className="w-full bg-transparent text-xs text-white placeholder:text-white/45 focus:outline-none"
                      />
                      {continentFilter && (
                        <button
                          type="button"
                          onClick={() => setContinentFilter("")}
                          className="text-white/55 hover:text-white cursor-pointer"
                        >
                          <X className="h-3.5 w-3.5" />
                        </button>
                      )}
                    </div>

                    {!continentFilter && (
                      <div className="mb-2.5 flex flex-wrap gap-1">
                        {CONTINENT_TABS.map((tab) => {
                          const active = activeContinent === tab.id;
                          return (
                            <button
                              key={tab.id}
                              type="button"
                              data-testid={`welcome-continent-tab-${tab.id}`}
                              onClick={() => setActiveContinent(tab.id)}
                              className={cn(
                                "rounded-full px-2.5 py-1 text-xs transition-colors cursor-pointer",
                                active
                                  ? "bg-white text-[#071124] font-semibold"
                                  : "bg-white/[0.07] text-white/75 hover:bg-white/15 hover:text-white"
                              )}
                            >
                              {tab.labels[locale]}
                            </button>
                          );
                        })}
                      </div>
                    )}

                    <div className="grid grid-cols-2 gap-1 overflow-y-auto pr-1">
                      {continentCountries.map((c) => (
                        <button
                          key={c.iso3}
                          type="button"
                          data-testid={`welcome-country-pick-${c.iso3}`}
                          onClick={() => handleOpenCountry(c.iso3)}
                          className="flex items-center justify-between gap-2 rounded-lg px-2.5 py-1.5 text-left text-xs text-white/85 transition-colors hover:bg-white/12 hover:text-white cursor-pointer"
                        >
                          <span className="truncate">{c.name[locale]}</span>
                        </button>
                      ))}
                    </div>
                  </div>
                )}
              </div>

              {/* Minimal Underline Search Field */}
              <div
                ref={searchWrapRef}
                className="pointer-events-auto fixed bottom-6 left-5 w-[calc(100vw-9rem)] md:bottom-auto md:left-auto md:right-7 md:top-7 md:w-[min(78vw,15.5rem)]"
              >
                <div
                  className={cn(
                    "flex items-center gap-2 border-b pb-1.5 transition-colors",
                    searchFocused ? "border-white/80" : "border-white/25 hover:border-white/50"
                  )}
                >
                  <Search className="h-3.5 w-3.5 shrink-0 text-white/65" />
                  <input
                    type="text"
                    data-testid="welcome-atlas-search-input"
                    value={searchQuery}
                    placeholder={copy.searchPlaceholder}
                    onFocus={() => setSearchFocused(true)}
                    onChange={(e) => {
                      setSearchQuery(e.target.value);
                      setSearchFocused(true);
                    }}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" && searchHits.length > 0) {
                        e.preventDefault();
                        const hit = searchHits[0];
                        handleOpenCountry(hit.iso3, {
                          lat: hit.lat,
                          lon: hit.lon,
                          placeIndex: hit.placeIndex,
                        });
                      }
                    }}
                    className="w-full bg-transparent text-xs tracking-wide text-white placeholder:text-white/45 focus:outline-none"
                  />
                  {searchQuery && (
                    <button
                      type="button"
                      aria-label="Clear search"
                      onClick={() => setSearchQuery("")}
                      className="text-white/55 hover:text-white cursor-pointer"
                    >
                      <X className="h-3.5 w-3.5" />
                    </button>
                  )}
                </div>

                {searchFocused && searchQuery.trim().length > 0 && (
                  <div
                    data-testid="welcome-atlas-search-results"
                    className="absolute right-0 bottom-full mb-2 md:bottom-auto md:top-full md:mt-2 w-72 overflow-hidden rounded-[12px] border border-white/[0.08] bg-[#060d1a]/88 p-1.5 shadow-2xl backdrop-blur-xl"
                  >
                    {searchHits.length === 0 ? (
                      <div role="status" aria-live="polite" className="px-3 py-2 text-xs text-white/60">
                        {copy.noSearchResults}
                      </div>
                    ) : (
                      searchHits.map((hit) => (
                        <button
                          key={hit.id}
                          type="button"
                          data-testid={`welcome-atlas-search-hit-${hit.id}`}
                          onClick={() =>
                            handleOpenCountry(hit.iso3, {
                              lat: hit.lat,
                              lon: hit.lon,
                              placeIndex: hit.placeIndex,
                            })
                          }
                          className="flex w-full items-center justify-between gap-2 rounded-[8px] px-2.5 py-2 text-left transition-colors hover:bg-white/10 cursor-pointer"
                        >
                          <div className="min-w-0">
                            <div className="truncate text-xs font-medium text-white">{hit.label}</div>
                            <div className="truncate text-xs text-white/55">{hit.sublabel}</div>
                          </div>
                          <ChevronRight className="h-3.5 w-3.5 shrink-0 text-white/45" />
                        </button>
                      ))
                    )}
                  </div>
                )}
              </div>
            </div>
          )}

          {/* Always-visible Enter App Button across Stage 2 (desktop bottom-right; mobile top-right on globe idle) */}
          {!fullBleedOpen && (
            <button
              type="button"
              data-testid="welcome-stage2-enter-app-btn"
              onClick={handleFinish}
              className={cn(
                "pointer-events-auto fixed bottom-6 right-5 z-40 items-center gap-2 rounded-full border border-white/25 bg-[#060d1a]/82 px-4 text-xs font-medium text-white/95 shadow-[0_8px_24px_rgba(0,0,0,0.45)] backdrop-blur-md transition-colors hover:bg-white hover:text-[#071124] cursor-pointer md:right-6 h-8 md:h-9",
                shownCountry ? "hidden md:flex" : "flex"
              )}
            >
              <span>{copy.enterAppShort}</span>
            </button>
          )}

          {/* Open Country State: Enlarged Frameless 16:10 Inspector (< > Modalities, Numbered Rows Places) */}
          {shownCountry && currentPlace && activeModality && (
            <div
              className={cn(
                "pointer-events-none fixed inset-0 z-30",
                panelClosing && "eo-globe-panel-out"
              )}
            >
              {/* ← Back to Globe Button (top-left) */}
              <button
                type="button"
                data-testid="welcome-atlas-back-btn"
                onClick={handleCloseCountry}
                className="eo-globe-fade pointer-events-auto fixed left-5 top-[72px] md:left-8 md:top-8 flex items-center gap-2 text-xs font-medium uppercase tracking-[0.16em] text-white/80 transition-colors hover:text-white cursor-pointer"
              >
                <ArrowLeft className="h-3.5 w-3.5" />
                <span>{copy.backToGlobe}</span>
              </button>

              {/* Desktop Low-Left Big Country Name + Tagline */}
              <div className="pointer-events-none hidden md:block fixed bottom-[5.2%] left-[5%] max-w-[42vw]">
                {outgoingCountry && (
                  <div className="eo-globe-name-out absolute inset-x-0 bottom-0">
                    <div
                      className="font-semibold uppercase leading-[0.95] text-white/[0.96]"
                      style={{
                        fontSize: "clamp(1.9rem, 4.6vw, 4.4rem)",
                        textShadow: "0 8px 60px rgba(3, 16, 40, 0.55)",
                      }}
                    >
                      {outgoingCountry.country.name[locale]}
                    </div>
                  </div>
                )}
                <div key={shownCountry.country.iso3} className="eo-globe-name-in">
                  <div
                    className="font-semibold uppercase leading-[0.95] text-white/[0.96]"
                    style={{
                      fontSize: "clamp(1.9rem, 4.6vw, 4.4rem)",
                      textShadow: "0 8px 60px rgba(3, 16, 40, 0.55)",
                    }}
                  >
                    {shownCountry.country.name[locale]}
                  </div>
                  <div className="mt-3.5 text-xs font-medium uppercase tracking-[0.28em] text-white/65">
                    {shownCountry.country.tagline[locale]}
                  </div>
                </div>
              </div>

              {/* Right Dynamic-Centered + Ceiling-Clamped Frameless Panel (centers when few places; clamps at ceiling and scrolls when many places) */}
              <aside
                data-testid="welcome-atlas-side-panel"
                className={cn(
                  "pointer-events-auto fixed right-0 bottom-0 md:top-0 w-full md:w-[min(54vw,52rem)] flex flex-col justify-start md:justify-center overflow-y-auto md:overflow-hidden px-[5%] pb-6 pt-4 md:pt-[clamp(3.25rem,7vh,5rem)] md:pb-[clamp(4.25rem,8vh,5.5rem)] md:pl-[1%] md:pr-[3.5%]",
                  "max-md:max-h-[86vh] max-md:bg-gradient-to-t max-md:from-[#06122c] max-md:via-[#06122c]/95 max-md:to-transparent",
                  outgoingCountry ? "" : "max-md:eo-globe-sheet-in md:eo-globe-slide-in"
                )}
              >
                <div className="relative flex w-full flex-col md:max-h-full md:min-h-0">
                  {outgoingCountry && (
                    <div
                      aria-hidden="true"
                      className="eo-globe-details-out pointer-events-none absolute inset-x-0 top-0"
                    >
                      <div
                        className="mx-auto aspect-[16/10] w-full rounded-2xl bg-[#0e1e38]/75 shadow-[0_30px_70px_-30px_rgba(0,0,0,0.75)]"
                        style={{ maxWidth: "min(100%, calc(48vh * 1.6))" }}
                      />
                    </div>
                  )}

                  <div
                    key={shownCountry.country.iso3}
                    className={cn(
                      "flex w-full flex-col gap-4 md:flex-initial md:min-h-0",
                      outgoingCountry && "eo-globe-details-in"
                    )}
                  >
                    {/* Mobile country title inside bottom sheet */}
                    <div className="md:hidden">
                      <div
                        className="font-semibold uppercase leading-none text-white"
                        style={{ fontSize: "clamp(1.55rem, 7.2vw, 2.3rem)" }}
                      >
                        {shownCountry.country.name[locale]}
                      </div>
                      <div className="mt-1.5 text-xs uppercase tracking-[0.22em] text-white/60">
                        {shownCountry.country.tagline[locale]}
                      </div>
                    </div>

                    {/* Featured 16:10 Clean Image Slider (< > switches Modality, Bottom Dots switch Modality, Top is pure!) */}
                    <div
                      data-testid="welcome-atlas-hero-card"
                      onClick={() => setFullBleedOpen(true)}
                      className="group relative mx-auto aspect-[16/10] w-full shrink-0 overflow-hidden rounded-2xl shadow-[0_30px_70px_-30px_rgba(0,0,0,0.75)] cursor-pointer"
                      style={{
                        background: "linear-gradient(135deg, #0e1e38, #0a1526)",
                        maxWidth: "min(100%, calc(48vh * 1.6))",
                      }}
                    >
                      {/* Horizontal Sliding Place Track (strictly mounts active slide image src to conserve Cloudflare requests) */}
                      <div
                        className="flex h-full w-full transition-transform duration-[850ms] ease-[cubic-bezier(0.45,0,0.12,1)]"
                        style={{ transform: `translateX(-${placeIndex * 100}%)` }}
                      >
                        {shownCountry.country.places.map((p, idx) => {
                          const isCurrent = idx === placeIndex;
                          const mFrame =
                            p.modalities.find((m) => m.key === preferredModalityKey) ?? p.modalities[0];
                          return (
                            <div key={p.id} className="relative h-full w-full shrink-0">
                              <SmoothEoPhoto
                                src={isCurrent ? mFrame.src : undefined}
                                alt={p.label[locale]}
                                locale={locale}
                                isActive={isCurrent}
                                eager={isCurrent}
                                onReady={isCurrent ? () => setActivePhotoReady(true) : undefined}
                                className="h-full w-full object-cover"
                              />
                              <div
                                aria-hidden="true"
                                className="pointer-events-none absolute inset-0"
                                style={{
                                  background:
                                    "linear-gradient(to top, rgba(6, 18, 44, 0.92) 0%, rgba(6, 18, 44, 0.25) 38%, transparent 62%)",
                                }}
                              />
                              <div className="absolute inset-x-5 bottom-6">
                                <div
                                  data-testid={isCurrent ? "welcome-active-modality-caption" : undefined}
                                  className="pointer-events-none mb-1.5 inline-flex max-w-[calc(100%-0.5rem)] flex-nowrap items-center gap-x-1.5 overflow-hidden whitespace-nowrap rounded-full bg-black/45 px-2.5 py-0.5 text-xs font-medium tracking-wide text-white/95 backdrop-blur-sm"
                                >
                                  <span
                                    className="h-1.5 w-1.5 shrink-0 rounded-full"
                                    style={{ backgroundColor: mFrame.dotColor }}
                                  />
                                  <span className="shrink-0">{mFrame.label[locale]}</span>
                                  <span className="truncate text-white/75">
                                    · {mFrame.badge}
                                    {mFrame.capturedAt ? ` · ${mFrame.capturedAt}` : ""}
                                  </span>
                                </div>
                                <div className="text-[19px] font-semibold leading-tight text-white">
                                  {p.label[locale]}
                                </div>
                                <div className="mt-1 line-clamp-2 text-xs leading-relaxed text-white/[0.78]">
                                  {p.blurb[locale]}
                                </div>
                              </div>
                            </div>
                          );
                        })}
                      </div>

                      {/* Top-right 36x36 Glass Full-Bleed Expand Button */}
                      <button
                        type="button"
                        data-testid="welcome-atlas-expand-btn"
                        aria-label={copy.expandView}
                        onClick={(e) => {
                          e.stopPropagation();
                          setFullBleedOpen(true);
                        }}
                        className="absolute right-3 top-3 z-20 flex h-9 w-9 items-center justify-center rounded-full bg-white/[0.16] text-white backdrop-blur-[6px] transition-colors hover:bg-white/[0.28] cursor-pointer"
                      >
                        <Maximize2 className="h-4 w-4" />
                      </button>

                      {/* Left / Right Chevrons switch DATA MODALITIES on the active place (synced with bottom dots) */}
                      {currentPlace.modalities.length > 1 && (
                        <>
                          <button
                            type="button"
                            data-testid="welcome-modality-prev-btn"
                            aria-label={copy.prevModality}
                            title={copy.prevModality}
                            onClick={(e) => {
                              e.stopPropagation();
                              const len = currentPlace.modalities.length;
                              const prevMod = currentPlace.modalities[(activeModalityIndex - 1 + len) % len];
                              setPreferredModalityKey(prevMod.key);
                            }}
                            className="absolute left-2.5 top-1/2 z-20 -translate-y-1/2 flex h-8 w-8 items-center justify-center rounded-full bg-black/[0.42] text-white backdrop-blur-[4px] transition-colors hover:bg-black/70 cursor-pointer"
                          >
                            <ChevronLeft className="h-4 w-4" />
                          </button>
                          <button
                            type="button"
                            data-testid="welcome-modality-next-btn"
                            aria-label={copy.nextModality}
                            title={copy.nextModality}
                            onClick={(e) => {
                              e.stopPropagation();
                              const len = currentPlace.modalities.length;
                              const nextMod = currentPlace.modalities[(activeModalityIndex + 1) % len];
                              setPreferredModalityKey(nextMod.key);
                            }}
                            className="absolute right-2.5 top-1/2 z-20 -translate-y-1/2 flex h-8 w-8 items-center justify-center rounded-full bg-black/[0.42] text-white backdrop-blur-[4px] transition-colors hover:bg-black/70 cursor-pointer"
                          >
                            <ChevronRight className="h-4 w-4" />
                          </button>
                        </>
                      )}

                      {/* Bottom-Center Interactive Dots switch DATA MODALITIES on the active place */}
                      <ModalityDotsBar
                        modalities={currentPlace.modalities}
                        activeModality={activeModality}
                        locale={locale}
                        onSelect={setPreferredModalityKey}
                        className="absolute bottom-2.5 left-1/2 z-20 -translate-x-1/2"
                      />
                    </div>

                    {/* Clean Atlas Numbered Hairline Place List (shrink-wraps when few places so panel centers; shrinks & scrolls below ceiling-clamped hero card when many places) */}
                    <div
                      data-testid="welcome-panel-place-list"
                      className="flex min-h-0 flex-initial flex-col overflow-y-auto overscroll-contain pr-1 pb-2 [scrollbar-width:thin] [scrollbar-color:rgba(255,255,255,0.22)_transparent]"
                    >
                      {shownCountry.country.places.map((p, idx) => {
                        const active = idx === placeIndex;
                        return (
                          <button
                            key={p.id}
                            type="button"
                            data-testid={`welcome-panel-place-row-${p.id}`}
                            onClick={() => setPlaceIndex(idx)}
                            className={cn(
                              "flex w-full shrink-0 items-center gap-3 border-t border-white/[0.13] text-left transition-opacity cursor-pointer",
                              shownCountry.country.places.length >= 7 ? "py-2" : "py-2.5",
                              active ? "opacity-100" : "opacity-50 hover:opacity-85"
                            )}
                          >
                            <span className="w-6 shrink-0 font-mono text-xs text-white/55">
                              {String(idx + 1).padStart(2, "0")}
                            </span>
                            <span className="min-w-0 flex-1 truncate text-sm font-medium uppercase tracking-[0.05em] text-white">
                              {p.label[locale]}
                            </span>
                            {active && (
                              <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-[#e23b3b]" />
                            )}
                          </button>
                        );
                      })}
                    </div>
                  </div>
                </div>
              </aside>
            </div>
          )}

          {/* Stage-2 Full-Bleed HD Lightbox Viewer (z-50) */}
          {fullBleedOpen && shownCountry && currentPlace && activeModality && (
            <div
              data-testid="welcome-atlas-fullbleed"
              onClick={handleCloseFullBleed}
              className={cn(
                "fixed inset-0 z-50 overflow-hidden bg-[#020812]",
                fullBleedClosing ? "eo-globe-fb-out" : "eo-globe-fade"
              )}
            >
              <SmoothEoPhoto
                src={activeModality.src}
                alt={`${currentPlace.label[locale]} · ${activeModality.label[locale]}`}
                locale={locale}
                eager
                className="eo-globe-expand-img h-full w-full object-cover"
              />
              <div
                aria-hidden="true"
                className="pointer-events-none absolute inset-0"
                style={{
                  background:
                    "linear-gradient(to top, rgba(4, 12, 30, 0.84) 0%, rgba(4, 12, 30, 0.05) 45%, rgba(4, 12, 30, 0.45) 100%)",
                }}
              />

              <button
                type="button"
                data-testid="welcome-fullbleed-close-btn"
                aria-label={copy.closeFullBleed}
                onClick={(e) => {
                  e.stopPropagation();
                  handleCloseFullBleed();
                }}
                className="fixed right-5 top-[72px] md:right-8 md:top-8 flex h-11 w-11 items-center justify-center rounded-full border border-white/30 bg-black/[0.32] text-white backdrop-blur-[6px] transition-colors hover:bg-black/60 cursor-pointer"
              >
                <X className="h-5 w-5" />
              </button>

              {currentPlace.modalities.length > 1 && (
                <>
                  <button
                    type="button"
                    aria-label={copy.prevModality}
                    onClick={(e) => {
                      e.stopPropagation();
                      const len = currentPlace.modalities.length;
                      const prevMod = currentPlace.modalities[(activeModalityIndex - 1 + len) % len];
                      setPreferredModalityKey(prevMod.key);
                    }}
                    className="fixed left-5 top-1/2 z-20 -translate-y-1/2 flex h-10 w-10 items-center justify-center rounded-full border border-white/25 bg-black/35 text-white backdrop-blur-md transition-colors hover:bg-black/65 cursor-pointer"
                  >
                    <ChevronLeft className="h-5 w-5" />
                  </button>
                  <button
                    type="button"
                    aria-label={copy.nextModality}
                    onClick={(e) => {
                      e.stopPropagation();
                      const len = currentPlace.modalities.length;
                      const nextMod = currentPlace.modalities[(activeModalityIndex + 1) % len];
                      setPreferredModalityKey(nextMod.key);
                    }}
                    className="fixed right-5 top-1/2 z-20 -translate-y-1/2 flex h-10 w-10 items-center justify-center rounded-full border border-white/25 bg-black/35 text-white backdrop-blur-md transition-colors hover:bg-black/65 cursor-pointer"
                  >
                    <ChevronRight className="h-5 w-5" />
                  </button>
                </>
              )}

              <div
                onClick={(e) => e.stopPropagation()}
                className="fixed inset-x-[6%] bottom-[7.5%] flex flex-col gap-5"
              >
                <div className="max-w-2xl">
                  <div className="eo-globe-rise-in flex items-center gap-2 text-xs font-medium uppercase tracking-[0.3em] text-white/80">
                    <span
                      className="h-2 w-2 rounded-full"
                      style={{ backgroundColor: activeModality.dotColor }}
                    />
                    <span>
                      {shownCountry.country.name[locale]} · {activeModality.label[locale]} ·{" "}
                      {activeModality.badge}
                      {activeModality.capturedAt ? ` · ${activeModality.capturedAt}` : ""}
                    </span>
                  </div>
                  <div
                    className="eo-globe-rise-in mt-1.5 font-semibold leading-none text-white"
                    style={{ fontSize: "clamp(2.2rem, 6.2vw, 5rem)" }}
                  >
                    {currentPlace.label[locale]}
                  </div>
                  <p className="eo-globe-rise-in mt-3 max-w-xl text-sm leading-relaxed text-white/85">
                    {currentPlace.blurb[locale]}
                  </p>
                </div>

                {/* Same clean interactive Modality Dots strip inside Full-Bleed */}
                <ModalityDotsBar
                  modalities={currentPlace.modalities}
                  activeModality={activeModality}
                  locale={locale}
                  onSelect={setPreferredModalityKey}
                  testId="welcome-fullbleed-modality-dots"
                />
              </div>
            </div>
          )}
        </>
      )}
    </div>
  );
}
