import { t } from "../i18n/index.ts";
import type { ApplicationSearchProgress as GeneratedApplicationSearchProgress } from "./generated/v3-contracts.ts";

export type ApplicationSearchProgress = GeneratedApplicationSearchProgress;

type LiveMetadataStatus = {
  state: string;
  done: number;
  total: number;
  percent: number;
  message: string;
  quantitative?: boolean;
};

export class ApplicationSearchProgressSequenceGuard {
  private readonly latest = new Map<string, { sequence: number; terminal: boolean }>();

  accept(progress: ApplicationSearchProgress): boolean {
    if (!Number.isSafeInteger(progress.sequence) || progress.sequence < 1) return false;
    const current = this.latest.get(progress.request_id);
    if (current && progress.sequence <= current.sequence) return false;
    if (current?.terminal && !progress.terminal) return false;
    this.latest.set(progress.request_id, {
      sequence: progress.sequence,
      terminal: progress.terminal,
    });
    return true;
  }

  reset(requestId?: string): void {
    if (requestId) {
      this.latest.delete(requestId);
    } else {
      this.latest.clear();
    }
  }
}

export type SarSearchStatusPresentation = {
  message: string;
  percent: number;
  indeterminate: boolean;
};

function boundedCount(value: unknown): number | null {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0
    ? value
    : null;
}

function queryRecord(query: unknown): Record<string, unknown> | null {
  return query !== null && typeof query === "object" && !Array.isArray(query)
    ? query as Record<string, unknown>
    : null;
}

export function sarSearchTargetCount(query: unknown): number | null {
  const record = queryRecord(query);
  const target = boundedCount(record?.max_results) ?? boundedCount(record?.result_limit);
  return target !== null && target >= 1 && target <= 10_000 ? target : null;
}

/** Left-panel status is deliberately live-only; completed facts belong to the result dock. */
export function sarSearchStatusPresentation(input: {
  busy: boolean;
  cancelling?: boolean;
  metadataStatus: LiveMetadataStatus | null;
  applicationProgress?: ApplicationSearchProgress | null;
  heartbeatElapsedMs?: number;
  sourceLabel?: string;
}): SarSearchStatusPresentation | null {
  if (!input.busy) return null;
  const sourceLabel = input.sourceLabel?.trim() || "SAR";
  if (input.cancelling) {
    return { message: t("shared.sarSearchStatus.stoppingSearch", { sourceLabel }), percent: 0, indeterminate: true };
  }
  const progress = input.applicationProgress;
  if (progress) {
    const admitted = boundedCount(progress.admitted_count);
    const denominator = boundedCount(progress.requested_limit);
    const determinate = admitted !== null && denominator !== null && denominator > 0;
    const percent = determinate
      ? Math.max(0, Math.min(100, Math.round((admitted / denominator) * 100)))
      : 0;
    const heartbeat = Math.max(progress.elapsed_ms, input.heartbeatElapsedMs ?? 0);
    const heartbeatSuffix = !progress.terminal && heartbeat >= 1_000
      ? t("shared.sarSearchStatus.waitedSeconds", { seconds: Math.floor(heartbeat / 1_000) })
      : "";
    return {
      message: `${progress.message}${heartbeatSuffix}`,
      percent,
      indeterminate: !determinate,
    };
  }
  const status = input.metadataStatus;
  const determinate = !!status && status.quantitative !== false && boundedCount(status.total) !== 0;
  const percent = determinate
    ? Math.max(0, Math.min(100, Number.isFinite(status.percent) ? status.percent : 0))
    : 0;
  return {
    message: status?.message || t("shared.sarSearchStatus.preparingSearch", { sourceLabel }),
    percent,
    indeterminate: !determinate,
  };
}
