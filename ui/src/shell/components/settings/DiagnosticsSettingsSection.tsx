import React, { useCallback, useEffect, useState } from "react";
import { FileText, Loader2, Trash2, Upload, X } from "lucide-react";
import { useI18n } from "@/i18n";
import { ModalOverlay } from "@/components/ui/ModalOverlay";
import {
  clearDiagnosticReports,
  getDiagnosticsState,
  listDiagnosticReports,
  setDiagnosticsEnabled,
  uploadDiagnosticReports,
  type DiagnosticsState,
  type SanitizedDiagnosticRecord,
} from "@/shell/api/diagnostics";

export function DiagnosticsSettingsSection() {
  const { t } = useI18n();
  const [state, setState] = useState<DiagnosticsState>({
    enabled: false,
    total_count: 0,
    pending_count: 0,
  });
  const [savingSwitch, setSavingSwitch] = useState(false);
  const [uploading, setUploading] = useState(false);
  const [clearing, setClearing] = useState(false);
  const [statusMessage, setStatusMessage] = useState<string | null>(null);
  const [viewerOpen, setViewerOpen] = useState(false);
  const [records, setRecords] = useState<SanitizedDiagnosticRecord[]>([]);
  const [loadingRecords, setLoadingRecords] = useState(false);

  const refreshState = useCallback(async () => {
    try {
      const next = await getDiagnosticsState();
      setState(next);
    } catch {
      // Keep default safe state when running without desktop runtime
    }
  }, []);

  useEffect(() => {
    void refreshState();
  }, [refreshState]);

  useEffect(() => {
    if (!viewerOpen) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopImmediatePropagation();
        setViewerOpen(false);
      }
    };
    window.addEventListener("keydown", handleKeyDown, true);
    return () => window.removeEventListener("keydown", handleKeyDown, true);
  }, [viewerOpen]);

  const handleToggle = async () => {
    if (savingSwitch) return;
    const prev = state.enabled;
    const nextVal = !prev;
    setState((cur) => ({ ...cur, enabled: nextVal }));
    setSavingSwitch(true);
    setStatusMessage(null);
    try {
      const updated = await setDiagnosticsEnabled(nextVal);
      setState(updated);
      setStatusMessage(nextVal ? t("settings.diagnostics.enabledStatus") : t("settings.diagnostics.disabledStatus"));
    } catch {
      setState((cur) => ({ ...cur, enabled: prev }));
      setStatusMessage(t("settings.diagnostics.saveFailedStatus"));
    } finally {
      setSavingSwitch(false);
    }
  };

  const handleOpenViewer = async () => {
    setViewerOpen(true);
    setLoadingRecords(true);
    try {
      const items = await listDiagnosticReports();
      setRecords(items);
    } catch {
      setRecords([]);
    } finally {
      setLoadingRecords(false);
    }
  };

  const handleUploadNow = async () => {
    if (uploading || state.pending_count === 0) return;
    setUploading(true);
    setStatusMessage(null);
    try {
      const res = await uploadDiagnosticReports();
      setStatusMessage(res.message_zh);
      await refreshState();
      if (viewerOpen) {
        setRecords(await listDiagnosticReports());
      }
    } catch {
      setStatusMessage(t("settings.diagnostics.uploadFailedStatus"));
    } finally {
      setUploading(false);
    }
  };

  const handleClear = async () => {
    if (clearing || state.total_count === 0) return;
    setClearing(true);
    setStatusMessage(null);
    try {
      const next = await clearDiagnosticReports();
      setState(next);
      setRecords([]);
      setStatusMessage(t("settings.diagnostics.clearedStatus"));
    } catch {
      setStatusMessage(t("settings.diagnostics.clearFailedStatus"));
    } finally {
      setClearing(false);
    }
  };

  return (
    <div className="eo-settings-section" data-testid="diagnostics-settings-section">
      <h3 className="text-[14px] font-semibold text-sp-text">{t("settings.diagnostics.title")}</h3>

      <div className="eo-settings-card">
        <div className="eo-settings-card-content divide-y divide-sp-divider">
          <div className="eo-settings-card-row">
            <div className="flex items-center justify-between gap-4">
              <div className="flex min-w-0 flex-1 flex-col gap-1">
                <h4 className="text-[13px] font-normal text-sp-text">{t("settings.diagnostics.switchTitle")}</h4>
                <div className="text-[12.5px] text-sp-text-secondary">
                  {t("settings.diagnostics.switchDescription")}
                </div>
              </div>
              <button
                type="button"
                role="switch"
                data-testid="diagnostics-upload-switch"
                aria-checked={state.enabled}
                aria-label={t("settings.diagnostics.switchTitle")}
                disabled={savingSwitch}
                onClick={() => void handleToggle()}
                className="eo-settings-switch shrink-0 cursor-pointer disabled:opacity-50"
              >
                <span className="eo-settings-switch-thumb" />
              </button>
            </div>
          </div>

          <div className="eo-settings-card-row">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <div className="text-[12.5px] text-sp-text-secondary font-mono tabular-nums" data-testid="diagnostics-local-count">
                {t("settings.diagnostics.localCount", { total: state.total_count, pending: state.pending_count })}
              </div>
              <div className="flex flex-wrap items-center gap-2">
                <button
                  type="button"
                  data-testid="diagnostics-view-records-btn"
                  onClick={() => void handleOpenViewer()}
                  className="eo-settings-btn-secondary"
                >
                  <FileText className="h-3.5 w-3.5" />
                  <span>{t("settings.diagnostics.viewRecords")}</span>
                </button>
                <button
                  type="button"
                  data-testid="diagnostics-upload-now-btn"
                  disabled={uploading || state.pending_count === 0}
                  onClick={() => void handleUploadNow()}
                  className="eo-settings-btn-secondary disabled:opacity-45"
                >
                  {uploading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Upload className="h-3.5 w-3.5" />}
                  <span>{t("settings.diagnostics.uploadNow")}</span>
                </button>
                <button
                  type="button"
                  data-testid="diagnostics-clear-btn"
                  disabled={clearing || state.total_count === 0}
                  onClick={() => void handleClear()}
                  className="eo-settings-btn-secondary disabled:opacity-45"
                >
                  <Trash2 className="h-3.5 w-3.5" />
                  <span>{t("settings.diagnostics.clearRecords")}</span>
                </button>
              </div>
            </div>
            {statusMessage && (
              <p className="mt-2 text-xs text-sp-text-secondary" role="status" data-testid="diagnostics-status-msg">
                {statusMessage}
              </p>
            )}
          </div>
        </div>
      </div>

      {viewerOpen && (
        <ModalOverlay
          onClose={() => setViewerOpen(false)}
          role="dialog"
          aria-modal={true}
          aria-label={t("settings.diagnostics.viewerTitle")}
          data-testid="diagnostics-viewer-modal"
          className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/35"
        >
          <div
            className="flex max-h-[80vh] w-full max-w-2xl flex-col rounded-card border border-sp-line bg-sp-card shadow-modal overflow-hidden"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-center justify-between border-b border-sp-divider px-5 py-3.5">
              <div>
                <h3 className="text-[14px] font-semibold text-sp-text">{t("settings.diagnostics.viewerTitle")}</h3>
                <p className="text-xs text-sp-text-muted">{t("settings.diagnostics.viewerSubtitle")}</p>
              </div>
              <button
                type="button"
                aria-label={t("settings.diagnostics.viewerClose")}
                onClick={() => setViewerOpen(false)}
                className="flex h-7 w-7 items-center justify-center rounded-sp-sm text-sp-text-muted hover:bg-sp-hover hover:text-sp-text cursor-pointer"
              >
                <X className="h-4 w-4" />
              </button>
            </div>
            <div className="flex-1 overflow-y-auto p-5 space-y-3">
              {loadingRecords ? (
                <div className="py-8 text-center text-xs text-sp-text-muted">{t("settings.diagnostics.viewerLoading")}</div>
              ) : records.length === 0 ? (
                <div className="py-8 text-center text-xs text-sp-text-muted">{t("settings.diagnostics.viewerEmpty")}</div>
              ) : (
                records.map((rec) => (
                  <div
                    key={rec.report_id}
                    className="rounded-sp-md border border-sp-line bg-sp-surface p-3 space-y-1.5"
                  >
                    <div className="flex items-center justify-between gap-2 text-xs">
                      <span className="font-mono font-semibold text-sp-text">
                        [{rec.category}] {rec.error_code}
                      </span>
                      <span className="font-mono text-sp-text-muted">
                        {rec.uploaded ? t("settings.diagnostics.uploadedBadge") : t("settings.diagnostics.pendingBadge")} · {rec.created_at}
                      </span>
                    </div>
                    <pre className="max-h-40 overflow-auto whitespace-pre-wrap break-all rounded-sp-sm bg-sp-canvas p-2 text-xs font-mono text-sp-text-secondary">
                      {JSON.stringify(rec, null, 2)}
                    </pre>
                  </div>
                ))
              )}
            </div>
          </div>
        </ModalOverlay>
      )}
    </div>
  );
}
