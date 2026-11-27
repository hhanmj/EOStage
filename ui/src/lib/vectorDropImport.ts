import { useEffect, useId, useRef } from "react";
import {
  previewAoiFile,
  previewAoiFileBundle,
  previewAoiFileBytes,
  setWorkingAoiGeojsonFeatures,
  type AoiPreviewOk,
  type Bbox,
  type Json,
} from "@/lib/bridge";
import { getDesktopShellBridge } from "@/lib/desktopShellBridge";
import {
  subscribeDesktopHostEvent,
  type WorkbenchFileDropEvent,
} from "@/lib/desktopHostEvents";
import { extractWgs84RingsAndBbox } from "@/lib/opticalAoiClipLogic";
import { isTauriDesktopHost } from "@/lib/bridge";
import {
  isAcceptedVectorFileName,
  vectorAoiDisplayName,
} from "@/shell/api/agentComposerLogic";
import { bboxArrayToBbox, readFileAsBase64 } from "@/shell/api/vectorAoiImport";

const SHAPEFILE_SIDECAR_EXTENSIONS = new Set([
  ".dbf",
  ".shx",
  ".prj",
  ".cpg",
  ".sbn",
  ".sbx",
]);

function fileExtensionLower(nameOrPath: string): string {
  const clean = nameOrPath.trim().split(/[\\/]/).pop() || "";
  const dot = clean.lastIndexOf(".");
  return dot > 0 && dot < clean.length - 1 ? clean.slice(dot).toLowerCase() : "";
}

function isVectorOrSidecarFileName(nameOrPath: string): boolean {
  return (
    isAcceptedVectorFileName(nameOrPath) ||
    SHAPEFILE_SIDECAR_EXTENSIONS.has(fileExtensionLower(nameOrPath))
  );
}

export function pickPrimaryVectorPath(paths: readonly string[]): string | null {
  const accepted = paths.filter((p) => isAcceptedVectorFileName(p));
  if (accepted.length === 0) return null;
  const shp = accepted.find((p) => fileExtensionLower(p) === ".shp");
  if (shp) return shp;
  const zip = accepted.find((p) => fileExtensionLower(p) === ".zip");
  return zip ?? accepted[0] ?? null;
}

export type VectorDropImportResult =
  | { ok: true; geometry: Record<string, unknown>; bbox: Bbox; label: string }
  | { ok: false; error: string };

function featureIdsFromPreview(preview: AoiPreviewOk): string[] {
  if (!Array.isArray(preview.features)) return [];
  return preview.features.map(
    (f) => f.id || (f.properties?.id as string) || String(f.index ?? 0),
  );
}

async function normalizeOrPersistPreview(
  preview: AoiPreviewOk,
  fallbackName: string,
  persistWorkingAoi: boolean,
): Promise<VectorDropImportResult> {
  if (!preview.geojson) {
    return { ok: false, error: "矢量文件中没有可用的多边形边界。" };
  }
  const featureIds = featureIdsFromPreview(preview);
  const label = vectorAoiDisplayName(preview.file_name || fallbackName) || "矢量研究区";
  const nameField = preview.display_field || "";

  if (persistWorkingAoi) {
    const result = await setWorkingAoiGeojsonFeatures(
      preview.geojson,
      featureIds,
      nameField,
      "merge",
      label,
    );
    if (!result.ok) {
      return { ok: false, error: result.error || "矢量范围保存失败。" };
    }
    const bbox = bboxArrayToBbox(result.bbox);
    const geom = result.geometry as Record<string, unknown> | undefined;
    if (!bbox || !geom) {
      return { ok: false, error: "无法解析归一化后的矢量外包框。" };
    }
    return {
      ok: true,
      geometry: geom,
      bbox,
      label: result.display_name || label,
    };
  }

  let normalizedGeom: Json = preview.geojson;
  let resolvedBbox: Bbox | null = null;
  if (isTauriDesktopHost()) {
    const normalizer = getDesktopShellBridge().application.aoiNormalize;
    if (typeof normalizer === "function") {
      const normalized = await normalizer({
        geojson: preview.geojson,
        feature_ids: featureIds,
        name_field: nameField,
        download_mode: "merge",
      });
      if (normalized && typeof normalized === "object") {
        const rec = normalized as Record<string, unknown>;
        if (rec.ok === false) {
          return {
            ok: false,
            error: String(rec.error ?? "矢量要素归一化失败。"),
          };
        }
        if (rec.geojson && typeof rec.geojson === "object") {
          normalizedGeom = rec.geojson as Json;
        } else if (rec.aoi_geojson && typeof rec.aoi_geojson === "object") {
          normalizedGeom = rec.aoi_geojson as Json;
        }
        resolvedBbox = bboxArrayToBbox(rec.bbox);
      }
    }
  }
  if (!resolvedBbox) {
    const extracted = extractWgs84RingsAndBbox(normalizedGeom, null);
    if (extracted) {
      const [west, south, east, north] = extracted.aoiBboxWgs84;
      resolvedBbox = { west, south, east, north, crs: "EPSG:4326" };
    } else if (preview.features?.[0]?.bbox) {
      resolvedBbox = preview.features[0].bbox;
    }
  }
  if (!resolvedBbox || !normalizedGeom || typeof normalizedGeom !== "object") {
    return { ok: false, error: "无法从矢量文件中解析有效地理范围。" };
  }
  return {
    ok: true,
    geometry: normalizedGeom as Record<string, unknown>,
    bbox: resolvedBbox,
    label,
  };
}

export async function importVectorFromNativePaths(
  paths: readonly string[],
  options: { persistWorkingAoi: boolean },
): Promise<VectorDropImportResult> {
  const primaryPath = pickPrimaryVectorPath(paths);
  if (!primaryPath) {
    return {
      ok: false,
      error: "请拖入 .shp、.zip (Shapefile 压缩包)、.geojson、.kml 或 .json 矢量文件。",
    };
  }
  const preview = await previewAoiFile(primaryPath);
  if (!preview.ok) {
    return { ok: false, error: preview.error || "无法读取矢量文件。" };
  }
  return normalizeOrPersistPreview(preview, primaryPath, options.persistWorkingAoi);
}

export async function importVectorFromHtml5Files(
  fileList: FileList | readonly File[],
  options: { persistWorkingAoi: boolean },
): Promise<VectorDropImportResult> {
  const files = Array.from(fileList).filter((f) => isVectorOrSidecarFileName(f.name));
  const primaryFile = files.find((f) => isAcceptedVectorFileName(f.name));
  if (!primaryFile) {
    return {
      ok: false,
      error: "请拖入 .shp（可连同 .dbf/.shx/.prj）、.zip、.geojson 或 .kml 矢量文件。",
    };
  }
  const primaryExt = fileExtensionLower(primaryFile.name);
  const useBundle = files.length > 1 || primaryExt === ".shp";
  const preview = useBundle
    ? await previewAoiFileBundle(
        await Promise.all(
          files.map(async (f) => ({
            name: f.name,
            base64: await readFileAsBase64(f),
          })),
        ),
      )
    : await previewAoiFileBytes(primaryFile.name, await readFileAsBase64(primaryFile));

  if (!preview.ok) {
    return { ok: false, error: preview.error || "无法解析拖入的矢量文件。" };
  }
  return normalizeOrPersistPreview(preview, primaryFile.name, options.persistWorkingAoi);
}

type NativeDropRegistration = {
  id: string;
  priority: number;
};

const activeNativeDropTargets = new Map<string, NativeDropRegistration>();

function isHighestPriorityNativeDropTarget(id: string): boolean {
  const own = activeNativeDropTargets.get(id);
  if (!own) return false;
  for (const other of activeNativeDropTargets.values()) {
    if (other.priority > own.priority) return false;
  }
  return true;
}

export function useNativeVectorDropTarget({
  enabled,
  priority,
  onHoverChange,
  onPathsDropped,
}: {
  enabled: boolean;
  priority: number;
  onHoverChange: (hovered: boolean) => void;
  onPathsDropped: (paths: readonly string[]) => void;
}) {
  const targetId = useId();
  const hoverCbRef = useRef(onHoverChange);
  const dropCbRef = useRef(onPathsDropped);
  hoverCbRef.current = onHoverChange;
  dropCbRef.current = onPathsDropped;

  useEffect(() => {
    if (!enabled) {
      activeNativeDropTargets.delete(targetId);
      hoverCbRef.current(false);
      return;
    }
    activeNativeDropTargets.set(targetId, { id: targetId, priority });
    const unsub = subscribeDesktopHostEvent<WorkbenchFileDropEvent>(
      "workbench:file-drop",
      (payload) => {
        if (!payload || !isHighestPriorityNativeDropTarget(targetId)) {
          hoverCbRef.current(false);
          return;
        }
        if (payload.kind === "hover" || payload.kind === "over") {
          hoverCbRef.current(true);
        } else if (payload.kind === "cancel") {
          hoverCbRef.current(false);
        } else if (payload.kind === "drop") {
          hoverCbRef.current(false);
          if (Array.isArray(payload.paths) && payload.paths.length > 0) {
            dropCbRef.current(payload.paths);
          }
        }
      },
    );
    return () => {
      activeNativeDropTargets.delete(targetId);
      hoverCbRef.current(false);
      unsub();
    };
  }, [enabled, priority, targetId]);
}
