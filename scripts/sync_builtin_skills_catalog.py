#!/usr/bin/env python3
"""Sync built-in skill files under `ui/public/skills/` with `manifest.json` and `ui/src/lib/geoscienceSkillsCatalog.ts`."""

from __future__ import annotations

import argparse
import json
import re
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SKILLS_DIR = ROOT / "ui" / "public" / "skills"
MANIFEST_PATH = SKILLS_DIR / "manifest.json"
CATALOG_TS_PATH = ROOT / "ui" / "src" / "lib" / "geoscienceSkillsCatalog.ts"

CATALOG_HEADER = """/**
 * geoscienceSkillsCatalog.ts
 * 自动生成自 ui/public/skills/ 下的 {count} 个真实地球科学、GIS 与 InSAR 技能包
 */

export interface SkillFileEntry {{
  path: string;
  sizeBytes: number;
  kind: "skill_md" | "license" | "reference" | "script" | "agent" | "doc";
}}

export interface GeoscienceSkillPackage {{
  id: string;
  nameZh: string;
  nameEn: string;
  discipline: "insar-sar" | "gis-spatial" | "earth-env" | "skill-engineering";
  disciplineLabel: string;
  badge: "内置" | "官方" | "自定义";
  author: string;
  license: string;
  version: string;
  slashCommand: string;
  shortDescriptionZh: string;
  overviewZh: string;
  sharedComponents: string[];
  sharedComponentNote: string;
  workbenchTool: string | null;
  exampleTasks: string[];
  fileCount: number;
  totalBytes: number;
  files: SkillFileEntry[];
  skillMd: string;
  bundledFileContents: Record<string, string>;
}}

export const GEOSCIENCE_SKILL_PACKAGES: readonly GeoscienceSkillPackage[] = {json_payload};
"""

NEW_SKILL_METADATA: list[dict] = [
    {
        "id": "eo-analysis-recipes",
        "zip_name": "eostage-eo-recipes-plugin-1.0.0.zip",
        "nameZh": "遥感与生态水文分析配方知识库",
        "nameEn": "EO & GEE Analysis Recipes Cookbook",
        "discipline": "insar-sar",
        "disciplineLabel": "遥感与雷达测量",
        "badge": "内置",
        "author": "EOStage",
        "license": "Apache-2.0",
        "version": "1.1.0",
        "slashCommand": "/eo-analysis-recipes",
        "shortDescriptionZh": "涵盖 83 项 GEE Python 云端算子（含 Cloud Score+、Dynamic World、LandTrendr/CCDC）与 16 项本地 GeoTIFF 统计配方。",
        "overviewZh": "集成 83 项 Google Earth Engine 云端分析配方（Sentinel-1/2、Cloud Score+、Dynamic World、Landsat RSEI 生态指数、MODIS GPP/NPP、Sentinel-5P 大气 NO2、ERA5-Land 气候距平、TerraClimate、LandTrendr/CCDC 与服务端 Join）及 16 项本地 GeoTIFF 分块流式统计配方，共 99 项即用卡片。",
        "sharedComponents": ["native-gis-runtime"],
        "sharedComponentNote": "配合内置 GEE 云端计算工具或本机 GDAL/PROJ 运行时",
        "workbenchTool": "gee_cloud_analyze",
        "exampleTasks": [
            "检索并应用 MODIS MOD17A2H 全年累计 GPP 与晴空质控合成配方",
            "按研究区计算 Sentinel-2 HARMONIZED 去云中位数 NDVI 与 NDWI",
            "对本地多时相 GeoTIFF 执行谐波拟合填补缺测（HANTS）与逐像元 Sen 斜率趋势分析",
        ],
    },
    {
        "id": "eo-agent-capability-routing",
        "zip_name": "eostage-eo-methodology-plugin-1.0.0.zip",
        "nameZh": "遥感智能体意图与工具匹配",
        "nameEn": "EO Agent Capability Routing",
        "discipline": "skill-engineering",
        "disciplineLabel": "技能工程与编排",
        "badge": "内置",
        "author": "EOStage",
        "license": "Apache-2.0",
        "version": "1.0.0",
        "slashCommand": "/eo-agent-capability-routing",
        "shortDescriptionZh": "将自然语言遥感请求拆解为可验证步骤，按可用工具、产品级别与返回证据选择调用或诚实降级。",
        "overviewZh": "面向多步遥感任务的意图拆解与工具能力匹配规程：明确检索影像、计算指数、导出栅格与验证落盘四类边界，核查前置访问条件并在能力缺失时保留已完成候选与诚实降级说明。",
        "sharedComponents": [],
        "sharedComponentNote": "纯编排与前置核验规程，无额外二进制依赖",
        "workbenchTool": None,
        "exampleTasks": [
            "评估当前可用工具链能否直接完成某县月度无云 NDVI 栅格交付并列出缺失前提",
        ],
    },
    {
        "id": "eo-model-explanation-review",
        "zip_name": "eostage-eo-methodology-plugin-1.0.0.zip",
        "nameZh": "遥感模型归因与误差解释",
        "nameEn": "EO Model Explanation Review",
        "discipline": "earth-env",
        "disciplineLabel": "地球与环境科学",
        "badge": "内置",
        "author": "EOStage",
        "license": "Apache-2.0",
        "version": "1.0.0",
        "slashCommand": "/eo-model-explanation-review",
        "shortDescriptionZh": "审查遥感分类与回归模型的 SHAP 归因、输出尺度可加性、波段共线性与分区残差误差边界。",
        "overviewZh": "规范遥感机器学习模型的 SHAP 贡献可加性复核（区分 log-odds 与概率尺度）、红光/近红外/NDVI 共线性特征组解释及分区残差诊断，严禁将模型特征依赖直接外推为生态因果干预结论。",
        "sharedComponents": [],
        "sharedComponentNote": "纯模型归因审查与误差诊断规程",
        "workbenchTool": None,
        "exampleTasks": [
            "核查随机森林生物量反演模型的 SHAP 特征归因尺度可加性与高共线波段解释边界",
        ],
    },
    {
        "id": "eo-model-transfer-evaluation",
        "zip_name": "eostage-eo-methodology-plugin-1.0.0.zip",
        "nameZh": "遥感模型跨区域评估与选型",
        "nameEn": "EO Model Transfer Evaluation",
        "discipline": "earth-env",
        "disciplineLabel": "地球与环境科学",
        "badge": "内置",
        "author": "EOStage",
        "license": "Apache-2.0",
        "version": "1.0.0",
        "slashCommand": "/eo-model-transfer-evaluation",
        "shortDescriptionZh": "面向同区插值、跨区迁移与未来预测任务设计空间与时间独立留出验证，核查分组重叠与泄漏。",
        "overviewZh": "针对遥感分类与回归模型的部署对齐验证规程：按空间块、行政区或时间窗口独立留出，自动核查训练集与测试集分组重叠泄漏，要求输出混淆矩阵、宏 F1 或原单位 MAE/RMSE。",
        "sharedComponents": [],
        "sharedComponentNote": "纯空间独立验证与迁移评估规程",
        "workbenchTool": None,
        "exampleTasks": [
            "我要把 A 市训练的 Sentinel-2 土地覆盖模型迁移到 B 市，如何设计无空间泄漏的评估方案？",
        ],
    },
    {
        "id": "eo-workbench-concept",
        "zip_name": "eostage-eo-methodology-plugin-1.0.0.zip",
        "nameZh": "遥感工作台任务状态与交付设计",
        "nameEn": "EO Workbench Concept Design",
        "discipline": "skill-engineering",
        "disciplineLabel": "技能工程与编排",
        "badge": "内置",
        "author": "EOStage",
        "license": "Apache-2.0",
        "version": "1.0.0",
        "slashCommand": "/eo-workbench-concept",
        "shortDescriptionZh": "将遥感功能需求梳理为用户任务、工作台状态契约、预览与物理落盘分离判据及可复现验收。",
        "overviewZh": "面向遥感工作台交互与状态机设计规程：严格区分数据可用性、质量合格、在线预览成功与本地文件落盘成功四类状态，确保参数变更后旧预览不冒充新结果。",
        "sharedComponents": [],
        "sharedComponentNote": "纯工作台状态契约与验收设计规程",
        "workbenchTool": None,
        "exampleTasks": [
            "把按城市制作月度植被指数图的需求拆解为工作台状态流转、异常兜底与验收条件",
        ],
    },
]

BANNED_BRAND_RE = re.compile(r"\b(?:ScenePort|InSAR\s+Studio)\b", re.IGNORECASE)
URL_QUERY_OR_FRAG_RE = re.compile(r"https?://[^\s)\]>\"']+[?#][^\s)\]>\"']*")

BUNDLE_ALL_TEXT_IDS = {
    "eo-analysis-recipes",
    "eo-agent-capability-routing",
    "eo-model-explanation-review",
    "eo-model-transfer-evaluation",
    "eo-workbench-concept",
    "gee-textbook-cookbook",
    "radar-satellite-analysis",
    "dem-datum-conversion",
    "s1-precise-orbit",
}

PARTIAL_METADATA_OVERRIDES: dict[str, dict] = {
    "gee-textbook-cookbook": {
        "version": "1.2.0",
        "shortDescriptionZh": "系统讲解 GEE 云端架构、Cloud Score+ 与 SCL 选型、谐波与趋势对比、Olofsson 无偏面积估计及 EECU 配额、跨传感器协调、QA 缩放与 DAG 排错 15 篇规程。",
        "overviewZh": "面向 Google Earth Engine Python 开发与遥感科学方法学的统一手册（5 章方法学教材 + 10 张工程实战守则卡）：覆盖光学去云与基线对齐、谐波回归与趋势/断点选型、空间独立交叉验证与 Olofsson (2014) 无偏面积、水文地形与预演分块导出，以及 EECU 配额控制、多传感器 QA/定标矩阵、S1 线性功率边界、数据血缘台账与服务端 DAG 性能调优；与 99 项即用配方库 eo-analysis-recipes 互补。",
    },
    "skill-creator": {
        "author": "Anthropic (anthropics/skills)",
        "license": "Apache-2.0",
    },
    "radar-satellite-analysis": {
        "nameZh": "SAR 与雷达卫星分析",
        "nameEn": "SAR & Radar Satellite Analysis",
        "author": "EOStage",
        "license": "GPL-3.0-or-later",
        "version": "2.3.0",
        "shortDescriptionZh": "面向 Sentinel-1（IW SLC 基线配对 / GRD / SLC-BURST）、ALOS-2 L 波段条带干涉、NISAR L/S 波段、精密轨道与 DEM 椭球高对齐及后向散射分析。",
        "overviewZh": "覆盖 EOStage 核心 SAR 与 InSAR 数据准备与干涉配对规程：包括 Sentinel-1 IW SLC 时空基线配对分支（Bperp < 150 m 与 6/12/24 天重访）、ALOS-2 PALSAR-2 L 波段植被穿透条带（SM1/SM3）滑坡与同震配对分支、POEORB/RESORB 精密轨道匹配、NISAR L/S 波段 AOI HDF5 子集裁剪、WGS84 椭球高 DEM 转换、GACOS 大气延迟批处理，以及 GRD dB 转线性功率与斑点滤波规程。",
        "exampleTasks": [
            "结合当前研究区检索 Sentinel-1 升降轨 IW SLC 影像，按 Bperp < 150 m 规划干涉对并匹配 AUX_POEORB 精密轨道",
            "针对高植被覆盖山区检索 ALOS-2 L 波段 SM1/SM3（HH 极化）干涉像对并规划配套 WGS84 椭球高 DEM",
            "如何针对数十 GB 的 NISAR L 波段 RSLC 数据仅按研究区 AOI 做远端 HDF5 子集裁剪？",
            "对 Sentinel-1 GRD VV/VH 分贝栅格做时空平均前，为什么必须先转换为线性功率？",
        ],
    },
    "dem-datum-conversion": {
        "version": "1.1.0",
    },
    "s1-precise-orbit": {
        "version": "1.1.0",
    },
}


def classify_kind(rel_path: str) -> str:
    if rel_path == "SKILL.md":
        return "skill_md"
    if rel_path.startswith("references/"):
        return "reference"
    if rel_path.startswith("scripts/"):
        return "script"
    if rel_path.startswith("agents/"):
        return "agent"
    if rel_path.upper().startswith("LICENSE"):
        return "license"
    return "doc"


def load_existing_ts_catalog() -> dict[str, dict]:
    ts_txt = CATALOG_TS_PATH.read_text(encoding="utf-8")
    match = re.search(
        r"GEOSCIENCE_SKILL_PACKAGES:\s*readonly\s+GeoscienceSkillPackage\[\]\s*=\s*(\[\s*\{[\s\S]*\n\]);",
        ts_txt,
    )
    assert match is not None, "Failed to parse GEOSCIENCE_SKILL_PACKAGES from geoscienceSkillsCatalog.ts"
    return {item["id"]: item for item in json.loads(match.group(1))}


def assert_clean_skill_text(label: str, text: str, check_url_query: bool = True) -> None:
    if m := BANNED_BRAND_RE.search(text):
        raise ValueError(f"{label} contains banned brand text: {m.group(0)!r}")
    if check_url_query and (m := URL_QUERY_OR_FRAG_RE.search(text)):
        raise ValueError(f"{label} contains URL query/fragment that triggers outbound sanitizer: {m.group(0)!r}")


def collect_disk_files(skill_dir: Path) -> tuple[list[dict], int]:
    files_meta: list[dict] = []
    total_bytes = 0
    for p in sorted(skill_dir.rglob("*")):
        if not p.is_file() or p.name.startswith("."):
            continue
        rel = p.relative_to(skill_dir).as_posix()
        size_bytes = p.stat().st_size
        total_bytes += size_bytes
        files_meta.append({
            "path": rel,
            "sizeBytes": size_bytes,
            "kind": classify_kind(rel),
        })
    return files_meta, total_bytes


def main() -> None:
    parser = argparse.ArgumentParser(description="Sync scrubbed built-in skills and regenerate catalog.")
    parser.add_argument(
        "--hub-dir",
        type=Path,
        default=Path.home() / "eostage-hub",
        help="Path to local eostage-hub checkout containing plugins/*.zip",
    )
    parser.add_argument(
        "--extract-from-hub",
        action="store_true",
        default=False,
        help="Extract new skill files from hub plugin archives (default: treat ui/public/skills on disk as authoritative).",
    )
    args = parser.parse_args()
    hub_plugins_dir = args.hub_dir.expanduser().resolve() / "plugins"

    manifest = json.loads(MANIFEST_PATH.read_text(encoding="utf-8"))
    ts_by_id = load_existing_ts_catalog()

    # 1. Refresh metadata & file counts from on-disk ui/public/skills (authoritative)
    existing_ids = {s["id"] for s in manifest["skills"]}
    new_ids = {m["id"] for m in NEW_SKILL_METADATA}
    for entry in manifest["skills"]:
        sid = entry["id"]
        skill_dir = SKILLS_DIR / sid
        assert skill_dir.is_dir(), f"Missing skill directory on disk: {skill_dir}"
        if sid not in new_ids:
            ts_pkg = ts_by_id.get(sid)
            if ts_pkg is not None:
                for key in ("author", "overviewZh", "shortDescriptionZh", "nameZh", "nameEn", "badge", "license", "version"):
                    entry[key] = ts_pkg[key]
        if sid in PARTIAL_METADATA_OVERRIDES:
            entry.update(PARTIAL_METADATA_OVERRIDES[sid])
        files_meta, total_bytes = collect_disk_files(skill_dir)
        entry["files"] = files_meta
        entry["fileCount"] = len(files_meta)
        entry["totalBytes"] = total_bytes

    # 2. Ensure the 5 bundled knowledge/methodology skills match metadata & disk (optional zip extract)
    for meta in NEW_SKILL_METADATA:
        sid = meta["id"]
        skill_dir = SKILLS_DIR / sid
        if args.extract_from_hub:
            zip_path = hub_plugins_dir / meta["zip_name"]
            assert zip_path.is_file(), f"Missing plugin archive: {zip_path}"
            prefix = f"skills/{sid}/"
            skill_dir.mkdir(parents=True, exist_ok=True)
            extracted_count = 0
            with zipfile.ZipFile(zip_path, "r") as zf:
                for member in sorted(zf.namelist()):
                    if not member.startswith(prefix) or member.endswith("/"):
                        continue
                    rel_path = member[len(prefix):]
                    content = zf.read(member).decode("utf-8")
                    assert_clean_skill_text(f"{sid}/{rel_path}", content)
                    out_file = skill_dir / rel_path
                    out_file.parent.mkdir(parents=True, exist_ok=True)
                    out_file.write_text(content, encoding="utf-8")
                    extracted_count += 1
            assert extracted_count > 0, f"No files extracted for {sid} from {zip_path}"
        else:
            assert skill_dir.is_dir(), f"Missing skill directory on disk: {skill_dir}"

        files_meta, total_bytes = collect_disk_files(skill_dir)
        manifest_entry = {
            "id": sid,
            "nameZh": meta["nameZh"],
            "nameEn": meta["nameEn"],
            "discipline": meta["discipline"],
            "disciplineLabel": meta["disciplineLabel"],
            "badge": meta["badge"],
            "author": meta["author"],
            "license": meta["license"],
            "version": meta["version"],
            "slashCommand": meta["slashCommand"],
            "shortDescriptionZh": meta["shortDescriptionZh"],
            "overviewZh": meta["overviewZh"],
            "sharedComponents": meta["sharedComponents"],
            "sharedComponentNote": meta["sharedComponentNote"],
            "workbenchTool": meta["workbenchTool"],
            "exampleTasks": meta["exampleTasks"],
            "fileCount": len(files_meta),
            "totalBytes": total_bytes,
            "files": files_meta,
        }
        assert_clean_skill_text(f"manifest:{sid}:short", manifest_entry["shortDescriptionZh"])
        assert_clean_skill_text(f"manifest:{sid}:overview", manifest_entry["overviewZh"])

        if sid in existing_ids:
            manifest["skills"] = [manifest_entry if s["id"] == sid else s for s in manifest["skills"]]
        else:
            manifest["skills"].append(manifest_entry)

    # 3. Refresh bundledFileContents from disk & write manifest.json and geoscienceSkillsCatalog.ts
    manifest["generatedAt"] = "2026-11-20T18:20:00Z"
    manifest["totalSkills"] = len(manifest["skills"])

    ts_packages: list[dict] = []
    for entry in manifest["skills"]:
        sid = entry["id"]
        skill_dir = SKILLS_DIR / sid
        ts_prev = ts_by_id.get(sid)
        skill_md = (skill_dir / "SKILL.md").read_text(encoding="utf-8")
        assert_clean_skill_text(f"{sid}/SKILL.md", skill_md)

        if sid in BUNDLE_ALL_TEXT_IDS or ts_prev is None:
            bundled = {}
            for fmeta in entry["files"]:
                rel = fmeta["path"]
                if rel.endswith((".md", ".txt", ".yaml", ".py")):
                    content = (skill_dir / rel).read_text(encoding="utf-8")
                    assert_clean_skill_text(f"{sid}/{rel}", content)
                    bundled[rel] = content
        else:
            bundled = {}
            preserved_paths = list(ts_prev.get("bundledFileContents", {}).keys())
            for fmeta in entry["files"]:
                rel = fmeta["path"]
                if fmeta.get("kind") == "license" and rel not in preserved_paths:
                    preserved_paths.append(rel)
            for rel in preserved_paths:
                disk_file = skill_dir / rel
                if disk_file.is_file():
                    content = disk_file.read_text(encoding="utf-8")
                    assert_clean_skill_text(f"{sid}/{rel}", content, check_url_query=False)
                    bundled[rel] = content
            bundled["SKILL.md"] = skill_md

        pkg_obj = dict(entry)
        pkg_obj["skillMd"] = skill_md
        pkg_obj["bundledFileContents"] = bundled
        ts_packages.append(pkg_obj)

    MANIFEST_PATH.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    catalog_ts = CATALOG_HEADER.format(
        count=len(ts_packages),
        json_payload=json.dumps(ts_packages, ensure_ascii=False, indent=2),
    )
    CATALOG_TS_PATH.write_text(catalog_ts, encoding="utf-8")
    print(f"Synced {len(ts_packages)} built-in skill packages to manifest.json and geoscienceSkillsCatalog.ts.")


if __name__ == "__main__":
    main()
