import type { DesktopGacosSubmissionState } from "./desktopShellBridge.ts";

const labels: Record<DesktopGacosSubmissionState, string> = {
  reserved: "尚未发送",
  submitting: "正在提交",
  submitted: "申请已提交",
  manual_verification_required: "网页未明确确认，请人工核对",
  submission_uncertain: "提交结果无法确认，请勿重复申请",
  rejected: "申请未提交或已被拒绝",
};

export function gacosSubmissionStateLabel(state: unknown): string {
  return typeof state === "string" && Object.hasOwn(labels, state)
    ? labels[state as DesktopGacosSubmissionState]
    : "申请状态未知";
}
