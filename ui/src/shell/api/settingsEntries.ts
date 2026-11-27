import type { ComponentType } from "react";
import {
  SlidersHorizontal,
  KeyRound,
  Globe,
  Bot,
  Cpu,
  BarChart3,
  HardDrive,
  ArrowDownUp,
} from "lucide-react";
import { t, type MessageKey } from "../../i18n/index.ts";
import type { SettingsCategory } from "../../components/SettingsModal";

export const LAST_SETTINGS_CATEGORY_STORAGE_KEY =
  "eostage.shell.lastSettingsCategory";

export type SettingsGroupKey = "basic" | "agent" | "data";

export interface SettingsCategoryItem {
  key: SettingsCategory;
  labelKey: MessageKey;
  /** Optional fallback label when i18n locale table is managed in another module. */
  fallbackLabel?: string;
  icon: ComponentType<{ className?: string }>;
  group: SettingsGroupKey;
}

export interface SettingsGroupDefinition {
  key: SettingsGroupKey;
  labelKey: MessageKey;
  items: readonly SettingsCategoryItem[];
}

export function normalizeSettingsCategory(
  raw: string | null | undefined,
): SettingsCategory {
  switch (raw) {
    case "appearance":
    case "models":
    case "auth":
    case "network_proxy":
    case "runtime_tools":
    case "network":
    case "usage_stats":
    case "storage":
      return raw;
    case "plugins_skills":
      return "runtime_tools";
    case "performance_usage":
    case "work_steps":
      return "appearance";
    default:
      return "appearance";
  }
}

export function readLastSettingsCategory(): SettingsCategory {
  try {
    if (typeof window === "undefined" || !window.localStorage) {
      return "appearance";
    }
    const stored = window.localStorage.getItem(LAST_SETTINGS_CATEGORY_STORAGE_KEY);
    return normalizeSettingsCategory(stored);
  } catch {
    return "appearance";
  }
}

export function rememberSettingsCategory(category: SettingsCategory): void {
  try {
    if (typeof window === "undefined" || !window.localStorage) {
      return;
    }
    window.localStorage.setItem(
      LAST_SETTINGS_CATEGORY_STORAGE_KEY,
      normalizeSettingsCategory(category),
    );
  } catch {
    // ignore storage failures in private/test environments
  }
}

export function getSettingsCategoryLabel(item: {
  labelKey: MessageKey;
  fallbackLabel?: string;
}): string {
  const translated = t(item.labelKey);
  if (translated && translated !== item.labelKey) {
    return translated;
  }
  return item.fallbackLabel ?? translated;
}

/**
 * 设置分类清单（供侧栏与设置面板共享，对齐三大功能分组）
 * - 基础设置：外观（含性能与用量、工作步骤显示）、模型设置、数据服务账号、网络与代理
 * - Agent 功能：运行环境与工具（runtime_tools: GDAL/PROJ 及桌面 GIS 引擎）、传输与并发（插件与技能/连接器统一归位至主侧栏「插件与技能」）
 * - 数据与统计：使用统计、存储与路径
 */
export const SETTINGS_CATEGORIES: readonly SettingsCategoryItem[] = [
  { key: "appearance", labelKey: "settings.categories.appearance", icon: SlidersHorizontal, group: "basic" },
  { key: "models", labelKey: "settings.categories.models", icon: Bot, group: "basic" },
  { key: "auth", labelKey: "settings.categories.auth", icon: KeyRound, group: "basic" },
  { key: "network_proxy", labelKey: "settings.categories.networkProxy", icon: Globe, group: "basic" },
  {
    key: "runtime_tools",
    labelKey: "settings.categories.runtimeTools",
    fallbackLabel: "运行环境与工具",
    icon: Cpu,
    group: "agent",
  },
  { key: "network", labelKey: "settings.categories.network", icon: ArrowDownUp, group: "agent" },
  { key: "usage_stats", labelKey: "settings.categories.usageStats", icon: BarChart3, group: "data" },
  { key: "storage", labelKey: "settings.categories.storage", icon: HardDrive, group: "data" },
] as const;

export const SETTINGS_GROUPS: readonly SettingsGroupDefinition[] = [
  {
    key: "basic",
    labelKey: "settings.groups.basic",
    items: SETTINGS_CATEGORIES.filter((item) => item.group === "basic"),
  },
  {
    key: "agent",
    labelKey: "settings.groups.agent",
    items: SETTINGS_CATEGORIES.filter((item) => item.group === "agent"),
  },
  {
    key: "data",
    labelKey: "settings.groups.data",
    items: SETTINGS_CATEGORIES.filter((item) => item.group === "data"),
  },
] as const;

export type SettingsBubbleKind = "modal" | "theme";

export interface SettingsBubbleEntry {
  key: string;
  label: string;
  category: SettingsCategory | null;
  kind: SettingsBubbleKind;
}

const SETTINGS_BUBBLE_ENTRIES = [
  { key: "appearance", labelKey: "settings.categories.appearance", category: "appearance", kind: "modal" },
  { key: "auth", labelKey: "settings.categories.auth", category: "auth", kind: "modal" },
  { key: "network_proxy", labelKey: "settings.categories.networkProxy", category: "network_proxy", kind: "modal" },
  { key: "models", labelKey: "settings.categories.models", category: "models", kind: "modal" },
  { key: "storage", labelKey: "settings.categories.storage", category: "storage", kind: "modal" },
  { key: "network", labelKey: "settings.categories.network", category: "network", kind: "modal" },
] as const;

export function getSettingsBubbleEntries(): readonly SettingsBubbleEntry[] {
  return SETTINGS_BUBBLE_ENTRIES.map(({ labelKey, ...entry }) => ({ ...entry, label: t(labelKey) }));
}

