/**
 * downloadFolderCatalog.ts
 * 下载目录「通用数据说明书 (README_数据说明.md)」与「QGIS 本地目录清单 (catalog.geojson)」生成器
 *
 * 设计目标：
 * - catalog.geojson 采用标准 OGC GeoJSON FeatureCollection (EPSG:4326)，可直接拖入 QGIS 自动识别所有下载图幅的空间范围框与属性表；
 * - README_数据说明.md 提供开箱即用的数据集说明、坐标系定义、分景文件清单及 QGIS / SNAP / GAMMA 加载指南。
 */

import type { ApplicationTask, TaskSceneItem } from "../shell/api/types.ts";
import { formatBytes } from "../shell/api/rowProjection.ts";
import { taskOutputDir } from "../shell/api/taskLogic.ts";
import {
  exportReproduciblePythonRecipe,
  OLMOEARTH_ARTIFACT_LICENSE_NOTICE_ZH,
} from "./eoAgentSuite.ts";

export interface CatalogGeoJsonFeature {
  type: "Feature";
  id: string;
  geometry: {
    type: "Polygon";
    coordinates: number[][][];
  };
  properties: {
    scene_id: string;
    task_id: string;
    task_name: string;
    dataset: string;
    provider: string;
    status: string;
    done_bytes: number;
    total_bytes: number | null;
    size_formatted: string;
    output_dir: string;
    relative_file_path: string;
    crs: string;
    created_at: string;
  };
}

export interface CatalogGeoJsonFeatureCollection {
  type: "FeatureCollection";
  name: string;
  crs: {
    type: "name";
    properties: {
      name: string;
    };
  };
  features: CatalogGeoJsonFeature[];
}

export interface DownloadFolderCatalogBundle {
  geojsonFileName: string;
  geojsonObject: CatalogGeoJsonFeatureCollection;
  geojsonText: string;
  readmeFileName: string;
  readmeMarkdown: string;
}

function inferExtensionForTask(task: ApplicationTask): string {
  const ds = (task.dataset || "").toLowerCase();
  if (ds.includes("dem") || ds.includes("cop30") || ds.includes("gee")) {
    return ".tif";
  }
  if (String(task.category).includes("orbit")) {
    return ".EOF";
  }
  return ".zip";
}

function extractBboxCoords(
  bboxLike: unknown
): [number, number, number, number] | null {
  if (
    Array.isArray(bboxLike) &&
    bboxLike.length === 4 &&
    bboxLike.every((v) => typeof v === "number" && Number.isFinite(v))
  ) {
    return [bboxLike[0], bboxLike[1], bboxLike[2], bboxLike[3]];
  }
  if (bboxLike && typeof bboxLike === "object" && "west" in bboxLike) {
    const b = bboxLike as { west: number; south: number; east: number; north: number };
    if (
      Number.isFinite(b.west) &&
      Number.isFinite(b.south) &&
      Number.isFinite(b.east) &&
      Number.isFinite(b.north)
    ) {
      return [b.west, b.south, b.east, b.north];
    }
  }
  return null;
}

function resolvePolygonForScene(
  task: ApplicationTask,
  _index: number,
  rawItem?: Record<string, unknown>
): number[][][] | null {
  // 1. 优先读取单景精确 geometry / footprint Polygon 坐标
  const geomCandidate = (rawItem?.geometry ?? rawItem?.footprint) as
    | { type?: string; coordinates?: unknown }
    | undefined;
  if (
    geomCandidate &&
    geomCandidate.type === "Polygon" &&
    Array.isArray(geomCandidate.coordinates) &&
    geomCandidate.coordinates.length > 0
  ) {
    return geomCandidate.coordinates as number[][][];
  }

  // 2. 其次读取单景 bbox / footprint_bbox，再回退到任务级 bbox
  const anyTask = task as unknown as { bbox?: unknown };
  const resolvedBbox =
    extractBboxCoords(rawItem?.bbox ?? rawItem?.footprint_bbox) ??
    extractBboxCoords(anyTask.bbox);

  if (!resolvedBbox) {
    // 事实优先：无真实坐标的场景直接跳过，绝不造假坐标
    return null;
  }

  const [west, south, east, north] = resolvedBbox;
  return [
    [
      [west, south],
      [east, south],
      [east, north],
      [west, north],
      [west, south],
    ],
  ];
}

function normalizeSceneEntries(
  task: ApplicationTask,
  explicitItems?: TaskSceneItem[]
): Array<{
  scene_id: string;
  status: string;
  done_bytes: number;
  total_bytes: number | null;
  raw?: Record<string, unknown>;
}> {
  const sourceItems =
    explicitItems && explicitItems.length > 0
      ? explicitItems
      : task.scene_items && task.scene_items.length > 0
      ? task.scene_items
      : null;

  if (sourceItems) {
    return sourceItems.map((item) => ({
      scene_id: item.scene_id,
      status: item.status,
      done_bytes: item.done_bytes ?? 0,
      total_bytes: item.total_bytes ?? null,
      raw: item as unknown as Record<string, unknown>,
    }));
  }

  if (task.scene_ids && task.scene_ids.length > 0) {
    const perSceneBytes =
      task.total_bytes && task.scene_ids.length > 0
        ? Math.round(task.total_bytes / task.scene_ids.length)
        : null;
    return task.scene_ids.map((id) => ({
      scene_id: id,
      status: task.status,
      done_bytes: perSceneBytes ?? 0,
      total_bytes: perSceneBytes,
    }));
  }

  return [
    {
      scene_id: task.name || task.task_id,
      status: task.status,
      done_bytes: task.done_bytes ?? 0,
      total_bytes: task.total_bytes ?? null,
    },
  ];
}

/**
 * 构建可直接拖入 QGIS 的本地目录清单 GeoJSON (FeatureCollection)
 */
export function buildDownloadFolderCatalogGeoJson(
  task: ApplicationTask,
  sceneItems?: TaskSceneItem[]
): CatalogGeoJsonFeatureCollection {
  const entries = normalizeSceneEntries(task, sceneItems);
  const ext = inferExtensionForTask(task);
  const outDir = taskOutputDir(task) || "./";
  const createdAt = task.updated_at || task.created_at || new Date().toISOString();

  const features: CatalogGeoJsonFeature[] = [];
  for (let idx = 0; idx < entries.length; idx++) {
    const entry = entries[idx];
    const poly = resolvePolygonForScene(task, idx, entry.raw);
    if (!poly) {
      // 事实优先：无真实坐标的场景直接跳过，不造假坐标要素
      continue;
    }
    const fileName = entry.scene_id.endsWith(ext)
      ? entry.scene_id
      : `${entry.scene_id}${ext}`;
    const bytes = entry.total_bytes ?? entry.done_bytes ?? 0;

    features.push({
      type: "Feature",
      id: `${task.task_id}:${entry.scene_id}`,
      geometry: {
        type: "Polygon",
        coordinates: poly,
      },
      properties: {
        scene_id: entry.scene_id,
        task_id: task.task_id,
        task_name: task.name,
        dataset: task.dataset || "unknown",
        provider: task.provider || "EOStage",
        status: entry.status,
        done_bytes: entry.done_bytes,
        total_bytes: entry.total_bytes,
        size_formatted: bytes > 0 ? formatBytes(bytes) : "--",
        output_dir: outDir,
        relative_file_path: `./${fileName}`,
        crs: "EPSG:4326",
        created_at: createdAt,
      },
    });
  }

  return {
    type: "FeatureCollection",
    name: `eostage_catalog_${task.task_id}`,
    crs: {
      type: "name",
      properties: {
        name: "urn:ogc:def:crs:OGC:1.3:CRS84",
      },
    },
    features,
  };
}

/**
 * 构建下载文件夹通用数据说明书 (README_数据说明.md)
 */
export function buildDownloadFolderReadmeMarkdown(
  task: ApplicationTask,
  sceneItems?: TaskSceneItem[]
): string {
  const entries = normalizeSceneEntries(task, sceneItems);
  const ext = inferExtensionForTask(task);
  const outDir = taskOutputDir(task) || "默认下载目录";
  const totalSize =
    typeof task.total_bytes === "number" && task.total_bytes > 0
      ? formatBytes(task.total_bytes)
      : formatBytes(task.done_bytes || 0);

  const rowsMd = entries
    .map((entry, idx) => {
      const fileName = entry.scene_id.endsWith(ext)
        ? entry.scene_id
        : `${entry.scene_id}${ext}`;
      const sizeStr =
        (entry.total_bytes ?? entry.done_bytes) > 0
          ? formatBytes(entry.total_bytes ?? entry.done_bytes)
          : "--";
      return `| ${idx + 1} | \`${entry.scene_id}\` | ${entry.status} | ${sizeStr} | \`./${fileName}\` |`;
    })
    .join("\n");

  const anyTask = task as unknown as {
    bbox?: unknown;
    time_range?: { start?: string; end?: string };
  };
  let resolvedBbox = extractBboxCoords(anyTask.bbox);
  if (!resolvedBbox) {
    for (const entry of entries) {
      resolvedBbox = extractBboxCoords(entry.raw?.bbox ?? entry.raw?.footprint_bbox);
      if (resolvedBbox) break;
    }
  }
  const startStr =
    anyTask.time_range?.start ||
    (task.created_at ? task.created_at.slice(0, 10) : "2026-01-01");
  const endStr =
    anyTask.time_range?.end ||
    (task.updated_at
      ? task.updated_at.slice(0, 10)
      : task.created_at
        ? task.created_at.slice(0, 10)
        : "2026-12-31");

  const recipeSection = resolvedBbox
    ? [
        `## 5. 零密钥可复现 Python 检索脚本 (Reproducible Recipe)`,
        `> ${OLMOEARTH_ARTIFACT_LICENSE_NOTICE_ZH}`,
        ``,
        `\`\`\`python`,
        exportReproduciblePythonRecipe({
          resourceId: task.dataset || "sentinel-1-grd",
          bbox: resolvedBbox,
          timeRange: { start: startStr, end: endStr },
          selectedSceneIds: entries.map((e) => e.scene_id),
          outputDir: outDir,
          includeOlmoEarthCompanion: true,
        }).trimEnd(),
        `\`\`\``,
        ``,
      ]
    : [];

  return [
    `# 通用数据说明书（EOStage 自动生成）`,
    ``,
    `## 1. 任务与数据集概览`,
    `- **任务名称**：${task.name}`,
    `- **任务 ID**：\`${task.task_id}\``,
    `- **数据平台 / 分发源**：${task.provider || "未知分发源"}`,
    `- **数据集标识**：${task.dataset || "遥感科学数据集"}`,
    `- **任务输出目录**：\`${outDir}\``,
    `- **清单导出说明**：通过浏览器导出目录清单文件 (catalog.geojson) 与数据说明书 (README_数据说明.md)；如需在 QGIS 中加载，请将导出的 catalog.geojson 移至数据所在目录或直接拖入 QGIS。`,
    `- **总景数 / 总体积**：${entries.length} 景 · ${totalSize}`,
    ``,
    `## 2. 空间参考与本地目录清单 (\`catalog.geojson\`)`,
    `- **目录清单格式**：标准 OGC GeoJSON \`FeatureCollection\`（坐标参考系 \`WGS 84 / EPSG:4326\`）。`,
    `- **QGIS 一键识别方法**：`,
    `  1. 打开 QGIS，直接将同目录下的 \`catalog.geojson\` 拖入地图画布或图层面板；`,
    `  2. QGIS 将自动渲染所有已下载影像的图幅范围多边形（Footprint）；`,
    `  3. 打开图层属性表即可查看每景影像的 \`scene_id\`、\`dataset\`、\`size_formatted\` 以及相对文件路径 \`relative_file_path\`。`,
    ``,
    `## 3. 分景数据清单`,
    `| 序号 | 景号 (Scene ID) | 状态 | 文件大小 | 相对路径 |`,
    `| :--- | :--- | :--- | :--- | :--- |`,
    rowsMd,
    ``,
    `## 4. 常见专业软件加载提示`,
    `- **QGIS / ArcGIS Pro**：先拖入 \`catalog.geojson\` 检视空间覆盖，再按需加载对应的 \`.tif\` 或解压后的影像波段。`,
    `- **ESA SNAP / GAMMA / ISCE3**：对于 SAR SLC/GRD 数据（\`.zip\`），建议保持原始压缩包命名直接导入处理流水线，避免破坏内部 \`manifest.safe\` 目录结构。`,
    ``,
    ...recipeSection,
  ].join("\n");
}

/**
 * 打包生成目录清单与说明书文本，并在浏览器/桌面端支持一键导出下载
 */
export function buildDownloadFolderCatalogBundle(
  task: ApplicationTask,
  sceneItems?: TaskSceneItem[]
): DownloadFolderCatalogBundle {
  const geojsonObject = buildDownloadFolderCatalogGeoJson(task, sceneItems);
  const geojsonText = JSON.stringify(geojsonObject, null, 2);
  const readmeMarkdown = buildDownloadFolderReadmeMarkdown(task, sceneItems);

  return {
    geojsonFileName: "catalog.geojson",
    geojsonObject,
    geojsonText,
    readmeFileName: "README_数据说明.md",
    readmeMarkdown,
  };
}

export function triggerBrowserTextDownload(fileName: string, content: string, mimeType: string): void {
  if (typeof document === "undefined" || typeof Blob === "undefined") return;
  if (typeof navigator !== "undefined" && /jsdom/i.test(navigator.userAgent || "")) return;
  try {
    const blob = new Blob([content], { type: mimeType });
    if (typeof URL === "undefined" || typeof URL.createObjectURL !== "function") return;
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = fileName;
    anchor.style.display = "none";
    document.body.appendChild(anchor);
    anchor.click();
    document.body.removeChild(anchor);
    URL.revokeObjectURL(url);
  } catch {
    // ignore in headless test environment
  }
}

export function exportDownloadFolderCatalogFiles(
  task: ApplicationTask,
  sceneItems?: TaskSceneItem[]
): DownloadFolderCatalogBundle {
  const bundle = buildDownloadFolderCatalogBundle(task, sceneItems);
  triggerBrowserTextDownload(
    bundle.geojsonFileName,
    bundle.geojsonText,
    "application/geo+json;charset=utf-8"
  );
  triggerBrowserTextDownload(
    bundle.readmeFileName,
    bundle.readmeMarkdown,
    "text/markdown;charset=utf-8"
  );
  return bundle;
}
