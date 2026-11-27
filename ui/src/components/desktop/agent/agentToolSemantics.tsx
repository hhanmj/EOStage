/**
 * agentToolSemantics.tsx
 * Agent 全量工具语义图标、EOStage 单色线条规范与简短人话说明注册表。
 *
 * 视觉纪律（统一交互布局 单色时间线规范）：
 * - 严禁给工具图标添加彩色染色或彩色底框；
 * - 白底统一灰黑线（text-[#52525b]），黑底统一灰白线（dark:text-[#a1a1aa]），靠图标造型区分工具语义。
 */

import React from "react";
import {
  AlertTriangle,
  Blocks,
  Cloud,
  Compass,
  Cpu,
  Database,
  Download,
  FileCode2,
  FileText,
  Gauge,
  Globe,
  KeyRound,
  Layers,
  ListChecks,
  ListTodo,
  MapPin,
  MessageSquareMore,
  Plug,
  Radar,
  Scissors,
  Search,
  ShieldCheck,
  SlidersHorizontal,
  SquareTerminal,
  Terminal,
  Waypoints,
  Wrench,
  type LucideIcon,
} from "lucide-react";

export type ToolSemanticCategory = "spatial" | "sensing" | "delivery" | "system";

export interface ToolSemanticMeta {
  category: ToolSemanticCategory;
  icon: LucideIcon;
  iconToneClass: string;
  nodeBgClass: string;
  defaultBriefZh: string;
}

export const EO_MONOCHROME_ICON_CLASS = "text-[#52525b] dark:text-[#a1a1aa]";
const MONOCHROME_TONE = {
  iconToneClass: EO_MONOCHROME_ICON_CLASS,
  nodeBgClass: "bg-transparent border-transparent",
} as const;

const TONE_BY_CATEGORY: Record<
  ToolSemanticCategory,
  { iconToneClass: string; nodeBgClass: string }
> = {
  spatial: MONOCHROME_TONE,
  sensing: MONOCHROME_TONE,
  delivery: MONOCHROME_TONE,
  system: MONOCHROME_TONE,
};

interface RawToolEntry {
  category: ToolSemanticCategory;
  icon: LucideIcon;
  defaultBriefZh: string;
}

const EXACT_TOOL_MAP: Record<string, RawToolEntry> = {
  // 1. 空间范围与本地/外部 GIS
  working_aoi_get: {
    category: "spatial",
    icon: Compass,
    defaultBriefZh: "读取当前绑定的研究区范围",
  },
  working_aoi_set: {
    category: "spatial",
    icon: MapPin,
    defaultBriefZh: "更新并锁定当前研究区边界",
  },
  resolve_aoi: {
    category: "spatial",
    icon: MapPin,
    defaultBriefZh: "解析研究区空间范围与坐标投影",
  },
  admin_boundary_search: {
    category: "spatial",
    icon: MapPin,
    defaultBriefZh: "查询标准行政区划空间边界",
  },
  global_disaster_search: {
    category: "spatial",
    icon: AlertTriangle,
    defaultBriefZh: "检索全球实时灾害事件与范围",
  },
  aoi_validate_boundary: {
    category: "spatial",
    icon: ShieldCheck,
    defaultBriefZh: "执行研究区边界双源安全校验",
  },
  map_workspace_action: {
    category: "spatial",
    icon: Compass,
    defaultBriefZh: "联动右侧地图视野与图层显示",
  },
  gis_toolchains_status: {
    category: "spatial",
    icon: Wrench,
    defaultBriefZh: "检测本机 GDAL/PROJ 与外部 GIS 环境",
  },
  gis_local_process: {
    category: "spatial",
    icon: Waypoints,
    defaultBriefZh: "执行保精度栅格重投影或矢量缓冲",
  },
  gis_external_execute: {
    category: "spatial",
    icon: SquareTerminal,
    defaultBriefZh: "调用外部 GIS 引擎生成交付工程",
  },

  // 2. 卫星目录检索、批次翻阅与 GEE/NISAR 云端遥感
  search_start: {
    category: "sensing",
    icon: Radar,
    defaultBriefZh: "按时空与轨道条件检索卫星目录",
  },
  application_sar_search_and_freeze: {
    category: "sensing",
    icon: Radar,
    defaultBriefZh: "检索雷达卫星目录并生成候选批次",
  },
  search_progress: {
    category: "sensing",
    icon: Radar,
    defaultBriefZh: "同步卫星目录检索命中结果",
  },
  workbench_page: {
    category: "sensing",
    icon: Layers,
    defaultBriefZh: "读取当前工作台候选影像列表",
  },
  batch_page: {
    category: "sensing",
    icon: Layers,
    defaultBriefZh: "翻阅检索批次景元数据与时空覆盖",
  },
  nisar_subset_preflight: {
    category: "sensing",
    icon: Scissors,
    defaultBriefZh: "预估 NISAR 研究区云端裁剪体积",
  },
  gee_catalog_search: {
    category: "sensing",
    icon: Cloud,
    defaultBriefZh: "检索 Earth Engine 云端遥感数据集",
  },
  gee_dataset_advisor: {
    category: "sensing",
    icon: Database,
    defaultBriefZh: "按科研角色配数、对比与核验资产",
  },
  gee_export_preflight: {
    category: "sensing",
    icon: ShieldCheck,
    defaultBriefZh: "预演导出分块规模、安全与血缘",
  },
  gee_check_availability: {
    category: "sensing",
    icon: Cloud,
    defaultBriefZh: "核验云端可用景数、云量与分块规划",
  },
  gee_cloud_analyze: {
    category: "sensing",
    icon: Cpu,
    defaultBriefZh: "云端计算遥感指数与合成预览图层",
  },
  gee_script_inspect: {
    category: "sensing",
    icon: FileCode2,
    defaultBriefZh: "预检自定义遥感脚本安全性与量纲",
  },
  gee_quota_status: {
    category: "sensing",
    icon: Gauge,
    defaultBriefZh: "查询云端算力配额与本月消耗",
  },

  // 3. 下载任务创建、局部拉取与队列控制
  create_task: {
    category: "delivery",
    icon: Download,
    defaultBriefZh: "创建卫星影像下载与配套交付任务",
  },
  nisar_create_subset_task: {
    category: "delivery",
    icon: Scissors,
    defaultBriefZh: "创建 NISAR 按研究区局部下载任务",
  },
  gee_chunked_download: {
    category: "delivery",
    icon: Download,
    defaultBriefZh: "按网格分块下载并拼接浮点 GeoTIFF",
  },
  update_download_task: {
    category: "delivery",
    icon: SlidersHorizontal,
    defaultBriefZh: "更新下载任务并发或配置参数",
  },
  task_action: {
    category: "delivery",
    icon: SlidersHorizontal,
    defaultBriefZh: "执行下载任务暂停、恢复或重试",
  },
  task_overview: {
    category: "delivery",
    icon: ListChecks,
    defaultBriefZh: "汇总下载队列进度与历史状态",
  },
  task_page: {
    category: "delivery",
    icon: ListChecks,
    defaultBriefZh: "读取下载任务列表与完成进度",
  },
  task_event_page: {
    category: "delivery",
    icon: FileText,
    defaultBriefZh: "读取任务传输事件与校验记录",
  },

  // 4. 系统就绪、账号凭据、数据源目录、技能包与规划交互
  service_readiness: {
    category: "system",
    icon: Cpu,
    defaultBriefZh: "检查本地存储与下载核心服务状态",
  },
  capabilities_get: {
    category: "system",
    icon: Cpu,
    defaultBriefZh: "读取各传感器检索与处理参数约束",
  },
  source_list: {
    category: "system",
    icon: Database,
    defaultBriefZh: "读取已接入的卫星与地形数据源清单",
  },
  credential_readiness: {
    category: "system",
    icon: KeyRound,
    defaultBriefZh: "检查各遥感数据中心凭据配置状态",
  },
  account_readiness: {
    category: "system",
    icon: ShieldCheck,
    defaultBriefZh: "核验数据服务账号登录与授权就绪态",
  },
  skill_read: {
    category: "system",
    icon: Blocks,
    defaultBriefZh: "按需载入遥感专业工作流技能规范",
  },
  toolset_activate: {
    category: "system",
    icon: Layers,
    defaultBriefZh: "按需解锁专域扩展工具集（GEE / NISAR / 本地 GIS / MCP）",
  },
  propose_plan: {
    category: "system",
    icon: ListTodo,
    defaultBriefZh: "制定结构化检索与任务执行计划",
  },
  ask_user_question: {
    category: "system",
    icon: MessageSquareMore,
    defaultBriefZh: "向你确认待定的检索或交付条件",
  },
};

export function resolveToolSemanticMeta(toolId: string | undefined | null): ToolSemanticMeta {
  const normalized = (toolId ?? "").trim();
  const exact = EXACT_TOOL_MAP[normalized];
  if (exact) {
    return {
      category: exact.category,
      icon: exact.icon,
      ...TONE_BY_CATEGORY[exact.category],
      defaultBriefZh: exact.defaultBriefZh,
    };
  }

  const lower = normalized.toLowerCase();
  if (lower.includes("web") || lower.includes("fetch") || lower.includes("http") || lower.includes("reader") || lower.includes("browser")) {
    return {
      category: "system",
      icon: Globe,
      ...TONE_BY_CATEGORY.system,
      defaultBriefZh: "读取外部网页或公开技术文档",
    };
  }
  if (lower.includes("search")) {
    return {
      category: "sensing",
      icon: Search,
      ...TONE_BY_CATEGORY.sensing,
      defaultBriefZh: "检索外部工具或关联数据记录",
    };
  }
  if (lower.startsWith("mcp_") || lower.startsWith("mcp__") || lower.includes("mcp")) {
    return {
      category: "system",
      icon: Plug,
      ...TONE_BY_CATEGORY.system,
      defaultBriefZh: "调用外部 MCP 扩展工具",
    };
  }

  return {
    category: "system",
    icon: Terminal,
    ...TONE_BY_CATEGORY.system,
    defaultBriefZh: "执行辅助诊断与上下文读取",
  };
}

export function renderToolSemanticGlyph(
  toolId: string | undefined | null,
  options?: {
    status?: string;
    resultCount?: number | null;
    className?: string;
  }
): React.ReactElement {
  const sizeCls = options?.className ?? "h-3.5 w-3.5 shrink-0";
  const meta = resolveToolSemanticMeta(toolId);
  const Icon = meta.icon;
  return (
    <Icon
      className={`${sizeCls} stroke-[1.6] ${meta.iconToneClass}`}
      aria-hidden="true"
    />
  );
}
