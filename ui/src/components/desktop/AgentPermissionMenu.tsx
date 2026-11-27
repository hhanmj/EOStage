/**
 * AgentPermissionMenu.tsx
 * Agent 输入框权限模式选择器组件（受控组件，无桥接直接依赖）
 * 规范：W5-2f 简报 2f-1b、W5-2g 简报 2g-1（三档，无额度表单）、A1 §3.1 与 §3.1.1
 */

import React, { useState, useRef, useEffect, useCallback } from "react";
import { SlidersHorizontal, ChevronDown, Check } from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";
import type {
  AgentPermissionMode,
  AgentPermissionModeView,
} from "../../lib/generated/v3-contracts";
import type { SimpleStorage } from "../../shell/api/shellRoutes";
import {
  PERMISSION_MODE_ROWS,
  permissionModeLabelZh,
  permissionScopeBadge,
  permissionModeWire,
  readFullAccessRiskAck,
  writeFullAccessRiskAck,
  needsFullAccessRiskPrompt,
  popoverPlacement,
  DESKTOP_TITLEBAR_HEIGHT,
} from "../../shell/api/agentPermissionLogic";
import { explainActionError } from "../../shell/api/errorExplanations";
import { FullAccessRiskDialog } from "./FullAccessRiskDialog";

export interface AgentPermissionMenuProps {
  view: AgentPermissionModeView | null;
  status: "loading" | "ready" | "error";
  onSelect: (mode: AgentPermissionMode) => Promise<void> | void;
  onSetGlobal?: (mode: AgentPermissionMode) => Promise<void> | void;
  onClearSession?: () => Promise<void> | void;
  pendingApply?: boolean;
  compact?: boolean;
  riskStorage?: SimpleStorage;
}

const defaultStorage: SimpleStorage = {
  getItem: (k: string) => {
    try {
      return typeof window !== "undefined" ? window.localStorage.getItem(k) : null;
    } catch {
      return null;
    }
  },
  setItem: (k: string, v: string) => {
    try {
      if (typeof window !== "undefined") window.localStorage.setItem(k, v);
    } catch {
      // 忽略
    }
  },
};

export function AgentPermissionMenu({
  view,
  status,
  onSelect,
  // onSetGlobal 与 onClearSession 保持兼容保留在 props 中，已弃用在菜单内部触发
  onSetGlobal: _onSetGlobal,
  onClearSession: _onClearSession,
  pendingApply = false,
  compact = false,
  riskStorage = defaultStorage,
}: AgentPermissionMenuProps) {
  const { t } = useI18n();
  const [menuOpen, setMenuOpen] = useState(false);
  const [riskDialogOpen, setRiskDialogOpen] = useState(false);
  const [pendingAction, setPendingAction] = useState<"select_full_access" | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);

  const containerRef = useRef<HTMLDivElement | null>(null);
  const triggerBtnRef = useRef<HTMLButtonElement | null>(null);
  const menuContainerRef = useRef<HTMLDivElement | null>(null);

  // 弹层方位与最大高度自适应
  const [placement, setPlacement] = useState<{ side: "up" | "down"; maxHeight: number }>({
    side: compact ? "up" : "down",
    maxHeight: 400,
  });
  const [permHighlight, setPermHighlight] = useState<{
    mode: string | null;
    y: number;
    h: number;
  }>({
    mode: null,
    y: 0,
    h: 44,
  });

  const updatePermHighlight = (el: HTMLElement, mode: string, index: number) => {
    const rawTop = el.offsetTop;
    const rawHeight = el.offsetHeight;
    setPermHighlight({
      mode,
      y: rawHeight > 0 ? rawTop : index * 46,
      h: rawHeight > 0 ? rawHeight : 44,
    });
  };

  const updatePlacement = useCallback(() => {
    if (!triggerBtnRef.current) return;
    const rect = triggerBtnRef.current.getBoundingClientRect();
    // 估算内容高度：标题 + 三项卡片
    const desiredHeight = 220;
    const vh = typeof window !== "undefined" ? window.innerHeight : 800;
    const res = popoverPlacement({
      triggerTop: rect.top,
      triggerBottom: rect.bottom,
      viewportHeight: vh,
      desiredHeight,
      topInset: DESKTOP_TITLEBAR_HEIGHT,
      margin: 8,
    });
    setPlacement(res);
  }, []);

  useEffect(() => {
    if (!menuOpen) return;
    updatePlacement();
    const handleResize = () => {
      updatePlacement();
    };
    window.addEventListener("resize", handleResize);
    return () => window.removeEventListener("resize", handleResize);
  }, [menuOpen, updatePlacement]);

  // 点击外部关闭弹层
  useEffect(() => {
    if (!menuOpen) return;
    const handleOutsideClick = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setMenuOpen(false);
        setActionError(null);
      }
    };
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !riskDialogOpen) {
        setMenuOpen(false);
        setActionError(null);
        triggerBtnRef.current?.focus();
      }
    };
    document.addEventListener("mousedown", handleOutsideClick);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("mousedown", handleOutsideClick);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [menuOpen, riskDialogOpen]);

  const handleToggleMenu = () => {
    if (status !== "ready" || !view) return;
    if (!menuOpen) {
      setActionError(null);
    }
    setMenuOpen((prev) => !prev);
  };

  const handleSelectMode = async (mode: AgentPermissionMode) => {
    try {
      setActionError(null);
      const wire = permissionModeWire(mode);
      if (wire) {
        await onSelect(wire);
        setMenuOpen(false);
      }
    } catch (err) {
      setActionError(explainActionError(err, t("agent.permission.saveFailed")));
    }
  };

  const handleSelectFullAccess = async () => {
    setActionError(null);
    const acked = readFullAccessRiskAck(riskStorage);
    if (needsFullAccessRiskPrompt("full_access", acked)) {
      setPendingAction("select_full_access");
      setMenuOpen(false);
      setRiskDialogOpen(true);
      return;
    }
    try {
      const wire = permissionModeWire("full_access");
      if (wire) {
        await onSelect(wire);
        setMenuOpen(false);
      }
    } catch (err) {
      setActionError(explainActionError(err, t("agent.permission.saveFailed")));
    }
  };

  const handleConfirmRisk = async () => {
    writeFullAccessRiskAck(riskStorage);
    setRiskDialogOpen(false);
    try {
      if (pendingAction === "select_full_access") {
        const wire = permissionModeWire("full_access");
        if (wire) {
          await onSelect(wire);
          setMenuOpen(false);
        }
      }
    } catch (err) {
      setActionError(explainActionError(err, t("agent.permission.saveFailed")));
      setMenuOpen(true);
    } finally {
      setPendingAction(null);
    }
  };

  const handleCancelRisk = () => {
    setRiskDialogOpen(false);
    setPendingAction(null);
  };

  // 按钮显示名与提示
  const isReady = status === "ready" && view !== null;
  const currentMode: AgentPermissionMode | null = view?.effective ?? null;
  const labelZh = isReady ? permissionModeLabelZh(currentMode) || t("agent.permission.label") : t("agent.permission.label");
  // 只在「发送时才生效」这一种情况下给出小标记；不再区分全局默认与会话设置
  const badgeZh =
    isReady && view && pendingApply
      ? permissionScopeBadge(view, view.effective)
      : "";
  const btnTitle =
    status === "loading"
      ? t("agent.permission.loading")
      : status === "error"
      ? t("agent.permission.loadFailed")
      : t("agent.permission.currentTitle", { label: labelZh, badge: badgeZh ? `（${badgeZh}）` : "" });

  return (
    <div ref={containerRef} className="relative inline-block shrink-0">
      {/* 触发按钮 (EOStage Desktop `SlidersHorizontal + Agent` 极简样式 + 弹簧箭头翻转) */}
      <button
        ref={triggerBtnRef}
        type="button"
        disabled={!isReady}
        onClick={handleToggleMenu}
        title={btnTitle}
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        aria-label={t("agent.permission.label")}
        className={cn(
          "cds-btn-squish flex h-[26px] items-center gap-1.5 px-2 rounded-[6px] text-[12.5px] font-normal transition-colors whitespace-nowrap",
          !isReady
            ? "text-[#a1a1aa] cursor-not-allowed opacity-60"
            : menuOpen
            ? "bg-[#f4f4f5] text-[#18181b] dark:bg-[#27272a] dark:text-white cursor-pointer"
            : "text-[#71717a] hover:bg-[#f4f4f5] hover:text-[#18181b] dark:text-[#a1a1aa] dark:hover:bg-[#27272a] dark:hover:text-white cursor-pointer"
        )}
      >
        <SlidersHorizontal className="h-[13.5px] w-[13.5px] shrink-0 stroke-[1.75]" />
        <span className="whitespace-nowrap">{labelZh}</span>
        {badgeZh && (
          <span className="text-xs text-[#a1a1aa] font-normal whitespace-nowrap">
            {badgeZh}
          </span>
        )}
        <ChevronDown
          className={cn(
            "h-3 w-3 text-[#a1a1aa] shrink-0 sp-permission-chevron",
            menuOpen && "is-open"
          )}
        />
      </button>

      {/* 权限模式弹层 */}
      {menuOpen && view && (
        <div
          ref={menuContainerRef}
          role="menu"
          aria-label={t("agent.permission.label")}
          data-placement={placement.side}
          onMouseLeave={() => setPermHighlight((prev) => ({ ...prev, mode: null }))}
          style={{
            maxHeight: placement.maxHeight > 0 ? `${placement.maxHeight}px` : undefined,
          }}
          className={cn(
            "sp-popover-blur-in absolute z-30 w-[320px] rounded-card bg-sp-surface dark:bg-sp-card p-1.5 shadow-overlay border border-sp-line flex flex-col overflow-hidden",
            placement.side === "up"
              ? "bottom-[calc(100%+8px)] left-0 origin-bottom-left"
              : "top-[calc(100%+8px)] left-0 origin-top-left"
          )}
        >
          <div className="shrink-0 px-2 py-1">
            <div className="text-xs font-semibold text-sp-text-muted tracking-wide select-none">{t("agent.permission.label")}</div>
            {actionError && (
              <div className="mt-1 rounded-sp-sm bg-sp-danger-bg px-2 py-1 text-xs text-sp-danger-text">
                {actionError}
              </div>
            )}
          </div>

          <div className="relative min-h-0 flex-1 overflow-y-auto flex flex-col gap-0.5">
            <div
              aria-hidden="true"
              data-testid="permission-menu-fluid-highlight"
              data-active-mode={permHighlight.mode ?? ""}
              className={cn(
                "sp-fluid-menu-highlight",
                permHighlight.mode !== null && "is-active"
              )}
              style={
                {
                  "--sp-highlight-y": `${permHighlight.y}px`,
                  "--sp-highlight-h": `${permHighlight.h}px`,
                } as React.CSSProperties
              }
            />
            {PERMISSION_MODE_ROWS.map((row, idx) => {
              const checked = currentMode === row.mode;
              const onActivate = () => {
                if (row.mode === "full_access") void handleSelectFullAccess();
                else void handleSelectMode(row.mode);
              };
              return (
                <div
                  key={row.mode}
                  role="menuitemradio"
                  data-testid={`permission-menu-item-${row.mode}`}
                  aria-checked={checked}
                  tabIndex={0}
                  onMouseEnter={(e) => updatePermHighlight(e.currentTarget, row.mode, idx)}
                  onFocus={(e) => updatePermHighlight(e.currentTarget, row.mode, idx)}
                  onClick={onActivate}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      onActivate();
                    }
                  }}
                  className={cn(
                    "sp-fluid-menu-item flex flex-col justify-center min-h-[44px] py-1.5 px-2.5 rounded-sp-sm cursor-pointer transition-colors duration-fast ease-sp outline-none focus-visible:ring-1 focus-visible:ring-sp-accent",
                    checked && "bg-sp-selected font-medium"
                  )}
                >
                  <div className="flex items-center justify-between">
                    <span
                      data-mode-label
                      className="text-xs font-medium text-sp-text leading-tight whitespace-nowrap"
                    >
                      {t(row.labelKey)}
                    </span>
                    {checked && <Check className="h-3.5 w-3.5 text-sp-accent shrink-0" />}
                  </div>
                  <span className="text-xs text-sp-text-muted leading-snug whitespace-normal mt-1">
                    {row.mode === "auto_approve"
                      ? `${t(row.descriptionKey)}${t("agent.permission.autoApproveRuleNote")}`
                      : t(row.descriptionKey)}
                  </span>
                </div>
              );
            })}
          </div>
        </div>
      )}

      {/* 风险确认对话框 */}
      <FullAccessRiskDialog
        open={riskDialogOpen}
        onConfirm={handleConfirmRisk}
        onCancel={handleCancelRisk}
        triggerRef={triggerBtnRef}
      />
    </div>
  );
}
