/**
 * CompanionDownloadPopover.tsx
 * 地图工作台配套下载弹层
 * 精密轨道（Sentinel-1）、30m Copernicus 参考 DEM 与 GACOS 大气延迟均已真实接通；
 * 支持下载主影像时自动附带，也支持点击「仅下载已选配套」独立补齐。
 */

import React, { useEffect, useRef } from "react";
import { Check, Download, Loader2, X } from "lucide-react";
import { useI18n } from "@/i18n";
import type { SceneRow, Bbox } from "../../lib/bridge";
import {
  type AuxiliaryExtentMode,
  type CompanionState,
  isSentinel1Resource,
  isSarCompanionResource,
  calculateOrbitMatch,
  resolveAuxiliaryExtent,
  estimateDemTiles,
  estimateGacosRequests,
  setSavedCompanionOrbitPreference,
  setSavedCompanionDemPreference,
  setSavedCompanionGacosPreference,
  setSavedCompanionExtentMode,
} from "../../lib/companionDownloadLogic";

export interface CompanionDownloadPopoverProps {
  isOpen: boolean;
  onClose: () => void;
  activeResourceId: string;
  selectedScenes: SceneRow[];
  workingAoiBbox?: Bbox | number[] | null;
  companionState: CompanionState;
  onCompanionStateChange: (nextState: CompanionState) => void;
  onTriggerStandaloneCompanion?: () => void | Promise<void>;
  standaloneBusy?: boolean;
  anchorClassName?: string;
}

export function CompanionDownloadPopover({
  isOpen,
  onClose,
  activeResourceId,
  selectedScenes,
  workingAoiBbox,
  companionState,
  onCompanionStateChange,
  onTriggerStandaloneCompanion,
  standaloneBusy = false,
}: CompanionDownloadPopoverProps) {
  const { t } = useI18n();
  const popoverRef = useRef<HTMLDivElement | null>(null);

  // Esc 键关闭
  useEffect(() => {
    if (!isOpen) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [isOpen, onClose]);

  // 点击外部关闭
  useEffect(() => {
    if (!isOpen) return;
    const handlePointerDown = (e: MouseEvent) => {
      if (popoverRef.current && !popoverRef.current.contains(e.target as Node)) {
        onClose();
      }
    };
    window.addEventListener("mousedown", handlePointerDown);
    return () => window.removeEventListener("mousedown", handlePointerDown);
  }, [isOpen, onClose]);

  if (!isOpen) return null;

  const isSentinel1 = isSentinel1Resource(activeResourceId);
  const isSar = isSarCompanionResource(activeResourceId);
  const orbitMatch = calculateOrbitMatch(selectedScenes, activeResourceId);
  const effectiveExtent = resolveAuxiliaryExtent(
    companionState.extentMode,
    workingAoiBbox ?? null,
    selectedScenes
  );
  const demEstimate = estimateDemTiles(effectiveExtent);
  const gacosEstimate = estimateGacosRequests(selectedScenes, effectiveExtent);

  const handleToggleOrbit = () => {
    if (!isSentinel1) return;
    const nextVal = !companionState.preciseOrbit;
    setSavedCompanionOrbitPreference(activeResourceId, nextVal);
    onCompanionStateChange({
      ...companionState,
      preciseOrbit: nextVal,
    });
  };

  const handleToggleDem = () => {
    if (!isSar) return;
    const nextVal = !companionState.dem;
    setSavedCompanionDemPreference(activeResourceId, nextVal);
    onCompanionStateChange({
      ...companionState,
      dem: nextVal,
    });
  };

  const handleToggleGacos = () => {
    if (!isSar) return;
    const nextVal = !companionState.gacos;
    setSavedCompanionGacosPreference(activeResourceId, nextVal);
    onCompanionStateChange({
      ...companionState,
      gacos: nextVal,
    });
  };

  const handleChangeExtentMode = (mode: AuxiliaryExtentMode) => {
    setSavedCompanionExtentMode(activeResourceId, mode);
    onCompanionStateChange({
      ...companionState,
      extentMode: mode,
    });
  };

  const hasActiveOrbit = isSentinel1 && companionState.preciseOrbit;
  const hasActiveDem = isSar && companionState.dem;
  const hasActiveGacos = isSar && companionState.gacos;
  const canRunStandalone =
    !standaloneBusy &&
    Boolean(onTriggerStandaloneCompanion) &&
    ((hasActiveDem && Boolean(effectiveExtent)) ||
      ((hasActiveOrbit || hasActiveGacos) && selectedScenes.length > 0));

  return (
    <div
      ref={popoverRef}
      role="dialog"
      aria-label={t("panels.companionDownload.title")}
      data-testid="companion-download-popover"
      className="absolute bottom-12 right-0 w-[420px] max-w-[calc(100vw-32px)] bg-sp-card border border-sp-line rounded-sp-card shadow-sp-overlay p-3 z-50 flex flex-col gap-2.5"
    >
      {/* 头部 */}
      <div className="flex items-center justify-between pb-2 border-b border-sp-line">
        <div className="flex items-baseline gap-2">
          <span className="text-xs font-semibold text-sp-text">
            {t("panels.companionDownload.title")}
          </span>
          <span className="text-xs text-sp-text-muted">
            {t("panels.companionDownload.autoMatchNote", { count: selectedScenes.length })}
          </span>
        </div>
        <button
          type="button"
          onClick={onClose}
          aria-label={t("panels.catalogModal.close")}
          className="text-sp-text-muted hover:text-sp-text p-1 rounded-sp-sm hover:bg-sp-hover cursor-pointer"
        >
          <X className="h-3.5 w-3.5" />
        </button>
      </div>

      {/* 配套项列表 */}
      <div className="flex flex-col gap-1.5">
        {/* 1. 精密轨道 POEORB（仅 Sentinel-1 显示） */}
        {isSentinel1 && (
          <div
            data-testid="companion-item-precise-orbit"
            onClick={handleToggleOrbit}
            className={`flex items-start gap-2.5 p-2 rounded-sp-sm transition-colors cursor-pointer ${
              companionState.preciseOrbit ? "bg-sp-hover" : "hover:bg-sp-surface"
            }`}
          >
            <input
              type="checkbox"
              checked={companionState.preciseOrbit}
              onClick={(e) => e.stopPropagation()}
              onChange={handleToggleOrbit}
              data-testid="checkbox-precise-orbit"
              className="mt-0.5 h-4 w-4 rounded border-sp-line text-sp-accent focus:ring-sp-accent cursor-pointer"
            />
            <div className="flex flex-col gap-0.5 flex-1 min-w-0">
              <div className="flex items-center justify-between gap-1">
                <span className="text-xs font-semibold text-sp-text">
                  {t("panels.companionDownload.preciseOrbitTitle")}
                </span>
                <div className="flex flex-col items-end gap-0.5 text-xs text-right shrink-0">
                  {orbitMatch.matchedCount > 0 && (
                    <span className="text-sp-success-text font-medium">
                      {t("panels.companionDownload.orbitMatched", { count: orbitMatch.matchedCount })}
                    </span>
                  )}
                  {orbitMatch.pendingCount > 0 && (
                    <span className="text-sp-status-warning font-medium">
                      {t("panels.companionDownload.orbitPending", { count: orbitMatch.pendingCount })}
                    </span>
                  )}
                </div>
              </div>
              <span className="text-xs text-sp-text-muted">
                {t("panels.companionDownload.preciseOrbitSubtitle")}
              </span>
            </div>
          </div>
        )}

        {/* 2. DEM · Copernicus 30 m（全部 SAR 数据源已真实接通，自动转换 WGS84 椭球高） */}
        <div
          data-testid="companion-item-dem"
          onClick={handleToggleDem}
          className={`flex items-start gap-2.5 p-2 rounded-sp-sm transition-colors ${
            !isSar
              ? "opacity-60 cursor-not-allowed select-none"
              : companionState.dem
              ? "bg-sp-hover cursor-pointer"
              : "hover:bg-sp-surface cursor-pointer"
          }`}
        >
          <input
            type="checkbox"
            disabled={!isSar}
            checked={isSar && companionState.dem}
            onClick={(e) => e.stopPropagation()}
            onChange={handleToggleDem}
            data-testid="checkbox-dem"
            className={`mt-0.5 h-4 w-4 rounded border-sp-line text-sp-accent focus:ring-sp-accent ${
              isSar ? "cursor-pointer" : "cursor-not-allowed"
            }`}
          />
          <div className="flex flex-col gap-0.5 flex-1 min-w-0">
            <div className="flex items-center justify-between gap-1">
              <span className="text-xs font-semibold text-sp-text">
                {t("panels.companionDownload.demTitle")}
              </span>
              {isSar && effectiveExtent ? (
                <span
                  data-testid="dem-estimate-badge"
                  className="text-xs text-sp-accent font-medium shrink-0"
                >
                  {t("panels.companionDownload.demEstimatedTiles", { count: demEstimate.estimatedTiles })}
                </span>
              ) : (
                <span className="text-xs text-sp-text-muted font-medium shrink-0">
                  {isSar ? "需先定范围" : "—"}
                </span>
              )}
            </div>
            <span className="text-xs text-sp-text-muted">
              {t("panels.companionDownload.demSubtitle")} · 自动转 WGS84 椭球高
            </span>
          </div>
        </div>

        {/* 3. GACOS 大气延迟（全部 SAR 数据源已真实接通后端） */}
        <div
          data-testid="companion-item-gacos"
          onClick={handleToggleGacos}
          className={`flex items-start gap-2.5 p-2 rounded-sp-sm transition-colors ${
            !isSar
              ? "opacity-60 cursor-not-allowed select-none"
              : companionState.gacos
              ? "bg-sp-hover cursor-pointer"
              : "hover:bg-sp-surface cursor-pointer"
          }`}
        >
          <input
            type="checkbox"
            disabled={!isSar}
            checked={isSar && companionState.gacos}
            onClick={(e) => e.stopPropagation()}
            onChange={handleToggleGacos}
            data-testid="checkbox-gacos"
            className={`mt-0.5 h-4 w-4 rounded border-sp-line text-sp-accent focus:ring-sp-accent ${
              isSar ? "cursor-pointer" : "cursor-not-allowed"
            }`}
          />
          <div className="flex flex-col gap-0.5 flex-1 min-w-0">
            <div className="flex items-center justify-between gap-1">
              <span className="text-xs font-semibold text-sp-text">
                {t("panels.companionDownload.gacosTitle")}
              </span>
              {isSar ? (
                <span
                  data-testid="gacos-estimate-badge"
                  className="text-xs text-sp-accent font-medium shrink-0"
                >
                  {t("panels.companionDownload.gacosEstimatedRequests", {
                    dates: gacosEstimate.uniqueDateCount,
                    requests: gacosEstimate.requestCount,
                  })}
                </span>
              ) : (
                <span className="text-xs text-sp-text-muted font-medium shrink-0">
                  —
                </span>
              )}
            </div>
            <span className="text-xs text-sp-text-muted">
              {t("panels.companionDownload.gacosSubtitle")}
            </span>
          </div>
        </div>
      </div>

      {/* 辅助数据范围选择（勾选 GACOS 或 DEM 时展示） */}
      {isSar && (companionState.gacos || companionState.dem) && (
        <div
          data-testid="companion-extent-selector"
          className="pt-2 border-t border-sp-line flex flex-col gap-1.5"
        >
          <span className="text-xs font-medium text-sp-text-secondary">
            {t("panels.companionDownload.extentSelectorTitle")}
          </span>
          <div className="flex flex-col gap-1">
            <label
              data-testid="extent-option-study-area"
              className="flex items-center gap-2 text-xs text-sp-text cursor-pointer hover:text-sp-accent"
            >
              <input
                type="radio"
                name="companion-extent-mode"
                checked={companionState.extentMode === "study_area"}
                onChange={() => handleChangeExtentMode("study_area")}
                className="h-3.5 w-3.5 text-sp-accent focus:ring-sp-accent cursor-pointer"
              />
              <span>{t("panels.companionDownload.extentStudyArea")}</span>
            </label>
            <label
              data-testid="extent-option-scene-coverage"
              className="flex items-center gap-2 text-xs text-sp-text cursor-pointer hover:text-sp-accent"
            >
              <input
                type="radio"
                name="companion-extent-mode"
                checked={companionState.extentMode === "scene_coverage"}
                onChange={() => handleChangeExtentMode("scene_coverage")}
                className="h-3.5 w-3.5 text-sp-accent focus:ring-sp-accent cursor-pointer"
              />
              <span>{t("panels.companionDownload.extentSceneCoverage")}</span>
            </label>
          </div>
        </div>
      )}

      {/* 底部操作条：随主影像附带说明 + 独立「仅下载已选配套」按钮 */}
      <div className="pt-2 border-t border-sp-line flex items-center justify-between gap-2">
        <div className="flex items-center gap-1.5 text-xs text-sp-text-secondary min-w-0">
          <Check className="h-3.5 w-3.5 text-sp-accent shrink-0" />
          <span className="truncate">
            {hasActiveOrbit
              ? t("panels.companionDownload.preciseOrbitDefaultNotice")
              : "下载主影像时自动同步附带"}
          </span>
        </div>
        {onTriggerStandaloneCompanion && (
          <button
            type="button"
            data-testid="btn-standalone-companion-download"
            disabled={!canRunStandalone}
            onClick={() => void onTriggerStandaloneCompanion()}
            title={
              !hasActiveOrbit && !hasActiveDem && !hasActiveGacos
                ? "请至少勾选一项配套数据"
                : !hasActiveDem && selectedScenes.length === 0
                ? "精轨和 GACOS 需先勾选至少 1 景影像；或仅勾选 30m DEM 按研究区下载"
                : "不重下主影像，单独创建已选配套下载任务"
            }
            className="eo-tactile-primary h-7 px-2.5 rounded-sp-sm text-xs font-medium inline-flex items-center gap-1 shrink-0 cursor-pointer disabled:opacity-45 disabled:cursor-not-allowed"
          >
            {standaloneBusy ? (
              <Loader2 className="h-3 w-3 animate-spin" />
            ) : (
              <Download className="h-3 w-3" />
            )}
            <span>仅下载已选配套</span>
          </button>
        )}
      </div>
    </div>
  );
}
