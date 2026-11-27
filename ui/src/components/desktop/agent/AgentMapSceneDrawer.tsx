/**
 * AgentMapSceneDrawer.tsx
 * Compact click-anchored scene inspector popover for the Agent 2.0 Split Map pane.
 * Displays selected scene scientific metadata (Scene ID, UTC acquisition time, orbit direction,
 * polarization, cloud cover / incidence angle / size) and direct actions (quicklook overlay toggle,
 * check/uncheck scene, create download task, close).
 */

import React, { useEffect } from "react";
import { Check, CheckSquare, Download, Eye, EyeOff, Orbit, Radio, Square, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";
import { formatBytes } from "@/shell/api/agentStreamLogic";
import type { SceneRow } from "@/shell/api/types";

export interface AgentSceneInspectorItem extends Partial<SceneRow> {
  scene_id: string;
  cloud_cover_percent?: number | null;
  incidence_angle_deg?: number | null;
  size_bytes?: number | null;
}

export interface AgentMapSceneDrawerProps {
  scene: AgentSceneInspectorItem | null;
  isChecked?: boolean;
  isQuicklookActive?: boolean;
  isDownloading?: boolean;
  onToggleQuicklook?: (sceneId: string, nextActive: boolean) => void;
  onToggleCheckScene?: (sceneId: string) => void;
  onDownloadScene?: (sceneId: string) => void;
  onClose?: () => void;
  className?: string;
}

function readNumericField(
  scene: AgentSceneInspectorItem,
  directVal: number | null | undefined,
  metaKeys: string[]
): number | null {
  if (typeof directVal === "number" && Number.isFinite(directVal)) {
    return directVal;
  }
  const meta = scene.native_metadata;
  if (meta && typeof meta === "object") {
    for (const key of metaKeys) {
      const raw = meta[key];
      if (typeof raw === "number" && Number.isFinite(raw)) {
        return raw;
      }
      if (typeof raw === "string" && raw.trim().length > 0) {
        const num = Number(raw);
        if (Number.isFinite(num)) return num;
      }
    }
  }
  return null;
}

export function inferResourceIdFromSceneId(raw: string): string | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  const upper = trimmed.toUpperCase();
  if (
    upper.includes("::COLLECTION::LANDSAT-C2-L2::") ||
    upper.includes("SCIENCE.LANDSAT.C2-L2") ||
    /^L[CETM]0[45789]_L2/.test(upper)
  ) {
    return "science.landsat.c2-l2.v1";
  }
  if (
    upper.includes("::COLLECTION::SENTINEL-2-L1C::") ||
    /^S2[AB]_MSIL1C_/.test(upper)
  ) {
    return "science.sentinel-2.l1c.v1";
  }
  if (
    upper.includes("::COLLECTION::SENTINEL-2-L2A::") ||
    upper.includes("OPTICAL.COPERNICUS.S2") ||
    /^S2[AB]_MSIL2A_/.test(upper)
  ) {
    return "optical.copernicus.s2.sr.v1";
  }
  if (upper.includes("NISAR_S_")) {
    return "science.nisar.s.ssar.rslc.v1";
  }
  if (
    upper.includes("C3622236985-ASF") ||
    (upper.includes("NISAR_") && !upper.includes("_PR_") && !upper.includes("PROVISIONAL") && upper.includes("_V1"))
  ) {
    return "science.nisar.l.rslc.validated.v1";
  }
  if (upper.includes("NISAR_L_") || upper.includes("C2853145197-ASF") || /^NISAR_/.test(upper)) {
    return "science.nisar.l.rslc.provisional.v1";
  }
  if (
    upper.includes("C3315903479-ASF") ||
    /^ALOS2/.test(upper)
  ) {
    return "science.alos-2.palsar-2.scansar-l1.1.v1";
  }
  if (
    upper.includes("C2834279090-ASF") ||
    upper.includes("OPERA_L")
  ) {
    return "science.opera.disp-s1.v1";
  }
  if (
    upper.includes("C1595771383-ASF") ||
    upper.includes("S1-GUNW")
  ) {
    return "science.aria.s1-gunw.v1";
  }
  if (
    upper.includes("::COLLECTION::SENTINEL-1::") ||
    /^S1[ABCD]_(IW|EW|SM)_(SLC|GRD)/.test(upper) ||
    /^S1_.*_BURST/.test(upper)
  ) {
    return "sar.sentinel1.slc.v1";
  }
  if (upper.includes("::COLLECTION::COP-DEM-GLO-30::")) {
    return "dem.copernicus.glo30.v1";
  }
  if (upper.includes("::COLLECTION::COP-DEM-GLO-90::")) {
    return "dem.copernicus.glo90.v1";
  }
  return null;
}

export function matchSceneIdFromList(raw: string, list: readonly SceneRow[]): string | null {
  const trimmed = raw.trim();
  if (!trimmed || list.length === 0) return null;
  const bare = trimmed.replace(/^PROVIDER::[^:]+::COLLECTION::[^:]+::ITEM::/i, "").trim();
  const shortEntity = bare.replace(
    /^([A-Z0-9]+_[A-Z0-9]+_\d{6}_\d{8})_\d{8}_(\d{2}_[A-Z0-9]+)$/i,
    "$1_$2"
  );
  const candidates = Array.from(new Set([trimmed, bare, shortEntity].filter(Boolean)));

  for (const candidate of candidates) {
    const exact = list.find(
      (s) =>
        s.scene_id === candidate ||
        s.product_key === candidate ||
        s.product_id === candidate ||
        s.file_name === candidate ||
        s.scene_id.endsWith(`::ITEM::${candidate}`)
    );
    if (exact) return exact.scene_id;
  }
  if (candidates.some((c) => c === "first" || c === "0" || c === "#1")) {
    return list[0]?.scene_id ?? null;
  }
  for (const candidate of candidates) {
    const partial = list.find(
      (s) =>
        s.scene_id.includes(candidate) ||
        (s.product_id ? s.product_id.includes(candidate) : false) ||
        (s.file_name ? s.file_name.includes(candidate) : false)
    );
    if (partial) return partial.scene_id;
  }
  return null;
}

export function AgentMapSceneDrawer({
  scene,
  isChecked = false,
  isQuicklookActive = false,
  isDownloading = false,
  onToggleQuicklook,
  onToggleCheckScene,
  onDownloadScene,
  onClose,
  className,
}: AgentMapSceneDrawerProps) {
  const { t } = useI18n();

  useEffect(() => {
    if (!scene || !onClose || typeof document === "undefined") return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !e.defaultPrevented) {
        e.preventDefault();
        onClose();
      }
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [scene, onClose]);

  if (!scene) {
    return null;
  }

  const rawOrbit = (scene.orbit_direction || "").trim().toUpperCase();
  const orbitLabel =
    rawOrbit === "ASCENDING" || rawOrbit === "ASC"
      ? t("agent.sceneDrawer.ascending")
      : rawOrbit === "DESCENDING" || rawOrbit === "DESC"
        ? t("agent.sceneDrawer.descending")
        : scene.orbit_direction?.trim() || null;

  const polarization = (scene.polarization || "").trim() || null;
  const acquisitionTime = (scene.acquisition_datetime || "").trim() || t("agent.unknown");

  const cloudCover = readNumericField(scene, scene.cloud_cover_percent, [
    "eo:cloud_cover",
    "cloud_cover",
    "cloudCover",
  ]);
  const incidenceAngle = readNumericField(scene, scene.incidence_angle_deg, [
    "view:incidence_angle",
    "sar:incidence_angle",
    "incidence_angle",
  ]);
  const sizeBytes =
    typeof scene.size_bytes === "number" && Number.isFinite(scene.size_bytes) && scene.size_bytes > 0
      ? scene.size_bytes
      : typeof scene.file_size_remote === "number" &&
          Number.isFinite(scene.file_size_remote) &&
          scene.file_size_remote > 0
        ? scene.file_size_remote
        : null;

  const metaBadges = [
    scene.platform?.trim(),
    scene.product_type?.trim(),
    scene.beam_mode?.trim(),
    typeof scene.relative_orbit === "number"
      ? t("agent.sceneDrawer.orbitTrack", { value: scene.relative_orbit })
      : null,
  ].filter((v): v is string => Boolean(v && v.length > 0));

  const displayTitle = (() => {
    const raw = (scene.file_name || scene.product_id || scene.scene_id || "").trim();
    const itemMarker = "::ITEM::";
    const idx = raw.lastIndexOf(itemMarker);
    return idx >= 0 ? raw.slice(idx + itemMarker.length) : raw;
  })();

  return (
    <div
      data-testid="agent-map-scene-drawer"
      data-scene-id={scene.scene_id}
      role="dialog"
      aria-modal="false"
      aria-label={t("agent.sceneDrawer.aria")}
      className={cn(
        "pointer-events-auto z-[750] rounded-sp-card border border-sp-line bg-sp-surface dark:bg-sp-card p-3 shadow-sp-overlay flex flex-col gap-2 w-[320px] max-w-full",
        className
      )}
    >
      {/* Top row: Scene ID + Close */}
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0 flex-1 flex flex-col gap-1">
          <div className="flex items-center gap-2 flex-wrap">
            <span className="text-xs font-semibold text-sp-accent select-none">
              {t("agent.sceneDrawer.title")}
            </span>
            {metaBadges.map((b, idx) => (
              <span
                key={`${b}-${idx}`}
                className="rounded bg-sp-surface-muted border border-sp-line px-1.5 py-0.5 text-xs font-medium text-sp-text-secondary"
              >
                {b}
              </span>
            ))}
          </div>
          <div
            data-testid="agent-scene-drawer-title"
            className="font-mono text-xs font-semibold text-sp-text break-all select-text"
          >
            {displayTitle}
          </div>
        </div>

        {onClose && (
          <button
            type="button"
            data-testid="btn-drawer-close"
            onClick={onClose}
            title={t("agent.sceneDrawer.close")}
            aria-label={t("agent.sceneDrawer.close")}
            className="rounded-md p-1 text-sp-text-muted hover:bg-sp-hover hover:text-sp-text transition-colors duration-fast ease-sp cursor-pointer shrink-0"
          >
            <X className="h-4 w-4" />
          </button>
        )}
      </div>

      {/* Scientific attributes grid */}
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1.5 text-xs text-sp-text-secondary select-text">
        <div className="flex items-center gap-1.5">
          <span className="text-sp-text-muted">{t("agent.sceneDrawer.acquisitionTime")}:</span>
          <span
            data-testid="agent-scene-drawer-time"
            className="font-mono text-sp-text tabular-nums"
          >
            {acquisitionTime}
          </span>
        </div>

        {orbitLabel && (
          <div className="inline-flex items-center gap-1">
            <Orbit className="h-3.5 w-3.5 text-sp-accent shrink-0" />
            <span data-testid="agent-scene-drawer-orbit" className="font-medium text-sp-text">
              {orbitLabel}
            </span>
          </div>
        )}

        {polarization && (
          <div className="inline-flex items-center gap-1">
            <Radio className="h-3.5 w-3.5 text-sp-text-muted shrink-0" />
            <span
              data-testid="agent-scene-drawer-polarization"
              className="rounded bg-sp-surface-muted border border-sp-line px-1.5 py-0.5 font-mono text-xs text-sp-text"
            >
              {polarization}
            </span>
          </div>
        )}

        {cloudCover !== null && (
          <div className="inline-flex items-center gap-1 tabular-nums">
            <span className="font-medium text-sp-text">
              {t("agent.sceneDrawer.cloudCover", { value: cloudCover.toFixed(1) })}
            </span>
          </div>
        )}

        {incidenceAngle !== null && (
          <div className="inline-flex items-center gap-1 tabular-nums">
            <span className="font-medium text-sp-text">
              {t("agent.sceneDrawer.incidenceAngle", { value: incidenceAngle.toFixed(1) })}
            </span>
          </div>
        )}

        {sizeBytes !== null && (
          <div className="inline-flex items-center gap-1 tabular-nums">
            <span className="text-sp-text-muted">{t("agent.sceneDrawer.size")}:</span>
            <span className="font-medium text-sp-text">{formatBytes(sizeBytes)}</span>
          </div>
        )}
      </div>

      {/* Action buttons */}
      <div className="flex flex-wrap items-center justify-end gap-1.5 pt-1.5 border-t border-sp-line select-none">
        {onToggleQuicklook && (
          <button
            type="button"
            data-testid="btn-drawer-toggle-quicklook"
            aria-pressed={isQuicklookActive}
            onClick={() => onToggleQuicklook(scene.scene_id, !isQuicklookActive)}
            className={cn(
              "inline-flex items-center gap-1 rounded-sp-sm border px-2 py-1 text-xs font-medium transition-colors duration-fast ease-sp cursor-pointer",
              isQuicklookActive
                ? "border-sp-accent bg-sp-hover text-sp-accent-strong"
                : "border-sp-line bg-sp-surface-muted text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text"
            )}
          >
            {isQuicklookActive ? (
              <EyeOff className="h-3.5 w-3.5" />
            ) : (
              <Eye className="h-3.5 w-3.5" />
            )}
            <span>
              {isQuicklookActive
                ? t("agent.sceneDrawer.quicklookOn")
                : t("agent.sceneDrawer.quicklookOff")}
            </span>
          </button>
        )}

        {onToggleCheckScene && (
          <button
            type="button"
            data-testid="btn-drawer-check-scene"
            aria-pressed={isChecked}
            onClick={() => onToggleCheckScene(scene.scene_id)}
            className={cn(
              "inline-flex items-center gap-1 rounded-sp-sm border px-2 py-1 text-xs font-medium transition-colors duration-fast ease-sp cursor-pointer",
              isChecked
                ? "border-sp-accent bg-sp-hover text-sp-accent-strong"
                : "border-sp-line bg-sp-surface-muted text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text"
            )}
          >
            {isChecked ? (
              <CheckSquare className="h-3.5 w-3.5 text-sp-accent" />
            ) : (
              <Square className="h-3.5 w-3.5 text-sp-text-muted" />
            )}
            <span>
              {isChecked
                ? t("agent.sceneDrawer.checkedScene")
                : t("agent.sceneDrawer.checkScene")}
            </span>
          </button>
        )}

        {onDownloadScene && (
          <button
            type="button"
            data-testid="btn-drawer-download-scene"
            disabled={isDownloading}
            onClick={() => onDownloadScene(scene.scene_id)}
            className="inline-flex items-center gap-1 rounded-sp-sm bg-sp-accent px-2.5 py-1 text-xs font-medium text-sp-canvas hover:bg-sp-accent-strong disabled:opacity-55 disabled:cursor-not-allowed transition-colors duration-fast ease-sp cursor-pointer"
          >
            {isDownloading ? (
              <Check className="h-3.5 w-3.5" />
            ) : (
              <Download className="h-3.5 w-3.5" />
            )}
            <span>{t("agent.sceneDrawer.downloadScene")}</span>
          </button>
        )}
      </div>
    </div>
  );
}
