import { useCallback, useEffect, useRef, useState } from "react";
import { Eye, Loader2, X } from "lucide-react";
import { EmptyState } from "@/components/ui/EmptyState";
import { MapPreviewPanel } from "@/components/MapPreviewPanel";
import { useDelayedFlag, useElapsedStage } from "@/hooks/useLoadingPrimitives";
import { cancelApplicationRequest, getPreviewAsset } from "@/lib/bridge";
import {
  EMPTY_PREVIEW_STATE,
  PreviewRequestCoordinator,
  PreviewResolutionError,
  type PreviewRuntimeState,
} from "@/lib/catalogPreview";

/** One disposable viewer bound to an immutable catalogue identity. */
export function FrozenProductPreview({
  batchId,
  itemId,
  productLabel,
  onClose,
  onPreviewChange,
  compactOverlay = false,
}: {
  batchId: string;
  itemId: string;
  productLabel: string;
  onClose(): void;
  onPreviewChange?: (state: PreviewRuntimeState) => void;
  compactOverlay?: boolean;
}) {
  const [preview, setPreview] = useState(EMPTY_PREVIEW_STATE);
  const showLoadingSpinner = useDelayedFlag(preview.status === "loading");
  const showSlowReassurance = useElapsedStage(preview.status === "loading", 10_000);
  const coordinator = useRef<PreviewRequestCoordinator | null>(null);
  if (!coordinator.current) coordinator.current = new PreviewRequestCoordinator(setPreview);

  useEffect(() => {
    onPreviewChange?.(preview);
  }, [onPreviewChange, preview]);

  const request = useCallback(() => {
    const requestId = `product-preview-${crypto.randomUUID()}`;
    void coordinator.current!.requestResolvedPreview(
      itemId,
      async (signal) => {
        if (signal.aborted) throw new DOMException("Preview cancelled", "AbortError");
        const result = await getPreviewAsset({
          request_id: requestId,
          batch_id: batchId,
          item_id: itemId,
        });
        if (signal.aborted) throw new DOMException("Preview cancelled", "AbortError");
        if (!result.ok) throw new PreviewResolutionError(result.code);
        return result.preview;
      },
      () => {
        void cancelApplicationRequest(requestId).catch(() => undefined);
      },
    );
  }, [batchId, itemId]);

  useEffect(() => {
    request();
    return () => coordinator.current?.dispose();
  }, [request]);

  if (compactOverlay) {
    return (
      <div
        data-subset-footprint-preview-bar
        className="pointer-events-auto absolute bottom-3 left-3 z-[520] flex max-w-[calc(100%-1.5rem)] flex-wrap items-center gap-2 rounded-sp-md border border-sp-border bg-sp-surface/95 px-3 py-1.5 text-xs text-sp-text shadow-float backdrop-blur-sm"
      >
        <Eye className="h-3.5 w-3.5 shrink-0 text-sp-text-secondary" />
        <span className="max-w-[240px] truncate font-semibold text-sp-text" title={productLabel}>
          框内预览：{productLabel}
        </span>
        {preview.status === "loading" && (
          <span className="inline-flex items-center gap-1 text-[11px] text-sp-text-muted">
            {showLoadingSpinner && <Loader2 className="h-3 w-3 animate-spin text-sp-text-secondary" />}
            <span>加载快视图中…</span>
            {showSlowReassurance && <span>（耗时较长，仍在连接）</span>}
          </span>
        )}
        {preview.status === "available" && (
          <span className="rounded-full border border-sp-border bg-sp-hover px-2 py-0.5 text-[11px] font-medium text-sp-text-secondary">
            已叠加至影像框
          </span>
        )}
        {preview.status === "unavailable" && (
          <span className="inline-flex items-center gap-1.5 text-[11px] text-sp-warning-text">
            <span className="rounded bg-sp-warning-bg px-1.5 py-0.5 font-medium">无预览图</span>
            <span>{preview.message || "仅显示影像范围框"}</span>
          </span>
        )}
        {(preview.status === "failed" || preview.status === "authentication_required") && (
          <EmptyState
            kind="failed"
            size="row"
            title={preview.message || "快视图加载失败，仅显示影像范围框"}
            primaryAction={{
              label: "重试",
              onClick: request,
            }}
            className="py-0 px-0 text-[11px] text-sp-warning-text"
          />
        )}
        <button
          type="button"
          aria-label="关闭产品预览"
          onClick={() => {
            coordinator.current?.cancel();
            onClose();
          }}
          className="ml-1 inline-flex items-center gap-1 rounded-sp-sm border border-sp-line bg-sp-surface px-2 py-0.5 text-[11px] font-medium text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text cursor-pointer"
        >
          <X className="h-3 w-3" />
          关闭预览
        </button>
      </div>
    );
  }

  return (
    <MapPreviewPanel
      preview={preview}
      activeProductKey={itemId}
      productLabel={productLabel}
      onClose={() => {
        coordinator.current?.cancel();
        onClose();
      }}
      onCancel={() => {
        coordinator.current?.cancel();
      }}
      onRetry={request}
    />
  );
}
