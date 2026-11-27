import type { Locale } from "../i18n/types.ts";
import type { AgentSkill, SkillDiscipline } from "./agentSkills.ts";

export interface PluginSuiteLocalization {
  name: string;
  description: string;
  tags: string[];
  skillName?: string;
  skillDescription?: string;
  connectorDescription?: string;
}

export const BUNDLED_PLUGIN_LOCALIZATION: Record<
  string,
  Partial<Record<Exclude<Locale, "zh-CN">, PluginSuiteLocalization>>
> = {
  "eostage-gee-plugin": {
    en: {
      name: "Google Earth Engine Science Catalog & Cloud Compute Plugin",
      description:
        "Integrates GEE public catalog search, AOI-clipped GeoTIFF exports, spectral/SAR indices, and XYZ tile map previews.",
      tags: ["Google Earth Engine", "GEE Cloud Compute", "Online AOI Clip", "MCP Connector"],
      skillName: "Google Earth Engine (GEE) Catalog & Cloud Compute Guide",
      skillDescription:
        "Query built-in GEE catalogs, export AOI-clipped GeoTIFFs, compute spectral indices, and overlay map tile layers",
      connectorDescription:
        "Built-in MCP connector for GEE catalog search, cloud clipping, index compute, and map tile previews",
    },
    ja: {
      name: "Google Earth Engine 科学カタログ＆クラウド解析プラグイン",
      description:
        "GEE 公開カタログ検索、AOI 切り抜き GeoTIFF 出力、スペクトル・SAR 指数演算、XYZ タイルマッププレビューを統合。",
      tags: ["Google Earth Engine", "GEE クラウド計算", "AOI 切り抜き", "MCP コネクタ"],
      skillName: "Google Earth Engine (GEE) カタログ検索・クラウド計算ガイド",
      skillDescription:
        "内蔵 GEE カタログ検索、AOI 指定 GeoTIFF エクスポート、オンライン指数計算およびタイルレイヤー重畳の手順",
      connectorDescription:
        "GEE 科学カタログ検索・クラウド切り抜き・指数演算・地図タイルプレビュー内蔵 MCP コネクタ",
    },
  },
  "eostage-gdal-stream-plugin": {
    en: {
      name: "GDAL /vsicurl/ Cloud COG Streaming & Terrain/Index Suite",
      description:
        "Powered by local Native GIS (GDAL 3.12 + PROJ 9.7): streams remote STAC/COGs via HTTP Range without full downloads to compute slope, aspect, hillshade, TPI/TRI, roughness, and NDVI/NDWI/NBR.",
      tags: ["GDAL /vsicurl/", "COG Streaming", "Terrain Operators", "Spectral Indices"],
      skillName: "GDAL /vsicurl/ Cloud-Optimized GeoTIFF Streaming Workflow",
      skillDescription:
        "Read remote STAC/COG bounding boxes via HTTP Range without downloading full archives and derive terrain/spectral indices",
    },
    ja: {
      name: "GDAL /vsicurl/ オンライン COG ストリーミング・地形/指数解析プラグイン",
      description:
        "ローカル Native GIS (GDAL 3.12 + PROJ 9.7) による /vsicurl/ HTTP Range ストリーミング演算群。全体をダウンロードせずに勾配・方位・陰影起伏・NDVI/NDWI/NBR を直接算出します。",
      tags: ["GDAL /vsicurl/", "COG ストリーミング", "地形演算", "スペクトル指数"],
      skillName: "GDAL /vsicurl/ クラウド最適化 GeoTIFF ストリーミング手順",
      skillDescription:
        "全画像を取得せずに HTTP Range でリモート STAC/COG の必要範囲のみを読み取り地形・指数解析を実行します",
    },
  },
  "eostage-open-geodata-plugin": {
    en: {
      name: "Open GeoData, Multi-Mirror DEM & Coordinate Transform Suite",
      description:
        "Unifies key-free global DEM mirrors (COP30/COP90/NASADEM/AW3D30), administrative boundaries, ERA5 weather, Overpass OSM, WorldPop, and WGS84/GCJ02/BD09/CGCS2000 transforms.",
      tags: ["Open GeoData", "Multi-Mirror DEM", "Coordinate Transform", "ERA5 / OSM"],
      skillName: "Multi-Source Open GeoData & Coordinate Alignment Workflow",
      skillDescription:
        "Fetch administrative boundaries, multi-mirror DEM tiles, ERA5 precipitation, and OSM vectors with automatic CRS normalization",
    },
    ja: {
      name: "オープン地理データ・マルチミラー DEM・座標変換スイート",
      description:
        "キー不要の全球 DEM ミラー (COP30/COP90/NASADEM/AW3D30)、行政境界、ERA5 気象、Overpass OSM、WorldPop、座標系変換 (WGS84/GCJ02/CGCS2000) を統合。",
      tags: ["Open GeoData", "マルチミラー DEM", "座標系変換", "ERA5 / OSM"],
      skillName: "オープン地理データ取得・座標系整合ワークフロー",
      skillDescription:
        "対象地域の行政境界・DEM タイル・ERA5 降水量・OSM ベクターを一括取得し、標準座標系へ自動変換します",
    },
  },
  "eostage-mapgis-compat-plugin": {
    en: {
      name: "MapGIS 6.x (.wt/.wl/.wp) Topology & Shapefile/GeoJSON Converter",
      description:
        "Reads MapGIS 6.x binary (WMAP D21/D22/D23) and ASCII exchange formats (.wt/.wl/.wp), builds directed polygon rings, and exports clean Shapefile or GeoJSON.",
      tags: ["MapGIS 6.x", ".wt/.wl/.wp", "Topology Build", "Shapefile Export"],
      skillName: "Historical MapGIS (.wt/.wl/.wp) Topology & QA Workflow",
      skillDescription:
        "Inspect MapGIS headers, reconstruct outer rings and inner island topology, and export standard Shapefiles or GeoJSON",
    },
    ja: {
      name: "MapGIS 6.x (.wt/.wl/.wp) トポロジ解析・Shapefile/GeoJSON 変換プラグイン",
      description:
        "歴史的地形図・地質図向けの MapGIS 6.x バイナリ (WMAP) および交換形式 (.wt/.wl/.wp) を解析し、ポリゴン環トポロジを構築して Shapefile / GeoJSON へ無損失出力します。",
      tags: ["MapGIS 6.x", ".wt/.wl/.wp", "トポロジ構築", "Shapefile 変換"],
      skillName: "地質図 MapGIS (.wt/.wl/.wp) トポロジ変換・品質検査手順",
      skillDescription:
        "MapGIS 点・線・面データのヘッダー検査と外周/内島トポロジ復元を行い Shapefile や GeoJSON を一括生成します",
    },
  },
};

export const GEOSCIENCE_SKILL_JA_NAMES: Record<string, string> = {
  "remote-sensing-analysis": "リモートセンシング画像解析",
  "change-detection": "衛星画像変化抽出",
  "terrain-hydrology": "地形・水文解析",
  "point-cloud-lidar": "点群データ・LiDAR・鉛直基準",
  "geospatial-converter": "空間座標変換・CAD 納品",
  "cartography-geoviz": "主題地図デザイン・可視化",
  "ecmwf-open-data": "ECMWF オープン気象予測",
  "radar-satellite-analysis": "SAR レーダー・衛星画像判読",
  "geo-deep-learning": "リモートセンシング深層学習",
  "geostatistics-interpolation": "地球統計学・クリギング補間",
  "spatial-statistics": "空間統計解析",
  "spatial-sql": "空間 SQL・空間インデックス (PostGIS / DuckDB)",
  "network-accessibility-analysis": "ネットワーク到達圏・経路解析",
  loopstructural: "3次元暗黙的地質モデリング (LoopStructural)",
  "near-surface-geophysics": "浅層物理探査データ解釈 (GPR / ERT / MT)",
  pastas: "地下水位時系列モデリング (Pastas)",
  "dem-datum-conversion": "DEM 鉛直測地系変換",
  "s1-precise-orbit": "Sentinel-1 精密軌道照合",
  "nisar-rslc-subset": "NISAR RSLC リモート範囲切り抜き",
  "s1-tops-burst": "Sentinel-1 TOPS バースト単位検索",
  "gacos-ztd-atmo": "GACOS 対流圏大気補正",
  "skill-creator": "地球科学スキルビルダー",
  "gee-textbook-cookbook": "GEE 衛星データ・地球科学クラウド解析ガイド",
  "sar-insar-textbook": "SAR 物理機構・InSAR 時系列変動解析ガイド",
  "ocean-tides-bathymetry": "海洋潮汐・全球海底地形ワークフロー (EOT20 / pyTMD / GEBCO)",
  "cryosphere-glacier-dem": "雪氷・氷河 DEM 3次元位置合わせ手順 (xDEM)",
  "landcover-change-clcd": "年次 30m 土地被覆・災害暴露評価手順 (WHU CLCD)",
  "eo-foundation-models": "地球観測基盤モデル・TorchGeo 深層学習手順",
  "eo-analysis-recipes": "地球観測・GEE エコハイドロロジー解析レシピ集",
  "eo-agent-capability-routing": "地球観測 Agent 意図解析・ツールルーティング",
  "eo-model-explanation-review": "地球観測モデル説明・誤差分析レビュー",
  "eo-model-transfer-evaluation": "地球観測モデル地域転用評価・選定",
  "eo-workbench-concept": "地球観測ワークベンチ状態管理・納品設計",
  "core-insar-chain": "InSAR 時系列データ取得統合ワークフロー",
  "core-sar-s1": "Sentinel-1 / NISAR 迅速検索",
  "core-optical-cloud": "低雲量光学マルチスペクトル選定",
  "core-dem-convert": "DEM 計画・標高基準変換",
  "builtin-insar-copilot-core": "EOStage データ準備・タスク編成コア規程",
  "builtin-s1-insar-pair-workflow": "Sentinel-1 時空間基線ペア選定ワークフロー",
  "builtin-dem-datum-alignment-workflow": "DEM 標高基準自動整合ワークフロー",
  "builtin-precise-orbit-poeorb-workflow": "Sentinel 精密軌道 POEORB 一括照合ワークフロー",
};

const SKILL_BADGE_MAP: Record<Exclude<Locale, "zh-CN">, Record<string, string>> = {
  en: { 官方: "Official", 内置: "Built-in", 自定义: "Custom" },
  ja: { 官方: "公式", 内置: "組み込み", 自定义: "カスタム" },
};

const SKILL_AUTHOR_MAP: Record<Exclude<Locale, "zh-CN">, Record<string, string>> = {
  en: {
    自定义: "Custom",
    本地导入: "Local Import",
    远程导入: "Remote Import",
    内置工作流: "Built-in Workflow",
    "EOStage 扩展库": "EOStage Hub",
  },
  ja: {
    自定义: "カスタム",
    本地导入: "ローカル導入",
    远程导入: "リモート導入",
    内置工作流: "組み込みワークフロー",
    "EOStage 扩展库": "EOStage ハブ",
  },
};

export function localizeSkillBadge(badge: string | undefined, locale: Locale): string | undefined {
  if (!badge || locale === "zh-CN") return badge;
  return SKILL_BADGE_MAP[locale][badge] ?? badge;
}

export function localizeSkillAuthor(author: string, locale: Locale): string {
  if (locale === "zh-CN") return author;
  return SKILL_AUTHOR_MAP[locale][author] ?? author;
}

export function localizeSkillDisciplineLabel(
  discipline: SkillDiscipline | undefined,
  fallbackLabel: string | undefined,
  locale: Locale,
): string | undefined {
  if (!discipline || locale === "zh-CN") return fallbackLabel;
  const labels: Record<Exclude<Locale, "zh-CN">, Record<SkillDiscipline, string>> = {
    en: {
      "earth-env": "Earth & Environmental",
      "gis-spatial": "GIS & Spatial",
      "insar-sar": "SAR & InSAR",
      "skill-engineering": "Workflow Engineering",
      custom: "Custom",
    },
    ja: {
      "earth-env": "地球・環境科学",
      "gis-spatial": "GIS・空間変換",
      "insar-sar": "SAR・レーダー測地",
      "skill-engineering": "スキル設計・連携",
      custom: "カスタム",
    },
  };
  return labels[locale][discipline] ?? fallbackLabel;
}

export function localizeAgentSkillForDisplay(skill: AgentSkill, locale: Locale): AgentSkill {
  if (locale === "zh-CN") return skill;
  // Check bundled plugin skills first
  for (const [pluginId, suiteMap] of Object.entries(BUNDLED_PLUGIN_LOCALIZATION)) {
    const loc = suiteMap[locale];
    if (!loc?.skillName) continue;
    if (
      skill.id === `plugin-skill-${pluginId}` ||
      skill.id.startsWith(`plugin-skill-${pluginId}-`) ||
      skill.name === loc.skillName
    ) {
      return {
        ...skill,
        name: loc.skillName,
        description: loc.skillDescription ?? skill.description,
        overview: loc.skillDescription ?? skill.overview,
        badge: (localizeSkillBadge(skill.badge, locale) ?? skill.badge) as AgentSkill["badge"],
        sourceLabel: localizeSkillSourceLabel(skill.sourceLabel, locale),
        disciplineLabel: localizeSkillDisciplineLabel(skill.discipline, skill.disciplineLabel, locale),
      };
    }
  }
  const jaName = GEOSCIENCE_SKILL_JA_NAMES[skill.id];
  const displayName =
    locale === "en"
      ? skill.nameEn || skill.name
      : jaName || skill.nameEn || skill.name;
  const localizedDesc =
    locale === "en"
      ? `${skill.nameEn || skill.name} domain skill package and operational SKILL.md workflow.`
      : `${jaName || skill.nameEn || skill.name} の専門スキルパッケージおよび運用手順 (SKILL.md)。`;
  const hasCjkDesc = /[\u4e00-\u9fff]/.test(skill.description);
  const hasCjkOverview = skill.overview ? /[\u4e00-\u9fff]/.test(skill.overview) : false;
  const localizedAuthor =
    skill.author === "开源地球科学社区"
      ? locale === "en"
        ? "Open Geoscience Community"
        : "オープン地球科学コミュニティ"
      : skill.author === "EOStage 扩展库"
        ? locale === "en"
          ? "EOStage Hub"
          : "EOStage ハブ"
        : skill.author;
  const localizedLicense =
    skill.license === "未声明协议"
      ? locale === "en"
        ? "License Unspecified"
        : "ライセンス未指定"
      : skill.license;
  return {
    ...skill,
    name: displayName,
    description: hasCjkDesc ? localizedDesc : skill.description,
    overview: hasCjkOverview ? localizedDesc : skill.overview,
    author: localizedAuthor,
    license: localizedLicense,
    disciplineLabel: localizeSkillDisciplineLabel(skill.discipline, skill.disciplineLabel, locale),
    badge: (localizeSkillBadge(skill.badge, locale) ?? skill.badge) as AgentSkill["badge"],
    sourceLabel: localizeSkillSourceLabel(skill.sourceLabel, locale),
  };
}

export function localizeSkillSourceLabel(sourceLabel: string | undefined, locale: Locale): string | undefined {
  if (!sourceLabel || locale === "zh-CN") return sourceLabel;
  const map: Record<Exclude<Locale, "zh-CN">, Record<string, string>> = {
    en: {
      自定义: "Custom",
      链接添加: "URL",
      文件夹导入: "Folder",
      插件: "Plugin",
      "插件/自定义": "Plugin / Custom",
      内置工作流: "Built-in",
      "EOStage 扩展库": "EOStage Hub",
    },
    ja: {
      自定义: "カスタム",
      链接添加: "URL 追加",
      文件夹导入: "フォルダー",
      插件: "プラグイン",
      "插件/自定义": "プラグイン / カスタム",
      内置工作流: "組み込み",
      "EOStage 扩展库": "EOStage ハブ",
    },
  };
  return map[locale][sourceLabel] ?? sourceLabel;
}
