import React, { useEffect, useMemo, useRef, useState } from "react";
import { Check, ListFilter, MoreHorizontal, Pin, Tag, Trash2, X } from "lucide-react";
import { EmptyState } from "@/components/ui/EmptyState";
import { EmptyStateIcon } from "@/components/ui/EmptyStateVisuals";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";
import {
  formatNotificationTime,
  getNotificationKindMeta,
  type NotificationItem,
} from "@/shell/api/notificationBell";
import {
  RELEASE_CHANGELOG_ENTRIES,
  hasUnreadReleaseNotice,
  markReleaseChangelogSeen,
} from "@/shell/api/releaseChangelog";
import {
  SpringDropdownSurface,
  useStretchyMenuHighlight,
} from "./SpringDropdownPrimitives";

export type NotificationFilterTab = "all" | "unread" | "changelog";
export type NotificationKindFilter = "all" | "completed" | "attention";

export interface NotificationPopoverProps {
  open: boolean;
  onClose: () => void;
  onPresentChange?: (present: boolean) => void;
  triggerRef?: React.RefObject<HTMLElement | null>;
  placement?: "bottom-start" | "bottom-end";
  variant?: "popover" | "side-drawer";
  emptyIcon?: "inbox" | "bell";
  unreadCount?: number;
  notifications?: NotificationItem[];
  unreadIds?: string[];
  onNotificationClick?: (item: NotificationItem) => void;
  onMarkAllRead?: () => void;
  onClearRead?: () => void;
  onPinnedChange?: (pinned: boolean) => void;
  initialTab?: NotificationFilterTab;
}

export function NotificationPopover({
  open,
  onClose,
  onPresentChange,
  triggerRef,
  placement = "bottom-end",
  variant = "popover",
  emptyIcon = "bell",
  unreadCount = 0,
  notifications = [],
  unreadIds = [],
  onNotificationClick,
  onMarkAllRead,
  onClearRead,
  onPinnedChange,
  initialTab,
}: NotificationPopoverProps) {
  const { t } = useI18n();
  const panelRef = useRef<HTMLDivElement | null>(null);
  const [tab, setTab] = useState<NotificationFilterTab>(initialTab ?? "all");
  const [kindFilter, setKindFilter] = useState<NotificationKindFilter>("all");
  const [pinned, setPinned] = useState(false);
  const [moreOpen, setMoreOpen] = useState(false);
  const [filterOpen, setFilterOpen] = useState(false);
  const [changelogUnread, setChangelogUnread] = useState<boolean>(() =>
    hasUnreadReleaseNotice()
  );
  const notifHighlight = useStretchyMenuHighlight({ x: 0, y: 0, variant: "card" });

  const isSideDrawer = variant === "side-drawer";

  useEffect(() => {
    onPinnedChange?.(pinned);
  }, [pinned, onPinnedChange]);

  useEffect(() => {
    if (open && initialTab) {
      setTab(initialTab);
    }
    if (open && tab === "changelog") {
      markReleaseChangelogSeen();
      setChangelogUnread(false);
    } else if (open) {
      setChangelogUnread(hasUnreadReleaseNotice());
    }
  }, [open, initialTab, tab]);

  useEffect(() => {
    if (!open) {
      setMoreOpen(false);
      setFilterOpen(false);
      notifHighlight.resetHighlight();
    }
  }, [open, notifHighlight.resetHighlight]);

  useEffect(() => {
    if (isSideDrawer) {
      onPresentChange?.(open);
    }
  }, [isSideDrawer, open, onPresentChange]);

  useEffect(() => {
    if (!open || typeof document === "undefined") return;

    const handlePointerDown = (e: MouseEvent) => {
      const target = e.target as Node | null;
      if (!target) return;
      if (panelRef.current?.contains(target)) return;
      if (triggerRef?.current?.contains(target)) return;
      if (isSideDrawer && pinned) return;
      onClose();
    };

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        if (moreOpen || filterOpen) {
          setMoreOpen(false);
          setFilterOpen(false);
          return;
        }
        if (isSideDrawer && pinned) return;
        e.preventDefault();
        onClose();
        triggerRef?.current?.focus();
      }
    };

    document.addEventListener("mousedown", handlePointerDown);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("mousedown", handlePointerDown);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [open, onClose, triggerRef, isSideDrawer, pinned, moreOpen, filterOpen]);

  const unreadSet = useMemo(() => new Set(unreadIds), [unreadIds]);

  const filteredItems = useMemo(() => {
    return notifications.filter((item) => {
      if (tab === "unread" && !unreadSet.has(item.id)) return false;
      if (kindFilter === "completed" && item.kind !== "completed") return false;
      if (kindFilter === "attention" && item.kind === "completed") return false;
      return true;
    });
  }, [notifications, tab, kindFilter, unreadSet]);

  if (isSideDrawer && !open) return null;

  const iconBtnCls =
    "flex h-6 w-6 items-center justify-center rounded-[6px] text-[#71717a] transition-colors hover:bg-[#f4f4f5] hover:text-[#18181b] cursor-pointer dark:text-[#a1a1aa] dark:hover:bg-[#27272a] dark:hover:text-white";

  const bodyContent = (
    <>
      {/* EOStage Header: Title on left, `•••` / Filter / Pin / Close on right */}
      <div
        data-spring-item=""
        className="flex items-center justify-between px-4 pt-3 pb-2"
      >
        <span className="text-[13px] font-semibold tracking-[-0.01em]">
          {t("shell.titlebar.notifications")}
        </span>

        <div className="relative flex items-center gap-0.5">
          <button
            type="button"
            data-testid="notification-more-btn"
            aria-label="更多通知操作"
            title="更多通知操作"
            onClick={() => {
              setFilterOpen(false);
              setMoreOpen((prev) => !prev);
            }}
            className={cn(iconBtnCls, moreOpen && "bg-[#f4f4f5] text-[#18181b] dark:bg-[#27272a] dark:text-white")}
          >
            <MoreHorizontal className="h-[15px] w-[15px] stroke-[1.75]" />
          </button>

          <button
            type="button"
            data-testid="notification-filter-btn"
            aria-label="筛选通知类型"
            title="筛选通知类型"
            onClick={() => {
              setMoreOpen(false);
              setFilterOpen((prev) => !prev);
            }}
            className={cn(
              iconBtnCls,
              (filterOpen || kindFilter !== "all") &&
                "bg-[#f4f4f5] text-[#18181b] dark:bg-[#27272a] dark:text-white"
            )}
          >
            <ListFilter className="h-[14px] w-[14px] stroke-[1.75]" />
          </button>

          {isSideDrawer && (
            <button
              type="button"
              data-testid="notification-pin-btn"
              aria-pressed={pinned}
              aria-label={pinned ? "取消固定通知栏" : "固定通知栏"}
              title={pinned ? "取消固定通知栏" : "固定通知栏"}
              onClick={() => setPinned((prev) => !prev)}
              className={cn(
                iconBtnCls,
                pinned && "bg-[#ebebeb] text-[#18181b] dark:bg-[#27272a] dark:text-white"
              )}
            >
              <Pin className="h-[14px] w-[14px] stroke-[1.75]" />
            </button>
          )}

          <button
            type="button"
            data-testid="notification-close-btn"
            aria-label="关闭通知"
            title="关闭"
            onClick={() => {
              setPinned(false);
              onClose();
            }}
            className={iconBtnCls}
          >
            <X className="h-[15px] w-[15px] stroke-[1.75]" />
          </button>

          {moreOpen && (
            <div
              data-testid="notification-more-menu"
              className="absolute top-full right-0 z-50 mt-1 w-44 rounded-[8px] border border-[#e4e4e7] bg-white p-1 shadow-lg dark:border-[#27272a] dark:bg-[#202024]"
            >
              <button
                type="button"
                data-testid="notification-mark-all-read"
                disabled={unreadCount === 0 || !onMarkAllRead}
                onClick={() => {
                  setMoreOpen(false);
                  onMarkAllRead?.();
                }}
                className="flex w-full items-center justify-between rounded-[5px] px-2.5 py-1.5 text-left text-xs text-[#18181b] hover:bg-[#f4f4f5] disabled:cursor-not-allowed disabled:opacity-45 cursor-pointer dark:text-[#f4f4f5] dark:hover:bg-[#27272a]"
              >
                <span>{t("shell.titlebar.markAllRead")}</span>
                <Check className="h-3.5 w-3.5 text-[#71717a]" />
              </button>
              <button
                type="button"
                data-testid="notification-clear-read"
                disabled={notifications.length === unreadCount || !onClearRead}
                onClick={() => {
                  setMoreOpen(false);
                  onClearRead?.();
                }}
                className="flex w-full items-center justify-between rounded-[5px] px-2.5 py-1.5 text-left text-xs text-[#18181b] hover:bg-[#f4f4f5] disabled:cursor-not-allowed disabled:opacity-45 cursor-pointer dark:text-[#f4f4f5] dark:hover:bg-[#27272a]"
              >
                <span>清空已读通知</span>
                <Trash2 className="h-3.5 w-3.5 text-[#71717a]" />
              </button>
            </div>
          )}

          {filterOpen && (
            <div
              data-testid="notification-filter-menu"
              className="absolute top-full right-0 z-50 mt-1 w-40 rounded-[8px] border border-[#e4e4e7] bg-white p-1 shadow-lg dark:border-[#27272a] dark:bg-[#202024]"
            >
              {(
                [
                  { id: "all", label: "全部类型" },
                  { id: "completed", label: "仅已完成" },
                  { id: "attention", label: "异常与待确认" },
                ] as const
              ).map((opt) => (
                <button
                  key={opt.id}
                  type="button"
                  data-testid={`notification-filter-option-${opt.id}`}
                  onClick={() => {
                    setKindFilter(opt.id);
                    setFilterOpen(false);
                  }}
                  className="flex w-full items-center justify-between rounded-[5px] px-2.5 py-1.5 text-left text-xs text-[#18181b] hover:bg-[#f4f4f5] cursor-pointer dark:text-[#f4f4f5] dark:hover:bg-[#27272a]"
                >
                  <span>{opt.label}</span>
                  {kindFilter === opt.id && <Check className="h-3.5 w-3.5 text-[#18181b] dark:text-white" />}
                </button>
              ))}
            </div>
          )}
        </div>
      </div>

      {/* EOStage Filter Pills: `全部` / `未读 0` / `更新日志` */}
      <div
        data-spring-item=""
        className="flex items-center justify-between gap-2 px-4 pb-2.5"
      >
        <div className="flex items-center gap-1" role="tablist" aria-label="通知筛选">
          <button
            type="button"
            role="tab"
            data-testid="notification-tab-all"
            aria-selected={tab === "all"}
            onClick={() => setTab("all")}
            className={cn(
              "rounded-full px-3 py-1 text-xs font-medium transition-colors cursor-pointer",
              tab === "all"
                ? "bg-[#ebebeb] text-[#18181b] dark:bg-[#27272a] dark:text-[#f4f4f5]"
                : "text-[#71717a] hover:bg-[#f4f4f5] hover:text-[#18181b] dark:text-[#a1a1aa] dark:hover:bg-[#222226] dark:hover:text-white"
            )}
          >
            全部
          </button>
          <button
            type="button"
            role="tab"
            data-testid="notification-tab-unread"
            aria-selected={tab === "unread"}
            onClick={() => setTab("unread")}
            className={cn(
              "inline-flex items-center gap-1.5 rounded-full px-3 py-1 text-xs font-medium tabular-nums transition-colors cursor-pointer",
              tab === "unread"
                ? "bg-[#ebebeb] text-[#18181b] dark:bg-[#27272a] dark:text-[#f4f4f5]"
                : "text-[#71717a] hover:bg-[#f4f4f5] hover:text-[#18181b] dark:text-[#a1a1aa] dark:hover:bg-[#222226] dark:hover:text-white"
            )}
          >
            <span>未读</span>
            <span className="text-[#a1a1aa] dark:text-[#71717a]">{unreadCount}</span>
          </button>
          <button
            type="button"
            role="tab"
            data-testid="notification-tab-changelog"
            aria-selected={tab === "changelog"}
            onClick={() => {
              setTab("changelog");
              markReleaseChangelogSeen();
              setChangelogUnread(false);
            }}
            className={cn(
              "inline-flex items-center gap-1.5 rounded-full px-3 py-1 text-xs font-medium transition-colors cursor-pointer",
              tab === "changelog"
                ? "bg-[#ebebeb] text-[#18181b] dark:bg-[#27272a] dark:text-[#f4f4f5]"
                : "text-[#71717a] hover:bg-[#f4f4f5] hover:text-[#18181b] dark:text-[#a1a1aa] dark:hover:bg-[#222226] dark:hover:text-white"
            )}
          >
            <span>更新日志</span>
            {changelogUnread && (
              <span
                data-testid="notification-changelog-unread-dot"
                className="h-1.5 w-1.5 rounded-full bg-sp-accent shrink-0"
                aria-label="未读版本更新"
              />
            )}
          </button>
        </div>
      </div>

      {/* Body: Changelog OR Stretchy Highlight Layer + Notification List */}
      <div
        ref={notifHighlight.containerRef}
        onMouseLeave={notifHighlight.clearHighlight}
        className={cn(
          "relative overflow-y-auto",
          isSideDrawer ? "flex flex-1 flex-col" : "max-h-72"
        )}
      >
        <div
          ref={notifHighlight.highlightRef}
          data-testid="spring-dropdown-highlight"
          aria-hidden="true"
          className="sp-spring-dropdown-hl pointer-events-none absolute top-0 left-0 z-0 rounded-[8px] bg-sp-hover opacity-0"
        />

        {tab === "changelog" ? (
          <div data-testid="notification-changelog-list" className="flex flex-col gap-3 p-3 select-text">
            {RELEASE_CHANGELOG_ENTRIES.map((entry, idx) => (
              <div
                key={entry.version}
                className="rounded-[10px] border border-[#e4e4e7] bg-[#fafafa] p-3 text-xs dark:border-[#27272a] dark:bg-[#222226]"
              >
                <div className="flex items-center justify-between gap-2">
                  <div className="inline-flex items-center gap-1.5 font-semibold text-[#18181b] dark:text-[#f4f4f5]">
                    <Tag className="h-3.5 w-3.5 text-sp-text-muted shrink-0" />
                    <span>v{entry.version}</span>
                    {idx === 0 && (
                      <span className="rounded-full border border-sp-line bg-sp-surface px-1.5 py-0.5 text-xs font-medium text-sp-text">
                        最新
                      </span>
                    )}
                  </div>
                  <span className="font-mono text-xs text-[#71717a] dark:text-[#a1a1aa] tabular-nums">
                    {entry.date}
                  </span>
                </div>
                <div className="mt-1 font-medium text-[#18181b] dark:text-[#f4f4f5]">
                  {entry.title}
                </div>
                <ul className="mt-1.5 space-y-1 text-xs leading-relaxed text-[#52525b] dark:text-[#a1a1aa]">
                  {entry.highlights.map((line, i) => (
                    <li key={i} className="flex items-start gap-1.5">
                      <span className="mt-1.5 h-1 w-1 rounded-full bg-sp-text-muted shrink-0" />
                      <span>{line}</span>
                    </li>
                  ))}
                </ul>
              </div>
            ))}
          </div>
        ) : filteredItems.length === 0 ? (
          <div
            data-spring-item=""
            className={cn(
              "flex flex-col items-center justify-center",
              isSideDrawer ? "flex-1 pb-12" : "py-4"
            )}
          >
            <EmptyState
              kind="all-done"
              size="page"
              visual={<EmptyStateIcon name="bell" size={48} neutral />}
              title={
                tab === "unread" && notifications.length > 0
                  ? "已读完全部通知"
                  : t("shell.titlebar.noNotifications")
              }
              description={t("shell.titlebar.notificationsDescription")}
            />
          </div>
        ) : (
          <div className="flex flex-col gap-0.5 p-1.5">
            {filteredItems.map((item) => {
              const isUnread = unreadSet.has(item.id);
              const meta = getNotificationKindMeta(item.kind);
              return (
                <button
                  key={item.id}
                  type="button"
                  data-spring-item=""
                  data-testid="titlebar-notification-item"
                  onMouseEnter={(e) => notifHighlight.activateElement(e.currentTarget)}
                  onFocus={(e) => notifHighlight.activateElement(e.currentTarget)}
                  onClick={() => {
                    if (!pinned) onClose();
                    onNotificationClick?.(item);
                  }}
                  className="relative z-10 flex w-full items-start gap-2.5 rounded-[8px] px-2.5 py-2.5 text-left text-xs transition-colors cursor-pointer outline-none"
                >
                  <span
                    className={cn(
                      "mt-0.5 shrink-0 rounded-full px-1.5 py-0.5 text-xs font-semibold",
                      meta.bgClass,
                      meta.textClass
                    )}
                  >
                    {meta.label}
                  </span>
                  <div className="min-w-0 flex-1">
                    <div className={cn("truncate text-sp-text", isUnread ? "font-semibold" : "font-medium opacity-85")}>
                      {item.title}
                    </div>
                    {Array.isArray(item.detail) && item.detail.length > 0 && (
                      <div className="mt-1 space-y-0.5 text-xs text-sp-text-muted break-words">
                        {item.detail.map((d, idx) => (
                          <p key={idx} className="leading-snug">
                            {d}
                          </p>
                        ))}
                      </div>
                    )}
                    <div className="mt-0.5 font-mono text-xs tabular-nums text-sp-text-muted">
                      {formatNotificationTime(item.occurred_at)}
                    </div>
                  </div>
                  {isUnread && (
                    <span
                      className="mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full bg-sp-accent"
                      aria-label={t("shell.titlebar.unread")}
                    />
                  )}
                </button>
              );
            })}
          </div>
        )}
      </div>
    </>
  );

  if (isSideDrawer) {
    return (
      <div
        ref={panelRef}
        tabIndex={-1}
        role="dialog"
        data-testid="notification-side-drawer"
        aria-label={t("shell.titlebar.notifications")}
        className="z-50 flex flex-col bg-white text-[#18181b] outline-none select-none dark:bg-[#18181b] dark:text-[#f4f4f5] absolute top-0 bottom-0 left-full w-[336px] border-r border-[#ebebeb] shadow-[8px_0_24px_-12px_rgba(0,0,0,0.12)] dark:border-[#27272a] dark:shadow-[8px_0_28px_-12px_rgba(0,0,0,0.55)]"
      >
        {bodyContent}
      </div>
    );
  }

  return (
    <SpringDropdownSurface
      open={open}
      onPresentChange={onPresentChange}
      panelRef={panelRef}
      tabIndex={-1}
      role="dialog"
      aria-label={t("shell.titlebar.notifications")}
      data-testid="titlebar-notification-popover"
      data-no-drag="true"
      className={cn(
        "absolute top-full mt-1.5 z-50 w-[340px] rounded-[12px] border border-sp-line bg-sp-surface shadow-sp-overlay outline-none select-none text-sp-text dark:bg-sp-card",
        placement === "bottom-start" ? "origin-top-left left-0" : "origin-top-right right-0"
      )}
    >
      {bodyContent}
    </SpringDropdownSurface>
  );
}
