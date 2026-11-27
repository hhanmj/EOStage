/**
 * companionGacosFlow.ts
 * 配套下载 GACOS 申请编排模块（规划 + 提交申请入库）
 * 抽离独立模块以保持 DesktopWorkbench 职责清晰并符合静态边界约束。
 */

import {
  planGacosRequest,
  submitGacosRequest,
  type SceneRow,
  type Bbox,
} from "./bridge.ts";
import {
  type AuxiliaryExtentMode,
  resolveAuxiliaryExtentGeojson,
} from "./companionDownloadLogic.ts";

export interface ExecuteCompanionGacosParams {
  outputRoot: string;
  batchId?: string | null;
  itemIds: string[];
  scenes: SceneRow[];
  extentMode: AuxiliaryExtentMode;
  studyAreaName?: string | null;
  studyAreaGeometry?: Record<string, unknown> | null;
  studyAreaBbox?: Bbox | number[] | null;
}

export type ExecuteCompanionGacosOutcome =
  | {
      ok: true;
      taskId: string;
      submittedCount: number;
      requestGroupCount: number;
      manualActionRequired: boolean;
      message: string;
    }
  | {
      ok: false;
      error: string;
    };

/**
 * 提取与后端冻结检索批次（FrozenSearchBatch.candidates.metadata.scene_id）一致的规范 Sentinel-1 scene_id
 */
export function canonicalSentinel1SceneId(scene: SceneRow): string {
  const meta = (scene.native_metadata ?? {}) as Record<string, unknown>;
  if (typeof meta.scene_id === "string" && meta.scene_id.trim()) {
    return meta.scene_id.trim();
  }
  const sarSceneId = (
    scene as unknown as { sar_product?: { scene_id?: string | null } }
  ).sar_product?.scene_id;
  if (typeof sarSceneId === "string" && sarSceneId.trim()) {
    return sarSceneId.trim();
  }
  if (typeof scene.file_name === "string" && scene.file_name.trim()) {
    return scene.file_name.trim().replace(/\.(zip|safe)$/i, "");
  }
  const raw = (scene.scene_id || "").trim();
  if (raw.includes("::ITEM::")) {
    return raw
      .replace(/^.*::ITEM::/, "")
      .replace(/-(SLC|RAW|OCN|GRD[A-Z0-9_]*)$/i, "");
  }
  return raw;
}

/**
 * 为已确认的 Sentinel-1 下载任务同步创建并提交配套 GACOS 申请任务
 */
export async function executeCompanionGacosRequest(
  params: ExecuteCompanionGacosParams
): Promise<ExecuteCompanionGacosOutcome> {
  const uniqueIds = Array.from(
    new Set(params.itemIds.map((id) => id.trim()).filter(Boolean))
  );
  if (uniqueIds.length === 0) {
    return {
      ok: false,
      error: "未找到可用于 GACOS 申请的影像场景 ID",
    };
  }

  // 建立多键映射（兼容 PROVIDER::...::ITEM::... 与裸 scene_id）
  const sceneMap = new Map<string, SceneRow>();
  for (const s of params.scenes) {
    const keys = [
      s.scene_id,
      s.product_key,
      s.product_id,
      (s as unknown as { item_id?: string }).item_id,
      canonicalSentinel1SceneId(s),
    ];
    for (const k of keys) {
      if (typeof k === "string" && k.trim()) {
        sceneMap.set(k.trim(), s);
      }
    }
  }

  const normalizedMap = new Map<string, SceneRow>();
  for (const id of uniqueIds) {
    const matched = sceneMap.get(id);
    if (!matched) continue;
    const canonicalId = canonicalSentinel1SceneId(matched);
    if (!canonicalId) continue;
    if (!normalizedMap.has(canonicalId)) {
      normalizedMap.set(canonicalId, {
        ...matched,
        scene_id: canonicalId,
      });
    }
  }
  const matchedScenes = Array.from(normalizedMap.values());

  if (matchedScenes.length === 0) {
    return {
      ok: false,
      error: "所选影像缺少采集时间快照，无法规划 GACOS 申请日期",
    };
  }

  const resolvedGeojson = resolveAuxiliaryExtentGeojson(
    params.extentMode,
    params.studyAreaGeometry,
    params.studyAreaBbox,
    matchedScenes
  );
  const safeAoiName = (params.studyAreaName || "").trim() || "GACOS_StudyArea";

  try {
    const planned = await planGacosRequest(params.outputRoot, {
      source_kind: "online_selected",
      source_batch_id: params.batchId ?? undefined,
      selected_scene_ids: matchedScenes.map((s) => s.scene_id),
      snapshot_scenes: matchedScenes,
      ...(resolvedGeojson
        ? {
            aoi_source: "frozen_import" as const,
            aoi_geojson: structuredClone(resolvedGeojson),
            aoi_name: safeAoiName,
          }
        : {}),
      output_format: "geotiff",
    });

    if (!planned.ok) {
      return {
        ok: false,
        error: planned.error || "GACOS 申请规划未通过",
      };
    }

    const reportRec = (planned.report ?? {}) as Record<string, unknown>;
    if (reportRec.valid === false || reportRec.has_errors === true) {
      const issues = Array.isArray(reportRec.issues)
        ? (reportRec.issues as Record<string, unknown>[])
        : [];
      const issueMsg = issues
        .map((it) => (typeof it.message === "string" ? it.message : ""))
        .filter(Boolean)
        .join("；");
      return {
        ok: false,
        error: issueMsg || "GACOS 申请范围或日期校验未通过",
      };
    }

    const submitted = await submitGacosRequest(planned.plan, true, false, false);
    if (!submitted.ok) {
      if (submitted.code === "GACOS_DUPLICATE_REQUEST") {
        const planRec = (planned.plan ?? {}) as Record<string, unknown>;
        const fp =
          typeof planRec.plan_fingerprint_sha256 === "string"
            ? planRec.plan_fingerprint_sha256
            : "";
        return {
          ok: true,
          taskId: fp ? `gacos-request-${fp}` : "gacos-request-existing",
          submittedCount: 0,
          requestGroupCount: 1,
          manualActionRequired: true,
          message: "相同范围与日期的 GACOS 申请已提交，可在下载中心查看记录",
        };
      }
      if (submitted.code === "GACOS_EMAIL_NOT_CONFIGURED") {
        return {
          ok: false,
          error:
            "尚未配置 GACOS 接收邮箱，请先在「设置 -> 数据服务账号 -> GACOS 大气延迟服务」中保存接收邮箱",
        };
      }
      return {
        ok: false,
        error: submitted.error || "GACOS 申请提交未完成",
      };
    }

    return {
      ok: true,
      taskId: submitted.task_id,
      submittedCount: submitted.submitted_count,
      requestGroupCount: Array.isArray(submitted.request_groups)
        ? submitted.request_groups.length
        : Math.max(1, submitted.submitted_count),
      manualActionRequired: submitted.manual_action_required,
      message: submitted.message,
    };
  } catch (err) {
    return {
      ok: false,
      error: err instanceof Error ? err.message : String(err),
    };
  }
}

export interface GacosBatchSubmitState {
  status: "pending" | "submitting" | "submitted" | "duplicate" | "failed";
  taskId?: string;
  error?: string;
}

export interface ExecuteStandaloneGacosParams {
  outputRoot: string;
  manualDatesText?: string | null;
  startDate?: string | null;
  endDate?: string | null;
  utcHour?: number | null;
  utcMinute?: number | null;
  utcTime?: string | null;
  outputFormat?: "geotiff" | "binary" | null;
  studyAreaName?: string | null;
  studyAreaGeometry?: Record<string, unknown> | null;
  studyAreaBbox?: Bbox | number[] | null;
  /** Optional 0-based batch indices to submit (defaults to all split batches). */
  selectedBatchIndices?: readonly number[];
  /** Live per-batch progress callback invoked before/after each <=20-date batch submission. */
  onBatchStatusChange?: (batchIndex: number, state: GacosBatchSubmitState) => void;
}

export function splitGacosDatesIntoBatches(
  validDates: readonly string[],
  batchSize = 20
): string[][] {
  const step = Math.max(1, Math.floor(batchSize));
  const out: string[][] = [];
  for (let i = 0; i < validDates.length; i += step) {
    out.push(validDates.slice(i, i + step));
  }
  return out;
}

function isValidCalendarYyyymmdd(token: string): boolean {
  if (!/^\d{8}$/.test(token)) return false;
  const year = Number(token.slice(0, 4));
  const month = Number(token.slice(4, 6));
  const day = Number(token.slice(6, 8));
  if (year < 1 || year > 9999 || month < 1 || month > 12 || day < 1 || day > 31) {
    return false;
  }
  const dt = new Date(0);
  dt.setUTCFullYear(year, month - 1, day);
  dt.setUTCHours(0, 0, 0, 0);
  return (
    dt.getUTCFullYear() === year &&
    dt.getUTCMonth() === month - 1 &&
    dt.getUTCDate() === day
  );
}

function normalizeCompactGacosDate(raw?: string | null): string | null {
  const digits = (raw ?? "").replace(/[^0-9]/g, "");
  return isValidCalendarYyyymmdd(digits) ? digits : null;
}

export function parseGacosManualDatesList(rawText: string): {
  validDates: string[];
  invalidTokens: string[];
} {
  const tokens = rawText
    .split(/[\r\n,;\s]+/)
    .map((s) => s.trim())
    .filter(Boolean);
  const validDates: string[] = [];
  const invalidTokens: string[] = [];
  const seen = new Set<string>();
  for (const token of tokens) {
    if (!isValidCalendarYyyymmdd(token)) {
      invalidTokens.push(token);
      continue;
    }
    if (!seen.has(token)) {
      seen.add(token);
      validDates.push(token);
    }
  }
  return { validDates, invalidTokens };
}

function extractBboxSpans(
  bboxInput?: Bbox | number[] | null
): { lonSpan: number; latSpan: number } | null {
  if (!bboxInput) return null;
  if (Array.isArray(bboxInput) && bboxInput.length >= 4) {
    const [w, s, e, n] = bboxInput.map(Number);
    if ([w, s, e, n].every(Number.isFinite)) {
      return { lonSpan: Math.abs(e - w), latSpan: Math.abs(n - s) };
    }
    return null;
  }
  const obj = bboxInput as Bbox;
  if (
    Number.isFinite(obj.west) &&
    Number.isFinite(obj.south) &&
    Number.isFinite(obj.east) &&
    Number.isFinite(obj.north)
  ) {
    return {
      lonSpan: Math.abs(obj.east - obj.west),
      latSpan: Math.abs(obj.north - obj.south),
    };
  }
  return null;
}

function parseSingleGacosUtcTime(
  utcHour?: number | null,
  utcMinute?: number | null,
  utcTime?: string | null
): { hour: number; minute: number } | null {
  if (
    typeof utcHour === "number" &&
    Number.isInteger(utcHour) &&
    utcHour >= 0 &&
    utcHour <= 23 &&
    typeof utcMinute === "number" &&
    Number.isInteger(utcMinute) &&
    utcMinute >= 0 &&
    utcMinute <= 59
  ) {
    return { hour: utcHour, minute: utcMinute };
  }
  const match = (utcTime ?? "").trim().match(/^(\d{1,2}):(\d{2})$/);
  if (match) {
    const hour = Number(match[1]);
    const minute = Number(match[2]);
    if (hour >= 0 && hour <= 23 && minute >= 0 && minute <= 59) {
      return { hour, minute };
    }
  }
  return null;
}

/**
 * 独立从工作台主动按 GACOS 官方约束（0.05°–10° 范围、8 位 YYYYMMDD 列表、精确到分钟 UTC、GeoTIFF/Binary 格式）提交申请
 */
export async function executeStandaloneGacosRequest(
  params: ExecuteStandaloneGacosParams
): Promise<ExecuteCompanionGacosOutcome> {
  const resolvedGeojson = resolveAuxiliaryExtentGeojson(
    "study_area",
    params.studyAreaGeometry,
    params.studyAreaBbox,
    []
  );
  if (!resolvedGeojson) {
    return {
      ok: false,
      error: "请先在地图或行政区划搜索中设定研究区 AOI",
    };
  }

  const spans = extractBboxSpans(params.studyAreaBbox);
  if (spans) {
    if (spans.lonSpan > 10.0 || spans.latSpan > 10.0) {
      return {
        ok: false,
        error: `研究区跨度 (${spans.lonSpan.toFixed(2)}° × ${spans.latSpan.toFixed(2)}°) 超出 GACOS 官方单次最大限制（经纬度跨度均须 ≤ 10°），请缩小研究区后重试`,
      };
    }
    if (spans.lonSpan < 0.05 || spans.latSpan < 0.05) {
      return {
        ok: false,
        error: `研究区跨度 (${spans.lonSpan.toFixed(3)}° × ${spans.latSpan.toFixed(3)}°) 过小，建议经纬度跨度 ≥ 0.05° 且 ≤ 10°`,
      };
    }
  }

  let dates: string[] = [];
  if (params.manualDatesText !== undefined && params.manualDatesText !== null) {
    const parsed = parseGacosManualDatesList(params.manualDatesText);
    if (parsed.invalidTokens.length > 0) {
      return {
        ok: false,
        error: `GACOS 日期格式错误：${parsed.invalidTokens.slice(0, 3).join(", ")} 须为 8 位有效公历日期 YYYYMMDD（如 20260501）`,
      };
    }
    dates = parsed.validDates;
    if (dates.length === 0) {
      return {
        ok: false,
        error: "请至少填写一个 8 位影像日期 (YYYYMMDD)，再提交 GACOS 申请",
      };
    }
  } else {
    dates = Array.from(
      new Set(
        [
          normalizeCompactGacosDate(params.startDate),
          normalizeCompactGacosDate(params.endDate),
        ].filter((d): d is string => Boolean(d))
      )
    );
    if (dates.length === 0) {
      return {
        ok: false,
        error: "请至少填写一个 8 位影像日期 (YYYYMMDD)",
      };
    }
  }

  const slot = parseSingleGacosUtcTime(params.utcHour, params.utcMinute, params.utcTime);
  if (!slot) {
    return {
      ok: false,
      error: "请指定精确到分钟的卫星过境 UTC 时刻（小时 00–23，分钟 00–59）",
    };
  }

  const outputFormat: "geotiff" | "binary" =
    params.outputFormat === "binary" ? "binary" : "geotiff";

  const dateBatches = splitGacosDatesIntoBatches(dates, 20);
  const selectedSet =
    params.selectedBatchIndices && params.selectedBatchIndices.length > 0
      ? new Set(params.selectedBatchIndices)
      : null;

  const safeAoiName = (params.studyAreaName || "").trim() || "GACOS_StudyArea";
  let lastTaskId = "gacos-standalone";
  let totalSubmitted = 0;
  let totalGroups = 0;
  let manualRequired = false;
  const batchErrors: string[] = [];

  try {
    for (let batchIndex = 0; batchIndex < dateBatches.length; batchIndex += 1) {
      if (selectedSet && !selectedSet.has(batchIndex)) {
        continue;
      }
      const dateChunk = dateBatches[batchIndex]!;
      params.onBatchStatusChange?.(batchIndex, { status: "submitting" });
      const planned = await planGacosRequest(params.outputRoot, {
        source_kind: "manual_dates",
        manual_dates: dateChunk,
        hour: slot.hour,
        minute: slot.minute,
        aoi_source: "frozen_import",
        aoi_geojson: structuredClone(resolvedGeojson),
        aoi_name: safeAoiName,
        output_format: outputFormat,
      });
      if (!planned.ok) {
        const msg = planned.error || "GACOS 申请规划未通过";
        params.onBatchStatusChange?.(batchIndex, { status: "failed", error: msg });
        batchErrors.push(`第 ${batchIndex + 1} 批：${msg}`);
        continue;
      }
      const reportRec = (planned.report ?? {}) as Record<string, unknown>;
      if (reportRec.valid === false || reportRec.has_errors === true) {
        const issues = Array.isArray(reportRec.issues)
          ? (reportRec.issues as Record<string, unknown>[])
          : [];
        const issueMsg = issues
          .map((it) => (typeof it.message === "string" ? it.message : ""))
          .filter(Boolean)
          .join("；");
        const msg = issueMsg || "GACOS 申请范围或日期校验未通过";
        params.onBatchStatusChange?.(batchIndex, { status: "failed", error: msg });
        batchErrors.push(`第 ${batchIndex + 1} 批：${msg}`);
        continue;
      }
      const submitted = await submitGacosRequest(planned.plan, true, false, false);
      if (!submitted.ok) {
        if (submitted.code === "GACOS_DUPLICATE_REQUEST") {
          const planRec = (planned.plan ?? {}) as Record<string, unknown>;
          const fp =
            typeof planRec.plan_fingerprint_sha256 === "string"
              ? planRec.plan_fingerprint_sha256
              : "";
          const dupId = fp ? `gacos-request-${fp}` : lastTaskId;
          lastTaskId = dupId;
          totalGroups += 1;
          manualRequired = true;
          params.onBatchStatusChange?.(batchIndex, { status: "duplicate", taskId: dupId });
          continue;
        }
        if (submitted.code === "GACOS_EMAIL_NOT_CONFIGURED") {
          const emailErr =
            "尚未配置 GACOS 接收邮箱，请先在「设置 -> 数据服务账号 -> GACOS 大气延迟服务」中保存接收邮箱";
          params.onBatchStatusChange?.(batchIndex, { status: "failed", error: emailErr });
          return {
            ok: false,
            error: emailErr,
          };
        }
        const subErr = submitted.error || "GACOS 申请提交未完成";
        params.onBatchStatusChange?.(batchIndex, { status: "failed", error: subErr });
        batchErrors.push(`第 ${batchIndex + 1} 批：${subErr}`);
        continue;
      }
      lastTaskId = submitted.task_id;
      totalSubmitted += submitted.submitted_count;
      totalGroups += Array.isArray(submitted.request_groups)
        ? submitted.request_groups.length
        : Math.max(1, submitted.submitted_count);
      if (submitted.manual_action_required) manualRequired = true;
      params.onBatchStatusChange?.(batchIndex, { status: "submitted", taskId: submitted.task_id });
    }

    if (batchErrors.length > 0 && totalSubmitted === 0 && totalGroups === 0) {
      return {
        ok: false,
        error: batchErrors.join("；"),
      };
    }

    return {
      ok: true,
      taskId: lastTaskId,
      submittedCount: totalSubmitted,
      requestGroupCount: totalGroups,
      manualActionRequired: manualRequired,
      message:
        batchErrors.length > 0
          ? `GACOS 部分批次已提交（共 ${totalGroups} 批，其中 ${batchErrors.length} 批失败，可在右侧卡片单独重试）`
          : totalSubmitted > 0
          ? `已提交 GACOS 大气延迟申请（${dates.join(" / ")} · 共 ${totalGroups} 个时次批，可在下载中心查看与导入结果）`
          : `相同 AOI 与日期（${dates.join(" / ")}）的 GACOS 申请已存在，可在下载中心查看记录`,
    };
  } catch (err) {
    return {
      ok: false,
      error: err instanceof Error ? err.message : String(err),
    };
  }
}
