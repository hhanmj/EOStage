export const TEMPORARY_DOWNLOAD_PATH_ERROR_ZH =
  "不能将系统临时目录（如 /tmp）设为持久化下载目录，请选择用户目录（如文稿/下载/桌面）或数据盘文件夹。";

export function isTemporarySystemDownloadPath(rawPath?: string | null): boolean {
  if (!rawPath) return false;
  const norm = rawPath.trim().replace(/\\/g, "/").toLowerCase();
  return (
    norm === "/tmp" ||
    norm.startsWith("/tmp/") ||
    norm === "/private/tmp" ||
    norm.startsWith("/private/tmp/") ||
    norm === "/var/tmp" ||
    norm.startsWith("/var/tmp/") ||
    norm.startsWith("/var/folders/") ||
    norm.startsWith("/private/var/folders/") ||
    norm.includes("/appdata/local/temp/") ||
    norm.startsWith("c:/windows/temp")
  );
}

export function nonTemporaryOutputDir(rawPath?: string | null): string {
  const trimmed = (rawPath || "").trim();
  return trimmed && !isTemporarySystemDownloadPath(trimmed) ? trimmed : "";
}

/**
 * Strip provider catalog routing prefixes such as `PROVIDER::...::ITEM::`
 * when presenting product IDs to end users in confirmation/subset modals.
 */
export function formatHumanSceneLabel(scene: {
  scene_id: string;
  file_name?: string | null;
  title?: string | null;
}): string {
  const candidateName = scene.file_name?.trim() || scene.title?.trim() || "";
  if (candidateName && !candidateName.startsWith("PROVIDER::")) {
    return candidateName;
  }
  const itemMarker = "::ITEM::";
  const markerIdx = scene.scene_id.lastIndexOf(itemMarker);
  if (markerIdx !== -1) {
    const tail = scene.scene_id.slice(markerIdx + itemMarker.length).trim();
    if (tail) return tail;
  }
  return scene.scene_id;
}
