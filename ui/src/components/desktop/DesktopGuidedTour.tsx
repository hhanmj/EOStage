import React, {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import {
  ChevronLeft,
  ChevronRight,
  Compass,
  Cpu,
  Database,
  Download,
  KeyRound,
  MapPin,
  MessageSquare,
  Radar,
  X,
} from "lucide-react";
import { useI18n } from "@/i18n";
import { cn } from "@/lib/utils";
import type { SourceDescriptor } from "@/shell/api/types";
import { buildCatalogItems, computeCategoryCounts } from "@/lib/dataSourceCatalogLogic";
import type { ShellRoute } from "@/shell/api/shellRoutes";
import type { SettingsCategory } from "../SettingsModal";

export const GUIDED_TOUR_STORAGE_KEY = "eostage.onboarding.guidedTourCompleted.v1";

export type GuidedTourPlacement = "top" | "right" | "bottom" | "left" | "center";

export interface GuidedTourCatalogSummary {
  primary: number;
  gee: number;
  total: number;
  freePrimary: number;
}

/**
 * Derive live Data Catalog counts directly from the catalog registry / runtime sources
 * so guided tour badges always match the sidebar counts shown in DataSourceCatalogModal.
 */
export function computeGuidedTourCatalogSummary(
  availableSources: readonly SourceDescriptor[] = []
): GuidedTourCatalogSummary {
  const items = buildCatalogItems([...availableSources], {});
  const counts = computeCategoryCounts(items, []);
  const primary = counts["primary-all"] ?? 0;
  const gee = counts["gee-catalog"] ?? 0;
  const freePrimary = items.filter(
    (item) =>
      item.subcategories.includes("primary-all") && item.accountRequirement === "free"
  ).length;
  return {
    primary,
    gee,
    total: primary + gee,
    freePrimary,
  };
}

export interface GuidedTourStepConfig {
  id:
    | "agent_composer"
    | "aoi_binding"
    | "map_workbench"
    | "data_catalog"
    | "download_center"
    | "model_settings"
    | "data_accounts";
  i18nKey:
    | "agentComposer"
    | "aoiBinding"
    | "mapWorkbench"
    | "dataCatalog"
    | "downloadCenter"
    | "modelSettings"
    | "dataAccounts";
  route: ShellRoute;
  settingsCategory?: SettingsCategory;
  openAdminModal?: boolean;
  openCatalog?: boolean;
  expandSearchPanel?: boolean;
  openAddProviderCatalog?: boolean;
  selector: string;
  preferredPlacement: GuidedTourPlacement;
  icon: "message" | "mapPin" | "radar" | "database" | "download" | "cpu" | "key";
}

export const DESKTOP_GUIDED_TOUR_STEPS: readonly GuidedTourStepConfig[] = [
  {
    id: "agent_composer",
    i18nKey: "agentComposer",
    route: "agent",
    selector:
      "[data-testid='agent-home-composer-shell'], [data-testid='agent-active-composer-shell']",
    preferredPlacement: "bottom",
    icon: "message",
  },
  {
    id: "aoi_binding",
    i18nKey: "aoiBinding",
    route: "agent",
    openAdminModal: true,
    selector: "[data-testid='admin-boundary-modal'], [data-testid='titlebar-aoi-pill']",
    preferredPlacement: "right",
    icon: "mapPin",
  },
  {
    id: "map_workbench",
    i18nKey: "mapWorkbench",
    route: "workbench",
    expandSearchPanel: true,
    selector: "[data-column='search'], [data-testid='rail-nav-workbench']",
    preferredPlacement: "right",
    icon: "radar",
  },
  {
    id: "data_catalog",
    i18nKey: "dataCatalog",
    route: "workbench",
    openCatalog: true,
    selector: "[data-testid='catalog-sidebar-nav'], [data-testid='data-source-catalog-modal']",
    preferredPlacement: "right",
    icon: "database",
  },
  {
    id: "download_center",
    i18nKey: "downloadCenter",
    route: "downloads",
    selector: "[data-testid='download-center-tour-anchor'], [data-testid='rail-nav-downloads']",
    preferredPlacement: "bottom",
    icon: "download",
  },
  {
    id: "model_settings",
    i18nKey: "modelSettings",
    route: "settings",
    settingsCategory: "models",
    openAddProviderCatalog: true,
    selector:
      "[data-testid='add-provider-catalog-view'], [data-testid='settings-models-tour-anchor'], [data-testid='rail-open-settings-btn']",
    preferredPlacement: "left",
    icon: "cpu",
  },
  {
    id: "data_accounts",
    i18nKey: "dataAccounts",
    route: "settings",
    settingsCategory: "auth",
    selector: "[data-testid='settings-accounts-tour-anchor'], [data-testid='rail-open-settings-btn']",
    preferredPlacement: "bottom",
    icon: "key",
  },
] as const;

interface SpotRect {
  top: number;
  left: number;
  width: number;
  height: number;
  right: number;
  bottom: number;
}

function resolvePortalContainer(): HTMLElement | null {
  if (typeof document === "undefined") return null;
  return (document.querySelector(".sp-modern") as HTMLElement | null) ?? document.body;
}

function measureVisibleTarget(selector: string): { rect: SpotRect; element: Element } | null {
  if (typeof document === "undefined" || !selector) return null;
  const parts = selector
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
  for (const part of parts) {
    const el = document.querySelector(part);
    if (!el) continue;
    const rect = el.getBoundingClientRect();
    if (rect.width >= 8 && rect.height >= 8) {
      return {
        element: el,
        rect: {
          top: rect.top,
          left: rect.left,
          width: rect.width,
          height: rect.height,
          right: rect.right,
          bottom: rect.bottom,
        },
      };
    }
  }
  return null;
}

function sameSpotRect(a: SpotRect | null, b: SpotRect | null): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return (
    Math.abs(a.top - b.top) < 0.5 &&
    Math.abs(a.left - b.left) < 0.5 &&
    Math.abs(a.width - b.width) < 0.5 &&
    Math.abs(a.height - b.height) < 0.5
  );
}

function StepIcon({
  icon,
  className,
}: {
  icon: GuidedTourStepConfig["icon"];
  className?: string;
}) {
  switch (icon) {
    case "message":
      return <MessageSquare className={className} />;
    case "mapPin":
      return <MapPin className={className} />;
    case "radar":
      return <Radar className={className} />;
    case "database":
      return <Database className={className} />;
    case "download":
      return <Download className={className} />;
    case "cpu":
      return <Cpu className={className} />;
    case "key":
      return <KeyRound className={className} />;
  }
}

export interface GuidedTourPromptBannerProps {
  open: boolean;
  onStartTour: () => void;
  onDismiss: () => void;
}

export function GuidedTourPromptBanner({
  open,
  onStartTour,
  onDismiss,
}: GuidedTourPromptBannerProps) {
  const { t } = useI18n();
  const container = resolvePortalContainer();
  if (!open || !container) return null;

  return createPortal(
    <div
      role="dialog"
      aria-modal={false}
      aria-label={t("shell.guidedTour.promptTitle")}
      data-testid="guided-tour-prompt-banner"
      className="fixed bottom-5 right-5 z-[1150] w-[320px] max-w-[calc(100vw-32px)] rounded-card border border-sp-line bg-sp-card p-4 shadow-modal text-sp-text"
    >
      <div className="flex items-start justify-between gap-2">
        <div className="flex items-center gap-2 min-w-0">
          <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-sp-sm border border-sp-line bg-sp-surface">
            <Compass className="h-3.5 w-3.5 text-sp-text" />
          </div>
          <div className="text-[13px] font-semibold text-sp-text truncate">
            {t("shell.guidedTour.promptTitle")}
          </div>
        </div>
        <button
          type="button"
          data-testid="guided-tour-prompt-close-btn"
          aria-label={t("shell.guidedTour.closeAria")}
          onClick={onDismiss}
          className="flex h-6 w-6 shrink-0 items-center justify-center rounded-sp-sm text-sp-text-muted transition-colors hover:bg-sp-hover hover:text-sp-text cursor-pointer"
        >
          <X className="h-3.5 w-3.5" />
        </button>
      </div>
      <p className="mt-2 text-xs leading-relaxed text-sp-text-secondary">
        {t("shell.guidedTour.promptSubtitle")}
      </p>
      <div className="mt-3 flex items-center justify-end gap-2">
        <button
          type="button"
          data-testid="guided-tour-prompt-dismiss-btn"
          onClick={onDismiss}
          className="eo-settings-btn-secondary h-8 px-3 text-xs whitespace-nowrap"
        >
          {t("shell.guidedTour.promptDismiss")}
        </button>
        <button
          type="button"
          data-testid="guided-tour-prompt-start-btn"
          onClick={onStartTour}
          className="eo-settings-btn-primary h-8 px-3 text-xs whitespace-nowrap"
        >
          {t("shell.guidedTour.promptStart")}
        </button>
      </div>
    </div>,
    container
  );
}

export interface DesktopGuidedTourProps {
  open: boolean;
  steps?: readonly GuidedTourStepConfig[];
  catalogSummary?: GuidedTourCatalogSummary;
  onStepEnter?: (step: GuidedTourStepConfig) => void;
  onFinish: (options?: { stayOnCurrent?: boolean; restoreRoute?: boolean }) => void;
}

export function DesktopGuidedTour({
  open,
  steps = DESKTOP_GUIDED_TOUR_STEPS,
  catalogSummary,
  onStepEnter,
  onFinish,
}: DesktopGuidedTourProps) {
  const { t } = useI18n();
  const defaultCatalogSummary = useMemo(() => computeGuidedTourCatalogSummary(), []);
  const activeCatalogSummary = catalogSummary ?? defaultCatalogSummary;
  const maskId = useId();
  const [stepIndex, setStepIndex] = useState(0);
  const [targetRect, setTargetRect] = useState<SpotRect | null>(null);
  const [cardSize, setCardSize] = useState<{ width: number; height: number }>({
    width: 368,
    height: 270,
  });
  const cardRef = useRef<HTMLDivElement | null>(null);
  const [isDark, setIsDark] = useState(false);

  const currentStep = steps[stepIndex] ?? steps[0];
  const totalSteps = steps.length;
  const isFirst = stepIndex === 0;
  const isLast = stepIndex >= totalSteps - 1;

  useEffect(() => {
    if (!open) {
      setStepIndex(0);
      setTargetRect(null);
      return;
    }
    setStepIndex(0);
  }, [open]);

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
    if (!open || !currentStep) return;
    onStepEnter?.(currentStep);
  }, [open, currentStep, onStepEnter]);

  // Pixel-locked target tracking across route switches, modal portals, and 260ms sidebar spring collapse/expand
  useEffect(() => {
    if (!open || !currentStep) return;

    let stopped = false;
    let rafHandle: number | null = null;
    let resizeObs: ResizeObserver | null = null;
    let observedTarget: Element | null = null;

    const syncRect = () => {
      if (stopped) return;
      const hit = measureVisibleTarget(currentStep.selector);
      const nextRect = hit?.rect ?? null;
      setTargetRect((prev) => (sameSpotRect(prev, nextRect) ? prev : nextRect));

      if (hit?.element && hit.element !== observedTarget && typeof ResizeObserver === "function") {
        observedTarget = hit.element;
        resizeObs?.observe(hit.element);
      }
    };

    if (typeof ResizeObserver === "function") {
      resizeObs = new ResizeObserver(syncRect);
      const rootIsland = document.querySelector(".sp-quiet-main-plane");
      const mainShell = document.querySelector(".sp-modern");
      if (rootIsland) resizeObs.observe(rootIsland);
      if (mainShell) resizeObs.observe(mainShell);
    }

    syncRect();

    // Lock spotlight frame to 60fps for 420ms so it tracks the 260ms sidebar spring transition seamlessly
    const loopEndMs = Date.now() + 420;
    const tick = () => {
      if (stopped) return;
      syncRect();
      if (Date.now() < loopEndMs && typeof window.requestAnimationFrame === "function") {
        rafHandle = window.requestAnimationFrame(tick);
      }
    };
    if (typeof window.requestAnimationFrame === "function") {
      rafHandle = window.requestAnimationFrame(tick);
    }

    window.addEventListener("resize", syncRect);
    window.addEventListener("scroll", syncRect, true);

    return () => {
      stopped = true;
      if (rafHandle !== null && typeof window.cancelAnimationFrame === "function") {
        window.cancelAnimationFrame(rafHandle);
      }
      resizeObs?.disconnect();
      window.removeEventListener("resize", syncRect);
      window.removeEventListener("scroll", syncRect, true);
    };
  }, [open, currentStep]);

  useLayoutEffect(() => {
    if (!open || !cardRef.current) return;
    const rect = cardRef.current.getBoundingClientRect();
    if (rect.width > 0 && rect.height > 0) {
      setCardSize((prev) =>
        Math.abs(prev.width - rect.width) > 2 || Math.abs(prev.height - rect.height) > 2
          ? { width: rect.width, height: rect.height }
          : prev
      );
    }
  }, [open, stepIndex, currentStep]);

  useEffect(() => {
    if (!open || typeof window === "undefined") return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        onFinish({ restoreRoute: true });
      } else if (e.key === "ArrowRight") {
        e.preventDefault();
        if (isLast) {
          onFinish();
        } else {
          setStepIndex((idx) => Math.min(totalSteps - 1, idx + 1));
        }
      } else if (e.key === "ArrowLeft") {
        e.preventDefault();
        setStepIndex((idx) => Math.max(0, idx - 1));
      }
    };
    window.addEventListener("keydown", handleKeyDown, true);
    return () => window.removeEventListener("keydown", handleKeyDown, true);
  }, [open, isLast, totalSteps, onFinish]);

  const spotPad = 6;
  const spotBox = useMemo(() => {
    if (!targetRect) return null;
    const vw = typeof window === "undefined" ? 1280 : window.innerWidth;
    const vh = typeof window === "undefined" ? 800 : window.innerHeight;
    const x = Math.max(4, targetRect.left - spotPad);
    const y = Math.max(4, targetRect.top - spotPad);
    const maxW = Math.max(16, vw - x - 4);
    const maxH = Math.max(16, vh - y - 4);
    return {
      x,
      y,
      width: Math.min(targetRect.width + spotPad * 2, maxW),
      height: Math.min(targetRect.height + spotPad * 2, maxH),
    };
  }, [targetRect]);

  const cardPositionStyle = useMemo<React.CSSProperties>(() => {
    const vw = typeof window === "undefined" ? 1280 : window.innerWidth;
    const vh = typeof window === "undefined" ? 800 : window.innerHeight;
    const cardWidth = Math.min(416, Math.max(280, vw - 24));
    const cardHeight = cardSize.height || 270;
    const gap = 16;
    const safeEdge = 16;

    if (!spotBox || currentStep?.preferredPlacement === "center") {
      return {
        width: cardWidth,
        left: Math.max(safeEdge, (vw - cardWidth) / 2),
        top: Math.max(safeEdge, (vh - cardHeight) / 2),
      };
    }

    const spotRight = spotBox.x + spotBox.width;
    const spotBottom = spotBox.y + spotBox.height;

    const fitsRight = spotRight + gap + cardWidth <= vw - safeEdge;
    const fitsLeft = spotBox.x - gap - cardWidth >= safeEdge;
    const fitsBottom = spotBottom + gap + cardHeight <= vh - safeEdge;
    const fitsTop = spotBox.y - gap - cardHeight >= safeEdge;

    const order: GuidedTourPlacement[] = [
      currentStep.preferredPlacement,
      "bottom",
      "right",
      "left",
      "top",
      "center",
    ];

    let chosen: GuidedTourPlacement = "center";
    for (const candidate of order) {
      if (candidate === "right" && fitsRight) {
        chosen = "right";
        break;
      }
      if (candidate === "left" && fitsLeft) {
        chosen = "left";
        break;
      }
      if (candidate === "bottom" && fitsBottom) {
        chosen = "bottom";
        break;
      }
      if (candidate === "top" && fitsTop) {
        chosen = "top";
        break;
      }
      if (candidate === "center") {
        chosen = "center";
        break;
      }
    }

    let left = (vw - cardWidth) / 2;
    let top = (vh - cardHeight) / 2;

    if (chosen === "right") {
      left = spotRight + gap;
      top = spotBox.y;
    } else if (chosen === "left") {
      left = spotBox.x - cardWidth - gap;
      top = spotBox.y;
    } else if (chosen === "bottom") {
      left = spotBox.x + (spotBox.width - cardWidth) / 2;
      top = spotBottom + gap;
    } else if (chosen === "top") {
      left = spotBox.x + (spotBox.width - cardWidth) / 2;
      top = spotBox.y - cardHeight - gap;
    }

    left = Math.min(Math.max(safeEdge, left), Math.max(safeEdge, vw - cardWidth - safeEdge));
    top = Math.min(Math.max(safeEdge, top), Math.max(safeEdge, vh - cardHeight - safeEdge));

    return { width: cardWidth, left, top };
  }, [spotBox, currentStep, cardSize.height]);

  const copyVars = useMemo(
    () => ({
      primary: activeCatalogSummary.primary,
      gee: activeCatalogSummary.gee,
      total: activeCatalogSummary.total,
      free: activeCatalogSummary.freePrimary,
    }),
    [activeCatalogSummary]
  );

  const container = resolvePortalContainer();
  if (!open || !currentStep || !container) return null;

  const badgeText = t(`shell.guidedTour.steps.${currentStep.i18nKey}.badge`, copyVars);
  const titleText = t(`shell.guidedTour.steps.${currentStep.i18nKey}.title`, copyVars);
  const bodyText = t(`shell.guidedTour.steps.${currentStep.i18nKey}.body`, copyVars);
  const chips = [
    t(`shell.guidedTour.steps.${currentStep.i18nKey}.chip1`, copyVars),
    t(`shell.guidedTour.steps.${currentStep.i18nKey}.chip2`, copyVars),
    t(`shell.guidedTour.steps.${currentStep.i18nKey}.chip3`, copyVars),
  ];

  return createPortal(
    <div
      role="dialog"
      aria-modal={true}
      aria-label={t("shell.guidedTour.dialogLabel")}
      data-testid="desktop-guided-tour"
      data-tour-step={currentStep.id}
      className="fixed inset-0 z-[1300]"
    >
      {/* Dimmed backdrop with clean SVG spotlight cutout */}
      <svg
        className="absolute inset-0 h-full w-full cursor-pointer"
        aria-hidden="true"
        onClick={() => onFinish({ restoreRoute: true })}
      >
        <defs>
          <mask id={maskId}>
            <rect width="100%" height="100%" fill="white" />
            {spotBox && (
              <rect
                x={spotBox.x}
                y={spotBox.y}
                width={spotBox.width}
                height={spotBox.height}
                rx={12}
                fill="black"
              />
            )}
          </mask>
        </defs>
        <rect
          width="100%"
          height="100%"
          fill={isDark ? "rgba(0, 0, 0, 0.62)" : "rgba(15, 23, 42, 0.42)"}
          mask={`url(#${maskId})`}
        />
      </svg>

      {/* Crisp spotlight border ring — zero coordinate transition so it stays 1:1 locked to sidebar motion */}
      {spotBox && (
        <div
          data-testid="guided-tour-spotlight-ring"
          className="pointer-events-none fixed rounded-[12px] border-2 border-sp-accent shadow-modal transition-opacity duration-150 motion-reduce:transition-none"
          style={{
            left: spotBox.x,
            top: spotBox.y,
            width: spotBox.width,
            height: spotBox.height,
          }}
        />
      )}

      {/* Calm monochrome popover card */}
      <div
        ref={cardRef}
        data-testid="guided-tour-card"
        onClick={(e) => e.stopPropagation()}
        style={cardPositionStyle}
        className="fixed z-10 rounded-card border border-sp-line bg-sp-card p-5 text-sp-text shadow-modal transition-opacity duration-150 motion-reduce:transition-none"
      >
        {/* Top bar: badge + counter + close */}
        <div className="flex items-center justify-between gap-2">
          <div className="flex items-center gap-2 min-w-0">
            <span className="inline-flex items-center gap-1.5 rounded-sp-sm border border-sp-line bg-sp-surface px-2 py-0.5 text-xs font-semibold text-sp-text">
              <StepIcon icon={currentStep.icon} className="h-3.5 w-3.5 shrink-0 text-sp-text" />
              <span className="truncate">{badgeText}</span>
            </span>
            <span
              data-testid="guided-tour-step-counter"
              className="text-xs font-semibold text-sp-text-muted tabular-nums whitespace-nowrap"
            >
              {t("shell.guidedTour.stepCounter", {
                current: stepIndex + 1,
                total: totalSteps,
              })}
            </span>
          </div>
          <button
            type="button"
            data-testid="guided-tour-close-btn"
            aria-label={t("shell.guidedTour.closeAria")}
            onClick={() => onFinish({ restoreRoute: true })}
            className="flex h-7 w-7 shrink-0 items-center justify-center rounded-sp-sm text-sp-text-muted transition-colors hover:bg-sp-hover hover:text-sp-text cursor-pointer"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        {/* Title & concise description */}
        <h2
          data-testid="guided-tour-step-title"
          className="mt-3 text-[15px] font-semibold leading-snug text-sp-text"
        >
          {titleText}
        </h2>
        <p className="mt-2 text-xs leading-relaxed text-sp-text-secondary">
          {bodyText}
        </p>

        {/* Feature highlights (3 concise monochrome chips) */}
        <div className="mt-3 flex flex-wrap gap-2">
          {chips.map((chip) => (
            <span
              key={chip}
              className="eo-settings-badge whitespace-nowrap"
            >
              {chip}
            </span>
          ))}
        </div>

        {/* Clickable segmented progress bar */}
        <div className="mt-4 flex items-center gap-1.5">
          {steps.map((stepItem, idx) => {
            const active = idx === stepIndex;
            const visited = idx < stepIndex;
            return (
              <button
                key={stepItem.id}
                type="button"
                data-testid={`guided-tour-dot-${idx}`}
                aria-label={t("shell.guidedTour.jumpToStepAria", { step: idx + 1 })}
                aria-current={active ? "step" : undefined}
                onClick={() => setStepIndex(idx)}
                className={cn(
                  "h-1.5 flex-1 rounded-full transition-colors duration-fast ease-sp motion-reduce:transition-none cursor-pointer",
                  active
                    ? "bg-sp-accent"
                    : visited
                      ? "bg-sp-text-secondary/50 hover:bg-sp-text-secondary"
                      : "bg-sp-surface-muted hover:bg-sp-line"
                )}
              />
            );
          })}
        </div>

        {/* Footer navigation */}
        <div className="mt-4 flex items-center justify-between gap-2 pt-3 border-t border-sp-divider">
          <button
            type="button"
            data-testid="guided-tour-skip-btn"
            onClick={() => onFinish({ restoreRoute: true })}
            className="text-xs font-semibold text-sp-text-muted transition-colors hover:text-sp-text whitespace-nowrap shrink-0 cursor-pointer"
          >
            {t("shell.guidedTour.skip")}
          </button>
          <div className="flex items-center gap-2">
            {!isFirst && (
              <button
                type="button"
                data-testid="guided-tour-prev-btn"
                onClick={() => setStepIndex((idx) => Math.max(0, idx - 1))}
                className="eo-settings-btn-secondary h-8 px-3 text-xs whitespace-nowrap"
              >
                <ChevronLeft className="h-3.5 w-3.5" />
                <span>{t("shell.guidedTour.prev")}</span>
              </button>
            )}
            {isLast && (
              <button
                type="button"
                data-testid="guided-tour-stay-btn"
                onClick={() => onFinish({ stayOnCurrent: true })}
                className="eo-settings-btn-secondary h-8 px-3 text-xs whitespace-nowrap"
              >
                <span>{t("shell.guidedTour.stayHere")}</span>
              </button>
            )}
            <button
              type="button"
              data-testid="guided-tour-next-btn"
              onClick={() => {
                if (isLast) {
                  onFinish();
                } else {
                  setStepIndex((idx) => Math.min(totalSteps - 1, idx + 1));
                }
              }}
              className="eo-settings-btn-primary h-8 px-3.5 text-xs whitespace-nowrap"
            >
              <span>{isLast ? t("shell.guidedTour.finish") : t("shell.guidedTour.next")}</span>
              {!isLast && <ChevronRight className="h-3.5 w-3.5" />}
            </button>
          </div>
        </div>
      </div>
    </div>,
    container
  );
}
