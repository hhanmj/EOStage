import { t } from "../../i18n/index.ts";
import {
  taskDeliveryProvider,
  sarTaskDatasetLabel,
} from "../../lib/sarTaskDisplayName.ts";

export function humanTaskLabel(params: {
  provider_id?: string | null;
  dataset_id?: string | null;
  selected_count?: number | null;
  scene_count?: number | null;
  task_id?: string | null;
  task_type?: string | null;
}): { title: string; subtitle: string } {
  const providerInfo = taskDeliveryProvider({
    provider_id: params.provider_id,
  });

  const rawDataset = (params.dataset_id || "").trim();
  const rawUpper = rawDataset.toUpperCase();
  const rawLower = rawDataset.toLowerCase();
  const taskType = (params.task_type || "").trim().toLowerCase();
  const providerLower = (params.provider_id || "").trim().toLowerCase();
  const taskIdLower = (params.task_id || "").trim().toLowerCase();

  let datasetLabel = "";
  if (
    providerLower === "sentinel1.orbit" ||
    taskType === "orbit" ||
    taskIdLower.endsWith("-orbit")
  ) {
    datasetLabel = "精密轨道 POEORB";
  } else if (providerLower === "gacos.webform" || taskIdLower.startsWith("gacos-request-")) {
    datasetLabel = "GACOS 大气延迟申请";
  } else if (providerLower === "gacos.manual-result" || taskIdLower.startsWith("gacos-result-")) {
    datasetLabel = "GACOS 结果下载与导入";
  } else if (providerLower === "gacos.local-result" || taskIdLower.startsWith("gacos-import-")) {
    datasetLabel = "GACOS 本地结果导入";
  } else if (rawLower.includes("gacos")) {
    datasetLabel = "GACOS 大气延迟";
  } else if (rawUpper.includes("SENTINEL-1") || rawUpper.includes("S1")) {
    if (taskType === "sentinel1_slc") {
      datasetLabel = "Sentinel-1 SLC";
    } else if (taskType === "sentinel1_grd") {
      datasetLabel = "Sentinel-1 GRD";
    } else if (taskType === "sentinel1_raw") {
      datasetLabel = "Sentinel-1 RAW";
    } else if (taskType === "sentinel1_ocn") {
      datasetLabel = "Sentinel-1 OCN";
    } else {
      datasetLabel = "Sentinel-1";
    }
  } else if (rawUpper.includes("NISAR")) {
    const bandLabel =
      rawLower.includes("nisar.s.") || rawLower.includes("ssar")
        ? "NISAR S-Band RSLC"
        : rawLower.includes("nisar.l.")
          ? "NISAR L-Band RSLC"
          : "NISAR";
    if (taskType === "nisar_subset") {
      datasetLabel = `${bandLabel} · 裁剪`;
    } else if (taskType === "nisar_complete") {
      datasetLabel = `${bandLabel} · 完整`;
    } else {
      datasetLabel = bandLabel;
    }
  } else if (rawUpper.includes("ALOS")) {
    datasetLabel = "ALOS PALSAR";
  } else if (rawLower === "optical.hls.l30.v2" || rawLower.includes("hls.l30")) {
    datasetLabel = "HLS L30";
  } else if (rawLower === "optical.hls.s30.v2" || rawLower.includes("hls.s30")) {
    datasetLabel = "HLS S30";
  } else if (rawLower === "optical.sentinel-2.l2a" || rawLower.includes("sentinel-2.l2a")) {
    datasetLabel = "Sentinel-2 L2A";
  } else if (rawLower === "optical.sentinel-2.l1c" || rawLower.includes("sentinel-2.l1c")) {
    datasetLabel = "Sentinel-2 L1C";
  } else if (rawLower === "optical.landsat.c2.l2" || rawLower.includes("landsat")) {
    datasetLabel = "Landsat C2 L2";
  } else if (rawDataset.startsWith("dem.")) {
    datasetLabel = t("shared.taskCardNaming.demLabel");
  } else {
    const fromHelper = sarTaskDatasetLabel({
      dataset_id: params.dataset_id,
      name: params.task_id,
    });
    datasetLabel = fromHelper && fromHelper !== "SAR" ? fromHelper : rawDataset || t("shared.taskCardNaming.sarImageryFallback");
  }

  const count = params.scene_count ?? params.selected_count ?? 1;
  const countSuffix = count > 1 ? t("shared.taskCardNaming.productCountSuffix", { count }) : "";

  const title = `${datasetLabel}${countSuffix}`;
  const subtitle = providerInfo.label && providerInfo.label !== "—" ? providerInfo.label : (params.provider_id || t("shared.taskCardNaming.sarDataSubtitleFallback"));

  return { title, subtitle };
}
