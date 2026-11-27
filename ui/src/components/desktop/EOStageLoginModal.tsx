import React, { useCallback, useEffect, useRef, useState } from "react";
import { Check, Copy, ExternalLink, Info, Loader2, RefreshCw, X } from "lucide-react";
import { ModalOverlay } from "@/components/ui/ModalOverlay";
import { FieldValidationPopover } from "@/components/ui/FieldValidationPopover";
import { HoverHintLabel } from "@/components/desktop/HoverHintLabel";
import {
  eostageAccountLoginCompleteCode,
  eostageAccountLoginPoll,
  eostageAccountLoginStart,
  notifyEOStageAccountChanged,
  openEOStageBrowserUrl,
  type EOStageAccountStatus,
  type EOStageLoginStartResult,
} from "@/shell/api/eostageAccount";
import { notifyChannelsChanged } from "@/shell/api/agentChannelsEvents";
import { cn } from "@/lib/utils";

export interface EOStageLoginModalProps {
  open: boolean;
  onClose: () => void;
  onSuccess?: (account: EOStageAccountStatus) => void;
}

export function EOStageLoginModal({ open, onClose, onSuccess }: EOStageLoginModalProps) {
  const [phase, setPhase] = useState<"starting" | "waiting" | "done" | "error">("starting");
  const [loginSession, setLoginSession] = useState<EOStageLoginStartResult | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);
  const [copiedUrl, setCopiedUrl] = useState(false);
  const [showManualFallback, setShowManualFallback] = useState(false);
  const [manualCode, setManualCode] = useState("");
  const [manualError, setManualError] = useState<string | null>(null);
  const [submittingCode, setSubmittingCode] = useState(false);
  const [completedEmail, setCompletedEmail] = useState<string | null>(null);
  const [browserOpened, setBrowserOpened] = useState(false);
  const mountedRef = useRef(true);
  const completedRef = useRef(false);
  const manualInputRef = useRef<HTMLInputElement | null>(null);

  const finishSuccess = useCallback(
    (account: EOStageAccountStatus) => {
      if (completedRef.current || !mountedRef.current) return;
      completedRef.current = true;
      setCompletedEmail(account.email ?? null);
      setPhase("done");
      notifyEOStageAccountChanged();
      notifyChannelsChanged();
      onSuccess?.(account);
      window.setTimeout(() => {
        if (mountedRef.current) {
          onClose();
        }
      }, 650);
    },
    [onClose, onSuccess]
  );

  const startFlow = useCallback(async (openBrowserAutomatically: boolean) => {
    setPhase("starting");
    setStatusError(null);
    setCopiedUrl(false);
    setBrowserOpened(openBrowserAutomatically);
    completedRef.current = false;
    try {
      const started = await eostageAccountLoginStart();
      if (!mountedRef.current) return;
      setLoginSession(started);
      setPhase("waiting");
      if (openBrowserAutomatically) {
        void openEOStageBrowserUrl(started.authorize_url);
      }
    } catch (err) {
      if (!mountedRef.current) return;
      setPhase("error");
      setStatusError(err instanceof Error ? err.message : "启动登录流程失败");
    }
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  useEffect(() => {
    if (!open) {
      setLoginSession(null);
      setManualCode("");
      setManualError(null);
      setShowManualFallback(false);
      setCompletedEmail(null);
      setBrowserOpened(false);
      completedRef.current = false;
      return;
    }
    void startFlow(false);
  }, [open, startFlow]);

  useEffect(() => {
    if (!open || phase !== "waiting" || !loginSession?.session_id) {
      return;
    }
    let cancelled = false;
    const pollOnce = async () => {
      try {
        const res = await eostageAccountLoginPoll(loginSession.session_id);
        if (cancelled || !mountedRef.current) return;
        if (res.status === "authorized" && res.account) {
          finishSuccess(res.account);
        } else if (res.status === "expired" || res.status === "error") {
          setPhase("error");
          setStatusError(res.message || "浏览器登录未完成，请重试");
        }
      } catch {
        // Keep polling transiently unless expired
      }
    };
    const timer = window.setInterval(() => {
      void pollOnce();
    }, 1500);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [open, phase, loginSession?.session_id, finishSuccess]);

  useEffect(() => {
    if (!open) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, onClose]);

  if (!open) return null;

  const handleCopyUrl = async () => {
    if (!loginSession?.authorize_url) return;
    try {
      await navigator.clipboard.writeText(loginSession.authorize_url);
      setCopiedUrl(true);
      window.setTimeout(() => setCopiedUrl(false), 1800);
    } catch {
      // ignore clipboard restriction
    }
  };

  const handleSubmitManualCode = async () => {
    const trimmed = manualCode.trim();
    if (!trimmed) {
      setManualError("请粘贴网页端显示的一次性登录码");
      manualInputRef.current?.focus();
      return;
    }
    setSubmittingCode(true);
    setManualError(null);
    try {
      const account = await eostageAccountLoginCompleteCode(trimmed);
      if (!mountedRef.current) return;
      finishSuccess(account);
    } catch (err) {
      if (!mountedRef.current) return;
      setManualError(err instanceof Error ? err.message : "登录码无效或已过期");
      manualInputRef.current?.focus();
    } finally {
      if (mountedRef.current) {
        setSubmittingCode(false);
      }
    }
  };

  return (
    <ModalOverlay
      role="dialog"
      aria-modal={true}
      aria-label="登录 EOStage 账号"
      data-testid="eostage-login-modal"
      closeOnBackdropClick={false}
    >
      <div
        className="w-full max-w-md rounded-modal border border-sp-line bg-sp-card p-6 shadow-modal space-y-5"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between gap-3">
          <div className="flex items-center gap-1.5 min-w-0">
            <h2 className="text-base font-semibold text-sp-text">登录 EOStage 账号</h2>
            <HoverHintLabel
              hint="本地检索下载与自带 API Key 无需登录，浏览器完成登录后将自动返回。"
              focusable
              align="start"
              placement="bottom"
              maxWidthClass="max-w-[260px]"
            >
              <Info className="h-3.5 w-3.5 shrink-0 text-sp-text-muted" aria-label="登录说明" />
            </HoverHintLabel>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="关闭登录窗口"
            className="flex h-7 w-7 shrink-0 items-center justify-center rounded-sp-sm text-[13px] text-sp-text-muted hover:bg-sp-hover hover:text-sp-text cursor-pointer"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="rounded-sp-md border border-sp-line bg-sp-surface p-4 space-y-3">
          {phase === "starting" && (
            <div className="flex items-center gap-2 text-[13px] text-sp-text">
              <Loader2 className="h-4 w-4 animate-spin shrink-0 text-sp-text-secondary" />
              <span>正在准备安全登录会话…</span>
            </div>
          )}

          {phase === "waiting" && (
            <>
              <div className="flex items-center gap-2 text-[13px] text-sp-text">
                {browserOpened ? (
                  <Loader2 className="h-4 w-4 animate-spin shrink-0 text-sp-text-secondary" />
                ) : (
                  <Check className="h-4 w-4 shrink-0 text-sp-text-secondary" />
                )}
                <span data-testid="eostage-login-status-text">
                  {browserOpened ? "等待浏览器完成登录…" : "登录链接已就绪"}
                </span>
              </div>
              <div className="flex flex-wrap items-center gap-2 pt-1">
                <button
                  type="button"
                  data-testid="eostage-login-reopen-btn"
                  disabled={!loginSession?.authorize_url}
                  onClick={() => {
                    if (!loginSession?.authorize_url) return;
                    setBrowserOpened(true);
                    void openEOStageBrowserUrl(loginSession.authorize_url);
                  }}
                  className="eo-settings-btn-primary"
                >
                  <ExternalLink className="h-3.5 w-3.5" />
                  <span>打开浏览器登录</span>
                </button>
                <button
                  type="button"
                  data-testid="eostage-login-copy-url-btn"
                  disabled={!loginSession?.authorize_url}
                  onClick={() => void handleCopyUrl()}
                  className="eo-settings-btn-secondary"
                >
                  {copiedUrl ? <Check className="h-3.5 w-3.5" /> : <Copy className="h-3.5 w-3.5" />}
                  <span>{copiedUrl ? "已复制链接" : "复制登录链接"}</span>
                </button>
              </div>
            </>
          )}

          {phase === "done" && (
            <div className="flex items-center gap-2 text-[13px] font-semibold text-sp-text">
              <Check className="h-4 w-4 shrink-0" />
              <span>{completedEmail ? `已登录 ${completedEmail}` : "登录成功，正在同步账号状态…"}</span>
            </div>
          )}

          {phase === "error" && (
            <div className="space-y-3">
              <p className="text-xs text-sp-danger-text leading-relaxed" role="alert">
                {statusError || "登录未能完成，请重试"}
              </p>
              <button
                type="button"
                data-testid="eostage-login-retry-btn"
                onClick={() => void startFlow(false)}
                className="eo-settings-btn-secondary"
              >
                <RefreshCw className="h-3.5 w-3.5" />
                <span>重新发起登录</span>
              </button>
            </div>
          )}
        </div>

        <div className="border-t border-sp-divider pt-3 space-y-2">
          <button
            type="button"
            data-testid="eostage-login-toggle-fallback-btn"
            onClick={() => {
              setShowManualFallback((prev) => !prev);
              setManualError(null);
            }}
            className="text-[13px] text-sp-text-secondary hover:text-sp-text cursor-pointer"
          >
            {showManualFallback ? "收起手动登录码入口" : "浏览器未自动跳回？粘贴登录码"}
          </button>

          {showManualFallback && (
            <div className="space-y-2 pt-1">
              <label htmlFor="eostage-manual-ticket" className="block text-[13px] text-sp-text-secondary">
                一次性登录码
              </label>
              <div className="flex items-center gap-2">
                <input
                  ref={manualInputRef}
                  id="eostage-manual-ticket"
                  data-testid="eostage-login-fallback-input"
                  type="text"
                  value={manualCode}
                  disabled={submittingCode}
                  aria-invalid={manualError ? "true" : "false"}
                  aria-describedby={manualError ? "eostage-login-fallback-error" : undefined}
                  placeholder="粘贴网页端显示的 eos_tkt_... 登录码"
                  onChange={(e) => {
                    setManualCode(e.target.value);
                    if (manualError) setManualError(null);
                  }}
                  onKeyDown={(e) => {
                    if (
                      e.nativeEvent.isComposing ||
                      (e.nativeEvent as unknown as { keyCode?: number }).keyCode === 229
                    ) {
                      return;
                    }
                    if (e.key === "Enter") {
                      e.preventDefault();
                      void handleSubmitManualCode();
                    }
                  }}
                  className={cn(
                    "eo-settings-control-input h-9 flex-1 font-mono",
                    manualError && "border-sp-text"
                  )}
                />
                <button
                  type="button"
                  data-testid="eostage-login-fallback-submit-btn"
                  disabled={submittingCode}
                  onClick={() => void handleSubmitManualCode()}
                  className="eo-settings-btn-primary shrink-0"
                >
                  {submittingCode ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : null}
                  <span>确认登录</span>
                </button>
              </div>
              <FieldValidationPopover
                id="eostage-login-fallback-error"
                testId="eostage-login-fallback-error"
                message={manualError}
                anchorRef={manualInputRef}
                onDismiss={() => setManualError(null)}
              />
            </div>
          )}
        </div>
      </div>
    </ModalOverlay>
  );
}
