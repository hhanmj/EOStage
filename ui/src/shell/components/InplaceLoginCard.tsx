import React, { useState } from "react";
import { KeyRound, X, CheckCircle2, Loader2, LogIn } from "lucide-react";
import type { TypedError, AccountId } from "../api/types.ts";
import {
  INPLACE_LOGIN_TITLE_KEY,
  INPLACE_LOGIN_SUCCESS_CONTINUE_KEY,
  INPLACE_LOGIN_LATER_BTN_KEY,
  INPLACE_LOGIN_SUBMIT_BTN_KEY,
  INPLACE_LOGIN_RETRYING_TEXT_KEY,
} from "../api/inplaceLoginText.ts";
import { useI18n } from "../../i18n/index.ts";
import { isFixtureBackend } from "../api";
import { useDelayedFlag } from "@/hooks/useLoadingPrimitives";

export interface InplaceLoginCardProps {
  error: TypedError;
  onClose: () => void;
  onSuccess?: () => Promise<void> | void;
  onOpenSettings?: (accountId?: AccountId) => void;
}

const ACCOUNT_DISPLAY_NAMES: Record<string, string> = {
  cdse: "Copernicus Data Space (CDSE)",
  earthdata: "NASA Earthdata (ASF DAAC)",
  bhoonidhi: "ISRO Bhoonidhi Portal",
  usgs_m2m: "USGS EarthExplorer / M2M",
  mpc: "Microsoft Planetary Computer (MPC)",
  opentopography: "OpenTopography",
};

export function InplaceLoginCard({
  error,
  onClose,
  onSuccess,
  onOpenSettings,
}: InplaceLoginCardProps) {
  const { t } = useI18n();
  const [loggingIn, setLoggingIn] = useState(false);
  const [loginSuccess, setLoginSuccess] = useState(false);
  const showLoginSpinner = useDelayedFlag(loggingIn);

  const accountId = error.needs_account || "earthdata";
  const accountName = ACCOUNT_DISPLAY_NAMES[accountId] || t("accounts.loginCard.fallbackAccount");
  const isFixture = isFixtureBackend();

  const handleSimulateOrDoLogin = async () => {
    // 生产环境未接入直接就地登录时，引导前往账号设置真实配置，绝不假装成功
    if (!isFixture) {
      if (onOpenSettings) {
        onClose();
        onOpenSettings(accountId as AccountId);
      }
      return;
    }

    setLoggingIn(true);
    try {
      // 开发演示模式下模拟登录流程（800ms 过渡反馈）
      await new Promise((resolve) => setTimeout(resolve, 800));
      setLoggingIn(false);
      setLoginSuccess(true);

      // 稍等 400ms 让用户看到成功态后自动重试原操作
      setTimeout(async () => {
        if (onSuccess) {
          await onSuccess();
        }
        onClose();
      }, 400);
    } catch (_err) {
      setLoggingIn(false);
    }
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={t(INPLACE_LOGIN_TITLE_KEY)}
      className="fixed inset-0 z-[2600] flex items-center justify-center bg-slate-950/45 backdrop-blur-[2px] p-4 transition-opacity duration-base ease-sp"
    >
      <div
        className="w-[440px] max-w-[92vw] overflow-hidden rounded-modal bg-sp-surface p-6 shadow-modal border border-sp-line"
        onClick={(e) => e.stopPropagation()}
      >
        {/* 卡片头部 */}
        <div className="flex items-center justify-between pb-4 border-b border-sp-divider">
          <div className="flex items-center gap-2">
            <div className="flex h-8 w-8 items-center justify-center rounded-sp-md bg-sp-selected text-sp-accent-strong">
              <KeyRound className="h-4 w-4" />
            </div>
            <div>
              <h4 className="text-sm font-bold text-sp-text">
                {t(INPLACE_LOGIN_TITLE_KEY)}
              </h4>
              <p className="text-xs text-sp-text-muted">
                {accountName}
              </p>
            </div>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label={t("accounts.loginCard.close")}
            className="flex h-7 w-7 items-center justify-center rounded-sp-sm text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text transition-colors duration-fast ease-sp cursor-pointer"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        {/* 错误说明与状态 */}
        <div className="py-4 space-y-3">
          <div className="text-xs leading-relaxed text-sp-text">
            {error.message_zh}
          </div>

          <div className="rounded-card bg-sp-surface-muted p-3 text-xs text-sp-text-secondary">
            {loginSuccess ? (
              <div className="flex items-center gap-2 text-sp-success-text">
                <CheckCircle2 className="h-4 w-4 shrink-0" />
                <span>{t(INPLACE_LOGIN_RETRYING_TEXT_KEY)}</span>
              </div>
            ) : (
              <div className="flex items-center gap-2 text-sp-text-muted">
                <div className="h-1.5 w-1.5 rounded-full bg-sp-accent-strong" />
                <span>{t(INPLACE_LOGIN_SUCCESS_CONTINUE_KEY)}</span>
              </div>
            )}
          </div>
        </div>

        {/* 底部按钮栏 */}
        <div className="flex items-center justify-end gap-2 pt-2 border-t border-sp-divider">
          <button
            type="button"
            onClick={onClose}
            disabled={loggingIn || loginSuccess}
            className="flex h-sp-control items-center px-3 rounded-sp-sm text-[13px] font-semibold text-sp-text-secondary hover:bg-sp-hover transition-colors duration-fast ease-sp cursor-pointer"
          >
            {t(INPLACE_LOGIN_LATER_BTN_KEY)}
          </button>

          {onOpenSettings && (
            <button
              type="button"
              onClick={() => {
                onClose();
                onOpenSettings(accountId as AccountId);
              }}
              disabled={loggingIn || loginSuccess}
              className="cred-action-btn transition-colors duration-fast ease-sp cursor-pointer"
            >
              {t("accounts.loginCard.openSettings")}
            </button>
          )}

          <button
            type="button"
            onClick={handleSimulateOrDoLogin}
            aria-busy={loggingIn}
            disabled={loggingIn || loginSuccess || (!isFixture && !onOpenSettings)}
            className="flex h-sp-control min-w-[104px] items-center justify-center gap-2 px-4 rounded-sp-sm text-[13px] font-semibold bg-sp-accent-fill text-white enabled:hover:opacity-90 shadow-card transition-colors duration-fast ease-sp disabled:opacity-50 cursor-pointer"
          >
            {showLoginSpinner ? (
              <Loader2 className="h-4 w-4 shrink-0 animate-spin" />
            ) : (
              <LogIn className="h-4 w-4 shrink-0" />
            )}
            <span>{isFixture ? t(INPLACE_LOGIN_SUBMIT_BTN_KEY) : t("accounts.loginCard.configure")}</span>
          </button>
        </div>
      </div>
    </div>
  );
}
