import test from "node:test";
import assert from "node:assert/strict";

import { setCurrentLocale, translate } from "./index.ts";
import {
  ASF_DATASET_ACCESS_RESTRICTED,
  asfRestrictedAccessPresentation,
} from "../lib/asfRestrictedAccess.ts";
import {
  ASF_REMOVED_REASON_ZH,
  EMPTY_PREVIEW_STATE,
  previewErrorMessage,
  restrictedSarReason,
} from "../lib/catalogPreview.ts";
import { DEM_DATASET_GROUPS, DEM_DATASET_OPTIONS } from "../lib/demSources.ts";
import { landsatAssetDisplayName, landsatAssetRoleLabel } from "../lib/landsatAssetDetails.ts";
import { presentLocalReuse, reacquireConfirmationText } from "../lib/localReusePresentation.ts";
import { opticalAdmissionReason } from "../lib/opticalDeliveryAdmission.ts";
import { nisarSubsetCapabilityMessage } from "../lib/sarSourceCapabilities.ts";
import { sarTaskDatasetLabel, sarTaskDisplayName, sarTaskTypeLabel } from "../lib/sarTaskDisplayName.ts";
import { SENTINEL2_REFLECTANCE_BANDS, sentinel2BandPresetLabel } from "../lib/sentinel2BandSelection.ts";
import { formatDuration, taskStatusLabel, workTitle } from "../shell/api/agentWork.ts";
import {
  autoCancelOutcomeText,
  CARD_REASON_ZH,
  projectApproval,
  projectErrorCard,
} from "../shell/api/approvalLogic.ts";
import { executeAutoCancel } from "../shell/api/autoCancelFlow.ts";
import { projectOutputDirRow, validateLocalOutputDir } from "../shell/api/confirmCardOutputDir.ts";
import {
  KNOWN_NETWORK_SOURCES,
  PROBE_ERROR_REASONS,
  PROXY_MODE_OPTIONS,
  SOURCE_OVERRIDE_OPTIONS,
  validateProxyUrl,
} from "../shell/api/networkSettingsLogic.ts";
import { failureSummaryText } from "../shell/api/resultListLogic.ts";
import { ROUTE_METADATA } from "../shell/api/shellRoutes.ts";
import { getStatusStateConfig } from "../shell/api/statusStateLogic.ts";

test("English singular/plural formatting selects xxx_one / xxx_other via Intl.PluralRules while preserving zh-CN", () => {
  assert.equal(translate("zh-CN", "mapWorkbench.results.sceneCount", { total: 1 }), "1 景");
  assert.equal(translate("zh-CN", "mapWorkbench.results.sceneCount", { total: 2 }), "2 景");
  assert.equal(translate("zh-CN", "mapWorkbench.results.sceneCount", { total: 0 }), "0 景");
  assert.equal(translate("en", "mapWorkbench.results.sceneCount", { total: 1 }), "1 scene");
  assert.equal(translate("en", "mapWorkbench.results.sceneCount", { total: 2 }), "2 scenes");
  assert.equal(translate("en", "mapWorkbench.results.sceneCount", { total: 0 }), "0 scenes");

  assert.equal(translate("zh-CN", "shared.approval.cancelledTasksOnly", { count: 1 }), "已取消 1 个任务");
  assert.equal(translate("zh-CN", "shared.approval.cancelledTasksOnly", { count: 2 }), "已取消 2 个任务");
  assert.equal(translate("zh-CN", "shared.approval.cancelledTasksOnly", { count: 0 }), "已取消 0 个任务");
  assert.equal(translate("en", "shared.approval.cancelledTasksOnly", { count: 1 }), "Cancelled 1 task");
  assert.equal(translate("en", "shared.approval.cancelledTasksOnly", { count: 2 }), "Cancelled 2 tasks");
  assert.equal(translate("en", "shared.approval.cancelledTasksOnly", { count: 3 }), "Cancelled 3 tasks");
  assert.equal(translate("en", "shared.approval.cancelledTasksOnly", { count: 0 }), "Cancelled 0 tasks");

  assert.equal(translate("en", "shared.catalogPreview.downloadableFilesCount", { count: 1 }), "1 download file");
  assert.equal(translate("en", "shared.catalogPreview.downloadableFilesCount", { count: 2 }), "2 download files");
  assert.equal(translate("en", "shared.catalogPreview.downloadableFilesCount", { count: 4 }), "4 download files");
  assert.equal(translate("en", "shared.catalogPreview.downloadableFilesCount", { count: 0 }), "0 download files");
});

test("shared lib/ modules evaluate dynamically across zh-CN, en, and ja while keeping zh-CN byte-identical", () => {
  try {
    setCurrentLocale("zh-CN");
    assert.equal(EMPTY_PREVIEW_STATE.message, "先激活一个产品以请求预览。");
    assert.equal(previewErrorMessage("provider_no_preview"), "Provider 没有提供可验证的预览资产。");
    assert.equal(ASF_REMOVED_REASON_ZH, "ASF 已从检索接口下线该数据集");
    assert.equal(restrictedSarReason("ERS"), "ASF 已从检索接口下线该数据集");
    assert.equal(asfRestrictedAccessPresentation("JERS-1", ASF_DATASET_ACCESS_RESTRICTED)?.badge, "🔒 访问受限");
    assert.equal(DEM_DATASET_GROUPS[0].label, "公共 AWS：无需密钥");
    assert.equal(DEM_DATASET_OPTIONS[0].hint, "EGM2008 正高 · 支持断点续传");
    assert.equal(SENTINEL2_REFLECTANCE_BANDS[1].label, "B02 蓝光");
    assert.equal(sentinel2BandPresetLabel("true_color"), "真彩色");
    assert.equal(sarTaskDisplayName({ task_type: "orbit" }), "Sentinel-1 精密轨道下载");
    assert.equal(sarTaskTypeLabel({ dataset_id: "ERS" }), "ERS-1 / ERS-2（ASF 已下线）");
    assert.equal(
      sarTaskDatasetLabel({ batch_context: { query: { delivery_kind: "remote_derived_subset" } } }),
      "NISAR 派生子集 · client-derived",
    );
    assert.equal(nisarSubsetCapabilityMessage(null, "L"), "正在检查当前数据是否可以裁剪。");
    assert.equal(landsatAssetDisplayName("blue"), "蓝光 B2");
    assert.equal(landsatAssetRoleLabel("primary"), "科学波段");
    assert.equal(
      presentLocalReuse({ disposition: "reused", grade: "service_checksum" } as any, "completed")?.label,
      "复用已有文件（已核对服务方校验值）",
    );
    assert.equal(
      reacquireConfirmationText("scene.tif"),
      "将重新下载此项。原文件会另存为 scene.tif.eostage-conflict-…，下载完成后由您决定是否删除。",
    );
    assert.equal(
      opticalAdmissionReason("MPC_ASSET_SIGN_UNAVAILABLE"),
      "MPC 签名服务暂时不可用，可稍后重试",
    );

    setCurrentLocale("en");
    assert.equal(EMPTY_PREVIEW_STATE.message, "Activate a product first to request a preview.");
    assert.equal(previewErrorMessage("provider_no_preview"), "Provider did not supply a verifiable preview asset.");
    assert.equal(restrictedSarReason("ERS"), "ASF has retired this dataset from the search API");
    assert.equal(asfRestrictedAccessPresentation("JERS-1", ASF_DATASET_ACCESS_RESTRICTED)?.badge, "🔒 Restricted Access");
    assert.equal(DEM_DATASET_GROUPS[0].label, "Public AWS: No API Key Required");
    assert.equal(DEM_DATASET_OPTIONS[0].hint, "EGM2008 orthometric height · Resume supported");
    assert.equal(SENTINEL2_REFLECTANCE_BANDS[1].label, "B02 Blue");
    assert.equal(sentinel2BandPresetLabel("true_color"), "True Color");
    assert.equal(sarTaskDisplayName({ task_type: "orbit" }), "Sentinel-1 Precise Orbit Download");
    assert.equal(sarTaskTypeLabel({ dataset_id: "ERS" }), "ERS-1 / ERS-2 (Retired from ASF)");
    assert.equal(
      sarTaskDatasetLabel({ batch_context: { query: { delivery_kind: "remote_derived_subset" } } }),
      "NISAR Derived Subset · client-derived",
    );
    assert.equal(nisarSubsetCapabilityMessage(null, "L"), "Checking whether the current data supports subsetting.");
    assert.equal(landsatAssetDisplayName("blue"), "Blue B2");
    assert.equal(landsatAssetRoleLabel("primary"), "Science Band");
    assert.equal(
      presentLocalReuse({ disposition: "reused", grade: "service_checksum" } as any, "completed")?.label,
      "Reused existing file (verified against provider checksum)",
    );
    assert.equal(
      opticalAdmissionReason("MPC_ASSET_SIGN_UNAVAILABLE"),
      "MPC signing service is temporarily unavailable; retry shortly",
    );

    setCurrentLocale("ja");
    assert.equal(EMPTY_PREVIEW_STATE.message, "プレビューをリクエストするには、先にプロダクトを選択してください。");
    assert.equal(asfRestrictedAccessPresentation("JERS-1", ASF_DATASET_ACCESS_RESTRICTED)?.badge, "🔒 アクセス制限");
    assert.equal(SENTINEL2_REFLECTANCE_BANDS[1].label, "B02 青バンド");
    assert.equal(sentinel2BandPresetLabel("true_color"), "トゥルーカラー");
    assert.equal(sarTaskDisplayName({ task_type: "orbit" }), "Sentinel-1 精密軌道ダウンロード");
    assert.equal(landsatAssetDisplayName("blue"), "青バンド B2");
  } finally {
    setCurrentLocale("zh-CN");
  }
});

test("shared shell/api/ modules evaluate dynamically across zh-CN, en, and ja while keeping zh-CN byte-identical", async () => {
  try {
    setCurrentLocale("zh-CN");
    assert.equal(CARD_REASON_ZH.output_dir_unknown, "下载目录未知，请在卡上选择");
    assert.equal(
      autoCancelOutcomeText({ runCancelled: true, cancelledTaskIds: ["t1"] }),
      "已取消本次运行和 1 个任务",
    );
    assert.equal(
      projectErrorCard({ code: "cancelled", status: "cancelled" }).wording,
      "已取消",
    );
    assert.equal(KNOWN_NETWORK_SOURCES[2].badge_zh, "默认例外");
    assert.equal(PROXY_MODE_OPTIONS[0].label_zh, "直连（默认推荐）");
    assert.equal(SOURCE_OVERRIDE_OPTIONS[0].label_zh, "跟随全局");
    assert.equal(PROBE_ERROR_REASONS.timeout, "连接超时（8 秒内未响应）");
    assert.equal(validateProxyUrl("").error_zh, "代理地址不能为空");
    assert.equal(getStatusStateConfig("empty").title, "暂无数据");
    assert.equal(validateLocalOutputDir(""), "请输入下载目录");
    assert.equal(projectOutputDirRow({ output_dir_source: "default", output_dir: "D:\\Data" }, null).sourceLabel, "默认目录");
    assert.equal(formatDuration(65_000), "1 分 05 秒");
    assert.equal(workTitle({ request: "" } as any), "未记录请求");
    assert.equal(taskStatusLabel(null), "已清理");
    assert.equal(ROUTE_METADATA.workbench.label, "地图工作台");

    setCurrentLocale("en");
    assert.equal(CARD_REASON_ZH.output_dir_unknown, "Download directory is unknown; select one on the card");
    assert.deepEqual(
      projectApproval({ decision: "manual", mode: "confirm", card_reasons: ["size_unknown"] }).reasonsZh,
      ["Estimated size is unknown"],
    );
    assert.equal(
      autoCancelOutcomeText({ runCancelled: true, cancelledTaskIds: ["t1"] }),
      "Cancelled this run and 1 task",
    );
    assert.equal(KNOWN_NETWORK_SOURCES[2].badge_zh, "Default Exception");
    assert.equal(PROXY_MODE_OPTIONS[0].label_zh, "Direct Connection (Recommended Default)");
    assert.equal(SOURCE_OVERRIDE_OPTIONS[0].label_zh, "Follow Global");
    assert.equal(PROBE_ERROR_REASONS.timeout, "Connection timed out (no response within 8s)");
    assert.equal(validateProxyUrl("").error_zh, "Proxy address cannot be empty");
    assert.equal(getStatusStateConfig("empty").title, "No Data");
    assert.equal(validateLocalOutputDir(""), "Enter a download directory");
    assert.equal(projectOutputDirRow({ output_dir_source: "default", output_dir: "D:\\Data" }, null).sourceLabel, "Default directory");
    assert.equal(formatDuration(65_000), "1m 05s");
    assert.equal(workTitle({ request: "" } as any), "Unrecorded request");
    assert.equal(taskStatusLabel(null), "Cleaned up");
    assert.equal(ROUTE_METADATA.workbench.label, "Map Workbench");

    const cancelRes = await executeAutoCancel({
      getFreshTasks: async () => [],
      runCancel: async () => true,
      cancelTask: async () => {},
      runLive: true,
      runId: "r1",
      createdTaskIds: [],
    });
    assert.equal(cancelRes.outcomeText, "Cancelled this run");

    setCurrentLocale("ja");
    assert.equal(CARD_REASON_ZH.output_dir_unknown, "ダウンロード先が不明です。カード上で選択してください");
    assert.equal(KNOWN_NETWORK_SOURCES[2].badge_zh, "デフォルト例外");
    assert.equal(getStatusStateConfig("empty").title, "データなし");
    assert.equal(ROUTE_METADATA.workbench.label, "地図ワークベンチ");
  } finally {
    setCurrentLocale("zh-CN");
  }
});

test("resultListLogic failureSummaryText prefers localized code explanation in en/ja and message_zh in zh-CN", () => {
  const failure: any = {
    failed_items: 1,
    total_items: 1,
    codes: ["DISK_LOW"],
    message_zh: "后端原始磁盘不足提示。",
  };
  try {
    setCurrentLocale("zh-CN");
    assert.equal(failureSummaryText(failure), "后端原始磁盘不足提示。");

    setCurrentLocale("en");
    const enSummary = failureSummaryText(failure);
    assert.ok(enSummary && !/[\u4e00-\u9fff]/.test(enSummary), `Expected English explanation, got: ${enSummary}`);

    setCurrentLocale("ja");
    const jaSummary = failureSummaryText(failure);
    assert.notEqual(jaSummary, "后端原始磁盘不足提示。");
  } finally {
    setCurrentLocale("zh-CN");
  }
});
