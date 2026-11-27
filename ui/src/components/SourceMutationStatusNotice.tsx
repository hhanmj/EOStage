import React from "react";
import {
  AlertCircle,
  AlertTriangle,
  CheckCircle2,
  Clock,
  FileText,
  HelpCircle,
  Info,
} from "lucide-react";
import type {
  DesktopSourceMutationCommitStatus,
  DesktopSourceMutationStatusResult,
  DesktopSourceMutationTargetStatus,
} from "../lib/desktopShellBridge";
import { Badge } from "./ui/badge";
import { cn } from "../lib/utils";

export type SourceMutationLanguage = "zh-CN" | "en" | "ja";

export interface SourceMutationStatusNoticeProps {
  /**
   * 来源变更状态查询结果（来自 DesktopShellBridge.querySourceMutationStatus）。
   */
  result?: DesktopSourceMutationStatusResult | null;
  /**
   * 兼容属性名。
   */
  statusResult?: DesktopSourceMutationStatusResult | null;
  /**
   * 界面展示语言（zh-CN、en、ja）。缺省为 "zh-CN"。
   */
  language?: SourceMutationLanguage;
  /**
   * 自定义外层样式类。
   */
  className?: string;
}

interface StatusItemCopy {
  badge: string;
  title: string;
  description: string;
  badgeVariant: "default" | "neutral" | "success" | "warning" | "outline";
}

const I18N_STRINGS: Record<
  SourceMutationLanguage,
  {
    title: string;
    commitSectionLabel: string;
    targetSectionLabel: string;
    queryUnavailableTitle: string;
    queryUnavailableDescription: string;
    commits: Record<DesktopSourceMutationCommitStatus, StatusItemCopy>;
    targets: Record<DesktopSourceMutationTargetStatus, StatusItemCopy>;
  }
> = {
  "zh-CN": {
    title: "来源变更状态（查询时的快照）",
    commitSectionLabel: "提交凭据 (Commit)",
    targetSectionLabel: "目标记录 (Target)",
    queryUnavailableTitle: "查询不可用",
    queryUnavailableDescription: "无法获取来源变更状态，请稍后重试或检查系统连接。",
    commits: {
      committed: {
        badge: "已提交",
        title: "已记录提交凭据",
        description: "已记录独立提交收据凭证，确认变更事实已持久化写入。",
        badgeVariant: "success",
      },
      legacy_committed: {
        badge: "历史证据",
        title: "历史提交证据（仅为历史事件证据）",
        description: "仅作为旧版历史事件证据存在，未包含新版独立凭据。",
        badgeVariant: "neutral",
      },
      event_committed: {
        badge: "事件记录",
        title: "事件流提交记录（仅为历史事件证据）",
        description: "活动事件流中记录过此事件；仅为历史事件证据，无独立提交收据。",
        badgeVariant: "neutral",
      },
      conflict: {
        badge: "提交冲突",
        title: "检测到提交冲突",
        description: "请求身份与已有记录冲突，无法据此判断原请求是否完成。",
        badgeVariant: "warning",
      },
      unknown: {
        badge: "未知",
        title: "状态未知（仅表示未知）",
        description: "明确仅表示当前状态未知，并不代表操作已停止或失败。",
        badgeVariant: "outline",
      },
    },
    targets: {
      present: {
        badge: "记录存在",
        title: "底层记录存在（仅表示记录存在）",
        description: "底层快照与运行时数据行存在；只表示记录存在，不代表任务当前可操作或界面可用。",
        badgeVariant: "neutral",
      },
      tombstoned: {
        badge: "已标记墓碑",
        title: "已废弃 / 标记墓碑",
        description: "查询时目标已标记删除。",
        badgeVariant: "outline",
      },
      missing: {
        badge: "记录不存在",
        title: "目标记录不存在",
        description: "查询时未找到目标的快照和运行时记录。",
        badgeVariant: "neutral",
      },
    },
  },
  en: {
    title: "Source mutation status at query time",
    commitSectionLabel: "Commit Status",
    targetSectionLabel: "Target Record Status",
    queryUnavailableTitle: "Query Unavailable",
    queryUnavailableDescription: "Unable to retrieve source mutation status; please check system connection or retry later.",
    commits: {
      committed: {
        badge: "Committed",
        title: "Commit Receipt Recorded",
        description: "An independent commit receipt has been recorded; the mutation is confirmed.",
        badgeVariant: "success",
      },
      legacy_committed: {
        badge: "Legacy Evidence",
        title: "Legacy Commit Evidence (historical event evidence only)",
        description: "Retained only as historical event evidence; does not carry an independent receipt.",
        badgeVariant: "neutral",
      },
      event_committed: {
        badge: "Event Evidence",
        title: "Event Stream Record (historical event evidence only)",
        description: "Recorded in the activity log only, as historical evidence without an independent receipt.",
        badgeVariant: "neutral",
      },
      conflict: {
        badge: "Conflict",
        title: "Commit Conflict Detected",
        description: "The request identity conflicts with an existing record; the original outcome cannot be determined from this.",
        badgeVariant: "warning",
      },
      unknown: {
        badge: "Unknown",
        title: "Unknown (state is unknown only)",
        description: "State is unknown only; does not imply that the background operation stopped or failed.",
        badgeVariant: "outline",
      },
    },
    targets: {
      present: {
        badge: "Present",
        title: "Record Present (indicates existence only)",
        description: "Snapshot and runtime rows exist; indicates only that the record exists, not that the task is actionable or valid in UI.",
        badgeVariant: "neutral",
      },
      tombstoned: {
        badge: "Tombstoned",
        title: "Tombstoned / Marked Obsolete",
        description: "The target was marked deleted at query time.",
        badgeVariant: "outline",
      },
      missing: {
        badge: "Missing",
        title: "Target Record Missing",
        description: "Neither snapshot nor runtime rows were found at query time.",
        badgeVariant: "neutral",
      },
    },
  },
  ja: {
    title: "照会時点のソース変更ステータス",
    commitSectionLabel: "コミット状態",
    targetSectionLabel: "ターゲット状態",
    queryUnavailableTitle: "照会を利用できません",
    queryUnavailableDescription: "ソース変更ステータスを取得できません。接続を確認するか、しばらくしてから再試行してください。",
    commits: {
      committed: {
        badge: "コミット済み",
        title: "コミット受領証記録済み",
        description: "独立したコミット受領証が記録されており、変更が永続化されています。",
        badgeVariant: "success",
      },
      legacy_committed: {
        badge: "過去の証拠",
        title: "レガシーコミット記録（過去のイベント証拠のみ）",
        description: "過去のイベント証拠としてのみ保持されており、独立した受領証はありません。",
        badgeVariant: "neutral",
      },
      event_committed: {
        badge: "イベント記録",
        title: "イベント記録（過去のイベント証拠のみ）",
        description: "アクティビティログにのみ記録されており、独立した受領証はありません（過去のイベント証拠のみ）。",
        badgeVariant: "neutral",
      },
      conflict: {
        badge: "競合",
        title: "コミット競合が検出されました",
        description: "要求の識別情報が既存の記録と競合しています。元の要求が完了したかどうかは判断できません。",
        badgeVariant: "warning",
      },
      unknown: {
        badge: "不明",
        title: "不明（単に状態が不明であることを示します）",
        description: "状態が不明であることを示しているだけであり、処理が停止または失敗したことを意味しません。",
        badgeVariant: "outline",
      },
    },
    targets: {
      present: {
        badge: "レコード存在",
        title: "レコード存在（存在の確認のみ）",
        description: "スナップショットおよび実行時データ行が存在します。これは記録の存在を示すのみで、タスクが操作可能であることを意味しません。",
        badgeVariant: "neutral",
      },
      tombstoned: {
        badge: "破棄済み",
        title: "破棄済み / トゥームストーン",
        description: "照会時点で対象には削除マークが付いています。",
        badgeVariant: "outline",
      },
      missing: {
        badge: "不在",
        title: "対象レコードが存在しません",
        description: "照会時点で対象のスナップショットと実行時行はいずれも見つかりません。",
        badgeVariant: "neutral",
      },
    },
  },
};

function renderCommitIcon(commit: DesktopSourceMutationCommitStatus) {
  switch (commit) {
    case "committed":
      return <CheckCircle2 aria-hidden="true" className="h-4 w-4 shrink-0 text-success" />;
    case "legacy_committed":
      return <FileText aria-hidden="true" className="h-4 w-4 shrink-0 text-muted-foreground" />;
    case "event_committed":
      return <Clock aria-hidden="true" className="h-4 w-4 shrink-0 text-muted-foreground" />;
    case "conflict":
      return <AlertTriangle aria-hidden="true" className="h-4 w-4 shrink-0 text-warning" />;
    case "unknown":
    default:
      return <HelpCircle aria-hidden="true" className="h-4 w-4 shrink-0 text-muted-foreground" />;
  }
}

function renderTargetIcon(target: DesktopSourceMutationTargetStatus) {
  switch (target) {
    case "present":
      return <Info aria-hidden="true" className="h-4 w-4 shrink-0 text-primary" />;
    case "tombstoned":
      return <AlertCircle aria-hidden="true" className="h-4 w-4 shrink-0 text-muted-foreground" />;
    case "missing":
    default:
      return <HelpCircle aria-hidden="true" className="h-4 w-4 shrink-0 text-muted-foreground" />;
  }
}

/**
 * EOStage 前端纯展示组件：展示来源数据变更的状态查询结果。
 * 遵循只读规范：不展示原始请求与目录，不提供清除、重试或释放门禁等操作。
 */
export function SourceMutationStatusNotice({
  result,
  statusResult,
  language = "zh-CN",
  className,
}: SourceMutationStatusNoticeProps) {
  const currentResult = result ?? statusResult;
  if (!currentResult) {
    return null;
  }

  const lang: SourceMutationLanguage =
    language === "en" || language === "ja" ? language : "zh-CN";
  const strings = I18N_STRINGS[lang];

  if (!currentResult.ok) {
    return (
      <div
        role="status"
        aria-live="polite"
        className={cn(
          "rounded-lg border border-destructive/20 bg-destructive/5 p-3 text-xs text-foreground",
          className,
        )}
      >
        <div className="flex items-start gap-2">
          <AlertCircle aria-hidden="true" className="mt-0.5 h-4 w-4 shrink-0 text-destructive" />
          <div className="min-w-0 space-y-0.5">
            <div className="flex items-center gap-1.5 font-medium text-destructive">
              <span>{strings.queryUnavailableTitle}</span>
            </div>
            <p className="text-muted-foreground leading-relaxed">
              {strings.queryUnavailableDescription}
            </p>
          </div>
        </div>
      </div>
    );
  }

  const { commit, target } = currentResult.status;
  const commitCopy = strings.commits[commit];
  const targetCopy = strings.targets[target];
  if (!commitCopy || !targetCopy) return null;

  return (
    <div
      role="region"
      aria-label={strings.title}
      className={cn(
        "rounded-lg border border-border bg-card p-3.5 text-xs text-foreground shadow-card space-y-3",
        className,
      )}
    >
      <div className="text-xs font-semibold text-muted-foreground">
        {strings.title}
      </div>

      <div className="grid grid-cols-1 gap-2.5 sm:grid-cols-2">
        {/* 提交凭据状态 */}
        <div
          role="group"
          aria-label={strings.commitSectionLabel}
          className="flex flex-col justify-between rounded-md border border-border/70 bg-background/50 p-2.5 space-y-1.5"
        >
          <div className="flex items-center justify-between gap-1.5">
            <span className="font-semibold text-foreground/90 flex items-center gap-1.5">
              {renderCommitIcon(commit)}
              {strings.commitSectionLabel}
            </span>
            <Badge variant={commitCopy.badgeVariant} className="text-[11px] px-1.5 py-0 shrink-0">
              {commitCopy.badge}
            </Badge>
          </div>
          <div className="space-y-0.5">
            <p className="font-medium text-foreground">{commitCopy.title}</p>
            <p className="text-muted-foreground leading-relaxed">{commitCopy.description}</p>
          </div>
        </div>

        {/* 目标记录状态 */}
        <div
          role="group"
          aria-label={strings.targetSectionLabel}
          className="flex flex-col justify-between rounded-md border border-border/70 bg-background/50 p-2.5 space-y-1.5"
        >
          <div className="flex items-center justify-between gap-1.5">
            <span className="font-semibold text-foreground/90 flex items-center gap-1.5">
              {renderTargetIcon(target)}
              {strings.targetSectionLabel}
            </span>
            <Badge variant={targetCopy.badgeVariant} className="text-[11px] px-1.5 py-0 shrink-0">
              {targetCopy.badge}
            </Badge>
          </div>
          <div className="space-y-0.5">
            <p className="font-medium text-foreground">{targetCopy.title}</p>
            <p className="text-muted-foreground leading-relaxed">{targetCopy.description}</p>
          </div>
        </div>
      </div>
    </div>
  );
}

export default SourceMutationStatusNotice;
