/**
 * Agent Composer 输入框与附加操作逻辑纯函数模块
 * 规范：W5-2a
 */

import type { AgentChannelView } from "./agentBridgeTypes.ts";
import { t } from "../../i18n/index.ts";
import { projectChannelView } from "./agentChannelsLogic.ts";

export type AgentAttachItemId = "rect-draw" | "admin-region" | "vector-upload" | "workflow" | "skills-tools";

export interface AgentAttachItem {
  id: AgentAttachItemId;
  label_zh: string;
  aside_zh: string | null;
}

export interface AgentAttachCapabilities {
  rectDraw: boolean;
  adminBoundary: boolean;
  vectorUpload: boolean;
  workflow?: boolean;
  skillsTools?: boolean;
}

export interface ApprovalModeOption {
  value: string;
  label_zh: string;
}

export interface AgentApprovalModeResult {
  options: ApprovalModeOption[];
  switchingDisabled: boolean;
}

export interface ComposerChannelOption {
  id: string;
  label_zh: string;
  isDefault: boolean;
  keyReady: boolean;
}

export interface ProjectChannelOptionsResult {
  options: ComposerChannelOption[];
  hasAny: boolean;
  allKeysMissing: boolean;
}

export const AGENT_VECTOR_EXTENSIONS: readonly string[] = [
  ".shp",
  ".zip",
  ".geojson",
  ".kml",
  ".kmz",
  ".json",
] as const;

/**
 * 获取可用的附加操作项列表（保持样稿顺序，能力不支持的项完全省略）
 */
export function agentAttachItems(
  capabilities: AgentAttachCapabilities
): AgentAttachItem[] {
  const items: AgentAttachItem[] = [];
  if (capabilities.rectDraw) {
    items.push({
      id: "rect-draw",
      label_zh: t("agent.composer.rectDraw"),
      aside_zh: null,
    });
  }
  if (capabilities.adminBoundary) {
    items.push({
      id: "admin-region",
      label_zh: t("agent.composer.adminRegion"),
      aside_zh: null,
    });
  }
  if (capabilities.vectorUpload) {
    items.push({
      id: "vector-upload",
      label_zh: t("agent.composer.uploadVector"),
      aside_zh: ".shp .geojson .kml",
    });
  }
  if (capabilities.workflow) {
    items.push({
      id: "workflow",
      label_zh: t("agent.composer.workflow"),
      aside_zh: t("agent.composer.workflowHint"),
    });
  }
  if (capabilities.skillsTools) {
    items.push({
      id: "skills-tools",
      label_zh: t("agent.composer.skillsTools"),
      aside_zh: t("agent.composer.skillsToolsHint"),
    });
  }
  return items;
}

/**
 * 审批模式选项映射（后端当前仅支持 confirm_each，禁用模式切换，忽略未知模式）
 */
export function agentApprovalModeOptions(
  supported: readonly string[]
): AgentApprovalModeResult {
  const options: ApprovalModeOption[] = [];
  for (const item of supported) {
    if (item === "confirm_each") {
      if (!options.some((opt) => opt.value === "confirm_each")) {
        options.push({ value: "confirm_each", label_zh: t("agent.composer.confirmEach") });
      }
    }
  }
  return {
    options,
    switchingDisabled: true,
  };
}

/**
 * 投射渠道选项列表（默认渠道排在最前，格式为 协议名 · 模型名）
 */
export function projectChannelOptions(
  views: AgentChannelView[]
): ProjectChannelOptionsResult {
  const hasAny = Array.isArray(views) && views.length > 0;
  const allKeysMissing = hasAny && views.every((v) => !v.key_ready);

  if (!hasAny) {
    return {
      options: [],
      hasAny: false,
      allKeysMissing: false,
    };
  }

  const defaultChannel = views.find((v) => v.is_default);
  const otherChannels = views.filter((v) => !v.is_default);

  const sortedViews = defaultChannel
    ? [defaultChannel, ...otherChannels]
    : views;

  const options: ComposerChannelOption[] = sortedViews.map((v) => {
    const projected = projectChannelView(v);
    return {
      id: v.id,
      label_zh: `${projected.protocol_zh} · ${v.model}`,
      isDefault: Boolean(v.is_default),
      keyReady: Boolean(v.key_ready),
    };
  });

  return {
    options,
    hasAny,
    allKeysMissing,
  };
}

/**
 * 计算输入框渠道指示标签文本
 */
export function composerChannelLabel(state: {
  hasAny: boolean;
  allKeysMissing: boolean;
  ready: boolean;
}): string {
  return state.ready && state.hasAny ? t("agent.composer.defaultChannel") : t("agent.composer.keyRequired");
}

/**
 * 发送门控条件判定
 */
export function canSendComposer(input: {
  text: string;
  channelReady: boolean;
  busy?: boolean;
}): boolean {
  return Boolean(input.text.trim()) && input.channelReady && !input.busy;
}

/**
 * 检查文件是否为受支持的矢量文件扩展名（不区分大小写）
 */
export function isAcceptedVectorFileName(name: string): boolean {
  if (!name || typeof name !== "string") return false;
  const trimmed = name.trim();
  const cleanName = trimmed.split(/[\\/]/).pop() || "";
  const dotIndex = cleanName.lastIndexOf(".");
  if (dotIndex <= 0 || dotIndex === cleanName.length - 1) {
    return false;
  }
  const ext = cleanName.slice(dotIndex).toLowerCase();
  return (AGENT_VECTOR_EXTENSIONS as readonly string[]).includes(ext);
}

/**
 * 从矢量文件名中提取展示名称（去除扩展名）
 */
export function vectorAoiDisplayName(fileName: string): string {
  if (!fileName || typeof fileName !== "string") return "";
  const cleanName = fileName.trim().split(/[\\/]/).pop() || "";
  const dotIndex = cleanName.lastIndexOf(".");
  if (dotIndex > 0) {
    return cleanName.slice(0, dotIndex);
  }
  return cleanName;
}
