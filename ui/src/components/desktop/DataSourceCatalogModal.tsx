/**
 * DataSourceCatalogModal.tsx
 * 数据源目录弹窗组件
 */

import React, { useState, useMemo, useEffect, useRef } from "react";
import {
  Search,
  X,
  Radio,
  Eye,
  Mountain,
  Compass,
  Clock,
  Cloud,
  Layers,
  Activity,
  ChevronRight,
  Globe,
  KeyRound,
  AlertCircle,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { ModalOverlay } from "@/components/ui/ModalOverlay";
import { EmptyState } from "@/components/ui/EmptyState";
import { useI18n } from "@/i18n";
import {
  getCredentialStatus,
  isTauriDesktopHost,
  queryOpenGeodata,
  type Bbox,
  type CredentialStatus,
} from "@/lib/bridge";
import type { SourceDescriptor } from "@/shell/api/types";
import { getAccountStatuses, isAccountStatusesWired } from "@/shell/api";
import { SubsetCornerBadge, SubsetScissorsIcon } from "./SubsetScissorsIcon";
import {
  getWorkbenchToolOptions,
  updateWorkbenchToolOptions,
} from "./ManualConditionsPanel";
import {
  buildCatalogItems,
  filterCatalogItems,
  computeCategoryCounts,
  getNextSelectableIndex,
  recordRecentSourceId,
  getRecentSourceIds,
  CATALOG_METADATA_REGISTRY,
  type CatalogCategoryKey,
  type CatalogBandTag,
  type CatalogAccountTag,
  type CatalogItem,
} from "@/lib/dataSourceCatalogLogic";
import {
  GEE_CATALOG_THEMES,
  filterGeePublicCatalog,
  type GeeCatalogTheme,
  type GeePublicDatasetEntry,
} from "@/lib/geePublicCatalogIndex";
import {
  localizeGeeDatasetEntry,
  localizeGeeThemeLabel,
} from "@/lib/dataSourceCatalogTranslations";

function mapCredentialStatusToAccountStates(status: CredentialStatus): Record<string, string> {
  const mapLoginPair = (raw: string | undefined): string => {
    const val = typeof raw === "string" ? raw.trim() : "";
    if (!val || val === "none" || val === "incomplete") return "credentials_missing";
    if (val === "unavailable") return "service_unavailable";
    return "login_required";
  };
  const mapOpentopography = (raw: string | undefined): string => {
    const val = typeof raw === "string" ? raw.trim() : "";
    if (val === "set") return "logged_in";
    if (val === "unavailable") return "service_unavailable";
    return "credentials_missing";
  };
  const mapGacos = (raw: string | undefined): string => {
    const val = typeof raw === "string" ? raw.trim() : "";
    if (!val || val === "none") return "credentials_missing";
    if (val === "unavailable") return "service_unavailable";
    return "logged_in";
  };
  return {
    cdse: mapLoginPair(status.cdse),
    usgs_m2m: mapLoginPair(status.usgs),
    earthdata: mapLoginPair(status.earthdata),
    bhoonidhi: mapLoginPair(status.bhoonidhi),
    mpc: "not_required",
    opentopography: mapOpentopography(status.opentopography),
    gacos: mapGacos(status.gacos),
  };
}

export interface DataSourceCatalogModalProps {
  isOpen: boolean;
  onClose: () => void;
  availableSources: SourceDescriptor[];
  activeResourceId: string;
  onSelectSource: (resourceId: string) => void;
  workingBbox?: Bbox | null;
  onOpenAccountSettings?: () => void;
  accountStatuses?: Record<string, string>;
  /** Lock modal strictly to Google Earth Engine catalog items (e.g. when opened from GEE mode). */
  lockedToGee?: boolean;
}

export function DataSourceCatalogModal({
  isOpen,
  onClose,
  availableSources,
  activeResourceId,
  onSelectSource,
  workingBbox,
  onOpenAccountSettings,
  accountStatuses: externalAccountStatuses,
  lockedToGee = false,
}: DataSourceCatalogModalProps) {
  const { t, locale } = useI18n();

  const [searchQuery, setSearchQuery] = useState("");
  const [activeCategory, setActiveCategory] = useState<CatalogCategoryKey>("primary-sar");
  const [bandFilter, setBandFilter] = useState<CatalogBandTag>("all");
  const [accountFilter, setAccountFilter] = useState<CatalogAccountTag>("all");
  const [highlightedId, setHighlightedId] = useState<string>(activeResourceId);

  const searchInputRef = useRef<HTMLInputElement | null>(null);
  const geeAssetInputRef = useRef<HTMLInputElement | null>(null);
  const listContainerRef = useRef<HTMLDivElement | null>(null);

  // 1. 最近使用 ID 列表与实时账号会话状态
  const [recentIds, setRecentIds] = useState<string[]>([]);
  const [liveAccountStatuses, setLiveAccountStatuses] = useState<
    Record<string, string> | undefined
  >(undefined);

  const [customGeeAssetId, setCustomGeeAssetId] = useState("");
  const [geeTheme, setGeeTheme] = useState<GeeCatalogTheme>("all");

  useEffect(() => {
    if (isOpen) {
      const opts = getWorkbenchToolOptions();
      setGeeTheme("all");
      setRecentIds(getRecentSourceIds());
      const normalizedActiveId =
        activeResourceId === "science.sentinel-2.l1c.v1"
          ? "science.sentinel-2.l2a.v1"
          : activeResourceId;
      const effectiveInitialHighlight =
        normalizedActiveId === "science.gee.catalog.v1" &&
        opts.geeSelectedPresetId &&
        opts.geeSelectedPresetId.startsWith("science.gee.")
          ? opts.geeSelectedPresetId
          : normalizedActiveId;
      const matchedMeta = CATALOG_METADATA_REGISTRY[effectiveInitialHighlight];
      if (lockedToGee) {
        setActiveCategory("gee-catalog");
      } else if (matchedMeta?.category) {
        setActiveCategory(matchedMeta.category);
      } else if (effectiveInitialHighlight.startsWith("science.gee.")) {
        setActiveCategory("gee-catalog");
      } else {
        setActiveCategory("primary-sar");
      }
      setHighlightedId(effectiveInitialHighlight);
      setSearchQuery("");
      setBandFilter("all");
      setAccountFilter("all");
      setCustomGeeAssetId(
        opts.geeAssetId && opts.geeAssetId !== "ALL" ? opts.geeAssetId : ""
      );
      window.setTimeout(() => {
        searchInputRef.current?.focus();
      }, 50);
    }
  }, [isOpen, activeResourceId, lockedToGee]);

  useEffect(() => {
    if (externalAccountStatuses) return;
    const isDesktop = isTauriDesktopHost();
    if (!isOpen && !isDesktop) return;
    let cancelled = false;

    const mergeAccountPatch = (
      patch: Record<string, string>,
      preserveLoggedIn = false
    ) => {
      if (cancelled) return;
      setLiveAccountStatuses((prev) => {
        const next: Record<string, string> = { ...(prev ?? {}) };
        for (const [key, val] of Object.entries(patch)) {
          if (preserveLoggedIn && next[key] === "logged_in" && val === "login_required") {
            continue;
          }
          next[key] = val;
        }
        return next;
      });
    };

    const loadAccounts = () => {
      if (isDesktop) {
        void getCredentialStatus()
          .then((credStatus) => {
            mergeAccountPatch(mapCredentialStatusToAccountStates(credStatus), true);
          })
          .catch(() => {
            // ignore and rely on session status query below
          });
      }
      if (isAccountStatusesWired()) {
        void getAccountStatuses()
          .then((list) => {
            const patch: Record<string, string> = {};
            for (const item of list) {
              patch[item.account_id] = item.state;
            }
            mergeAccountPatch(patch, false);
          })
          .catch(() => {
            // ignore and fall back to static registry / local credential status
          });
      }
      void queryOpenGeodata({ dataset: "gee_account_status" })
        .then((geeRes: any) => {
          if (geeRes && geeRes.ok) {
            mergeAccountPatch(
              { gee: geeRes.configured ? "logged_in" : "credentials_missing" },
              false
            );
          }
        })
        .catch(() => {
          // ignore in browser fixture mode
        });
    };

    loadAccounts();
    const onCredentialsChanged = () => loadAccounts();
    window.addEventListener("eostage:credentials-changed", onCredentialsChanged);
    return () => {
      cancelled = true;
      window.removeEventListener("eostage:credentials-changed", onCredentialsChanged);
    };
  }, [isOpen, externalAccountStatuses]);

  const effectiveAccountStatuses = useMemo(() => {
    if (externalAccountStatuses) {
      return {
        gee: "credentials_missing",
        ...externalAccountStatuses,
      };
    }
    return liveAccountStatuses;
  }, [externalAccountStatuses, liveAccountStatuses]);

  // 2. 构造所有 Catalog Items（仅后端支持的真实常规数据源 + 实时账号配置状态）
  const allItems = useMemo(() => {
    const items = buildCatalogItems(availableSources, effectiveAccountStatuses, locale);
    return lockedToGee
      ? items.filter((item) => item.resourceId.startsWith("science.gee."))
      : items;
  }, [availableSources, effectiveAccountStatuses, lockedToGee, locale]);

  // 3. 统计各分类包含条目数
  const categoryCounts = useMemo(
    () => computeCategoryCounts(allItems, recentIds),
    [allItems, recentIds]
  );

  const isCrossCategorySearch = !lockedToGee && searchQuery.trim().length > 0;

  // 4. 过滤当前条目列表
  const filteredItems = useMemo(
    () =>
      filterCatalogItems(allItems, {
        category: lockedToGee ? "gee-catalog" : activeCategory,
        ignoreCategory: isCrossCategorySearch,
        query: searchQuery,
        band: lockedToGee ? "all" : bandFilter,
        account: lockedToGee ? "all" : accountFilter,
        recentIds,
      }),
    [
      allItems,
      lockedToGee,
      activeCategory,
      isCrossCategorySearch,
      searchQuery,
      bandFilter,
      accountFilter,
      recentIds,
    ]
  );

  const categoryShortLabel = (cat: CatalogCategoryKey): string => {
    switch (cat) {
      case "primary-sar":
        return t("panels.catalogModal.radarSar");
      case "primary-optical":
        return t("panels.catalogModal.optical");
      case "primary-dem":
        return t("panels.catalogModal.elevationDem");
      case "primary-thematic":
        return t("panels.catalogModal.thematicAndTiles");
      case "auxiliary-orbit":
        return t("panels.catalogModal.preciseOrbit");
      case "auxiliary-atmospheric":
        return t("panels.catalogModal.atmospheric");
      case "auxiliary-geodesy":
        return t("panels.catalogModal.geodesyAndEvents");
      case "gee-catalog":
        return "GEE";
      default:
        return t("panels.catalogModal.primaryAll");
    }
  };

  // 5. 确保高亮项在当前有效列表中（若不在则指向第一个可选条目）
  useEffect(() => {
    if (filteredItems.length === 0) return;
    const exists = filteredItems.some((i) => i.resourceId === highlightedId);
    if (!exists) {
      const firstValid = filteredItems.find((i) => !i.disabled);
      if (firstValid) {
        setHighlightedId(firstValid.resourceId);
      } else {
        setHighlightedId(filteredItems[0].resourceId);
      }
    }
  }, [filteredItems, highlightedId]);

  // 6. 当前选中的高亮条目
  const currentItem = useMemo(
    () =>
      allItems.find((i) => i.resourceId === highlightedId) ||
      filteredItems[0] ||
      allItems[0],
    [allItems, filteredItems, highlightedId]
  );

  // 7. 处理选中确认
  const handleConfirmSelect = (item?: CatalogItem) => {
    const target = item || currentItem;
    if (!target || target.disabled) return;
    recordRecentSourceId(target.resourceId);
    setRecentIds(getRecentSourceIds());
    if (target.resourceId === "science.gee.catalog.v1") {
      updateWorkbenchToolOptions({
        geeAssetId: "ALL",
        geeTheme: "all",
        geeSelectedPresetId: "science.gee.catalog.v1",
      });
      if (!lockedToGee) onSelectSource("science.gee.catalog.v1");
    } else if (target.resourceId.startsWith("science.gee.")) {
      const resolvedGeeAssetId =
        target.geeAssetId ||
        CATALOG_METADATA_REGISTRY[target.resourceId]?.geeAssetId ||
        getWorkbenchToolOptions().geeAssetId ||
        "ALL";
      updateWorkbenchToolOptions({
        geeAssetId: resolvedGeeAssetId,
        geeSelectedPresetId: target.resourceId,
      });
      if (!lockedToGee) onSelectSource("science.gee.catalog.v1");
    } else if (!lockedToGee) {
      onSelectSource(target.resourceId);
    }
    onClose();
  };

  const handleUseCustomGeeAsset = (assetId: string) => {
    const trimmed = assetId.trim();
    if (!trimmed) return;
    updateWorkbenchToolOptions({
      geeAssetId: trimmed,
      geeSelectedPresetId: "science.gee.catalog.v1",
    });
    recordRecentSourceId("science.gee.catalog.v1");
    setRecentIds(getRecentSourceIds());
    if (!lockedToGee) onSelectSource("science.gee.catalog.v1");
    onClose();
  };

  const handleSelectGeePublicEntry = (entry: GeePublicDatasetEntry) => {
    updateWorkbenchToolOptions({
      geeAssetId: entry.assetId,
      geeSelectedPresetId: entry.presetResourceId ?? "science.gee.catalog.v1",
    });
    recordRecentSourceId("science.gee.catalog.v1");
    setRecentIds(getRecentSourceIds());
    if (!lockedToGee) onSelectSource("science.gee.catalog.v1");
    onClose();
  };

  const geeCatalogMatches = useMemo(
    () => filterGeePublicCatalog(geeTheme, searchQuery),
    [geeTheme, searchQuery]
  );

  // 8. 键盘交互（全局 Esc, 上下移动, Enter 确认）
  useEffect(() => {
    if (!isOpen) return;

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        const activeEl = document.activeElement;
        if (
          (activeEl instanceof HTMLInputElement ||
            activeEl instanceof HTMLTextAreaElement) &&
          Boolean(activeEl.closest("[data-testid='data-source-catalog-modal']"))
        ) {
          e.preventDefault();
          if (activeEl.value.length > 0) {
            if (activeEl === searchInputRef.current) {
              setSearchQuery("");
            } else if (activeEl === geeAssetInputRef.current) {
              setCustomGeeAssetId("");
            } else {
              const proto =
                activeEl instanceof HTMLTextAreaElement
                  ? HTMLTextAreaElement.prototype
                  : HTMLInputElement.prototype;
              const descriptor = Object.getOwnPropertyDescriptor(proto, "value");
              descriptor?.set?.call(activeEl, "");
              activeEl.dispatchEvent(new Event("input", { bubbles: true }));
            }
          } else {
            activeEl.blur();
          }
          return;
        }
        e.preventDefault();
        onClose();
        return;
      }

      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const currentIdx = filteredItems.findIndex((i) => i.resourceId === highlightedId);
        const nextIdx = getNextSelectableIndex(
          filteredItems,
          currentIdx,
          e.key === "ArrowDown" ? "down" : "up"
        );
        if (nextIdx !== -1 && filteredItems[nextIdx]) {
          setHighlightedId(filteredItems[nextIdx].resourceId);
        }
        return;
      }

      if (e.key === "Enter") {
        e.preventDefault();
        if (currentItem && !currentItem.disabled) {
          handleConfirmSelect(currentItem);
        }
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [isOpen, filteredItems, highlightedId, currentItem]);

  if (!isOpen) return null;

  const handleJumpToAccountSettings = (targetAccountId: string) => {
    if (typeof window !== "undefined") {
      window.location.hash = `scroll=${targetAccountId}&expand=${targetAccountId}`;
      window.dispatchEvent(
        new window.CustomEvent("eostage:focus-account-card", {
          detail: { accountId: targetAccountId },
        })
      );
    }
    onClose();
    onOpenAccountSettings?.();
  };

  const renderStatusBadge = (item: CatalogItem) => {
    const isGee = item.resourceId.startsWith("science.gee.");
    switch (item.status) {
      case "available": {
        const isFreeDirect = item.accountRequirement === "free" && !isGee;
        const label = isFreeDirect
          ? t("panels.catalogModal.statusFreeReady")
          : t("panels.catalogModal.statusConfiguredReady");
        return (
          <span className="text-xs text-sp-success-text flex items-center gap-1 font-medium shrink-0">
            <span className="inline-block w-1.5 h-1.5 rounded-full bg-sp-success-text" />
            {label}
          </span>
        );
      }
      case "search_only": {
        const label = isGee
          ? t("panels.catalogModal.statusGeeSearchOnly")
          : t("panels.catalogModal.statusCredentialsSearchOnly");
        return (
          <span className="text-xs text-sp-warning-text flex items-center gap-1 font-medium shrink-0">
            <span className="inline-block w-1.5 h-1.5 rounded-full bg-sp-warning-text" />
            {label}
          </span>
        );
      }
      case "account_required": {
        return (
          <span className="text-xs text-sp-text-muted flex items-center gap-1 font-medium shrink-0">
            <span className="inline-block w-1.5 h-1.5 rounded-full bg-sp-text-muted" />
            {t("panels.catalogModal.statusPendingCredentials")}
          </span>
        );
      }
      case "in_progress":
        return (
          <span className="text-xs text-sp-text-muted flex items-center gap-1 font-medium shrink-0">
            <span className="inline-block w-1.5 h-1.5 rounded-full bg-sp-line" />
            {t("panels.catalogModal.statusInProgress")}
          </span>
        );
    }
  };

  const handleCategorySelect = (cat: CatalogCategoryKey) => {
    setActiveCategory(cat);
    const nextFiltered = filterCatalogItems(allItems, {
      category: cat,
      query: searchQuery,
      band: bandFilter,
      account: accountFilter,
      recentIds,
    });
    if (nextFiltered.length > 0) {
      const firstSelectable = nextFiltered.find((i) => !i.disabled) ?? nextFiltered[0];
      if (firstSelectable) {
        setHighlightedId(firstSelectable.resourceId);
      }
    }
  };

  const handleNavKeyDown = (e: React.KeyboardEvent, cat: CatalogCategoryKey) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      e.stopPropagation();
      handleCategorySelect(cat);
    }
  };

  const allChipLabel = t("panels.catalogModal.allChip");

  const navItemClass = (cat: CatalogCategoryKey) =>
    cn(
      "h-8 px-2.5 rounded-[8px] flex items-center justify-between gap-2 text-xs cursor-pointer select-none transition-colors duration-fast focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-sp-accent",
      activeCategory === cat
        ? "bg-sp-selected text-sp-text font-semibold shadow-xs"
        : "text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text"
    );

  const chipClass = (active: boolean) =>
    cn(
      "h-6 px-2 text-xs rounded-full cursor-pointer select-none border transition-colors duration-fast flex items-center focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-sp-accent",
      active
        ? "bg-sp-selected border-sp-text text-sp-text font-medium"
        : "bg-sp-surface border-sp-line text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text"
    );

  return (
    <ModalOverlay
      className="z-50 sp-modal-backdrop-enter"
      role="dialog"
      aria-modal={true}
      aria-label={t("panels.catalogModal.title")}
      closeOnBackdropClick={false}
      transparentBackdrop={true}
    >
      <div
        className="w-[min(94vw,920px)] h-[min(88vh,640px)] bg-sp-surface rounded-[14px] border border-sp-line shadow-[0_20px_60px_-15px_rgba(0,0,0,0.28)] ring-1 ring-black/10 dark:ring-white/10 flex flex-col overflow-hidden sp-modal-panel-enter"
        data-testid="data-source-catalog-modal"
      >
        {/* 顶部搜索框与关闭按钮 */}
        <div className="h-12 px-3 border-b border-sp-line flex items-center gap-2 bg-sp-surface shrink-0">
          <Search className="h-4 w-4 text-sp-text-muted shrink-0" />
          <input
            ref={searchInputRef}
            type="text"
            className="flex-1 bg-transparent text-xs text-sp-text placeholder:text-sp-text-muted outline-none h-full"
            placeholder={t("panels.catalogModal.searchPlaceholder")}
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
          />
          {searchQuery && (
            <button
              type="button"
              className="p-1 text-sp-text-muted hover:text-sp-text rounded-full hover:bg-sp-hover"
              onClick={() => setSearchQuery("")}
              title={t("panels.catalogModal.close")}
            >
              <X className="h-3.5 w-3.5" />
            </button>
          )}
          <span className="text-xs text-sp-text-muted px-1.5 py-0.5 rounded border border-sp-line bg-sp-surface-muted select-none">
            ESC
          </span>
          <button
            type="button"
            className="p-1 text-sp-text-muted hover:text-sp-text rounded-sp-sm hover:bg-sp-hover cursor-pointer"
            onClick={onClose}
            aria-label={t("panels.catalogModal.close")}
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        {/* 主体三栏布局 */}
        <div className="flex flex-1 min-h-0 divide-x divide-sp-line">
          {/* 左栏：分类导航与快捷筛选（lockedToGee 时仅保留 GEE 目录） */}
          <div
            data-testid="catalog-sidebar-nav"
            className="w-[196px] shrink-0 p-2 bg-sp-surface-muted flex flex-col gap-1 overflow-y-auto"
          >
            {!lockedToGee && (
              <>
            {/* 最近使用 */}
            <div
              role="button"
              tabIndex={0}
              data-testid="catalog-nav-recent"
              className={navItemClass("recent")}
              onClick={() => handleCategorySelect("recent")}
              onKeyDown={(e) => handleNavKeyDown(e, "recent")}
            >
              <span className="flex min-w-0 items-center gap-1.5">
                <Clock className="h-3.5 w-3.5 shrink-0 text-sp-text-muted" />
                <span className="truncate">{t("panels.catalogModal.recent")}</span>
              </span>
              <span className="text-xs text-sp-text-muted font-normal shrink-0 tabular-nums">
                {categoryCounts.recent}
              </span>
            </div>

            {/* 主数据分组 */}
            <div className="text-xs font-semibold text-sp-text-muted px-2 pt-2.5 pb-1 select-none">
              {t("panels.catalogModal.primary")}
            </div>
            <div
              role="button"
              tabIndex={0}
              data-testid="catalog-nav-primary-all"
              className={navItemClass("primary-all")}
              onClick={() => handleCategorySelect("primary-all")}
              onKeyDown={(e) => handleNavKeyDown(e, "primary-all")}
            >
              <span className="flex min-w-0 items-center gap-1.5">
                <Layers className="h-3.5 w-3.5 shrink-0 text-sp-text-muted" />
                <span className="truncate">{t("panels.catalogModal.primaryAll")}</span>
              </span>
              <span className="text-xs text-sp-text-muted font-normal shrink-0 tabular-nums">
                {categoryCounts["primary-all"]}
              </span>
            </div>
            <div
              role="button"
              tabIndex={0}
              data-testid="catalog-nav-primary-sar"
              className={navItemClass("primary-sar")}
              onClick={() => handleCategorySelect("primary-sar")}
              onKeyDown={(e) => handleNavKeyDown(e, "primary-sar")}
            >
              <span className="flex min-w-0 items-center gap-1.5">
                <Radio className="h-3.5 w-3.5 shrink-0 text-sp-text-muted" />
                <span className="truncate">{t("panels.catalogModal.radarSar")}</span>
              </span>
              <span className="text-xs text-sp-text-muted font-normal shrink-0 tabular-nums">
                {categoryCounts["primary-sar"]}
              </span>
            </div>
            <div
              role="button"
              tabIndex={0}
              data-testid="catalog-nav-primary-optical"
              className={navItemClass("primary-optical")}
              onClick={() => handleCategorySelect("primary-optical")}
              onKeyDown={(e) => handleNavKeyDown(e, "primary-optical")}
            >
              <span className="flex min-w-0 items-center gap-1.5">
                <Eye className="h-3.5 w-3.5 shrink-0 text-sp-text-muted" />
                <span className="truncate">{t("panels.catalogModal.optical")}</span>
              </span>
              <span className="text-xs text-sp-text-muted font-normal shrink-0 tabular-nums">
                {categoryCounts["primary-optical"]}
              </span>
            </div>
            <div
              role="button"
              tabIndex={0}
              data-testid="catalog-nav-primary-dem"
              className={navItemClass("primary-dem")}
              onClick={() => handleCategorySelect("primary-dem")}
              onKeyDown={(e) => handleNavKeyDown(e, "primary-dem")}
            >
              <span className="flex min-w-0 items-center gap-1.5">
                <Mountain className="h-3.5 w-3.5 shrink-0 text-sp-text-muted" />
                <span className="truncate">{t("panels.catalogModal.elevationDem")}</span>
              </span>
              <span className="text-xs text-sp-text-muted font-normal shrink-0 tabular-nums">
                {categoryCounts["primary-dem"]}
              </span>
            </div>

            <div
              role="button"
              tabIndex={0}
              data-testid="catalog-nav-primary-thematic"
              className={navItemClass("primary-thematic")}
              onClick={() => handleCategorySelect("primary-thematic")}
              onKeyDown={(e) => handleNavKeyDown(e, "primary-thematic")}
            >
              <span className="flex min-w-0 items-center gap-1.5">
                <Layers className="h-3.5 w-3.5 shrink-0 text-sp-text-muted" />
                <span className="truncate">{t("panels.catalogModal.thematicAndTiles")}</span>
              </span>
              <span className="text-xs text-sp-text-muted font-normal shrink-0 tabular-nums">
                {categoryCounts["primary-thematic"]}
              </span>
            </div>

            {/* 辅助数据分组 */}
            <div className="text-xs font-semibold text-sp-text-muted px-2 pt-2.5 pb-1 select-none">
              {t("panels.catalogModal.auxiliary")}
            </div>
            <div
              role="button"
              tabIndex={0}
              data-testid="catalog-nav-auxiliary-orbit"
              className={navItemClass("auxiliary-orbit")}
              onClick={() => handleCategorySelect("auxiliary-orbit")}
              onKeyDown={(e) => handleNavKeyDown(e, "auxiliary-orbit")}
            >
              <span className="flex min-w-0 items-center gap-1.5">
                <Compass className="h-3.5 w-3.5 shrink-0 text-sp-text-muted" />
                <span className="truncate">{t("panels.catalogModal.preciseOrbit")}</span>
              </span>
              <span className="text-xs text-sp-text-muted font-normal shrink-0 tabular-nums">
                {categoryCounts["auxiliary-orbit"]}
              </span>
            </div>
            <div
              role="button"
              tabIndex={0}
              data-testid="catalog-nav-auxiliary-atmospheric"
              className={navItemClass("auxiliary-atmospheric")}
              onClick={() => handleCategorySelect("auxiliary-atmospheric")}
              onKeyDown={(e) => handleNavKeyDown(e, "auxiliary-atmospheric")}
            >
              <span className="flex min-w-0 items-center gap-1.5">
                <Cloud className="h-3.5 w-3.5 shrink-0 text-sp-text-muted" />
                <span className="truncate">{t("panels.catalogModal.atmospheric")}</span>
              </span>
              <span className="text-xs text-sp-text-muted font-normal shrink-0 tabular-nums">
                {categoryCounts["auxiliary-atmospheric"]}
              </span>
            </div>

            <div
              role="button"
              tabIndex={0}
              data-testid="catalog-nav-auxiliary-geodesy"
              className={navItemClass("auxiliary-geodesy")}
              onClick={() => handleCategorySelect("auxiliary-geodesy")}
              onKeyDown={(e) => handleNavKeyDown(e, "auxiliary-geodesy")}
            >
              <span className="flex min-w-0 items-center gap-1.5">
                <Activity className="h-3.5 w-3.5 shrink-0 text-sp-text-muted" />
                <span className="truncate">{t("panels.catalogModal.geodesyAndEvents")}</span>
              </span>
              <span className="text-xs text-sp-text-muted font-normal shrink-0 tabular-nums">
                {categoryCounts["auxiliary-geodesy"]}
              </span>
            </div>
              </>
            )}

            {/* 云端数据目录分组 */}
            <div className="text-xs font-semibold text-sp-text-muted px-2 pt-2.5 pb-1 select-none">
              {t("panels.catalogModal.cloudCatalog")}
            </div>
            <div
              role="button"
              tabIndex={0}
              data-testid="catalog-nav-gee"
              className={navItemClass("gee-catalog")}
              onClick={() => handleCategorySelect("gee-catalog")}
              onKeyDown={(e) => handleNavKeyDown(e, "gee-catalog")}
            >
              <span className="flex min-w-0 items-center gap-1.5">
                <Globe className="h-3.5 w-3.5 shrink-0 text-sp-text-muted" />
                <span className="truncate">Google Earth Engine</span>
              </span>
              <span className="text-xs text-sp-text-muted font-normal shrink-0 tabular-nums">
                {categoryCounts["gee-catalog"] || 0}
              </span>
            </div>

            {lockedToGee && (
              <div className="mx-1 mt-2 rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 py-2 text-xs leading-relaxed text-sp-text-muted">
                {t("panels.catalogModal.geeLockedHint")}
              </div>
            )}

            {!lockedToGee && (
              <>
            {/* 波段筛选标签 */}
            <div className="text-xs font-semibold text-sp-text-muted px-2 pt-2.5 pb-1 select-none">
              {t("panels.catalogModal.band")}
            </div>
            <div className="flex flex-wrap gap-1 px-1">
              {(["all", "C", "L", "S", "optical"] as CatalogBandTag[]).map((b) => (
                <button
                  key={b}
                  type="button"
                  className={chipClass(bandFilter === b)}
                  onClick={() => setBandFilter(b)}
                >
                  {b === "all" ? allChipLabel : b === "optical" ? t("panels.catalogModal.optical") : b}
                </button>
              ))}
            </div>

            {/* 账号筛选标签 */}
            <div className="text-xs font-semibold text-sp-text-muted px-2 pt-2 pb-1 select-none">
              {t("panels.catalogModal.account")}
            </div>
            <div className="flex flex-wrap gap-1 px-1">
              {(["all", "free", "account_required"] as CatalogAccountTag[]).map((a) => (
                <button
                  key={a}
                  type="button"
                  className={chipClass(accountFilter === a)}
                  onClick={() => setAccountFilter(a)}
                >
                  {a === "all"
                    ? allChipLabel
                    : a === "free"
                    ? t("panels.catalogModal.free")
                    : t("panels.catalogModal.accountRequired")}
                </button>
              ))}
            </div>
              </>
            )}
          </div>

          {/* 中栏：数据源列表与分组 */}
          <div
            ref={listContainerRef}
            className="flex-1 min-w-0 p-2 overflow-y-auto flex flex-col gap-1 bg-sp-surface"
          >
            {activeCategory === "gee-catalog" && (
              <div className="flex flex-col gap-2 mb-1.5 shrink-0">
                <div
                  data-testid="catalog-gee-theme-browser"
                  className="p-2.5 rounded-[10px] bg-sp-surface-muted border border-sp-line flex flex-col gap-2"
                >
                  <div className="flex flex-wrap items-center justify-between gap-x-2 gap-y-0.5 text-xs">
                    <span className="font-semibold text-sp-text">
                      {t("panels.catalogModal.geeCatalogSectionHeader", {
                        curated: categoryCounts["gee-catalog"] || 0,
                        indexed: geeCatalogMatches.length,
                      })}
                    </span>
                    <span className="text-xs text-sp-text-muted">
                      {t("panels.catalogModal.geeDefaultAllNote")}
                    </span>
                  </div>
                  <div className="flex flex-wrap gap-1">
                    {GEE_CATALOG_THEMES.map((th) => {
                      const active = geeTheme === th.id;
                      return (
                        <button
                          key={th.id}
                          type="button"
                          data-testid={`gee-theme-chip-${th.id}`}
                          onClick={() => setGeeTheme(th.id)}
                          className={chipClass(active)}
                        >
                          {localizeGeeThemeLabel(th.id, th.label, locale)}
                        </button>
                      );
                    })}
                  </div>
                  <div className="max-h-40 overflow-y-auto grid grid-cols-1 gap-1 pr-0.5">
                    {geeCatalogMatches.length === 0 ? (
                      <EmptyState
                        kind="no-results"
                        size="menu"
                        title={
                          searchQuery.trim()
                            ? t("panels.catalogModal.geeNoMatchCustom", { query: searchQuery.trim() })
                            : t("panels.catalogModal.geeNoMatchEmpty")
                        }
                      />
                    ) : (
                      geeCatalogMatches.map((rawEntry) => {
                        const entry = localizeGeeDatasetEntry(rawEntry, locale);
                        const activeGeeAsset = getWorkbenchToolOptions().geeAssetId;
                        const isPicked = activeGeeAsset === entry.assetId;
                        return (
                          <button
                            key={entry.assetId}
                            type="button"
                            data-testid={`gee-public-entry-${entry.assetId.replace(/[^a-zA-Z0-9_-]/g, "_")}`}
                            onClick={() => handleSelectGeePublicEntry(entry)}
                            className={cn(
                              "w-full px-2.5 py-1.5 rounded-[8px] border flex items-center justify-between gap-2 text-left transition-colors cursor-pointer",
                              isPicked
                                ? "border-sp-text bg-sp-selected"
                                : "border-sp-line bg-sp-surface hover:bg-sp-hover"
                            )}
                          >
                            <div className="min-w-0 flex-1">
                              <div className="text-xs font-medium text-sp-text truncate">
                                {entry.title}
                              </div>
                              <div className="text-xs text-sp-text-muted truncate">
                                <span className="font-mono">{entry.assetId}</span>
                                <span> · {entry.resolution} · {entry.era}</span>
                              </div>
                            </div>
                            <span
                              className={cn(
                                "shrink-0 text-xs px-1.5 py-0.5 rounded border whitespace-nowrap",
                                isPicked
                                  ? "bg-sp-accent-fill text-sp-accent-ink border-transparent"
                                  : "bg-sp-surface-muted border-sp-line text-sp-text-secondary"
                              )}
                            >
                              {isPicked ? t("panels.catalogModal.pickedBtn") : t("panels.catalogModal.pickBtn")}
                            </span>
                          </button>
                        );
                      })
                    )}
                  </div>
                </div>

                <div
                  data-testid="catalog-gee-custom-asset-container"
                  className="p-2.5 rounded-[10px] bg-sp-surface-muted border border-sp-line flex flex-col gap-1.5 shrink-0"
                >
                  <div className="flex items-center justify-between text-xs font-medium text-sp-text">
                    <span>{t("panels.catalogModal.customGeeAssetLabel")}</span>
                    <span className="text-xs text-sp-text-muted">
                      {effectiveAccountStatuses?.gee === "logged_in"
                        ? t("panels.catalogModal.geeProjectReadyBadge")
                        : t("panels.catalogModal.geeSearchOnlyBadge")}
                    </span>
                  </div>
                  <div className="flex items-center gap-1.5">
                    <input
                      ref={geeAssetInputRef}
                      type="text"
                      data-testid="gee-custom-asset-id-input"
                      placeholder={t("panels.catalogModal.customGeePlaceholder")}
                      value={customGeeAssetId}
                      onChange={(e) => setCustomGeeAssetId(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") {
                          e.stopPropagation();
                          e.preventDefault();
                          if (customGeeAssetId.trim()) {
                            handleUseCustomGeeAsset(customGeeAssetId.trim());
                          }
                        }
                      }}
                      className="flex-1 h-8 rounded px-2 font-mono text-xs bg-sp-surface border border-sp-line text-sp-text placeholder:text-sp-text-muted outline-none focus:border-sp-text"
                    />
                    <button
                      type="button"
                      data-testid="gee-custom-asset-id-submit-btn"
                      disabled={!customGeeAssetId.trim()}
                      onClick={() => handleUseCustomGeeAsset(customGeeAssetId.trim())}
                      className="h-8 px-3 rounded bg-sp-accent-fill text-sp-accent-ink text-xs font-medium hover:opacity-90 disabled:opacity-40 cursor-pointer shrink-0"
                    >
                      {t("panels.catalogModal.loadBtn")}
                    </button>
                  </div>
                </div>
              </div>
            )}
            {isCrossCategorySearch && filteredItems.length > 0 && (
              <div
                data-testid="catalog-cross-category-banner"
                className="px-2.5 py-1.5 mb-0.5 rounded-[8px] bg-sp-surface-muted border border-sp-line flex items-center justify-between gap-2 text-xs text-sp-text-secondary shrink-0"
              >
                <span className="truncate">
                  {t("panels.catalogModal.searchingAllSources", { count: filteredItems.length })}
                </span>
                <button
                  type="button"
                  onClick={() => setSearchQuery("")}
                  className="text-xs text-sp-text-muted hover:text-sp-text underline shrink-0 cursor-pointer"
                >
                  {t("panels.catalogModal.clearBtn")}
                </button>
              </div>
            )}
            {filteredItems.length === 0 ? (
              <div className="h-full flex flex-col items-center justify-center gap-2.5 px-4 text-center">
                <div className="text-xs text-sp-text-muted">
                  {t("panels.catalogModal.noMatchingSources")}
                </div>
                {(searchQuery.trim().length > 0 ||
                  bandFilter !== "all" ||
                  accountFilter !== "all" ||
                  (!lockedToGee && activeCategory !== "primary-all")) && (
                  <button
                    type="button"
                    data-testid="catalog-reset-search-btn"
                    onClick={() => {
                      setSearchQuery("");
                      setBandFilter("all");
                      setAccountFilter("all");
                      if (!lockedToGee) {
                        setActiveCategory("primary-all");
                      }
                    }}
                    className="h-7 px-3 rounded-[8px] border border-sp-line bg-sp-surface-muted text-xs text-sp-text hover:bg-sp-hover cursor-pointer"
                  >
                    {lockedToGee
                      ? t("panels.catalogModal.resetSearchFilter")
                      : t("panels.catalogModal.clearFilterViewAll")}
                  </button>
                )}
              </div>
            ) : (
              filteredItems.map((item) => {
                const isSelected = item.resourceId === highlightedId;
                const normalizedCurrentResourceId =
                  activeResourceId === "science.sentinel-2.l1c.v1"
                    ? "science.sentinel-2.l2a.v1"
                    : activeResourceId;
                const activeGeePresetId =
                  normalizedCurrentResourceId === "science.gee.catalog.v1"
                    ? (getWorkbenchToolOptions().geeSelectedPresetId || "science.gee.catalog.v1")
                    : null;
                const isCurrentActive =
                  activeGeePresetId !== null
                    ? item.resourceId === activeGeePresetId
                    : item.resourceId === normalizedCurrentResourceId;

                return (
                  <div
                    key={item.resourceId}
                    role="button"
                    tabIndex={item.disabled ? -1 : 0}
                    aria-disabled={item.disabled || undefined}
                    data-resource-id={item.resourceId}
                    className={cn(
                      "relative p-2.5 rounded-[10px] flex items-center justify-between gap-3 cursor-pointer select-none transition-colors duration-fast border focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-sp-accent",
                      isSelected
                        ? "bg-sp-selected border-sp-line shadow-xs"
                        : "border-transparent hover:bg-sp-hover",
                      item.disabled && "opacity-50 cursor-not-allowed hover:bg-transparent"
                    )}
                    onClick={() => {
                      setHighlightedId(item.resourceId);
                    }}
                    onDoubleClick={() => {
                      if (!item.disabled) handleConfirmSelect(item);
                    }}
                    onKeyDown={(e) => {
                      if (e.key === " ") {
                        e.preventDefault();
                        setHighlightedId(item.resourceId);
                      } else if (e.key === "Enter" && !item.disabled) {
                        e.preventDefault();
                        e.stopPropagation();
                        handleConfirmSelect(item);
                      }
                    }}
                  >
                    <div className="flex flex-col gap-0.5 min-w-0 flex-1">
                      <div className="flex items-start gap-1.5 flex-wrap">
                        <span className="inline-flex items-start gap-1 min-w-0 max-w-full">
                          <span className="text-xs font-semibold leading-snug text-sp-text">
                            {item.name}
                          </span>
                          {item.supportsOnlineCropping && (
                            <SubsetCornerBadge
                              testId={`catalog-crop-badge-${item.resourceId}`}
                              title={t("panels.catalogModal.onlineClipTitle")}
                              className="h-3.5 w-3.5 shrink-0 -mt-0.5"
                            />
                          )}
                        </span>
                        {isCrossCategorySearch && item.category !== activeCategory && (
                          <span className="text-xs px-1.5 py-0.5 rounded-full bg-sp-surface-muted text-sp-text-muted border border-sp-line shrink-0">
                            {categoryShortLabel(item.category)}
                          </span>
                        )}
                        {isCurrentActive && (
                          <span
                            data-testid={`catalog-current-source-badge-${item.resourceId}`}
                            className="text-xs px-1.5 py-0.5 rounded-full bg-sp-surface-muted text-sp-text-secondary border border-sp-line shrink-0"
                          >
                            {t("panels.catalogModal.currentSource")}
                          </span>
                        )}
                      </div>
                      <span className="text-xs text-sp-text-muted truncate">
                        {item.subtitle}
                      </span>
                    </div>

                    <div className="flex items-center gap-1.5 shrink-0">
                      {renderStatusBadge(item)}
                    </div>
                  </div>
                );
              })
            )}
          </div>

          {/* 右栏：条目详情与置底确认按钮 */}
          <div className="w-[280px] shrink-0 bg-sp-surface flex flex-col overflow-hidden">
            {currentItem ? (
              <>
                <div className="flex-1 min-h-0 p-4 flex flex-col gap-4 overflow-y-auto">
                  <div className="flex flex-col gap-1">
                    <div className="flex items-start justify-between gap-2">
                      <h3 className="text-sm font-semibold text-sp-text m-0 leading-snug">
                        {currentItem.name}
                      </h3>
                      {currentItem.supportsOnlineCropping && (
                        <span
                          data-testid="detail-online-crop-badge"
                          className="inline-flex shrink-0 items-center gap-1 rounded-sp-sm border border-sp-line bg-sp-surface-muted px-1.5 py-0.5 text-xs font-medium text-sp-text-secondary"
                        >
                          <SubsetScissorsIcon className="h-3 w-3" />
                          {t("panels.catalogModal.onlineClipBadge")}
                        </span>
                      )}
                    </div>
                    <p className="text-xs text-sp-text-secondary leading-relaxed m-0 break-words">
                      {currentItem.description}
                    </p>
                  </div>

                  {/* 元数据摘要 */}
                  <div className="p-3 rounded-[10px] bg-sp-surface-muted border border-sp-line flex flex-col gap-1.5 text-xs">
                    {currentItem.satellites && (
                      <div className="flex items-start justify-between gap-2">
                        <span className="text-sp-text-muted shrink-0">
                          {t("panels.catalogModal.metaSatellites")}
                        </span>
                        <span className="text-sp-text font-medium text-right break-words">
                          {currentItem.satellites}
                        </span>
                      </div>
                    )}
                    {currentItem.timeRange && (
                      <div className="flex items-start justify-between gap-2">
                        <span className="text-sp-text-muted shrink-0">
                          {t("panels.catalogModal.metaTime")}
                        </span>
                        <span className="text-sp-text font-medium text-right">
                          {currentItem.timeRange}
                        </span>
                      </div>
                    )}
                    {currentItem.band && (
                      <div className="flex items-start justify-between gap-2">
                        <span className="text-sp-text-muted shrink-0">
                          {t("panels.catalogModal.metaBand")}
                        </span>
                        <span className="text-sp-text font-medium text-right">
                          {currentItem.band}
                        </span>
                      </div>
                    )}
                    <div className="flex items-start justify-between gap-2">
                      <span className="text-sp-text-muted shrink-0">
                        {t("panels.catalogModal.metaAccount")}
                      </span>
                      <span className="text-sp-text font-medium text-right">
                        {currentItem.accountText}
                      </span>
                    </div>
                    {currentItem.geeAssetId && (
                      <div className="flex items-start justify-between gap-2">
                        <span className="text-sp-text-muted shrink-0">
                          GEE Asset ID
                        </span>
                        <span className="font-mono text-sp-text font-medium text-right break-all">
                          {currentItem.geeAssetId}
                        </span>
                      </div>
                    )}
                  </div>

                  {/* B9: 凭据状态引导卡片（仅检索或待配置凭据时展示直达配置入口） */}
                  {currentItem.status === "search_only" && (
                    <div
                      data-testid="catalog-credential-guidance-card"
                      className="rounded-sp-md border border-sp-line bg-sp-warning-bg p-3 flex flex-col gap-2 text-xs text-sp-warning-text"
                    >
                      <div className="flex items-start gap-1.5">
                        <AlertCircle className="h-3.5 w-3.5 shrink-0 mt-0.5" />
                        <span className="leading-relaxed">
                          {currentItem.resourceId.startsWith("science.gee.")
                            ? t("panels.catalogModal.geeNeedsCredentialsHint")
                            : t("panels.catalogModal.sourceSearchOnlyCredentialsHint")}
                        </span>
                      </div>
                      <button
                        type="button"
                        data-testid="catalog-open-account-settings-btn"
                        onClick={() =>
                          handleJumpToAccountSettings(
                            currentItem.resourceId.startsWith("science.gee.")
                              ? "gee"
                              : currentItem.requiredAccountIds?.[0] || "earthdata"
                          )
                        }
                        className="eo-tactile-secondary inline-flex h-7 items-center justify-center gap-1 rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 text-xs font-semibold text-sp-text hover:bg-sp-hover cursor-pointer"
                      >
                        <KeyRound className="h-3 w-3" />
                        <span>
                          {currentItem.resourceId.startsWith("science.gee.")
                            ? t("panels.catalogModal.gotoGeeCredentialsBtn")
                            : t("panels.catalogModal.gotoServiceCredentialsBtn")}
                        </span>
                      </button>
                    </div>
                  )}

                  {currentItem.status === "account_required" && (
                    <div
                      data-testid="catalog-credential-guidance-card"
                      className="rounded-sp-md border border-sp-line bg-sp-surface-muted p-3 flex flex-col gap-2 text-xs text-sp-text-secondary"
                    >
                      <div className="flex items-start gap-1.5">
                        <KeyRound className="h-3.5 w-3.5 shrink-0 mt-0.5 text-sp-text-muted" />
                        <span className="leading-relaxed">
                          {t("panels.catalogModal.channelCredentialsNeeded", {
                            accounts: currentItem.accountText
                              .replace(/（(?:待配置凭据|验证未通过|認証情報未設定|認証未確認)）|\((?:Pending credentials|Verification failed)\)/g, "")
                              .trim(),
                          })}
                        </span>
                      </div>
                      <button
                        type="button"
                        data-testid="catalog-open-account-settings-btn"
                        onClick={() =>
                          handleJumpToAccountSettings(
                            currentItem.requiredAccountIds?.[0] || "earthdata"
                          )
                        }
                        className="eo-tactile-secondary inline-flex h-7 items-center justify-center gap-1 rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 text-xs font-semibold text-sp-text hover:bg-sp-hover cursor-pointer"
                      >
                        <KeyRound className="h-3 w-3" />
                        <span>{t("panels.catalogModal.gotoDataAccountsBtn")}</span>
                      </button>
                    </div>
                  )}

                  {/* 可筛选条件 */}
                  {currentItem.filters && currentItem.filters.length > 0 && (
                    <div className="flex flex-col gap-1.5">
                      <span className="text-xs font-medium text-sp-text-secondary">
                        {t("panels.catalogModal.applicableFilters")}
                      </span>
                      <div className="flex flex-wrap gap-1.5">
                        {currentItem.filters.map((filter, i) => (
                          <span
                            key={i}
                            className="px-2 py-0.5 rounded-full bg-sp-surface-muted border border-sp-line text-xs text-sp-text-secondary"
                          >
                            {filter}
                          </span>
                        ))}
                      </div>
                    </div>
                  )}

                  {/* 可配套下载 */}
                  {currentItem.companionDownloads && currentItem.companionDownloads.length > 0 && (
                    <div className="flex flex-col gap-1.5">
                      <span className="text-xs font-medium text-sp-text-secondary">
                        {t("panels.catalogModal.companionDownloads")}
                      </span>
                      <div className="flex flex-wrap gap-1.5">
                        {currentItem.companionDownloads.map((comp, i) => (
                          <span
                            key={i}
                            className="px-2 py-0.5 rounded-full bg-sp-surface-muted border border-sp-line text-xs text-sp-text-secondary"
                          >
                            {comp}
                          </span>
                        ))}
                      </div>
                    </div>
                  )}

                  {/* 注意事项 */}
                  {currentItem.usageNotes && currentItem.usageNotes.length > 0 && (
                    <div
                      data-testid="catalog-detail-usage-notes"
                      className="flex flex-col gap-1.5"
                    >
                      <span className="inline-flex items-center gap-1 text-xs font-medium text-sp-text-secondary">
                        <AlertCircle className="h-3.5 w-3.5 shrink-0 text-sp-warning-text" />
                        <span>{t("panels.catalogModal.usageNotes")}</span>
                      </span>
                      <ul className="m-0 list-disc pl-4 rounded-[10px] bg-sp-surface-muted border border-sp-line p-2.5 flex flex-col gap-1 text-xs text-sp-text-secondary leading-relaxed">
                        {currentItem.usageNotes.map((note, i) => (
                          <li key={i}>{note}</li>
                        ))}
                      </ul>
                    </div>
                  )}
                </div>

                {/* 底部常驻确认按钮 */}
                <div className="shrink-0 border-t border-sp-line bg-sp-surface p-3">
                  <button
                    type="button"
                    data-testid="catalog-confirm-use-btn"
                    disabled={currentItem.disabled}
                    className="eo-tactile-primary w-full h-9 rounded-[8px] bg-sp-accent-fill text-sp-accent-ink text-xs font-medium disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer flex items-center justify-center gap-1"
                    onClick={() => handleConfirmSelect(currentItem)}
                  >
                    <span>{t("panels.catalogModal.useThisSource")}</span>
                    <ChevronRight className="h-3.5 w-3.5" />
                  </button>
                </div>
              </>
            ) : null}
          </div>
        </div>
      </div>
    </ModalOverlay>
  );
}
