import React from "react";
import { Scissors } from "lucide-react";
import { cn } from "../../lib/utils.ts";

/**
 * 统一裁剪图标：刀尖统一朝向右上角（↗，-rotate-45）。
 * 用于：
 * 1. 数据源目录中支持在线裁剪的数据集右上角标识；
 * 2. 下载中心裁剪任务产品微标签右上角标识；
 * 3. 地图工作台「AOI 裁剪下载」按钮与裁剪工作区标题栏。
 */
export function SubsetScissorsIcon({
  className,
  "aria-hidden": ariaHidden = true,
}: {
  className?: string;
  "aria-hidden"?: boolean;
}) {
  return (
    <Scissors
      aria-hidden={ariaHidden}
      className={cn("-rotate-45 shrink-0", className)}
    />
  );
}

export function SubsetCornerBadge({
  title = "支持在线 AOI 裁剪下载",
  testId = "subset-scissors-badge",
  size = "sm",
  className,
}: {
  title?: string;
  testId?: string;
  size?: "sm" | "md";
  className?: string;
}) {
  return (
    <span
      data-testid={testId}
      title={title}
      className={cn(
        "inline-flex items-center justify-center rounded-sp-xs bg-transparent text-sp-text-secondary",
        size === "md" ? "h-4.5 w-4.5" : "h-4 w-4",
        className,
      )}
    >
      <SubsetScissorsIcon className={size === "md" ? "h-3.5 w-3.5" : "h-3 w-3"} />
    </span>
  );
}
