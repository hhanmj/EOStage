import React, { useState, useCallback, useEffect, useMemo, useRef } from "react";
import { Search, MapPin, X, Loader2, AlertCircle, History, Trash2 } from "lucide-react";
import { ModalOverlay } from "@/components/ui/ModalOverlay";
import { useI18n } from "@/i18n";
import { useDelayedFlag } from "@/hooks/useLoadingPrimitives";
import { Skeleton } from "@/components/ui/Skeleton";
import { EmptyState } from "@/components/ui/EmptyState";
import {
  searchAdminBoundaries,
  resolveAdminBoundary,
  isTauriDesktopHost,
  type AdminBoundary,
  type AdminBoundaryResult,
  type ApiError,
} from "../../lib/bridge.ts";
import { canonicalAdministrativeAoiGeometry } from "../../lib/administrativeAoiGeometry.ts";
import { createLatestResultGate } from "../../shell/api/latestResultGate.ts";
import {
  getAoiSearchHistory,
  saveAoiSearchHistory,
  removeAoiSearchHistoryItem,
  clearAoiSearchHistory,
  resolveAdminBoundaryAuthority,
  type AoiHistoryItem,
} from "../../lib/aoiBookmarksAndMerge.ts";
import { formatSignedLat, formatSignedLon } from "../../lib/geoCoords.ts";

export interface AdminBoundaryModalServices {
  search?: (query: string) => Promise<AdminBoundaryResult>;
  resolve?: (candidate: AdminBoundary) => Promise<AdminBoundary | ApiError>;
}

export interface AdminBoundaryModalProps {
  open: boolean;
  onClose: () => void;
  onSelectBoundary: (boundaryGeojson: Record<string, unknown>, displayName: string, bbox: [number, number, number, number]) => Promise<void>;
  services?: AdminBoundaryModalServices;
}

export const AdminBoundaryModal: React.FC<AdminBoundaryModalProps> = ({
  open,
  onClose,
  onSelectBoundary,
  services,
}) => {
  const { t } = useI18n();
  const [query, setQuery] = useState("");
  const [loading, setLoading] = useState(false);
  const [candidates, setCandidates] = useState<AdminBoundary[]>([]);
  const [selectedCandidate, setSelectedCandidate] = useState<AdminBoundary | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const showSearchingSpinner = useDelayedFlag(loading);
  const showSubmittingSpinner = useDelayedFlag(submitting);
  const [historyList, setHistoryList] = useState<AoiHistoryItem[]>(() => getAoiSearchHistory());

  const composingRef = useRef(false);
  const debounceTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const queryCacheRef = useRef(new Map<string, AdminBoundaryResult>());
  const searchGate = useMemo(createLatestResultGate, []);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      if (debounceTimerRef.current) {
        clearTimeout(debounceTimerRef.current);
      }
    };
  }, []);

  useEffect(() => {
    if (open) {
      setHistoryList(getAoiSearchHistory());
      setErrorMessage(null);
    } else if (debounceTimerRef.current) {
      clearTimeout(debounceTimerRef.current);
    }
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if ((e.key === "Escape" || e.code === "Escape" || e.keyCode === 27) && !submitting && !composingRef.current) {
        e.preventDefault();
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("keyup", onKey, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("keyup", onKey, true);
    };
  }, [open, submitting, onClose]);

  const runSearch = useCallback(
    async (rawQuery: string) => {
      const q = rawQuery.trim();
      setSelectedCandidate(null);

      if (!q) {
        await searchGate.run(
          async () => ({ ok: true as const, query: "", results: [] }),
          () => {
            if (!mountedRef.current) return;
            setCandidates([]);
            setErrorMessage(null);
            setLoading(false);
          }
        );
        return;
      }

      setErrorMessage(null);
      setLoading(true);

      await searchGate.run(
        async (): Promise<AdminBoundaryResult> => {
          const cached = queryCacheRef.current.get(q);
          if (cached) return cached;

          if (services?.search) {
            const customRes = await services.search(q);
            if (customRes.ok) {
              queryCacheRef.current.set(q, customRes);
            }
            return customRes;
          }

          if (!isTauriDesktopHost()) {
            return {
              ok: false,
              error: t("mapWorkbench.admin.desktopOnly"),
              code: "ADMIN_BOUNDARY_DESKTOP_REQUIRED",
            };
          }

          const res = await searchAdminBoundaries(q);
          if (res.ok) {
            queryCacheRef.current.set(q, res);
          }
          return res;
        },
        (res) => {
          if (!mountedRef.current) return;
          if (res.ok) {
            const list = res.results || [];
            setCandidates(list);
            if (list.length === 0) {
              setErrorMessage(t("mapWorkbench.admin.noMatches"));
            } else {
              setErrorMessage(null);
            }
          } else {
            setCandidates([]);
            setErrorMessage(res.error || t("mapWorkbench.admin.searchFailed"));
          }
          setLoading(false);
        }
      );
    },
    [searchGate, services, t]
  );

  const scheduleSearch = useCallback(
    (nextValue: string, immediate = false) => {
      if (debounceTimerRef.current) {
        clearTimeout(debounceTimerRef.current);
        debounceTimerRef.current = null;
      }
      if (composingRef.current) return;
      if (immediate || !nextValue.trim()) {
        void runSearch(nextValue);
      } else {
        debounceTimerRef.current = setTimeout(() => {
          void runSearch(nextValue);
        }, 300);
      }
    },
    [runSearch]
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLInputElement>) => {
      if ((e.key === "Escape" || e.code === "Escape" || e.keyCode === 27) && !submitting) {
        e.preventDefault();
        e.stopPropagation();
        onClose();
        return;
      }
      // 视觉基线 4b.1: 中文输入法选字时不误发提交
      if (composingRef.current || e.nativeEvent.isComposing || e.keyCode === 229) return;
      if (e.key === "Enter") {
        e.preventDefault();
        scheduleSearch(e.currentTarget.value, true);
      }
    },
    [scheduleSearch, submitting, onClose]
  );

  const handleConfirm = useCallback(async () => {
    if (!selectedCandidate) return;
    setSubmitting(true);
    setErrorMessage(null);

    try {
      let resolved = selectedCandidate;
      const rawGeometry = resolved.geometry ?? resolved.geojson;
      let canonicalGeometry = canonicalAdministrativeAoiGeometry(rawGeometry) as Record<string, unknown> | null;

      if (!canonicalGeometry) {
        const resolver = services?.resolve ?? resolveAdminBoundary;
        const resolveRes = await resolver(selectedCandidate);
        if ("ok" in resolveRes && !resolveRes.ok) {
          setErrorMessage(resolveRes.error || t("mapWorkbench.admin.resolveFailed"));
          return;
        }
        resolved = resolveRes as AdminBoundary;
        const freshRaw = resolved.geometry ?? resolved.geojson;
        canonicalGeometry = canonicalAdministrativeAoiGeometry(freshRaw) as Record<string, unknown> | null;
      }

      if (!canonicalGeometry) {
        setErrorMessage(t("mapWorkbench.admin.missingPolygon"));
        return;
      }

      const displayName = resolved.display_name ?? resolved.label;
      const bboxTuple: [number, number, number, number] = [
        resolved.bbox.west,
        resolved.bbox.south,
        resolved.bbox.east,
        resolved.bbox.north,
      ];

      const auth = resolveAdminBoundaryAuthority(resolved);
      setHistoryList(
        saveAoiSearchHistory({
          label: displayName,
          query: query.trim() || undefined,
          bbox: resolved.bbox,
          geometry: canonicalGeometry,
          adminLevel: resolved.admin_level ?? undefined,
          authoritySource: auth.sourceKey,
        })
      );

      await onSelectBoundary(canonicalGeometry, displayName, bboxTuple);
      onClose();
    } catch (err: any) {
      setErrorMessage(err?.message || t("mapWorkbench.admin.applyFailed"));
    } finally {
      setSubmitting(false);
    }
  }, [selectedCandidate, services, query, onSelectBoundary, onClose, t]);

  const handleApplyHistoryItem = useCallback(
    async (item: AoiHistoryItem) => {
      setSubmitting(true);
      setErrorMessage(null);
      try {
        const canonicalGeometry =
          (canonicalAdministrativeAoiGeometry(item.geometry) as Record<string, unknown> | null) ??
          item.geometry;
        if (!canonicalGeometry) {
          setErrorMessage(t("mapWorkbench.admin.missingPolygon"));
          return;
        }
        const bboxTuple: [number, number, number, number] = [
          item.bbox.west,
          item.bbox.south,
          item.bbox.east,
          item.bbox.north,
        ];
        setHistoryList(
          saveAoiSearchHistory({
            label: item.label,
            query: item.query,
            bbox: item.bbox,
            geometry: canonicalGeometry,
            adminLevel: item.adminLevel,
            authoritySource: item.authoritySource,
          })
        );
        await onSelectBoundary(canonicalGeometry, item.label, bboxTuple);
        onClose();
      } catch (err: any) {
        setErrorMessage(err?.message || t("mapWorkbench.admin.applyFailed"));
      } finally {
        setSubmitting(false);
      }
    },
    [onSelectBoundary, onClose, t]
  );

  if (!open) return null;

  return (
    <ModalOverlay
      role="dialog"
      aria-modal={true}
      aria-labelledby="admin-boundary-modal-title"
      className="z-[1200] animate-in fade-in duration-fast"
      closeOnBackdropClick={false}
      onClose={() => {
        if (!submitting) onClose();
      }}
    >
      <div
        data-testid="admin-boundary-modal"
        className="relative w-full max-w-lg rounded-[14px] border border-sp-line bg-sp-surface shadow-modal p-5 space-y-4 overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        {/* 标题栏 */}
        <div className="flex items-center justify-between border-b border-sp-divider pb-3">
          <div className="flex items-center gap-2">
            <MapPin className="h-4 w-4 text-sp-accent" />
            <h2 id="admin-boundary-modal-title" className="text-sm font-semibold text-sp-text">
              {t("mapWorkbench.admin.title")}
            </h2>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label={t("mapWorkbench.admin.close")}
            className="rounded-[8px] p-1 text-sp-text-secondary hover:bg-sp-hover transition-colors cursor-pointer"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        {/* 说明与自动搜索输入框（输入即自动检索，无需点击按钮） */}
        <div className="space-y-2">
          <div className="text-xs text-sp-text-muted">
            {t("mapWorkbench.admin.description")}
          </div>
          <div className="relative">
            <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-sp-text-disabled" />
            <input
              type="text"
              data-testid="admin-modal-search-input"
              aria-label={t("mapWorkbench.admin.title")}
              value={query}
              onChange={(e) => {
                const next = e.target.value;
                setQuery(next);
                scheduleSearch(next);
              }}
              onCompositionStart={() => {
                composingRef.current = true;
                if (debounceTimerRef.current) {
                  clearTimeout(debounceTimerRef.current);
                  debounceTimerRef.current = null;
                }
              }}
              onCompositionEnd={(e) => {
                composingRef.current = false;
                const next = e.currentTarget.value;
                setQuery(next);
                scheduleSearch(next);
              }}
              onKeyDown={handleKeyDown}
              placeholder={t("mapWorkbench.admin.placeholder")}
              className="h-9 w-full rounded-[8px] border border-sp-line bg-sp-surface pl-8 pr-8 text-xs text-sp-text placeholder:text-sp-text-disabled focus:outline-none focus:ring-1 focus:ring-sp-accent"
            />
            {showSearchingSpinner ? (
              <span className="pointer-events-none absolute right-2.5 top-1/2 -translate-y-1/2 flex items-center justify-center">
                <Loader2
                  data-testid="admin-modal-searching"
                  aria-label={t("mapWorkbench.admin.searching")}
                  className="h-3.5 w-3.5 animate-spin text-sp-accent"
                />
              </span>
            ) : !loading && query ? (
              <button
                type="button"
                aria-label={t("mapWorkbench.admin.clearInputAria")}
                onClick={() => {
                  setQuery("");
                  scheduleSearch("", true);
                }}
                className="absolute right-2 top-1/2 -translate-y-1/2 rounded-[6px] p-0.5 text-sp-text-muted hover:text-sp-text hover:bg-sp-hover cursor-pointer"
              >
                <X className="h-3.5 w-3.5" />
              </button>
            ) : null}
          </div>
        </div>

        {/* 错误提示条 */}
        {errorMessage && (
          <div className="flex items-start gap-2 rounded-[10px] bg-sp-danger-bg/40 border border-sp-danger-text/20 p-2.5 text-xs text-sp-danger-text">
            <AlertCircle className="h-4 w-4 shrink-0 mt-0.5" />
            <span>{errorMessage}</span>
          </div>
        )}

        {/* 匹配候选结果列表（位于上方） */}
        <div data-testid="admin-modal-candidates" className="space-y-1.5">
          <div className="flex items-center justify-between text-xs font-medium text-sp-text-secondary">
            <span>
              {t("mapWorkbench.admin.candidateCount")} {candidates.length > 0 && `(${candidates.length})`}
            </span>
            {showSearchingSpinner && <span className="text-sp-accent">{t("mapWorkbench.admin.loading")}</span>}
          </div>
          <div className="min-h-[120px] max-h-44 overflow-y-auto rounded-[10px] border border-sp-line bg-sp-card p-1.5 space-y-1">
            {candidates.length === 0 ? (
              showSearchingSpinner ? (
                <div className="space-y-1.5 py-1" aria-busy="true" aria-label={t("mapWorkbench.admin.loading")}>
                  <Skeleton className="h-9 w-full rounded-[8px]" />
                  <Skeleton className="h-9 w-full rounded-[8px]" />
                </div>
              ) : loading ? (
                <div className="h-[78px]" aria-busy="true" />
              ) : (
                <div className="py-5 text-center text-xs text-sp-text-disabled">
                  {t("mapWorkbench.admin.empty")}
                </div>
              )
            ) : (
              candidates.map((c) => {
                const isSelected =
                  selectedCandidate?.provider_object_id === c.provider_object_id &&
                  selectedCandidate?.provider === c.provider;
                return (
                  <button
                    key={`${c.provider}-${c.provider_object_id || c.label}`}
                    type="button"
                    data-testid="admin-modal-candidate-item"
                    onClick={() => setSelectedCandidate(c)}
                    className={`w-full flex items-center justify-between rounded-[8px] p-2 text-left text-xs transition-colors duration-fast cursor-pointer ${
                      isSelected
                        ? "bg-sp-selected shadow-card text-sp-accent-strong ring-1 ring-sp-accent"
                        : "hover:bg-sp-hover text-sp-text"
                    }`}
                  >
                    <div className="min-w-0 flex-1">
                      <div className="font-medium truncate">{c.display_name || c.label}</div>
                      <div className="text-xs text-sp-text-muted truncate">
                        {t("mapWorkbench.admin.extent")} {formatSignedLon(c.bbox.west, 2)} ~ {formatSignedLon(c.bbox.east, 2)} · {formatSignedLat(c.bbox.south, 2)} ~ {formatSignedLat(c.bbox.north, 2)}
                      </div>
                    </div>
                    {c.admin_level && c.admin_level.trim().toLowerCase() !== "administrative" && (
                      <span className="shrink-0 ml-2 rounded-full px-2 py-0.5 text-xs bg-sp-surface border border-sp-line text-sp-text-secondary">
                        {c.admin_level}
                      </span>
                    )}
                  </button>
                );
              })
            )}
          </div>
        </div>

        {/* 最近检索与使用历史（位于候选结果列表下方） */}
        <div data-testid="admin-modal-history" className="space-y-1.5">
          <div className="flex items-center justify-between">
            <span className="flex items-center gap-1.5 text-xs font-medium text-sp-text-secondary">
              <History className="h-3.5 w-3.5 text-sp-accent" />
              <span>{t("mapWorkbench.admin.recentHistoryTitle")} {historyList.length > 0 && `(${historyList.length})`}</span>
            </span>
            {historyList.length > 0 && (
              <button
                type="button"
                data-testid="admin-modal-history-clear"
                onClick={() => setHistoryList(clearAoiSearchHistory())}
                className="rounded-[6px] px-1.5 py-0.5 text-xs text-sp-text-muted hover:text-sp-danger-text hover:bg-sp-hover cursor-pointer"
              >
                {t("mapWorkbench.admin.clearHistoryBtn")}
              </button>
            )}
          </div>
          {historyList.length === 0 ? (
            <EmptyState
              kind="first-use"
              size="row"
              title={t("mapWorkbench.admin.emptyHistoryTitle")}
              className="rounded-[10px] border border-sp-line bg-sp-card px-3"
            />
          ) : (
            <div className="max-h-32 overflow-y-auto rounded-[10px] border border-sp-line bg-sp-card p-1.5 space-y-1">
              {historyList.map((item) => {
                const isSelected =
                  selectedCandidate?.provider_object_id === item.id &&
                  selectedCandidate?.provider === "history";
                return (
                  <div
                    key={item.id}
                    data-testid="admin-modal-history-item"
                    className={`flex items-center justify-between gap-2 rounded-[8px] px-2.5 py-1.5 text-xs transition-colors ${
                      isSelected
                        ? "bg-sp-selected text-sp-accent-strong ring-1 ring-sp-accent"
                        : "hover:bg-sp-hover text-sp-text"
                    }`}
                  >
                    <button
                      type="button"
                      onClick={() => {
                        setSelectedCandidate({
                          boundary_id: item.id,
                          provider: "history",
                          provider_object_id: item.id,
                          label: item.label,
                          display_name: item.label,
                          admin_level: item.adminLevel || t("mapWorkbench.admin.historyFallbackLevel"),
                          bbox: item.bbox,
                          geometry: item.geometry,
                        });
                      }}
                      className="min-w-0 flex-1 text-left cursor-pointer"
                    >
                      <div className="font-medium truncate">{item.label}</div>
                      <div className="text-xs text-sp-text-muted truncate">
                        {t("mapWorkbench.admin.extent")} {formatSignedLon(item.bbox.west, 2)} ~ {formatSignedLon(item.bbox.east, 2)} · {formatSignedLat(item.bbox.south, 2)} ~ {formatSignedLat(item.bbox.north, 2)}
                      </div>
                    </button>
                    <div className="flex items-center gap-1 shrink-0">
                      <button
                        type="button"
                        aria-label={t("mapWorkbench.admin.applyDirectlyAria", { label: item.label })}
                        onClick={() => void handleApplyHistoryItem(item)}
                        disabled={submitting}
                        className="rounded-[6px] bg-sp-surface border border-sp-line px-2 py-0.5 text-xs text-sp-accent hover:bg-sp-hover cursor-pointer"
                      >
                        {t("mapWorkbench.admin.applyDirectlyBtn")}
                      </button>
                      <button
                        type="button"
                        aria-label={t("mapWorkbench.admin.deleteHistoryAria", { label: item.label })}
                        onClick={() => setHistoryList(removeAoiSearchHistoryItem(item.id))}
                        className="rounded-[6px] p-1 text-sp-text-muted hover:text-sp-danger-text hover:bg-sp-hover cursor-pointer"
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </button>
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </div>

        {/* 底部按钮栏 */}
        <div className="flex items-center justify-end gap-2 border-t border-sp-divider pt-3">
          <button
            type="button"
            onClick={onClose}
            className="cred-action-btn rounded-[8px] h-8 px-3 text-xs cursor-pointer"
          >
            {t("mapWorkbench.admin.cancel")}
          </button>
          <button
            type="button"
            aria-busy={submitting}
            onClick={() => void handleConfirm()}
            disabled={!selectedCandidate || submitting}
            className="rounded-[8px] bg-sp-accent-fill text-white h-8 min-w-[96px] px-4 text-xs font-medium transition-colors hover:opacity-90 disabled:opacity-40 disabled:cursor-not-allowed flex items-center justify-center gap-1.5 cursor-pointer"
          >
            {showSubmittingSpinner && <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin" />}
            <span>{t("mapWorkbench.admin.useAsAoi")}</span>
          </button>
        </div>
      </div>
    </ModalOverlay>
  );
};
