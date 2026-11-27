import { Loader2, RotateCcw, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import { useDelayedFlag, useElapsedStage } from "@/hooks/useLoadingPrimitives";
import type { PreviewRuntimeState } from "@/lib/catalogPreview";

type MapPreviewStatusProps = {
  preview: PreviewRuntimeState;
  activeProductKey: string | null;
  onCancel: () => void;
  onClose: () => void;
  onRetry: () => void;
};

/** Controls for the existing map layer; this component never loads another image. */
export function MapPreviewStatus({ preview, activeProductKey, onCancel, onClose, onRetry }: MapPreviewStatusProps) {
  const isVisible =
    Boolean(activeProductKey) &&
    preview.product_key === activeProductKey &&
    preview.status !== "not_requested" &&
    preview.status !== "stale";
  const loading = isVisible && preview.status === "loading";
  const showLoadingSpinner = useDelayedFlag(loading);
  const showSlowReassurance = useElapsedStage(loading, 10_000);

  if (!isVisible) return null;

  const failed = ["failed", "unavailable", "authentication_required"].includes(preview.status);
  const cancelled = preview.status === "cancelled";
  const message = loading
    ? showSlowReassurance
      ? "正在加载影像（耗时较长，仍在读取瓦片…）"
      : "正在加载影像"
    : failed ? preview.message || "影像预览不可用"
      : cancelled ? "预览已取消" : "影像预览";

  return (
    <div className="pointer-events-none absolute inset-x-3 bottom-[6.75rem] z-[530]">
      <section aria-label="地图影像预览" data-preview-status={preview.status}
        className="pointer-events-auto flex w-fit max-w-full min-w-0 items-center gap-2 rounded-md border border-border bg-background/95 px-2 py-1.5 text-xs shadow-sm">
        {loading && showLoadingSpinner && <Loader2 aria-hidden className="h-3.5 w-3.5 shrink-0 animate-spin motion-reduce:animate-none" />}
        <p role={failed ? "alert" : "status"} title={message}
          className={failed ? "min-w-0 truncate text-destructive" : "min-w-0 truncate"}>{message}</p>
        {loading && <Button size="sm" variant="ghost" className="h-7 shrink-0 px-2" onClick={onCancel}
          aria-label="取消地图预览">取消</Button>}
        {(failed || cancelled) && <Button size="sm" variant="ghost" className="h-7 shrink-0 px-2" onClick={onRetry}
          aria-label="重试地图预览"><RotateCcw className="h-3.5 w-3.5" />重试</Button>}
        <Button size="sm" variant="ghost" className="h-7 w-7 shrink-0 p-0" onClick={onClose}
          aria-label="关闭地图预览" title="关闭预览"><X className="h-3.5 w-3.5" /></Button>
      </section>
    </div>
  );
}
