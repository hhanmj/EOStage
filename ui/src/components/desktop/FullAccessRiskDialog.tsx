/**
 * FullAccessRiskDialog.tsx
 * 完全访问模式一次性风险提示对话框（规范对齐 A1 §3.1 与 W5-2f 简报）
 */

import React, { useEffect, useRef, useState } from "react";
import { AlertTriangle, Download, Globe, ShieldCheck } from "lucide-react";
import { useI18n } from "@/i18n";

export interface FullAccessRiskDialogProps {
  open: boolean;
  onConfirm: () => void;
  onCancel: () => void;
  triggerRef?: React.RefObject<HTMLElement | null>;
}

export function FullAccessRiskDialog({
  open,
  onConfirm,
  onCancel,
  triggerRef,
}: FullAccessRiskDialogProps) {
  const { t } = useI18n();
  const cancelBtnRef = useRef<HTMLButtonElement | null>(null);
  const [acknowledged, setAcknowledged] = useState(false);

  useEffect(() => {
    if (open) setAcknowledged(false);
  }, [open]);

  // Esc 关闭与打开时焦点定位到取消按钮
  useEffect(() => {
    if (!open) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onCancel();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    // 聚焦到取消按钮
    const timer = setTimeout(() => {
      cancelBtnRef.current?.focus();
    }, 0);

    return () => {
      window.removeEventListener("keydown", handleKeyDown);
      clearTimeout(timer);
      triggerRef?.current?.focus();
    };
  }, [open, onCancel, triggerRef]);

  if (!open) return null;

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="full-access-risk-title"
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/20 p-4 animate-in fade-in duration-fast"
    >
      <div aria-hidden="true" className="absolute inset-0" onClick={onCancel} />
      <div
        className="relative w-full max-w-[470px] rounded-modal bg-sp-surface p-6 shadow-modal animate-in zoom-in-95 duration-fast"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-3">
          <AlertTriangle className="h-5 w-5 shrink-0 text-sp-warning-text" strokeWidth={1.8} />
          <h2
            id="full-access-risk-title"
            className="text-base font-semibold text-sp-text"
          >
            {t("agent.fullAccess.title")}
          </h2>
        </div>

        <p className="mt-3 text-sm leading-6 text-sp-text-secondary">
          {t("agent.fullAccess.description")}
        </p>

        <div className="mt-5 overflow-hidden rounded-card bg-sp-card px-4">
          <div className="flex gap-3 py-4">
            <Download className="mt-0.5 h-5 w-5 shrink-0 text-sp-accent" strokeWidth={1.8} />
            <div>
              <h3 className="text-sm font-semibold leading-5 text-sp-text">{t("agent.fullAccess.downloadsTitle")}</h3>
              <p className="mt-1 text-[13px] leading-5 text-sp-text-secondary">
                {t("agent.fullAccess.downloadsDescription")}
              </p>
            </div>
          </div>
          <div className="flex gap-3 border-t border-sp-divider py-4">
            <Globe className="mt-0.5 h-5 w-5 shrink-0 text-sp-accent" strokeWidth={1.8} />
            <div>
              <h3 className="text-sm font-semibold leading-5 text-sp-text">{t("agent.fullAccess.networkTitle")}</h3>
              <p className="mt-1 text-[13px] leading-5 text-sp-text-secondary">
                {t("agent.fullAccess.networkDescription")}
              </p>
            </div>
          </div>
          <div className="flex gap-3 border-t border-sp-divider py-4">
            <ShieldCheck className="mt-0.5 h-5 w-5 shrink-0 text-sp-accent" strokeWidth={1.8} />
            <div>
              <h3 className="text-sm font-semibold leading-5 text-sp-text">{t("agent.fullAccess.limitsTitle")}</h3>
              <p className="mt-1 text-[13px] leading-5 text-sp-text-secondary">
                {t("agent.fullAccess.limitsDescription")}
              </p>
            </div>
          </div>
        </div>

        <p className="mt-5 text-[13px] leading-5 text-sp-text-secondary">
          {t("agent.fullAccess.stopNote")}
        </p>

        <label className="mt-5 flex w-fit cursor-pointer items-center gap-2 text-sm text-sp-text">
          <input
            type="checkbox"
            checked={acknowledged}
            onChange={(e) => setAcknowledged(e.target.checked)}
            className="h-4 w-4 shrink-0 accent-sp-accent-strong cursor-pointer focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sp-accent"
          />
          <span>{t("agent.fullAccess.acknowledge")}</span>
        </label>

        <div className="mt-6 grid grid-cols-2 gap-3">
          <button
            ref={cancelBtnRef}
            type="button"
            onClick={onCancel}
            className="h-9 rounded-sp-sm bg-sp-card px-4 text-sm text-sp-text hover:bg-sp-hover transition-colors duration-fast ease-sp cursor-pointer outline-none focus-visible:ring-2 focus-visible:ring-sp-accent"
          >
            {t("agent.cancel")}
          </button>
          <button
            type="button"
            disabled={!acknowledged}
            onClick={onConfirm}
            className="h-9 rounded-sp-sm bg-sp-danger-strong px-4 text-sm font-medium text-white hover:opacity-90 transition-opacity duration-fast ease-sp cursor-pointer disabled:cursor-not-allowed disabled:opacity-40 outline-none focus-visible:ring-2 focus-visible:ring-sp-accent focus-visible:ring-offset-2 focus-visible:ring-offset-sp-surface"
          >
            {t("agent.fullAccess.confirm")}
          </button>
        </div>
      </div>
    </div>
  );
}
