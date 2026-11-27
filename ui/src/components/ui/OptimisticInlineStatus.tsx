import React from "react";
import { Check } from "lucide-react";
import { cn } from "@/lib/utils";
import { useDelayedFlag } from "@/hooks/useLoadingPrimitives";

export type OptimisticSyncState = "idle" | "saving" | "saved" | "error";

export function OptimisticInlineStatus({
  state,
  savingLabel = "保存中…",
  savedLabel = "已保存",
  errorMessage,
  onRetry,
  className,
  testId,
}: {
  state: OptimisticSyncState;
  savingLabel?: string;
  savedLabel?: string;
  errorMessage?: string | null;
  onRetry?: () => void;
  className?: string;
  testId?: string;
}) {
  const showSaving = useDelayedFlag(state === "saving");

  return (
    <span
      data-testid={testId}
      aria-live="polite"
      className={cn(
        "inline-flex items-center gap-1.5 min-h-[18px] text-[11px] shrink-0",
        className
      )}
    >
      {state === "error" ? (
        <span className="inline-flex items-center gap-1.5 text-sp-danger-text sp-optimistic-shake">
          <span className="truncate max-w-[180px]">{errorMessage || "保存失败，已恢复原状态"}</span>
          {onRetry && (
            <button
              type="button"
              onClick={onRetry}
              className="font-medium underline underline-offset-2 hover:opacity-80 cursor-pointer"
            >
              重试
            </button>
          )}
        </span>
      ) : showSaving ? (
        <span className="text-sp-text-muted">{savingLabel}</span>
      ) : state === "saved" ? (
        <span className="inline-flex items-center gap-1 text-sp-success-text sp-check-morph-in">
          <Check className="h-3 w-3 shrink-0" />
          <span>{savedLabel}</span>
        </span>
      ) : null}
    </span>
  );
}
