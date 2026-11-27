import React, {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  Search,
  Terminal,
  Map as MapIcon,
  Compass,
  SlidersHorizontal,
  Download,
  Blocks,
  Cpu,
  Puzzle,
  Database,
  MessageSquare,
  FolderOpen,
  FileText,
  CornerDownLeft,
  X,
  Plus,
} from "lucide-react";
import { ModalOverlay } from "@/components/ui/ModalOverlay";
import { cn } from "@/lib/utils";
import { EmptyState } from "@/components/ui/EmptyState";
import {
  SPRING_PRESETS,
  canAnimateSpringInBrowser,
  clamp,
  createSpring,
  createStretchyHighlight,
  hideStretchyHighlight,
  measureRelativeLayoutRect,
  readStretchyHighlightMetrics,
  settleIfClose,
  snapSpring,
  stepSpringSubdivided,
  stepStretchyHighlight,
  targetStretchyHighlight,
  tuneSpring,
} from "@/lib/springDropdownMotion";
import type { ShellRoute } from "@/shell/api/shellRoutes";
import type { SettingsCategory } from "@/components/SettingsModal";
import {
  SETTINGS_CATEGORIES,
  getSettingsCategoryLabel,
} from "@/shell/api/settingsEntries";
import {
  buildCatalogItems,
  getRecentSourceIds,
  type CatalogItem,
} from "@/lib/dataSourceCatalogLogic";
import {
  getAllSkills,
  loadDisabledSkillIds,
  type AgentSkill,
} from "@/lib/agentSkills";
import { historyTitle } from "@/shell/api/agentHistoryLogic";
import type { AgentSessionSummary } from "@/lib/generated/v3-contracts";
import type { ApplicationTask } from "@/shell/api/types";

export type CommandPaletteTabKey =
  | "all"
  | "workbench"
  | "catalog"
  | "sessions_tasks"
  | "artifacts";

export type ScientificWorkbenchMode = "sar" | "dem" | "orbit" | "convert" | "gee";

export interface LocalArtifactEntry {
  id: string;
  title: string;
  path: string;
  source?: string;
}

export interface GlobalCommandPaletteProps {
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  /** 是否监听 Cmd/Ctrl+K 全局快捷键（默认 true） */
  enableGlobalShortcut?: boolean;
  onNavigate?: (route: ShellRoute) => void;
  onOpenScientificMode?: (mode: ScientificWorkbenchMode, presetTool?: string) => void;
  onOpenSettingsCategory?: (category: SettingsCategory) => void;
  onOpenSession?: (sessionId: string) => void;
  onNewChat?: () => void;
  onSelectDataSource?: (resourceId: string) => void;
  onSelectSkill?: (skill: AgentSkill) => void;
  onOpenTask?: (taskId: string) => void;
  onOpenLocalPath?: (path: string) => void;
  sessions?: AgentSessionSummary[];
  tasks?: ApplicationTask[];
  skills?: AgentSkill[];
  dataSources?: CatalogItem[];
  localPaths?: LocalArtifactEntry[];
  disabledSourceIds?: Set<string>;
}

interface PaletteCommandItem {
  id: string;
  group: Exclude<CommandPaletteTabKey, "all">;
  groupLabel: string;
  title: string;
  subtitle: string;
  badge?: string;
  keywords: string;
  icon: "workbench" | "dem" | "gee" | "download" | "plugins" | "settings" | "source" | "skill" | "session" | "task" | "folder" | "chat";
  disabled?: boolean;
  onSelect: () => void;
}

const PALETTE_TABS: ReadonlyArray<{
  key: CommandPaletteTabKey;
  label: string;
}> = [
  { key: "all", label: "全部" },
  { key: "workbench", label: "功能与工作台" },
  { key: "catalog", label: "数据源与技能" },
  { key: "sessions_tasks", label: "对话与下载任务" },
  { key: "artifacts", label: "本地成果文件" },
];

const TASK_STATUS_LABEL_MAP: Record<string, string> = {
  queued: "排队中",
  running: "进行中",
  paused: "已暂停",
  completed: "已完成",
  failed: "失败",
  cancelled: "已取消",
};

function renderCommandIcon(kind: PaletteCommandItem["icon"]) {
  const cls = "h-4 w-4 shrink-0 text-sp-text-secondary";
  switch (kind) {
    case "workbench":
      return <MapIcon className={cls} strokeWidth={1.75} />;
    case "dem":
      return <Compass className={cls} strokeWidth={1.75} />;
    case "gee":
      return <Terminal className={cls} strokeWidth={1.75} />;
    case "download":
      return <Download className={cls} strokeWidth={1.75} />;
    case "plugins":
      return <Blocks className={cls} strokeWidth={1.75} />;
    case "settings":
      return <SlidersHorizontal className={cls} strokeWidth={1.75} />;
    case "source":
      return <Database className={cls} strokeWidth={1.75} />;
    case "skill":
      return <Puzzle className={cls} strokeWidth={1.75} />;
    case "session":
      return <MessageSquare className={cls} strokeWidth={1.75} />;
    case "task":
      return <Cpu className={cls} strokeWidth={1.75} />;
    case "folder":
      return <FolderOpen className={cls} strokeWidth={1.75} />;
    case "chat":
      return <Plus className={cls} strokeWidth={1.75} />;
    default:
      return <FileText className={cls} strokeWidth={1.75} />;
  }
}

export function GlobalCommandPalette({
  open: controlledOpen,
  onOpenChange,
  enableGlobalShortcut = true,
  onNavigate,
  onOpenScientificMode,
  onOpenSettingsCategory,
  onOpenSession,
  onNewChat,
  onSelectDataSource,
  onSelectSkill,
  onOpenTask,
  onOpenLocalPath,
  sessions = [],
  tasks = [],
  skills: providedSkills,
  dataSources: providedDataSources,
  localPaths: providedLocalPaths = [],
  disabledSourceIds,
}: GlobalCommandPaletteProps) {
  const [internalOpen, setInternalOpen] = useState(false);
  const isOpen = controlledOpen !== undefined ? controlledOpen : internalOpen;

  const setOpen = useCallback(
    (next: boolean) => {
      if (controlledOpen === undefined) {
        setInternalOpen(next);
      }
      onOpenChange?.(next);
    },
    [controlledOpen, onOpenChange],
  );

  const [query, setQuery] = useState("");
  const [activeTab, setActiveTab] = useState<CommandPaletteTabKey>("all");
  const [activeIndex, setActiveIndex] = useState(0);

  const inputRef = useRef<HTMLInputElement | null>(null);
  const dialogRef = useRef<HTMLDivElement | null>(null);
  const tabListRef = useRef<HTMLDivElement | null>(null);
  const tabHighlightRef = useRef<HTMLDivElement | null>(null);
  const tabRefs = useRef<Partial<Record<CommandPaletteTabKey, HTMLButtonElement | null>>>({});
  const resultsRef = useRef<HTMLDivElement | null>(null);
  const rowHighlightRef = useRef<HTMLDivElement | null>(null);
  const rowRefs = useRef<Map<number, HTMLButtonElement>>(new Map());
  const prevRowTopsRef = useRef<Map<string, number>>(new Map());

  const [springTabsReady, setSpringTabsReady] = useState(false);
  const [springRowsReady, setSpringRowsReady] = useState(false);

  const openSpringRafRef = useRef<number | null>(null);
  const tabHighlightRafRef = useRef<number | null>(null);
  const rowHighlightRafRef = useRef<number | null>(null);
  const flipRafRef = useRef<number | null>(null);
  const tabHighlightStateRef = useRef(createStretchyHighlight());
  const rowHighlightStateRef = useRef(createStretchyHighlight());

  useEffect(
    () => () => {
      if (typeof window === "undefined") return;
      if (openSpringRafRef.current !== null) window.cancelAnimationFrame(openSpringRafRef.current);
      if (tabHighlightRafRef.current !== null) window.cancelAnimationFrame(tabHighlightRafRef.current);
      if (rowHighlightRafRef.current !== null) window.cancelAnimationFrame(rowHighlightRafRef.current);
      if (flipRafRef.current !== null) window.cancelAnimationFrame(flipRafRef.current);
    },
    [],
  );

  useEffect(() => {
    if (!enableGlobalShortcut) return;
    const handleGlobalKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.isComposing) return;
      if ((e.metaKey || e.ctrlKey) && !e.altKey && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setOpen(!isOpen);
      }
    };
    window.addEventListener("keydown", handleGlobalKey);
    return () => window.removeEventListener("keydown", handleGlobalKey);
  }, [enableGlobalShortcut, isOpen, setOpen]);

  useEffect(() => {
    if (!isOpen) return;
    setQuery("");
    setActiveTab("all");
    setActiveIndex(0);
    const id = window.setTimeout(() => {
      inputRef.current?.focus();
    }, 0);
    return () => window.clearTimeout(id);
  }, [isOpen]);

  const recentSourceIds = useMemo(() => new Set(getRecentSourceIds()), [isOpen]);

  const catalogSources = useMemo<CatalogItem[]>(() => {
    const base = providedDataSources ?? buildCatalogItems();
    if (!disabledSourceIds || disabledSourceIds.size === 0) return base;
    return base.map((item) =>
      disabledSourceIds.has(item.resourceId) ? { ...item, disabled: true } : item
    );
  }, [providedDataSources, disabledSourceIds]);

  const allSkills = useMemo<AgentSkill[]>(() => {
    return providedSkills ?? getAllSkills();
  }, [providedSkills]);

  const disabledSkills = useMemo(() => loadDisabledSkillIds(), [isOpen]);

  const derivedArtifacts = useMemo<LocalArtifactEntry[]>(() => {
    const byPath = new Map<string, LocalArtifactEntry>();
    for (const item of providedLocalPaths) {
      const normalized = item.path.trim();
      if (!normalized || byPath.has(normalized)) continue;
      byPath.set(normalized, { ...item, path: normalized });
    }
    for (const task of tasks) {
      if (task.output_dir && task.output_dir.trim()) {
        const dir = task.output_dir.trim();
        if (!byPath.has(dir)) {
          byPath.set(dir, {
            id: `dir-${task.task_id}`,
            title: task.name || task.dataset || "下载任务输出目录",
            path: dir,
            source: task.provider || task.category,
          });
        }
      }
      for (const item of task.scene_items || []) {
        if (item.destination && item.destination.trim()) {
          const dest = item.destination.trim();
          if (!byPath.has(dest)) {
            byPath.set(dest, {
              id: `file-${task.task_id}-${item.scene_id}`,
              title: item.scene_id || task.name,
              path: dest,
              source: task.provider || "本地成果",
            });
          }
        }
      }
    }
    return Array.from(byPath.values());
  }, [providedLocalPaths, tasks]);

  const closeAndRun = useCallback(
    (action: () => void) => {
      setOpen(false);
      action();
    },
    [setOpen],
  );

  const allCommands = useMemo<PaletteCommandItem[]>(() => {
    const items: PaletteCommandItem[] = [];

    // 1. 功能与工作台
    items.push(
      {
        id: "cmd-new-chat",
        group: "workbench",
        groupLabel: "功能与工作台",
        title: "新建智能会话",
        subtitle: "开启新的 EOStage Agent 遥感科研与下载对话",
        badge: "工作台",
        keywords: "new chat agent 对话 新建会话 智能体",
        icon: "chat",
        onSelect: () => {
          if (onNewChat) onNewChat();
          else onNavigate?.("agent");
        },
      },
      {
        id: "cmd-workbench-search",
        group: "workbench",
        groupLabel: "功能与工作台",
        title: "数据检索与地图工作台",
        subtitle: "按研究区 AOI、时相与传感器检索 SAR / 光学 / DEM 数据",
        badge: "核心",
        keywords: "search map aoi 数据检索 地图工作台 sentinel landsat",
        icon: "workbench",
        onSelect: () => {
          if (onOpenScientificMode) onOpenScientificMode("sar");
          onNavigate?.("workbench");
        },
      },
      {
        id: "cmd-dem-convert",
        group: "workbench",
        groupLabel: "功能与工作台",
        title: "DEM 转换",
        subtitle: "高程基准正高转 WGS84 椭球高及 SARscape (.dem / .hdr) 格式转换",
        badge: "DEM 工具",
        keywords: "dem convert sarscape hdr envi ellipsoid egm96 egm2008 椭球高 正高 格式转换 基准转换 高程",
        icon: "dem",
        onSelect: () => {
          if (onOpenScientificMode) onOpenScientificMode("convert");
          else onNavigate?.("workbench");
        },
      },
      {
        id: "cmd-gee-cloud",
        group: "workbench",
        groupLabel: "功能与工作台",
        title: "GEE 云端计算与制图",
        subtitle: "Google Earth Engine 科学目录检索、AOI 在线裁剪与光谱指数计算",
        badge: "云端",
        keywords: "gee google earth engine 云端计算 制图 ndvi 指数",
        icon: "gee",
        onSelect: () => {
          if (onOpenScientificMode) onOpenScientificMode("gee");
          else onNavigate?.("workbench");
        },
      },
      {
        id: "cmd-orbit-aux",
        group: "workbench",
        groupLabel: "功能与工作台",
        title: "精密轨道与大气辅助产品",
        subtitle: "Sentinel-1 POEORB/RESORB 精轨与对流层延迟辅助检索",
        badge: "辅助",
        keywords: "orbit poeorb resorb 精密轨道 gacos 辅助",
        icon: "dem",
        onSelect: () => {
          if (onOpenScientificMode) onOpenScientificMode("orbit");
          else onNavigate?.("workbench");
        },
      },
      {
        id: "cmd-downloads",
        group: "workbench",
        groupLabel: "功能与工作台",
        title: "下载与任务中心",
        subtitle: "查看实时分景下载队列、速率曲线与历史任务记录",
        badge: "任务",
        keywords: "downloads queue task 下载中心 任务队列 传输",
        icon: "download",
        onSelect: () => onNavigate?.("downloads"),
      },
      {
        id: "cmd-plugins",
        group: "workbench",
        groupLabel: "功能与工作台",
        title: "插件与技能",
        subtitle: "管理分析技能提示词、扩展插件包与外部 MCP 工具连接",
        badge: "扩展",
        keywords: "plugins skills mcp 分析技能 插件包 连接器",
        icon: "plugins",
        onSelect: () => onNavigate?.("plugins"),
      },
    );

    // 设置全部分区
    for (const cat of SETTINGS_CATEGORIES) {
      const label = getSettingsCategoryLabel(cat);
      items.push({
        id: `cmd-settings-${cat.key}`,
        group: "workbench",
        groupLabel: "功能与工作台",
        title: `设置 · ${label}`,
        subtitle: `打开系统设置中的「${label}」分页`,
        badge: "设置",
        keywords: `settings 设置 ${cat.key} ${label}`,
        icon: "settings",
        onSelect: () => {
          if (onOpenSettingsCategory) {
            onOpenSettingsCategory(cat.key);
          } else {
            onNavigate?.("settings");
          }
        },
      });
    }

    // 2. 数据源与技能
    for (const src of catalogSources) {
      const isRecent = recentSourceIds.has(src.resourceId);
      items.push({
        id: `cmd-source-${src.resourceId}`,
        group: "catalog",
        groupLabel: "数据源与技能",
        title: src.name,
        subtitle: `${src.subtitle}${src.provider ? ` · ${src.provider}` : ""}`,
        badge: isRecent ? "最近使用" : src.band ? `${src.band} 波段` : src.provider || "数据源",
        keywords: `${src.resourceId} ${src.name} ${src.subtitle} ${src.provider || ""} ${src.satellites || ""} ${src.description}`,
        icon: "source",
        disabled: src.disabled,
        onSelect: () => {
          if (onSelectDataSource) {
            onSelectDataSource(src.resourceId);
          } else {
            onNavigate?.("workbench");
          }
        },
      });
    }

    for (const skill of allSkills) {
      const isOff = disabledSkills.has(skill.id);
      items.push({
        id: `cmd-skill-${skill.id}`,
        group: "catalog",
        groupLabel: "数据源与技能",
        title: skill.name,
        subtitle: skill.description,
        badge: isOff ? "技能（已停用）" : "分析技能",
        keywords: `${skill.id} ${skill.name} ${skill.nameEn || ""} ${skill.description} ${skill.author || ""} ${(skill.tags || []).join(" ")}`,
        icon: "skill",
        onSelect: () => {
          if (onSelectSkill) {
            onSelectSkill(skill);
          } else {
            onNavigate?.("plugins");
          }
        },
      });
    }

    // 3. 对话与下载任务
    for (const session of sessions) {
      const title = historyTitle(session.title);
      const runBadge =
        session.last_run_status === "running"
          ? "进行中"
          : session.last_run_status === "awaiting_approval"
            ? "等待确认"
            : session.last_run_status === "failed"
              ? "执行失败"
              : session.last_run_status === "cancelled"
                ? "已取消"
                : session.run_ids.length > 0
                  ? `${session.run_ids.length} 轮执行`
                  : "空闲会话";
      items.push({
        id: `cmd-session-${session.session_id}`,
        group: "sessions_tasks",
        groupLabel: "对话与下载任务",
        title,
        subtitle: `会话 · ${session.session_id}`,
        badge: runBadge,
        keywords: `${title} ${session.session_id} session 对话`,
        icon: "session",
        onSelect: () => {
          if (onOpenSession) onOpenSession(session.session_id);
          else onNavigate?.("agent");
        },
      });
    }

    for (const task of tasks) {
      const statusText = TASK_STATUS_LABEL_MAP[task.status] ?? task.status;
      items.push({
        id: `cmd-task-${task.task_id}`,
        group: "sessions_tasks",
        groupLabel: "对话与下载任务",
        title: task.name || task.dataset || task.task_id,
        subtitle: `${task.provider || task.category} · ${task.output_dir || task.task_id}`,
        badge: statusText,
        keywords: `${task.task_id} ${task.name} ${task.provider || ""} ${task.dataset || ""} ${task.output_dir || ""} ${statusText}`,
        icon: "task",
        onSelect: () => {
          if (onOpenTask) onOpenTask(task.task_id);
          else onNavigate?.("downloads");
        },
      });
    }

    // 4. 本地成果文件
    for (const artifact of derivedArtifacts) {
      items.push({
        id: `cmd-artifact-${artifact.id}`,
        group: "artifacts",
        groupLabel: "本地成果文件",
        title: artifact.title,
        subtitle: artifact.path,
        badge: artifact.source || "本地路径",
        keywords: `${artifact.title} ${artifact.path} ${artifact.source || ""} 本地 文件 目录`,
        icon: "folder",
        onSelect: () => {
          if (onOpenLocalPath) onOpenLocalPath(artifact.path);
          else onNavigate?.("downloads");
        },
      });
    }

    return items;
  }, [
    allSkills,
    catalogSources,
    derivedArtifacts,
    disabledSkills,
    onNavigate,
    onNewChat,
    onOpenLocalPath,
    onOpenScientificMode,
    onOpenSession,
    onOpenSettingsCategory,
    onOpenTask,
    onSelectDataSource,
    onSelectSkill,
    sessions,
    tasks,
  ]);

  const tabCounts = useMemo(() => {
    const q = query.trim().toLowerCase();
    const matchesQuery = (item: PaletteCommandItem) =>
      !q ||
      item.title.toLowerCase().includes(q) ||
      item.subtitle.toLowerCase().includes(q) ||
      item.keywords.toLowerCase().includes(q);
    const counts: Record<CommandPaletteTabKey, number> = {
      all: 0,
      workbench: 0,
      catalog: 0,
      sessions_tasks: 0,
      artifacts: 0,
    };
    for (const item of allCommands) {
      if (!matchesQuery(item)) continue;
      counts.all += 1;
      counts[item.group] += 1;
    }
    return counts;
  }, [allCommands, query]);

  const filteredCommands = useMemo(() => {
    const q = query.trim().toLowerCase();
    return allCommands.filter((item) => {
      if (activeTab !== "all" && item.group !== activeTab) return false;
      if (!q) return true;
      return (
        item.title.toLowerCase().includes(q) ||
        item.subtitle.toLowerCase().includes(q) ||
        item.keywords.toLowerCase().includes(q)
      );
    });
  }, [activeTab, allCommands, query]);

  useEffect(() => {
    setActiveIndex(0);
  }, [query, activeTab]);

  useEffect(() => {
    if (filteredCommands.length === 0) {
      setActiveIndex(0);
      return;
    }
    if (activeIndex >= filteredCommands.length) {
      setActiveIndex(filteredCommands.length - 1);
    }
  }, [activeIndex, filteredCommands.length]);

  useEffect(() => {
    const node = rowRefs.current.get(activeIndex);
    if (node && typeof node.scrollIntoView === "function") {
      node.scrollIntoView({ block: "nearest" });
    }
  }, [activeIndex]);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      setOpen(false);
      return;
    }
    if (e.key === "ArrowDown") {
      e.preventDefault();
      if (filteredCommands.length === 0) return;
      setActiveIndex((prev) => (prev + 1) % filteredCommands.length);
      return;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      if (filteredCommands.length === 0) return;
      setActiveIndex(
        (prev) => (prev - 1 + filteredCommands.length) % filteredCommands.length,
      );
      return;
    }
    if (e.key === "Enter") {
      const target = filteredCommands[activeIndex];
      if (!target || target.disabled) return;
      e.preventDefault();
      closeAndRun(target.onSelect);
    }
  };

  // 1. 面板打开时的 4-substep 弹簧弹出 + 前 8 行级联错峰渐显（对齐 #db-command-palette）
  useLayoutEffect(() => {
    if (!isOpen) {
      prevRowTopsRef.current.clear();
      hideStretchyHighlight(tabHighlightStateRef.current, true);
      hideStretchyHighlight(rowHighlightStateRef.current, true);
      setSpringTabsReady(false);
      setSpringRowsReady(false);
      return;
    }

    const dialog = dialogRef.current;
    if (!canAnimateSpringInBrowser(dialog)) return;

    if (openSpringRafRef.current !== null) {
      window.cancelAnimationFrame(openSpringRafRef.current);
      openSpringRafRef.current = null;
    }

    const popSp = createSpring(0, SPRING_PRESETS.selectOpen.k, SPRING_PRESETS.selectOpen.c);
    const fadeSp = createSpring(0, SPRING_PRESETS.fade.k, SPRING_PRESETS.fade.c);
    popSp.t = 1;
    fadeSp.t = 1;

    const firstRows = resultsRef.current
      ? Array.from(resultsRef.current.querySelectorAll<HTMLElement>("[data-palette-spring-row]")).slice(0, 8)
      : [];
    const rowSprings = firstRows.map(() =>
      createSpring(0, SPRING_PRESETS.fade.k, SPRING_PRESETS.fade.c)
    );
    firstRows.forEach((el) => {
      el.style.opacity = "0";
      el.style.transform = "translate3d(0, 6px, 0)";
    });

    dialog!.style.opacity = "0";
    dialog!.style.transform = "translate3d(0, -10px, 0) scale3d(0.955, 0.955, 1)";
    dialog!.style.transformOrigin = "top center";

    let last = performance.now();
    let elapsedMs = 0;
    const tickOpen = (now: number) => {
      const dt = Math.min((now - last) / 1000, 1 / 30);
      last = now;
      elapsedMs += dt * 1000;

      stepSpringSubdivided(popSp, dt);
      stepSpringSubdivided(fadeSp, dt);

      const s = 0.955 + 0.045 * popSp.x;
      const y = (1 - popSp.x) * -10;
      const op = clamp(fadeSp.x, 0, 1);

      if (dialogRef.current) {
        dialogRef.current.style.opacity = `${op.toFixed(3)}`;
        dialogRef.current.style.transform = `translate3d(0, ${y.toFixed(2)}px, 0) scale3d(${s.toFixed(4)}, ${s.toFixed(4)}, 1)`;
      }

      let rowsDone = true;
      firstRows.forEach((el, idx) => {
        const rs = rowSprings[idx];
        if (!rs) return;
        if (elapsedMs >= idx * 26) {
          rs.t = 1;
          stepSpringSubdivided(rs, dt);
        }
        const p = clamp(rs.x, 0, 1);
        el.style.opacity = `${p.toFixed(3)}`;
        el.style.transform = `translate3d(0, ${((1 - p) * 6).toFixed(2)}px, 0)`;
        if (!settleIfClose(rs, 0.015, 0.08) || rs.t < 1) rowsDone = false;
      });

      const done =
        settleIfClose(popSp, 0.004, 0.03) &&
        settleIfClose(fadeSp, 0.015, 0.08) &&
        rowsDone;

      if (done) {
        openSpringRafRef.current = null;
        if (dialogRef.current) {
          dialogRef.current.style.opacity = "";
          dialogRef.current.style.transform = "";
          dialogRef.current.style.transformOrigin = "";
        }
        firstRows.forEach((el) => {
          el.style.opacity = "";
          el.style.transform = "";
        });
        return;
      }
      openSpringRafRef.current = window.requestAnimationFrame(tickOpen);
    };
    openSpringRafRef.current = window.requestAnimationFrame(tickOpen);
  }, [isOpen]);

  // 2. 顶部分类标签栏横向弹簧滑块
  useLayoutEffect(() => {
    if (!isOpen) return;
    const container = tabListRef.current;
    const pill = tabHighlightRef.current;
    const targetBtn = tabRefs.current[activeTab] ?? null;
    if (!canAnimateSpringInBrowser(container) || !pill || !targetBtn) {
      setSpringTabsReady(false);
      return;
    }
    setSpringTabsReady(true);

    const rect = measureRelativeLayoutRect(container!, targetBtn);
    targetStretchyHighlight(tabHighlightStateRef.current, rect, false, SPRING_PRESETS.trail);

    if (tabHighlightRafRef.current !== null) {
      window.cancelAnimationFrame(tabHighlightRafRef.current);
      tabHighlightRafRef.current = null;
    }

    let last = performance.now();
    const tickTab = (now: number) => {
      const dt = Math.min((now - last) / 1000, 1 / 30);
      last = now;
      const done = stepStretchyHighlight(tabHighlightStateRef.current, dt, false);
      const m = readStretchyHighlightMetrics(tabHighlightStateRef.current);
      if (tabHighlightRef.current) {
        tabHighlightRef.current.style.transform = `translate3d(${m.left.toFixed(2)}px, ${m.top.toFixed(2)}px, 0)`;
        tabHighlightRef.current.style.width = `${m.width.toFixed(2)}px`;
        tabHighlightRef.current.style.height = `${m.height.toFixed(2)}px`;
        tabHighlightRef.current.style.opacity = `${m.opacity.toFixed(3)}`;
      }
      if (!done) {
        tabHighlightRafRef.current = window.requestAnimationFrame(tickTab);
      } else {
        tabHighlightRafRef.current = null;
      }
    };
    tabHighlightRafRef.current = window.requestAnimationFrame(tickTab);
  }, [activeTab, isOpen, tabCounts]);

  // 3. 搜索结果列表：上下方向键 / 鼠标悬停弹性跟随选中条 + 过滤结果位置弹簧补位
  useLayoutEffect(() => {
    if (!isOpen) return;
    const container = resultsRef.current;
    const pill = rowHighlightRef.current;
    if (!canAnimateSpringInBrowser(container) || !pill) {
      setSpringRowsReady(false);
      return;
    }
    setSpringRowsReady(true);

    const activeBtn = rowRefs.current.get(activeIndex) ?? null;
    if (activeBtn && filteredCommands.length > 0) {
      const rect = measureRelativeLayoutRect(container!, activeBtn);
      targetStretchyHighlight(rowHighlightStateRef.current, rect, false, SPRING_PRESETS.cardTrail);
    } else {
      hideStretchyHighlight(rowHighlightStateRef.current, false);
    }

    if (rowHighlightRafRef.current !== null) {
      window.cancelAnimationFrame(rowHighlightRafRef.current);
      rowHighlightRafRef.current = null;
    }

    let last = performance.now();
    const tickRow = (now: number) => {
      const dt = Math.min((now - last) / 1000, 1 / 30);
      last = now;
      const done = stepStretchyHighlight(rowHighlightStateRef.current, dt, false);
      const m = readStretchyHighlightMetrics(rowHighlightStateRef.current);
      if (rowHighlightRef.current) {
        rowHighlightRef.current.style.transform = `translate3d(${m.left.toFixed(2)}px, ${m.top.toFixed(2)}px, 0)`;
        rowHighlightRef.current.style.width = `${m.width.toFixed(2)}px`;
        rowHighlightRef.current.style.height = `${m.height.toFixed(2)}px`;
        rowHighlightRef.current.style.opacity = `${m.opacity.toFixed(3)}`;
      }
      if (!done) {
        rowHighlightRafRef.current = window.requestAnimationFrame(tickRow);
      } else {
        rowHighlightRafRef.current = null;
      }
    };
    rowHighlightRafRef.current = window.requestAnimationFrame(tickRow);
  }, [activeIndex, filteredCommands, isOpen]);

  // 实时输入过滤时：对前 10 条留存匹配项执行轻量 FLIP 位置弹簧补位
  useLayoutEffect(() => {
    if (!isOpen) return;
    const container = resultsRef.current;
    if (!canAnimateSpringInBrowser(container)) return;

    const prevMap = prevRowTopsRef.current;
    const nextMap = new Map<string, number>();
    const movers: Array<{ el: HTMLElement; sp: ReturnType<typeof createSpring> }> = [];

    const visibleLimit = Math.min(filteredCommands.length, 10);
    for (let i = 0; i < visibleLimit; i += 1) {
      const cmd = filteredCommands[i];
      const btn = rowRefs.current.get(i);
      if (!cmd || !btn) continue;
      const curTop = btn.offsetTop;
      nextMap.set(cmd.id, curTop);
      const prevTop = prevMap.get(cmd.id);
      if (prevTop !== undefined) {
        const delta = clamp(prevTop - curTop, -72, 72);
        if (Math.abs(delta) >= 3) {
          const sp = createSpring(delta, SPRING_PRESETS.fade.k, SPRING_PRESETS.fade.c);
          tuneSpring(sp, SPRING_PRESETS.fade.k, SPRING_PRESETS.fade.c, false);
          sp.t = 0;
          btn.style.transform = `translate3d(0, ${delta.toFixed(2)}px, 0)`;
          movers.push({ el: btn, sp });
        }
      }
    }
    prevRowTopsRef.current = nextMap;

    if (movers.length === 0) return;
    if (flipRafRef.current !== null) {
      window.cancelAnimationFrame(flipRafRef.current);
      flipRafRef.current = null;
    }

    let last = performance.now();
    const tickFlip = (now: number) => {
      const dt = Math.min((now - last) / 1000, 1 / 30);
      last = now;
      let allDone = true;
      for (const m of movers) {
        stepSpringSubdivided(m.sp, dt);
        m.el.style.transform = `translate3d(0, ${m.sp.x.toFixed(2)}px, 0)`;
        if (!settleIfClose(m.sp, 0.25, 1.0)) {
          allDone = false;
        }
      }
      if (allDone) {
        flipRafRef.current = null;
        for (const m of movers) {
          m.el.style.transform = "";
        }
        return;
      }
      flipRafRef.current = window.requestAnimationFrame(tickFlip);
    };
    flipRafRef.current = window.requestAnimationFrame(tickFlip);
  }, [activeTab, filteredCommands, isOpen, query]);

  if (!isOpen) return null;

  return (
    <ModalOverlay
      closeOnBackdropClick={true}
      onClose={() => setOpen(false)}
      role="dialog"
      aria-modal={true}
      aria-label="EOStage 全局命令与工作区面板"
      data-testid="global-command-palette-overlay"
      className="items-start pt-[8vh] p-4 sm:p-6"
    >
      <div
        ref={dialogRef}
        data-testid="global-command-palette"
        data-spring-palette="true"
        onKeyDown={handleKeyDown}
        className="relative flex flex-col w-[min(780px,92vw)] h-[min(560px,80vh)] rounded-[16px] border border-sp-line bg-sp-surface shadow-sp-modal overflow-hidden"
      >
        {/* 顶部搜索输入栏 */}
        <div className="shrink-0 flex items-center gap-2.5 border-b border-sp-line bg-sp-canvas px-4 py-3">
          <Search className="h-4 w-4 shrink-0 text-sp-text-muted" strokeWidth={1.75} />
          <input
            ref={inputRef}
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="搜索工作台功能、DEM 转换、GEE、数据源、分析技能、历史会话或本地成果路径…"
            aria-label="全局搜索指令"
            data-testid="command-palette-search-input"
            className="flex-1 min-w-0 bg-transparent text-sm text-sp-text placeholder:text-sp-text-muted focus:outline-none"
          />
          {query && (
            <button
              type="button"
              aria-label="清空搜索"
              onClick={() => {
                setQuery("");
                inputRef.current?.focus();
              }}
              className="inline-flex h-6 w-6 items-center justify-center rounded-sp-sm text-sp-text-muted hover:text-sp-text hover:bg-sp-hover cursor-pointer"
            >
              <X className="h-3.5 w-3.5" strokeWidth={1.75} />
            </button>
          )}
          <button
            type="button"
            aria-label="关闭命令面板"
            data-testid="command-palette-close-btn"
            onClick={() => setOpen(false)}
            className="inline-flex h-7 items-center gap-1 rounded-sp-sm border border-sp-line bg-sp-surface px-2 text-xs text-sp-text-muted hover:text-sp-text whitespace-nowrap shrink-0 cursor-pointer"
          >
            <kbd className="font-mono">Esc</kbd>
          </button>
        </div>

        {/* 分类切换栏（固定高度，零抖动 + 横向弹簧滑块） */}
        <div
          ref={tabListRef}
          role="tablist"
          aria-label="命令分类"
          className="relative shrink-0 flex items-center gap-1.5 border-b border-sp-line bg-sp-card/60 px-4 py-2 overflow-x-auto"
        >
          <div
            ref={tabHighlightRef}
            data-testid="command-palette-tab-highlight"
            aria-hidden="true"
            className="pointer-events-none absolute top-0 left-0 rounded-sp-sm border border-sp-text-secondary bg-sp-surface opacity-0"
          />
          {PALETTE_TABS.map((tab) => {
            const active = activeTab === tab.key;
            return (
              <button
                key={tab.key}
                ref={(el) => {
                  tabRefs.current[tab.key] = el;
                }}
                type="button"
                role="tab"
                aria-selected={active}
                data-testid={`command-palette-tab-${tab.key}`}
                onClick={() => {
                  setActiveTab(tab.key);
                  inputRef.current?.focus();
                }}
                className={cn(
                  "relative z-10 inline-flex h-7 shrink-0 items-center gap-1.5 rounded-sp-sm border px-2.5 text-xs whitespace-nowrap transition-colors cursor-pointer",
                  active
                    ? cn(
                        "font-semibold text-sp-text",
                        springTabsReady
                          ? "border-transparent bg-transparent"
                          : "border-sp-text-secondary bg-sp-surface"
                      )
                    : "border-transparent bg-transparent text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text",
                )}
              >
                <span>{tab.label}</span>
                <span className="text-xs font-normal text-sp-text-muted tabular-nums">
                  {tabCounts[tab.key]}
                </span>
              </button>
            );
          })}
        </div>

        {/* 结果列表（固定 flex-1 内部滚动 + 纵向弹性跟随选中条） */}
        <div
          ref={resultsRef}
          role="listbox"
          aria-label="搜索结果"
          data-testid="command-palette-results"
          className="relative flex-1 min-h-0 overflow-y-auto p-2.5 space-y-1"
        >
          <div
            ref={rowHighlightRef}
            data-testid="command-palette-spring-highlight"
            aria-hidden="true"
            className="pointer-events-none absolute top-0 left-0 rounded-[10px] bg-sp-hover ring-1 ring-sp-line opacity-0"
          />
          {filteredCommands.length === 0 ? (
            <div className="flex h-full items-center justify-center px-6">
              <EmptyState
                kind="no-results"
                size="menu"
                testId="command-palette-empty"
                title={
                  query.trim()
                    ? `未找到匹配「${query.trim()}」的命令、数据源或会话`
                    : "未找到匹配的命令、数据源或会话"
                }
              />
            </div>
          ) : (
            filteredCommands.map((item, idx) => {
              const isSelected = idx === activeIndex;
              return (
                <button
                  key={item.id}
                  ref={(el) => {
                    if (el) rowRefs.current.set(idx, el);
                    else rowRefs.current.delete(idx);
                  }}
                  type="button"
                  role="option"
                  aria-selected={isSelected}
                  disabled={item.disabled}
                  data-palette-spring-row=""
                  data-testid={`command-palette-item-${item.id}`}
                  data-active={isSelected ? "true" : "false"}
                  onMouseEnter={() => setActiveIndex(idx)}
                  onClick={() => {
                    if (item.disabled) return;
                    closeAndRun(item.onSelect);
                  }}
                  className={cn(
                    "relative z-10 flex w-full items-center justify-between gap-3 rounded-[10px] px-3 py-2 text-left transition-colors cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed",
                    isSelected
                      ? cn(
                          "text-sp-text",
                          !springRowsReady && "bg-sp-hover ring-1 ring-sp-line"
                        )
                      : cn(
                          "text-sp-text",
                          !springRowsReady && "hover:bg-sp-hover/60"
                        ),
                  )}
                >
                  <div className="flex items-center gap-3 min-w-0 flex-1">
                    {renderCommandIcon(item.icon)}
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-2">
                        <span className="truncate text-sm font-medium text-sp-text">
                          {item.title}
                        </span>
                        {activeTab === "all" && (
                          <span className="shrink-0 whitespace-nowrap rounded bg-sp-card px-1.5 py-0.5 text-xs text-sp-text-muted border border-sp-line/70">
                            {item.groupLabel}
                          </span>
                        )}
                      </div>
                      <p className="mt-0.5 truncate text-xs text-sp-text-secondary">
                        {item.subtitle}
                      </p>
                    </div>
                  </div>

                  <div className="flex items-center gap-2 shrink-0">
                    {item.badge && (
                      <span className="whitespace-nowrap rounded-full border border-sp-line bg-sp-card px-2 py-0.5 text-xs text-sp-text-secondary">
                        {item.badge}
                      </span>
                    )}
                    {isSelected && (
                      <CornerDownLeft className="h-3.5 w-3.5 text-sp-text-muted shrink-0" strokeWidth={1.75} />
                    )}
                  </div>
                </button>
              );
            })
          )}
        </div>

        {/* 底部键盘引导条 */}
        <div className="shrink-0 flex items-center justify-between gap-3 border-t border-sp-line bg-sp-canvas px-4 py-2 text-xs text-sp-text-muted">
          <div className="flex items-center gap-3 whitespace-nowrap">
            <span>↑ / ↓ 选择</span>
            <span>Enter 执行</span>
            <span>Esc 关闭</span>
          </div>
          <span className="whitespace-nowrap">
            快捷键 <kbd className="font-mono text-sp-text">Ctrl / ⌘ + K</kbd>
          </span>
        </div>
      </div>
    </ModalOverlay>
  );
}
