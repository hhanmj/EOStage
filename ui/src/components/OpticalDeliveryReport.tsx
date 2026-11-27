import { useEffect, useState } from "react";
import type { DesktopOpticalDeliveryAdmission } from "@/lib/desktopShellBridge";
import { loadTaskDeliveryAdmission } from "@/lib/bridge";
import { opticalAdmissionDiagnostic, opticalAdmissionReason } from "@/lib/opticalDeliveryAdmission";
import { Button } from "@/components/ui/button";

/** On-demand projection of the persisted task report, never a second task store. */
export function OpticalDeliveryReport({ taskId, fileCount }: { taskId: string; fileCount: number }) {
  const [resolved, setResolved] = useState<{ taskId: string; report: DesktopOpticalDeliveryAdmission | null } | null>(null);
  const [failedTaskId, setFailedTaskId] = useState<string | null>(null);
  const report = resolved?.taskId === taskId ? resolved.report : null;
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let current = true;
    setResolved(null); setFailedTaskId(null);
    void loadTaskDeliveryAdmission(taskId).then(result => {
      if (!current) return;
      if (result.ok) setResolved({ taskId, report: result.admission });
      else setFailedTaskId(taskId);
    }).catch(() => { if (current) setFailedTaskId(taskId); });
    return () => { current = false; };
  }, [taskId, attempt]);
  if (failedTaskId === taskId) return <div role="alert" className="flex items-center gap-2 px-5 py-2 text-xs text-destructive">
    下载准备记录读取失败。
    <Button size="sm" variant="ghost" onClick={() => setAttempt(value => value + 1)}>重读记录</Button>
  </div>;
  if (!report) return null;
  return <section aria-label="下载准备记录" className="border-b px-5 py-2 text-xs">
    <p>请求 {report.requested_count} 景 · 已加入 {report.admitted_count} 景 · 未加入 {report.rejected_count} 景 · 下载文件 {fileCount} 个</p>
    {report.rejections.length > 0 && <details open className="mt-2">
      <summary className="cursor-pointer font-medium">未加入的影像</summary>
      <div className="mt-1 max-h-44 space-y-1 overflow-auto">
        {report.rejections.map(row => <div key={row.source_item_id} className="min-w-0 break-all" title={row.source_item_id}>
          <span className="font-mono">{row.source_item_id.split("::ITEM::").at(-1)}</span>
          {"："}{opticalAdmissionReason(row.reason_code, row.reason_detail)} <span className="text-muted-foreground">（{opticalAdmissionDiagnostic(row)}）</span>
        </div>)}
      </div>
    </details>}
  </section>;
}
