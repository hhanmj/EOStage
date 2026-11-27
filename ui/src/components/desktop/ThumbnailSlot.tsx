import React, { useEffect, useRef, useState } from "react";
import { ImageOff } from "lucide-react";
import type { SceneRow } from "@/lib/bridge";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";
import {
  thumbnailUrlFor,
  isAllowedByCsp,
  isProxyablePreviewUrl,
  sharedThumbnailQueue,
  placeholderReason,
  type ThumbnailState,
} from "@/shell/api/thumbnailLogic";
import { sharedGeodataPreviewQueue } from "@/shell/api/geodataPreviewLoader";
import {
  opticalQuicklookCacheKey,
  opticalQuicklookPlan,
} from "@/shell/api/opticalQuicklookLogic";
import { sharedOpticalQuicklookQueue } from "@/shell/api/opticalQuicklookLoader";

export interface ThumbnailSlotProps {
  sceneId?: string;
  scene?: Partial<SceneRow> | null;
  batchId?: string | null;
  className?: string;
}

/**
 * 48x48 快视图插槽（W5-3c）
 * - 懒加载：仅当元素进入视口（IntersectionObserver，rootMargin: 200px）时提交至全局单例队列；
 * - 限制并发：最多 4 个并发，FIFO 排队；
 * - 失败不重试：失败后持久标记为 failed，后续不再发起网络重试；
 * - 尺寸固定：严格保持 48x48，无 spinner 动画，防止布局抖动；
 * - 卸载清理：组件 unmount 时取消排队中的请求。
 */
export function ThumbnailSlot({
  sceneId: _sceneId,
  scene,
  batchId,
  className = "",
}: ThumbnailSlotProps) {
  const { t } = useI18n();
  const containerRef = useRef<HTMLDivElement>(null);
  const sceneRef = useRef(scene);
  sceneRef.current = scene;
  const hasScene = Boolean(scene);

  const url = thumbnailUrlFor(scene);
  const isAllowed = Boolean(url && isAllowedByCsp(url));
  const isProxyable = Boolean(url && !isAllowed && isProxyablePreviewUrl(url));
  const opticalPlan = url ? null : opticalQuicklookPlan(scene as SceneRow | null | undefined);
  const opticalKey = scene && opticalPlan && opticalPlan.kind !== "unavailable"
    ? opticalQuicklookCacheKey(scene as SceneRow, batchId)
    : null;
  const cachedOpticalUrl = opticalKey
    ? sharedOpticalQuicklookQueue.getCachedUrl(opticalKey)
    : undefined;
  const cachedProxiedUrl = url && isProxyable
    ? sharedGeodataPreviewQueue.getCachedUrl(url)
    : undefined;
  const [opticalUrl, setOpticalUrl] = useState<string | null>(cachedOpticalUrl ?? null);
  const [proxiedUrl, setProxiedUrl] = useState<string | null>(cachedProxiedUrl ?? null);
  const [state, setState] = useState<ThumbnailState>(() => {
    if (url) {
      if (isAllowed) {
        return sharedThumbnailQueue.getCachedState(url) ?? "pending";
      }
      if (isProxyable) {
        return sharedGeodataPreviewQueue.getCachedState(url) ?? "pending";
      }
      return "placeholder";
    }
    if (!opticalKey) return "placeholder";
    return sharedOpticalQuicklookQueue.getCachedState(opticalKey) ?? "pending";
  });

  useEffect(() => {
    const activeScene = sceneRef.current;
    if (url) {
      if (isAllowed) {
        setProxiedUrl(null);
        const cached = sharedThumbnailQueue.getCachedState(url);
        if (cached) {
          setState(cached);
          return;
        }
        setState("pending");
      } else if (isProxyable) {
        const cachedDataUrl = sharedGeodataPreviewQueue.getCachedUrl(url);
        if (cachedDataUrl) {
          setProxiedUrl(cachedDataUrl);
          setState("loaded");
          return;
        }
        setProxiedUrl(null);
        setState("pending");
      } else {
        setProxiedUrl(null);
        setState("placeholder");
        return;
      }
    } else {
      setProxiedUrl(null);
      setOpticalUrl(null);
      if (!opticalKey || !activeScene || !opticalPlan || opticalPlan.kind === "unavailable") {
        setState("placeholder");
        return;
      }
      const cached = sharedOpticalQuicklookQueue.getCachedState(opticalKey);
      if (cached) {
        setOpticalUrl(sharedOpticalQuicklookQueue.getCachedUrl(opticalKey) ?? null);
        setState(cached);
        return;
      }
      setState("pending");
    }

    // 若运行环境不支持 IntersectionObserver（如某些老环境或 node），回退为不发起请求
    if (typeof IntersectionObserver === "undefined") {
      return;
    }

    let isCancelled = false;
    let enqueuedDirectUrl: string | null = null;
    let enqueuedProxyUrl: string | null = null;
    let enqueuedOpticalKey: string | null = null;
    let settled = false;
    const observer = new IntersectionObserver(
      (entries) => {
        const entry = entries[0];
        if (!entry?.isIntersecting) return;
        observer.disconnect();
        const latestScene = sceneRef.current;
        if (url && isAllowed) {
          enqueuedDirectUrl = url;
          void sharedThumbnailQueue.enqueue(url).then((status) => {
            settled = true;
            if (!isCancelled) setState(status);
          });
        } else if (url && isProxyable) {
          enqueuedProxyUrl = url;
          void sharedGeodataPreviewQueue.enqueue(url).then((result) => {
            settled = true;
            if (isCancelled) return;
            setProxiedUrl(result.dataUrl);
            setState(result.status);
          });
        } else if (opticalKey && latestScene) {
          enqueuedOpticalKey = opticalKey;
          void sharedOpticalQuicklookQueue.enqueue({
            scene: latestScene as SceneRow,
            batchId,
          }).then((status) => {
            settled = true;
            if (isCancelled) return;
            setOpticalUrl(sharedOpticalQuicklookQueue.getCachedUrl(opticalKey) ?? null);
            setState(status);
          });
        }
      },
      { rootMargin: "200px" }
    );

    if (containerRef.current) {
      observer.observe(containerRef.current);
    }

    return () => {
      isCancelled = true;
      observer.disconnect();
      if (!settled) {
        if (enqueuedDirectUrl) sharedThumbnailQueue.cancel(enqueuedDirectUrl);
        else if (enqueuedProxyUrl) sharedGeodataPreviewQueue.cancel(enqueuedProxyUrl);
        else if (enqueuedOpticalKey) sharedOpticalQuicklookQueue.cancel(enqueuedOpticalKey);
      }
    };
  }, [url, isAllowed, isProxyable, hasScene, opticalKey, opticalPlan?.kind, batchId]);

  const displayUrl = url && isAllowed ? url : (proxiedUrl || opticalUrl);
  if (state === "loaded" && displayUrl) {
    return (
      <img
        src={displayUrl}
        alt={t("mapWorkbench.thumbnail.quicklook")}
        loading="lazy"
        className={cn(
          "w-12 h-12 rounded-sp-sm shrink-0 select-none ring-1 ring-sp-line object-contain",
          opticalPlan ? "bg-sp-surface-muted/40" : "bg-neutral-950/90",
          className
        )}
      />
    );
  }

  if (state === "pending") {
    return (
      <div
        ref={containerRef}
        className={cn(
          "w-12 h-12 rounded-sp-sm ring-1 ring-sp-line bg-sp-card select-none shrink-0",
          className
        )}
        title={t("mapWorkbench.thumbnail.loading")}
        aria-label={t("mapWorkbench.thumbnail.loading")}
      />
    );
  }

  if (state === "failed") {
    return (
      <div
        className={cn(
          "w-12 h-12 rounded-sp-sm ring-1 ring-sp-line bg-sp-card flex items-center justify-center text-xs font-semibold text-sp-text-muted select-none shrink-0",
          className
        )}
        title={t("mapWorkbench.thumbnail.failed")}
        aria-label={t("mapWorkbench.thumbnail.failed")}
      >
        <ImageOff className="h-4 w-4 text-sp-text-muted/70" aria-hidden="true" />
      </div>
    );
  }

  const reason = placeholderReason(scene);
  const placeholderTitle = opticalPlan?.kind === "unavailable"
    ? opticalPlan.title
    : reason === "blocked"
      ? t("mapWorkbench.thumbnail.blocked")
      : t("mapWorkbench.thumbnail.missing");

  return (
    <div
      className={cn(
        "w-12 h-12 rounded-sp-sm ring-1 ring-sp-line bg-sp-card flex items-center justify-center text-xs font-semibold text-sp-text-muted select-none shrink-0",
        className
      )}
      title={placeholderTitle}
      aria-label={placeholderTitle}
    >
      <ImageOff className="h-4 w-4 text-sp-text-muted/70" aria-hidden="true" />
    </div>
  );
}
