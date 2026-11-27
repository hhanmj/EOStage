import { t } from "../i18n/index.ts";
import { queryOpenGeodata } from "./bridge.ts";
import type {
  WaybackProbeZoomResult,
  WaybackScanStatusResponse,
  WaybackVersionsResponse,
} from "./waybackLogic.ts";

export * from "./waybackLogic.ts";

export async function fetchWaybackVersions(
  forceRefresh = false,
): Promise<WaybackVersionsResponse> {
  const res = await queryOpenGeodata({
    dataset: "esri_wayback_versions",
    gee_params: { force_refresh: forceRefresh },
  });
  if (!res || !res.ok) {
    throw new Error(String(res?.error || t("mapWorkbench.wayback.fetchVersionsFailed")));
  }
  return res as unknown as WaybackVersionsResponse;
}

export async function probeWaybackMaxZoom(
  bbox: [number, number, number, number],
  versionId?: string,
): Promise<WaybackProbeZoomResult> {
  const res = await queryOpenGeodata({
    dataset: "esri_wayback_probe_zoom",
    bbox,
    gee_params: versionId ? { version_id: versionId } : undefined,
  });
  if (!res || !res.ok) {
    throw new Error(String(res?.error || t("mapWorkbench.wayback.probeZoomFailed")));
  }
  return res as unknown as WaybackProbeZoomResult;
}

export async function startWaybackScan(
  bbox: [number, number, number, number],
  zoom: number,
  mode: "fast" | "fine" | "official" = "fast",
  forceRefresh = false,
): Promise<WaybackScanStatusResponse> {
  const res = await queryOpenGeodata({
    dataset: "esri_wayback_scan_start",
    bbox,
    gee_params: { zoom, mode, force_refresh: forceRefresh },
  });
  if (!res || !res.ok) {
    throw new Error(String(res?.error || t("mapWorkbench.wayback.startScanFailed")));
  }
  return res as unknown as WaybackScanStatusResponse;
}

export async function getWaybackScanStatus(
  scanId: string,
): Promise<WaybackScanStatusResponse> {
  const res = await queryOpenGeodata({
    dataset: "esri_wayback_scan_status",
    gee_params: { scan_id: scanId },
  });
  if (!res || !res.ok) {
    throw new Error(String(res?.error || t("mapWorkbench.wayback.scanStatusFailed")));
  }
  return res as unknown as WaybackScanStatusResponse;
}

export async function cancelWaybackScan(scanId: string): Promise<void> {
  await queryOpenGeodata({
    dataset: "esri_wayback_scan_cancel",
    gee_params: { scan_id: scanId },
  }).catch(() => {});
}
