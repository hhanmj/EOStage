import { AlertCircle, Info } from "lucide-react";
import { gacosPlanIssues } from "@/lib/gacosPlanIssues";

/** Displays the native planning report; it does not own submission eligibility. */
export function GacosPlanIssues({ report }: { report: unknown }) {
  const issues = gacosPlanIssues(report);
  if (issues.length === 0) return null;
  const blocked = issues.some((issue) => issue.severity === "error");
  return (
    <section aria-label="GACOS 预览问题" role={blocked ? "alert" : "status"}
      className="space-y-2 rounded-lg border border-warning/40 bg-warning/5 p-3 text-xs">
      <h3 className="flex items-center gap-2 font-semibold">
        {blocked ? <AlertCircle className="h-4 w-4 shrink-0 text-warning" /> : <Info className="h-4 w-4 shrink-0" />}
        {blocked ? "提交前需要处理的问题" : "请求预览提示"}
      </h3>
      <ul className="divide-y divide-border">
        {issues.map((issue) => (
          <li key={`${issue.code}:${issue.severity}:${issue.message}`} className="space-y-1 py-2 first:pt-0 last:pb-0">
            <p className="break-words font-medium">{issue.message}</p>
            <p className="break-words leading-5 text-muted-foreground">{issue.action}</p>
            <code className="block break-all text-[11px] text-muted-foreground">{issue.code}</code>
          </li>
        ))}
      </ul>
    </section>
  );
}
