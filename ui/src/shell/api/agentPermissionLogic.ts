import type {
  AgentPermissionMode,
  AgentPermissionModeView,
} from "../../lib/generated/v3-contracts.ts";
import { t } from "../../i18n/index.ts";

export const FULL_ACCESS_RISK_ACK_KEY = "eostage.agent.fullAccessRiskAcknowledged";

/**
 * 权限选择器三档模式（仅控制工具执行权限，不包含额度或目录设置）。
 * 说明文字按 W5-2g 简报原文。
 */
export const PERMISSION_MODE_ROWS: ReadonlyArray<{
  mode: AgentPermissionMode;
  labelKey: "agent.permission.confirmChanges" | "agent.permission.autoApprove" | "agent.permission.fullAccess";
  descriptionKey: "agent.permission.confirmChangesDescription" | "agent.permission.autoApproveDescription" | "agent.permission.fullAccessDescription";
}> = [
  { mode: "confirm_changes", labelKey: "agent.permission.confirmChanges", descriptionKey: "agent.permission.confirmChangesDescription" },
  {
    mode: "auto_approve",
    labelKey: "agent.permission.autoApprove",
    descriptionKey: "agent.permission.autoApproveDescription",
  },
  { mode: "full_access", labelKey: "agent.permission.fullAccess", descriptionKey: "agent.permission.fullAccessDescription" },
];

function isPermissionMode(value: unknown): value is AgentPermissionMode {
  return value === "confirm_changes" || value === "auto_approve" || value === "full_access";
}

/**
 * 权限模式中文名。W3e 旧值（per_item / within_quota）与未知值返回 null（不编造）；
 * 旧事件里的模式由 approvalLogic 显示为「未知模式」。
 */
export function permissionModeLabelZh(mode: AgentPermissionMode | string | null | undefined): string | null {
  if (!isPermissionMode(mode)) return null;
  return t(`agent.permission.${mode === "confirm_changes" ? "confirmChanges" : mode === "auto_approve" ? "autoApprove" : "fullAccess"}`);
}

/**
 * 权限作用域徽章说明
 * view.session_override 非 null -> "会话设置"
 * 否则 -> "全局默认"
 * effective 取自 view，不自行推算
 */
export function permissionScopeBadge(
  view: AgentPermissionModeView,
  pendingSessionMode?: AgentPermissionMode | null
): string {
  if (pendingSessionMode != null) {
    return t("agent.permission.pending");
  }
  return view.session_override != null ? t("agent.permission.session") : t("agent.permission.global");
}

/**
 * 发送给 agent_permission_mode_set_* 的载荷：只收三个名字（后端 deny 其他值）。
 */
export function permissionModeWire(mode: AgentPermissionMode | string | null | undefined): AgentPermissionMode | null {
  return isPermissionMode(mode) ? mode : null;
}

export function readFullAccessRiskAck(
  storage?: { getItem(k: string): string | null } | null
): boolean {
  if (!storage) return false;
  try {
    return storage.getItem(FULL_ACCESS_RISK_ACK_KEY) === "1";
  } catch {
    return false;
  }
}

export function writeFullAccessRiskAck(
  storage?: { setItem(k: string, v: string): void } | null
): void {
  if (!storage) return;
  try {
    storage.setItem(FULL_ACCESS_RISK_ACK_KEY, "1");
  } catch {
    // 异常吞掉
  }
}

export function needsFullAccessRiskPrompt(
  targetMode: AgentPermissionMode | string | null | undefined,
  acked: boolean
): boolean {
  return targetMode === "full_access" && !acked;
}

export type ApplyPendingSessionModeResult =
  | { ok: true; view?: AgentPermissionModeView }
  | { ok: false; error?: unknown };

/**
 * 发送前应用待用的会话权限模式
 * 纯异步函数，方便单元测试与独立验证
 */
export async function applyPendingSessionMode(
  setSession: (
    sessionId: string,
    mode: AgentPermissionMode
  ) => Promise<AgentPermissionModeView>,
  sessionId: string,
  pending: AgentPermissionMode | null
): Promise<ApplyPendingSessionModeResult> {
  if (!pending) {
    return { ok: true };
  }
  try {
    const view = await setSession(sessionId, pending);
    return { ok: true, view };
  } catch (error) {
    return { ok: false, error };
  }
}

export interface PopoverPlacementOptions {
  triggerTop: number;
  triggerBottom: number;
  viewportHeight: number;
  desiredHeight: number;
  topInset?: number;
  margin?: number;
}

export interface PopoverPlacementResult {
  side: "up" | "down";
  maxHeight: number;
}

/**
 * DesktopTitlebar 实际高度（对应 DesktopTitlebar.tsx 中 `<header className="... h-10 ...">`，Tailwind h-10 = 40px）
 * 用于弹层计算上方可用空间时扣除标题栏高度，防止向上展开时盖住标题栏
 */
export const DESKTOP_TITLEBAR_HEIGHT = 40;

/**
 * 弹层方位与最大高度纯判定函数
 * - 下方可用空间：viewportHeight - triggerBottom
 * - 上方可用空间：triggerTop - topInset（扣除顶栏/标题栏高度，防遮挡）
 * - 下方够（spaceBelow >= desiredHeight） -> down
 * - 下方不够上方够（spaceAbove >= desiredHeight） -> up
 * - 都不够 -> 取空间较大的一侧（若 spaceAbove > spaceBelow 则 "up"，否则 "down"）
 * - maxHeight = 该侧可用空间 - 8（且 >= 0）
 */
export function popoverPlacement(
  options: PopoverPlacementOptions
): PopoverPlacementResult {
  const { triggerTop, triggerBottom, viewportHeight, desiredHeight, topInset = 0, margin = 0 } = options;
  // margin：弹层与所在边界（标题栏下沿或窗口底边）之间额外保留的距离；8 是弹层与触发按钮的间隙
  const spaceBelow = Math.max(0, viewportHeight - triggerBottom - margin);
  const spaceAbove = Math.max(0, triggerTop - topInset - margin);

  if (spaceBelow >= desiredHeight) {
    return {
      side: "down",
      maxHeight: Math.max(0, spaceBelow - 8),
    };
  }
  if (spaceAbove >= desiredHeight) {
    return {
      side: "up",
      maxHeight: Math.max(0, spaceAbove - 8),
    };
  }
  const chooseUp = spaceAbove > spaceBelow;
  const chosenSpace = chooseUp ? spaceAbove : spaceBelow;
  return {
    side: chooseUp ? "up" : "down",
    maxHeight: Math.max(0, chosenSpace - 8),
  };
}

/**
 * 计算用于界面显示的权限模式视图
 * 纯函数：
 * - 若 backendView 为 null/undefined：直接返回 null（绝不编造 per_item 全局默认）
 * - 若 pendingSessionMode 非 null：覆盖 effective 与 session_override 为待用模式，global_default 保留后端视图
 * - 若 pendingSessionMode 为 null：原样返回 backendView
 */
export function displayedPermissionView(
  backendView: AgentPermissionModeView | null | undefined,
  pendingSessionMode: AgentPermissionMode | null | undefined
): AgentPermissionModeView | null {
  if (!backendView) {
    return null;
  }
  if (!pendingSessionMode) {
    return backendView;
  }
  return {
    effective: pendingSessionMode,
    session_override: pendingSessionMode,
    global_default: backendView.global_default,
  };
}
