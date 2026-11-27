import React, { useId } from "react";
import { cn } from "@/lib/utils";

/**
 * 带可见悬停说明的短标签（真机上原生 title 不可靠）。说明常驻 DOM、用 aria-describedby 关联，
 * 悬停或键盘聚焦时显示；样式沿用活动栏的气泡提示。没有说明时只渲染标签。
 * 标签在按钮里时传 parentTooltipId：按钮（带 group/evidence 类）持有 aria-describedby，按钮键盘聚焦时显示说明。
 */
export function HoverHintLabel({
  hint,
  className,
  focusable = false,
  parentTooltipId,
  align = "start",
  placement = "bottom",
  maxWidthClass,
  children,
}: {
  hint: string | null | undefined;
  className?: string;
  /** 不在按钮里时设为 true，让键盘也能聚焦到标签看说明 */
  focusable?: boolean;
  parentTooltipId?: string;
  /** 靠右的标签可让气泡向左展开。 */
  align?: "start" | "end";
  /** 弹窗底部字段可向上展开避免被滚动容器裁剪 */
  placement?: "bottom" | "top";
  /** 自定义气泡最大宽度（例如窄弹窗内使用 max-w-[240px]） */
  maxWidthClass?: string;
  children: React.ReactNode;
}) {
  const ownId = useId();
  if (!hint) return <span className={className}>{children}</span>;
  const inParent = typeof parentTooltipId === "string" && parentTooltipId.length > 0;
  const id = inParent ? parentTooltipId : ownId;
  const selfFocusable = focusable && !inParent;
  return (
    <span className="relative inline-flex items-center group/hint">
      <span
        className={cn(
          "inline-flex items-center",
          className,
          selfFocusable && "cursor-help rounded-sp-sm outline-none focus-visible:ring-1 focus-visible:ring-sp-accent"
        )}
        aria-describedby={inParent ? undefined : id}
        tabIndex={selfFocusable ? 0 : undefined}
      >
        {children}
      </span>
      <span
        id={id}
        role="tooltip"
        className={cn(
          "pointer-events-none absolute z-[80] w-max whitespace-normal break-words rounded-sp-sm bg-sp-surface px-2.5 py-1.5 text-xs font-normal leading-relaxed text-sp-text shadow-overlay ring-1 ring-sp-line opacity-0 transition-opacity duration-fast ease-sp group-hover/hint:opacity-100 group-focus-within/hint:opacity-100",
          maxWidthClass ?? "max-w-[min(32rem,calc(100vw-2rem))]",
          placement === "top" ? "bottom-full mb-1.5" : "top-full mt-1",
          align === "end" ? "right-0" : "left-0",
          inParent && "group-focus-visible/evidence:opacity-100"
        )}
      >
        {hint}
      </span>
    </span>
  );
}
