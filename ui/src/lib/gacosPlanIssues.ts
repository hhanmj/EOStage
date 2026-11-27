export type GacosPlanIssue = {
  code: string;
  severity: "error" | "warning" | "info";
  message: string;
  action: string;
};

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown> : null;
}

const guidance: Record<string, { message: string; action: string }> = {
  GACOS_BBOX_TOO_LARGE: {
    message: "请求范围超过服务允许的 10° 跨度。",
    action: "缩小 AOI，并检查缓冲后的最终请求范围；重新生成请求清单。",
  },
  GACOS_TIME_CONFLICT: {
    message: "同一场景标识对应了不同的采集时间。",
    action: "核对并更正来源记录的 UTC 采集时间，再重新导入和生成清单。",
  },
  GACOS_TIME_REQUIRED: {
    message: "部分场景或日期缺少明确的 UTC 小时和分钟。",
    action: "填写 UTC 小时和分钟后重新生成请求清单；不要用本地时间代替 UTC。",
  },
};

function safeMessage(value: unknown): string {
  if (typeof value !== "string") return "";
  const text = value.trim();
  // Reports are secret-free by contract; still do not expose raw URLs or credential text.
  if (!text || /(?:https?:\/\/|[A-Z]:\\|[\w.+-]+@[\w.-]+|password|token|secret|authorization|bearer)/i.test(text)) return "";
  return text.length > 500 ? `${text.slice(0, 500)}…` : text;
}

/** Presentation only. Never changes report.has_errors or authorizes submission. */
export function gacosPlanIssues(report: unknown): GacosPlanIssue[] {
  const source = record(report);
  if (!source) return [];
  const issues: GacosPlanIssue[] = [];
  const seen = new Set<string>();
  for (const value of Array.isArray(source.issues) ? source.issues : []) {
    const item = record(value);
    if (!item) continue;
    const code = typeof item.code === "string" && /^[A-Z][A-Z0-9_]{1,95}$/.test(item.code)
      ? item.code : "GACOS_PLAN_ISSUE";
    const severity = item.severity === "warning" ? "warning" : item.severity === "info" ? "info" : "error";
    const known = guidance[code];
    const message = known?.message || safeMessage(item.message) || "预览报告存在未说明的问题。";
    const key = `${code}:${severity}:${message}`;
    if (seen.has(key)) continue;
    seen.add(key);
    issues.push({ code, severity, message, action: known?.action || "核对输入与诊断记录后重新生成清单；不要绕过提交前校验。" });
  }
  if (source.has_errors && !issues.some((issue) => issue.severity === "error")) {
    issues.unshift({
      code: "GACOS_PLAN_BLOCKED",
      severity: "error",
      message: "服务端阻断了请求预览，但没有返回可展示的具体错误项。",
      action: "保留当前输入并查看诊断记录。缺少原因不是输入已通过，暂不能提交。",
    });
  }
  return issues;
}
