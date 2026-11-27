// EOStage 34px Titlebar with workspace & spatial controls
import React, { useState, useEffect, useRef, useMemo } from "react";
import {
  Bell,
  Minus,
  Square,
  X,
  OctagonX,
  PanelLeft,
  PanelRight,
  ArrowLeft,
  ArrowRight,
  ArrowUpRight,
  MessageSquare,
  Map as MapIcon,
  Plus,
  Settings,
  Download,
  History,
  Blocks,
  Layers,
  MapPin,
  SquarePen,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n, type MessageKey } from "@/i18n";
import { EmptyState } from "@/components/ui/EmptyState";
import {
  closeNativeWindow,
  minimizeNativeWindow,
  startNativeWindowDrag,
  toggleNativeWindowMaximize,
} from "@/lib/bridge";
import { type NotificationItem } from "@/shell/api/notificationBell";
import { NotificationPopover } from "./NotificationPopover";
import {
  SpringCheckmark,
  SpringChevron,
  SpringDropdownSurface,
  SpringSlotTriggerValue,
  useStretchyMenuHighlight,
} from "./SpringDropdownPrimitives";
import type { ShellRoute } from "@/shell/api/shellRoutes";
import type { ApplicationTask } from "@/shell/api/types";
import { formatBytes } from "@/shell/api/rowProjection";
import { normalizeTaskDisplayName } from "./downloadFilterLogic";
import { historyNavShortcut, isMacPlatform, modShortcut } from "@/lib/platformShortcuts";

export interface StopAllProps {
  enabled: boolean;
  reason_zh: string;
  busy: boolean;
  onClick: () => void;
  live_runs?: number;
  agent_active_tasks?: number;
}

export interface DesktopTitlebarProps {
  title?: string;
  titleIcon?: React.ReactNode;
  unreadCount?: number;
  notifications?: NotificationItem[];
  unreadIds?: string[];
  onNotificationClick?: (item: NotificationItem) => void;
  onMarkAllRead?: () => void;
  newChat?: { enabled: boolean; onClick: () => void };
  panelToggle?: { open: boolean; onToggle: () => void; sceneCount?: number };
  leadingBadge?: React.ReactNode;
  stopAll?: StopAllProps;
  // Navigation & Workspace Context
  sidebarExpanded?: boolean;
  onToggleSidebar?: () => void;
  activeRoute?: ShellRoute | null;
  contextSubtitle?: string | null;
  canGoBack?: boolean;
  canGoForward?: boolean;
  onGoBack?: () => void;
  onGoForward?: () => void;
  onNavigate?: (route: ShellRoute) => void;
  onStartNewChat?: () => void;
  onOpenDataCatalog?: () => void;
  // Spatial AOI & Download Telemetry in Titlebar
  aoiSummary?: string | null;
  onOpenAoiModal?: () => void;
  onClearAoi?: () => void;
  activeDownloadCount?: number;
  downloadTasks?: ApplicationTask[];
  onOpenDownloadCenter?: () => void;
  onOpenDownloadDrawer?: (taskId?: string) => void;
  // Backward-compatible optional fields
  onOpenNotifications?: () => void;
  dark?: boolean;
  onToggleDark?: () => void;
}

const ROUTE_META: Record<
  ShellRoute,
  { labelKey: MessageKey; shortcutKey: string; icon: React.ComponentType<{ className?: string }> }
> = {
  agent: { labelKey: "shell.titlebar.routes.agent", shortcutKey: "1", icon: MessageSquare },
  workbench: { labelKey: "shell.titlebar.routes.workbench", shortcutKey: "2", icon: MapIcon },
  downloads: { labelKey: "shell.titlebar.routes.downloads", shortcutKey: "3", icon: Download },
  history: { labelKey: "shell.titlebar.routes.history", shortcutKey: "4", icon: History },
  plugins: { labelKey: "shell.titlebar.routes.plugins", shortcutKey: "5", icon: Blocks },
  settings: { labelKey: "shell.titlebar.routes.settings", shortcutKey: ",", icon: Settings },
};

function taskStatusMeta(
  status: ApplicationTask["status"],
  t: (key: MessageKey) => string,
): {
  label: string;
  dotClass: string;
  badgeClass: string;
} {
  const neutralBadge =
    "border border-sp-line bg-sp-surface-muted text-sp-text-secondary";
  switch (status) {
    case "running":
      return {
        label: t("shell.titlebar.taskStatus.running"),
        dotClass: "bg-sp-accent",
        badgeClass: neutralBadge,
      };
    case "queued":
      return {
        label: t("shell.titlebar.taskStatus.queued"),
        dotClass: "bg-sp-status-warning",
        badgeClass: neutralBadge,
      };
    case "paused":
      return {
        label: t("shell.titlebar.taskStatus.paused"),
        dotClass: "bg-sp-text-muted",
        badgeClass: neutralBadge,
      };
    case "completed":
      return {
        label: t("shell.titlebar.taskStatus.completed"),
        dotClass: "bg-sp-status-success",
        badgeClass: neutralBadge,
      };
    case "failed":
    case "cancelled":
    case "interrupted":
    default:
      return {
        label:
          status === "cancelled"
            ? t("shell.titlebar.taskStatus.cancelled")
            : status === "interrupted"
              ? t("shell.titlebar.taskStatus.interrupted")
              : t("shell.titlebar.taskStatus.failed"),
        dotClass: "bg-sp-status-danger",
        badgeClass: neutralBadge,
      };
  }
}

export function DesktopTitlebar({
  title,
  titleIcon,
  unreadCount = 0,
  notifications = [],
  unreadIds = [],
  onNotificationClick,
  onMarkAllRead,
  newChat,
  panelToggle,
  leadingBadge,
  stopAll,
  sidebarExpanded = false,
  onToggleSidebar,
  activeRoute = "agent",
  contextSubtitle,
  canGoBack = false,
  canGoForward = false,
  onGoBack,
  onGoForward,
  onNavigate,
  onStartNewChat,
  onOpenDataCatalog,
  aoiSummary,
  onOpenAoiModal,
  onClearAoi,
  activeDownloadCount = 0,
  downloadTasks = [],
  onOpenDownloadCenter,
  onOpenDownloadDrawer,
  onOpenNotifications,
}: DesktopTitlebarProps) {
  const { t } = useI18n();
  const [workspaceMenuOpen, setWorkspaceMenuOpen] = useState(false);
  const [downloadPopoverOpen, setDownloadPopoverOpen] = useState(false);
  const [popoverOpen, setPopoverOpen] = useState(false);
  const [wsSurfacePresent, setWsSurfacePresent] = useState(false);
  const [dlSurfacePresent, setDlSurfacePresent] = useState(false);
  const [notifSurfacePresent, setNotifSurfacePresent] = useState(false);
  const workspaceMenuRef = useRef<HTMLDivElement>(null);
  const workspaceTriggerRef = useRef<HTMLButtonElement>(null);
  const downloadPopoverRef = useRef<HTMLDivElement>(null);
  const downloadTriggerRef = useRef<HTMLButtonElement>(null);
  const bellRef = useRef<HTMLButtonElement>(null);
  const wsHighlight = useStretchyMenuHighlight({ x: 0, y: 0 });
  const dlHighlight = useStretchyMenuHighlight({ x: 0, y: 0, variant: "card" });

  const handleMinimize = () => {
    void minimizeNativeWindow();
  };

  const handleMaximize = () => {
    void toggleNativeWindowMaximize();
  };

  const handleClose = () => {
    void closeNativeWindow();
  };

  useEffect(() => {
    if (!workspaceMenuOpen) {
      wsHighlight.resetHighlight();
    }
  }, [workspaceMenuOpen, wsHighlight.resetHighlight]);

  useEffect(() => {
    if (!downloadPopoverOpen) {
      dlHighlight.resetHighlight();
    }
  }, [downloadPopoverOpen, dlHighlight.resetHighlight]);

  useEffect(() => {
    if (!workspaceMenuOpen) return;
    const handleClickOutside = (e: MouseEvent) => {
      if (workspaceMenuRef.current && !workspaceMenuRef.current.contains(e.target as Node)) {
        setWorkspaceMenuOpen(false);
      }
    };
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        setWorkspaceMenuOpen(false);
        workspaceTriggerRef.current?.focus();
        return;
      }
      if (e.key === "ArrowDown" || e.key === "ArrowUp" || e.key === "Home" || e.key === "End") {
        const container = wsHighlight.containerRef.current;
        if (!container) return;
        const items = Array.from(
          container.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:not([disabled])')
        );
        if (items.length === 0) return;
        e.preventDefault();
        const activeIdx = items.findIndex((item) => item === document.activeElement);
        let nextIdx = 0;
        if (e.key === "Home") {
          nextIdx = 0;
        } else if (e.key === "End") {
          nextIdx = items.length - 1;
        } else if (e.key === "ArrowDown") {
          nextIdx = activeIdx >= 0 ? (activeIdx + 1) % items.length : 0;
        } else {
          nextIdx = activeIdx >= 0 ? (activeIdx - 1 + items.length) % items.length : items.length - 1;
        }
        const nextItem = items[nextIdx];
        nextItem.focus();
        wsHighlight.activateElement(nextItem);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [workspaceMenuOpen, wsHighlight.activateElement, wsHighlight.containerRef]);

  useEffect(() => {
    if (!downloadPopoverOpen) return;
    const handleClickOutside = (e: MouseEvent) => {
      if (downloadPopoverRef.current && !downloadPopoverRef.current.contains(e.target as Node)) {
        setDownloadPopoverOpen(false);
      }
    };
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        setDownloadPopoverOpen(false);
        downloadTriggerRef.current?.focus();
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    window.addEventListener("keydown", handleKeyDown, true);
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
      window.removeEventListener("keydown", handleKeyDown, true);
    };
  }, [downloadPopoverOpen]);

  const currentRouteKey: ShellRoute = activeRoute ?? "agent";
  const currentMeta = ROUTE_META[currentRouteKey] ?? ROUTE_META.agent;
  const CurrentRouteIcon = currentMeta.icon;

  const previewTasks = useMemo(() => {
    const priority = (s: ApplicationTask["status"]) =>
      s === "running" ? 0 : s === "queued" ? 1 : s === "paused" ? 2 : s === "failed" ? 3 : 4;
    return [...downloadTasks]
      .sort((a, b) => priority(a.status) - priority(b.status))
      .slice(0, 5);
  }, [downloadTasks]);

  const showDownloadTrigger = Boolean(
    onOpenDownloadCenter || onOpenDownloadDrawer || activeDownloadCount > 0 || downloadTasks.length > 0
  );

  const lastTitlebarMaximizeAtRef = useRef<number>(0);

  const toggleTitlebarMaximizeDeduplicated = () => {
    const now = Date.now();
    if (now - lastTitlebarMaximizeAtRef.current < 280) {
      return;
    }
    lastTitlebarMaximizeAtRef.current = now;
    void toggleNativeWindowMaximize();
  };

  const isInteractiveTitlebarTarget = (target: EventTarget | null): boolean => {
    const candidate = target as { closest?: (selector: string) => Element | null } | null;
    if (!candidate || typeof candidate.closest !== "function") return false;
    return Boolean(
      candidate.closest(
        "button, a, input, select, textarea, [role='button'], [role='dialog'], [role='menu'], [data-no-drag]",
      ),
    );
  };

  const handleHeaderMouseDown = (e: React.MouseEvent<HTMLElement>) => {
    if (e.button !== 0 || isInteractiveTitlebarTarget(e.target)) {
      return;
    }
    // 阻止 mousedown 冒泡到 document，避免与 Tauri 内置 data-tauri-drag-region 监听器重复触发拖拽/双击最大化
    e.stopPropagation();
    if (e.detail === 2) {
      e.preventDefault();
      toggleTitlebarMaximizeDeduplicated();
      return;
    }
    if (e.detail > 2) {
      e.preventDefault();
      return;
    }
    const startX = e.screenX;
    const startY = e.screenY;
    let cleaned = false;
    const cleanup = () => {
      if (cleaned) return;
      cleaned = true;
      window.removeEventListener("mousemove", onMove, true);
      window.removeEventListener("mouseup", cleanup, true);
    };
    const onMove = (moveEv: MouseEvent) => {
      if ((moveEv.buttons & 1) === 0) {
        cleanup();
        return;
      }
      if (Math.hypot(moveEv.screenX - startX, moveEv.screenY - startY) >= 3) {
        cleanup();
        void startNativeWindowDrag();
      }
    };
    window.addEventListener("mousemove", onMove, true);
    window.addEventListener("mouseup", cleanup, true);
  };

  const handleHeaderMouseUp = (e: React.MouseEvent<HTMLElement>) => {
    if (e.button !== 0 || isInteractiveTitlebarTarget(e.target)) {
      return;
    }
    if (e.detail === 2) {
      e.stopPropagation();
    }
  };

  const isMac = isMacPlatform();

  return (
    <header
      data-tauri-drag-region
      onMouseDown={handleHeaderMouseDown}
      onMouseUp={handleHeaderMouseUp}
      onDoubleClick={(e) => {
        if (isInteractiveTitlebarTarget(e.target)) {
          return;
        }
        e.stopPropagation();
        toggleTitlebarMaximizeDeduplicated();
      }}
      className={cn(
        "sp-quiet-titlebar relative grid grid-cols-[minmax(max-content,1fr)_minmax(0,auto)_minmax(max-content,1fr)] h-[36px] w-full shrink-0 items-center justify-between bg-transparent pl-2.5 text-[#18181b] select-none cursor-default dark:bg-transparent dark:text-[#f4f4f5]",
        isMac ? "pr-2" : "pr-1",
        workspaceMenuOpen ||
        downloadPopoverOpen ||
        popoverOpen ||
        wsSurfacePresent ||
        dlSurfacePresent ||
        notifSurfacePresent
          ? "z-[2560]"
          : "z-50"
      )}
    >
      {/* Left: macOS Traffic Lights (on Mac) + Sidebar Toggle + History Back/Forward */}
      <div
        data-tauri-drag-region
        className="flex min-w-0 items-center gap-0.5 h-full"
      >
        {isMac && (
          <div
            className="ml-1 mr-2 flex items-center gap-[7px] shrink-0"
            style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
            aria-label={t("shell.titlebar.windowControls")}
          >
            <button
              type="button"
              onClick={handleClose}
              title={t("shell.titlebar.closeWindow")}
              aria-label={t("shell.titlebar.closeWindow")}
              data-testid="macos-traffic-close"
              className="group/tl flex h-[12px] w-[12px] items-center justify-center rounded-full border border-[#e0443e] bg-[#ff5f57] text-[#4d0000] transition-opacity hover:opacity-90 cursor-pointer"
            >
              <X className="h-2 w-2 opacity-0 group-hover/tl:opacity-100" strokeWidth={2.2} />
            </button>
            <button
              type="button"
              onClick={handleMinimize}
              title={t("shell.titlebar.minimize")}
              aria-label={t("shell.titlebar.minimize")}
              data-testid="macos-traffic-minimize"
              className="group/tl flex h-[12px] w-[12px] items-center justify-center rounded-full border border-[#d89e24] bg-[#febc2e] text-[#5c3c00] transition-opacity hover:opacity-90 cursor-pointer"
            >
              <Minus className="h-2 w-2 opacity-0 group-hover/tl:opacity-100" strokeWidth={2.2} />
            </button>
            <button
              type="button"
              onClick={handleMaximize}
              title={t("shell.titlebar.maximizeRestore")}
              aria-label={t("shell.titlebar.maximizeRestore")}
              data-testid="macos-traffic-maximize"
              className="group/tl flex h-[12px] w-[12px] items-center justify-center rounded-full border border-[#1aab29] bg-[#28c840] text-[#004d00] transition-opacity hover:opacity-90 cursor-pointer"
            >
              <span className="text-[8px] font-bold leading-none opacity-0 group-hover/tl:opacity-100">+</span>
            </button>
          </div>
        )}

        {/* 1. Sidebar Toggle Button */}
        <button
          type="button"
          onClick={onToggleSidebar}
          aria-label={sidebarExpanded ? t("shell.nav.collapseSidebar") : t("shell.nav.expandSidebar")}
          title={
            sidebarExpanded
              ? `${t("shell.nav.collapseSidebar")} (${modShortcut("B")})`
              : `${t("shell.nav.expandSidebar")} (${modShortcut("B")})`
          }
          className="flex h-[26px] w-[26px] shrink-0 items-center justify-center rounded-[7px] text-[#52525b] transition-colors hover:bg-black/[0.06] hover:text-[#18181b] cursor-pointer dark:text-[#a1a1aa] dark:hover:bg-white/[0.07] dark:hover:text-white"
        >
          <PanelLeft className="h-[14px] w-[14px] stroke-[1.75]" />
        </button>

        {/* 2. Real Navigation History Back / Forward Arrows */}
        <div className="mx-0.5 flex items-center gap-0.5 shrink-0">
          <button
            type="button"
            disabled={!canGoBack}
            onClick={() => onGoBack?.()}
            title={`${t("shell.titlebar.navBack")} (${historyNavShortcut("back")})`}
            aria-label={t("shell.titlebar.navBack")}
            className={cn(
              "flex h-[26px] w-[24px] items-center justify-center rounded-[7px] transition-colors",
              canGoBack
                ? "text-[#52525b] hover:bg-black/[0.06] hover:text-[#18181b] cursor-pointer dark:text-[#a1a1aa] dark:hover:bg-white/[0.07] dark:hover:text-white"
                : "text-[#c8c9ce] cursor-not-allowed dark:text-[#3f3f46]"
            )}
          >
            <ArrowLeft className="h-3.5 w-3.5 stroke-[1.75]" />
          </button>
          <button
            type="button"
            disabled={!canGoForward}
            onClick={() => onGoForward?.()}
            title={`${t("shell.titlebar.navForward")} (${historyNavShortcut("forward")})`}
            aria-label={t("shell.titlebar.navForward")}
            className={cn(
              "flex h-[26px] w-[24px] items-center justify-center rounded-[7px] transition-colors",
              canGoForward
                ? "text-[#52525b] hover:bg-black/[0.06] hover:text-[#18181b] cursor-pointer dark:text-[#a1a1aa] dark:hover:bg-white/[0.07] dark:hover:text-white"
                : "text-[#c8c9ce] cursor-not-allowed dark:text-[#3f3f46]"
            )}
          >
            <ArrowRight className="h-3.5 w-3.5 stroke-[1.75]" />
          </button>
        </div>

        {/* 3. Leading Badge & Active Context Subtitle */}
        <div data-tauri-drag-region className="flex min-w-0 items-center gap-1.5 pl-1">
          {leadingBadge && (
            <div className="shrink-0 flex items-center" data-no-drag onMouseDown={(e) => e.stopPropagation()}>
              {leadingBadge}
            </div>
          )}

          {contextSubtitle && (
            <span
              data-tauri-drag-region
              className="max-w-[120px] lg:max-w-[280px] truncate text-[12px] font-medium text-[#52525b] dark:text-[#a1a1aa]"
              title={contextSubtitle}
            >
              {contextSubtitle}
            </span>
          )}
        </div>
      </div>

      {/* Center: Clean Draggable Region */}
      <div data-tauri-drag-region className="flex flex-1 items-center justify-center gap-1.5 h-full min-w-0 px-2">
        {titleIcon && <span data-tauri-drag-region className="shrink-0 flex items-center">{titleIcon}</span>}
        <span data-tauri-drag-region className="truncate text-xs font-medium text-[#52525b] dark:text-[#a1a1aa]">
          {title ?? t("shell.titlebar.defaultTitle")}
        </span>
      </div>

      {/* Right: Spatial AOI Pill + Download Concise Popover + Spatial Panel Toggle + Workspace Switcher (replacing Stop All) + Notifications + Window Controls */}
      <div
        data-tauri-drag-region
        className="flex items-center justify-end gap-1 lg:gap-1.5 h-full"
      >
        {/* 0. New Chat Button (when passed, e.g. when sidebar is collapsed) */}
        {newChat && (
          <button
            type="button"
            disabled={!newChat.enabled}
            onClick={newChat.onClick}
            aria-label={t("shell.titlebar.newChat")}
            title={t("shell.titlebar.newChat")}
            className={cn(
              "flex h-[24px] items-center gap-1.5 px-2 rounded-[5px] text-xs font-medium transition-colors select-none whitespace-nowrap shrink-0",
              !newChat.enabled
                ? "text-[#a1a1aa] cursor-not-allowed opacity-60 dark:text-[#52525b]"
                : "text-[#52525b] hover:bg-[#f4f4f5] hover:text-[#18181b] cursor-pointer dark:text-[#a1a1aa] dark:hover:bg-[#27272a] dark:hover:text-white"
            )}
          >
            <SquarePen className="h-3.5 w-3.5 shrink-0" />
            <span className="hidden min-[1280px]:inline">{t("shell.titlebar.newChat")}</span>
          </button>
        )}

        {/* 1. Real Spatial AOI Status Pill */}
        {onOpenAoiModal && (
          <div className="flex items-center rounded-full border border-black/[0.075] bg-white/92 shadow-[0_1px_2px_rgba(0,0,0,0.03)] text-xs transition-colors hover:border-black/[0.14] dark:border-white/[0.08] dark:bg-[#1e1f24]/90">
            <button
              type="button"
              data-testid="titlebar-aoi-pill"
              onClick={onOpenAoiModal}
              title={aoiSummary ? t("shell.titlebar.aoiBoundTooltip", { aoi: aoiSummary }) : t("shell.titlebar.aoiUnboundTooltip")}
              className="flex h-[23px] items-center gap-1.5 px-2.5 text-[#3f3f46] hover:text-[#18181b] cursor-pointer dark:text-[#a1a1aa] dark:hover:text-white"
            >
              <span
                className={cn(
                  "h-1.5 w-1.5 shrink-0 rounded-full",
                  aoiSummary ? "bg-[#10b981]" : "bg-[#a1a1aa]"
                )}
              />
              <MapPin className="h-3 w-3 shrink-0 text-[#71717a]" />
              <span className="max-w-[92px] lg:max-w-[140px] truncate font-medium">
                {aoiSummary ? aoiSummary : t("shell.titlebar.aoiUnbound")}
              </span>
            </button>
            {aoiSummary && onClearAoi && (
              <button
                type="button"
                onClick={(e) => {
                  e.stopPropagation();
                  onClearAoi();
                }}
                title={t("shell.titlebar.clearAoi")}
                aria-label={t("shell.titlebar.clearAoi")}
                className="flex h-[23px] items-center pr-2 pl-0.5 text-[#a1a1aa] hover:text-[#18181b] cursor-pointer dark:hover:text-white"
              >
                <X className="h-3 w-3" />
              </button>
            )}
          </div>
        )}

        {/* 2. Top-Right Download Concise Popover Trigger (简洁浮窗 + 进入下载中心入口) */}
        {showDownloadTrigger && (
          <div ref={downloadPopoverRef} className="relative">
            <button
              ref={downloadTriggerRef}
              type="button"
              data-testid="titlebar-download-trigger"
              aria-label={t("shell.titlebar.downloadTriggerAria")}
              aria-haspopup="true"
              aria-expanded={downloadPopoverOpen}
              onClick={() => {
                setWorkspaceMenuOpen(false);
                setPopoverOpen(false);
                setDownloadPopoverOpen((prev) => !prev);
              }}
              title={
                activeDownloadCount > 0
                  ? t("shell.titlebar.downloadTriggerActiveTooltip", { active: activeDownloadCount })
                  : t("shell.titlebar.downloadTriggerIdleTooltip", { total: downloadTasks.length })
              }
              className={cn(
                "flex h-[23px] items-center gap-1.5 rounded-full px-2.5 text-xs font-medium transition-colors cursor-pointer",
                activeDownloadCount > 0
                  ? "bg-[#ebf3ff] text-[#2563eb] hover:bg-[#dbeafe] dark:bg-[#172554]/50 dark:text-[#60a5fa]"
                  : "border border-black/[0.075] bg-white/92 shadow-[0_1px_2px_rgba(0,0,0,0.03)] text-[#52525b] hover:bg-white hover:text-[#18181b] dark:border-white/[0.08] dark:bg-[#1e1f24]/90 dark:text-[#a1a1aa] dark:hover:text-white"
              )}
            >
              <Download className={cn("h-3 w-3 shrink-0", activeDownloadCount > 0 && "animate-bounce")} />
              <span className="font-mono tabular-nums">
                {activeDownloadCount > 0 ? activeDownloadCount : downloadTasks.length}
              </span>
            </button>

            <SpringDropdownSurface
              open={downloadPopoverOpen}
              onPresentChange={setDlSurfacePresent}
              role="dialog"
              aria-label={t("shell.titlebar.downloadPopoverAria")}
              data-testid="titlebar-download-popover"
              data-no-drag="true"
              className="origin-top-right absolute right-0 top-full mt-1.5 z-50 w-80 rounded-[12px] border border-[#e4e4e7] bg-white shadow-xl outline-none select-none text-[#18181b] dark:border-[#27272a] dark:bg-[#1e1e22] dark:text-[#f4f4f5]"
            >
              {/* Popover Header */}
              <div
                data-spring-item=""
                className="flex items-center justify-between border-b border-[#ebebeb] px-3.5 py-2.5 dark:border-[#27272a]"
              >
                <div className="min-w-0">
                  <div className="flex items-center gap-2">
                    <h3 className="text-xs font-semibold">{t("shell.titlebar.downloadPopoverTitle")}</h3>
                    <span className="rounded-full bg-[#f4f4f5] px-1.5 py-0.2 font-mono text-xs text-[#52525b] dark:bg-[#27272a] dark:text-[#a1a1aa]">
                      {activeDownloadCount > 0
                        ? t("shell.titlebar.downloadPopoverActiveCount", { active: activeDownloadCount, total: downloadTasks.length })
                        : t("shell.titlebar.downloadPopoverTotalCount", { total: downloadTasks.length })}
                    </span>
                  </div>
                  <p className="text-xs text-[#71717a] mt-0.5 dark:text-[#a1a1aa]">
                    {t("shell.titlebar.downloadPopoverSubtitle")}
                  </p>
                </div>
                {onOpenDownloadDrawer && (
                  <button
                    type="button"
                    data-testid="titlebar-open-download-drawer-btn"
                    onClick={() => {
                      setDownloadPopoverOpen(false);
                      onOpenDownloadDrawer();
                    }}
                    className="shrink-0 rounded-[5px] border border-[#e4e4e7] bg-[#fafafa] px-2 py-1 text-xs font-medium text-[#3f3f46] hover:bg-[#f4f4f5] hover:text-[#18181b] cursor-pointer dark:border-[#27272a] dark:bg-[#27272a] dark:text-[#e4e7]"
                  >
                    {t("shell.titlebar.downloadPopoverSideDrawer")}
                  </button>
                )}
              </div>

              {/* Task List Preview with Stretchy Spring Highlight */}
              <div
                ref={dlHighlight.containerRef}
                onMouseLeave={dlHighlight.clearHighlight}
                className="relative max-h-64 overflow-y-auto"
              >
                <div
                  ref={dlHighlight.highlightRef}
                  data-testid="spring-dropdown-highlight"
                  aria-hidden="true"
                  className="sp-spring-dropdown-hl pointer-events-none absolute top-0 left-0 z-0 rounded-[8px] bg-[#f4f4f5] opacity-0 dark:bg-[#27272a]"
                />
                <div className="flex flex-col gap-0.5 p-1.5">
                  {previewTasks.length === 0 ? (
                    <div data-spring-item="">
                      <EmptyState kind="all-done" size="menu" title={t("shell.titlebar.downloadPopoverEmpty")} className="py-6" />
                    </div>
                  ) : (
                    previewTasks.map((task) => {
                      const meta = taskStatusMeta(task.status, t);
                      const doneBytes = task.done_bytes ?? 0;
                      const totalBytes =
                        typeof task.total_bytes === "number" && task.total_bytes > 0
                          ? task.total_bytes
                          : null;
                      const pct = totalBytes
                        ? Math.min(100, Math.round((doneBytes / totalBytes) * 100))
                        : task.status === "completed"
                        ? 100
                        : 0;

                      return (
                        <button
                          key={task.task_id}
                          type="button"
                          data-spring-item=""
                          data-testid="titlebar-download-popover-item"
                          onMouseEnter={(e) => dlHighlight.activateElement(e.currentTarget)}
                          onFocus={(e) => dlHighlight.activateElement(e.currentTarget)}
                          onClick={() => {
                            setDownloadPopoverOpen(false);
                            if (onOpenDownloadDrawer) {
                              onOpenDownloadDrawer(task.task_id);
                            } else {
                              onOpenDownloadCenter?.();
                            }
                          }}
                          className="relative z-10 flex w-full flex-col gap-1.5 rounded-[8px] px-2.5 py-2 text-left transition-colors cursor-pointer outline-none"
                        >
                          <div className="flex items-center justify-between gap-2">
                            <span className="min-w-0 flex-1 truncate text-xs font-medium text-[#18181b] dark:text-[#f4f4f5]">
                              {normalizeTaskDisplayName(task)}
                            </span>
                            <span
                              className={cn(
                                "inline-flex shrink-0 items-center gap-1 rounded-full px-1.5 py-0.5 text-xs font-medium",
                                meta.badgeClass
                              )}
                            >
                              <span className={cn("h-1.5 w-1.5 rounded-full", meta.dotClass)} />
                              <span>{meta.label}</span>
                            </span>
                          </div>

                          <div className="flex items-center gap-2">
                            <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-black/[0.08] dark:bg-white/[0.12]">
                              <div
                                className={cn(
                                  "h-full rounded-full transition-[width] duration-fast ease-sp",
                                  task.status === "failed"
                                    ? "bg-rose-500"
                                    : task.status === "completed"
                                    ? "bg-[#10b981]"
                                    : "bg-[#2563eb]"
                                )}
                                style={{ width: `${pct}%` }}
                              />
                            </div>
                            <span className="shrink-0 font-mono text-xs tabular-nums text-[#71717a] dark:text-[#a1a1aa]">
                              {totalBytes
                                ? `${formatBytes(doneBytes)} / ${formatBytes(totalBytes)}`
                                : doneBytes > 0
                                ? formatBytes(doneBytes)
                                : `${pct}%`}
                            </span>
                          </div>
                        </button>
                      );
                    })
                  )}
                </div>
              </div>

              {/* Popover Footer: Enter Download Center Page */}
              <div
                data-spring-item=""
                className="border-t border-[#ebebeb] p-2 dark:border-[#27272a]"
              >
                <button
                  type="button"
                  data-testid="titlebar-enter-download-center-btn"
                  onClick={() => {
                    setDownloadPopoverOpen(false);
                    if (onOpenDownloadCenter) {
                      onOpenDownloadCenter();
                    } else {
                      onNavigate?.("downloads");
                    }
                  }}
                  className="flex h-7 w-full items-center justify-center gap-1.5 rounded-[6px] bg-[#18181b] px-3 text-xs font-medium text-white transition-opacity hover:opacity-90 cursor-pointer dark:bg-[#f4f4f5] dark:text-[#18181b]"
                >
                  <span>{t("shell.titlebar.enterDownloadCenter")}</span>
                  <ArrowUpRight className="h-3.5 w-3.5" />
                </button>
              </div>
            </SpringDropdownSurface>
          </div>
        )}

        {/* 3. Right Map & Results Panel Toggle (when in Agent workbench) */}
        {panelToggle && (
          <button
            type="button"
            data-testid="titlebar-panel-toggle"
            onClick={panelToggle.onToggle}
            aria-pressed={panelToggle.open}
            title={panelToggle.open ? t("shell.titlebar.collapseMap") : t("shell.titlebar.expandMap")}
            aria-label={panelToggle.open ? t("shell.titlebar.collapseMap") : t("shell.titlebar.expandMap")}
            className={cn(
              "flex h-[24px] items-center gap-1.5 rounded-[7px] px-2 text-xs font-medium transition-colors cursor-pointer whitespace-nowrap shrink-0",
              panelToggle.open
                ? "bg-black/[0.072] text-[#18181b] dark:bg-white/[0.09] dark:text-white"
                : "text-[#52525b] hover:bg-black/[0.05] hover:text-[#18181b] dark:text-[#a1a1aa] dark:hover:bg-white/[0.06] dark:hover:text-white"
            )}
          >
            <PanelRight className="h-[13.5px] w-[13.5px] stroke-[1.75]" />
            <span className="hidden sm:inline">{t("shell.titlebar.mapPanel")}</span>
            {typeof panelToggle.sceneCount === "number" && panelToggle.sceneCount > 0 && (
              <span className="rounded-full bg-black/[0.08] px-1.5 py-0.2 font-mono text-xs text-[#18181b] dark:bg-[#3f3f46] dark:text-[#f4f4f5]">
                {panelToggle.sceneCount}
              </span>
            )}
          </button>
        )}

        {/* 4. Right-Side Workspace Switcher (Spring Dropdown with Stretchy Highlight, Slot Roll-Up & Draw Tick) */}
        <div ref={workspaceMenuRef} className="relative flex items-center">
          <button
            ref={workspaceTriggerRef}
            type="button"
            data-testid="titlebar-workspace-switcher"
            aria-haspopup="menu"
            aria-expanded={workspaceMenuOpen}
            onClick={() => {
              setDownloadPopoverOpen(false);
              setPopoverOpen(false);
              setWorkspaceMenuOpen((prev) => !prev);
            }}
            title={t("shell.titlebar.switchWorkspaceTooltip")}
            className="flex h-[24px] shrink-0 items-center gap-1.5 rounded-[7px] border border-black/[0.075] bg-white/92 shadow-[0_1px_2px_rgba(0,0,0,0.03)] px-2 text-[12px] font-medium text-[#18181b] transition-colors hover:bg-white cursor-pointer dark:border-white/[0.08] dark:bg-[#1e1f24]/90 dark:text-[#f4f4f5] dark:hover:bg-[#27272a]"
          >
            <SpringSlotTriggerValue
              routeKey={currentRouteKey}
              icon={CurrentRouteIcon}
              label={t(currentMeta.labelKey)}
            />
            <SpringChevron open={workspaceMenuOpen} className="h-3 w-3 text-[#71717a]" />
          </button>

          <SpringDropdownSurface
            open={workspaceMenuOpen}
            onPresentChange={setWsSurfacePresent}
            role="menu"
            aria-label={t("shell.titlebar.workspaceMenuAria")}
            data-testid="titlebar-workspace-menu"
            data-no-drag="true"
            className="origin-top-right absolute top-full right-0 z-50 mt-1.5 w-56 rounded-[12px] border border-[#e4e4e7] bg-white shadow-lg dark:border-[#27272a] dark:bg-[#1e1e22]"
            innerClassName="p-1"
          >
            <div
              ref={wsHighlight.containerRef}
              onMouseLeave={wsHighlight.clearHighlight}
              className="relative flex flex-col gap-0.5"
            >
              <div
                ref={wsHighlight.highlightRef}
                data-testid="spring-dropdown-highlight"
                aria-hidden="true"
                className="sp-spring-dropdown-hl pointer-events-none absolute top-0 left-0 z-0 rounded-[7px] bg-[#f4f4f5] opacity-0 dark:bg-[#27272a]"
              />

              <button
                type="button"
                role="menuitem"
                data-spring-item=""
                onMouseEnter={(e) => wsHighlight.activateElement(e.currentTarget)}
                onFocus={(e) => wsHighlight.activateElement(e.currentTarget)}
                onClick={() => {
                  onStartNewChat?.();
                  onNavigate?.("agent");
                  setWorkspaceMenuOpen(false);
                }}
                className="relative z-10 flex w-full items-center justify-between whitespace-nowrap rounded-[5px] px-2.5 py-1.5 text-left text-[12px] text-[#18181b] cursor-pointer outline-none dark:text-[#f4f4f5]"
              >
                <span className="flex items-center gap-2">
                  <Plus className="h-3.5 w-3.5 text-[#52525b] dark:text-[#a1a1aa]" />
                  <span>{t("shell.titlebar.newSession")}</span>
                </span>
                <span className="font-mono text-xs text-[#a1a1aa]">{modShortcut("N")}</span>
              </button>

              <div className="relative z-10 my-1 h-px bg-[#ebebeb] dark:bg-[#27272a]" />

              {(["agent", "workbench", "downloads"] as const).map((routeKey) => {
                const item = ROUTE_META[routeKey];
                const Icon = item.icon;
                const isCurrent = currentRouteKey === routeKey;
                return (
                  <button
                    key={routeKey}
                    type="button"
                    role="menuitem"
                    aria-current={isCurrent ? "page" : undefined}
                    data-spring-item=""
                    onMouseEnter={(e) => wsHighlight.activateElement(e.currentTarget)}
                    onFocus={(e) => wsHighlight.activateElement(e.currentTarget)}
                    onClick={() => {
                      onNavigate?.(routeKey);
                      setWorkspaceMenuOpen(false);
                    }}
                    className="relative z-10 flex w-full items-center justify-between whitespace-nowrap rounded-[5px] px-2.5 py-1.5 text-left text-[12px] text-[#18181b] cursor-pointer outline-none dark:text-[#f4f4f5]"
                  >
                    <span className="flex items-center gap-2">
                      <Icon className="h-3.5 w-3.5 text-[#52525b] dark:text-[#a1a1aa]" />
                      <span>{t(item.labelKey)}</span>
                    </span>
                    <span className="flex items-center gap-1.5">
                      <span className="font-mono text-xs text-[#a1a1aa]">{modShortcut(item.shortcutKey)}</span>
                      <SpringCheckmark visible={isCurrent} menuOpen={workspaceMenuOpen} />
                    </span>
                  </button>
                );
              })}

              <button
                type="button"
                role="menuitem"
                data-spring-item=""
                data-testid="titlebar-menu-data-catalog"
                onMouseEnter={(e) => wsHighlight.activateElement(e.currentTarget)}
                onFocus={(e) => wsHighlight.activateElement(e.currentTarget)}
                onClick={() => {
                  if (onOpenDataCatalog) {
                    onOpenDataCatalog();
                  } else {
                    onNavigate?.("workbench");
                  }
                  setWorkspaceMenuOpen(false);
                }}
                className="relative z-10 flex w-full items-center justify-between whitespace-nowrap rounded-[5px] px-2.5 py-1.5 text-left text-[12px] text-[#18181b] cursor-pointer outline-none dark:text-[#f4f4f5]"
              >
                <span className="flex items-center gap-2">
                  <Layers className="h-3.5 w-3.5 text-[#52525b] dark:text-[#a1a1aa]" />
                  <span>{t("shell.titlebar.dataCatalog")}</span>
                </span>
                <span className="font-mono text-xs text-[#a1a1aa]">{modShortcut("K")}</span>
              </button>

              {(["history"] as const).map((routeKey) => {
                const item = ROUTE_META[routeKey];
                const Icon = item.icon;
                const isCurrent = currentRouteKey === routeKey;
                return (
                  <button
                    key={routeKey}
                    type="button"
                    role="menuitem"
                    aria-current={isCurrent ? "page" : undefined}
                    data-spring-item=""
                    onMouseEnter={(e) => wsHighlight.activateElement(e.currentTarget)}
                    onFocus={(e) => wsHighlight.activateElement(e.currentTarget)}
                    onClick={() => {
                      onNavigate?.(routeKey);
                      setWorkspaceMenuOpen(false);
                    }}
                    className="relative z-10 flex w-full items-center justify-between whitespace-nowrap rounded-[5px] px-2.5 py-1.5 text-left text-[12px] text-[#18181b] cursor-pointer outline-none dark:text-[#f4f4f5]"
                  >
                    <span className="flex items-center gap-2">
                      <Icon className="h-3.5 w-3.5 text-[#52525b] dark:text-[#a1a1aa]" />
                      <span>{t(item.labelKey)}</span>
                    </span>
                    <span className="flex items-center gap-1.5">
                      <span className="font-mono text-xs text-[#a1a1aa]">{modShortcut(item.shortcutKey)}</span>
                      <SpringCheckmark visible={isCurrent} menuOpen={workspaceMenuOpen} />
                    </span>
                  </button>
                );
              })}

              <div className="relative z-10 my-1 h-px bg-[#ebebeb] dark:bg-[#27272a]" />

              <button
                type="button"
                role="menuitem"
                data-spring-item=""
                onMouseEnter={(e) => wsHighlight.activateElement(e.currentTarget)}
                onFocus={(e) => wsHighlight.activateElement(e.currentTarget)}
                onClick={() => {
                  onNavigate?.("settings");
                  setWorkspaceMenuOpen(false);
                }}
                className="relative z-10 flex w-full items-center justify-between whitespace-nowrap rounded-[5px] px-2.5 py-1.5 text-left text-[12px] text-[#18181b] cursor-pointer outline-none dark:text-[#f4f4f5]"
              >
                <span className="flex items-center gap-2">
                  <Settings className="h-3.5 w-3.5 text-[#52525b] dark:text-[#a1a1aa]" />
                  <span>{t("shell.titlebar.preferencesEllipsis")}</span>
                </span>
                <span className="font-mono text-xs text-[#a1a1aa]">{modShortcut(",")}</span>
              </button>
            </div>
          </SpringDropdownSurface>
        </div>

        {/* Optional legacy stopAll slot (only rendered when explicitly passed by isolated tests) */}
        {stopAll && (
          <button
            type="button"
            disabled={!stopAll.enabled || stopAll.busy}
            onClick={stopAll.onClick}
            aria-label={t("shell.titlebar.stopAll")}
            title={
              stopAll.busy
                ? t("shell.titlebar.stopping")
                : !stopAll.enabled
                ? stopAll.reason_zh || t("shell.titlebar.stopAllUnavailable")
                : typeof stopAll.live_runs === "number" && typeof stopAll.agent_active_tasks === "number"
                ? t("shell.titlebar.stopAllSummary", { runs: stopAll.live_runs, tasks: stopAll.agent_active_tasks })
                : t("shell.titlebar.stopAll")
            }
            className={cn(
              "flex h-[22px] items-center gap-1.5 rounded-full px-2.5 text-xs font-medium transition-colors select-none whitespace-nowrap shrink-0",
              stopAll.busy || !stopAll.enabled
                ? "text-sp-text-muted cursor-not-allowed dark:text-[#71717a]"
                : "bg-rose-500/10 text-rose-600 hover:bg-rose-500/20 cursor-pointer dark:text-rose-400"
            )}
          >
            <OctagonX className="h-3 w-3 shrink-0" />
            <span className="hidden min-[1280px]:inline">{t("shell.titlebar.stopAll")}</span>
          </button>
        )}

        {/* 4b. Notification Bell in Titlebar when sidebar is collapsed (or standalone titlebar) */}
        {!sidebarExpanded && (
          <div className="relative">
            <button
              ref={bellRef}
              type="button"
              onClick={() => {
                setWorkspaceMenuOpen(false);
                setDownloadPopoverOpen(false);
                setPopoverOpen(!popoverOpen);
                onOpenNotifications?.();
              }}
              aria-haspopup="true"
              aria-expanded={popoverOpen}
              aria-label={t("shell.titlebar.notifications")}
              title={t("shell.titlebar.notificationsHint")}
              className={cn(
                "relative flex h-[24px] w-[24px] items-center justify-center rounded-[5px] text-[#52525b] transition-colors hover:bg-[#f4f4f5] hover:text-[#18181b] cursor-pointer dark:text-[#a1a1aa] dark:hover:bg-[#27272a] dark:hover:text-white",
                popoverOpen && "bg-[#f4f4f5] text-[#18181b] dark:bg-[#27272a] dark:text-white"
              )}
            >
              <Bell className="h-3.5 w-3.5" />
              {unreadCount > 0 && (
                <span
                  className="absolute top-1 right-1 h-1.5 w-1.5 rounded-full bg-rose-500"
                  aria-hidden="true"
                />
              )}
            </button>

            <NotificationPopover
              open={popoverOpen}
              onPresentChange={setNotifSurfacePresent}
              onClose={() => {
                setPopoverOpen(false);
                bellRef.current?.focus();
              }}
              triggerRef={bellRef}
              placement="bottom-end"
              emptyIcon="bell"
              unreadCount={unreadCount}
              notifications={notifications}
              unreadIds={unreadIds}
              onNotificationClick={onNotificationClick}
              onMarkAllRead={onMarkAllRead}
            />
          </div>
        )}

        {/* 5. Windows Native Caption Controls (hidden on macOS in favor of left traffic lights) */}
        {!isMac && (
          <div
            className="ml-1 flex h-full items-center"
            style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
            aria-label={t("shell.titlebar.windowControls")}
          >
            <button
              type="button"
              onClick={handleMinimize}
              title={t("shell.titlebar.minimize")}
              aria-label={t("shell.titlebar.minimize")}
              className="flex h-full w-[42px] items-center justify-center text-[#52525b] transition-colors hover:bg-[#f4f4f5] cursor-pointer dark:text-[#a1a1aa] dark:hover:bg-[#27272a]"
            >
              <Minus className="h-3 w-3" strokeWidth={1.4} />
            </button>
            <button
              type="button"
              onClick={handleMaximize}
              title={t("shell.titlebar.maximizeRestore")}
              aria-label={t("shell.titlebar.maximizeRestore")}
              className="flex h-full w-[42px] items-center justify-center text-[#52525b] transition-colors hover:bg-[#f4f4f5] cursor-pointer dark:text-[#a1a1aa] dark:hover:bg-[#27272a]"
            >
              <Square className="h-3 w-3" strokeWidth={1.4} />
            </button>
            <button
              type="button"
              onClick={handleClose}
              title={t("shell.titlebar.closeWindow")}
              aria-label={t("shell.titlebar.closeWindow")}
              className="flex h-full w-[42px] items-center justify-center text-[#52525b] transition-colors hover:bg-[#e81123] hover:text-white cursor-pointer dark:text-[#a1a1aa]"
            >
              <X className="h-3.5 w-3.5" strokeWidth={1.4} />
            </button>
          </div>
        )}
      </div>
    </header>
  );
}
