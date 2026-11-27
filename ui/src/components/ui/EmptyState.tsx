import React from "react";
import { RefreshCw } from "lucide-react";
import { cn } from "@/lib/utils";

export type EmptyStateKind =
  | "first-use"
  | "no-results"
  | "all-done"
  | "failed"
  | "no-access";

export type EmptyStateSize = "page" | "card" | "row" | "menu";

export interface EmptyStateAction {
  label: string;
  onClick: () => void;
  icon?: React.ReactNode;
  loading?: boolean;
  disabled?: boolean;
  ariaLabel?: string;
  testId?: string;
}

export interface EmptyStateProps {
  kind: EmptyStateKind;
  size: EmptyStateSize;
  /** 仅 page 与 card 尺寸渲染图示（图标或缩小界面线框），外部需自行保证 aria-hidden="true" */
  visual?: React.ReactNode;
  title: string;
  /** 仅 page 尺寸展示；card / row / menu 自动省略说明 */
  description?: React.ReactNode;
  /** all-done 类型不展示按钮；其余类型至多一主一次两个动作 */
  primaryAction?: EmptyStateAction;
  secondaryAction?: EmptyStateAction;
  className?: string;
  testId?: string;
}

/**
 * 统一空状态组件：
 * - 四档尺寸：page / card / row / menu
 * - 五类语义：first-use / no-results / all-done / failed / no-access
 * - 固定结构顺序：图示 -> 标题 -> 说明 -> 按钮
 * - no-results 自动挂载 aria-live="polite" 温和播报
 */
export const EmptyState: React.FC<EmptyStateProps> = ({
  kind,
  size,
  visual,
  title,
  description,
  primaryAction,
  secondaryAction,
  className,
  testId,
}) => {
  const isAllDone = kind === "all-done";
  const isNoResults = kind === "no-results";
  const isFailed = kind === "failed";

  const liveProps = isNoResults
    ? ({ role: "status", "aria-live": "polite" } as const)
    : {};

  if (size === "menu") {
    return (
      <div
        data-testid={testId}
        {...liveProps}
        className={cn(
          "sp-empty-enter px-3 py-2 text-center text-xs text-sp-text-muted select-none",
          className
        )}
      >
        {title}
      </div>
    );
  }

  if (size === "row") {
    const action = !isAllDone ? primaryAction : undefined;
    return (
      <div
        data-testid={testId}
        {...liveProps}
        className={cn(
          "sp-empty-enter flex items-center justify-between gap-2 px-2 py-2 text-xs text-sp-text-muted",
          className
        )}
      >
        <span className="truncate">{title}</span>
        {action && (
          <button
            type="button"
            onClick={action.onClick}
            disabled={action.disabled || action.loading}
            aria-label={action.ariaLabel}
            data-testid={action.testId}
            className="inline-flex shrink-0 items-center gap-1 rounded-sp-sm px-2 py-1 text-[13px] font-semibold text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text transition-colors duration-fast ease-sp cursor-pointer disabled:opacity-50"
          >
            {isFailed ? (
              <RefreshCw
                aria-hidden="true"
                className={cn("h-3.5 w-3.5 shrink-0", action.loading && "animate-spin")}
              />
            ) : (
              action.icon
            )}
            <span>{action.label}</span>
          </button>
        )}
      </div>
    );
  }

  if (size === "card") {
    const action = !isAllDone ? primaryAction : undefined;
    return (
      <div
        data-testid={testId}
        {...liveProps}
        className={cn(
          "sp-empty-enter flex items-center justify-between gap-3 rounded-card border border-sp-line bg-sp-surface-muted px-3 py-2 select-none",
          className
        )}
      >
        <div className="flex min-w-0 items-center gap-3">
          {visual && (
            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-sp-sm border border-sp-line bg-sp-surface">
              {visual}
            </div>
          )}
          <div className="min-w-0 truncate text-[13px] font-semibold text-sp-text">
            {title}
          </div>
        </div>
        {action && (
          <button
            type="button"
            onClick={action.onClick}
            disabled={action.disabled || action.loading}
            aria-label={action.ariaLabel}
            data-testid={action.testId}
            className="inline-flex h-8 shrink-0 items-center gap-1 rounded-sp-sm px-2 text-[13px] font-semibold text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text transition-colors duration-fast ease-sp cursor-pointer disabled:opacity-50"
          >
            {isFailed ? (
              <RefreshCw
                aria-hidden="true"
                className={cn("h-3.5 w-3.5 shrink-0", action.loading && "animate-spin")}
              />
            ) : (
              action.icon
            )}
            <span>{action.label}</span>
          </button>
        )}
      </div>
    );
  }

  // size === "page": 略高于垂直居中（约 40% 视高视觉重心），结构固定为 图示 -> 标题 -> 说明 -> 按钮
  const showPrimaryAsSecondary = isNoResults || isFailed;
  const visiblePrimary = !isAllDone ? primaryAction : undefined;
  const visibleSecondary = !isAllDone ? secondaryAction : undefined;

  return (
    <div
      data-testid={testId}
      {...liveProps}
      className={cn(
        "sp-empty-enter flex w-full flex-col items-center justify-center text-center px-6 pt-6 pb-12 select-none",
        className
      )}
    >
      {visual && <div className="mb-3 flex shrink-0 items-center justify-center">{visual}</div>}

      <div className="max-w-sm space-y-1">
        <div className="text-sm font-semibold text-sp-text tracking-tight">{title}</div>
        {description && (
          <div className="text-xs leading-relaxed text-sp-text-muted">{description}</div>
        )}
      </div>

      {(visiblePrimary || visibleSecondary) && (
        <div className="mt-4 flex flex-wrap items-center justify-center gap-2">
          {visiblePrimary && (
            <button
              type="button"
              onClick={visiblePrimary.onClick}
              disabled={visiblePrimary.disabled || visiblePrimary.loading}
              aria-label={visiblePrimary.ariaLabel}
              data-testid={visiblePrimary.testId}
              className={cn(
                "inline-flex h-8 items-center gap-1 rounded-sp-sm px-3 text-[13px] font-semibold transition-colors duration-fast ease-sp cursor-pointer disabled:opacity-50",
                showPrimaryAsSecondary
                  ? "border border-sp-line bg-sp-surface text-sp-text hover:bg-sp-hover shadow-xs"
                  : "bg-sp-accent-fill text-sp-accent-ink hover:opacity-90 shadow-xs"
              )}
            >
              {isFailed ? (
                <RefreshCw
                  aria-hidden="true"
                  className={cn("h-3.5 w-3.5 shrink-0", visiblePrimary.loading && "animate-spin")}
                />
              ) : (
                visiblePrimary.icon
              )}
              <span>{visiblePrimary.label}</span>
            </button>
          )}
          {visibleSecondary && (
            <button
              type="button"
              onClick={visibleSecondary.onClick}
              disabled={visibleSecondary.disabled}
              aria-label={visibleSecondary.ariaLabel}
              data-testid={visibleSecondary.testId}
              className={cn(
                "inline-flex h-8 items-center gap-1 rounded-sp-sm px-3 text-[13px] font-semibold transition-colors duration-fast ease-sp cursor-pointer disabled:opacity-50",
                isNoResults || kind === "no-access"
                  ? "text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text"
                  : "border border-sp-line bg-sp-surface text-sp-text hover:bg-sp-hover shadow-xs"
              )}
            >
              {visibleSecondary.icon}
              <span>{visibleSecondary.label}</span>
            </button>
          )}
        </div>
      )}
    </div>
  );
};
