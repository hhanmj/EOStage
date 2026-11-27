import { t } from "../../i18n/index.ts";
import { formatByteSize } from "../../lib/formatByteSize.ts";

export interface StoragePanelInfo {
  displayPath: string;
  isConfigured: boolean;
  dbSize: string;
  diskFree: string;
  pickActionLabel: string;
}

export interface StorageLiveMetrics {
  dbSizeBytes?: number | null;
  diskFreeBytes?: number | null;
}

export function formatStoragePanelInfo(
  storageRootPath?: string | null,
  liveMetrics?: StorageLiveMetrics | null
): StoragePanelInfo {
  const trimmed = storageRootPath?.trim() || "";
  const dbSize =
    liveMetrics && typeof liveMetrics.dbSizeBytes === "number" && liveMetrics.dbSizeBytes >= 0
      ? formatByteSize(liveMetrics.dbSizeBytes, "--")
      : "--";
  const diskFree =
    liveMetrics && typeof liveMetrics.diskFreeBytes === "number" && liveMetrics.diskFreeBytes >= 0
      ? formatByteSize(liveMetrics.diskFreeBytes, "--")
      : "--";
  return {
    displayPath: trimmed.length > 0 ? trimmed : t("shared.settingsStorage.unconfiguredRootPath"),
    isConfigured: trimmed.length > 0,
    dbSize,
    diskFree,
    pickActionLabel: t("shared.settingsStorage.pickActionLabel"),
  };
}
