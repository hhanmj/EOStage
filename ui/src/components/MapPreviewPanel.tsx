import {
  AlertCircle,
  Download,
  FolderOpen,
  Eye,
  EyeOff,
  KeyRound,
  Loader2,
  Maximize2,
  RotateCcw,
  Scan,
  X,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
} from "react";

import type { PreviewRuntimeState } from "@/lib/catalogPreview";
import {
  isDesktopControlledPreviewDataUrl,
  isPreviewActionAllowed,
} from "@/lib/catalogPreview";
import {
  panPreview,
  PREVIEW_MAX_ZOOM,
  PREVIEW_MIN_ZOOM,
  PREVIEW_WHEEL_ZOOM_FACTOR,
  fitPreviewViewport,
  resetPreviewViewport,
  zoomPreviewAroundPoint,
} from "@/lib/mapPreviewViewport";
import { cn } from "@/lib/utils";

export type MapPreviewPanelProps = {
  preview: PreviewRuntimeState;
  activeProductKey: string | null;
  productLabel: string | null;
  onClose: () => void;
  onCancel: () => void;
  onRetry?: () => void;
  onSave?: () => void;
  onOpenSaveDirectory?: () => void;
  saving?: boolean;
  savedPath?: string | null;
};

function statusLabel(preview: PreviewRuntimeState): string {
  if (preview.status === "loading") return "加载中";
  if (preview.status === "available") return "可用";
  if (preview.status === "authentication_required") return "需要认证";
  if (preview.status === "failed") return "加载失败";
  if (preview.status === "unavailable") return "无预览图";
  if (preview.status === "cancelled") return "已取消";
  return "未加载";
}

function StatusIcon({ status }: { status: PreviewRuntimeState["status"] }) {
  if (status === "loading") {
    return <Loader2 className="h-4 w-4 animate-spin motion-reduce:animate-none" />;
  }
  if (status === "authentication_required") return <KeyRound className="h-4 w-4" />;
  if (status === "failed") return <AlertCircle className="h-4 w-4" />;
  if (status === "available") return <Eye className="h-4 w-4" />;
  return <EyeOff className="h-4 w-4" />;
}

function previewSource(url: string | null): string {
  if (!url) return "来源未知";
  if (isDesktopControlledPreviewDataUrl(url)) {
    return "桌面受控快视图";
  }
  try {
    return new URL(url).hostname;
  } catch {
    return "来源无效";
  }
}


function previewFileName(label: string | null, url: string | null): string {
  if (isDesktopControlledPreviewDataUrl(url)) return label || "product-preview";
  if (url) {
    try {
      const name = decodeURIComponent(new URL(url).pathname.split("/").at(-1) || "");
      if (name) return name;
    } catch {
      // Fall through to the product label.
    }
  }
  return label || "product-preview";
}

export function MapPreviewPanel({
  preview,
  activeProductKey,
  productLabel,
  onClose,
  onCancel,
  onRetry,
  onSave,
  onOpenSaveDirectory,
  saving = false,
  savedPath = null,
}: MapPreviewPanelProps) {
  const [viewMode, setViewMode] = useState<"fit" | "actual">("fit");
  const [zoom, setZoom] = useState(1);
  const [offset, setOffset] = useState({ x: 0, y: 0 });
  const [naturalSize, setNaturalSize] = useState({ width: 0, height: 0 });
  const [viewportSize, setViewportSize] = useState({ width: 0, height: 0 });
  const [dragging, setDragging] = useState(false);
  const viewportRef = useRef<HTMLDivElement>(null);
  const imageRef = useRef<HTMLImageElement>(null);
  // A replacement URL needs a new DOM node even for the same product, so a
  // late load/error event cannot be attributed to its replacement.
  const imageIdentity = useMemo(() => crypto.randomUUID(), [activeProductKey, preview.url]);
  const [failedImageIdentity, setFailedImageIdentity] = useState<string | null>(null);
  const dragRef = useRef<{
    pointerId: number;
    clientX: number;
    clientY: number;
  } | null>(null);

  const available = preview.status === "available" && Boolean(preview.url);
  const publicActionAllowed = available && isPreviewActionAllowed(preview.url);
  const displayPreview: PreviewRuntimeState = preview.status === "available" && !publicActionAllowed
    ? { ...preview, status: "failed", message: "该来源未通过预览安全校验，无法显示预览图。" }
    : available && failedImageIdentity === imageIdentity
      ? { ...preview, status: "failed", message: "预览图显示失败，请重试加载。原始数据和下载任务不受影响。" }
      : preview;
  const showingImage = displayPreview.status === "available" && publicActionAllowed;
  const controlledPreview = isDesktopControlledPreviewDataUrl(preview.url);
  const saveTitle = publicActionAllowed
    ? (onSave
      ? (controlledPreview ? "保存桌面受控预览图" : "保存公共预览图")
      : "当前入口未提供保存预览图操作")
    : "该来源未通过预览安全校验";
  const fileName = previewFileName(productLabel, preview.url);
  const source = previewSource(preview.url);
  const sourceTitle = isDesktopControlledPreviewDataUrl(preview.url)
    ? "桌面受控快视图"
    : preview.url || undefined;

  useLayoutEffect(() => {
    setFailedImageIdentity(null);
    setViewMode("fit");
    setZoom(1);
    setOffset({ x: 0, y: 0 });
    setDragging(false);
    dragRef.current = null;
    const image = imageRef.current;
    const width = image?.complete ? image.naturalWidth : 0;
    const height = image?.complete ? image.naturalHeight : 0;
    setNaturalSize(width > 0 && height > 0 ? { width, height } : { width: 0, height: 0 });
  }, [preview.url, activeProductKey, available]);

  const measureViewport = () => {
    const viewport = viewportRef.current;
    if (!viewport) return;
    const nextSize = {
      width: viewport.clientWidth,
      height: viewport.clientHeight,
    };
    setViewportSize((current) => (
      current.width === nextSize.width && current.height === nextSize.height
        ? current
        : nextSize
    ));
  };

  useEffect(() => {
    if (!showingImage) return;
    const viewport = viewportRef.current;
    if (!viewport) return;
    measureViewport();
    const observer = typeof ResizeObserver === "function"
      ? new ResizeObserver(measureViewport)
      : null;
    observer?.observe(viewport);
    window.addEventListener("resize", measureViewport);
    return () => {
      observer?.disconnect();
      window.removeEventListener("resize", measureViewport);
    };
  }, [showingImage, imageIdentity]);

  const fitTransform = fitPreviewViewport(naturalSize, viewportSize);
  const displayTransform = viewMode === "fit"
    ? fitTransform
    : { zoom, offset };
  const minimumZoom = Math.min(PREVIEW_MIN_ZOOM, fitTransform.zoom);
  const imageStyle = useMemo(() => {
    if (naturalSize.width <= 0 || naturalSize.height <= 0) return undefined;
    return {
      width: `${naturalSize.width * displayTransform.zoom}px`,
      height: `${naturalSize.height * displayTransform.zoom}px`,
      maxWidth: "none",
      maxHeight: "none",
      transform: `translate3d(${displayTransform.offset.x}px, ${displayTransform.offset.y}px, 0)`,
      transformOrigin: "top left",
    };
  }, [displayTransform.offset.x, displayTransform.offset.y, displayTransform.zoom, naturalSize]);

  useEffect(() => {
    if (!showingImage) return;
    const viewport = viewportRef.current;
    if (!viewport) return;
    const handleWheel = (event: WheelEvent) => {
      if (!Number.isFinite(event.deltaY) || event.deltaY === 0) return;
      event.preventDefault();
      const bounds = viewport.getBoundingClientRect();
      const anchor = {
        x: event.clientX - bounds.left,
        y: event.clientY - bounds.top,
      };
      const currentTransform = viewMode === "fit" ? fitTransform : { zoom, offset };
      const direction = event.deltaY < 0
        ? PREVIEW_WHEEL_ZOOM_FACTOR
        : 1 / PREVIEW_WHEEL_ZOOM_FACTOR;
      const result = zoomPreviewAroundPoint(
        currentTransform.zoom,
        currentTransform.offset,
        currentTransform.zoom * direction,
        anchor,
        minimumZoom,
      );
      setViewMode("actual");
      setZoom(result.zoom);
      setOffset(result.offset);
    };
    viewport.addEventListener("wheel", handleWheel, { passive: false });
    return () => viewport.removeEventListener("wheel", handleWheel);
  }, [showingImage, imageIdentity, fitTransform.offset.x, fitTransform.offset.y, fitTransform.zoom, minimumZoom, offset, viewMode, zoom]);

  if (!activeProductKey || preview.status === "not_requested" || preview.status === "stale") {
    return null;
  }
  if (preview.product_key && preview.product_key !== activeProductKey) return null;

  const viewportCenter = (): { x: number; y: number } => {
    const viewport = viewportRef.current;
    return viewport
      ? { x: viewport.clientWidth / 2, y: viewport.clientHeight / 2 }
      : { x: 0, y: 0 };
  };

  const applyZoom = (nextZoom: number, anchor: { x: number; y: number }) => {
    const currentTransform = viewMode === "fit" ? fitTransform : { zoom, offset };
    const result = zoomPreviewAroundPoint(
      currentTransform.zoom,
      currentTransform.offset,
      nextZoom,
      anchor,
      minimumZoom,
    );
    setViewMode("actual");
    setZoom(result.zoom);
    setOffset(result.offset);
  };

  const enterActual = (nextZoom: number) => {
    if (nextZoom === 1) {
      setViewMode("actual");
      setZoom(1);
      setOffset({ x: 0, y: 0 });
      return;
    }
    applyZoom(nextZoom, viewportCenter());
  };

  const resetViewport = () => {
    const reset = resetPreviewViewport();
    setViewMode("fit");
    setZoom(reset.zoom);
    setOffset(reset.offset);
    setDragging(false);
    dragRef.current = null;
  };

  const fitViewport = () => {
    setViewMode("fit");
    setZoom(1);
    setOffset({ x: 0, y: 0 });
    setDragging(false);
    dragRef.current = null;
  };

  const handlePointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (viewMode !== "actual" || event.button !== 0) return;
    dragRef.current = {
      pointerId: event.pointerId,
      clientX: event.clientX,
      clientY: event.clientY,
    };
    if (event.currentTarget.setPointerCapture) {
      event.currentTarget.setPointerCapture(event.pointerId);
    }
    setDragging(true);
  };

  const handlePointerMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    const delta = {
      x: event.clientX - drag.clientX,
      y: event.clientY - drag.clientY,
    };
    dragRef.current = {
      pointerId: drag.pointerId,
      clientX: event.clientX,
      clientY: event.clientY,
    };
    setOffset((current) => panPreview(current, delta));
  };

  const finishPointerDrag = (event: ReactPointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    if (event.currentTarget.hasPointerCapture?.(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    dragRef.current = null;
    setDragging(false);
  };

  return (
    <aside
      data-map-preview-panel
      data-preview-status={displayPreview.status}
      aria-label="当前产品预览"
      className="flex h-full min-h-0 flex-col overflow-hidden bg-background"
    >
      <header className="flex h-9 min-w-0 shrink-0 items-center gap-2 border-b border-border/70 px-2.5 text-xs">
        <StatusIcon status={displayPreview.status} />
        <span className="shrink-0 font-semibold">产品预览</span>
        <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-muted-foreground" title={productLabel || undefined}>
          {productLabel || activeProductKey}
        </span>
        <span
          className={cn(
            "shrink-0 rounded border px-1.5 py-0.5 text-[10px]",
            displayPreview.status === "available" && "border-success/35 bg-success/10 text-success",
            displayPreview.status === "failed" && "border-destructive/35 bg-destructive/10 text-destructive",
            displayPreview.status === "authentication_required" && "border-warning/35 bg-warning/10 text-warning",
          )}
        >
          {statusLabel(displayPreview)}
        </span>
        <button
          type="button"
          aria-label="关闭产品预览"
          title="关闭产品预览"
          className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          onClick={onClose}
        >
          <X className="h-4 w-4" />
        </button>
      </header>

      {showingImage ? (
        <>
          <div className="flex h-9 shrink-0 items-center gap-1 border-b border-border/60 px-2">
            <button type="button" aria-label="适合窗口" title="适合窗口" className="grid h-7 w-7 place-items-center rounded-md hover:bg-muted" onClick={fitViewport}>
              <Maximize2 className="h-3.5 w-3.5" />
            </button>
            <button type="button" aria-label="原始尺寸" title="原始尺寸（1:1）" className="grid h-7 w-7 place-items-center rounded-md hover:bg-muted" onClick={() => enterActual(1)}>
              <Scan className="h-3.5 w-3.5" />
            </button>
            <button type="button" aria-label="重置视图" title="重置视图" className="grid h-7 w-7 place-items-center rounded-md hover:bg-muted" onClick={resetViewport}>
              <RotateCcw className="h-3.5 w-3.5" />
            </button>
            <button type="button" aria-label="缩小预览" title="缩小预览" className="grid h-7 w-7 place-items-center rounded-md hover:bg-muted disabled:opacity-40" disabled={viewMode === "actual" && zoom <= minimumZoom} onClick={() => enterActual(displayTransform.zoom / 1.25)}>
              <ZoomOut className="h-3.5 w-3.5" />
            </button>
            <button type="button" aria-label="放大预览" title="放大预览" className="grid h-7 w-7 place-items-center rounded-md hover:bg-muted disabled:opacity-40" disabled={viewMode === "actual" && zoom >= PREVIEW_MAX_ZOOM} onClick={() => enterActual(displayTransform.zoom * 1.25)}>
              <ZoomIn className="h-3.5 w-3.5" />
            </button>
            <span className="ml-1 text-[10px] tabular-nums text-muted-foreground">
              {viewMode === "fit" ? "适合" : `${Math.round(zoom * 100)}%`}
            </span>
            <div className="ml-auto flex items-center gap-1">
              <button type="button" className="inline-flex h-7 items-center gap-1 rounded-md px-2 text-[11px] hover:bg-muted disabled:opacity-40" disabled={!publicActionAllowed || !onSave || saving} title={saveTitle} onClick={onSave}>
                {saving ? <Loader2 className="h-3.5 w-3.5 animate-spin motion-reduce:animate-none" /> : <Download className="h-3.5 w-3.5" />}
                保存
              </button>
            </div>
          </div>
          <div
            ref={viewportRef}
            data-preview-viewport
            aria-label="预览视口"
            className={cn(
              "relative min-h-0 flex-1 overflow-hidden bg-slate-950/90 touch-none",
              viewMode === "actual" && "cursor-grab",
              dragging && "cursor-grabbing",
            )}
            onPointerDown={handlePointerDown}
            onPointerMove={handlePointerMove}
            onPointerUp={finishPointerDrag}
            onPointerCancel={finishPointerDrag}
          >
            <img
              key={imageIdentity}
              ref={imageRef}
              src={preview.url!}
              alt={productLabel ? productLabel + " 预览" : "当前产品 预览"}
              draggable={false}
              className="absolute left-0 top-0 max-w-none select-none"
              data-preview-zoom={displayTransform.zoom}
              data-preview-offset-x={displayTransform.offset.x}
              data-preview-offset-y={displayTransform.offset.y}
              style={imageStyle}
              onLoad={(event) => {
                if (event.currentTarget !== imageRef.current) return;
                setNaturalSize({
                  width: event.currentTarget.naturalWidth,
                  height: event.currentTarget.naturalHeight,
                });
                measureViewport();
              }}
              onError={(event) => {
                if (event.currentTarget !== imageRef.current) return;
                setFailedImageIdentity(imageIdentity);
              }}
            />
          </div>
          <footer className="flex min-w-0 shrink-0 items-center gap-2 border-t border-border/60 px-2.5 py-1 text-[10px] text-muted-foreground">
            {savedPath ? (
              <button
                type="button"
                className="flex min-w-0 flex-1 items-center gap-1 rounded px-1 py-0.5 text-left hover:bg-muted hover:text-foreground"
                title={onOpenSaveDirectory ? `保存到：${savedPath}` : "当前入口未提供打开保存目录操作"}
                disabled={!onOpenSaveDirectory}
                onClick={onOpenSaveDirectory}
              >
                <FolderOpen className="h-3 w-3 shrink-0" />
                <span className="min-w-0 truncate font-mono">保存到：{savedPath}</span>
              </button>
            ) : (
              <span className="min-w-0 flex-1 truncate font-mono" title={fileName}>{fileName}</span>
            )}
            <span className="shrink-0" title={sourceTitle}>来源：{source}</span>
          </footer>
        </>
      ) : (
        <div className="flex min-h-24 flex-1 items-center gap-3 px-4 py-3 text-xs">
          <span className="shrink-0 text-muted-foreground"><StatusIcon status={displayPreview.status} /></span>
          <p className="min-w-0 flex-1 break-words leading-5 text-muted-foreground">
            {displayPreview.message || "当前产品没有可展示的预览图。"}
          </p>
          {(displayPreview.status === "failed" || displayPreview.status === "cancelled") && onRetry && (
            <button type="button" aria-label="重试预览" className="shrink-0 rounded-md border px-2 py-1.5 hover:bg-muted" onClick={() => {
              setFailedImageIdentity(null);
              onRetry();
            }}>
              <RotateCcw className="mr-1 inline h-3.5 w-3.5" />
              重试
            </button>
          )}
          {preview.status === "loading" && (
            <button type="button" className="shrink-0 rounded-md border px-2 py-1.5 hover:bg-muted" onClick={onCancel}>
              取消
            </button>
          )}
        </div>
      )}
    </aside>
  );
}
