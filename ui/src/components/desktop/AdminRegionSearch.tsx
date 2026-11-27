import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Bookmark, History, Layers, Loader2, Plus, RotateCcw, Search, ShieldCheck, Trash2, X } from "lucide-react";
import { useI18n, type t as I18nTranslateFn } from "@/i18n";
import { cn } from "@/lib/utils";
import type { AdminBoundary, AdminBoundaryResult, ApiError, Bbox } from "@/lib/bridge";
import { searchAdminBoundaries, resolveAdminBoundary } from "@/lib/bridge";
import { canonicalAdministrativeAoiGeometry } from "@/lib/administrativeAoiGeometry";
import { createLatestResultGate } from "@/shell/api/latestResultGate";
import { AdminBoundaryModal } from "./AdminBoundaryModal";
import { useDelayedFlag } from "@/hooks/useLoadingPrimitives";
import { EmptyState } from "@/components/ui/EmptyState";
import {
  getAoiBookmarks,
  saveAoiBookmark,
  removeAoiBookmark,
  getAoiSearchHistory,
  saveAoiSearchHistory,
  removeAoiSearchHistoryItem,
  clearAoiSearchHistory,
  mergeAoiRegions,
  resolveAdminBoundaryAuthority,
  type AoiBookmarkItem,
  type AoiHistoryItem,
  type MergeableRegionItem,
} from "@/lib/aoiBookmarksAndMerge";

export type WorkingAoiSnapshot = {
  bbox: Bbox | null;
  geometry: Record<string, unknown> | null;
  label: string;
};

type BoundaryServices = {
  search: (query: string, limit: number) => Promise<AdminBoundaryResult>;
  resolve: (candidate: AdminBoundary) => Promise<AdminBoundary | ApiError>;
};

const defaultBoundaryServices: BoundaryServices = {
  search: (query, limit) => searchAdminBoundaries(query, "", "", "", limit),
  resolve: resolveAdminBoundary,
};

function formatCandidateAdminLevel(levelRaw: string | null | undefined): string {
  const level = levelRaw?.trim() || "";
  if (!level) return "";
  if (level.toLowerCase() === "administrative" || level.toLowerCase() === "boundary") {
    return "";
  }
  return level;
}

function formatAuthorityBadgeDisplay(
  badgeZh: string,
  sourceKey: string,
  t: typeof I18nTranslateFn,
): string {
  const lower = `${sourceKey} ${badgeZh}`.toLowerCase();
  if (lower.includes("nominatim") || lower.includes("osm") || lower.includes("openstreetmap")) {
    return t("mapWorkbench.admin.authorityOsm");
  }
  if (lower.includes("datav")) {
    return t("mapWorkbench.admin.authorityDatav");
  }
  return badgeZh;
}

export function adminBoundaryFullPath(candidate: AdminBoundary): string {
  const path = (candidate.display_name || candidate.label)
    .split(",")
    .map((part) => part.trim())
    .filter(Boolean)
    .join(" · ");
  const level = formatCandidateAdminLevel(candidate.admin_level);
  return level && !path.endsWith(` · ${level}`) && path !== level ? `${path} · ${level}` : path;
}

export function AdminRegionSearch({
  workingAoi,
  workingGeometry,
  aoiLabel,
  areaText,
  onReplaceAoi,
  onPreviewGeometry,
  onOpenAdminModal,
  services = defaultBoundaryServices,
}: {
  workingAoi: Bbox | null;
  workingGeometry: Record<string, unknown> | null;
  aoiLabel: string;
  areaText: string;
  onReplaceAoi: (
    geometry: Record<string, unknown> | null,
    displayName: string,
    bbox: [number, number, number, number] | null,
  ) => Promise<void>;
  onPreviewGeometry: (geometry: Record<string, unknown> | null) => void;
  onOpenAdminModal?: () => void;
  services?: BoundaryServices;
}) {
  const { t } = useI18n();
  const displayValue = workingAoi
    ? `${aoiLabel}${areaText ? ` · ${areaText}` : ""}`
    : "";
  const [draft, setDraft] = useState<string | null>(null);
  const [candidates, setCandidates] = useState<AdminBoundary[]>([]);
  const [loading, setLoading] = useState(false);
  const [applyingKey, setApplyingKey] = useState<string | null>(null);
  const showSearchingSpinner = useDelayedFlag(loading || applyingKey !== null);
  const [error, setError] = useState(null as string | null);
  const [undo, setUndo] = useState<WorkingAoiSnapshot | null>(null);
  const [notice, setNotice] = useState<"replaced" | "restored" | "bookmarked" | null>(null);
  const [adminModalOpen, setAdminModalOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);

  // 范围收藏夹、检索历史与多区域合并篮状态
  const [bookmarksOpen, setBookmarksOpen] = useState(false);
  const [bookmarks, setBookmarks] = useState<AoiBookmarkItem[]>(() => getAoiBookmarks());
  const [historyOpen, setHistoryOpen] = useState(false);
  const [historyList, setHistoryList] = useState<AoiHistoryItem[]>(() => getAoiSearchHistory());
  const [mergeBasket, setMergeBasket] = useState<MergeableRegionItem[]>([]);

  const queryCache = useRef(new Map<string, AdminBoundaryResult>());
  const geometryCache = useRef(new Map<string, Promise<Record<string, unknown> | null>>());
  const searchGate = useMemo(createLatestResultGate, []);
  const previewGate = useMemo(createLatestResultGate, []);
  const composing = useRef(false);
  const applyVersionRef = useRef(0);
  const mounted = useRef(true);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const onPreviewGeometryRef = useRef(onPreviewGeometry);
  onPreviewGeometryRef.current = onPreviewGeometry;
  const currentSnapshot = useRef<WorkingAoiSnapshot>({ bbox: workingAoi, geometry: workingGeometry, label: aoiLabel });
  currentSnapshot.current = { bbox: workingAoi, geometry: workingGeometry, label: aoiLabel };

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      applyVersionRef.current += 1;
      if (timer.current) clearTimeout(timer.current);
      onPreviewGeometryRef.current?.(null);
    };
  }, []);

  useEffect(() => {
    if (typeof document === "undefined") return;
    const handleOutsideMouseDown = (event: MouseEvent) => {
      if (adminModalOpen) return;
      const root = rootRef.current;
      if (!root || root.contains(event.target as Node)) return;
      if (candidates.length > 0 || historyOpen || bookmarksOpen || draft !== null) {
        setCandidates([]);
        setHistoryOpen(false);
        setBookmarksOpen(false);
        setDraft(null);
        onPreviewGeometryRef.current?.(null);
      }
    };
    document.addEventListener("mousedown", handleOutsideMouseDown);
    return () => document.removeEventListener("mousedown", handleOutsideMouseDown);
  }, [adminModalOpen, bookmarksOpen, candidates.length, draft, historyOpen]);

  const runSearch = useCallback(async (value: string) => {
    const query = value.trim();
    setLoading(Boolean(query));
    setError(null);
    await searchGate.run(async (): Promise<AdminBoundaryResult> => {
      if (!query) return { ok: true, query: "", results: [] };
      const cached = queryCache.current.get(query);
      if (cached) return cached;
      try {
        const result = await services.search(query, 12);
        if (result.ok) {
          queryCache.current.set(query, result);
        }
        return result;
      } catch (reason) {
        return { ok: false, error: reason instanceof Error ? reason.message : t("mapWorkbench.admin.inlineSearchFailed"), code: "ADMIN_BOUNDARY_SEARCH_FAILED" };
      }
    }, (result) => {
      if (!mounted.current) return;
      const filtered = result.ok
        ? result.results.filter((candidate) => {
            const rawGeom = (candidate.geometry ?? candidate.geojson) as Record<string, unknown> | null | undefined;
            if (rawGeom && typeof rawGeom === "object" && rawGeom.type === "Point") {
              return false;
            }
            const idStr = String(candidate.provider_object_id ?? candidate.boundary_id ?? "").toLowerCase();
            if (idStr.startsWith("node:") || idStr.startsWith("node/")) {
              return false;
            }
            return true;
          }).slice(0, 8)
        : [];
      setCandidates(filtered);
      setError(result.ok ? null : result.error || t("mapWorkbench.admin.inlineSearchFailed"));
      setLoading(false);
    });
  }, [searchGate, services, t]);

  const scheduleSearch = useCallback((value: string, immediate = false) => {
    if (timer.current) clearTimeout(timer.current);
    if (composing.current) return;
    if (immediate) void runSearch(value);
    else timer.current = setTimeout(() => void runSearch(value), 400);
  }, [runSearch]);

  const resolveGeometry = useCallback((candidate: AdminBoundary, forceRefresh = false) => {
    const key = `${candidate.provider || ""}:${candidate.provider_object_id || candidate.boundary_id || candidate.label}`;
    if (forceRefresh) {
      geometryCache.current.delete(key);
    }
    const cached = geometryCache.current.get(key);
    if (cached) return cached;
    const request = (async () => {
      const inline = canonicalAdministrativeAoiGeometry(candidate.geometry ?? candidate.geojson);
      if (inline) return inline;
      const resolved = await services.resolve(candidate);
      if ("ok" in resolved) {
        geometryCache.current.delete(key);
        return null;
      }
      const geom = canonicalAdministrativeAoiGeometry(resolved.geometry ?? resolved.geojson);
      if (!geom) {
        geometryCache.current.delete(key);
      }
      return geom;
    })().catch(() => {
      geometryCache.current.delete(key);
      return null;
    });
    geometryCache.current.set(key, request);
    return request;
  }, [services]);

  const preview = useCallback(async (candidate: AdminBoundary) => {
    setError(null);
    await previewGate.run(() => resolveGeometry(candidate), (geometry) => {
      if (!mounted.current) return;
      onPreviewGeometry(geometry);
      if (!geometry) setError(t("mapWorkbench.admin.previewFailed"));
    });
  }, [onPreviewGeometry, previewGate, resolveGeometry, t]);

  const clearPreview = useCallback(() => {
    void previewGate.run(async () => null, (geometry) => {
      if (mounted.current) onPreviewGeometry(geometry);
    });
  }, [onPreviewGeometry, previewGate]);

  const apply = useCallback(async (candidate: AdminBoundary) => {
    setError(null);
    const candidateKey = `${candidate.provider || ""}:${candidate.provider_object_id || candidate.boundary_id || candidate.label}`;
    setApplyingKey(candidateKey);
    const applyVersion = ++applyVersionRef.current;
    try {
      let geometry = await resolveGeometry(candidate);
      if (!geometry) {
        geometry = await resolveGeometry(candidate, true);
      }
      if (!mounted.current || applyVersion !== applyVersionRef.current) return;
      if (!geometry) {
        setError(t("mapWorkbench.admin.invalidPolygonHint"));
        return;
      }
      const previous = currentSnapshot.current;
      const displayName = candidate.display_name || candidate.label;
      await onReplaceAoi(geometry, displayName, [
        candidate.bbox.west,
        candidate.bbox.south,
        candidate.bbox.east,
        candidate.bbox.north,
      ]);
      const auth = resolveAdminBoundaryAuthority(candidate);
      setHistoryList(
        saveAoiSearchHistory({
          label: displayName,
          query: draft ?? undefined,
          bbox: candidate.bbox,
          geometry,
          adminLevel: candidate.admin_level ?? undefined,
          authoritySource: auth.sourceKey,
        })
      );
      setUndo(previous);
      setNotice("replaced");
      setDraft(null);
      setCandidates([]);
      setHistoryOpen(false);
      onPreviewGeometry(null);
    } catch (reason) {
      if (mounted.current && applyVersion === applyVersionRef.current) {
        setError(reason instanceof Error ? reason.message : t("mapWorkbench.admin.replaceFailed"));
      }
    } finally {
      if (mounted.current && applyVersion === applyVersionRef.current) {
        setApplyingKey(null);
      }
    }
  }, [draft, onPreviewGeometry, onReplaceAoi, resolveGeometry, t]);

  const addCandidateToMergeBasket = useCallback(async (candidate: AdminBoundary) => {
    setError(null);
    const geometry = await resolveGeometry(candidate);
    if (!mounted.current) return;
    if (!geometry) {
      setError(t("mapWorkbench.admin.invalidBoundary"));
      return;
    }
    const id = `${candidate.provider || ""}:${candidate.provider_object_id || candidate.boundary_id || candidate.label}`;
    const label = candidate.display_name || candidate.label;
    const auth = resolveAdminBoundaryAuthority(candidate);
    setMergeBasket((prev) => {
      if (prev.some((item) => item.id === id)) return prev;
      return [
        ...prev,
        {
          id,
          label,
          bbox: candidate.bbox,
          geometry,
          authoritySource: auth.sourceKey,
        },
      ];
    });
  }, [resolveGeometry, t]);

  const handleApplyMergedRegions = useCallback(async () => {
    if (mergeBasket.length === 0) return;
    const merged = mergeAoiRegions(mergeBasket);
    if (!merged) {
      setError(t("mapWorkbench.admin.invalidBoundary"));
      return;
    }
    const previous = currentSnapshot.current;
    try {
      await onReplaceAoi(merged.geometry, merged.label, merged.tupleBbox);
      setHistoryList(
        saveAoiSearchHistory({
          label: merged.label,
          bbox: merged.bbox,
          geometry: merged.geometry,
          authoritySource: "merged_regions",
        })
      );
      setUndo(previous);
      setNotice("replaced");
      setMergeBasket([]);
      setDraft(null);
      setCandidates([]);
      setHistoryOpen(false);
      onPreviewGeometry(null);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : t("mapWorkbench.admin.replaceFailed"));
    }
  }, [mergeBasket, onPreviewGeometry, onReplaceAoi, t]);

  const handleBookmarkCurrent = useCallback(() => {
    if (!workingAoi || !workingGeometry) return;
    const updated = saveAoiBookmark({
      label: aoiLabel || t("mapWorkbench.admin.currentWorkspaceExtent"),
      bbox: workingAoi,
      geometry: workingGeometry,
    });
    setBookmarks(updated);
    setNotice("bookmarked");
  }, [aoiLabel, t, workingAoi, workingGeometry]);

  const handleApplyBookmark = useCallback(async (bm: AoiBookmarkItem) => {
    const previous = currentSnapshot.current;
    try {
      await onReplaceAoi(bm.geometry, bm.label, [
        bm.bbox.west,
        bm.bbox.south,
        bm.bbox.east,
        bm.bbox.north,
      ]);
      setUndo(previous);
      setNotice("replaced");
      setBookmarksOpen(false);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : t("mapWorkbench.admin.replaceFailed"));
    }
  }, [onReplaceAoi, t]);

  const handleApplyHistory = useCallback(async (item: AoiHistoryItem) => {
    const previous = currentSnapshot.current;
    try {
      await onReplaceAoi(item.geometry, item.label, [
        item.bbox.west,
        item.bbox.south,
        item.bbox.east,
        item.bbox.north,
      ]);
      setHistoryList(
        saveAoiSearchHistory({
          label: item.label,
          query: item.query,
          bbox: item.bbox,
          geometry: item.geometry,
          adminLevel: item.adminLevel,
          authoritySource: item.authoritySource,
        })
      );
      setUndo(previous);
      setNotice("replaced");
      setHistoryOpen(false);
      setDraft(null);
      setCandidates([]);
      onPreviewGeometry(null);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : t("mapWorkbench.admin.replaceFailed"));
    }
  }, [onPreviewGeometry, onReplaceAoi, t]);

  const restore = useCallback(async () => {
    if (!undo) return;
    setError(null);
    try {
      await onReplaceAoi(undo.geometry, undo.label, undo.bbox ? [undo.bbox.west, undo.bbox.south, undo.bbox.east, undo.bbox.north] : null);
      setUndo(null);
      setNotice("restored");
      setDraft(null);
      setCandidates([]);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : t("mapWorkbench.admin.restoreFailed"));
    }
  }, [onReplaceAoi, t, undo]);

  const topCandidateAuthority = useMemo(() => {
    if (candidates.length === 0) return null;
    return resolveAdminBoundaryAuthority(candidates[0]);
  }, [candidates]);

  const hasInlineUndo = Boolean(undo && !loading && applyingKey === null && draft === null);
  const hasInlineClear = Boolean(!loading && applyingKey === null && (workingAoi !== null || (draft !== null && draft.length > 0)));

  return (
    <div ref={rootRef} className="relative min-w-0 flex-1">
      {notice && (
        <span role="status" className="sr-only">
          {notice === "replaced"
            ? t("mapWorkbench.admin.replaced")
            : notice === "restored"
              ? t("mapWorkbench.admin.restored")
              : t("mapWorkbench.admin.bookmarkedCurrent")}
        </span>
      )}

      {/* 多区域合并篮横条 */}
      {mergeBasket.length > 0 && (
        <div
          data-testid="aoi-merge-basket"
          className="mb-2 rounded-[10px] border border-sp-line bg-sp-card p-2 text-xs space-y-1.5"
        >
          <div className="flex items-center justify-between gap-2">
            <span className="flex items-center gap-1 font-semibold text-sp-text">
              <Layers className="h-3.5 w-3.5 text-sp-accent" />
              <span>{t("mapWorkbench.admin.mergeBasketTitle", { count: mergeBasket.length })}</span>
            </span>
            <div className="flex items-center gap-1">
              <button
                type="button"
                data-testid="aoi-merge-apply-btn"
                onClick={() => void handleApplyMergedRegions()}
                className="rounded-[6px] bg-sp-accent-fill px-2 py-0.5 text-xs font-semibold text-white hover:opacity-90 cursor-pointer"
              >
                {t("mapWorkbench.admin.mergeApplyBtn")}
              </button>
              <button
                type="button"
                aria-label={t("mapWorkbench.admin.clearMergeListAria")}
                onClick={() => setMergeBasket([])}
                className="p-0.5 text-sp-text-muted hover:text-sp-text cursor-pointer"
              >
                <X className="h-3.5 w-3.5" />
              </button>
            </div>
          </div>
          <div className="flex flex-wrap gap-1">
            {mergeBasket.map((item) => (
              <span
                key={item.id}
                className="inline-flex items-center gap-1 rounded-full bg-sp-surface border border-sp-line px-2 py-0.5 text-xs text-sp-text"
              >
                <span className="truncate max-w-[120px]">{item.label}</span>
                <button
                  type="button"
                  aria-label={t("mapWorkbench.admin.removeMergeItemAria", { label: item.label })}
                  onClick={() =>
                    setMergeBasket((prev) => prev.filter((x) => x.id !== item.id))
                  }
                  className="text-sp-text-muted hover:text-sp-danger-text cursor-pointer"
                >
                  <X className="h-3 w-3" />
                </button>
              </span>
            ))}
          </div>
        </div>
      )}

      <div className="relative flex gap-1">
        <div className="relative min-w-0 flex-1">
          <button
            type="button"
            data-testid="aoi-search-icon"
            title={t("mapWorkbench.admin.openBoundaryModalAria")}
            aria-label={t("mapWorkbench.admin.openBoundaryModalAria")}
            onClick={() => {
              if (onOpenAdminModal) {
                onOpenAdminModal();
              } else {
                setAdminModalOpen(true);
              }
            }}
            className="absolute left-1 top-1/2 -translate-y-1/2 rounded-[6px] p-1 text-sp-text-secondary hover:bg-sp-hover hover:text-sp-accent cursor-pointer z-10 transition-colors"
          >
            <Search className="h-3.5 w-3.5" />
          </button>
          <input
            aria-label={t("mapWorkbench.conditions.spatialExtent")}
            placeholder={
              workingAoi
                ? t("mapWorkbench.conditions.spatialExtent")
                : t("mapWorkbench.conditions.adminSearchPlaceholder")
            }
            value={draft ?? displayValue}
            onFocus={() => {
              setDraft((value) => value ?? (workingAoi ? aoiLabel : ""));
              setNotice(null);
              setHistoryList(getAoiSearchHistory());
            }}
            onChange={(event) => {
              setDraft(event.target.value);
              if (historyOpen) setHistoryOpen(false);
              scheduleSearch(event.target.value);
            }}
            onCompositionStart={() => {
              composing.current = true;
              if (timer.current) clearTimeout(timer.current);
            }}
            onCompositionEnd={(event) => {
              composing.current = false;
              setDraft(event.currentTarget.value);
              scheduleSearch(event.currentTarget.value);
            }}
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                setDraft(null);
                setCandidates([]);
                setHistoryOpen(false);
                onPreviewGeometry(null);
                return;
              }
              if (event.key !== "Enter" || composing.current || event.nativeEvent.isComposing || event.keyCode === 229) return;
              event.preventDefault();
              scheduleSearch(event.currentTarget.value, true);
            }}
            className={cn(
              "h-9 w-full rounded-[8px] border border-sp-line bg-sp-card pl-7 text-xs leading-normal text-sp-text placeholder:text-sp-text-muted outline-none focus:border-sp-accent focus:ring-1 focus:ring-sp-accent",
              showSearchingSpinner || (hasInlineUndo !== hasInlineClear)
                ? "pr-7"
                : hasInlineUndo && hasInlineClear
                  ? "pr-[52px]"
                  : "pr-2.5",
            )}
          />
          {showSearchingSpinner ? (
            <Loader2 aria-label={t("mapWorkbench.admin.searching")} className="absolute right-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 animate-spin text-sp-accent" />
          ) : (hasInlineUndo || hasInlineClear) ? (
            <div className="absolute right-1 top-1/2 -translate-y-1/2 inline-flex items-center gap-0.5">
              {hasInlineUndo && (
                <button
                  type="button"
                  data-testid="aoi-undo-replace-btn"
                  title={`${t("mapWorkbench.admin.replaced")} · ${t("mapWorkbench.admin.undo")}`}
                  aria-label={t("mapWorkbench.admin.undo")}
                  onClick={() => void restore()}
                  className="rounded p-1 text-sp-text-muted hover:bg-sp-hover hover:text-sp-accent cursor-pointer transition-colors"
                >
                  <RotateCcw className="h-3.5 w-3.5" />
                </button>
              )}
              {hasInlineClear && (
                <button
                  type="button"
                  data-testid="aoi-search-clear"
                  title={t("mapWorkbench.admin.clearSpatialExtentAria")}
                  aria-label={t("mapWorkbench.admin.clearSpatialExtentAria")}
                  onClick={() => {
                    setDraft(null);
                    setCandidates([]);
                    setNotice(null);
                    setUndo(null);
                    setHistoryOpen(false);
                    setBookmarksOpen(false);
                    onPreviewGeometry(null);
                    if (workingAoi !== null) {
                      void onReplaceAoi(null, "", null);
                    }
                  }}
                  className="rounded p-1 text-sp-text-muted hover:bg-sp-hover hover:text-sp-text cursor-pointer transition-colors"
                >
                  <X className="h-3.5 w-3.5" />
                </button>
              )}
            </div>
          ) : null}
        </div>

        {/* 行政区检索历史按钮 */}
        <button
          type="button"
          data-testid="aoi-history-trigger"
          aria-label={t("mapWorkbench.admin.historyTriggerAria")}
          title={t("mapWorkbench.admin.historyTriggerAria")}
          onClick={() => {
            setHistoryList(getAoiSearchHistory());
            setBookmarksOpen(false);
            setHistoryOpen((prev) => !prev);
          }}
          className={cn(
            "h-9 w-[30px] shrink-0 rounded-[8px] border border-sp-line inline-flex items-center justify-center transition-colors cursor-pointer",
            historyOpen
              ? "bg-sp-selected text-sp-accent"
              : "bg-sp-card text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover"
          )}
        >
          <History className="h-3.5 w-3.5" />
        </button>

        {/* 范围收藏夹按钮 */}
        <button
          type="button"
          data-testid="aoi-bookmarks-trigger"
          aria-label={t("mapWorkbench.admin.bookmarksTriggerAria")}
          title={t("mapWorkbench.admin.bookmarksTriggerTitle")}
          onClick={() => {
            setBookmarks(getAoiBookmarks());
            setHistoryOpen(false);
            setBookmarksOpen((prev) => !prev);
          }}
          className={cn(
            "h-9 w-[30px] shrink-0 rounded-[8px] border border-sp-line inline-flex items-center justify-center transition-colors cursor-pointer",
            bookmarksOpen
              ? "bg-sp-selected text-sp-accent"
              : "bg-sp-card text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover"
          )}
        >
          <Bookmark className="h-3.5 w-3.5" />
        </button>
      </div>

      {error && <div role="alert" className="mt-1 text-xs text-sp-danger-text">{error}</div>}
      {candidates.length > 0 && (
        <div role="listbox" aria-label={t("mapWorkbench.admin.candidates")} className="absolute inset-x-0 top-full z-[1100] mt-1 max-h-80 overflow-y-auto rounded-[10px] bg-sp-surface p-1.5 shadow-overlay border border-sp-line">
          {topCandidateAuthority && (
            <div className="flex flex-wrap items-center justify-between gap-x-2 gap-y-0.5 px-2 py-1 text-xs text-sp-text-muted border-b border-sp-divider mb-0.5">
              <span className="inline-flex min-w-0 items-center gap-1">
                <ShieldCheck className="h-3 w-3 shrink-0 text-sp-accent" />
                <span className="truncate">
                  {t("mapWorkbench.admin.sourcePrefix", {
                    source: formatAuthorityBadgeDisplay(
                      topCandidateAuthority.badgeZh,
                      topCandidateAuthority.sourceKey,
                      t,
                    ),
                  })}
                </span>
              </span>
              <span className="whitespace-nowrap shrink-0">{t("mapWorkbench.admin.clickReplacePlusMerge")}</span>
            </div>
          )}
          {candidates.map((candidate) => {
            const label = adminBoundaryFullPath(candidate);
            const key = `${candidate.provider || ""}:${candidate.provider_object_id || candidate.boundary_id || candidate.label}`;
            const isApplying = applyingKey === key;
            return (
              <div key={key} className="flex items-center gap-1">
                <button
                  type="button"
                  role="option"
                  aria-selected="false"
                  disabled={applyingKey !== null}
                  title={label}
                  onMouseEnter={() => void preview(candidate)}
                  onFocus={() => void preview(candidate)}
                  onMouseLeave={clearPreview}
                  onBlur={clearPreview}
                  onClick={() => void apply(candidate)}
                  className="min-w-0 flex-1 h-8 rounded-[8px] px-2.5 text-left text-xs text-sp-text hover:bg-sp-hover focus:bg-sp-hover focus:outline-none truncate cursor-pointer disabled:opacity-60 inline-flex items-center gap-1.5"
                >
                  {isApplying && <Loader2 className="h-3 w-3 animate-spin text-sp-accent shrink-0" />}
                  <span className="truncate">
                    {isApplying ? t("mapWorkbench.admin.loadingBoundarySuffix", { label }) : label}
                  </span>
                </button>
                <button
                  type="button"
                  aria-label={t("mapWorkbench.admin.addMergeAria", { label })}
                  title={t("mapWorkbench.admin.addMergeBasketTitle")}
                  disabled={applyingKey !== null}
                  onClick={() => void addCandidateToMergeBasket(candidate)}
                  className="shrink-0 h-8 w-8 rounded-[8px] inline-flex items-center justify-center text-xs text-sp-accent hover:bg-sp-hover cursor-pointer disabled:opacity-50"
                >
                  +
                </button>
              </div>
            );
          })}

          {/* 最近检索历史位于候选列表下方 */}
          {historyList.length > 0 && (
            <div
              data-testid="aoi-candidates-history-section"
              className="mt-1.5 border-t border-sp-divider pt-1.5 space-y-1"
            >
              <div className="flex items-center justify-between px-2 py-0.5 text-xs text-sp-text-muted">
                <span className="inline-flex items-center gap-1 font-medium">
                  <History className="h-3 w-3 text-sp-accent" />
                  <span>{t("mapWorkbench.admin.recentHistoryTitle")}</span>
                </span>
                <button
                  type="button"
                  onClick={() => setHistoryList(clearAoiSearchHistory())}
                  className="rounded-[6px] px-1.5 py-0.5 text-xs text-sp-text-muted hover:text-sp-danger-text hover:bg-sp-hover cursor-pointer"
                >
                  {t("mapWorkbench.admin.clearHistoryBtn")}
                </button>
              </div>
              {historyList.slice(0, 4).map((item) => (
                <div
                  key={item.id}
                  className="flex items-center justify-between gap-2 rounded-[8px] px-2.5 py-1.5 hover:bg-sp-hover"
                >
                  <button
                    type="button"
                    onMouseEnter={() => onPreviewGeometry(item.geometry)}
                    onMouseLeave={clearPreview}
                    onClick={() => void handleApplyHistory(item)}
                    className="min-w-0 flex-1 text-left text-xs text-sp-text truncate cursor-pointer"
                    title={item.label}
                  >
                    <span className="font-medium">{item.label}</span>
                    {item.adminLevel && (
                      <span className="ml-1.5 text-sp-text-muted">· {item.adminLevel}</span>
                    )}
                  </button>
                  <button
                    type="button"
                    aria-label={t("mapWorkbench.admin.deleteHistoryAria", { label: item.label })}
                    onClick={() => setHistoryList(removeAoiSearchHistoryItem(item.id))}
                    className="rounded-[6px] p-1 text-sp-text-muted hover:text-sp-danger-text hover:bg-sp-hover cursor-pointer"
                  >
                    <Trash2 className="h-3 w-3" />
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      {/* 行政区检索历史弹出面板（未展开候选列表时独立展示） */}
      {historyOpen && candidates.length === 0 && (
        <div
          data-testid="aoi-history-popover"
          className="sp-popover-blur-in origin-top-right absolute inset-x-0 top-full z-[1100] mt-1 rounded-[10px] border border-sp-line bg-sp-surface p-2.5 shadow-overlay text-xs space-y-2"
        >
          <div className="flex items-center justify-between gap-2 pb-1.5 border-b border-sp-divider">
            <span className="flex items-center gap-1.5 font-semibold text-sp-text">
              <History className="h-3.5 w-3.5 text-sp-accent" />
              <span>{t("mapWorkbench.admin.recentHistoryTitle")}</span>
            </span>
            <div className="flex items-center gap-1.5">
              {historyList.length > 0 && (
                <button
                  type="button"
                  data-testid="aoi-history-clear-btn"
                  onClick={() => setHistoryList(clearAoiSearchHistory())}
                  className="rounded-[6px] px-1.5 py-0.5 text-xs text-sp-text-muted hover:text-sp-danger-text hover:bg-sp-hover cursor-pointer"
                >
                  {t("mapWorkbench.admin.clearHistoryBtn")}
                </button>
              )}
              <button
                type="button"
                aria-label={t("mapWorkbench.admin.closeHistoryAria")}
                onClick={() => setHistoryOpen(false)}
                className="rounded-[6px] p-0.5 text-sp-text-muted hover:text-sp-text hover:bg-sp-hover cursor-pointer"
              >
                <X className="h-3.5 w-3.5" />
              </button>
            </div>
          </div>

          {historyList.length === 0 ? (
            <EmptyState
              kind="first-use"
              size="row"
              title={t("mapWorkbench.admin.emptyHistoryTitle")}
            />
          ) : (
            <div className="max-h-52 overflow-y-auto space-y-1">
              {historyList.map((item) => (
                <div
                  key={item.id}
                  data-testid="aoi-history-item"
                  className="flex items-center justify-between gap-2 rounded-[8px] px-2 py-1.5 hover:bg-sp-hover"
                >
                  <button
                    type="button"
                    onMouseEnter={() => onPreviewGeometry(item.geometry)}
                    onMouseLeave={clearPreview}
                    onClick={() => void handleApplyHistory(item)}
                    className="min-w-0 flex-1 text-left text-xs text-sp-text truncate cursor-pointer"
                    title={item.label}
                  >
                    <span className="font-medium">{item.label}</span>
                    {item.adminLevel && (
                      <span className="ml-1.5 text-sp-text-muted">· {item.adminLevel}</span>
                    )}
                  </button>
                  <div className="flex items-center gap-1 shrink-0">
                    <button
                      type="button"
                      title={t("mapWorkbench.admin.addMultiMergeTitle")}
                      aria-label={t("mapWorkbench.admin.mergeHistoryAria", { label: item.label })}
                      onClick={() => {
                        setMergeBasket((prev) =>
                          prev.some((x) => x.id === item.id)
                            ? prev
                            : [
                                ...prev,
                                {
                                  id: item.id,
                                  label: item.label,
                                  bbox: item.bbox,
                                  geometry: item.geometry,
                                  authoritySource: item.authoritySource,
                                },
                              ]
                        );
                      }}
                      className="rounded-[6px] px-1.5 py-0.5 text-xs text-sp-accent hover:bg-sp-card cursor-pointer"
                    >
                      {t("mapWorkbench.admin.plusMergeBtn")}
                    </button>
                    <button
                      type="button"
                      aria-label={t("mapWorkbench.admin.deleteHistoryAria", { label: item.label })}
                      onClick={() => setHistoryList(removeAoiSearchHistoryItem(item.id))}
                      className="rounded-[6px] p-1 text-sp-text-muted hover:text-sp-danger-text hover:bg-sp-hover cursor-pointer"
                    >
                      <Trash2 className="h-3 w-3" />
                    </button>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      {/* 范围收藏夹弹出面板 */}
      {bookmarksOpen && (
        <div
          data-testid="aoi-bookmarks-popover"
          className="sp-popover-blur-in origin-top-right absolute inset-x-0 top-full z-[1100] mt-1 rounded-[10px] border border-sp-line bg-sp-surface p-2.5 shadow-overlay text-xs space-y-2"
        >
          <div className="flex items-center justify-between gap-2 pb-1.5 border-b border-sp-divider">
            <span className="font-semibold text-sp-text">{t("mapWorkbench.admin.bookmarksTriggerAria")}</span>
            <div className="flex items-center gap-1.5">
              {workingAoi && workingGeometry && (
                <button
                  type="button"
                  data-testid="aoi-bookmark-save-current"
                  onClick={handleBookmarkCurrent}
                  className="inline-flex items-center gap-1 rounded-[6px] bg-sp-card border border-sp-line px-2 py-0.5 text-xs text-sp-accent hover:bg-sp-hover cursor-pointer"
                >
                  <Plus className="h-3 w-3" />
                  <span>{t("mapWorkbench.admin.saveCurrentBookmarkBtn")}</span>
                </button>
              )}
              <button
                type="button"
                aria-label={t("mapWorkbench.admin.closeBookmarksAria")}
                onClick={() => setBookmarksOpen(false)}
                className="rounded-[6px] p-0.5 text-sp-text-muted hover:text-sp-text hover:bg-sp-hover cursor-pointer"
              >
                <X className="h-3.5 w-3.5" />
              </button>
            </div>
          </div>

          {bookmarks.length === 0 ? (
            <EmptyState
              kind="first-use"
              size="row"
              title={t("mapWorkbench.admin.emptyBookmarksTitle")}
            />
          ) : (
            <div className="max-h-52 overflow-y-auto space-y-1">
              {bookmarks.map((bm) => (
                <div
                  key={bm.id}
                  className="flex items-center justify-between gap-2 rounded-[8px] px-2 py-1.5 hover:bg-sp-hover"
                >
                  <button
                    type="button"
                    onClick={() => void handleApplyBookmark(bm)}
                    className="min-w-0 flex-1 text-left text-xs text-sp-text truncate cursor-pointer"
                    title={bm.label}
                  >
                    {bm.label}
                  </button>
                  <div className="flex items-center gap-1 shrink-0">
                    <button
                      type="button"
                      title={t("mapWorkbench.admin.addMultiMergeTitle")}
                      aria-label={t("mapWorkbench.admin.mergeBookmarkAria", { label: bm.label })}
                      onClick={() => {
                        setMergeBasket((prev) =>
                          prev.some((x) => x.id === bm.id)
                            ? prev
                            : [
                                ...prev,
                                {
                                  id: bm.id,
                                  label: bm.label,
                                  bbox: bm.bbox,
                                  geometry: bm.geometry,
                                  authoritySource: bm.authoritySource,
                                },
                              ]
                        );
                      }}
                      className="rounded-[6px] px-1.5 py-0.5 text-xs text-sp-accent hover:bg-sp-card cursor-pointer"
                    >
                      {t("mapWorkbench.admin.plusMergeBtn")}
                    </button>
                    <button
                      type="button"
                      aria-label={t("mapWorkbench.admin.deleteBookmarkAria", { label: bm.label })}
                      onClick={() => setBookmarks(removeAoiBookmark(bm.id))}
                      className="rounded-[6px] p-1 text-sp-text-muted hover:text-sp-danger-text hover:bg-sp-hover cursor-pointer"
                    >
                      <Trash2 className="h-3 w-3" />
                    </button>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      {adminModalOpen && (
        <AdminBoundaryModal
          open={adminModalOpen}
          onClose={() => setAdminModalOpen(false)}
          services={{
            search: (q) => services.search(q, 10),
            resolve: services.resolve,
          }}
          onSelectBoundary={async (geom, displayName, bboxTuple) => {
            const previous = currentSnapshot.current;
            await onReplaceAoi(geom, displayName, bboxTuple);
            setHistoryList(getAoiSearchHistory());
            setUndo(previous);
            setNotice("replaced");
            setDraft(null);
            setCandidates([]);
            setHistoryOpen(false);
            onPreviewGeometry(null);
            setAdminModalOpen(false);
          }}
        />
      )}
    </div>
  );
}
