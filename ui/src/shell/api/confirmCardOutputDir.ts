import { t } from "../../i18n/index.ts";

export type OutputDirRow = {
  editable: boolean;
  dir: string | null;
  sourceLabel: string | null;
  approveBlocked: boolean;
  hint: string | null;
};

/**
 * 校验本地下载目录是否合法。
 * 逐条对齐 crates/insar-agent/src/permission.rs:68-80 valid_output_dir：
 * - 70-73 行：长度 ≥ 3、第 1 字符 ASCII 字母、第 2 字符 ':'、第 3 字符 '\' 或 '/'；拒绝相对路径、UNC、设备路径等；
 * - 74 行：去首尾空白后相等（不得以空格开头或结尾）；
 * - 75 行：UTF-8 字节长度 ≤ 1024；
 * - 76 行：不含控制字符；
 * - 77-79 行：按 '\' 和 '/' 切分后任何一段都不是 '.' 或 '..'。
 * 合法返回 null，否则返回用户友好的中文原因。
 */
export function validateLocalOutputDir(raw: string): string | null {
  if (typeof raw !== "string" || raw.length === 0) {
    return t("shared.confirmCardOutputDir.enterDownloadDir");
  }

  // 74 行：path.trim() == path
  if (raw.trim() !== raw) {
    return t("shared.confirmCardOutputDir.noLeadingTrailingSpaces");
  }

  // 75 行：path.len() <= 1024（UTF-8 字节数）
  const byteLen = new TextEncoder().encode(raw).length;
  if (byteLen > 1024) {
    return t("shared.confirmCardOutputDir.dirTooLong");
  }

  // 70-73 行：长度 ≥ 3，首字符 ASCII 字母，次字符 ':'，第三字符 '\' 或 '/'
  if (raw.length < 3 || !/^[A-Za-z]:[\\/]/.test(raw)) {
    return t("shared.confirmCardOutputDir.localDrivePathOnly");
  }

  // 76 行：!path.chars().any(char::is_control)
  if (/[\u0000-\u001f\u007f-\u009f]/.test(raw)) {
    return t("shared.confirmCardOutputDir.noControlChars");
  }

  // 77-79 行：!path.split(['\\', '/']).any(|segment| segment == "." || segment == "..")
  const segments = raw.split(/[\\/]/);
  for (const seg of segments) {
    if (seg === "." || seg === "..") {
      return t("shared.confirmCardOutputDir.noDotSegments");
    }
  }

  return null;
}

/**
 * 判断快照是否属于 create_task（而非 task_action / update_download_task）。
 * 后端事实：create_task 始终写出 output_dir_source（可能为 null）且 already_queued_count 为数字；
 * task_action 无 output_dir_source 键且 already_queued_count 为 null。
 */
export function isCreateTaskSnapshot(snapshot: unknown): boolean {
  if (snapshot === null || typeof snapshot !== "object") {
    return false;
  }
  if (Object.prototype.hasOwnProperty.call(snapshot, "output_dir_source")) {
    return true;
  }
  const queued = (snapshot as Record<string, unknown>).already_queued_count;
  return typeof queued === "number" && Number.isFinite(queued);
}

function extractSnapshotDir(snapshot: unknown): string | null {
  if (snapshot !== null && typeof snapshot === "object" && "output_dir" in snapshot) {
    const raw = (snapshot as Record<string, unknown>).output_dir;
    if (typeof raw === "string") {
      const trimmed = raw.trim();
      return trimmed.length > 0 ? raw : null;
    }
  }
  return null;
}

/**
 * 将确认卡快照和卡上选定目录投影为界面展示行数据。
 */
export function projectOutputDirRow(
  snapshot: unknown,
  chosen: string | null,
): OutputDirRow {
  const editable = isCreateTaskSnapshot(snapshot);

  let dir: string | null = null;
  if (chosen !== null) {
    dir = chosen.trim().length > 0 ? chosen : null;
  } else {
    dir = extractSnapshotDir(snapshot);
  }

  let sourceLabel: string | null = null;
  if (chosen !== null) {
    sourceLabel = t("shared.confirmCardOutputDir.sourceCardSelected");
  } else if (snapshot !== null && typeof snapshot === "object" && "output_dir_source" in snapshot) {
    const rawSource = (snapshot as Record<string, unknown>).output_dir_source;
    if (rawSource === "default") {
      sourceLabel = t("shared.confirmCardOutputDir.sourceDefaultDir");
    } else if (rawSource === "model_suggested") {
      sourceLabel = t("shared.confirmCardOutputDir.sourceModelSuggested");
    } else if (rawSource === "chosen_on_card") {
      sourceLabel = t("shared.confirmCardOutputDir.sourceCardSelected");
    } else if (rawSource === "from_message") {
      sourceLabel = t("shared.confirmCardOutputDir.sourceMessageSpecified");
    } else {
      sourceLabel = null;
    }
  }

  const approveBlocked = editable && dir === null;
  const hint = approveBlocked ? t("shared.confirmCardOutputDir.selectDirFirstHint") : null;

  return {
    editable,
    dir,
    sourceLabel,
    approveBlocked,
    hint,
  };
}

/**
 * 确定调用 agent_plan_approve 时的第三参数 output_dir。
 * 仅在快照为 create_task、chosen 存在且合法、且与快照原样目录不同时传回 chosen；其余传 undefined。
 */
export function approveOutputDirArg(
  snapshot: unknown,
  chosen: string | null,
): string | undefined {
  if (!isCreateTaskSnapshot(snapshot)) {
    return undefined;
  }
  if (chosen === null) {
    return undefined;
  }
  if (validateLocalOutputDir(chosen) !== null) {
    return undefined;
  }

  const snapshotDirRaw =
    snapshot !== null &&
    typeof snapshot === "object" &&
    "output_dir" in snapshot &&
    typeof (snapshot as Record<string, unknown>).output_dir === "string"
      ? ((snapshot as Record<string, unknown>).output_dir as string)
      : null;

  if (chosen === snapshotDirRaw) {
    return undefined;
  }

  return chosen;
}

/**
 * 判定是否向用户提供「使用默认目录」快捷选项。
 * defaultDir 去首尾空白后非空、校验合法、且不等于当前目录时返回该目录，否则 null。
 */
export function defaultDirOffer(
  defaultDir: string | null | undefined,
  currentDir: string | null,
): string | null {
  if (typeof defaultDir !== "string") {
    return null;
  }
  const trimmed = defaultDir.trim();
  if (trimmed.length === 0) {
    return null;
  }
  if (validateLocalOutputDir(trimmed) !== null) {
    return null;
  }
  if (trimmed === currentDir) {
    return null;
  }
  return trimmed;
}
