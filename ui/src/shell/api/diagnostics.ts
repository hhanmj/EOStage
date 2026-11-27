import {
  clearDiagnosticReports,
  getDiagnosticsState,
  listDiagnosticReports,
  recordDiagnosticReport,
  setDiagnosticsEnabled,
  uploadDiagnosticReports,
} from "../../lib/bridge.ts";

export interface DiagnosticsState {
  enabled: boolean;
  total_count: number;
  pending_count: number;
}

export interface SanitizedDiagnosticRecord {
  report_id: string;
  created_at: string;
  app_version: string;
  os: string;
  arch: string;
  category: string;
  error_code: string;
  http_status?: number | null;
  source: string;
  sanitized_message: string;
  sanitized_stack: string;
  uploaded?: boolean;
}

export interface DiagnosticsUploadResult {
  attempted: number;
  uploaded: number;
  failed: number;
  message_zh: string;
}

let captureInstalled = false;
const recentSignatures = new Map<string, number>();
const CLIENT_THROTTLE_MS = 30_000;

export {
  clearDiagnosticReports,
  getDiagnosticsState,
  listDiagnosticReports,
  setDiagnosticsEnabled,
  uploadDiagnosticReports,
};

export async function recordDiagnosticError(input: {
  category: "frontend_error" | "unhandled_rejection" | "react_boundary" | "backend_error";
  errorCode?: string;
  httpStatus?: number | null;
  source?: string;
  message: string;
  stack?: string;
}): Promise<void> {
  const sig = `${input.category}|${input.errorCode ?? ""}|${input.source ?? ""}|${input.message.slice(0, 80)}`;
  const now = Date.now();
  const prev = recentSignatures.get(sig);
  if (prev !== undefined && now - prev < CLIENT_THROTTLE_MS) {
    return;
  }
  recentSignatures.set(sig, now);
  if (recentSignatures.size > 40) {
    const oldest = recentSignatures.keys().next().value;
    if (oldest !== undefined) recentSignatures.delete(oldest);
  }
  try {
    await recordDiagnosticReport({
      category: input.category,
      error_code: input.errorCode ?? null,
      http_status: input.httpStatus ?? null,
      source: input.source ?? "ui",
      message: input.message || "Unexpected error",
      stack: input.stack ?? null,
    });
  } catch {
    // Diagnostics capture must never throw or recurse
  }
}

export function installGlobalDiagnosticsCapture(): void {
  if (captureInstalled || typeof window === "undefined") {
    return;
  }
  captureInstalled = true;

  window.addEventListener("error", (event) => {
    const msg = event?.message || String(event?.error?.message || "Uncaught error");
    const stack =
      event?.error && typeof event.error.stack === "string"
        ? event.error.stack
        : event?.filename
          ? `${event.filename}:${event.lineno ?? 0}:${event.colno ?? 0}`
          : "";
    void recordDiagnosticError({
      category: "frontend_error",
      errorCode: event?.error?.name || "WINDOW_ERROR",
      source: event?.filename ? String(event.filename).split(/[\\/]/).pop() || "window" : "window",
      message: msg,
      stack,
    });
  });

  window.addEventListener("unhandledrejection", (event) => {
    const reason = event?.reason;
    const msg =
      reason instanceof Error
        ? reason.message
        : typeof reason === "string"
          ? reason
          : "Unhandled promise rejection";
    const stack = reason instanceof Error && typeof reason.stack === "string" ? reason.stack : "";
    void recordDiagnosticError({
      category: "unhandled_rejection",
      errorCode: reason instanceof Error ? reason.name : "UNHANDLED_REJECTION",
      source: "promise",
      message: msg,
      stack,
    });
  });
}
