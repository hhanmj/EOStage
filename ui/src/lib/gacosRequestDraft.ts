export type GacosRequestMode = "online_batch" | "manual_dates";
export type GacosOutputFormat = "geotiff" | "binary";

/** A display-only reference captured from the latest Sentinel-1 search batch. */
export type GacosSentinelBatchReference = {
  batchId: string;
  label: string;
  totalCount: number;
};

export type GacosAoiDraft = {
  name: string;
  geometry: Record<string, unknown>;
};

export type GacosRequestDraft = {
  mode: GacosRequestMode;
  /** Frozen only after the user explicitly imports a latest-batch reference. */
  sourceBatch: GacosSentinelBatchReference | null;
  /** This is a request-local AOI snapshot; it does not change the working AOI. */
  aoi: GacosAoiDraft | null;
  /** Manual dates are entered one YYYYMMDD value per line. */
  datesText: string;
  /** Stored as select values so the native planner can distinguish empty from 0. */
  hour: string;
  minute: string;
  outputFormat: GacosOutputFormat;
};

export const initialGacosRequestDraft: GacosRequestDraft = {
  mode: "online_batch",
  sourceBatch: null,
  aoi: null,
  datesText: "",
  hour: "",
  minute: "",
  outputFormat: "geotiff",
};

export type GacosRequestDraftValidation = {
  valid: boolean;
  errors: string[];
  /** Canonical, first-seen, unique YYYYMMDD values from manual input. */
  dates: string[];
};

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function validPolygonalGeometry(value: unknown): boolean {
  const geometry = record(value);
  return !!geometry
    && (geometry.type === "Polygon" || geometry.type === "MultiPolygon")
    && Array.isArray(geometry.coordinates);
}

function validUtcPart(value: string, maximum: number): boolean {
  const text = value.trim();
  if (!/^\d+$/.test(text)) return false;
  const number = Number(text);
  return Number.isInteger(number) && number >= 0 && number <= maximum;
}

function parseCalendarDate(value: string): boolean {
  if (!/^\d{8}$/.test(value)) return false;
  const year = Number(value.slice(0, 4));
  const month = Number(value.slice(4, 6));
  const day = Number(value.slice(6, 8));
  if (year < 1 || year > 9999 || month < 1 || month > 12 || day < 1 || day > 31) return false;

  // setUTCFullYear avoids Date's special 1900 offset for years 0 through 99.
  const date = new Date(0);
  date.setUTCFullYear(year, month - 1, day);
  date.setUTCHours(0, 0, 0, 0);
  return date.getUTCFullYear() === year
    && date.getUTCMonth() === month - 1
    && date.getUTCDate() === day;
}

function validateManualDates(datesText: string, errors: string[]): string[] {
  const lines = datesText.split(/\r?\n/).map((line) => line.trim()).filter(Boolean);
  if (lines.length === 0) {
    errors.push("请填写至少一个 GACOS 日期，每行使用 8 位 YYYYMMDD。");
    return [];
  }

  const dates: string[] = [];
  const seen = new Set<string>();
  for (const line of lines) {
    if (!/^\d{8}$/.test(line)) {
      errors.push(`日期“${line}”必须是 8 位 YYYYMMDD。`);
      continue;
    }
    if (!parseCalendarDate(line)) {
      errors.push(`日期“${line}”不是有效的日历日期。`);
      continue;
    }
    if (!seen.has(line)) {
      seen.add(line);
      dates.push(line);
    }
  }
  if (dates.length === 0 && lines.length > 0) {
    errors.push("没有可用于 GACOS 请求的有效日期。");
  }
  return dates;
}

function validateTime(
  draft: GacosRequestDraft,
  errors: string[],
): void {
  const hour = draft.hour.trim();
  const minute = draft.minute.trim();
  const hasHour = hour.length > 0;
  const hasMinute = minute.length > 0;

  if (draft.mode === "manual_dates") {
    if (!hasHour) errors.push("手填日期模式必须选择 UTC 小时。");
    else if (!validUtcPart(hour, 23)) errors.push("UTC 小时必须是 0–23 的整数。");
    if (!hasMinute) errors.push("手填日期模式必须选择 UTC 分钟。");
    else if (!validUtcPart(minute, 59)) errors.push("UTC 分钟必须是 0–59 的整数。");
    return;
  }

  // Online scenes normally carry acquisition UTC. These two values are only
  // an explicit fallback for rows that contain a date without a time.
  if (hasHour !== hasMinute) {
    errors.push("在线日期补时需要同时选择 UTC 小时和分钟。");
  }
  if (hasHour && !validUtcPart(hour, 23)) errors.push("UTC 小时必须是 0–23 的整数。");
  if (hasMinute && !validUtcPart(minute, 59)) errors.push("UTC 分钟必须是 0–59 的整数。");
}

/**
 * Validate a request-local draft without reading Store state or doing network
 * work. Native planning remains responsible for resolving the frozen batch.
 */
export function validateGacosRequestDraft(
  draft: GacosRequestDraft,
): GacosRequestDraftValidation {
  const errors: string[] = [];
  let dates: string[] = [];

  if (draft.mode === "online_batch") {
    const sourceBatch = record(draft.sourceBatch);
    if (!sourceBatch) {
      errors.push("在线模式请先点击“导入最新 Sentinel-1 结果”冻结批次。");
    } else {
      if (typeof sourceBatch.batchId !== "string" || !sourceBatch.batchId.trim()) {
        errors.push("冻结的 Sentinel-1 批次缺少有效 batch ID。");
      }
      if (typeof sourceBatch.label !== "string" || !sourceBatch.label.trim()) {
        errors.push("冻结的 Sentinel-1 批次缺少可读名称。");
      }
      if (
        typeof sourceBatch.totalCount !== "number"
        || !Number.isSafeInteger(sourceBatch.totalCount)
        || sourceBatch.totalCount < 1
      ) {
        errors.push("冻结的 Sentinel-1 批次必须包含至少一景结果。");
      }
    }
  } else if (draft.mode === "manual_dates") {
    dates = validateManualDates(draft.datesText, errors);
  } else {
    errors.push("GACOS 来源模式无效。");
  }

  if (!draft.aoi) {
    errors.push("请先导入当前 AOI 或本地 AOI 文件。");
  } else {
    if (!draft.aoi.name.trim()) errors.push("AOI 名称不能为空。");
    if (!validPolygonalGeometry(draft.aoi.geometry)) {
      errors.push("AOI 必须包含真实的 Polygon 或 MultiPolygon 几何。");
    }
  }

  validateTime(draft, errors);

  if (draft.outputFormat !== "geotiff" && draft.outputFormat !== "binary") {
    errors.push("请选择 GeoTIFF 或 Binary grid 输出格式。");
  }

  return {
    valid: errors.length === 0,
    errors: [...new Set(errors)],
    dates,
  };
}
