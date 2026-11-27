import React, { useEffect, useState } from "react";
import {
  ArrowLeft,
  ChevronDown,
  ChevronUp,
  ExternalLink,
  Loader2,
  MessageSquare,
  QrCode,
  RefreshCw,
  RotateCcw,
  User,
  X,
} from "lucide-react";
import { ModalOverlay } from "@/components/ui/ModalOverlay";
import { useI18n } from "@/i18n";
import { checkAppUpdate, getAppInfo, openExternalUrl } from "@/lib/bridge";
import { modShortcut } from "@/lib/platformShortcuts";

export interface AboutEOStageModalProps {
  open: boolean;
  onClose: () => void;
  onReplayOnboarding?: () => void;
  onReplayWelcome?: () => void;
}

type LegalDocView = "about" | "terms" | "privacy" | "contact";

interface UpdateCheckState {
  status: "idle" | "checking" | "up_to_date" | "update_available" | "no_release" | "offline_error";
  message?: string;
  latestVersion?: string | null;
  releaseUrl?: string | null;
}

function GitHubCatOutlineIcon({ className }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.75"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      className={className}
    >
      <path d="M15 22v-4a4.8 4.8 0 0 0-1-3.5c3 0 6-2 6-5.5.08-1.25-.27-2.48-1-3.5.28-1.15.28-2.35 0-3.5 0 0-1 0-3 1.5-2.64-.5-5.36-.5-8 0C6 2 5 2 5 2c-.3 1.15-.3 2.35 0 3.5A5.403 5.403 0 0 0 4 9c0 3.5 3 5.5 6 5.5-.39.49-.68 1.05-.85 1.65-.17.6-.22 1.23-.15 1.85v4" />
      <path d="M9 18c-4.51 2-5-2-7-2" />
    </svg>
  );
}

export function AboutEOStageModal({
  open,
  onClose,
  onReplayOnboarding,
  onReplayWelcome,
}: AboutEOStageModalProps) {
  const { t, locale } = useI18n();
  const [appVersion, setAppVersion] = useState("2.2.0-beta.0");
  const [docView, setDocView] = useState<LegalDocView>("about");
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  const [updateState, setUpdateState] = useState<UpdateCheckState>({ status: "idle" });
  const [isDark, setIsDark] = useState(false);

  useEffect(() => {
    if (!open || typeof document === "undefined") return;
    const syncTheme = () => {
      const root = document.documentElement;
      setIsDark(
        root.classList.contains("dark") || root.getAttribute("data-theme") === "dark"
      );
    };
    syncTheme();
    const observer = new MutationObserver(syncTheme);
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class", "data-theme"],
    });
    return () => observer.disconnect();
  }, [open]);

  useEffect(() => {
    if (!open) return;
    setDocView("about");
    setUpdateState({ status: "idle" });
    let cancelled = false;
    void getAppInfo()
      .then((info) => {
        if (!cancelled && info?.version) {
          setAppVersion(info.version);
        }
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [open]);

  useEffect(() => {
    if (!open || typeof window === "undefined") return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, onClose]);

  if (!open) return null;

  const handleCheckUpdate = async () => {
    if (updateState.status === "checking") return;
    setUpdateState({ status: "checking" });
    try {
      const res = await checkAppUpdate();
      if (!res) {
        setUpdateState({
          status: "offline_error",
          message: t("shell.about.updateUnsupported"),
        });
        return;
      }
      const nextStatus = (res.status as UpdateCheckState["status"]) || "up_to_date";
      const localizedFallback =
        nextStatus === "update_available"
          ? res.latest_version
            ? t("shell.about.updateAvailable", { version: res.latest_version })
            : t("shell.about.updateAvailableGeneric")
          : nextStatus === "no_release"
            ? t("shell.about.updateNoRelease")
            : nextStatus === "offline_error"
              ? t("shell.about.updateCheckFailed")
              : t("shell.about.updateUpToDate");
      setUpdateState({
        status: nextStatus,
        message: locale === "zh-CN" && res.message_zh ? res.message_zh : localizedFallback,
        latestVersion: res.latest_version,
        releaseUrl: res.release_url,
      });
    } catch {
      setUpdateState({
        status: "offline_error",
        message: t("shell.about.updateCheckFailed"),
      });
    }
  };

  return (
    <ModalOverlay
      closeOnBackdropClick={true}
      onClose={onClose}
      role="dialog"
      aria-modal={true}
      aria-label={t("shell.about.modalAria")}
      data-testid="about-eostage-modal"
      className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/35"
    >
      <div
        className="flex max-h-[86vh] w-full max-w-lg flex-col rounded-card border border-sp-line bg-sp-card shadow-modal overflow-hidden text-sp-text"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Top bar */}
        <div className="flex items-center justify-between border-b border-sp-divider px-5 py-3.5">
          <div className="flex items-center gap-2 min-w-0">
            {docView !== "about" && (
              <button
                type="button"
                data-testid="about-back-btn"
                onClick={() => setDocView("about")}
                className="flex h-7 items-center gap-1 rounded-sp-sm px-2 text-xs text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text cursor-pointer"
              >
                <ArrowLeft className="h-3.5 w-3.5" />
                <span>{t("shell.about.back")}</span>
              </button>
            )}
            <h2 className="text-[14px] font-semibold truncate">
              {docView === "terms"
                ? t("shell.about.titleTerms")
                : docView === "privacy"
                  ? t("shell.about.titlePrivacy")
                  : docView === "contact"
                    ? t("shell.about.titleContact")
                    : t("shell.about.titleAbout")}
            </h2>
          </div>
          <button
            type="button"
            aria-label={t("shell.about.closeAria")}
            onClick={onClose}
            className="flex h-7 w-7 items-center justify-center rounded-sp-sm text-sp-text-muted hover:bg-sp-hover hover:text-sp-text cursor-pointer"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        {/* Main body */}
        <div className="flex-1 overflow-y-auto px-5 py-4 space-y-4">
          {docView === "terms" ? (
            <div className="space-y-3 text-xs leading-relaxed text-sp-text-secondary" data-testid="about-terms-view">
              <p className="text-sp-text font-semibold">{t("shell.about.termsHeading1")}</p>
              <p>{t("shell.about.termsBody1")}</p>
              <p className="text-sp-text font-semibold">{t("shell.about.termsHeading2")}</p>
              <p>{t("shell.about.termsBody2")}</p>
              <p className="text-sp-text font-semibold">{t("shell.about.termsHeading3")}</p>
              <p>{t("shell.about.termsBody3")}</p>
              <p>{t("shell.about.termsBody4")}</p>
            </div>
          ) : docView === "privacy" ? (
            <div className="space-y-3 text-xs leading-relaxed text-sp-text-secondary" data-testid="about-privacy-view">
              <p className="text-sp-text font-semibold">{t("shell.about.privacyHeading1")}</p>
              <p>{t("shell.about.privacyBody1")}</p>
              <p className="text-sp-text font-semibold">{t("shell.about.privacyHeading2")}</p>
              <p>{t("shell.about.privacyBody2")}</p>
              <p className="text-sp-text font-semibold">{t("shell.about.privacyHeading3")}</p>
              <p>{t("shell.about.privacyBody3")}</p>
            </div>
          ) : docView === "contact" ? (
            <div
              className="flex flex-col items-center justify-center gap-3 py-4 text-center"
              data-testid="about-contact-view"
            >
              <div className="rounded-sp-md border border-sp-line bg-white p-2 shadow-sm">
                <img
                  src="/contact/wechat_story.jpg"
                  alt={t("shell.about.wechatQrAlt")}
                  className="h-auto w-60 max-w-full rounded-sp-sm object-contain"
                />
              </div>
              <p className="text-xs text-sp-text-secondary">{t("shell.about.wechatScanHint")}</p>
            </div>
          ) : (
            <>
              {/* Identity + explicit update check */}
              <div className="flex items-center justify-between gap-4 rounded-sp-md border border-sp-line bg-sp-surface p-4">
                <div className="flex items-center gap-3 min-w-0">
                  <img
                    src={isDark ? "/app-logo.svg#dark" : "/app-logo.png"}
                    alt="EOStage"
                    data-testid="about-real-app-icon"
                    className="h-9 w-auto max-w-[200px] object-contain shrink-0"
                  />
                  <span
                    data-testid="about-app-version"
                    className="eo-settings-badge font-mono tabular-nums shrink-0"
                  >
                    v{appVersion}
                  </span>
                </div>
                <button
                  type="button"
                  data-testid="about-check-update-btn"
                  disabled={updateState.status === "checking"}
                  onClick={() => void handleCheckUpdate()}
                  className="eo-settings-btn-secondary shrink-0 disabled:opacity-50"
                >
                  {updateState.status === "checking" ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <RefreshCw className="h-3.5 w-3.5" />
                  )}
                  <span>{t("shell.about.checkUpdate")}</span>
                </button>
              </div>

              {updateState.status !== "idle" && updateState.message && (
                <div
                  role="status"
                  data-testid="about-update-status"
                  className="flex flex-wrap items-center justify-between gap-2 rounded-sp-sm border border-sp-line bg-sp-surface px-3 py-2 text-xs text-sp-text-secondary"
                >
                  <span>{updateState.message}</span>
                  {updateState.releaseUrl &&
                    (updateState.status === "update_available" ||
                      updateState.status === "no_release" ||
                      updateState.status === "offline_error") && (
                      <button
                        type="button"
                        onClick={() => void openExternalUrl(updateState.releaseUrl!)}
                        className="inline-flex items-center gap-1 font-medium text-sp-text hover:underline cursor-pointer"
                      >
                        <span>{t("shell.about.openReleasePage")}</span>
                        <ExternalLink className="h-3 w-3" />
                      </button>
                    )}
                </div>
              )}

              {/* 4 quick tiles: Author, Open-Source Repo, Feedback, Contact Author */}
              <div className="grid grid-cols-2 gap-3" data-testid="about-action-grid">
                <button
                  type="button"
                  data-testid="about-author-btn"
                  className="flex flex-col items-center justify-center gap-1.5 rounded-sp-md border border-sp-line bg-sp-surface px-4 py-4 text-center"
                >
                  <User className="h-5 w-5 text-sp-text" strokeWidth={1.75} />
                  <span className="text-[13px] font-semibold text-sp-text">{t("shell.about.authorTitle")}</span>
                  <span className="text-xs text-sp-text-muted">{t("shell.about.authorSubtitle")}</span>
                </button>

                <button
                  type="button"
                  data-testid="about-github-btn"
                  className="flex flex-col items-center justify-center gap-1.5 rounded-sp-md border border-sp-line bg-sp-surface px-4 py-4 text-center"
                >
                  <GitHubCatOutlineIcon className="h-5 w-5 text-sp-text" />
                  <span className="text-[13px] font-semibold text-sp-text">{t("shell.about.githubTitle")}</span>
                  <span className="text-xs text-sp-text-muted">{t("shell.about.githubSubtitle")}</span>
                </button>

                <button
                  type="button"
                  data-testid="about-open-feedback-btn"
                  className="flex flex-col items-center justify-center gap-1.5 rounded-sp-md border border-sp-line bg-sp-surface px-4 py-4 text-center"
                >
                  <MessageSquare className="h-5 w-5 text-sp-text" strokeWidth={1.75} />
                  <span className="text-[13px] font-semibold text-sp-text">{t("shell.about.feedbackTitle")}</span>
                  <span className="text-xs text-sp-text-muted">{t("shell.about.feedbackSubtitle")}</span>
                </button>

                <button
                  type="button"
                  data-testid="about-contact-btn"
                  onClick={() => setDocView("contact")}
                  className="flex flex-col items-center justify-center gap-1.5 rounded-sp-md border border-sp-line bg-sp-surface px-4 py-4 text-center transition-colors hover:bg-sp-hover cursor-pointer"
                >
                  <QrCode className="h-5 w-5 text-sp-text" strokeWidth={1.75} />
                  <span className="text-[13px] font-semibold text-sp-text">{t("shell.about.contactTitle")}</span>
                  <span className="text-xs text-sp-text-muted">{t("shell.about.contactSubtitle")}</span>
                </button>
              </div>

              {/* Quick utilities: Onboarding replay + Shortcuts reference */}
              <div className="rounded-sp-md border border-sp-line bg-sp-surface divide-y divide-sp-divider">
                {onReplayOnboarding && (
                  <div className="flex items-center justify-between gap-3 px-3.5 py-2.5">
                    <div className="flex items-center gap-2 text-xs text-sp-text">
                      <RotateCcw className="h-3.5 w-3.5 text-sp-text-secondary" />
                      <span>{t("shell.about.guidedTourLabel")}</span>
                    </div>
                    <div className="flex items-center gap-2">
                      {onReplayWelcome && (
                        <button
                          type="button"
                          data-testid="rail-replay-welcome-btn"
                          onClick={() => {
                            onClose();
                            onReplayWelcome();
                          }}
                          className="eo-settings-btn-secondary h-7 px-2.5 text-xs whitespace-nowrap"
                        >
                          <span>{t("shell.about.welcomePageBtn")}</span>
                        </button>
                      )}
                      <button
                        type="button"
                        data-testid="rail-replay-onboarding-btn"
                        onClick={() => {
                          onClose();
                          onReplayOnboarding();
                        }}
                        className="eo-settings-btn-secondary h-7 px-2.5 text-xs whitespace-nowrap"
                      >
                        <span>{t("shell.about.openBtn")}</span>
                      </button>
                    </div>
                  </div>
                )}

                <div>
                  <button
                    type="button"
                    data-testid="about-toggle-shortcuts-btn"
                    aria-expanded={shortcutsOpen}
                    onClick={() => setShortcutsOpen((prev) => !prev)}
                    className="flex w-full items-center justify-between px-3.5 py-2.5 text-xs font-medium text-sp-text hover:bg-sp-hover cursor-pointer"
                  >
                    <span>{t("shell.about.shortcutsTitle")}</span>
                    {shortcutsOpen ? (
                      <ChevronUp className="h-3.5 w-3.5 text-sp-text-muted" />
                    ) : (
                      <ChevronDown className="h-3.5 w-3.5 text-sp-text-muted" />
                    )}
                  </button>
                  {shortcutsOpen && (
                    <div
                      data-testid="about-shortcuts-list"
                      className="border-t border-sp-divider px-3.5 py-2.5 space-y-1.5 text-xs text-sp-text-secondary"
                    >
                      {[
                        [t("shell.about.shortcutNewChat"), modShortcut("N")],
                        [t("shell.about.shortcutSwitchSession"), "Alt + ↑ / ↓"],
                        [t("shell.about.shortcutHistoryNav"), "Alt + ← / →"],
                        [t("shell.about.shortcutToggleSidebar"), modShortcut("B")],
                        [t("shell.about.shortcutCommandPalette"), modShortcut("K")],
                        [t("shell.about.shortcutSwitchView"), modShortcut("1..4")],
                        [t("shell.about.shortcutPreferences"), modShortcut(",")],
                        [t("shell.about.shortcutZoom"), `${modShortcut("+")} / - / 0`],
                      ].map(([label, keys]) => (
                        <div key={label} className="flex items-center justify-between gap-3">
                          <span className="truncate">{label}</span>
                          <span className="font-mono text-xs text-sp-text-muted whitespace-nowrap shrink-0">
                            {keys}
                          </span>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              </div>
            </>
          )}
        </div>

        {/* Footer links */}
        <div className="flex flex-wrap items-center justify-between gap-2 border-t border-sp-divider bg-sp-surface px-5 py-3 text-xs text-sp-text-secondary">
          <div className="flex items-center gap-4">
            <button
              type="button"
              data-testid="about-open-terms-btn"
              onClick={() => setDocView("terms")}
              className="hover:text-sp-text hover:underline cursor-pointer"
            >
              {t("shell.about.termsLink")}
            </button>
            <button
              type="button"
              data-testid="about-open-privacy-btn"
              onClick={() => setDocView("privacy")}
              className="hover:text-sp-text hover:underline cursor-pointer"
            >
              {t("shell.about.privacyLink")}
            </button>
          </div>
          <span className="text-xs text-sp-text-muted">EOStage Workbench</span>
        </div>
      </div>
    </ModalOverlay>
  );
}
