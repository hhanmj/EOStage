import React, { useState, useCallback, useEffect, useLayoutEffect, useRef } from "react";
import { flushSync } from "react-dom";
import {
  Plus,
  Search,
  Inbox,
  Layers,
  Map,
  Map as MapIcon,
  Download,
  BookOpen,
  HelpCircle,
  Settings,
  ArrowLeft,
  Blocks,
  X,
  User,
  RefreshCw,
} from "lucide-react";
import { type EOStageAccountStatus } from "@/shell/api/eostageAccount";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";
import { modShortcut } from "@/lib/platformShortcuts";
import type { SettingsCategory } from "../SettingsModal";
import {
  SETTINGS_CATEGORIES,
  SETTINGS_GROUPS,
  getSettingsCategoryLabel,
  readLastSettingsCategory,
  rememberSettingsCategory,
} from "@/shell/api/settingsEntries";
import type { ShellRoute } from "@/shell/api/shellRoutes";
import { historyFixtureSessionsFactory } from "./DesktopAgentWorkbench";
import { SidebarSessionTree } from "./SidebarSessionTree";
import { type NotificationItem } from "@/shell/api/notificationBell";
import { SpringDropdownSurface } from "./SpringDropdownPrimitives";
import { hasUnreadReleaseNotice, subscribeReleaseChangelog } from "@/shell/api/releaseChangelog";
import { NotificationPopover } from "./NotificationPopover";
import { AboutEOStageModal } from "./AboutEOStageModal";
import {
  SPRING_PRESETS,
  canAnimateSpringInBrowser,
  clamp,
  createSpring,
  createStretchyHighlight,
  hideStretchyHighlight,
  measureRelativeLayoutRect,
  readStretchyHighlightMetrics,
  settleIfClose,
  snapSpring,
  stepSpringSubdivided,
  stepStretchyHighlight,
  targetStretchyHighlight,
  tuneSpring,
  type Spring1D,
} from "@/lib/springDropdownMotion";

export interface DesktopActivityRailProps {
  activeRoute: ShellRoute | null;
  onNavigate: (route: ShellRoute) => void;
  downloadsAlert?: boolean;
  onOpenDownloads?: () => void;
  onOpenDataCatalog?: () => void;
  dataCatalogOpen?: boolean;
  activeDataSourceLabel?: string | null;
  onOpenSettingsCategory?: (category: SettingsCategory) => void;
  dark?: boolean;
  onToggleDark?: () => void;
  appVersion?: string | null;
  expanded?: boolean;
  onToggleExpanded?: () => void;
  sidebarWidth?: number;
  onSidebarWidthChange?: (width: number) => void;
  onNewChat?: () => void;
  onOpenSession?: (sessionId: string) => void;
  activeSessionId?: string | null;
  sessionsRefreshKey?: string | null;
  fontSize?: number;
  onFontSizeChange?: (px: number) => void;
  settingsCategory?: SettingsCategory;
  onSettingsCategoryChange?: (category: SettingsCategory) => void;
  onCloseSettings?: () => void;
  onOpenCommandPalette?: () => void;
  activeSessionRunStatus?: string | null;
  sessionRunStatuses?: Record<string, string>;
  // Inbox notification props
  unreadCount?: number;
  notifications?: NotificationItem[];
  unreadIds?: string[];
  onNotificationClick?: (item: NotificationItem) => void;
  onMarkAllRead?: () => void;
  onClearRead?: () => void;
  onInboxOpenChange?: (open: boolean) => void;
  onReplayOnboarding?: () => void;
  onReplayWelcome?: () => void;
  accountStatus?: EOStageAccountStatus | null;
  onOpenLogin?: () => void;
  onOpenBilling?: (tab?: "balance" | "usage") => void;
  onRefreshAccount?: () => void;
  onLogoutAccount?: () => void;
}

export {
  SHELL_FONT_SIZES,
  MIN_SHELL_FONT_SIZE,
  MAX_SHELL_FONT_SIZE,
  DEFAULT_SHELL_FONT_SIZE,
} from "../SettingsModal";

export const NAV_ITEMS = [
  { route: "workbench", labelKey: "shell.nav.mapWorkbench", icon: Map },
  { route: "downloads", labelKey: "shell.nav.downloads", icon: Download },
  { route: "plugins", labelKey: "shell.nav.plugins", icon: Blocks },
] as const;

const DEFAULT_SIDEBAR_WIDTH = 248;
const MIN_SIDEBAR_WIDTH = 200;
const MAX_SIDEBAR_WIDTH = 360;
const SIDEBAR_WIDTH_STORAGE_KEY = "eostage.shell.sidebarWidth";
const COLLAPSED_RAIL_WIDTH = 52;
const DRAG_COLLAPSE_BELOW = 150;
const DRAG_EXPAND_ABOVE = 120;
const DRAG_HYSTERESIS = 24;
const SIDEBAR_MOTION_MS = 260;
const REDUCED_MOTION_QUERY = "(prefers-reduced-motion: reduce)";

function usePrefersReducedMotion(): boolean {
  const [reduced, setReduced] = useState<boolean>(
    () =>
      typeof window !== "undefined" &&
      typeof window.matchMedia === "function" &&
      window.matchMedia(REDUCED_MOTION_QUERY).matches
  );
  useEffect(() => {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") return;
    const query = window.matchMedia(REDUCED_MOTION_QUERY);
    const onChange = () => setReduced(query.matches);
    query.addEventListener?.("change", onChange);
    return () => query.removeEventListener?.("change", onChange);
  }, []);
  return reduced;
}

function nextFrame(callback: () => void): () => void {
  if (typeof window.requestAnimationFrame !== "function") {
    const timer = window.setTimeout(callback, 16);
    return () => window.clearTimeout(timer);
  }
  let inner = 0;
  const outer = window.requestAnimationFrame(() => {
    inner = window.requestAnimationFrame(callback);
  });
  return () => {
    window.cancelAnimationFrame?.(outer);
    if (inner) window.cancelAnimationFrame?.(inner);
  };
}

function readPersistedSidebarWidth(): number {
  try {
    const saved =
      typeof window !== "undefined"
        ? window.localStorage.getItem(SIDEBAR_WIDTH_STORAGE_KEY)
        : null;
    if (saved) {
      const num = parseInt(saved, 10);
      if (!Number.isNaN(num) && num >= MIN_SIDEBAR_WIDTH && num <= MAX_SIDEBAR_WIDTH) {
        return num;
      }
    }
  } catch {
    // ignore
  }
  return DEFAULT_SIDEBAR_WIDTH;
}

function writePersistedSidebarWidth(w: number): void {
  try {
    if (typeof window !== "undefined") {
      window.localStorage.setItem(SIDEBAR_WIDTH_STORAGE_KEY, String(w));
    }
  } catch {
    // ignore
  }
}

export function DesktopActivityRail({
  activeRoute,
  onNavigate,
  downloadsAlert = false,
  onOpenDownloads,
  onOpenDataCatalog,
  dataCatalogOpen = false,
  activeDataSourceLabel,
  onOpenSettingsCategory,
  expanded = true,
  onToggleExpanded,
  sidebarWidth: controlledWidth,
  onSidebarWidthChange,
  onNewChat,
  onOpenSession,
  activeSessionId,
  sessionsRefreshKey,
  settingsCategory,
  onSettingsCategoryChange,
  onCloseSettings,
  onOpenCommandPalette,
  activeSessionRunStatus,
  sessionRunStatuses,
  unreadCount = 0,
  notifications = [],
  unreadIds = [],
  onNotificationClick,
  onMarkAllRead,
  onClearRead,
  onInboxOpenChange,
  onReplayOnboarding,
  onReplayWelcome,
  accountStatus,
  onOpenLogin,
  onOpenBilling,
  onRefreshAccount,
  onLogoutAccount,
}: DesktopActivityRailProps) {
  const { t } = useI18n();
  const [localWidth, setLocalWidth] = useState<number>(readPersistedSidebarWidth);
  const currentWidth = controlledWidth ?? localWidth;
  const navItems = NAV_ITEMS.map((item) => ({ ...item, label: t(item.labelKey) }));
  const [isDragging, setIsDragging] = useState(false);
  const [dragLiveWidth, setDragLiveWidth] = useState<number | null>(null);
  const [dragZoneExpanded, setDragZoneExpanded] = useState<boolean | null>(null);
  const [snapAnimating, setSnapAnimating] = useState(false);
  const snapTimerRef = useRef<number | null>(null);
  const reducedMotion = usePrefersReducedMotion();
  const resolvedSettingsCategory = settingsCategory ?? readLastSettingsCategory();
  const [inboxOpen, setInboxOpen] = useState(false);
  const [inboxPinned, setInboxPinned] = useState(false);
  const [hasUnreadChangelog, setHasUnreadChangelog] = useState<boolean>(() =>
    hasUnreadReleaseNotice()
  );
  useEffect(() => {
    return subscribeReleaseChangelog(() => {
      setHasUnreadChangelog(hasUnreadReleaseNotice());
    });
  }, []);
  const [helpOpen, setHelpOpen] = useState(false);
  const [accountMenuOpen, setAccountMenuOpen] = useState(false);
  const inboxRef = useRef<HTMLDivElement>(null);
  const helpRef = useRef<HTMLDivElement>(null);
  const accountRef = useRef<HTMLDivElement>(null);
  const navRef = useRef<HTMLElement | null>(null);
  const expandedSheetRef = useRef<HTMLDivElement | null>(null);
  const headerBlockRef = useRef<HTMLDivElement | null>(null);
  const headerNaturalHeightRef = useRef<number>(44);
  const helpBtnWrapRef = useRef<HTMLDivElement | null>(null);
  const primaryNavRef = useRef<HTMLDivElement | null>(null);
  const navHighlightRef = useRef<HTMLDivElement | null>(null);
  const navBtnRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const [hoveredNavIdx, setHoveredNavIdx] = useState<number | null>(null);
  const [springNavReady, setSpringNavReady] = useState(false);
  const [springWidthActive, setSpringWidthActive] = useState(false);
  const [springBrowserMode, setSpringBrowserMode] = useState(
    () =>
      typeof window !== "undefined" &&
      typeof window.requestAnimationFrame === "function" &&
      !(
        typeof window.matchMedia === "function" &&
        window.matchMedia(REDUCED_MOTION_QUERY).matches
      )
  );

  const widthSpringRef = useRef<Spring1D>(
    createSpring(
      expanded ? currentWidth : COLLAPSED_RAIL_WIDTH,
      SPRING_PRESETS.sidebarExpand.k,
      SPRING_PRESETS.sidebarExpand.c
    )
  );
  const railRafRef = useRef<number | null>(null);
  const railSettleTimerRef = useRef<number | null>(null);
  const navHighlightStateRef = useRef(createStretchyHighlight());
  const navHighlightRafRef = useRef<number | null>(null);
  const navHighlightSettleTimerRef = useRef<number | null>(null);
  const prevSpringWidthActiveRef = useRef<boolean>(false);

  useEffect(
    () => () => {
      if (snapTimerRef.current !== null) window.clearTimeout(snapTimerRef.current);
      if (railSettleTimerRef.current !== null) {
        window.clearTimeout(railSettleTimerRef.current);
      }
      if (navHighlightSettleTimerRef.current !== null) {
        window.clearTimeout(navHighlightSettleTimerRef.current);
      }
      if (railRafRef.current !== null && typeof window !== "undefined") {
        window.cancelAnimationFrame(railRafRef.current);
      }
      if (navHighlightRafRef.current !== null && typeof window !== "undefined") {
        window.cancelAnimationFrame(navHighlightRafRef.current);
      }
    },
    []
  );

  // 切换路由或开关弹窗时立即清空悬停态，避免离开前残留高亮底色块
  useEffect(() => {
    setHoveredNavIdx(null);
  }, [activeRoute, dataCatalogOpen]);

  // 仅在存在悬停项时监听窗口指针边界与失焦，彻底兜底 WKWebView 偶发吞掉 mouseleave 导致的阴影块滞留
  useEffect(() => {
    if (hoveredNavIdx === null || typeof window === "undefined") return;
    const clearHover = () => setHoveredNavIdx(null);
    const onPointerMove = (ev: PointerEvent) => {
      const container = primaryNavRef.current;
      if (!container) {
        setHoveredNavIdx(null);
        return;
      }
      const rect = container.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) return;
      if (
        ev.clientX < rect.left + 1 ||
        ev.clientX > rect.right - 1 ||
        ev.clientY < rect.top + 1 ||
        ev.clientY > rect.bottom - 1
      ) {
        setHoveredNavIdx(null);
      }
    };
    window.addEventListener("pointermove", onPointerMove, { passive: true });
    window.addEventListener("blur", clearHover);
    return () => {
      window.removeEventListener("pointermove", onPointerMove);
      window.removeEventListener("blur", clearHover);
    };
  }, [hoveredNavIdx]);

  const targetExpanded = isDragging && dragZoneExpanded !== null ? dragZoneExpanded : expanded;

  const [renderExpanded, setRenderExpanded] = useState(expanded);
  useEffect(() => {
    if (targetExpanded === renderExpanded) return;
    if (targetExpanded || reducedMotion) {
      setRenderExpanded(targetExpanded);
      return;
    }
    // 真实浏览器弹簧收起期间保持同一套图标与渐变层连续收拢，由弹簧落定回调精确切换，避免中途卸载会话树造成主线程掉帧卡顿
    if (canAnimateSpringInBrowser(navRef.current)) {
      return;
    }
    const timer = window.setTimeout(() => setRenderExpanded(false), SIDEBAR_MOTION_MS);
    return () => window.clearTimeout(timer);
  }, [targetExpanded, renderExpanded, reducedMotion]);

  const isExpandedView = targetExpanded || renderExpanded;

  const [contentShown, setContentShown] = useState(true);
  const prevTargetExpandedRef = useRef(targetExpanded);
  useLayoutEffect(() => {
    const wasExpanded = prevTargetExpandedRef.current;
    prevTargetExpandedRef.current = targetExpanded;
    if (wasExpanded === targetExpanded) return;
    if (reducedMotion || !targetExpanded) {
      setContentShown(true);
      return;
    }
    setContentShown(false);
    return nextFrame(() => setContentShown(true));
  }, [targetExpanded, reducedMotion]);
  const contentVisible = !isExpandedView || (contentShown && targetExpanded);

  useEffect(() => {
    if (!targetExpanded && !inboxPinned) {
      setInboxOpen(false);
      setAccountMenuOpen(false);
    }
  }, [targetExpanded, inboxPinned]);

  const clearSnapAnimation = useCallback(() => {
    if (snapTimerRef.current !== null) {
      window.clearTimeout(snapTimerRef.current);
      snapTimerRef.current = null;
    }
    setSnapAnimating(false);
  }, []);

  const startSnapAnimation = useCallback(() => {
    if (snapTimerRef.current !== null) window.clearTimeout(snapTimerRef.current);
    setSnapAnimating(true);
    snapTimerRef.current = window.setTimeout(() => {
      snapTimerRef.current = null;
      setSnapAnimating(false);
    }, SIDEBAR_MOTION_MS);
  }, []);

  useEffect(() => {
    if (!accountMenuOpen) return;
    const handleClickOutside = (e: MouseEvent) => {
      if (accountRef.current && !accountRef.current.contains(e.target as Node)) {
        setAccountMenuOpen(false);
      }
    };
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setAccountMenuOpen(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [accountMenuOpen]);

  const updateWidth = useCallback(
    (w: number) => {
      const clamped = Math.max(MIN_SIDEBAR_WIDTH, Math.min(MAX_SIDEBAR_WIDTH, Math.round(w)));
      setLocalWidth(clamped);
      writePersistedSidebarWidth(clamped);
      onSidebarWidthChange?.(clamped);
    },
    [onSidebarWidthChange]
  );

  const handleOpenSettings = () => {
    const lastCategory = readLastSettingsCategory();
    if (onOpenSettingsCategory) {
      onOpenSettingsCategory(lastCategory);
    }
    onNavigate("settings");
  };

  const handleCloseSettings = () => {
    if (onCloseSettings) {
      onCloseSettings();
    } else {
      onNavigate("agent");
    }
  };

  const handleResetWidth = useCallback(() => {
    updateWidth(DEFAULT_SIDEBAR_WIDTH);
    if (!expanded && onToggleExpanded) {
      onToggleExpanded();
    }
  }, [expanded, onToggleExpanded, updateWidth]);

  const handlePointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    try {
      e.currentTarget.setPointerCapture(e.pointerId);
    } catch {
      // ignore
    }
    const handle = e.currentTarget;
    const prevBodyCursor = typeof document !== "undefined" ? document.body.style.cursor : "";
    const prevBodyUserSelect = typeof document !== "undefined" ? document.body.style.userSelect : "";
    if (typeof document !== "undefined") {
      document.body.style.cursor = "col-resize";
      document.body.style.userSelect = "none";
    }
    setIsDragging(true);
    setDragZoneExpanded(expanded);

    const startX = e.clientX;
    const startW = expanded ? currentWidth : COLLAPSED_RAIL_WIDTH;
    let zoneExpanded = expanded;
    const collapseBelow = expanded ? DRAG_COLLAPSE_BELOW : DRAG_EXPAND_ABOVE - DRAG_HYSTERESIS;
    const expandAbove = expanded ? DRAG_COLLAPSE_BELOW + DRAG_HYSTERESIS : DRAG_EXPAND_ABOVE;

    const onPointerMove = (ev: PointerEvent) => {
      const rawW = startW + (ev.clientX - startX);
      const nextZone = zoneExpanded ? rawW >= collapseBelow : rawW > expandAbove;
      if (nextZone !== zoneExpanded) {
        zoneExpanded = nextZone;
        setDragZoneExpanded(nextZone);
        startSnapAnimation();
      } else if (zoneExpanded && rawW >= MIN_SIDEBAR_WIDTH) {
        clearSnapAnimation();
      }

      if (!zoneExpanded) {
        setDragLiveWidth(COLLAPSED_RAIL_WIDTH);
      } else if (expanded) {
        setDragLiveWidth(Math.max(MIN_SIDEBAR_WIDTH, Math.min(MAX_SIDEBAR_WIDTH, rawW)));
      } else {
        setDragLiveWidth(Math.min(MAX_SIDEBAR_WIDTH, Math.max(currentWidth, Math.round(rawW))));
      }
    };

    const onPointerUp = (ev: PointerEvent) => {
      window.removeEventListener("pointermove", onPointerMove);
      window.removeEventListener("pointerup", onPointerUp);
      window.removeEventListener("pointercancel", onPointerUp);
      if (typeof document !== "undefined") {
        document.body.style.cursor = prevBodyCursor;
        document.body.style.userSelect = prevBodyUserSelect;
      }
      try {
        handle.releasePointerCapture(ev.pointerId);
      } catch {
        // ignore
      }
      setIsDragging(false);
      setDragLiveWidth(null);
      setDragZoneExpanded(null);

      const finalRawW = startW + (ev.clientX - startX);
      if (expanded) {
        if (!zoneExpanded) {
          onToggleExpanded?.();
        } else {
          updateWidth(finalRawW);
        }
      } else if (zoneExpanded) {
        if (finalRawW > currentWidth) {
          updateWidth(finalRawW);
        }
        onToggleExpanded?.();
      }
    };

    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", onPointerUp);
    window.addEventListener("pointercancel", onPointerUp);
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "ArrowLeft") {
      e.preventDefault();
      if (expanded) {
        const next = currentWidth - 16;
        if (next < 150) {
          onToggleExpanded?.();
        } else {
          updateWidth(next);
        }
      }
    } else if (e.key === "ArrowRight") {
      e.preventDefault();
      if (!expanded) {
        onToggleExpanded?.();
      } else {
        updateWidth(currentWidth + 16);
      }
    } else if (e.key === "Enter") {
      e.preventDefault();
      handleResetWidth();
    }
  };

  const effectiveWidth = isDragging
    ? (dragLiveWidth ?? (expanded ? currentWidth : COLLAPSED_RAIL_WIDTH))
    : (expanded ? currentWidth : COLLAPSED_RAIL_WIDTH);
  const sheetWidth = Math.max(
    MIN_SIDEBAR_WIDTH,
    effectiveWidth >= MIN_SIDEBAR_WIDTH ? effectiveWidth : currentWidth
  );
  const widthAnimating = !reducedMotion && !springWidthActive && (!isDragging || snapAnimating);

  // `+ 新建会话` 仅作为触发动作，不锁定固定选中阴影块，悬停滑出后彻底消失
  const activePrimaryNavIdx = dataCatalogOpen
    ? 3
    : activeRoute === "workbench"
      ? 1
      : activeRoute === "downloads"
        ? 2
        : activeRoute === "history"
          ? 4
          : activeRoute === "plugins"
            ? 5
            : null;

  // 4-substep 1D 侧边栏丝滑弹簧收放：收起时文字带右缘羽化渐变柔和消融、同一组导航图标连续上移归位零生硬切换（jsdom 与 reduced-motion 保持同步静态）
  const prevSpringExpandedRef = useRef<boolean>(targetExpanded);
  const prevRouteForRailRef = useRef<ShellRoute | null>(activeRoute);
  useLayoutEffect(() => {
    const navEl = navRef.current;
    const wasExpanded = prevSpringExpandedRef.current;
    prevSpringExpandedRef.current = targetExpanded;
    const prevRoute = prevRouteForRailRef.current;
    prevRouteForRailRef.current = activeRoute;
    const leavingSettingsToCollapsed =
      prevRoute === "settings" && activeRoute !== "settings" && !targetExpanded;

    const resetHelpWrapStyles = () => {
      const helpWrap = helpBtnWrapRef.current;
      if (!helpWrap) return;
      helpWrap.style.width = "";
      helpWrap.style.marginRight = "";
      helpWrap.style.overflow = "";
      helpWrap.style.opacity = "";
      helpWrap.style.transform = "";
    };

    const clearInlineRailMorph = () => {
      const sheet = expandedSheetRef.current;
      const hdr = headerBlockRef.current;
      if (sheet) {
        sheet.style.maskImage = "";
        sheet.style.webkitMaskImage = "";
      }
      if (hdr) {
        hdr.style.height = "";
        hdr.style.paddingTop = "";
        hdr.style.paddingBottom = "";
        hdr.style.opacity = "";
        hdr.style.overflow = "";
      }
      if (primaryNavRef.current) {
        primaryNavRef.current.style.paddingTop = "";
      }
      resetHelpWrapStyles();
      if (navEl) {
        navEl.querySelectorAll<HTMLElement>("[data-rail-fade]").forEach((el) => {
          el.style.opacity = "";
          el.style.transform = "";
        });
      }
    };

    if (
      !canAnimateSpringInBrowser(navEl) ||
      reducedMotion ||
      activeRoute === "settings" ||
      leavingSettingsToCollapsed
    ) {
      if (railRafRef.current !== null) {
        window.cancelAnimationFrame(railRafRef.current);
        railRafRef.current = null;
      }
      if (railSettleTimerRef.current !== null) {
        window.clearTimeout(railSettleTimerRef.current);
        railSettleTimerRef.current = null;
      }
      clearInlineRailMorph();
      setSpringBrowserMode(false);
      setSpringWidthActive(false);
      snapSpring(widthSpringRef.current, effectiveWidth);
      if (navEl) {
        navEl.style.width = `${effectiveWidth}px`;
      }
      if (renderExpanded !== targetExpanded && (activeRoute === "settings" || !targetExpanded)) {
        setRenderExpanded(targetExpanded);
      }
      return;
    }

    setSpringBrowserMode(true);
    if (
      headerBlockRef.current &&
      !headerBlockRef.current.style.height &&
      headerBlockRef.current.offsetHeight > 24 &&
      targetExpanded &&
      wasExpanded
    ) {
      headerNaturalHeightRef.current = headerBlockRef.current.offsetHeight;
    }

    if (isDragging && !snapAnimating) {
      if (railRafRef.current !== null) {
        window.cancelAnimationFrame(railRafRef.current);
        railRafRef.current = null;
      }
      if (railSettleTimerRef.current !== null) {
        window.clearTimeout(railSettleTimerRef.current);
        railSettleTimerRef.current = null;
      }
      snapSpring(widthSpringRef.current, effectiveWidth);
      navEl!.style.width = `${effectiveWidth}px`;
      setSpringWidthActive(false);
      if (!targetExpanded) {
        if (navEl) {
          navEl.querySelectorAll<HTMLElement>("[data-rail-fade]").forEach((el) => {
            el.style.opacity = "0.000";
          });
        }
        if (renderExpanded) {
          setRenderExpanded(false);
        }
      } else {
        clearInlineRailMorph();
      }
      return;
    }

    if (wasExpanded === targetExpanded && Math.abs(widthSpringRef.current.t - effectiveWidth) < 0.5) {
      if (!springWidthActive) {
        snapSpring(widthSpringRef.current, effectiveWidth);
        if (navEl) navEl.style.width = `${effectiveWidth}px`;
        if (!targetExpanded) {
          clearInlineRailMorph();
          if (renderExpanded) {
            setRenderExpanded(false);
          }
        }
      }
      return;
    }

    if (railRafRef.current !== null) {
      window.cancelAnimationFrame(railRafRef.current);
      railRafRef.current = null;
    }
    if (railSettleTimerRef.current !== null) {
      window.clearTimeout(railSettleTimerRef.current);
      railSettleTimerRef.current = null;
    }

    if (wasExpanded !== targetExpanded && hoveredNavIdx !== null) {
      setHoveredNavIdx(null);
    }

    if (
      headerBlockRef.current &&
      !headerBlockRef.current.style.height &&
      headerBlockRef.current.offsetHeight > 24
    ) {
      headerNaturalHeightRef.current = headerBlockRef.current.offsetHeight;
    }

    const startW =
      Math.abs(widthSpringRef.current.x - effectiveWidth) > 2
        ? widthSpringRef.current.x
        : navEl!.offsetWidth > 0
          ? navEl!.offsetWidth
          : widthSpringRef.current.x;
    if (Math.abs(widthSpringRef.current.x - startW) > 2) {
      widthSpringRef.current.x = startW;
    }
    if (targetExpanded) {
      tuneSpring(
        widthSpringRef.current,
        SPRING_PRESETS.sidebarExpand.k,
        SPRING_PRESETS.sidebarExpand.c,
        false
      );
    } else {
      tuneSpring(
        widthSpringRef.current,
        SPRING_PRESETS.sidebarCollapse.k,
        SPRING_PRESETS.sidebarCollapse.c,
        false
      );
    }
    widthSpringRef.current.t = effectiveWidth;

    const fullExpandedW = Math.max(MIN_SIDEBAR_WIDTH, currentWidth);
    const hdrNaturalH = headerNaturalHeightRef.current;
    const fadeEls = Array.from(navEl!.querySelectorAll<HTMLElement>("[data-rail-fade]"));
    const activeBtnForMorph =
      activePrimaryNavIdx !== null ? navBtnRefs.current[activePrimaryNavIdx] : null;
    const initialActiveRect =
      activeBtnForMorph && primaryNavRef.current
        ? measureRelativeLayoutRect(primaryNavRef.current, activeBtnForMorph)
        : null;

    const applyContinuousMorph = (w: number) => {
      const p = clamp(
        (w - COLLAPSED_RAIL_WIDTH) / Math.max(1, fullExpandedW - COLLAPSED_RAIL_WIDTH),
        0,
        1
      );

      // 文字与次要区块先于右边框平滑渐变消融（p 从 1.0 降至 0.36 期间透明度平滑降为 0，并轻微左移 6px）
      // 绝不对包含常驻功能图标的容器整体加 CSS maskImage，确保导航图标从头到尾 100% 常驻清晰不消失
      const textOp = clamp((p - 0.36) / 0.64, 0, 1);
      const textShiftX = (1 - textOp) * -6;
      for (let i = 0; i < fadeEls.length; i += 1) {
        const el = fadeEls[i];
        el.style.opacity = `${textOp.toFixed(3)}`;
        el.style.transform = `translate3d(${textShiftX.toFixed(2)}px, 0, 0)`;
      }

      // 顶部文字标题栏平滑收拢高度，使下方 6 枚常驻导航图标不断帧、不卸载地平滑上滑归位
      const hdr = headerBlockRef.current;
      if (hdr) {
        const hdrOp = clamp((p - 0.3) / 0.7, 0, 1);
        hdr.style.overflow = "hidden";
        hdr.style.height = `${(hdrNaturalH * p).toFixed(2)}px`;
        hdr.style.paddingTop = `${(10 * p).toFixed(2)}px`;
        hdr.style.paddingBottom = `${(8 * p).toFixed(2)}px`;
        hdr.style.opacity = `${hdrOp.toFixed(3)}`;
      }
      if (primaryNavRef.current) {
        primaryNavRef.current.style.paddingTop = `${(4 + 4 * (1 - p)).toFixed(2)}px`;
      }

      // 激活导航项底色胶囊随宽度从整行平滑收缩为 30x30 图标方块（使用首帧预测量尺寸，避免动画期间逐帧回流掉帧）
      const pill = navHighlightRef.current;
      if (pill && initialActiveRect) {
        const expandedLeft = initialActiveRect.width > 30 ? initialActiveRect.left : 10;
        const expandedWidth =
          initialActiveRect.width > 30 ? initialActiveRect.width : Math.max(30, fullExpandedW - 20);
        const contractedLeft = expandedLeft + (11 - expandedLeft) * (1 - p);
        const contractedWidth = 30 + Math.max(0, expandedWidth - 30) * p;
        const currentRect = {
          left: contractedLeft,
          top: initialActiveRect.top,
          width: contractedWidth,
          height: initialActiveRect.height,
        };
        targetStretchyHighlight(
          navHighlightStateRef.current,
          currentRect,
          true,
          SPRING_PRESETS.trail
        );
        pill.style.transform = `translate3d(${contractedLeft.toFixed(2)}px, ${initialActiveRect.top.toFixed(2)}px, 0)`;
        pill.style.width = `${contractedWidth.toFixed(2)}px`;
        pill.style.height = `${initialActiveRect.height.toFixed(2)}px`;
        pill.style.opacity = "1";
      } else if (pill) {
        hideStretchyHighlight(navHighlightStateRef.current, true);
        pill.style.opacity = "0";
      }
    };

    const finishWidthSpringSettle = () => {
      if (railRafRef.current !== null) {
        window.cancelAnimationFrame(railRafRef.current);
        railRafRef.current = null;
      }
      if (railSettleTimerRef.current !== null) {
        window.clearTimeout(railSettleTimerRef.current);
        railSettleTimerRef.current = null;
      }
      if (!targetExpanded) {
        applyContinuousMorph(COLLAPSED_RAIL_WIDTH);
        snapSpring(widthSpringRef.current, COLLAPSED_RAIL_WIDTH);
        if (navRef.current) {
          navRef.current.style.width = `${COLLAPSED_RAIL_WIDTH}px`;
        }
        resetHelpWrapStyles();
        try {
          flushSync(() => {
            setSpringWidthActive(false);
            setRenderExpanded(false);
          });
        } catch {
          setSpringWidthActive(false);
          setRenderExpanded(false);
        }
        return;
      }
      snapSpring(widthSpringRef.current, effectiveWidth);
      clearInlineRailMorph();
      if (navRef.current) {
        navRef.current.style.width = `${effectiveWidth}px`;
      }
      const pill = navHighlightRef.current;
      const activeBtn =
        activePrimaryNavIdx !== null ? navBtnRefs.current[activePrimaryNavIdx] : null;
      const navContainer = primaryNavRef.current;
      if (pill && activeBtn && navContainer) {
        const finalRect = measureRelativeLayoutRect(navContainer, activeBtn);
        targetStretchyHighlight(
          navHighlightStateRef.current,
          finalRect,
          true,
          SPRING_PRESETS.trail
        );
        pill.style.transform = `translate3d(${finalRect.left.toFixed(2)}px, ${finalRect.top.toFixed(2)}px, 0)`;
        pill.style.width = `${finalRect.width.toFixed(2)}px`;
        pill.style.height = `${finalRect.height.toFixed(2)}px`;
        pill.style.opacity = "1";
      } else if (pill) {
        hideStretchyHighlight(navHighlightStateRef.current, true);
        pill.style.opacity = "0";
      }
      try {
        flushSync(() => {
          setContentShown(true);
          setSpringWidthActive(false);
        });
      } catch {
        setContentShown(true);
        setSpringWidthActive(false);
      }
    };

    applyContinuousMorph(startW);
    setSpringWidthActive(true);
    let last = performance.now();

    const tick = (now: number) => {
      const dt = Math.min((now - last) / 1000, 1 / 30);
      last = now;

      stepSpringSubdivided(widthSpringRef.current, dt);
      const w = targetExpanded
        ? clamp(widthSpringRef.current.x, COLLAPSED_RAIL_WIDTH, effectiveWidth)
        : Math.max(COLLAPSED_RAIL_WIDTH, widthSpringRef.current.x);

      if (navRef.current) {
        navRef.current.style.width = `${w.toFixed(2)}px`;
      }
      applyContinuousMorph(w);

      const widthDone = settleIfClose(widthSpringRef.current, 0.35, 1.2);
      if (widthDone) {
        finishWidthSpringSettle();
        return;
      }
      railRafRef.current = window.requestAnimationFrame(tick);
    };

    railRafRef.current = window.requestAnimationFrame(tick);
    railSettleTimerRef.current = window.setTimeout(finishWidthSpringSettle, 460);
  }, [activePrimaryNavIdx, activeRoute, currentWidth, effectiveWidth, isDragging, reducedMotion, snapAnimating, targetExpanded]);

  // 主导航栏共享弹性跟随高亮条（悬停与激活态平滑拉伸切换；收放宽度动画进行中由连续形变循环统一接管）
  useLayoutEffect(() => {
    const wasSpringWidthActive = prevSpringWidthActiveRef.current;
    prevSpringWidthActiveRef.current = springWidthActive;
    const justFinishedWidthSpring = wasSpringWidthActive && !springWidthActive;

    if (navHighlightSettleTimerRef.current !== null && typeof window !== "undefined") {
      window.clearTimeout(navHighlightSettleTimerRef.current);
      navHighlightSettleTimerRef.current = null;
    }

    const container = primaryNavRef.current;
    const pill = navHighlightRef.current;
    if (activeRoute === "settings" || !canAnimateSpringInBrowser(container) || !pill) {
      if (navHighlightRafRef.current !== null && typeof window !== "undefined") {
        window.cancelAnimationFrame(navHighlightRafRef.current);
        navHighlightRafRef.current = null;
      }
      hideStretchyHighlight(navHighlightStateRef.current, true);
      if (pill) {
        pill.style.opacity = "0";
      }
      setSpringNavReady(false);
      return;
    }
    setSpringNavReady(true);
    if (springWidthActive) {
      if (navHighlightRafRef.current !== null) {
        window.cancelAnimationFrame(navHighlightRafRef.current);
        navHighlightRafRef.current = null;
      }
      return;
    }

    if (!isExpandedView) {
      if (navHighlightRafRef.current !== null) {
        window.cancelAnimationFrame(navHighlightRafRef.current);
        navHighlightRafRef.current = null;
      }
      const targetIdx = hoveredNavIdx ?? activePrimaryNavIdx;
      const targetBtn = targetIdx !== null ? navBtnRefs.current[targetIdx] : null;
      if (targetBtn && container) {
        const rect = measureRelativeLayoutRect(container, targetBtn);
        targetStretchyHighlight(
          navHighlightStateRef.current,
          { left: 11, top: rect.top, width: 30, height: rect.height },
          true,
          SPRING_PRESETS.trail
        );
        const m = readStretchyHighlightMetrics(navHighlightStateRef.current);
        pill.style.transform = `translate3d(${m.left.toFixed(2)}px, ${m.top.toFixed(2)}px, 0)`;
        pill.style.width = `${m.width.toFixed(2)}px`;
        pill.style.height = `${m.height.toFixed(2)}px`;
        pill.style.opacity = `${m.opacity.toFixed(3)}`;
      } else {
        hideStretchyHighlight(navHighlightStateRef.current, true);
        pill.style.opacity = "0";
      }
      return;
    }

    const targetIdx = hoveredNavIdx ?? activePrimaryNavIdx;
    const targetBtn = targetIdx !== null ? navBtnRefs.current[targetIdx] : null;
    if (justFinishedWidthSpring && hoveredNavIdx === null) {
      if (navHighlightRafRef.current !== null) {
        window.cancelAnimationFrame(navHighlightRafRef.current);
        navHighlightRafRef.current = null;
      }
      if (targetBtn && container) {
        const rect = measureRelativeLayoutRect(container, targetBtn);
        targetStretchyHighlight(navHighlightStateRef.current, rect, true, SPRING_PRESETS.trail);
        const m = readStretchyHighlightMetrics(navHighlightStateRef.current);
        pill.style.transform = `translate3d(${m.left.toFixed(2)}px, ${m.top.toFixed(2)}px, 0)`;
        pill.style.width = `${m.width.toFixed(2)}px`;
        pill.style.height = `${m.height.toFixed(2)}px`;
        pill.style.opacity = `${m.opacity.toFixed(3)}`;
      } else {
        hideStretchyHighlight(navHighlightStateRef.current, true);
        pill.style.opacity = "0";
      }
      return;
    }
    if (targetBtn) {
      const rect = measureRelativeLayoutRect(container!, targetBtn);
      targetStretchyHighlight(navHighlightStateRef.current, rect, false, SPRING_PRESETS.trail);
      const m0 = readStretchyHighlightMetrics(navHighlightStateRef.current);
      pill.style.transform = `translate3d(${m0.left.toFixed(2)}px, ${m0.top.toFixed(2)}px, 0)`;
      pill.style.width = `${m0.width.toFixed(2)}px`;
      pill.style.height = `${m0.height.toFixed(2)}px`;
      pill.style.opacity = `${Math.max(m0.opacity, 0.01).toFixed(3)}`;
    } else {
      hideStretchyHighlight(navHighlightStateRef.current, false);
      if (parseFloat(pill.style.opacity || "0") > 0.99) {
        pill.style.opacity = "0.995";
      }
    }

    if (navHighlightRafRef.current !== null) {
      window.cancelAnimationFrame(navHighlightRafRef.current);
      navHighlightRafRef.current = null;
    }

    const applyHighlightMetrics = (m: ReturnType<typeof readStretchyHighlightMetrics>) => {
      if (!navHighlightRef.current) return;
      navHighlightRef.current.style.transform = `translate3d(${m.left.toFixed(2)}px, ${m.top.toFixed(2)}px, 0)`;
      navHighlightRef.current.style.width = `${m.width.toFixed(2)}px`;
      navHighlightRef.current.style.height = `${m.height.toFixed(2)}px`;
      navHighlightRef.current.style.opacity = `${m.opacity <= 0.008 ? "0" : m.opacity.toFixed(3)}`;
    };

    const settleNavHighlight = () => {
      if (navHighlightSettleTimerRef.current !== null) {
        window.clearTimeout(navHighlightSettleTimerRef.current);
        navHighlightSettleTimerRef.current = null;
      }
      if (navHighlightRafRef.current !== null) {
        window.cancelAnimationFrame(navHighlightRafRef.current);
        navHighlightRafRef.current = null;
      }
      stepStretchyHighlight(navHighlightStateRef.current, 1, true);
      applyHighlightMetrics(readStretchyHighlightMetrics(navHighlightStateRef.current));
    };

    let last = performance.now();
    const tickPill = (now: number) => {
      const dt = Math.min((now - last) / 1000, 1 / 30);
      last = now;
      const done = stepStretchyHighlight(navHighlightStateRef.current, dt, false);
      applyHighlightMetrics(readStretchyHighlightMetrics(navHighlightStateRef.current));
      if (!done) {
        navHighlightRafRef.current = window.requestAnimationFrame(tickPill);
      } else {
        navHighlightRafRef.current = null;
        if (navHighlightSettleTimerRef.current !== null) {
          window.clearTimeout(navHighlightSettleTimerRef.current);
          navHighlightSettleTimerRef.current = null;
        }
      }
    };
    navHighlightRafRef.current = window.requestAnimationFrame(tickPill);
    // WKWebView 后台/节流导致 rAF 停顿时的确定性落定兜底，确保移出后阴影必归 0
    navHighlightSettleTimerRef.current = window.setTimeout(settleNavHighlight, 220);
  }, [activePrimaryNavIdx, activeRoute, hoveredNavIdx, isExpandedView, sheetWidth, springWidthActive]);

  const navMotionClass = cn(
    widthAnimating
      ? "transition-[width] duration-[240ms] ease-[cubic-bezier(0.22,1,0.36,1)]"
      : "transition-none",
    !springBrowserMode && [
      "[&_[data-rail-fade]]:transition-[opacity,transform] [&_[data-rail-fade]]:duration-[180ms] [&_[data-rail-fade]]:ease-[cubic-bezier(0.22,1,0.36,1)]",
      "motion-reduce:[&_[data-rail-fade]]:transition-none",
      contentVisible
        ? "[&_[data-rail-fade]]:opacity-100 [&_[data-rail-fade]]:translate-x-0"
        : "[&_[data-rail-fade]]:opacity-0 [&_[data-rail-fade]]:-translate-x-1.5 [&_[data-rail-fade]]:pointer-events-none",
    ]
  );

  const renderDragHandle = () => (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={t("shell.nav.resizeSidebar")}
      tabIndex={0}
      onPointerDown={handlePointerDown}
      onKeyDown={handleKeyDown}
      onDoubleClick={handleResetWidth}
      className="group absolute right-0 top-0 bottom-0 z-40 flex w-2 cursor-col-resize items-center justify-center select-none outline-none"
    >
      <div
        className={cn(
          "h-full w-px transition-colors duration-150",
          isDragging
            ? "w-0.5 bg-[#2563eb]"
            : "bg-transparent group-hover:bg-[#d4d4d8] dark:group-hover:bg-[#3f3f46]"
        )}
      />
      <div
        aria-hidden="true"
        className={cn(
          "pointer-events-none absolute h-7 w-1 rounded-full transition-all duration-150",
          isDragging
            ? "scale-y-110 bg-[#2563eb] opacity-100"
            : "bg-[#a1a1aa]/70 opacity-0 group-hover:opacity-100 dark:bg-[#52525b]"
        )}
      />
    </div>
  );

  const navStyleWidth =
    springBrowserMode && !isDragging && !snapAnimating && !reducedMotion
      ? clamp(widthSpringRef.current.x, COLLAPSED_RAIL_WIDTH, MAX_SIDEBAR_WIDTH)
      : effectiveWidth;

  return (
    <nav
      ref={navRef}
      data-sidebar-state={targetExpanded ? "expanded" : "collapsed"}
      data-spring-rail="true"
      aria-label={t("shell.nav.main")}
      onMouseLeave={() => setHoveredNavIdx(null)}
      onPointerLeave={() => setHoveredNavIdx(null)}
      style={{ width: `${navStyleWidth}px` }}
      className={cn(
        "sp-quiet-sidebar relative z-40 flex h-full shrink-0 flex-col bg-transparent select-none dark:bg-transparent",
        inboxOpen || helpOpen || accountMenuOpen ? "overflow-visible" : "overflow-hidden",
        navMotionClass
      )}
    >
      {renderDragHandle()}
      {activeRoute === "settings" ? (
        !targetExpanded ? (
          <div className="flex h-full w-[52px] shrink-0 flex-col items-center justify-between pt-2 pb-2.5">
            <div className="flex w-full flex-col items-center gap-0.5">
              <button
                type="button"
                onClick={handleCloseSettings}
                aria-label={t("shell.nav.backToWorkspace")}
                title={t("shell.nav.backToWorkspace")}
                className="relative flex h-[30px] w-[30px] items-center justify-center rounded-[8px] text-[#52525b] transition-colors hover:bg-black/[0.06] hover:text-[#18181b] cursor-pointer dark:text-[#a1a1aa] dark:hover:bg-white/[0.07] dark:hover:text-white"
              >
                <ArrowLeft className="h-[15px] w-[15px] shrink-0 stroke-[1.6]" />
              </button>
              {SETTINGS_CATEGORIES.map((item) => {
                const normalizedActive =
                  resolvedSettingsCategory === "performance_usage" ||
                  resolvedSettingsCategory === "work_steps"
                    ? "appearance"
                    : resolvedSettingsCategory;
                const isActive = normalizedActive === item.key;
                const Icon = item.icon;
                const label = getSettingsCategoryLabel(item);
                return (
                  <button
                    key={item.key}
                    type="button"
                    onClick={() => {
                      rememberSettingsCategory(item.key);
                      onSettingsCategoryChange?.(item.key);
                    }}
                    aria-label={label}
                    title={label}
                    aria-current={isActive ? "page" : undefined}
                    data-active={isActive ? "true" : "false"}
                    className={cn(
                      "relative flex h-[30px] w-[30px] items-center justify-center rounded-[8px] transition-colors cursor-pointer outline-none",
                      isActive
                        ? "bg-black/[0.072] text-[#18181b] dark:bg-white/[0.09] dark:text-white"
                        : "text-[#52525b] hover:bg-black/[0.048] hover:text-[#18181b] dark:text-[#a1a1aa] dark:hover:bg-white/[0.06] dark:hover:text-white"
                    )}
                  >
                    <Icon className="h-[15px] w-[15px] shrink-0 stroke-[1.6]" />
                  </button>
                );
              })}
            </div>
          </div>
        ) : (
          <div
            style={{ width: `${sheetWidth}px`, minWidth: `${sheetWidth}px` }}
            className="flex h-full shrink-0 flex-col px-2.5 pt-2 pb-2"
          >
            <div className="flex items-center gap-1 px-1 pb-2">
              <button
                type="button"
                onClick={handleCloseSettings}
                aria-label={t("shell.nav.backToWorkspace")}
                className="flex h-8 flex-1 items-center gap-2 rounded-[9px] px-2.5 text-[13px] font-medium text-[#18181b] transition-colors hover:bg-black/[0.06] cursor-pointer overflow-hidden whitespace-nowrap dark:text-[#f4f4f5] dark:hover:bg-white/[0.07]"
              >
                <ArrowLeft className="h-4 w-4 shrink-0" />
                <span className="truncate">{t("shell.nav.back")}</span>
              </button>
            </div>

            <div className="flex flex-1 flex-col gap-3 overflow-y-auto pr-0.5" data-testid="rail-settings-grouped-nav">
              {SETTINGS_GROUPS.map((group) => (
                <div key={group.key} data-settings-group={group.key} className="flex flex-col gap-0.5">
                  <div className="px-2.5 py-1 text-xs font-semibold text-[#71717a] select-none">
                    {t(group.labelKey)}
                  </div>
                  {group.items.map((item) => {
                    const normalizedActive =
                      resolvedSettingsCategory === "performance_usage" ||
                      resolvedSettingsCategory === "work_steps"
                        ? "appearance"
                        : resolvedSettingsCategory;
                    const isActive = normalizedActive === item.key;
                    const Icon = item.icon;
                    const label = getSettingsCategoryLabel(item);
                    return (
                      <button
                        key={item.key}
                        type="button"
                        onClick={() => {
                          rememberSettingsCategory(item.key);
                          onSettingsCategoryChange?.(item.key);
                        }}
                        aria-label={label}
                        aria-current={isActive ? "page" : undefined}
                        data-active={isActive ? "true" : "false"}
                        className={cn(
                          "relative flex h-[30px] w-full items-center gap-2.5 rounded-[9px] px-2.5 text-[13px] transition-colors cursor-pointer outline-none overflow-hidden whitespace-nowrap",
                          isActive
                            ? "bg-black/[0.072] font-medium text-[#18181b] dark:bg-white/[0.09] dark:text-white"
                            : "text-[#3f3f46] hover:bg-black/[0.045] dark:text-[#a1a1aa] dark:hover:bg-white/[0.052]"
                        )}
                      >
                        <Icon className="h-4 w-4 shrink-0 text-[#52525b] dark:text-[#a1a1aa]" />
                        <span className="truncate whitespace-nowrap">{label}</span>
                      </button>
                    );
                  })}
                </div>
              ))}
            </div>
          </div>
        )
      ) : (
        <div
          ref={expandedSheetRef}
          style={{
            width: isExpandedView ? `${sheetWidth}px` : `${COLLAPSED_RAIL_WIDTH}px`,
            minWidth: isExpandedView ? `${sheetWidth}px` : `${COLLAPSED_RAIL_WIDTH}px`,
          }}
          className="flex h-full shrink-0 flex-col"
        >
          {/* 1. Top Workspace Text Header (`EOStage` / `遥感智能工作台` + Search & Inbox icons; unmounts once collapsed settles) */}
          {isExpandedView && (
            <div
              ref={headerBlockRef}
              data-rail-spring-row=""
              className="flex items-center justify-between px-3.5 pt-2.5 pb-2"
            >
              <div data-rail-fade="" className="flex min-w-0 flex-col">
                <span className="truncate text-[13px] font-semibold leading-[1.2] text-[#18181b] dark:text-[#f4f4f5]">
                  EOStage
                </span>
                <span className="truncate text-xs leading-[1.2] text-[#71717a] dark:text-[#a1a1aa]">
                  {t("shell.nav.workspaceSubtitle")}
                </span>
              </div>

              {/* Right: Search & Inbox Icon Buttons */}
              <div ref={inboxRef} data-rail-fade="" className="relative flex items-center gap-0.5">
                <button
                  type="button"
                  data-testid="rail-open-command-palette-btn"
                  onClick={() => onOpenCommandPalette?.()}
                  title={t("shell.nav.commandPaletteTitle")}
                  aria-label={t("shell.nav.commandPaletteAria")}
                  className="flex h-[27px] w-[27px] items-center justify-center rounded-[8px] text-[#52525b] transition-colors hover:bg-black/[0.06] hover:text-[#18181b] cursor-pointer dark:text-[#a1a1aa] dark:hover:bg-white/[0.07] dark:hover:text-white"
                >
                  <Search className="h-[15px] w-[15px] stroke-[1.75]" />
                </button>

                <button
                  type="button"
                  onClick={() => setInboxOpen((prev) => !prev)}
                  aria-haspopup="true"
                  aria-expanded={inboxOpen}
                  title={t("shell.titlebar.notifications")}
                  aria-label={t("shell.titlebar.notifications")}
                  className={cn(
                    "relative flex h-[27px] w-[27px] items-center justify-center rounded-[8px] text-[#52525b] transition-colors hover:bg-black/[0.06] hover:text-[#18181b] cursor-pointer dark:text-[#a1a1aa] dark:hover:bg-white/[0.07] dark:hover:text-white",
                    inboxOpen && "bg-black/[0.072] text-[#18181b] dark:bg-white/[0.09] dark:text-white"
                  )}
                >
                  <Inbox className="h-[15px] w-[15px] stroke-[1.75]" />
                  {(unreadCount > 0 || hasUnreadChangelog) && (
                    <span className="absolute top-1 right-1 h-1.5 w-1.5 rounded-full bg-[#2563eb]" />
                  )}
                </button>
              </div>
            </div>
          )}

          {/* 2. Primary Navigation Menu: Single persistent icon column across expanded, collapsing, and collapsed states */}
          <div
            ref={primaryNavRef}
            data-hovered-nav={hoveredNavIdx !== null ? String(hoveredNavIdx) : ""}
            onMouseLeave={() => setHoveredNavIdx(null)}
            onPointerLeave={() => setHoveredNavIdx(null)}
            className={cn(
              "relative flex flex-col gap-0.5",
              isExpandedView ? "px-2.5 py-1" : "px-2 pt-2 pb-1"
            )}
          >
            <div
              ref={navHighlightRef}
              data-testid="rail-spring-highlight"
              aria-hidden="true"
              className="pointer-events-none absolute top-0 left-0 rounded-[9px] bg-black/[0.072] opacity-0 dark:bg-white/[0.09]"
            />

            {/* + 新建会话 */}
            <div data-rail-spring-row="">
              <button
                ref={(el) => {
                  navBtnRefs.current[0] = el;
                }}
                type="button"
                onMouseEnter={() => setHoveredNavIdx(0)}
                onMouseLeave={(e) => {
                  const next = e.relatedTarget;
                  if (
                    !(next instanceof Node) ||
                    !primaryNavRef.current?.contains(next)
                  ) {
                    setHoveredNavIdx(null);
                  }
                }}
                onClick={() => (onNewChat ? onNewChat() : onNavigate("agent"))}
                aria-label={t("shell.titlebar.newChat")}
                title={t("shell.titlebar.newChat")}
                className={cn(
                  "relative z-10 flex h-[30px] items-center text-[13px] transition-colors cursor-pointer outline-none overflow-hidden",
                  isExpandedView ? "w-full gap-2.5 rounded-[9px] px-[10.5px]" : "mx-auto w-[30px] justify-center rounded-[8px] px-0",
                  "text-[#3f3f46] dark:text-[#a1a1aa]",
                  !springNavReady && (isExpandedView ? "hover:bg-black/[0.045] dark:hover:bg-white/[0.052]" : "hover:bg-black/[0.048] dark:hover:bg-white/[0.06]")
                )}
              >
                <Plus className="h-[15px] w-[15px] shrink-0 stroke-[1.75] text-[#52525b] dark:text-[#a1a1aa]" />
                {isExpandedView && (
                  <span data-rail-fade="" className="truncate">
                    {t("shell.titlebar.newChat")}
                  </span>
                )}
              </button>
            </div>

            {navItems.map((item) => {
              const isWorkbench = item.route === "workbench";
              const isDownloads = item.route === "downloads";
              const isActive = !dataCatalogOpen && activeRoute === item.route;
              const Icon = item.icon;
              const navIdx = isWorkbench ? 1 : isDownloads ? 2 : 5;
              const expandedText = t(item.labelKey);
              return (
                <React.Fragment key={item.route}>
                  <div data-rail-spring-row="">
                    <button
                      ref={(el) => {
                        navBtnRefs.current[navIdx] = el;
                      }}
                      type="button"
                      data-testid={`rail-nav-${item.route}`}
                      onMouseEnter={() => setHoveredNavIdx(navIdx)}
                      onMouseLeave={(e) => {
                        const next = e.relatedTarget;
                        if (
                          !(next instanceof Node) ||
                          !primaryNavRef.current?.contains(next)
                        ) {
                          setHoveredNavIdx(null);
                        }
                      }}
                      onClick={() => {
                        if (isDownloads && onOpenDownloads) onOpenDownloads();
                        onNavigate(item.route);
                      }}
                      aria-label={t(item.labelKey)}
                      title={t(item.labelKey)}
                      aria-current={isActive ? "page" : undefined}
                      className={cn(
                        "relative z-10 flex h-[30px] items-center text-[13px] transition-colors cursor-pointer outline-none overflow-hidden",
                        isExpandedView
                          ? cn(
                              "w-full rounded-[9px] px-[10.5px]",
                              isDownloads ? "justify-between gap-2" : "gap-2.5"
                            )
                          : "mx-auto w-[30px] justify-center rounded-[8px] px-0",
                        isActive
                          ? cn(
                              "font-medium text-[#18181b] dark:text-white",
                              !springNavReady && "bg-black/[0.072] dark:bg-white/[0.09]"
                            )
                          : cn(
                              "text-[#3f3f46] dark:text-[#a1a1aa]",
                              !springNavReady && (isExpandedView ? "hover:bg-black/[0.045] dark:hover:bg-white/[0.052]" : "hover:bg-black/[0.048] dark:hover:bg-white/[0.06]")
                            )
                      )}
                    >
                      {isDownloads ? (
                        <>
                          <span className="flex items-center gap-2.5 truncate">
                            <Icon className="h-[15px] w-[15px] shrink-0 stroke-[1.6] text-[#52525b] dark:text-[#a1a1aa]" />
                            {isExpandedView && (
                              <span data-rail-fade="" className="truncate">
                                {expandedText}
                              </span>
                            )}
                          </span>
                          {!isExpandedView && downloadsAlert && (
                            <span
                              className="absolute top-1 right-1 h-1.5 w-1.5 rounded-full bg-[#2563eb]"
                              aria-hidden="true"
                            />
                          )}
                          {isExpandedView && downloadsAlert && (
                            <span
                              data-rail-fade=""
                              className="h-2 w-2 shrink-0 rounded-full bg-[#2563eb]"
                              aria-hidden="true"
                            />
                          )}
                        </>
                      ) : (
                        <>
                          <Icon className="h-[15px] w-[15px] shrink-0 stroke-[1.6] text-[#52525b] dark:text-[#a1a1aa]" />
                          {isExpandedView && (
                            <span data-rail-fade="" className="truncate">
                              {expandedText}
                            </span>
                          )}
                        </>
                      )}
                    </button>
                  </div>

                  {isDownloads && (
                    <>
                      <div data-rail-spring-row="">
                        <button
                          ref={(el) => {
                            navBtnRefs.current[3] = el;
                          }}
                          type="button"
                          data-testid="rail-nav-data-catalog"
                          onMouseEnter={() => setHoveredNavIdx(3)}
                          onMouseLeave={(e) => {
                            const next = e.relatedTarget;
                            if (
                              !(next instanceof Node) ||
                              !primaryNavRef.current?.contains(next)
                            ) {
                              setHoveredNavIdx(null);
                            }
                          }}
                          onClick={() => {
                            if (onOpenDataCatalog) {
                              onOpenDataCatalog();
                            } else {
                              onNavigate("workbench");
                            }
                          }}
                          aria-label={t("shell.titlebar.dataCatalog")}
                          title={
                            activeDataSourceLabel
                              ? `${t("shell.titlebar.dataCatalog")}（${activeDataSourceLabel} · ${modShortcut("K")}）`
                              : `${t("shell.titlebar.dataCatalog")} (${modShortcut("K")})`
                          }
                          aria-expanded={dataCatalogOpen}
                          className={cn(
                            "relative z-10 flex h-[30px] items-center text-[13px] transition-colors cursor-pointer outline-none overflow-hidden",
                            isExpandedView
                              ? "w-full justify-between gap-2 rounded-[9px] px-[10.5px]"
                              : "mx-auto w-[30px] justify-center rounded-[8px] px-0",
                            dataCatalogOpen
                              ? cn(
                                  "font-medium text-[#18181b] dark:text-white",
                                  !springNavReady && "bg-black/[0.072] dark:bg-white/[0.09]"
                                )
                              : cn(
                                  "text-[#3f3f46] dark:text-[#a1a1aa]",
                                  !springNavReady && (isExpandedView ? "hover:bg-black/[0.045] dark:hover:bg-white/[0.052]" : "hover:bg-black/[0.048] dark:hover:bg-white/[0.06]")
                                )
                          )}
                        >
                          <span className="flex min-w-0 items-center gap-2.5 truncate">
                            <Layers className="h-[15px] w-[15px] shrink-0 stroke-[1.6] text-[#52525b] dark:text-[#a1a1aa]" />
                            {isExpandedView && (
                              <span data-rail-fade="" className="truncate">
                                {t("shell.titlebar.dataCatalog")}
                              </span>
                            )}
                          </span>
                          {isExpandedView && activeDataSourceLabel && (
                            <span
                              data-rail-fade=""
                              className="max-w-[84px] truncate rounded-[6px] bg-black/[0.055] px-1.5 py-0.5 text-xs text-[#52525b] dark:bg-white/[0.07] dark:text-[#a1a1aa]"
                            >
                              {activeDataSourceLabel}
                            </span>
                          )}
                        </button>
                      </div>

                      <div data-rail-spring-row="">
                        <button
                          ref={(el) => {
                            navBtnRefs.current[4] = el;
                          }}
                          type="button"
                          onMouseEnter={() => setHoveredNavIdx(4)}
                          onMouseLeave={(e) => {
                            const next = e.relatedTarget;
                            if (
                              !(next instanceof Node) ||
                              !primaryNavRef.current?.contains(next)
                            ) {
                              setHoveredNavIdx(null);
                            }
                          }}
                          onClick={() => onNavigate("history")}
                          aria-label={t("shell.nav.allChats")}
                          title={t("shell.nav.allChats")}
                          aria-current={!dataCatalogOpen && activeRoute === "history" ? "page" : undefined}
                          className={cn(
                            "relative z-10 flex h-[30px] items-center text-[13px] transition-colors cursor-pointer outline-none overflow-hidden",
                            isExpandedView
                              ? "w-full gap-2.5 rounded-[9px] px-[10.5px]"
                              : "mx-auto w-[30px] justify-center rounded-[8px] px-0",
                            !dataCatalogOpen && activeRoute === "history"
                              ? cn(
                                  "font-medium text-[#18181b] dark:text-white",
                                  !springNavReady && "bg-black/[0.072] dark:bg-white/[0.09]"
                                )
                              : cn(
                                  "text-[#3f3f46] dark:text-[#a1a1aa]",
                                  !springNavReady && (isExpandedView ? "hover:bg-black/[0.045] dark:hover:bg-white/[0.052]" : "hover:bg-black/[0.048] dark:hover:bg-white/[0.06]")
                                )
                          )}
                        >
                          <BookOpen className="h-[15px] w-[15px] shrink-0 stroke-[1.6] text-[#52525b] dark:text-[#a1a1aa]" />
                          {isExpandedView && (
                            <span data-rail-fade="" className="truncate">
                              {t("shell.nav.allChats")}
                            </span>
                          )}
                        </button>
                      </div>
                    </>
                  )}
                </React.Fragment>
              );
            })}
          </div>

          {/* 3. Sessions Group (`SidebarSessionTree`) */}
          {isExpandedView ? (
            <div data-rail-fade="" className="flex min-h-0 flex-1 flex-col">
              {onOpenSession ? (
                <SidebarSessionTree
                  activeRoute={activeRoute}
                  activeSessionId={activeSessionId}
                  onOpenSession={onOpenSession}
                  onNavigate={onNavigate}
                  refreshKey={sessionsRefreshKey}
                  fixtureSessions={
                    typeof window !== "undefined" &&
                    (window.location.search.includes("fixture=1") ||
                      window.location.hash.includes("fixture"))
                      ? historyFixtureSessionsFactory
                      : null
                  }
                  onNewChat={onNewChat}
                  activeSessionRunStatus={activeSessionRunStatus}
                  sessionRunStatuses={sessionRunStatuses}
                />
              ) : (
                <div className="flex-1" />
              )}
            </div>
          ) : (
            <div className="flex-1" />
          )}

          {/* 4. Bottom Account Card + Footer Help/Settings */}
          <div ref={accountRef} className="relative mt-auto border-t border-black/[0.06] px-[11px] pt-2 dark:border-white/[0.08]">
            <button
              type="button"
              data-testid="rail-account-btn"
              onClick={() => {
                if (!accountStatus?.logged_in) {
                  setAccountMenuOpen(false);
                  onOpenLogin?.();
                  return;
                }
                setAccountMenuOpen((prev) => !prev);
              }}
              aria-label={
                accountStatus?.logged_in
                  ? t("shell.accountRail.accountWithEmailAria", { email: accountStatus.email ?? "" })
                  : t("shell.accountRail.signInAccountAria")
              }
              title={
                accountStatus?.logged_in
                  ? `${accountStatus.email ?? t("shell.accountRail.accountLabel")} · ${accountStatus.total_credits.toLocaleString()} Credits`
                  : t("shell.accountRail.signInAccountAria")
              }
              className={cn(
                "flex items-center rounded-[8px] text-left transition-colors cursor-pointer outline-none",
                isExpandedView
                  ? "w-full gap-2 px-2 py-1.5 text-[#3f3f46] hover:bg-black/[0.048] hover:text-[#18181b] dark:text-[#d4d4d8] dark:hover:bg-white/[0.06] dark:hover:text-white"
                  : "h-[30px] w-[30px] justify-center text-[#71717a] hover:bg-black/[0.06] hover:text-[#18181b] dark:text-[#a1a1aa] dark:hover:bg-white/[0.07] dark:hover:text-white",
                accountMenuOpen && "bg-black/[0.072] dark:bg-white/[0.09]"
              )}
            >
              <User className="h-[15px] w-[15px] shrink-0 stroke-[1.6] text-[#52525b] dark:text-[#a1a1aa]" />
              {isExpandedView && (
                <div data-rail-fade="" className="flex min-w-0 flex-1 flex-col">
                  <span className="truncate text-[12.5px] font-medium leading-[1.25] text-[#18181b] dark:text-[#f4f4f5]">
                    {accountStatus?.logged_in ? accountStatus.email || t("shell.accountRail.accountLabel") : t("shell.accountRail.signInShort")}
                  </span>
                  <span className="truncate text-xs tabular-nums leading-[1.2] text-[#71717a] dark:text-[#a1a1aa]">
                    {accountStatus?.logged_in
                      ? `${accountStatus.total_credits.toLocaleString()} Credits`
                      : t("shell.accountRail.signInSubtitle")}
                  </span>
                </div>
              )}
            </button>

            <SpringDropdownSurface
              open={Boolean(accountMenuOpen && accountStatus?.logged_in)}
              data-testid="rail-account-popover"
              className={cn(
                "origin-bottom-left absolute z-50 w-60 rounded-[12px] border border-[#e4e4e7] bg-white p-2 shadow-xl dark:border-[#27272a] dark:bg-[#1e1e22]",
                isExpandedView ? "bottom-full left-2.5 mb-1.5" : "bottom-1 left-full ml-2"
              )}
            >
              {accountStatus?.logged_in && (
                <>
                  <div data-spring-item="" className="px-2 py-1.5 border-b border-[#f4f4f5] dark:border-[#27272a]">
                    <div className="truncate text-[12.5px] font-semibold text-[#18181b] dark:text-[#f4f4f5]">
                      {accountStatus.email}
                    </div>
                    <div className="mt-1 flex items-center justify-between gap-2 text-xs text-[#71717a] dark:text-[#a1a1aa]">
                      <span className="tabular-nums">
                        {t("shell.accountRail.availableCredits", { credits: accountStatus.total_credits.toLocaleString() })}
                      </span>
                      <button
                        type="button"
                        data-testid="rail-account-refresh-btn"
                        onClick={() => onRefreshAccount?.()}
                        aria-label={t("shell.accountRail.refreshCreditsAria")}
                        title={t("shell.accountRail.refreshBalanceTitle")}
                        className="flex h-6 w-6 items-center justify-center rounded text-[#71717a] hover:bg-[#f4f4f5] hover:text-[#18181b] cursor-pointer dark:hover:bg-[#27272a] dark:hover:text-white"
                      >
                        <RefreshCw className="h-3 w-3" />
                      </button>
                    </div>
                  </div>
                  <div data-spring-item="" className="py-1 space-y-0.5 text-[12.5px]">
                    <button
                      type="button"
                      data-testid="rail-account-billing-btn"
                      onClick={() => {
                        setAccountMenuOpen(false);
                        onOpenBilling?.("balance");
                      }}
                      className="flex h-8 w-full items-center rounded-[6px] px-2 text-left text-[#18181b] hover:bg-[#f4f4f5] cursor-pointer dark:text-[#e4e4e7] dark:hover:bg-[#27272a]"
                    >
                      {t("shell.accountRail.billingMenu")}
                    </button>
                    <button
                      type="button"
                      data-testid="rail-account-usage-btn"
                      onClick={() => {
                        setAccountMenuOpen(false);
                        onOpenBilling?.("usage");
                      }}
                      className="flex h-8 w-full items-center rounded-[6px] px-2 text-left text-[#18181b] hover:bg-[#f4f4f5] cursor-pointer dark:text-[#e4e4e7] dark:hover:bg-[#27272a]"
                    >
                      {t("shell.accountRail.aiUsageMenu")}
                    </button>
                    <button
                      type="button"
                      data-testid="rail-account-models-btn"
                      onClick={() => {
                        setAccountMenuOpen(false);
                        onOpenSettingsCategory?.("models");
                        onNavigate("settings");
                      }}
                      className="flex h-8 w-full items-center rounded-[6px] px-2 text-left text-[#18181b] hover:bg-[#f4f4f5] cursor-pointer dark:text-[#e4e4e7] dark:hover:bg-[#27272a]"
                    >
                      {t("shell.accountRail.officialModelsMenu")}
                    </button>
                  </div>
                  <div data-spring-item="" className="border-t border-[#f4f4f5] pt-1 dark:border-[#27272a]">
                    <button
                      type="button"
                      data-testid="rail-account-logout-btn"
                      onClick={() => {
                        setAccountMenuOpen(false);
                        onLogoutAccount?.();
                      }}
                      className="flex h-8 w-full items-center rounded-[6px] px-2 text-left text-[12.5px] text-[#71717a] hover:bg-[#f4f4f5] hover:text-[#18181b] cursor-pointer dark:text-[#a1a1aa] dark:hover:bg-[#27272a] dark:hover:text-white"
                    >
                      {t("shell.accountRail.signOutMenu")}
                    </button>
                  </div>
                </>
              )}
            </SpringDropdownSurface>
          </div>

          <div
            ref={helpRef}
            className={cn(
              "relative flex px-[11px] py-2",
              isExpandedView ? "flex-row items-center" : "flex-col items-center gap-1"
            )}
          >
            <div
              ref={helpBtnWrapRef}
              className={cn("shrink-0", isExpandedView && "mr-1")}
            >
              <button
                type="button"
                data-testid="rail-open-about-btn"
                onClick={() => setHelpOpen((prev) => !prev)}
                aria-label={t("shell.accountRail.shortcutsAndHelpAria")}
                title={t("shell.accountRail.aboutEostageTitle")}
                className={cn(
                  "flex items-center justify-center rounded-[8px] text-[#71717a] transition-colors hover:bg-black/[0.06] hover:text-[#18181b] cursor-pointer dark:text-[#a1a1aa] dark:hover:bg-white/[0.07] dark:hover:text-white",
                  isExpandedView ? "h-[27px] w-[27px]" : "h-[30px] w-[30px]"
                )}
              >
                <HelpCircle className="h-[15px] w-[15px] stroke-[1.6]" />
              </button>
            </div>

            <button
              type="button"
              data-testid="rail-open-settings-btn"
              onClick={handleOpenSettings}
              aria-label={t("shell.nav.settings")}
              title={t("shell.nav.settings")}
              className="flex h-[30px] w-[30px] items-center justify-center rounded-[8px] text-[#71717a] transition-colors hover:bg-black/[0.06] hover:text-[#18181b] cursor-pointer dark:text-[#a1a1aa] dark:hover:bg-white/[0.07] dark:hover:text-white"
            >
              <Settings className="h-[15px] w-[15px] stroke-[1.6]" />
            </button>
          </div>
        </div>
      )}
      <AboutEOStageModal
        open={helpOpen}
        onClose={() => setHelpOpen(false)}
        onReplayOnboarding={onReplayOnboarding}
        onReplayWelcome={onReplayWelcome}
      />
      <NotificationPopover
        open={inboxOpen}
        onClose={() => {
          setInboxPinned(false);
          setInboxOpen(false);
        }}
        onPresentChange={onInboxOpenChange}
        onPinnedChange={setInboxPinned}
        triggerRef={inboxRef}
        variant="side-drawer"
        emptyIcon="inbox"
        unreadCount={unreadCount}
        notifications={notifications}
        unreadIds={unreadIds}
        onNotificationClick={onNotificationClick}
        onMarkAllRead={onMarkAllRead}
        onClearRead={onClearRead}
      />
    </nav>
  );
}
