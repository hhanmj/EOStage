import { taskOutputDir } from "@/shell/api/taskLogic";
import { taskCardSizeLine } from "@/shell/api/taskCardLogic";
import { formatRunStarted } from "@/shell/api/agentRunBadges";

/**
 * DesktopAgentWorkbench.tsx
 * Agent 工作台：集成真实会话环、事件流轮询、计划/确认卡、运行记录与取消
 * 规范：EOStage Agent UI 规范
 */

import React, {
  useState,
  useEffect,
  useMemo,
  useCallback,
  useRef,
} from "react";
import {
  Send,
  Plus,
  Trash2,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  Clock,
  AlertTriangle,
  RotateCcw,
  ExternalLink,
  MapPin,
  Calendar,
  Layers,
  PanelLeftClose,
  PanelLeftOpen,
  Check,
  X,
  FileQuestion,
  Search,
  Settings,
  HelpCircle,
  ArrowDown,
  CheckCircle2,
  FileText,
  Download,
  Play,
  Pause,
  ShieldCheck,
  Map as MapIcon,
  Square,
  CheckSquare,
  List,
  ClipboardList,
  Eye,
  EyeOff,
  LocateFixed,
  ChevronUp,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";
import type { Bbox, AoiPreviewOk } from "@/lib/bridge";
import {
  previewAoiFileBytes,
  setWorkingAoiGeojsonFeatures,
  pickDirectory,
  getCredentialStatus,
} from "@/lib/bridge";
import { VectorFeaturePickerModal } from "./VectorFeaturePickerModal";
import { ChatTokenizedText } from "./agent/ChatTokenizedText";
import {
  getBusySendPrefs,
  getWorkStepsPrefs,
  pickLatestUsageRun,
  shouldExpandToolGroupByDefault,
  shouldHideIntermediateReply,
  subscribeBusySendPrefs,
  subscribeWorkStepsPrefs,
  type BusySendMode,
  type WorkStepsPrefs,
} from "@/lib/performanceAndUsagePrefs";
import {
  WorkbenchMap,
} from "@/components/WorkbenchMap";
import { StatusStateCard } from "@/shell/components/StatusStateCard";
import { EmptyState } from "@/components/ui/EmptyState";
import { EmptyStateIcon, EmptyStateWireframe } from "@/components/ui/EmptyStateVisuals";
import { useDelayedFlag } from "@/hooks/useLoadingPrimitives";
import { AgentMessagesSkeleton } from "@/components/ui/Skeleton";
import { getDesktopShellBridge, isTauriDesktopHost } from "@/lib/desktopShellBridge";
import {
  resolveAgentToolDisplayName,
  resolveAoiSync,
} from "@/shell/api/agentLogic";
import { GeeScriptConfirmCard } from "./GeeScriptConfirmCard";
import {
  classifyStreamEvents,
  confirmSnapshotOf,
  buildSendContext,
  errorCancelledWording,
  channelReadinessFromViews,
  type StreamItem,
  type StreamMessageItem,
  type StreamToolGroupItem,
  type StreamPlanCardItem,
  type StreamConfirmCardItem,
  type StreamProgressCardItem,
  type StreamConclusionItem,
  type StreamErrorItem,
  type ExtractedConfirmSnapshot,
  type ChannelReadinessResult,
} from "@/shell/api/agentStreamLogic";
import {
  projectToolGroup,
  projectEvidence,
  buildTaskCardModels,
  projectSearchCard,
  findTerminalProgressForPlan,
  progressStatusLineZh,
} from "@/shell/api/agentConversationLogic";
import { ORBIT_LEGEND_LABELS_EN } from "@/shell/api/orbitTerms";
import {
  confirmCardSource,
  isSearchToolGroup,
  searchCardContext,
} from "@/shell/api/searchCardSource";
import {
  newlySettledSearchCallIds,
  extractSearchResourceIdFromEvents,
} from "@/shell/api/agentSearchViewSync";
import {
  getBatchPage,
  openOutputDirectory,
} from "@/shell/api";
import type { ApplicationTask, SceneRow, RowTaskState, RowAction } from "@/shell/api/types";
import type {
  WorkbenchDrawMode,
  MapLayerKey,
  GeodataRasterOverlaySpec,
} from "@/components/WorkbenchMap";
import { isEnabledMapProvider, type MapProviderKey } from "@/lib/mapProviders";
import { useWorkbenchMapPreview } from "../../hooks/useWorkbenchMapPreview";
import { useSceneSwipeComparison } from "../../hooks/useSceneSwipeComparison";
import {
  canonicalAgentDatasetShortName,
  extractSessionBatchesFromEvents,
  filterBatchCollectionByAoi,
  projectAgentSessionBatchCollection,
  resolveAgentSessionSourceLabel,
  sceneIntersectsAoiBbox,
  type AgentSessionBatchEntry,
} from "@/shell/api/agentSessionWorkspace";
import {
  AgentUnifiedResultsDeck,
  resolveAgentBatchShortName,
} from "./agent/AgentMultiBatchResultsView";
import { subscribeDesktopHostEvent } from "@/lib/desktopHostEvents";
import { CatalogResultRow } from "./CatalogResultRow";
import { AgentTaskCard } from "./AgentTaskCard";
import { AgentErrorCard } from "./AgentErrorCard";
import { AgentConclusionCard } from "./AgentConclusionCard";
import { AgentToolGroupCard } from "./AgentToolGroupCard";
import { AgentMarkdownRenderer } from "./agent/AgentMarkdownRenderer";
import {
  AgentThinkingCard,
  UnifiedTurnProcessHeader,
} from "./agent/AgentThinkingCard";
import { AgentTaskProgressCard } from "./agent/AgentTaskProgressCard";
import { AgentMessageActionBar } from "./agent/AgentMessageActionBar";
import {
  AgentQuestionCard,
  isPassiveDismissQuestionOption,
} from "./agent/AgentQuestionCard";
import { notifyEOStageAccountChanged } from "@/shell/api/eostageAccount";
import {
  inferResourceIdFromSceneId,
  matchSceneIdFromList,
} from "./agent/AgentMapSceneDrawer";
import {
  AgentComposerEyebrow,
  type QueuedComposerMessage,
} from "./agent/AgentComposerEyebrow";
import { AgentWorkPanel } from "./AgentWorkPanel";
import { TaskArtifactsVerificationPanel } from "./TaskArtifactsVerificationPanel";
import { countSessionActiveDownloads } from "./agent/sessionDownloadScope";
import {
  MAX_COMPOSER_ATTACHMENTS,
  formatAttachmentSize,
  readFileAsComposerAttachment,
  toWireAttachments,
  type ComposerAttachment,
  type WireAgentAttachment,
} from "./agent/composerAttachments";
import { createdTaskCount } from "@/shell/api/agentWork";
import {
  resolvePanelOpen,
  resolvePanelMode,
  resolvePanelTab,
  panelWidth,
  clampPanelWidth,
  type PanelTab,
  type PanelOpenIntent,
} from "@/shell/api/agentPanelLogic";
import {
  vectorAoiDisplayName,
} from "@/shell/api/agentComposerLogic";
import { subscribeChannelsChanged } from "@/shell/api/agentChannelsEvents";
import { subscribeSessionsChanged } from "./useSessionOrganizer";
import { extractBase64FromDataUrl } from "@/shell/api/vectorAoiImport";
import {
  parseStoredModelSelection,
  resolveModelSelection,
} from "@/shell/api/channelModels";
import { setChannelHealth } from "@/shell/api/channelHealth";
import { AgentComposer } from "./AgentComposer";
import { AgentConfirmCard } from "./AgentConfirmCard";
import { AgentPlanCard } from "./AgentPlanCard";
import { useAgentActiveSession } from "./agentSessionHooks";
import { useAgentActivityNotifier } from "./agentActivityHooks";
import {
  rememberApprovedOutputDir,
  recallApprovedOutputDir,
} from "@/shell/api/approvedOutputDirMemory";
import {
  projectApproval,
  planAutoCancel,
  autoCancelOutcomeText,
} from "@/shell/api/approvalLogic";
import { executeAutoCancel } from "@/shell/api/autoCancelFlow";
import type {
  AgentSessionSummary,
  AgentRunSummary,
  AgentWorkItem,
  AgentStreamEvent,
  AgentChannelView,
  AgentPermissionMode,
  AgentPermissionModeView,
  AgentLiveDeltaPayload,
} from "@/shell/api/agentBridgeTypes";
import {
  sessionTitleFromRequest,
  resolveActiveSession,
} from "@/shell/api/agentHistoryLogic";
import { truncateMiddle } from "@/shell/api/rowProjection";
import { explainErrorCode, explainActionError } from "@/shell/api/errorExplanations";
import {
  applyPendingSessionMode,
  displayedPermissionView,
} from "@/shell/api/agentPermissionLogic";
import {
  agentCreatedTaskIds,
  stopAllSummary,
} from "@/shell/api/stopAllLogic";
import type { AgentStopAllResult, AgentModelSelection } from "@/lib/generated/v3-contracts";
import {
  createFixtureEvents__fixture,
  createFixtureSessions__fixture,
  createFixtureRuns__fixture,
  createFixtureChannelViews__fixture,
  createFixturePermissionView__fixture,
  createFixtureStopAllResult__fixture,
} from "@/shell/api/agentWorkbenchFixture";

// 生产构建 DCE 常量折叠门控：生产环境下为 null，所有 fixture 模块及数据将被构建工具彻底剔除
const fixtureEventsFactory = import.meta.env.DEV ? createFixtureEvents__fixture : null;
const fixtureSessionsFactory = import.meta.env.DEV ? createFixtureSessions__fixture : null;
const fixtureRunsFactory = import.meta.env.DEV ? createFixtureRuns__fixture : null;
const fixtureChannelsFactory = import.meta.env.DEV ? createFixtureChannelViews__fixture : null;
const fixturePermissionViewFactory = import.meta.env.DEV ? createFixturePermissionView__fixture : null;

export const historyFixtureSessionsFactory = fixtureSessionsFactory;
export const stopAllFixtureFactory = import.meta.env.DEV ? createFixtureStopAllResult__fixture : null;

const AGENT_MODEL_SELECTION_STORAGE_KEY = "eostage.agent.modelSelection";
const sessionModelSelections: Record<string, AgentModelSelection | null> = {};

function readStoredModelSelection(): AgentModelSelection | null {
  try {
    return typeof window !== "undefined"
      ? parseStoredModelSelection(window.localStorage.getItem(AGENT_MODEL_SELECTION_STORAGE_KEY))
      : null;
  } catch {
    return null;
  }
}

export interface DesktopAgentWorkbenchProps {
  /** 全局默认下载目录（= 存储根设置），确认卡「使用默认目录」用 */
  defaultDownloadDir?: string;
  /** 数据源中文名（resource_id -> display_name_zh），计划卡检索披露用 */
  sourceNames?: Readonly<Record<string, string>>;
  openSessionId?: string | null;
  newChatNonce?: number;
  workbenchAoi?: Bbox | null;
  aoiLabel?: string;
  timeRange?: { start: string | null; end: string | null } | null;
  sourceId?: string | null;
  onUpdateWorkbenchAoi?: (aoi: Bbox, aoiName?: string) => void;
  onOpenDownloadCenter?: (taskId?: string) => void;
  onOpenSettings?: (category?: "network" | "models" | "accounts" | "auth" | "performance_usage" | "work_steps") => void;
  onDrawRectRequest?: () => void;
  onLocateAdminBoundary?: () => void;
  onAgentChannelSelect?: (id: string) => void;
  agentTasks?: ApplicationTask[];
  refreshAgentTasks?: () => Promise<ApplicationTask[]>;
  onTaskAction?: (
    taskId: string,
    revision: number,
    action: "pause" | "resume" | "cancel" | "retry"
  ) => Promise<void>;
  onViewOnMap?: () => void;
  onActiveSessionTitleChange?: (title: string | null) => void;
  onActiveSessionChange?: (sessionId: string | null) => void;
  onAgentSearchSettled?: (info: { resourceId: string | null }) => void;
  onAgentSessionBatchSettled?: (info: {
    sessionId: string | null;
    batchId: string | null;
    resourceId: string | null;
    loadedCount: number | null;
    totalCount: number | null;
    selectThisBatch?: boolean;
  }) => void | Promise<void>;
  onSessionCreatedSnapshot?: (sessionId: string) => void | Promise<void>;
  batchTotalCount?: number | null;
  focusBoundsRequest?: { bbox: Bbox; nonce: number } | null;
  onAgentActivityChange?: (activity: { liveRunIds: string[]; agentTaskIds: string[] }) => void;
  onActivityChanged?: () => void;
  onStopAllEvent?: (sessionId: string, seq: number, payload: AgentStopAllResult) => void;
  sidebarWidth?: number;
  panelOpen?: boolean;
  onPanelOpenChange?: (open: boolean) => void;
  scenes?: SceneRow[];
  rowStates?: Map<string, RowTaskState>;
  selectedSceneId?: string | null;
  onSelectScene?: (sceneId: string | null) => void;
  checkedSceneIds?: Set<string>;
  onToggleCheckScene?: (sceneId: string) => void;
  onToggleCheckAll?: () => void;
  onRowAction?: (itemId: string, action: RowAction) => void;
  creatingItemIds?: Set<string>;
  onDownloadChecked?: () => void;
  downloadBusy?: boolean;
  workingGeometry?: Record<string, unknown> | null;
  geodataRasterOverlay?: GeodataRasterOverlaySpec | null;
  batchId?: string | null;
  sessionBatches?: readonly AgentSessionBatchEntry[];
  activeBatchFilter?: "all" | string;
  onSelectBatchFilter?: (filter: "all" | string) => void;
  onOpenInMapWorkbench?: (
    resourceId?: string | null,
    batchId?: string | null
  ) => void | Promise<void>;
  onSyncWorkingAoi?: () => void | Promise<void>;
  drawMode?: WorkbenchDrawMode;
  drawActive?: boolean;
  onDrawModeChange?: (mode: WorkbenchDrawMode) => void;
  onDrawActiveChange?: (active: boolean) => void;
  onClearLayers?: () => void;
  onRectDraw?: (bbox: Bbox) => void;
  onPolygonDraw?: (polygon: any) => void;
  onPointDraw?: (lat: number, lng: number) => void;
  mapLayerKey?: MapLayerKey;
  onMapLayerChange?: (key: MapLayerKey) => void;
  onOpenTileDownloadWorkbench?: (suggestedTileSource: string) => void;
  forcedSubscene?:
    | "plan-pending"
    | "confirm-pending"
    | "confirm-approved"
    | "confirm-expired"
    | "card-dir-default"
    | "card-dir-empty"
    | "card-dir-model"
    | "card-dir-message"
    | "message-dir-ref"
    | "plan-disclosure"
    | "conclusion"
    | "reasoning-refused"
    | "model-truncated"
    | "conversation-full"
    | "no-channel"
    | null;
}


const LOCAL_STORAGE_PANEL_WIDTH_KEY = "eostage.agent.panelWidth";

function readPersistedPanelWidth(): number | null {
  try {
    if (typeof window === "undefined" || !window.localStorage) return null;
    const raw = window.localStorage.getItem(LOCAL_STORAGE_PANEL_WIDTH_KEY);
    if (!raw) return null;
    const parsed = Number(raw);
    return Number.isFinite(parsed) && parsed > 0 ? parsed : null;
  } catch {
    return null;
  }
}

function writePersistedPanelWidth(width: number | null): void {
  try {
    if (typeof window === "undefined" || !window.localStorage) return;
    if (width === null) {
      window.localStorage.removeItem(LOCAL_STORAGE_PANEL_WIDTH_KEY);
    } else {
      window.localStorage.setItem(LOCAL_STORAGE_PANEL_WIDTH_KEY, String(Math.round(width)));
    }
  } catch {
    // ignore
  }
}


function normalizeAgentBasemapKey(raw: string): MapLayerKey | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  if (isEnabledMapProvider(trimmed as MapProviderKey)) {
    return trimmed as MapLayerKey;
  }
  const lower = trimmed.toLowerCase();
  const aliasMap: Record<string, MapProviderKey> = {
    satellite: "arcgisSatellite",
    imagery: "arcgisSatellite",
    arcgis_satellite: "arcgisSatellite",
    卫星: "arcgisSatellite",
    卫星影像: "arcgisSatellite",
    topo: "arcgisTopo",
    terrain: "arcgisTopo",
    arcgis_topo: "arcgisTopo",
    地形: "arcgisTopo",
    地形图: "arcgisTopo",
    opentopo: "openTopoMap",
    opentopomap: "openTopoMap",
    street: "arcgisStreet",
    streets: "arcgisStreet",
    arcgis_street: "arcgisStreet",
    街道: "arcgisStreet",
    openstreetmap: "osm",
    light: "cartoLight",
    carto_light: "cartoLight",
    浅色: "cartoLight",
    dark: "cartoDark",
    carto_dark: "cartoDark",
    深色: "cartoDark",
  };
  const mapped = aliasMap[lower];
  return mapped && isEnabledMapProvider(mapped) ? mapped : null;
}

function extractToolGroupFocusTarget(
  item: StreamToolGroupItem,
  allEvents?: readonly AgentStreamEvent[]
): {
  itemIds: string[];
  batchId: string | null;
} {
  const itemIds: string[] = [];
  let batchId: string | null = null;
  for (const round of item.rounds) {
    const rec = round as unknown as Record<string, unknown>;
    const subCalls = Array.isArray(rec.calls) ? (rec.calls as Record<string, unknown>[]) : [rec];
    for (const callObj of subCalls) {
      for (const container of [callObj, callObj.args, callObj.output, callObj.result]) {
        if (!container || typeof container !== "object") continue;
        const obj = container as Record<string, unknown>;
        if (!batchId && typeof obj.batch_id === "string" && obj.batch_id.trim()) {
          batchId = obj.batch_id.trim();
        }
        if (Array.isArray(obj.item_ids)) {
          for (const id of obj.item_ids) {
            if (typeof id === "string" && id.trim() && !itemIds.includes(id.trim())) {
              itemIds.push(id.trim());
            }
          }
        }
        if (typeof obj.scene_id === "string" && obj.scene_id.trim() && !itemIds.includes(obj.scene_id.trim())) {
          itemIds.push(obj.scene_id.trim());
        }
      }
    }
  }
  if (!batchId && allEvents && item.run_id) {
    for (const ev of allEvents) {
      if (ev.kind !== "workspace_sync") continue;
      const p = (ev.payload || {}) as Record<string, unknown>;
      if (p.run_id === item.run_id && p.action === "search_updated" && typeof p.batch_id === "string" && p.batch_id.trim()) {
        batchId = p.batch_id.trim();
      }
    }
  }
  return { itemIds, batchId };
}

function extractConfirmFocusTarget(item: StreamConfirmCardItem): {
  itemIds: string[];
  batchId: string | null;
} {
  const snap = (item.snapshot || {}) as Record<string, unknown>;
  const batchId =
    typeof snap.batch_id === "string" && snap.batch_id.trim() ? snap.batch_id.trim() : null;
  const itemIds = Array.isArray(snap.item_ids)
    ? snap.item_ids.filter((id): id is string => typeof id === "string" && id.trim().length > 0)
    : [];
  return { itemIds, batchId };
}

const EMPTY_AGENT_TASKS: ApplicationTask[] = [];
const EMPTY_SCENES: SceneRow[] = [];
const EMPTY_ROW_STATES: Map<string, RowTaskState> = new Map();
const EMPTY_STRING_SET: Set<string> = new Set();

export function DesktopAgentWorkbench({
  defaultDownloadDir = "",
  sourceNames,
  openSessionId = null,
  newChatNonce,
  workbenchAoi = null,
  aoiLabel,
  timeRange,
  sourceId,
  onUpdateWorkbenchAoi,
  onOpenDownloadCenter,
  onOpenSettings,
  onDrawRectRequest,
  onLocateAdminBoundary,
  onAgentChannelSelect,
  agentTasks = EMPTY_AGENT_TASKS,
  refreshAgentTasks,
  onTaskAction,
  onViewOnMap,
  onActiveSessionTitleChange,
  onActiveSessionChange,
  onAgentSearchSettled,
  onAgentSessionBatchSettled,
  onSessionCreatedSnapshot,
  batchTotalCount = null,
  focusBoundsRequest = null,
  onAgentActivityChange,
  onActivityChanged,
  onStopAllEvent,
  sidebarWidth: sidebarWidthProp,
  panelOpen,
  onPanelOpenChange,
  scenes = EMPTY_SCENES,
  rowStates = EMPTY_ROW_STATES,
  selectedSceneId = null,
  onSelectScene,
  checkedSceneIds = EMPTY_STRING_SET,
  onToggleCheckScene,
  onToggleCheckAll,
  onRowAction,
  creatingItemIds = EMPTY_STRING_SET,
  onDownloadChecked,
  downloadBusy = false,
  workingGeometry = null,
  geodataRasterOverlay = null,
  batchId = null,
  sessionBatches,
  activeBatchFilter,
  onSelectBatchFilter,
  onOpenInMapWorkbench,
  onSyncWorkingAoi,
  drawMode = "rect",
  drawActive = false,
  onDrawModeChange,
  onDrawActiveChange,
  onClearLayers,
  onRectDraw,
  onPolygonDraw,
  onPointDraw,
  mapLayerKey = "arcgisSatellite",
  onMapLayerChange,
  onOpenTileDownloadWorkbench,
  forcedSubscene = null,
}: DesktopAgentWorkbenchProps) {
  const { t } = useI18n();
  // 1. 响应式布局宽度监听与面板状态
  const [viewportWidth, setViewportWidth] = useState<number>(() =>
    typeof window !== "undefined" ? window.innerWidth : 1280
  );

  useEffect(() => {
    const handleResize = () => setViewportWidth(window.innerWidth);
    window.addEventListener("resize", handleResize);
    return () => window.removeEventListener("resize", handleResize);
  }, []);

  const [panelRequested, setPanelRequested] = useState<boolean>(false);
  const [panelIntent, setPanelIntent] = useState<PanelOpenIntent>(null);

  useEffect(() => {
    if (panelOpen === false) {
      setPanelRequested(false);
      setPanelIntent(null);
    } else if (panelOpen === true) {
      setPanelRequested(true);
    }
  }, [panelOpen]);

  const isPanelOpen = resolvePanelOpen({
    requested: panelOpen !== undefined ? panelOpen : panelRequested,
    viewportWidth,
    openIntent: panelIntent,
  });

  const handleClosePanel = useCallback(() => {
    setPanelRequested(false);
    setPanelIntent(null);
    onPanelOpenChange?.(false);
  }, [onPanelOpenChange]);

  const panelMode = resolvePanelMode(viewportWidth);
  const defaultPanelWidth = panelWidth(viewportWidth);
  const [userPanelWidth, setUserPanelWidth] = useState<number | null>(() => readPersistedPanelWidth());
  const [isDraggingSeparator, setIsDraggingSeparator] = useState(false);
  const splitContainerRef = useRef<HTMLDivElement>(null);

  const effectivePanelWidth: number | null = useMemo(() => {
    if (defaultPanelWidth === null) return null;
    const desired = userPanelWidth ?? defaultPanelWidth;
    const container = splitContainerRef.current;
    const sidebar = sidebarWidthProp ?? (container
      ? (viewportWidth - container.clientWidth)
      : (typeof document !== "undefined" ? (document.querySelector(`nav[aria-label='${t("shell.nav.main")}']`)?.clientWidth || 0) : 0));
    return clampPanelWidth(desired, viewportWidth, sidebar);
  }, [defaultPanelWidth, userPanelWidth, viewportWidth, sidebarWidthProp, t]);

  // 侧栏变宽时面板先让位：effectivePanelWidth 在渲染时按实际侧栏宽度重新裁决，
  // 不回写用户设定的宽度，侧栏变窄后面板能恢复原宽度。

  // Re-clamp on window resize
  useEffect(() => {
    const handleResize = () => {
      const w = window.innerWidth;
      setViewportWidth(w);
        setUserPanelWidth((prev) => {
          if (prev === null) return null;
          const container = splitContainerRef.current;
          const sidebar = container
            ? (w - container.clientWidth)
            : (typeof document !== "undefined" ? (document.querySelector(`nav[aria-label='${t("shell.nav.main")}']`)?.clientWidth || 0) : 0);
          const clamped = clampPanelWidth(prev, w, sidebar);
          if (clamped !== prev) {
            writePersistedPanelWidth(clamped);
            return clamped;
          }
          return prev;
        });
      };
      window.addEventListener("resize", handleResize);
      return () => window.removeEventListener("resize", handleResize);
    }, [t]);

  // While dragging, disable text selection across document
  useEffect(() => {
    if (!isDraggingSeparator) return;
    const prevSelect = document.body.style.userSelect;
    document.body.style.userSelect = "none";
    return () => {
      document.body.style.userSelect = prevSelect;
    };
  }, [isDraggingSeparator]);

  const handleSeparatorPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const handle = e.currentTarget;
    const prevBodyCursor = typeof document !== "undefined" ? document.body.style.cursor : "";
    const prevBodyUserSelect = typeof document !== "undefined" ? document.body.style.userSelect : "";
    if (typeof document !== "undefined") {
      document.body.style.cursor = "col-resize";
      document.body.style.userSelect = "none";
    }
    try {
      handle.setPointerCapture(e.pointerId);
    } catch {
      // ignore
    }
    setIsDraggingSeparator(true);

    const startX = e.clientX;
    const currentW = effectivePanelWidth ?? defaultPanelWidth ?? 538;
    const sidebar = splitContainerRef.current
      ? (window.innerWidth - splitContainerRef.current.clientWidth)
      : (typeof document !== "undefined" ? (document.querySelector(`nav[aria-label='${t("shell.nav.main")}']`)?.clientWidth || 0) : 0);

    let latestDesired = currentW;

    const onPointerMove = (ev: PointerEvent) => {
      const delta = startX - ev.clientX;
      const desired = currentW + delta;
      latestDesired = desired;
      if (desired < 280) {
        setUserPanelWidth(Math.max(0, desired));
      } else {
        const clamped = clampPanelWidth(desired, window.innerWidth, sidebar);
        setUserPanelWidth(clamped);
        writePersistedPanelWidth(clamped);
      }
    };

    const onPointerUp = (ev: PointerEvent) => {
      setIsDraggingSeparator(false);
      if (typeof document !== "undefined") {
        document.body.style.cursor = prevBodyCursor;
        document.body.style.userSelect = prevBodyUserSelect;
      }
      try {
        handle.releasePointerCapture(ev.pointerId);
      } catch {
        // ignore
      }
      window.removeEventListener("pointermove", onPointerMove);
      window.removeEventListener("pointerup", onPointerUp);
      window.removeEventListener("pointercancel", onPointerUp);

      if (latestDesired < 280) {
        // 拖动窄于 280px，释放时吸附关闭面板，重新打开恢复上次有效宽度
        setUserPanelWidth(currentW);
        writePersistedPanelWidth(currentW);
        handleClosePanel();
      } else {
        const clamped = clampPanelWidth(latestDesired, window.innerWidth, sidebar);
        setUserPanelWidth(clamped);
        writePersistedPanelWidth(clamped);
      }
    };

    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", onPointerUp);
    window.addEventListener("pointercancel", onPointerUp);
  };

  const handleSeparatorKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    const currentW = effectivePanelWidth ?? defaultPanelWidth ?? 538;
    const sidebar = splitContainerRef.current
      ? (viewportWidth - splitContainerRef.current.clientWidth)
      : (typeof document !== "undefined" ? (document.querySelector(`nav[aria-label='${t("shell.nav.main")}']`)?.clientWidth || 0) : 0);

    if (e.key === "ArrowLeft") {
      e.preventDefault();
      const desired = currentW + 16;
      const clamped = clampPanelWidth(desired, viewportWidth, sidebar);
      setUserPanelWidth(clamped);
      writePersistedPanelWidth(clamped);
    } else if (e.key === "ArrowRight") {
      e.preventDefault();
      const desired = currentW - 16;
      const clamped = clampPanelWidth(desired, viewportWidth, sidebar);
      setUserPanelWidth(clamped);
      writePersistedPanelWidth(clamped);
    } else if (e.key === "Enter") {
      e.preventDefault();
      handleResetPanelWidth();
    }
  };

  const handleResetPanelWidth = () => {
    setUserPanelWidth(null);
    writePersistedPanelWidth(null);
  };
  const [panelTab, setPanelTab] = useState<PanelTab>(() => {
    if (typeof window !== "undefined" && import.meta.env.DEV) {
      if (window.location.hash.includes("tab=task")) return "task";
    }
    return "map";
  });
  const [taskPanelSubview, setTaskPanelSubview] = useState<"progress" | "artifacts">("progress");

  const [hydratedBatchScenes, setHydratedBatchScenes] = useState<SceneRow[]>([]);
  const [hydratedBatchTotalCount, setHydratedBatchTotalCount] = useState<number | null>(null);
  const [localHydratedBatches, setLocalHydratedBatches] = useState<AgentSessionBatchEntry[]>([]);
  const [localBatchFilter, setLocalBatchFilter] = useState<"all" | string>("all");
  const [dockMode, setDockMode] = useState<"collapsed" | "split" | "full">(() => {
    if (typeof window !== "undefined" && import.meta.env.DEV && window.location.hash.includes("tab=results")) {
      return "full";
    }
    return "collapsed";
  });
  const [dockSplitRatio, setDockSplitRatio] = useState<number>(0.45);
  const splitContainerColumnRef = useRef<HTMLDivElement | null>(null);

  const effectiveRawBatches =
    sessionBatches && sessionBatches.length > 0 ? sessionBatches : localHydratedBatches;
  const effectiveRequestedFilter = activeBatchFilter ?? localBatchFilter;
  const effectiveBatchCollection = useMemo(() => {
    const projected = projectAgentSessionBatchCollection(effectiveRawBatches, effectiveRequestedFilter);
    return filterBatchCollectionByAoi(projected, workbenchAoi);
  }, [effectiveRawBatches, effectiveRequestedFilter, workbenchAoi]);

  const filteredStandaloneScenes = useMemo(
    () => scenes.filter((s) => sceneIntersectsAoiBbox(s, workbenchAoi)),
    [scenes, workbenchAoi]
  );
  const filteredHydratedScenes = useMemo(
    () => hydratedBatchScenes.filter((s) => sceneIntersectsAoiBbox(s, workbenchAoi)),
    [hydratedBatchScenes, workbenchAoi]
  );

  const effectiveScenes =
    effectiveBatchCollection.batches.length > 0
      ? effectiveBatchCollection.scenes
      : filteredStandaloneScenes.length > 0
        ? filteredStandaloneScenes
        : filteredHydratedScenes;
  const effectiveBatchTotalCount =
    effectiveBatchCollection.batches.length > 0
      ? effectiveBatchCollection.totalCount
      : filteredStandaloneScenes.length !== scenes.length
        ? filteredStandaloneScenes.length
        : (batchTotalCount ?? hydratedBatchTotalCount);
  const effectiveRowStates =
    effectiveBatchCollection.batches.length > 0 ? effectiveBatchCollection.rowStates : rowStates;
  const effectiveActiveBatchId = effectiveBatchCollection.activeBatchId ?? batchId;
  const effectiveActiveResourceId = effectiveBatchCollection.activeResourceId ?? sourceId ?? null;

  const resolveBatchIdForScene = useCallback(
    (scene: SceneRow | null | undefined): string | null => {
      if (!scene) return effectiveActiveBatchId ?? null;
      for (const b of effectiveBatchCollection.batches) {
        if (b.scenes.some((s) => s.scene_id === scene.scene_id)) {
          return b.batchId;
        }
      }
      return effectiveActiveBatchId ?? null;
    },
    [effectiveActiveBatchId, effectiveBatchCollection.batches]
  );

  const selectedSceneBatchId = useMemo(() => {
    if (!selectedSceneId) return effectiveActiveBatchId;
    const found = effectiveScenes.find((s) => s.scene_id === selectedSceneId) ?? null;
    return resolveBatchIdForScene(found);
  }, [effectiveActiveBatchId, effectiveScenes, resolveBatchIdForScene, selectedSceneId]);

  const {
    mapPreviewEnabled,
    setMapPreviewEnabled,
    pinnedPreviewIds,
    pinnedPreviews,
    togglePinPreviewScene,
    handlePinnedPreviewError,
    hoveredSceneId,
    setHoveredSceneId,
    selectedSceneObj,
    hasMapPreviewAvailable,
    activePreviewTileLayer,
    activePreviewKey,
    selectedScenePreviewBbox,
    mapPreviewState,
    identityIndex,
    stackScenesFor,
    handleSceneSelectFromMap,
  } = useWorkbenchMapPreview({
    scenes: effectiveScenes,
    selectedSceneId,
    batchId: selectedSceneBatchId,
    onSelectScene,
    explicitToggleMode: true,
  });

  const {
    swipeLeftLayer,
    swipeRightLayer,
    swipePosition,
    setSwipePosition,
    clearSwipe,
  } = useSceneSwipeComparison({
    workingAoi: workbenchAoi,
    defaultBatchId: effectiveActiveBatchId,
    resolveBatchIdForScene,
  });

  const tasksById = useMemo(() => {
    const map = new Map<string, ApplicationTask>();
    for (const tk of agentTasks) {
      map.set(tk.task_id, tk);
    }
    return map;
  }, [agentTasks]);

  const [locateRequest, setLocateRequest] = useState<{ sceneId: string; seq: number } | null>(null);

  const [localFocusBoundsRequest, setLocalFocusBoundsRequest] = useState<{ bbox: Bbox; nonce: number } | null>(null);
  const focusNonceRef = useRef(0);
  const triggerFocusBounds = useCallback((bbox: Bbox | null | undefined) => {
    if (
      !bbox ||
      ![bbox.west, bbox.south, bbox.east, bbox.north].every(Number.isFinite) ||
      bbox.west >= bbox.east ||
      bbox.south >= bbox.north
    ) {
      return;
    }
    focusNonceRef.current += 1;
    setLocalFocusBoundsRequest({ bbox, nonce: focusNonceRef.current });
  }, []);

  useEffect(() => {
    if (focusBoundsRequest) {
      focusNonceRef.current = Math.max(focusNonceRef.current + 1, focusBoundsRequest.nonce);
      setLocalFocusBoundsRequest({
        bbox: focusBoundsRequest.bbox,
        nonce: focusNonceRef.current,
      });
    }
  }, [focusBoundsRequest]);

  const openSplitPanel = useCallback(
    (tab: PanelTab = "map") => {
      if (tab === "results") {
        setPanelTab("map");
        setDockMode("full");
      } else {
        setPanelTab(tab);
        if (tab === "map" && dockMode === "full") {
          setDockMode("split");
        }
      }
      setPanelIntent("map-view");
      setPanelRequested(true);
      onPanelOpenChange?.(true);
      if (tab === "map" || tab === "results") {
        onViewOnMap?.();
      }
    },
    [dockMode, onPanelOpenChange, onViewOnMap]
  );

  const handleDrawRectAction = useCallback(() => {
    setPanelTab("map");
    setPanelIntent("rect-draw");
    setPanelRequested(true);
    onPanelOpenChange?.(true);
    onDrawModeChange?.("rect");
    onDrawActiveChange?.(true);
    onDrawRectRequest?.();
  }, [onPanelOpenChange, onDrawModeChange, onDrawActiveChange, onDrawRectRequest]);

  // 2. 桥接与环境识别 (开发模式浏览器无原生桥接时回退为 null，安全进入 isFixtureMode)
  const bridge = useMemo(() => {
    try {
      return getDesktopShellBridge();
    } catch {
      return null;
    }
  }, []);
  const isRealDesktop = isTauriDesktopHost() && Boolean(bridge?.agent);
  const isFixtureMode =
    import.meta.env.DEV && Boolean(fixtureEventsFactory) && (!isRealDesktop || Boolean(forcedSubscene));

const SESSION_DRAFTS_STORAGE_KEY = "eostage.agent.sessionDrafts.v1";

function loadStoredSessionDraft(key: string): string {
  if (typeof window === "undefined" || !window.localStorage) return "";
  try {
    const raw = window.localStorage.getItem(SESSION_DRAFTS_STORAGE_KEY);
    if (!raw) return "";
    const parsed = JSON.parse(raw) as Record<string, unknown>;
    return parsed && typeof parsed[key] === "string" ? (parsed[key] as string) : "";
  } catch {
    return "";
  }
}

function saveStoredSessionDraft(key: string, value: string): void {
  if (typeof window === "undefined" || !window.localStorage) return;
  try {
    const raw = window.localStorage.getItem(SESSION_DRAFTS_STORAGE_KEY);
    const parsed =
      raw && typeof JSON.parse(raw) === "object" && JSON.parse(raw) !== null
        ? (JSON.parse(raw) as Record<string, string>)
        : {};
    if (value) {
      parsed[key] = value;
    } else {
      delete parsed[key];
    }
    window.localStorage.setItem(SESSION_DRAFTS_STORAGE_KEY, JSON.stringify(parsed));
  } catch {
    // ignore storage quota errors
  }
}

  // 3. 会话列表状态（真实模式取自 bridge.agent.sessionsList）
  const [sessions, setSessions] = useState<AgentSessionSummary[]>([]);
  const lastProcessedWorkspaceSeqRef = useRef<number>(0);
  const [liveDelta, setLiveDelta] = useState<{
    runId: string;
    stepIndex: number;
    thinkingStatus: "idle" | "thinking" | "done";
    thinkingText: string;
    startedAtMs: number | null;
    durationMs: number | null;
    assistantText: string;
  } | null>(null);
  const sessionDraftsRef = useRef<Record<string, string>>({});
  const [inputText, setInputTextState] = useState<string>(() =>
    loadStoredSessionDraft(openSessionId || "__new__")
  );
  const [liveThinkingExpanded, setLiveThinkingExpanded] = useState<boolean>(true);

  // 活动会话：响应 openSessionId 切换与「新对话」nonce（各对话独立保存草稿，新建对话开启空白草稿），真正变化时上报上层
  const [activeSessionId, setActiveSessionId, sessionGuard] = useAgentActiveSession({
    openSessionId,
    newChatNonce,
    onActiveSessionChange,
    onOpenSession: () => {
      setPendingSessionMode(null);
      setLiveDelta(null);
    },
    onNewChat: () => {
      cursorRef.current = 0;
      lastProcessedWorkspaceSeqRef.current = 0;
      sessionDraftsRef.current["__new__"] = "";
      saveStoredSessionDraft("__new__", "");
      setInputTextState("");
      setLiveDelta(null);
      setEvents([]);
      setRuns([]);
      setActiveRunId(null);
      setAutoCancelOutcomes({});
    },
  });
  const [_loadingSessions, setLoadingSessions] = useState<boolean>(true);
  const [loadingSessionHistory, setLoadingSessionHistory] = useState<boolean>(false);
  const showSessionHistorySkeleton = useDelayedFlag(loadingSessionHistory);
  const [liveDeltaMinHeight, setLiveDeltaMinHeight] = useState<number>(0);
  const liveDeltaInnerRef = useRef<HTMLDivElement | null>(null);

  // 4. 事件流与游标
  const [events, setEvents] = useState<AgentStreamEvent[]>([]);
  const cursorRef = useRef<number>(0);
  const seenSearchCallIdsRef = useRef<Set<string>>(new Set());

  useEffect(() => {
    const newCallIds = newlySettledSearchCallIds(events, seenSearchCallIdsRef.current);
    if (newCallIds.length === 0) return;
    for (const callId of newCallIds) {
      seenSearchCallIdsRef.current.add(callId);
    }
    if (!onAgentSearchSettled) return;
    for (const callId of newCallIds) {
      const resourceId = extractSearchResourceIdFromEvents(events, callId);
      onAgentSearchSettled({ resourceId });
    }
  }, [events, onAgentSearchSettled]);

  // 新到达的空间/检索/工作区同步事件：幂等自动展开右侧地图面板与同步画布状态
  const pendingSelectSceneRef = useRef<string | null>(null);
  const scenesLatestRef = useRef<SceneRow[]>(scenes);
  scenesLatestRef.current = scenes;

  useEffect(() => {
    if (!pendingSelectSceneRef.current || scenes.length === 0) return;
    const resolved = matchSceneIdFromList(pendingSelectSceneRef.current, scenes);
    if (resolved) {
      pendingSelectSceneRef.current = null;
      onSelectScene?.(resolved);
    }
  }, [scenes, onSelectScene]);

  useEffect(() => {
    if (events.length === 0) return;
    let maxSeq = lastProcessedWorkspaceSeqRef.current;
    for (const ev of events) {
      if (typeof ev.seq !== "number" || ev.seq <= lastProcessedWorkspaceSeqRef.current) {
        continue;
      }
      if (ev.seq > maxSeq) maxSeq = ev.seq;
      const payload = (ev.payload || {}) as Record<string, any>;
      if (ev.kind === "tool_round" || ev.kind === "progress_card") {
        const toolId = String(payload.tool_id || "");
        const status = String(payload.status || "");
        if (
          toolId === "working_aoi_set" ||
          toolId === "search_start" ||
          toolId === "search_progress" ||
          toolId === "map_workspace_action"
        ) {
          openSplitPanel("map");
        }
        if (toolId === "working_aoi_set" && status === "done") {
          void onSyncWorkingAoi?.();
        }
      } else if (ev.kind === "plan_card") {
        if (
          payload.auto_executed === true &&
          Array.isArray(payload.steps) &&
          payload.steps.some(
            (st: any) =>
              st?.tool_id === "working_aoi_set" ||
              st?.tool_id === "search_start"
          )
        ) {
          openSplitPanel("map");
        }
      } else if (ev.kind === "workspace_sync") {
        const action = String(payload.action || "");
        if (action === "open_panel") {
          const rawTab = String(payload.tab || "map");
          const nextTab: PanelTab =
            rawTab === "results" ? "results" : rawTab === "task" || rawTab === "tasks" ? "task" : "map";
          openSplitPanel(nextTab);
        } else if (action === "aoi_changed") {
          openSplitPanel("map");
          void onSyncWorkingAoi?.();
          const rawBbox = payload.bbox;
          if (Array.isArray(rawBbox) && rawBbox.length >= 4) {
            const [west, south, east, north] = rawBbox.map(Number);
            triggerFocusBounds({ west, south, east, north, crs: "EPSG:4326" });
          } else if (rawBbox && typeof rawBbox === "object") {
            const b = rawBbox as Record<string, unknown>;
            triggerFocusBounds({
              west: Number(b.west),
              south: Number(b.south),
              east: Number(b.east),
              north: Number(b.north),
              crs: "EPSG:4326",
            });
          }
        } else if (action === "search_updated") {
          openSplitPanel("map");
          const syncBatchId =
            typeof payload.batch_id === "string" && payload.batch_id.trim()
              ? payload.batch_id.trim()
              : null;
          const syncResourceId =
            typeof payload.resource_id === "string" && payload.resource_id.trim()
              ? payload.resource_id.trim()
              : null;
          const loadedCount =
            typeof payload.loaded_count === "number" ? payload.loaded_count : null;
          const totalCount =
            typeof payload.total_count === "number" ? payload.total_count : null;
          if (totalCount !== null) {
            setHydratedBatchTotalCount(totalCount);
          }
          if (onAgentSessionBatchSettled) {
            void onAgentSessionBatchSettled({
              sessionId: activeSessionId || null,
              batchId: syncBatchId,
              resourceId: syncResourceId,
              loadedCount,
              totalCount,
              selectThisBatch: true,
            });
          } else if (onAgentSearchSettled) {
            onAgentSearchSettled({ resourceId: syncResourceId });
          }
        } else if (action === "fit_bounds") {
          openSplitPanel("map");
          const rawBbox = payload.bbox;
          if (Array.isArray(rawBbox) && rawBbox.length >= 4) {
            const [west, south, east, north] = rawBbox.map(Number);
            triggerFocusBounds({ west, south, east, north, crs: "EPSG:4326" });
          } else if (payload.target === "aoi") {
            void onSyncWorkingAoi?.();
            if (workbenchAoi) triggerFocusBounds(workbenchAoi);
          }
        } else if (action === "select_scene") {
          openSplitPanel("map");
          if (typeof payload.scene_id === "string" && payload.scene_id.trim().length > 0) {
            const rawScene = payload.scene_id.trim();
            const matched = matchSceneIdFromList(rawScene, scenesLatestRef.current);
            if (matched) {
              pendingSelectSceneRef.current = null;
              onSelectScene?.(matched);
            } else {
              pendingSelectSceneRef.current = rawScene;
              const targetResource = inferResourceIdFromSceneId(rawScene);
              if (targetResource && targetResource !== sourceId && onAgentSearchSettled) {
                onAgentSearchSettled({ resourceId: targetResource });
              } else {
                onSelectScene?.(rawScene);
              }
            }
          }
        } else if (action === "toggle_preview") {
          openSplitPanel("map");
          if (typeof payload.enabled === "boolean") {
            setMapPreviewEnabled(payload.enabled);
          }
        } else if (action === "set_basemap") {
          openSplitPanel("map");
          if (typeof payload.layer_key === "string" && payload.layer_key.trim().length > 0) {
            const validKey = normalizeAgentBasemapKey(payload.layer_key);
            if (validKey) {
              onMapLayerChange?.(validKey);
            }
          }
        }
      }
    }
    lastProcessedWorkspaceSeqRef.current = maxSeq;
  }, [
    activeSessionId,
    events,
    onAgentSearchSettled,
    onAgentSessionBatchSettled,
    onMapLayerChange,
    onSelectScene,
    onSyncWorkingAoi,
    openSplitPanel,
    setMapPreviewEnabled,
    sourceId,
    triggerFocusBounds,
    workbenchAoi,
  ]);

  const autoHydratedHistoryBatchesRef = useRef<Set<string>>(new Set());

  useEffect(() => {
    setHydratedBatchScenes([]);
    setHydratedBatchTotalCount(null);
    setLocalHydratedBatches([]);
    setLocalBatchFilter("all");
    setDockMode("collapsed");
    autoHydratedHistoryBatchesRef.current.clear();
  }, [activeSessionId]);

  // 历史多源会话自动补齐：扫描当前会话事件流中的全部非空检索批次（如 Sentinel-1 + Sentinel-2 + NISAR + Landsat），确保打开旧会话也能恢复全部数据源
  useEffect(() => {
    if (!activeSessionId || isFixtureMode || events.length === 0) return;
    const discovered = extractSessionBatchesFromEvents(events);
    if (discovered.length === 0) return;
    const knownIds = new Set(effectiveBatchCollection.batches.map((b) => b.batchId));
    for (const item of discovered) {
      const key = `${activeSessionId}:${item.batchId}`;
      if (knownIds.has(item.batchId) || autoHydratedHistoryBatchesRef.current.has(key)) {
        continue;
      }
      autoHydratedHistoryBatchesRef.current.add(key);
      if (onAgentSessionBatchSettled) {
        void onAgentSessionBatchSettled({
          sessionId: activeSessionId,
          batchId: item.batchId,
          resourceId: item.resourceId,
          loadedCount: null,
          totalCount: item.totalCount,
          selectThisBatch: false,
        });
      } else {
        void getBatchPage(item.batchId, 0, 0, 250)
          .then((page) => {
            if (page.scenes.length === 0 && (page.total_count ?? 0) === 0) return;
            const nextRows = new Map<string, RowTaskState>();
            for (const r of page.rows) nextRows.set(r.item_id, r);
            setLocalHydratedBatches((prev) => {
              if (prev.some((b) => b.batchId === item.batchId)) return prev;
              return [
                ...prev,
                {
                  batchId: item.batchId,
                  resourceId: item.resourceId,
                  revision: 0,
                  scenes: page.scenes,
                  rowStates: nextRows,
                  totalCount: Math.max(page.total_count ?? 0, page.scenes.length),
                  updatedAtMs: Date.now(),
                },
              ];
            });
          })
          .catch(() => {});
      }
    }
  }, [
    activeSessionId,
    effectiveBatchCollection.batches,
    events,
    isFixtureMode,
    onAgentSessionBatchSettled,
  ]);

  const fitScenesOrAoiBounds = useCallback(
    (poolScenes: readonly SceneRow[]) => {
      const bboxPool = poolScenes
        .map((s) => s.footprint_bbox)
        .filter((b): b is Bbox =>
          Boolean(b && [b.west, b.south, b.east, b.north].every(Number.isFinite))
        );
      if (bboxPool.length > 0) {
        let west = Infinity;
        let south = Infinity;
        let east = -Infinity;
        let north = -Infinity;
        for (const b of bboxPool) {
          west = Math.min(west, b.west);
          south = Math.min(south, b.south);
          east = Math.max(east, b.east);
          north = Math.max(north, b.north);
        }
        triggerFocusBounds({ west, south, east, north, crs: "EPSG:4326" });
        return;
      }
      if (workbenchAoi) {
        triggerFocusBounds(workbenchAoi);
      }
    },
    [triggerFocusBounds, workbenchAoi]
  );

  const handleSelectDatasetFilter = useCallback(
    (nextFilter: "all" | string) => {
      if (onSelectBatchFilter) {
        onSelectBatchFilter(nextFilter);
      } else {
        setLocalBatchFilter(nextFilter);
      }
      const nextProjected = projectAgentSessionBatchCollection(effectiveRawBatches, nextFilter);
      if (nextProjected.scenes.length > 0) {
        fitScenesOrAoiBounds(nextProjected.scenes);
      }
    },
    [effectiveRawBatches, fitScenesOrAoiBounds, onSelectBatchFilter]
  );

  const [expandedTurns, setExpandedTurns] = useState<Record<string, boolean>>({});
  const toggleTurnExpand = useCallback((turnKey: string, currentExpanded: boolean) => {
    setExpandedTurns((prev) => ({
      ...prev,
      [turnKey]: !currentExpanded,
    }));
  }, []);

  const handleFocusScenesOnMap = useCallback(
    async (opts?: { itemIds?: string[]; batchId?: string | null; bbox?: Bbox | null }) => {
      openSplitPanel("map");
      const targetIds = Array.isArray(opts?.itemIds)
        ? opts!.itemIds.filter((id): id is string => typeof id === "string" && id.trim().length > 0)
        : [];
      if (targetIds.length === 0) {
        // 查看整批结果时自动在地图下方展开同屏结果清单，消除切页割裂
        setDockMode("split");
      }

      if (opts?.batchId) {
        const known = effectiveBatchCollection.batches.find((b) => b.batchId === opts.batchId);
        if (known) {
          handleSelectDatasetFilter(opts.batchId);
          return;
        }
      }

      let candidateScenes =
        effectiveScenes.length > 0
          ? effectiveScenes
          : scenesLatestRef.current.length > 0
            ? scenesLatestRef.current
            : hydratedBatchScenes;
      let resolvedBatchId = opts?.batchId ?? effectiveActiveBatchId ?? null;
      let resolvedResourceId: string | null = effectiveActiveResourceId;
      if (!resolvedBatchId) {
        for (let i = events.length - 1; i >= 0; i--) {
          const ev = events[i];
          const p = (ev.payload || {}) as Record<string, any>;
          if (typeof p.batch_id === "string" && p.batch_id.trim()) {
            resolvedBatchId = p.batch_id.trim();
            if (typeof p.resource_id === "string" && p.resource_id.trim()) {
              resolvedResourceId = p.resource_id.trim();
            }
            break;
          }
          const snap = p.snapshot as Record<string, any> | undefined;
          if (snap && typeof snap.batch_id === "string" && snap.batch_id.trim()) {
            resolvedBatchId = snap.batch_id.trim();
            break;
          }
        }
      } else {
        for (let i = events.length - 1; i >= 0; i--) {
          const p = (events[i].payload || {}) as Record<string, any>;
          if (p.batch_id === resolvedBatchId && typeof p.resource_id === "string" && p.resource_id.trim()) {
            resolvedResourceId = p.resource_id.trim();
            break;
          }
        }
      }

      const switchingBatch = Boolean(opts?.batchId && opts.batchId !== effectiveActiveBatchId);
      if ((candidateScenes.length === 0 || switchingBatch) && resolvedBatchId && !isFixtureMode) {
        try {
          const page = await getBatchPage(resolvedBatchId, 0, 0, 250);
          if (page.scenes.length > 0) {
            candidateScenes = page.scenes;
            setHydratedBatchScenes(page.scenes);
            setHydratedBatchTotalCount(page.total_count ?? page.scenes.length);
            void onAgentSessionBatchSettled?.({
              sessionId: activeSessionId || null,
              batchId: resolvedBatchId,
              resourceId: resolvedResourceId,
              loadedCount: page.scenes.length,
              totalCount: page.total_count ?? page.scenes.length,
              selectThisBatch: Boolean(opts?.batchId),
            });
          }
        } catch {
          // ignore batch hydrate error and fall back to AOI bounds
        }
      }

      const matchedScenes: SceneRow[] = [];
      for (const rawId of targetIds) {
        const resolvedId = matchSceneIdFromList(rawId, candidateScenes);
        const found = resolvedId
          ? candidateScenes.find((s) => s.scene_id === resolvedId)
          : candidateScenes.find((s) => s.scene_id === rawId || s.product_key === rawId);
        if (found) matchedScenes.push(found);
      }
      if (matchedScenes.length > 0) {
        onSelectScene?.(matchedScenes[0].scene_id);
      }

      const bboxPool = (matchedScenes.length > 0 ? matchedScenes : candidateScenes)
        .map((s) => s.footprint_bbox)
        .filter((b): b is Bbox => Boolean(b && [b.west, b.south, b.east, b.north].every(Number.isFinite)));

      if (bboxPool.length > 0) {
        let west = Infinity;
        let south = Infinity;
        let east = -Infinity;
        let north = -Infinity;
        for (const b of bboxPool) {
          west = Math.min(west, b.west);
          south = Math.min(south, b.south);
          east = Math.max(east, b.east);
          north = Math.max(north, b.north);
        }
        triggerFocusBounds({ west, south, east, north, crs: "EPSG:4326" });
        return;
      }
      if (opts?.bbox) {
        triggerFocusBounds(opts.bbox);
        return;
      }
      if (workbenchAoi) {
        triggerFocusBounds(workbenchAoi);
        return;
      }
      void onSyncWorkingAoi?.();
    },
    [
      activeSessionId,
      effectiveActiveBatchId,
      effectiveActiveResourceId,
      effectiveBatchCollection.batches,
      effectiveScenes,
      events,
      handleSelectDatasetFilter,
      hydratedBatchScenes,
      isFixtureMode,
      onAgentSessionBatchSettled,
      onSelectScene,
      onSyncWorkingAoi,
      openSplitPanel,
      triggerFocusBounds,
      workbenchAoi,
    ]
  );

  // 5. 运行记录列表（Runs）与活动 Run
  const [runs, setRuns] = useState<AgentRunSummary[]>([]);
  const [activeRunId, setActiveRunId] = useState<string | null>(null);
  const [answeredQuestionByKey, setAnsweredQuestionByKey] = useState<
    Record<string, { value: string; values?: string[]; label: string; passive: boolean }>
  >({});
  const [lastPromptByRunId, setLastPromptByRunId] = useState<Record<string, number>>({});
  const [fastPathByRunId, setFastPathByRunId] = useState<Record<string, string | null>>({});
  const fetchedLastPromptRunsRef = useRef<Set<string>>(new Set());

  // 每次切换会话时清空本地会话内问题卡片的临时选择映射（历史消息会从流中反推）
  useEffect(() => {
    setAnsweredQuestionByKey({});
  }, [activeSessionId]);

  // 每次官方/Agent 对话轮次结束时，即时及延迟 1.5s 触发侧边栏 Credits 余额刷新
  const prevActiveRunIdRef = useRef<string | null>(null);
  useEffect(() => {
    const prev = prevActiveRunIdRef.current;
    prevActiveRunIdRef.current = activeRunId;
    if (prev && !activeRunId) {
      notifyEOStageAccountChanged();
      const timer = window.setTimeout(() => {
        notifyEOStageAccountChanged();
      }, 1500);
      return () => window.clearTimeout(timer);
    }
    return undefined;
  }, [activeRunId]);

  const latestUsageRun = useMemo(() => pickLatestUsageRun(runs), [runs]);
  const latestUsageRunId = latestUsageRun?.run_id ?? null;
  const latestUsageRoute = latestUsageRun?.model_route ?? null;

  useEffect(() => {
    if (!latestUsageRunId || isFixtureMode || !bridge?.agent?.runsOpen) return;
    if (fetchedLastPromptRunsRef.current.has(latestUsageRunId)) return;
    fetchedLastPromptRunsRef.current.add(latestUsageRunId);
    let cancelled = false;
    void bridge.agent
      .runsOpen(latestUsageRunId)
      .then((detail) => {
        if (cancelled || !detail || !Array.isArray((detail as { events?: unknown }).events)) return;
        const evs = (
          detail as { events: Array<{ kind?: string; payload?: Record<string, unknown> }> }
        ).events;
        let lastPrompt: number | null = null;
        for (let i = evs.length - 1; i >= 0; i--) {
          if (evs[i]?.kind !== "model_reply") continue;
          const u = evs[i]?.payload?.usage as
            | {
                prompt?: unknown;
                input_tokens?: unknown;
                cached?: unknown;
                cached_tokens?: unknown;
                cached_input_tokens?: unknown;
                cache_read_input_tokens?: unknown;
                prompt_cache_hit_tokens?: unknown;
                cache_creation?: unknown;
                cache_creation_input_tokens?: unknown;
              }
            | undefined;
          if (!u || typeof u !== "object") continue;
          const rawP =
            typeof u.prompt === "number" && Number.isFinite(u.prompt) && u.prompt > 0
              ? u.prompt
              : typeof u.input_tokens === "number" &&
                  Number.isFinite(u.input_tokens) &&
                  u.input_tokens > 0
                ? u.input_tokens
                : 0;
          const rawCached =
            typeof u.cached === "number" && Number.isFinite(u.cached) && u.cached >= 0
              ? u.cached
              : typeof u.cached_tokens === "number" &&
                  Number.isFinite(u.cached_tokens) &&
                  u.cached_tokens >= 0
                ? u.cached_tokens
                : typeof u.cached_input_tokens === "number" &&
                    Number.isFinite(u.cached_input_tokens) &&
                    u.cached_input_tokens >= 0
                  ? u.cached_input_tokens
                  : typeof u.cache_read_input_tokens === "number" &&
                      Number.isFinite(u.cache_read_input_tokens) &&
                      u.cache_read_input_tokens >= 0
                    ? u.cache_read_input_tokens
                    : typeof u.prompt_cache_hit_tokens === "number" &&
                        Number.isFinite(u.prompt_cache_hit_tokens) &&
                        u.prompt_cache_hit_tokens >= 0
                      ? u.prompt_cache_hit_tokens
                      : 0;
          const rawCreation =
            typeof u.cache_creation === "number" &&
            Number.isFinite(u.cache_creation) &&
            u.cache_creation >= 0
              ? u.cache_creation
              : typeof u.cache_creation_input_tokens === "number" &&
                  Number.isFinite(u.cache_creation_input_tokens) &&
                  u.cache_creation_input_tokens >= 0
                ? u.cache_creation_input_tokens
                : 0;
          const route = (latestUsageRoute ?? "").toLowerCase();
          const isAnth = rawCreation > 0 || route.startsWith("anthropic");
          const effectivePrompt =
            isAnth || rawCached > rawP
              ? rawP + rawCached + rawCreation
              : rawP + rawCreation;
          if (effectivePrompt > 0) {
            lastPrompt = effectivePrompt;
            break;
          }
        }
        let fastPath: string | null = null;
        for (let i = evs.length - 1; i >= 0; i--) {
          if (evs[i]?.kind !== "request") continue;
          fastPath =
            typeof evs[i]?.payload?.fast_path === "string"
              ? (evs[i]?.payload?.fast_path as string)
              : null;
          break;
        }
        if (lastPrompt !== null) {
          setLastPromptByRunId((prev) =>
            prev[latestUsageRunId] === lastPrompt
              ? prev
              : { ...prev, [latestUsageRunId]: lastPrompt }
          );
        }
        setFastPathByRunId((prev) =>
          prev[latestUsageRunId] === fastPath && latestUsageRunId in prev
            ? prev
            : { ...prev, [latestUsageRunId]: fastPath }
        );
      })
      .catch(() => {
        fetchedLastPromptRunsRef.current.delete(latestUsageRunId);
      });
    return () => {
      cancelled = true;
    };
  }, [latestUsageRunId, latestUsageRoute, isFixtureMode, bridge]);

  const calibratedContextRuns = useMemo(
    () =>
      runs.map((r) => {
        const lastPrompt = lastPromptByRunId[r.run_id];
        const hasLastPrompt = typeof lastPrompt === "number" && Boolean(r.usage_total);
        const hasFastPath = Object.prototype.hasOwnProperty.call(fastPathByRunId, r.run_id);
        if (!hasLastPrompt && !hasFastPath) return r;
        return {
          ...r,
          ...(hasLastPrompt && r.usage_total
            ? { usage_total: { ...r.usage_total, last_prompt: lastPrompt } }
            : {}),
          ...(hasFastPath ? { fast_path: fastPathByRunId[r.run_id] } : {}),
        };
      }),
    [runs, lastPromptByRunId, fastPathByRunId]
  );

  const hasActiveLiveDelta = Boolean(
    activeRunId &&
      liveDelta &&
      liveDelta.runId === activeRunId &&
      (liveDelta.thinkingStatus === "thinking" ||
        liveDelta.thinkingText.trim().length > 0 ||
        liveDelta.assistantText.trim().length > 0)
  );

  useEffect(() => {
    setLiveDeltaMinHeight(0);
    setLiveThinkingExpanded(true);
  }, [activeRunId, activeSessionId]);

  // 5a. 本对话的工作（任务与成果）：会话切换、运行列表更新（运行结束）与任务轮询时刷新
  const [sessionWork, setSessionWork] = useState<AgentWorkItem[] | null>(null);
  const [showOnboardingCallout, setShowOnboardingCallout] = useState<boolean>(() => {
    try {
      return typeof window !== "undefined"
        ? window.localStorage.getItem("eostage.onboarding.calloutDismissed") !== "1"
        : true;
    } catch {
      return true;
    }
  });
  const [dontNotifyAgain, setDontNotifyAgain] = useState<boolean>(false);
  const [missingDataAccounts, setMissingDataAccounts] = useState<string[]>([]);
  const [credentialStatusLoaded, setCredentialStatusLoaded] = useState<boolean>(false);

  const refreshDataAccountReadiness = useCallback(async () => {
    try {
      const status = await getCredentialStatus();
      const isConfigured = (val?: string | null) =>
        Boolean(val && val !== "none" && val !== "unavailable");
      const missing: string[] = [];
      if (!isConfigured(status.cdse)) missing.push("Copernicus CDSE");
      if (!isConfigured(status.earthdata)) missing.push("NASA Earthdata");
      if (!isConfigured(status.usgs)) missing.push("USGS");
      setMissingDataAccounts(missing);
    } catch {
      // 保守降级：查询异常时不误报缺失
      setMissingDataAccounts([]);
    } finally {
      setCredentialStatusLoaded(true);
    }
  }, []);

  useEffect(() => {
    void refreshDataAccountReadiness();
    if (typeof window === "undefined") return;
    const onFocus = () => {
      void refreshDataAccountReadiness();
    };
    window.addEventListener("focus", onFocus);
    return () => {
      window.removeEventListener("focus", onFocus);
    };
  }, [refreshDataAccountReadiness]);

  const dismissOnboardingCallout = useCallback(
    (persist: boolean) => {
      setShowOnboardingCallout(false);
      if (persist) {
        try {
          if (typeof window !== "undefined") {
            window.localStorage.setItem("eostage.onboarding.calloutDismissed", "1");
          }
        } catch {
          // ignore
        }
      }
    },
    []
  );
  const sessionTaskCount = createdTaskCount(sessionWork);
  useEffect(() => {
    const agent = bridge?.agent;
    if (!activeSessionId) {
      setSessionWork(null);
      return;
    }
    if (isFixtureMode) {
      if (runs.length === 0) {
        setSessionWork(null);
      } else {
        setSessionWork(
          runs.map((run, idx) => ({
            run,
            request:
              idx === 0
                ? "检索大同盆地试验区 2024 年 4 月的 Sentinel-1 升轨数据并准备下载"
                : "补充检索茂县叠溪镇 Sentinel-1 SLC 干涉数据",
            plan_ids: idx === 0 ? ["fixture-plan-01"] : [],
            created_task_ids: idx === 0 && agentTasks.length > 0 ? [agentTasks[0].task_id] : [],
          }))
        );
      }
      return;
    }
    if (!agent?.sessionWork) {
      setSessionWork(null);
      return;
    }
    let stale = false;
    agent.sessionWork(activeSessionId).then(
      (items) => { if (!stale) setSessionWork(items); },
      () => { /* 保留上次结果 */ },
    );
    return () => { stale = true; };
  }, [activeSessionId, bridge?.agent, isFixtureMode, runs, agentTasks]);

  // 5b. 自动执行取消结果文案映射（plan_id -> outcomeText）
  const [autoCancelOutcomes, setAutoCancelOutcomes] = useState<
    Record<string, string>
  >({});
  useEffect(() => {
    setAutoCancelOutcomes({});
  }, [activeSessionId]);

  // 上报当前活动运行与 Agent 建任务
  useEffect(() => {
    if (!onAgentActivityChange) return;
    const liveRunIds = activeRunId ? [activeRunId] : [];
    const agentTaskIds = agentCreatedTaskIds(events);
    onAgentActivityChange({ liveRunIds, agentTaskIds });
  }, [activeRunId, events, onAgentActivityChange]);

  // 通知上层重读活动状态 (C1 规则 12)
  useAgentActivityNotifier({
    sessionId: activeSessionId,
    activeRunId,
    events,
    onActivityChanged,
  });

  // 新收到的 stop_all 事件上报给上层
  const lastProcessedStopAllSeqRef = useRef<number>(-1);
  useEffect(() => {
    lastProcessedStopAllSeqRef.current = -1;
  }, [activeSessionId]);

  useEffect(() => {
    if (!onStopAllEvent || !activeSessionId) return;
    for (const ev of events) {
      if (ev.kind === "stop_all" && ev.seq > lastProcessedStopAllSeqRef.current) {
        lastProcessedStopAllSeqRef.current = ev.seq;
        onStopAllEvent(
          activeSessionId,
          ev.seq,
          ev.payload as unknown as AgentStopAllResult
        );
      }
    }
  }, [events, activeSessionId, onStopAllEvent]);
  const [channels, setChannels] = useState<AgentChannelView[]>([]);
  const [channelReadiness, setChannelReadiness] = useState<ChannelReadinessResult>({
    ready: false,
    disabled: true,
    reason_zh: t("agent.channels.checking"),
  });

  // 7. 矢量文件导入忙态与错误状态
  const [vectorBusy, setVectorBusy] = useState<boolean>(false);
  const [vectorError, setVectorError] = useState<string | null>(null);
  const [pendingVectorPreview, setPendingVectorPreview] = useState<{
    fileName: string;
    preview: AoiPreviewOk;
  } | null>(null);

  // 8. 折叠组与卡片状态（联动独立设置入口「工作步骤显示」：stepDetailMode + collapseTiming + foldedReplies）
  const [workStepsPrefs, setWorkStepsPrefsState] = useState<WorkStepsPrefs>(
    () => getWorkStepsPrefs()
  );
  const { stepDetailMode, collapseTiming, foldedReplies } = workStepsPrefs;
  useEffect(() => {
    return subscribeWorkStepsPrefs((prefs) => {
      setWorkStepsPrefsState((prev) => {
        if (
          prev.stepDetailMode !== prefs.stepDetailMode ||
          prev.collapseTiming !== prefs.collapseTiming
        ) {
          setExpandedGroups({});
        }
        return prefs;
      });
    });
  }, []);

  const [expandedGroups, setExpandedGroups] = useState<Record<string, boolean>>({});
  const toggleGroupExpand = (groupKey: string, currentExpanded?: boolean) => {
    setExpandedGroups((prev) => ({
      ...prev,
      [groupKey]: currentExpanded !== undefined ? !currentExpanded : !prev[groupKey],
    }));
  };

  const [expandedEvidences, setExpandedEvidences] = useState<Record<string, boolean>>({});
  const toggleEvidenceExpand = (runId: string) => {
    setExpandedEvidences((prev) => ({
      ...prev,
      [runId]: !prev[runId],
    }));
  };

  // 9. 输入框状态（按会话隔离独立草稿）、排队/立即引导模式与输入法合成防护 (基线 4b-1)
  const activeDraftKey = activeSessionId || "__new__";
  const setInputText = useCallback(
    (updater: string | ((prev: string) => string)) => {
      setInputTextState((prev) => {
        const next = typeof updater === "function" ? updater(prev) : updater;
        sessionDraftsRef.current[activeDraftKey] = next;
        saveStoredSessionDraft(activeDraftKey, next);
        return next;
      });
    },
    [activeDraftKey]
  );
  const modelSelectionRef = useRef<AgentModelSelection | null>(readStoredModelSelection());
  const [modelSelectionVersion, setModelSelectionVersion] = useState(0);
  const handleModelSelectionChange = useCallback(
    (selection: AgentModelSelection | null) => {
      modelSelectionRef.current = selection;
      const key = activeSessionId || "__global__";
      sessionModelSelections[key] = selection;
      sessionModelSelections["__global__"] = selection;
      setModelSelectionVersion((v) => v + 1);
    },
    [activeSessionId]
  );
  const [composerAttachments, setComposerAttachments] = useState<ComposerAttachment[]>([]);
  const [attachmentError, setAttachmentError] = useState<string | null>(null);
  const composerAttachmentsRef = useRef<ComposerAttachment[]>([]);
  composerAttachmentsRef.current = composerAttachments;

  const handleAddComposerAttachments = useCallback(async (files: File[]) => {
    if (files.length === 0) return;
    setAttachmentError(null);
    const currentCount = composerAttachmentsRef.current.length;
    const remainingSlots = Math.max(0, MAX_COMPOSER_ATTACHMENTS - currentCount);
    if (remainingSlots <= 0) {
      setAttachmentError(
        t("agent.composer.maxAttachmentsError", { max: MAX_COMPOSER_ATTACHMENTS })
      );
      return;
    }
    let firstError: string | null =
      files.length > remainingSlots
        ? t("agent.composer.maxAttachmentsTruncatedError", {
            max: MAX_COMPOSER_ATTACHMENTS,
            count: remainingSlots,
          })
        : null;
    const added: ComposerAttachment[] = [];
    for (const file of files.slice(0, remainingSlots)) {
      const res = await readFileAsComposerAttachment(file);
      if (res.attachment) {
        added.push(res.attachment);
      } else if (res.error && !firstError) {
        firstError = res.error;
      }
    }
    if (added.length > 0) {
      setComposerAttachments((prev) =>
        [...prev, ...added].slice(0, MAX_COMPOSER_ATTACHMENTS)
      );
    }
    if (firstError) {
      setAttachmentError(firstError);
    }
  }, [t]);

  const handleRemoveComposerAttachment = useCallback((id: string) => {
    setComposerAttachments((prev) => prev.filter((item) => item.id !== id));
    setAttachmentError(null);
  }, []);

  const [busySendMode, setBusySendModeState] = useState<BusySendMode>(
    () => getBusySendPrefs().busySendMode
  );
  useEffect(() => {
    return subscribeBusySendPrefs((prefs) => {
      setBusySendModeState(prefs.busySendMode);
    });
  }, []);
  const [queuedMessages, setQueuedMessages] = useState<QueuedComposerMessage[]>([]);
  const [isSteering, setIsSteering] = useState(false);
  const pendingSteerTextRef = useRef<string | null>(null);
  useEffect(() => {
    setQueuedMessages([]);
    setIsSteering(false);
    pendingSteerTextRef.current = null;
    setAttachmentError(null);
    const nextDraftKey = activeSessionId || "__new__";
    const rememberedDraft = Object.prototype.hasOwnProperty.call(
      sessionDraftsRef.current,
      nextDraftKey
    )
      ? sessionDraftsRef.current[nextDraftKey]
      : loadStoredSessionDraft(nextDraftKey);
    sessionDraftsRef.current[nextDraftKey] = rememberedDraft;
    setInputTextState(rememberedDraft);
    const sessionKey = activeSessionId || "__global__";
    modelSelectionRef.current = Object.prototype.hasOwnProperty.call(
      sessionModelSelections,
      sessionKey
    )
      ? sessionModelSelections[sessionKey]
      : readStoredModelSelection();
  }, [activeSessionId]);

  // 10. 滚动跟随与跳至最新 (基线 4b-2)
  const scrollContainerRef = useRef<HTMLDivElement | null>(null);
  const [shouldAutoScroll, setShouldAutoScroll] = useState<boolean>(true);
  const [showScrollBottomBtn, setShowScrollBottomBtn] = useState<boolean>(false);

  // 11. 临时状态提示与错误
  const [actionError, setActionError] = useState<string | null>(null);
  const [demoNotice, setDemoNotice] = useState<string | null>(null);

  // 刷新渠道就绪状态
  const checkChannels = useCallback(async () => {
    if (forcedSubscene === "no-channel") {
      setChannels([]);
      setChannelReadiness({
        ready: false,
        disabled: true,
        reason_zh: t("agent.channels.unconfigured"),
      });
      return;
    }
    if (!bridge?.agent) {
      if (isFixtureMode && fixtureChannelsFactory) {
        const fixtureChannels = fixtureChannelsFactory();
        setChannels(fixtureChannels);
        setChannelReadiness(channelReadinessFromViews(fixtureChannels));
      } else {
        setChannels([]);
        setChannelReadiness({
          ready: false,
          disabled: true,
          reason_zh: t("agent.channels.unavailable"),
        });
      }
      return;
    }
    try {
      // 底层对应 agent_channels_get
      const views = await bridge.agent.channelsGet();
      setChannels(views);
      setChannelReadiness(channelReadinessFromViews(views));
    } catch {
      setChannels([]);
      setChannelReadiness({
        ready: false,
        disabled: true,
        reason_zh: t("agent.channels.loadFailed"),
      });
    }
  }, [bridge?.agent, forcedSubscene, isFixtureMode]);

  useEffect(() => {
    checkChannels();
  }, [checkChannels]);

  useEffect(() => {
    return subscribeChannelsChanged(() => {
      void checkChannels();
    });
  }, [checkChannels]);

  // 刷新会话列表
  const refreshSessions = useCallback(
    async (selectId?: string) => {
      const targetSelectId = selectId || openSessionId || null;
      if (!bridge?.agent) {
        if (isFixtureMode && fixtureSessionsFactory) {
          const fixtureSessions = fixtureSessionsFactory();
          setSessions(fixtureSessions);
          if (targetSelectId) {
            setActiveSessionId(targetSelectId);
          } else if (forcedSubscene) {
            setActiveSessionId(fixtureSessions[0]?.session_id || "");
          } else {
            setActiveSessionId((prev) => (openSessionId || prev ? (openSessionId || prev) : ""));
          }
          setLoadingSessions(false);
        } else {
          setSessions([]);
          setActiveSessionId((prev) => (openSessionId || prev ? (openSessionId || prev) : ""));
          setLoadingSessions(false);
        }
        return;
      }
      const token = sessionGuard.capture();
      try {
        setLoadingSessions(true);
        // 底层对应 agent_sessions_list
        const list = await bridge.agent.sessionsList();
        setSessions(list);
        // 请求期间点过「新对话」：不再选中旧会话
        if (targetSelectId && sessionGuard.noNewChatSince(token)) {
          setActiveSessionId(targetSelectId);
        }
      } catch {
        setActionError(t("agent.workbench.sessionsLoadFailed"));
      } finally {
        setLoadingSessions(false);
      }
    },
    [bridge?.agent, forcedSubscene, isFixtureMode, openSessionId, sessionGuard]
  );

  useEffect(() => {
    refreshSessions(openSessionId || undefined);
  }, [refreshSessions, openSessionId]);

  // 侧栏/历史页重命名、移动等整理操作后同步会话标题
  useEffect(() => subscribeSessionsChanged(() => void refreshSessions()), [refreshSessions]);

  // 渠道切换处理
  const handleChannelSelect = useCallback(
    async (id: string) => {
      if (onAgentChannelSelect) {
        onAgentChannelSelect(id);
      }
      if (isRealDesktop && bridge?.agent) {
        try {
          const updatedViews = await bridge.agent.channelsSetDefault(id);
          setChannels(updatedViews);
          setChannelReadiness(channelReadinessFromViews(updatedViews));
        } catch (err: any) {
          setActionError(explainActionError(err, t("agent.channels.switchFailed")));
        }
      } else if (isFixtureMode) {
        setChannels((prev) =>
          prev.map((c) => ({
            ...c,
            is_default: c.id === id,
          }))
        );
        setDemoNotice(t("agent.workbench.demoChannelChanged"));
      }
    },
    [bridge?.agent, isFixtureMode, isRealDesktop, onAgentChannelSelect]
  );

  // 矢量文件导入处理
  const readFileAsBase64 = (file: File): Promise<string> => {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => {
        const result = reader.result;
        if (typeof result === "string") {
          const base64 = extractBase64FromDataUrl(result);
          resolve(base64);
        } else {
          reject(new Error(t("agent.workbench.fileReadFailed")));
        }
      };
      reader.onerror = () => reject(reader.error || new Error(t("agent.workbench.fileReadFailed")));
      reader.readAsDataURL(file);
    });
  };

  const handleVectorFile = useCallback(
    async (file: File) => {
      setVectorBusy(true);
      setVectorError(null);
      try {
        const base64Data = await readFileAsBase64(file);
        const preview = await previewAoiFileBytes(file.name, base64Data);
        if (!preview.ok) {
          setVectorError(preview.error || t("agent.workbench.vectorPreviewFailed"));
          return;
        }
        if (!preview.geojson) {
          setVectorError(t("agent.workbench.noGeometry"));
          return;
        }
        if (preview.features && preview.features.length > 1) {
          setPendingVectorPreview({ fileName: file.name, preview });
          return;
        }
        const featureIds = preview.features
          ? preview.features.map(
              (f) => f.id || (f.properties?.id as string) || String(f.index ?? 0)
            )
          : [];
        const displayName = vectorAoiDisplayName(file.name) || t("agent.workbench.vectorSelection");
        const result = await setWorkingAoiGeojsonFeatures(
          preview.geojson,
          featureIds,
          preview.display_field || "",
          "merge",
          displayName
        );
        if (!result.ok) {
          setVectorError(result.error || t("agent.workbench.vectorNormalizeFailed"));
          return;
        }
        if (
          result.bbox &&
          Array.isArray(result.bbox) &&
          result.bbox.length === 4
        ) {
          const [west, south, east, north] = result.bbox;
          const crs = (result.crs as any) || "EPSG:4326";
          onUpdateWorkbenchAoi?.({ west, south, east, north, crs }, displayName);
        }
      } catch (err) {
        setVectorError(
          explainActionError(err, t("agent.workbench.vectorUnknownError"))
        );
      } finally {
        setVectorBusy(false);
      }
    },
    [onUpdateWorkbenchAoi]
  );

  const handleConfirmVectorPicker = useCallback(
    async (params: {
      featureIds: string[];
      displayField: string;
      displayName: string;
    }) => {
      if (!pendingVectorPreview || !pendingVectorPreview.preview.geojson) return;
      setVectorBusy(true);
      setVectorError(null);
      try {
        const result = await setWorkingAoiGeojsonFeatures(
          pendingVectorPreview.preview.geojson,
          params.featureIds,
          params.displayField,
          "merge",
          params.displayName
        );
        if (!result.ok) {
          setVectorError(result.error || t("agent.workbench.vectorNormalizeFailed"));
          return;
        }
        if (
          result.bbox &&
          Array.isArray(result.bbox) &&
          result.bbox.length === 4
        ) {
          const [west, south, east, north] = result.bbox;
          const crs = (result.crs as any) || "EPSG:4326";
          onUpdateWorkbenchAoi?.({ west, south, east, north, crs }, params.displayName);
        }
        setPendingVectorPreview(null);
      } catch (err) {
        setVectorError(
          explainActionError(err, t("agent.workbench.vectorUnknownError"))
        );
      } finally {
        setVectorBusy(false);
      }
    },
    [pendingVectorPreview, onUpdateWorkbenchAoi]
  );

  // 权限模式状态
  const [permissionView, setPermissionView] = useState<AgentPermissionModeView | null>(null);
  const [permissionStatus, setPermissionStatus] = useState<"loading" | "ready" | "error">("loading");
  const [pendingSessionMode, setPendingSessionMode] = useState<AgentPermissionMode | null>(null);
  const permissionReqSeqRef = useRef(0);

  // 只要 pendingSessionMode 非 null，选择器就展示待用模式
  const displayedPermissionViewValue = useMemo(() => {
    return displayedPermissionView(permissionView, pendingSessionMode);
  }, [permissionView, pendingSessionMode]);

  // 拉取权限模式视图
  useEffect(() => {
    let cancelled = false;
    const loadPermission = async () => {
      if (isFixtureMode) {
        if (fixturePermissionViewFactory) {
          setPermissionView(fixturePermissionViewFactory());
        }
        setPermissionStatus("ready");
        return;
      }
      if (!bridge?.agent) {
        setPermissionStatus("error");
        return;
      }
      setPermissionStatus("loading");
      const reqSeq = ++permissionReqSeqRef.current;
      try {
        const view = await bridge.agent.permissionModeGet(activeSessionId || null);
        if (!cancelled && reqSeq === permissionReqSeqRef.current) {
          setPermissionView(view);
          setPermissionStatus("ready");
        }
      } catch {
        if (!cancelled && reqSeq === permissionReqSeqRef.current) {
          setPermissionStatus("error");
        }
      }
    };
    void loadPermission();
    return () => {
      cancelled = true;
    };
  }, [activeSessionId, isFixtureMode, bridge]);

  const handlePermissionSelect = useCallback(
    async (mode: AgentPermissionMode) => {
      if (isFixtureMode) {
        setPermissionView((prev) =>
          prev
            ? {
                ...prev,
                effective: mode,
                session_override: mode,
              }
            : null
        );
        return;
      }
      if (!activeSessionId) {
        setPendingSessionMode(mode);
        return;
      }
      if (!bridge?.agent) return;
      permissionReqSeqRef.current++;
      const updated = await bridge.agent.permissionModeSetSession(activeSessionId, mode);
      setPermissionView(updated);
    },
    [activeSessionId, isFixtureMode, bridge]
  );

  const handlePermissionClearSession = useCallback(async () => {
    if (isFixtureMode) {
      setPermissionView((prev) =>
        prev
          ? {
              ...prev,
              effective: prev.global_default,
              session_override: null,
            }
          : null
      );
      return;
    }
    if (!activeSessionId) {
      setPendingSessionMode(null);
      return;
    }
    if (!bridge?.agent) return;
    permissionReqSeqRef.current++;
    const updated = await bridge.agent.permissionModeSetSession(activeSessionId, null);
    setPermissionView(updated);
  }, [activeSessionId, isFixtureMode, bridge]);

  const handlePermissionSetGlobal = useCallback(
    async (mode: AgentPermissionMode) => {
      if (isFixtureMode) {
        setPermissionView((prev) =>
          prev
            ? {
                ...prev,
                global_default: mode,
                effective: prev.session_override ?? mode,
              }
            : null
        );
        return;
      }
      if (!bridge?.agent) return;
      permissionReqSeqRef.current++;
      await bridge.agent.permissionModeSetGlobal(mode);
      const reqSeq = ++permissionReqSeqRef.current;
      const refreshed = await bridge.agent.permissionModeGet(activeSessionId || null);
      if (reqSeq === permissionReqSeqRef.current) {
        setPermissionView(refreshed);
      }
    },
    [activeSessionId, isFixtureMode, bridge]
  );

  // 切换会话时重置并拉取历史事件和 Runs
  const loadSessionHistory = useCallback(async (sessionId: string) => {
    if (!sessionId) {
      setLoadingSessionHistory(false);
      setEvents([]);
      setRuns([]);
      cursorRef.current = 0;
      return;
    }

    if (isFixtureMode && fixtureEventsFactory && fixtureRunsFactory) {
      setLoadingSessionHistory(false);
      const fixtureEvents = fixtureEventsFactory(forcedSubscene);
      for (const cid of newlySettledSearchCallIds(fixtureEvents, seenSearchCallIdsRef.current)) {
        seenSearchCallIdsRef.current.add(cid);
      }
      lastProcessedWorkspaceSeqRef.current = fixtureEvents.reduce(
        (acc, e) => (typeof e.seq === "number" && e.seq > acc ? e.seq : acc),
        0
      );
      setEvents(fixtureEvents);
      cursorRef.current = fixtureEvents.length;
      setRuns(fixtureRunsFactory(sessionId));
      return;
    }

    if (!bridge?.agent) {
      setLoadingSessionHistory(false);
      setEvents([]);
      setRuns([]);
      cursorRef.current = 0;
      lastProcessedWorkspaceSeqRef.current = 0;
      return;
    }

    const token = sessionGuard.capture();
    setLoadingSessionHistory(true);
    try {
      cursorRef.current = 0;
      // 底层对应 agent_stream_poll 从 0 开始回填完整历史
      const page = await bridge.agent.streamPoll(sessionId, 0);
      // 期间换了会话或点了「新对话」：丢弃
      if (token.sessionId !== sessionId || !sessionGuard.stillCurrent(token)) return;
      for (const cid of newlySettledSearchCallIds(page.events, seenSearchCallIdsRef.current)) {
        seenSearchCallIdsRef.current.add(cid);
      }
      // 仅对刚创建/正在运行的会话允许首包触发面板同步；历史已完成会话静默回填
      const runList = await bridge.agent.runsList(sessionId);
      if (!sessionGuard.stillCurrent(token)) return;
      const runningRun = runList.find((r) => r.status === "running");
      if (!runningRun) {
        lastProcessedWorkspaceSeqRef.current = page.events.reduce(
          (acc, e) => (typeof e.seq === "number" && e.seq > acc ? e.seq : acc),
          0
        );
      }
      setEvents(page.events);
      cursorRef.current = page.cursor;
      setRuns(runList);

      if (runningRun) {
        setActiveRunId(runningRun.run_id);
      } else {
        setActiveRunId(null);
        setLiveDelta(null);
      }
    } catch (err) {
      setActionError(t("agent.workbench.historyLoadFailed"));
    } finally {
      if (sessionGuard.stillCurrent(token)) {
        setLoadingSessionHistory(false);
      }
    }
  }, [bridge?.agent, forcedSubscene, isFixtureMode, sessionGuard, t]);

  useEffect(() => {
    if (activeSessionId) {
      loadSessionHistory(activeSessionId);
    }
  }, [activeSessionId, loadSessionHistory]);

  // 轮询逻辑：活动 Run 期间每 1 秒 streamPoll 一次（配合 agent:stream-event 推送实时唤醒）
  const pollTriggerRef = useRef<(() => void) | null>(null);
  useEffect(() => {
    if (!activeRunId || !activeSessionId || !bridge?.agent || isFixtureMode) {
      pollTriggerRef.current = null;
      return;
    }

    let isPolling = false;
    let queuedPoll = false;

    const runPollOnce = async (checkRunsWhenIdle: boolean) => {
      if (isPolling) {
        queuedPoll = true;
        return;
      }
      isPolling = true;
      const token = sessionGuard.capture();
      try {
        const page = await bridge.agent!.streamPoll(activeSessionId, cursorRef.current);
        if (token.sessionId !== activeSessionId || !sessionGuard.stillCurrent(token)) return;
        if (page.events.length > 0) {
          cursorRef.current = page.cursor;
          setEvents((prev) => {
            const existingSeqs = new Set(prev.map((e) => e.seq));
            const newOnes = page.events.filter((e) => !existingSeqs.has(e.seq));
            return [...prev, ...newOnes];
          });

          // 若本批事件已固化 message 或 conclusion，清空对应的临时增量缓冲
          if (
            page.events.some(
              (e) =>
                e.kind === "conclusion" ||
                e.kind === "error" ||
                (e.kind === "message" && (e.payload as any)?.role === "assistant")
            )
          ) {
            setLiveDelta((prev) =>
              prev ? { ...prev, assistantText: "", thinkingStatus: "done" } : null
            );
          }

          const terminalEvent = page.events.find(
            (e) =>
              (e.kind === "conclusion" || e.kind === "error") &&
              (e.payload as any)?.run_id === activeRunId
          );
          if (terminalEvent) {
            const sessionKey = activeSessionId || "__global__";
            const currentSel = resolveModelSelection(
              channels,
              Object.prototype.hasOwnProperty.call(sessionModelSelections, sessionKey)
                ? sessionModelSelections[sessionKey]
                : modelSelectionRef.current ?? readStoredModelSelection()
            );
            const fallbackDefaultCh =
              channels.find((c) => c.is_default && c.key_ready) ??
              channels.find((c) => c.key_ready);
            const resolvedChId = currentSel?.channel_id ?? fallbackDefaultCh?.id;
            if (resolvedChId) {
              if (terminalEvent.kind === "conclusion") {
                setChannelHealth(resolvedChId, "ok");
              } else if (terminalEvent.kind === "error") {
                const payload = (terminalEvent.payload ?? {}) as {
                  code?: string;
                  message_zh?: string;
                };
                const errCode = String(payload.code || "");
                if (errCode.startsWith("model_") && errCode !== "model_truncated") {
                  setChannelHealth(
                    resolvedChId,
                    "error",
                    payload.message_zh || undefined
                  );
                }
              }
            }
            setActiveRunId(null);
            setLiveDelta(null);
            const updatedRuns = await bridge.agent!.runsList(activeSessionId);
            if (sessionGuard.stillCurrent(token)) setRuns(updatedRuns);
          }
        } else if (checkRunsWhenIdle) {
          const latestRuns = await bridge.agent!.runsList(activeSessionId);
          if (!sessionGuard.stillCurrent(token)) return;
          setRuns(latestRuns);
          const currentRun = latestRuns.find((r) => r.run_id === activeRunId);
          if (currentRun && currentRun.status !== "running") {
            setActiveRunId(null);
            setLiveDelta(null);
          }
        }
      } catch {
        // 轮询异常不阻塞
      } finally {
        isPolling = false;
        if (queuedPoll) {
          queuedPoll = false;
          void runPollOnce(false);
        }
      }
    };

    pollTriggerRef.current = () => {
      void runPollOnce(false);
    };

    const interval = setInterval(() => {
      void runPollOnce(true);
    }, 1500);

    return () => clearInterval(interval);
  }, [activeRunId, activeSessionId, bridge?.agent, isFixtureMode, sessionGuard]);

  useEffect(() => {
    if (!activeRunId || !activeSessionId || isFixtureMode) {
      return;
    }

    const unsubStream = subscribeDesktopHostEvent<{ session_id?: string }>(
      "agent:stream-event",
      (payload) => {
        if (!payload || payload.session_id === activeSessionId) {
          pollTriggerRef.current?.();
        }
      }
    );

    const unsubDelta = subscribeDesktopHostEvent<AgentLiveDeltaPayload>(
      "agent:live-delta",
      (payload) => {
        if (!payload || payload.session_id !== activeSessionId) return;
        setLiveDelta((prev) => {
          const sameStep =
            prev &&
            prev.runId === payload.run_id &&
            prev.stepIndex === payload.step_index;
          const base = sameStep
            ? prev
            : {
                runId: payload.run_id,
                stepIndex: payload.step_index,
                thinkingStatus: "idle" as const,
                thinkingText: "",
                startedAtMs: payload.started_at_ms ?? Date.now(),
                durationMs: null,
                assistantText: "",
              };
          if (payload.kind === "thinking_start") {
            return {
              ...base,
              thinkingStatus: "thinking",
              startedAtMs: payload.started_at_ms ?? base.startedAtMs ?? Date.now(),
            };
          }
          if (payload.kind === "thinking") {
            return {
              ...base,
              thinkingStatus: "thinking",
              thinkingText: base.thinkingText + (payload.delta || ""),
              startedAtMs: payload.started_at_ms ?? base.startedAtMs ?? Date.now(),
            };
          }
          if (payload.kind === "thinking_end") {
            return {
              ...base,
              thinkingStatus: "done",
              durationMs: payload.duration_ms ?? base.durationMs,
            };
          }
          if (payload.kind === "text") {
            return {
              ...base,
              thinkingStatus: base.thinkingStatus === "thinking" ? "done" : base.thinkingStatus,
              assistantText: base.assistantText + (payload.delta || ""),
            };
          }
          return base;
        });
      }
    );

    return () => {
      unsubStream();
      unsubDelta();
    };
  }, [activeRunId, activeSessionId, isFixtureMode]);

  // 分类事件流项
  const classifiedItems: StreamItem[] = useMemo(() => {
    return classifyStreamEvents(events);
  }, [events]);

  // 记录所有已被 confirm_card 或 plan_card 消费的 plan_id，防止 progress_card 产生冗余「状态：执行中/已完成」行
  const allConfirmPlanIds = useMemo(() => {
    const set = new Set<string>();
    for (const item of classifiedItems) {
      if (item.kind === "confirm_card") {
        set.add(item.plan_id);
      }
    }
    return set;
  }, [classifiedItems]);

  const allPlanCardIds = useMemo(() => {
    const set = new Set<string>();
    for (const item of classifiedItems) {
      if (item.kind === "plan_card") {
        set.add(item.plan_id);
      }
    }
    return set;
  }, [classifiedItems]);

  // 统计每个 plan_id 关联的所有 progress_card 的 created_task_ids 并集
  const createdTaskIdsByPlan = useMemo(() => {
    const map = new Map<string, string[]>();
    for (const item of classifiedItems) {
      if (
        item.kind === "progress_card" &&
        item.plan_id &&
        Array.isArray(item.created_task_ids)
      ) {
        const list = map.get(item.plan_id) || [];
        for (const tid of item.created_task_ids) {
          if (typeof tid === "string" && !list.includes(tid)) {
            list.push(tid);
          }
        }
        map.set(item.plan_id, list);
      }
    }
    return map;
  }, [classifiedItems]);

  // 记录本会话中结论 reasoning_off_refused 为 true 的 run_id 集合
  const reasoningRefusedRunIds = useMemo(() => {
    const set = new Set<string>();
    for (const item of classifiedItems) {
      if (item.kind === "conclusion" && item.reasoning_off_refused && item.run_id) {
        set.add(item.run_id);
      }
    }
    return set;
  }, [classifiedItems]);

  // 记录每个 run_id 对应的最后一个 tool_group 项在 classifiedItems 中的索引
  const lastToolGroupIdxByRunId = useMemo(() => {
    const map = new Map<string, number>();
    for (let i = 0; i < classifiedItems.length; i++) {
      const item = classifiedItems[i];
      if (item.kind === "tool_group" && item.run_id) {
        map.set(item.run_id, i);
      }
    }
    return map;
  }, [classifiedItems]);

  // Owner 决策 2026-11-16：回复底栏模型用量直接展示本轮汇总（usage_total），不再逐次读取或展开第 1/2 次子调用明细
  const usageTotalByRun = useMemo(() => {
    const map = new Map<string, AgentRunSummary["usage_total"]>();
    for (const run of runs) map.set(run.run_id, run.usage_total ?? null);
    return map;
  }, [runs]);

  const batchLabelsById = useMemo(() => {
    const labels: Record<string, string> = {};
    let pendingResourceName: string | null = null;
    for (const ev of events) {
      const p = (ev.payload || {}) as Record<string, unknown>;
      if (ev.kind === "plan_card" && Array.isArray(p.steps)) {
        for (const st of p.steps) {
          if (st && typeof st === "object") {
            const disc = (st as Record<string, unknown>).disclosure;
            if (disc && typeof disc === "object") {
              const rid = (disc as Record<string, unknown>).resource_id;
              if (typeof rid === "string" && sourceNames?.[rid]) {
                pendingResourceName = sourceNames[rid];
              }
            }
          }
        }
      }
      const rawBid =
        typeof p.batch_id === "string"
          ? p.batch_id.trim()
          : typeof p.search_batch_id === "string"
            ? p.search_batch_id.trim()
            : "";
      if (rawBid) {
        const rid = typeof p.resource_id === "string" ? p.resource_id.trim() : "";
        const normalizedDs = canonicalAgentDatasetShortName(
          rid || null,
          [],
          sourceNames,
          (rid && sourceNames?.[rid]) || pendingResourceName || null
        );
        const dsName =
          normalizedDs !== "检索批次"
            ? normalizedDs
            : (rid && sourceNames?.[rid]) || pendingResourceName || "";
        const cnt =
          typeof p.result_count === "number"
            ? p.result_count
            : typeof p.total_count === "number"
              ? p.total_count
              : null;
        if (dsName && cnt !== null) {
          labels[rawBid] = t("agent.workbench.batchScenesLabel", { name: dsName, count: cnt });
        } else if (dsName && !labels[rawBid]) {
          labels[rawBid] = dsName;
        }
      }
    }
    for (const b of effectiveBatchCollection.batches) {
      if (!labels[b.batchId]) {
        const shortName = resolveAgentBatchShortName(b, sourceNames);
        const cnt = Math.max(b.totalCount, b.scenes.length);
        labels[b.batchId] =
          cnt > 0
            ? t("agent.workbench.batchScenesLabel", { name: shortName, count: cnt })
            : shortName;
      }
    }
    return labels;
  }, [events, sourceNames, effectiveBatchCollection.batches, t]);
  const lastTerminalIdxByRunId = useMemo(() => {
    const map = new Map<string, number>();
    for (let i = 0; i < classifiedItems.length; i++) {
      const item = classifiedItems[i];
      if (
        (item.kind === "conclusion" ||
          item.kind === "error" ||
          (item.kind === "message" && item.role === "assistant")) &&
        item.run_id
      ) {
        map.set(item.run_id, i);
      }
    }
    return map;
  }, [classifiedItems]);

  // 记录已存在独立非空 StreamThinkingItem 的 run_id，避免结论卡重复渲染第二条思考标题
  const runsWithStandaloneThinkingCard = useMemo(() => {
    const set = new Set<string>();
    for (const item of classifiedItems) {
      if (item.kind === "thinking" && item.run_id && item.text.trim().length > 0) {
        set.add(item.run_id);
      }
    }
    return set;
  }, [classifiedItems]);

  // 检测尚未回答且未在本机被动关闭的最后一张 ask_user_question 卡（停靠在输入框上方）
  const questionStateKey = useCallback(
    (item: { run_id?: string | null; seq: number; question_zh: string }, idx: number) =>
      `${activeSessionId || "local"}:${item.run_id || "run"}:${item.seq || idx}:${item.question_zh}`,
    [activeSessionId]
  );

  const pendingQuestionInfo = useMemo(() => {
    if (forcedSubscene) return null;
    for (let i = classifiedItems.length - 1; i >= 0; i--) {
      const it = classifiedItems[i];
      if (it.kind === "message" && it.role === "user") break;
      if (it.kind === "question_card") {
        const key = questionStateKey(it, i);
        if (answeredQuestionByKey[key]) {
          continue;
        }
        return { idx: i, key, item: it };
      }
    }
    return null;
  }, [answeredQuestionByKey, classifiedItems, forcedSubscene, questionStateKey]);

  const activeRunParkedForConfirm = useMemo(() => {
    if (!activeRunId) return false;
    return classifiedItems.some((item, idx) => {
      if (item.kind !== "confirm_card" && item.kind !== "plan_card") return false;
      if (item.run_id !== activeRunId) return false;
      if (item.auto_executed || item.approval?.decision !== "manual") return false;
      return findTerminalProgressForPlan(classifiedItems.slice(idx + 1), item.plan_id) === null;
    });
  }, [activeRunId, classifiedItems]);

  const composerActivelyBusy = Boolean(activeRunId) && !activeRunParkedForConfirm;

  // 当部分模型（如内建思考不外吐 <think> 的模型）未写入 thinking_ms 时，按本轮运行耗时减去工具等待耗时回退展示顶部思考耗时
  const turnDurationMsByRun = useMemo(() => {
    const toolMsByRun = new Map<string, number>();
    for (const item of classifiedItems) {
      if (item.kind === "tool_group" && item.run_id) {
        let sum = toolMsByRun.get(item.run_id) ?? 0;
        for (const r of item.rounds) {
          if (typeof r.duration_ms === "number" && Number.isFinite(r.duration_ms) && r.duration_ms > 0) {
            sum += r.duration_ms;
          }
        }
        toolMsByRun.set(item.run_id, sum);
      }
    }
    const map = new Map<string, number>();
    for (const run of runs) {
      if (!run.started_at || !run.ended_at) continue;
      const elapsed = Date.parse(run.ended_at) - Date.parse(run.started_at);
      if (!Number.isFinite(elapsed) || elapsed <= 0) continue;
      const toolMs = toolMsByRun.get(run.run_id) ?? 0;
      map.set(run.run_id, Math.max(100, elapsed - toolMs));
    }
    return map;
  }, [classifiedItems, runs]);

  // 轮次级统一思考与执行时间线：
  // - 进行中（isTurnActive）：默认展开在统一的「思考中 · X.X 秒」波浪标题下，平铺思考段落与单色工具步骤；
  // - 完成后（!isTurnActive）：将全部思考段落 + 中间过渡文本 + 工具执行步骤一并收纳为单行「已思考 X.X 秒 · N 个步骤」。
  const turnProcessMeta = useMemo(() => {
    const byItemIdx = new Map<
      number,
      {
        turnKey: string;
        isActive: boolean;
        isExpanded: boolean;
        isFirst: boolean;
        startedAtMs: number | null;
        durationMs: number;
        toolStepCount: number;
        runningToolHint: string | null;
        inlineConclusionThinkingText: string | null;
        inlineConclusionThinkingMs: number | null;
      }
    >();
    const suppressedConclusionIndices = new Set<number>();
    const activeRunIdsWithHeader = new Set<string>();
    const collapsedActiveRunIds = new Set<string>();
    if (forcedSubscene || classifiedItems.length === 0) {
      return {
        byItemIdx,
        suppressedConclusionIndices,
        activeRunIdsWithHeader,
        collapsedActiveRunIds,
      };
    }

    const turnRanges: Array<{ start: number; end: number }> = [];
    let currentStart = 0;
    for (let i = 0; i < classifiedItems.length; i++) {
      const it = classifiedItems[i];
      if (it.kind === "message" && it.role === "user" && i > currentStart) {
        turnRanges.push({ start: currentStart, end: i - 1 });
        currentStart = i;
      }
    }
    turnRanges.push({ start: currentStart, end: classifiedItems.length - 1 });

    for (const range of turnRanges) {
      const turnRunIds = new Set<string>();
      for (let i = range.start; i <= range.end; i++) {
        const rid = (classifiedItems[i] as { run_id?: string }).run_id;
        if (rid) turnRunIds.add(rid);
      }
      const isTurnActive =
        (activeRunId !== null && turnRunIds.has(activeRunId)) ||
        runs.some((r) => r.status === "running" && turnRunIds.has(r.run_id));

      let finalReplyIdx = -1;
      for (let i = range.end; i >= range.start; i--) {
        if (classifiedItems[i].kind === "conclusion") {
          finalReplyIdx = i;
          break;
        }
      }
      if (!isTurnActive && finalReplyIdx === -1) {
        for (let i = range.end; i >= range.start; i--) {
          const it = classifiedItems[i];
          if (it.kind === "message" && it.role === "assistant") {
            finalReplyIdx = i;
            break;
          }
        }
      }

      const collapsibleIndices: number[] = [];
      let hasProcessItem = false;
      let hasStandaloneThinking = false;
      let toolStepCount = 0;
      let durationMs = 0;
      let startedAtMs: number | null = null;
      let runningToolHint: string | null = null;

      for (let i = range.start; i <= range.end; i++) {
        const it = classifiedItems[i];
        if (it.kind === "thinking") {
          hasProcessItem = true;
          hasStandaloneThinking = true;
          collapsibleIndices.push(i);
          if (typeof it.started_at_ms === "number" && it.started_at_ms > 0) {
            startedAtMs = startedAtMs === null ? it.started_at_ms : Math.min(startedAtMs, it.started_at_ms);
          }
          if (typeof it.thinking_ms === "number" && it.thinking_ms > 0) {
            durationMs = Math.max(durationMs, it.thinking_ms);
          }
        } else if (it.kind === "tool_group") {
          hasProcessItem = true;
          collapsibleIndices.push(i);
          for (const round of it.rounds) {
            const rec = round as unknown as { calls?: unknown[] };
            toolStepCount += Array.isArray(rec.calls) ? Math.max(1, rec.calls.length) : 1;
            if (typeof round.duration_ms === "number" && round.duration_ms > 0) {
              durationMs += round.duration_ms;
            }
            if (round.status === "running") {
              runningToolHint = resolveAgentToolDisplayName(round.tool_id);
            }
          }
        } else if (it.kind === "plan_card") {
          hasProcessItem = true;
          collapsibleIndices.push(i);
          if (!runningToolHint) {
            const activePlanStep = it.steps.find((st) => st.status === "running");
            if (activePlanStep) {
              runningToolHint =
                activePlanStep.label_zh || resolveAgentToolDisplayName(activePlanStep.tool_id);
            }
          }
        } else if (
          it.kind === "progress_card" &&
          !allConfirmPlanIds.has(it.plan_id) &&
          !allPlanCardIds.has(it.plan_id)
        ) {
          collapsibleIndices.push(i);
        } else if (it.kind === "message" && it.role === "assistant" && i !== finalReplyIdx) {
          collapsibleIndices.push(i);
        } else if (it.kind === "conclusion") {
          if (typeof it.thinking_ms === "number" && it.thinking_ms > 0) {
            durationMs = Math.max(durationMs, it.thinking_ms);
          }
        }
      }

      if (isTurnActive && !hasProcessItem) continue;
      if (collapsibleIndices.length === 0) continue;

      if (startedAtMs === null) {
        for (const r of runs) {
          if (turnRunIds.has(r.run_id) && r.started_at) {
            const parsed = Date.parse(r.started_at);
            if (Number.isFinite(parsed) && parsed > 0) {
              startedAtMs = startedAtMs === null ? parsed : Math.min(startedAtMs, parsed);
            }
          }
        }
      }

      if (durationMs <= 0) {
        for (const rid of turnRunIds) {
          const d = turnDurationMsByRun.get(rid);
          if (typeof d === "number" && d > 0) {
            durationMs = Math.max(durationMs, d);
          }
        }
      }

      let inlineConclusionThinkingText: string | null = null;
      let inlineConclusionThinkingMs: number | null = null;
      if (finalReplyIdx >= 0) {
        const finalIt = classifiedItems[finalReplyIdx];
        if (finalIt.kind === "conclusion") {
          suppressedConclusionIndices.add(finalReplyIdx);
          if (
            !hasStandaloneThinking &&
            typeof finalIt.thinking_text === "string" &&
            finalIt.thinking_text.trim().length > 0
          ) {
            inlineConclusionThinkingText = finalIt.thinking_text;
            inlineConclusionThinkingMs =
              typeof finalIt.thinking_ms === "number" ? finalIt.thinking_ms : null;
          }
        }
      }

      const turnKey = `turn-${range.start}`;
      const isExpanded =
        expandedTurns[turnKey] !== undefined
          ? expandedTurns[turnKey]
          : isTurnActive
            ? true
            : stepDetailMode === "detailed";
      const firstIdx = collapsibleIndices[0];

      if (isTurnActive) {
        for (const rid of turnRunIds) {
          activeRunIdsWithHeader.add(rid);
          if (!isExpanded) {
            collapsedActiveRunIds.add(rid);
          }
        }
      }

      for (const cIdx of collapsibleIndices) {
        byItemIdx.set(cIdx, {
          turnKey,
          isActive: isTurnActive,
          isExpanded,
          isFirst: cIdx === firstIdx,
          startedAtMs,
          durationMs,
          toolStepCount,
          runningToolHint,
          inlineConclusionThinkingText: cIdx === firstIdx ? inlineConclusionThinkingText : null,
          inlineConclusionThinkingMs: cIdx === firstIdx ? inlineConclusionThinkingMs : null,
        });
      }
    }

    return {
      byItemIdx,
      suppressedConclusionIndices,
      activeRunIdsWithHeader,
      collapsedActiveRunIds,
    };
  }, [
    activeRunId,
    allConfirmPlanIds,
    allPlanCardIds,
    classifiedItems,
    expandedTurns,
    forcedSubscene,
    runs,
    stepDetailMode,
    turnDurationMsByRun,
  ]);

  useEffect(() => {
    const el = liveDeltaInnerRef.current;
    const isCollapsed =
      !liveThinkingExpanded ||
      Boolean(activeRunId && turnProcessMeta.collapsedActiveRunIds.has(activeRunId));
    if (isCollapsed) {
      setLiveDeltaMinHeight(0);
      return;
    }
    if (!el || !hasActiveLiveDelta) return;
    const measure = () => {
      const nextHeight = Math.ceil(el.getBoundingClientRect().height);
      if (nextHeight > 0) {
        setLiveDeltaMinHeight((prev) => (nextHeight > prev ? nextHeight : prev));
      }
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [
    activeRunId,
    hasActiveLiveDelta,
    liveThinkingExpanded,
    turnProcessMeta.collapsedActiveRunIds,
  ]);

  // 统计本会话关联下载任务的进行中与已完成数量（供输入框上方统一眉栏展示）
  const sessionTaskProgress = useMemo(() => {
    const taskIdSet = new Set<string>(agentCreatedTaskIds(events));
    if (sessionWork) {
      for (const w of sessionWork) {
        for (const tid of w.created_task_ids) {
          taskIdSet.add(tid);
        }
      }
    }
    let activeTaskCount = 0;
    let completedTaskCount = 0;
    for (const task of agentTasks) {
      if (!taskIdSet.has(task.task_id)) continue;
      if (task.status === "completed") {
        completedTaskCount += 1;
      } else if (
        task.status === "running" ||
        task.status === "queued" ||
        task.status === "paused"
      ) {
        activeTaskCount += 1;
      }
    }
    return { activeTaskCount, completedTaskCount };
  }, [agentTasks, events, sessionWork]);

  // 滚动监听与跟随
  const handleScroll = () => {
    const el = scrollContainerRef.current;
    if (!el) return;
    const distanceToBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
    setShouldAutoScroll(distanceToBottom <= 40);
    setShowScrollBottomBtn(distanceToBottom > 120);
  };

  const scrollToBottom = (smooth = true) => {
    const el = scrollContainerRef.current;
    if (!el) return;
    el.scrollTo({
      top: el.scrollHeight,
      behavior: smooth ? "smooth" : "instant",
    });
    setShouldAutoScroll(true);
    setShowScrollBottomBtn(false);
  };

  useEffect(() => {
    if (shouldAutoScroll) {
      scrollToBottom(false);
    }
  }, [classifiedItems.length, shouldAutoScroll]);

  // 发送指定文本（支持普通发送、附件发送、问题卡点选、排队出列、立即引导、编辑重发与重试轮次复用）
  const handleSendText = async (
    rawText: string,
    options?: {
      clearComposerOnSuccess?: boolean;
      clearAttachmentsOnSuccess?: boolean;
      attachments?: readonly ComposerAttachment[];
      targetSessionOverride?: string | null;
      retryOnBusy?: boolean;
      modelSelectionOverride?: AgentModelSelection | null;
    }
  ) => {
    const trimmed = rawText.trim();
    const wireAttachments: WireAgentAttachment[] =
      options?.attachments && options.attachments.length > 0
        ? toWireAttachments(options.attachments)
        : [];
    if ((!trimmed && wireAttachments.length === 0) || channelReadiness.disabled) return;
    if (activeRunId && !options?.retryOnBusy) return;

    const sessionTitleSeed =
      trimmed || wireAttachments[0]?.name || t("agent.workbench.attachmentAnalysisSeed");

    if (isFixtureMode) {
      const demoSessId = activeSessionId || `demo-sess-${Date.now().toString(36)}`;
      if (!activeSessionId) {
        if (onSessionCreatedSnapshot) {
          await onSessionCreatedSnapshot(demoSessId);
        }
        const nowIso = new Date().toISOString();
        setSessions((prev) => [
          {
            session_id: demoSessId,
            title: sessionTitleFromRequest(sessionTitleSeed) ?? "新会话", // i18n-exempt: user-generated session title content
            created_at: nowIso,
            updated_at: nowIso,
            run_ids: ["demo-run-item"],
          },
          ...prev,
        ]);
        setActiveSessionId(demoSessId);
      }
      setEvents((prev) => [
        ...prev,
        {
          seq: prev.length + 1,
          kind: "message",
          payload: {
            role: "user",
            run_id: "demo-run-item",
            text: trimmed,
            context: wireAttachments.length > 0 ? { attachments: wireAttachments } : null,
          },
        },
      ]);
      if (options?.clearComposerOnSuccess) {
        setInputText("");
      }
      if (options?.clearAttachmentsOnSuccess) {
        setComposerAttachments([]);
        setAttachmentError(null);
      }
      setDemoNotice(t("agent.workbench.demoMessageSent"));
      return;
    }
    if (!bridge?.agent) return;
    const agent = bridge.agent;

    setActionError(null);
    let targetSessionId = options?.targetSessionOverride ?? activeSessionId;

    try {
      // 若当前没有活动会话，先创建一个并立即固化当前工作台 AOI/结果快照到该新会话
      if (!targetSessionId) {
        const created = await agent.sessionsCreate(sessionTitleFromRequest(sessionTitleSeed));
        targetSessionId = created.session_id;
        lastProcessedWorkspaceSeqRef.current = 0;
        const draftSelection = resolveModelSelection(
          channels,
          Object.prototype.hasOwnProperty.call(sessionModelSelections, "__global__")
            ? sessionModelSelections["__global__"]
            : (modelSelectionRef.current ?? readStoredModelSelection())
        );
        sessionModelSelections[created.session_id] = draftSelection;
        modelSelectionRef.current = draftSelection;
        if (onSessionCreatedSnapshot) {
          await onSessionCreatedSnapshot(created.session_id);
        }
        sessionDraftsRef.current["__new__"] = "";
        saveStoredSessionDraft("__new__", "");
        if (options?.clearComposerOnSuccess) {
          sessionDraftsRef.current[created.session_id] = "";
          saveStoredSessionDraft(created.session_id, "");
          setInputTextState("");
        }
        setActiveSessionId(created.session_id);
        await refreshSessions(created.session_id);
      }

      // 只要 pendingSessionMode 非 null 且有目标会话，发送前都先应用；成功才清空；失败不发送并保留输入
      if (pendingSessionMode && targetSessionId) {
        const applied = await applyPendingSessionMode(
          (sid, mode) => agent.permissionModeSetSession(sid, mode),
          targetSessionId,
          pendingSessionMode
        );
        if (!applied.ok) {
          setActionError(t("agent.permission.applyFailed"));
          return;
        }
        setPendingSessionMode(null);
        if (applied.view) {
          permissionReqSeqRef.current++;
          setPermissionView(applied.view);
        }
      }

      // 构造 SendContext：优先保留真实 Polygon / MultiPolygon 矢量边界；
      // 当工作区高精度多边形序列化超过 200 KiB（如成都市 276 KiB、合肥市 784 KiB 等精细行政边界）时，
      // 传递轻量引用 { ref: "working_aoi" }，由后端直接从 Store 的 working_aoi 水合完整真实多边形，
      // 既严格保留真实多边形几何（CAPB-AOI-001），又避免触发 SendContext 256 KiB IPC 上限与事件流膨胀。
      const effectiveAoiLabel = aoiLabel && aoiLabel !== "全球视图" ? aoiLabel : "工作区范围"; // i18n-exempt: model-facing send-context content and existing label identity
      const INLINE_WORKING_AOI_MAX_BYTES = 200 * 1024;
      const workingGeometryOversized =
        workingGeometry &&
        typeof workingGeometry === "object" &&
        new TextEncoder().encode(JSON.stringify(workingGeometry)).length >
          INLINE_WORKING_AOI_MAX_BYTES;
      const resolvedGeometry: Record<string, unknown> | null =
        workingGeometry && typeof workingGeometry === "object"
          ? workingGeometryOversized
            ? { ref: "working_aoi" }
            : workingGeometry
          : workbenchAoi
            ? {
                type: "Polygon",
                coordinates: [
                  [
                    [workbenchAoi.west, workbenchAoi.south],
                    [workbenchAoi.east, workbenchAoi.south],
                    [workbenchAoi.east, workbenchAoi.north],
                    [workbenchAoi.west, workbenchAoi.north],
                    [workbenchAoi.west, workbenchAoi.south],
                  ],
                ],
              }
            : null;
      const activeSessionKey = targetSessionId || "__global__";
      const activeModelSelection =
        options && "modelSelectionOverride" in options && options.modelSelectionOverride !== undefined
          ? options.modelSelectionOverride
          : Object.prototype.hasOwnProperty.call(
                sessionModelSelections,
                activeSessionKey
              )
            ? sessionModelSelections[activeSessionKey]
            : modelSelectionRef.current ?? readStoredModelSelection();
      const baseSendContext = buildSendContext(
        resolvedGeometry
          ? {
              label: effectiveAoiLabel,
              geometry: resolvedGeometry,
            }
          : null,
        timeRange ?? null,
        sourceId ?? null,
        resolveModelSelection(channels, activeModelSelection)
      );
      const sendContext =
        wireAttachments.length > 0
          ? ({ ...baseSendContext, attachments: wireAttachments } as typeof baseSendContext)
          : baseSendContext;

      // 底层对应 agent_send（若刚从 runCancel 转向，宿主后台线程释放 live 锁可能有短暂毫秒窗口，按需自动重试）
      let runId: string | null = null;
      const maxAttempts = options?.retryOnBusy ? 25 : 1;
      for (let attempt = 0; attempt < maxAttempts; attempt++) {
        try {
          runId = await bridge.agent.send(targetSessionId, trimmed, sendContext);
          break;
        } catch (sendErr: any) {
          const errStr = JSON.stringify(sendErr || "").toLowerCase();
          const isTransientBusy =
            attempt + 1 < maxAttempts &&
            (errStr.includes("busy") || errStr.includes("running"));
          if (!isTransientBusy) {
            throw sendErr;
          }
          await new Promise((r) => setTimeout(r, 150));
        }
      }
      if (!runId) return;

      if (options?.clearComposerOnSuccess) {
        setInputText("");
      }
      if (options?.clearAttachmentsOnSuccess) {
        setComposerAttachments([]);
        setAttachmentError(null);
      }
      setLiveDelta(null);
      setActiveRunId(runId);

      // 立即触发一次拉取
      const page = await bridge.agent.streamPoll(targetSessionId, cursorRef.current);
      if (page.events.length > 0) {
        cursorRef.current = page.cursor;
        setEvents((prev) => {
          const existingSeqs = new Set(prev.map((e) => e.seq));
          const newOnes = page.events.filter((e) => !existingSeqs.has(e.seq));
          return [...prev, ...newOnes];
        });
      }

      const updatedRuns = await bridge.agent.runsList(targetSessionId);
      setRuns(updatedRuns);
      const justSentRun = updatedRuns.find((r) => r.run_id === runId);
      if (justSentRun && justSentRun.status !== "running") {
        setActiveRunId(null);
      }
    } catch (err: any) {
      setActionError(explainActionError(err, t("agent.workbench.sendFailed")));
    }
  };

  // 发送输入框消息：空闲时直接发送（支持纯文本或附带图片/文件）；回复进行中按「通用设置」走排队等待或立即引导
  const handleSendMessage = async () => {
    const trimmed = inputText.trim();
    const pendingAttachments = composerAttachmentsRef.current;
    if ((!trimmed && pendingAttachments.length === 0) || channelReadiness.disabled) return;

    if ((!activeRunId || activeRunParkedForConfirm) && !isSteering) {
      await handleSendText(trimmed, {
        clearComposerOnSuccess: true,
        clearAttachmentsOnSuccess: true,
        attachments: pendingAttachments,
      });
      return;
    }

    if (busySendMode === "steer") {
      setInputText("");
      pendingSteerTextRef.current = trimmed;
      setIsSteering(true);
      if (activeRunId && bridge?.agent && !isFixtureMode) {
        try {
          await bridge.agent.runCancel(activeRunId);
        } catch {
          // 若当前轮恰好已完成，下方空转监听会自动承接发送
        }
      }
      return;
    }

    // 默认：加入待发队列，输入框立即清空以便继续输入或一键点击「立即引导」
    setInputText("");
    setQueuedMessages((prev) => [
      ...prev,
      {
        id: `q-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`,
        text: trimmed,
      },
    ]);
  };

  // 在待发队列中对某条消息点「立即引导」：中止当前生成并优先按该新消息转向
  const handleSteerQueued = useCallback(
    async (id: string) => {
      const target = queuedMessages.find((m) => m.id === id);
      if (!target) return;
      setQueuedMessages((prev) => prev.filter((m) => m.id !== id));
      pendingSteerTextRef.current = target.text;
      setIsSteering(true);
      if (activeRunId && bridge?.agent && !isFixtureMode) {
        try {
          await bridge.agent.runCancel(activeRunId);
        } catch {
          // ignore if run just finished
        }
      } else if (!activeRunId) {
        const nextText = pendingSteerTextRef.current;
        pendingSteerTextRef.current = null;
        setIsSteering(false);
        if (nextText) {
          void handleSendText(nextText, { retryOnBusy: true });
        }
      }
    },
    [activeRunId, bridge?.agent, isFixtureMode, queuedMessages]
  );

  const handleEditQueued = useCallback((id: string) => {
    setQueuedMessages((prev) => {
      const target = prev.find((m) => m.id === id);
      if (target) {
        setInputText((cur) => (cur.trim().length > 0 ? `${cur}\n${target.text}` : target.text));
      }
      return prev.filter((m) => m.id !== id);
    });
  }, []);

  const handleRemoveQueued = useCallback((id: string) => {
    setQueuedMessages((prev) => prev.filter((m) => m.id !== id));
  }, []);

  // 当上一轮结束或被转向取消后（activeRunId 清空），自动派发挂起的「立即引导」或队首待发消息
  useEffect(() => {
    if (activeRunId || channelReadiness.disabled) return;
    if (pendingSteerTextRef.current) {
      const steerText = pendingSteerTextRef.current;
      pendingSteerTextRef.current = null;
      setIsSteering(false);
      void handleSendText(steerText, { retryOnBusy: true });
      return;
    }
    if (queuedMessages.length > 0) {
      const [first, ...rest] = queuedMessages;
      setQueuedMessages(rest);
      void handleSendText(first.text, { retryOnBusy: true });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeRunId, channelReadiness.disabled, queuedMessages]);

  // 轮次编辑并重发：截断该用户消息起的所有后续事件并恢复轮前 AOI，再发送新文本
  const handleEditResendTurn = async (beforeSeq: number, newText: string) => {
    const trimmed = newText.trim();
    if (!trimmed || activeRunId) return;
    if (isFixtureMode) {
      setEvents((prev) => prev.filter((e) => e.seq < beforeSeq));
      await handleSendText(trimmed);
      return;
    }
    if (!activeSessionId || !bridge?.agent) return;
    try {
      setActionError(null);
      if (bridge.agent.sessionTruncate) {
        const res = await bridge.agent.sessionTruncate(activeSessionId, beforeSeq, true);
        cursorRef.current = res.cursor;
        lastProcessedWorkspaceSeqRef.current = Math.min(
          lastProcessedWorkspaceSeqRef.current,
          Math.max(0, beforeSeq - 1)
        );
        if (res.aoi_restored) {
          void onSyncWorkingAoi?.();
        }
      }
      setEvents((prev) => prev.filter((e) => e.seq < beforeSeq));
      await handleSendText(trimmed, { targetSessionOverride: activeSessionId });
    } catch (err: any) {
      setActionError(explainActionError(err, t("agent.workbench.sendFailed")));
    }
  };

  // 撤回本轮及后续：截断该用户消息起的所有事件，恢复轮前 AOI，并将原提问回填至输入框
  const handleRevertTurn = async (beforeSeq: number, originalText: string) => {
    if (activeRunId) return;
    if (isFixtureMode) {
      setEvents((prev) => prev.filter((e) => e.seq < beforeSeq));
      setInputText(originalText);
      return;
    }
    if (!activeSessionId || !bridge?.agent) return;
    try {
      setActionError(null);
      if (bridge.agent.sessionTruncate) {
        const res = await bridge.agent.sessionTruncate(activeSessionId, beforeSeq, true);
        cursorRef.current = res.cursor;
        lastProcessedWorkspaceSeqRef.current = Math.min(
          lastProcessedWorkspaceSeqRef.current,
          Math.max(0, beforeSeq - 1)
        );
        if (res.aoi_restored) {
          void onSyncWorkingAoi?.();
        }
      }
      setEvents((prev) => prev.filter((e) => e.seq < beforeSeq));
      const updatedRuns = await bridge.agent.runsList(activeSessionId);
      setRuns(updatedRuns);
      setInputText(originalText);
    } catch (err: any) {
      setActionError(explainActionError(err, t("agent.workbench.sendFailed")));
    }
  };

  // 重试指定运行（或最后一轮用户提问）
  const handleRetryTurn = async (runId?: string) => {
    if (activeRunId) return;
    const userMessages = classifiedItems.filter(
      (it): it is StreamMessageItem => it.kind === "message" && it.role === "user"
    );
    const matched =
      (runId ? userMessages.find((m) => m.run_id === runId) : undefined) ??
      userMessages[userMessages.length - 1];
    if (!matched) return;
    await handleEditResendTurn(matched.seq, matched.text);
  };

  const activeDownloadCount = useMemo(
    () =>
      countSessionActiveDownloads({
        activeSessionId,
        events,
        sessionWork,
        tasks: agentTasks,
      }),
    [activeSessionId, agentTasks, events, sessionWork]
  );

  // 取消当前 Run
  const handleCancelRun = async () => {
    if (!activeRunId || !bridge?.agent) return;
    if (isFixtureMode) {
      setDemoNotice(t("agent.workbench.demoCancelUnavailable"));
      return;
    }
    try {
      // 底层对应 agent_run_cancel
      await bridge.agent.runCancel(activeRunId);
    } catch {
      setActionError(t("agent.workbench.cancelRunFailed"));
    }
  };

  // 确认卡选择目录辅助函数
  const pickDirectoryForCard = async (): Promise<string | null> => {
    try {
      const res = await pickDirectory(t("agent.workbench.pickDirectory"));
      if (res && res.ok && res.path && res.path.trim().length > 0) {
        return res.path;
      }
      return null;
    } catch (err: any) {
      setActionError(explainActionError(err, t("agent.workbench.pickDirectoryFailed")));
      return null;
    }
  };

  // 确认卡：批准
  const handleApprovePlan = async (
    planId: string,
    baseSnapshot: unknown,
    outputDir?: string,
    options?: { nisarSubset?: boolean }
  ) => {
    if (isFixtureMode) {
      setDemoNotice(t("agent.workbench.demoNoEnqueue"));
      return;
    }
    if (!bridge?.agent) return;
    try {
      setActionError(null);
      const rawSnapshot =
        options?.nisarSubset && baseSnapshot && typeof baseSnapshot === "object"
          ? { ...(baseSnapshot as Record<string, unknown>), nisar_subset: true }
          : baseSnapshot;
      // 底层对应 agent_plan_approve (必须原样回传 snapshot)
      await bridge.agent.planApprove(planId, rawSnapshot, outputDir);
      rememberApprovedOutputDir(planId, outputDir);
      // 主动触发一次拉取最新 progress_card 事件
      if (activeSessionId) {
        const page = await bridge.agent.streamPoll(activeSessionId, cursorRef.current);
        if (page.events.length > 0) {
          cursorRef.current = page.cursor;
          setEvents((prev) => [...prev, ...page.events]);
        }
      }
    } catch (err: any) {
      setActionError(explainActionError(err, t("agent.workbench.approveFailed")));
    }
  };

  // 确认卡：拒绝
  const handleRejectPlan = async (planId: string) => {
    if (isFixtureMode) {
      setDemoNotice(t("agent.workbench.demoNoReject"));
      return;
    }
    if (!bridge?.agent) return;
    try {
      setActionError(null);
      // 底层对应 agent_plan_reject
      await bridge.agent.planReject(planId);
      if (activeSessionId) {
        const page = await bridge.agent.streamPoll(activeSessionId, cursorRef.current);
        if (page.events.length > 0) {
          cursorRef.current = page.cursor;
          setEvents((prev) => [...prev, ...page.events]);
        }
      }
    } catch (err: any) {
      setActionError(explainActionError(err, t("agent.workbench.rejectFailed")));
    }
  };

  // 确认卡：自动执行计划取消调用链
  const handleCancelAuto = async (
    planId: string,
    runId: string,
    createdTaskIds: string[]
  ) => {
    const runLive =
      runId === activeRunId ||
      runs.some((r) => r.run_id === runId && r.status === "running");

    if (isFixtureMode) {
      const plan = planAutoCancel({
        runId,
        runLive,
        createdTaskIds,
        tasks: agentTasks,
      });
      const outcomeText = autoCancelOutcomeText(
        {
          runCancelled: plan.cancelRun,
          cancelledTaskIds: plan.cancelTasks.map((t) => t.taskId),
          failedTaskIds: [],
          skipped: plan.skipped,
        },
        agentTasks
      );
      setAutoCancelOutcomes((prev) => ({ ...prev, [planId]: outcomeText }));
      return;
    }

    const outcome = await executeAutoCancel(
      {
        runLive,
        getFreshTasks: async () => {
          if (refreshAgentTasks) {
            return await refreshAgentTasks();
          }
          return agentTasks;
        },
        runCancel: async (targetRunId) => {
          if (!bridge?.agent) {
            throw new Error(t("agent.workbench.bridgeUnavailable"));
          }
          return await bridge.agent.runCancel(targetRunId);
        },
        cancelTask: async (taskId, revision) => {
          if (!onTaskAction) {
            throw new Error(t("agent.workbench.taskActionUnavailable"));
          }
          await onTaskAction(taskId, revision, "cancel");
        },
      },
      {
        runId,
        createdTaskIds,
      }
    );

    if (outcome.runCancelled && activeRunId === runId) {
      setActiveRunId(null);
    }

    setAutoCancelOutcomes((prev) => ({ ...prev, [planId]: outcome.outcomeText }));
  };

  // 弱双向同步 AOI
  const handleApplyAoiToWorkbench = (proposedAoi: Bbox, aoiName?: string) => {
    const resolved = resolveAoiSync(workbenchAoi, proposedAoi, true);
    if (resolved && onUpdateWorkbenchAoi) {
      onUpdateWorkbenchAoi(resolved, aoiName);
    }
  };

  const currentSession = useMemo(() => {
    return resolveActiveSession(sessions, activeSessionId);
  }, [sessions, activeSessionId]);

  useEffect(() => {
    onActiveSessionTitleChange?.(currentSession?.title ?? null);
  }, [currentSession?.title, onActiveSessionTitleChange]);

  // 真实绑定 Alt + ↑ / ↓ 切换相邻会话
  useEffect(() => {
    const onAltArrow = (e: KeyboardEvent) => {
      if (!e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return;
      if (e.key !== "ArrowUp" && e.key !== "ArrowDown") return;
      if (sessions.length === 0) return;
      e.preventDefault();
      const idx = sessions.findIndex((s) => s.session_id === activeSessionId);
      let nextIdx = 0;
      if (idx === -1) {
        nextIdx = e.key === "ArrowDown" ? 0 : sessions.length - 1;
      } else if (e.key === "ArrowDown") {
        nextIdx = (idx + 1) % sessions.length;
      } else {
        nextIdx = (idx - 1 + sessions.length) % sessions.length;
      }
      const nextSession = sessions[nextIdx];
      if (nextSession) {
        setActiveSessionId(nextSession.session_id);
        onActiveSessionChange?.(nextSession.session_id);
      }
    };
    window.addEventListener("keydown", onAltArrow);
    return () => window.removeEventListener("keydown", onAltArrow);
  }, [sessions, activeSessionId, onActiveSessionChange]);

  const renderSidePanel = () => {
    if (!isPanelOpen) return null;
    return (
      <aside
        className={cn(
          "side-panel flex flex-col min-w-0 min-h-0 bg-sp-surface z-30",
          panelMode === "overlay"
            ? viewportWidth < 1000
              ? "absolute inset-0 z-30 w-full h-full bg-sp-surface"
              : "fixed right-0 top-12 bottom-0 w-[420px] max-w-full shadow-modal border-l border-sp-line"
            : "relative h-full border-l border-sp-line",
          isDraggingSeparator && "pointer-events-none select-none"
        )}
        style={panelMode === "docked" && effectivePanelWidth !== null ? { width: `${effectivePanelWidth}px` } : undefined}
        aria-label={t("agent.workbench.sidePanel")}
      >
        {/* 面板头部：单层半透明磨砂悬浮胶囊（无双层白底嵌套，100% 高度让渡给地图） */}
        <div
          data-testid="agent-split-panel-header"
          className="panel-head absolute top-3 left-1/2 -translate-x-1/2 z-[760] flex h-8 w-max max-w-[calc(100%-8.5rem)] items-center gap-0.5 rounded-full border border-black/10 dark:border-white/15 bg-white/75 dark:bg-zinc-900/72 px-1 shadow-sm backdrop-blur-md select-none"
        >
          <AdaptivePanelPillNavbar
            panelTab={panelTab === "map" && dockMode === "full" ? "results" : panelTab}
            onSelectTab={(nextTab) => {
              if (nextTab === "results") {
                setPanelTab("map");
                setDockMode("full");
              } else if (nextTab === "map") {
                setPanelTab("map");
                setDockMode((prev) => (prev === "full" ? "split" : prev));
              } else {
                setPanelTab(nextTab);
              }
            }}
            mapLabel={t("agent.workbench.map")}
            resultsLabel={t("agent.workbench.resultsList", {
              count:
                effectiveBatchTotalCount && effectiveBatchTotalCount > effectiveScenes.length
                  ? `${effectiveScenes.length}/${effectiveBatchTotalCount}`
                  : effectiveScenes.length,
            })}
            taskTitle={t("agent.workbench.tasksTitle", { count: agentTasks.length })}
            taskLabel={t("agent.workbench.workAndResults", { count: sessionTaskCount })}
          />

          {panelMode === "docked" ? (
            <button
              type="button"
              data-testid="btn-collapse-split-panel"
              aria-label={t("agent.workbench.collapsePanel")}
              title={t("agent.workbench.collapsePanel")}
              onClick={handleClosePanel}
              className="flex h-6 w-6 items-center justify-center rounded-full text-sp-text-secondary hover:bg-black/[0.06] dark:hover:bg-white/10 hover:text-sp-text cursor-pointer transition-colors shrink-0"
            >
              <ChevronRight className="w-3.5 h-3.5" />
            </button>
          ) : (
            <button
              type="button"
              aria-label={t("agent.workbench.closePanel")}
              title={t("agent.workbench.closePanel")}
              onClick={handleClosePanel}
              className="flex h-6 w-6 items-center justify-center rounded-full text-sp-text-secondary hover:text-sp-text hover:bg-black/[0.06] dark:hover:bg-white/10 cursor-pointer transition-colors shrink-0"
            >
              <X className="w-3.5 h-3.5" />
            </button>
          )}
        </div>

        {/* 面板内容（地图占满 100% 高度；清单/任务保留顶部避让以免被悬浮药丸遮挡） */}
        <div
          data-testid="eo-session-pane-well"
          className={cn(
            "flex-1 min-h-0 flex flex-col overflow-hidden",
            panelTab !== "map" && "mx-2 mb-2 mt-14 eo-inset-well"
          )}
        >
          {panelTab === "map" && (
            <div
              ref={splitContainerColumnRef}
              className="flex flex-1 min-h-0 flex-col w-full overflow-hidden"
            >
              <div
                className={cn(
                  "relative w-full overflow-hidden bg-slate-100 dark:bg-black",
                  dockMode === "full" ? "hidden" : "flex-1 min-h-[180px]"
                )}
              >
                <WorkbenchMap
                  declutter
                  chrome="panel"
                  disableSceneHoverCard
                  verticalTopRightControls
                  topOverlayOffsetClass="top-3"
                  fitScope="aoi-and-results"
                  lockViewportOnSceneChange={true}
                  popupTitleMode="display_name"
                  hideDownloadEntry
                  bbox={workbenchAoi}
                  aoiBbox={workbenchAoi}
                  aoiGeometry={workingGeometry}
                  aoiName={aoiLabel}
                  geodataRasterOverlay={geodataRasterOverlay}
                  strictWorkspaceAoi={true}
                  orbitLabels={{ ...ORBIT_LEGEND_LABELS_EN, heading: "Orbit direction" }}
                  scenes={effectiveScenes}
                  focusBoundsRequest={localFocusBoundsRequest}
                  selectedSceneId={
                    (selectedSceneObj?.scene_id ?? selectedSceneId)
                      ? (identityIndex.toIdentity.get(selectedSceneObj?.scene_id ?? selectedSceneId!) ??
                          (selectedSceneObj?.scene_id ?? selectedSceneId))
                      : null
                  }
                  hoveredSceneId={
                    hoveredSceneId
                      ? (identityIndex.toIdentity.get(hoveredSceneId) ?? hoveredSceneId)
                      : null
                  }
                  preview={mapPreviewState}
                  pinnedPreviews={mapPreviewEnabled ? pinnedPreviews : []}
                  onPinnedPreviewError={handlePinnedPreviewError}
                  previewTileLayer={activePreviewTileLayer}
                  activePreviewProductKey={activePreviewKey}
                  previewBbox={
                    mapPreviewEnabled && hasMapPreviewAvailable ? selectedScenePreviewBbox : null
                  }
                  previewGeometry={
                    mapPreviewEnabled && hasMapPreviewAvailable
                      ? ((selectedSceneObj?.footprint_geojson as any) ?? null)
                      : null
                  }
                  stackScenesFor={stackScenesFor}
                  onSceneSelect={(s: any) => {
                    const nextId = typeof s === "string" ? s : s?.scene_id || null;
                    handleSceneSelectFromMap(nextId);
                    if (nextId) {
                      setLocateRequest((prev) => ({ sceneId: nextId, seq: (prev?.seq ?? 0) + 1 }));
                      setDockMode((prev) => (prev === "collapsed" ? "split" : prev));
                    }
                  }}
                  drawMode={drawMode}
                  drawActive={drawActive}
                  onDrawModeChange={onDrawModeChange || (() => {})}
                  onDrawActiveChange={onDrawActiveChange || (() => {})}
                  onClearLayers={onClearLayers || (() => {})}
                  onRectDraw={onRectDraw || (() => {})}
                  onPolygonDraw={onPolygonDraw || (() => {})}
                  onPointDraw={onPointDraw || (() => {})}
                  layerKey={mapLayerKey}
                  onLayerChange={onMapLayerChange || (() => {})}
                  onOpenTileDownloadWorkbench={onOpenTileDownloadWorkbench}
                  swipeLeftLayer={swipeLeftLayer}
                  swipeRightLayer={swipeRightLayer}
                  swipePosition={swipePosition}
                  onSwipePositionChange={setSwipePosition}
                  onClearSwipe={clearSwipe}
                  topControls={
                    <>
                      {(effectiveScenes.length > 0 || Boolean(workbenchAoi)) && (
                        <button
                          type="button"
                          data-testid="btn-agent-fit-bounds"
                          onClick={() => fitScenesOrAoiBounds(effectiveScenes)}
                          title={t("agent.multiBatch.fitMapExtent")}
                          className="h-7 px-2.5 rounded-full bg-sp-surface/90 text-xs font-medium text-sp-text-secondary hover:text-sp-text inline-flex items-center gap-1.5 shadow-overlay transition-colors cursor-pointer"
                        >
                          <LocateFixed className="h-3.5 w-3.5" />
                          <span>{t("agent.multiBatch.fitMapExtent")}</span>
                        </button>
                      )}
                      {hasMapPreviewAvailable && (
                        <button
                          type="button"
                          data-testid="btn-toggle-map-preview"
                          onClick={() => setMapPreviewEnabled((prev) => !prev)}
                          title={
                            mapPreviewEnabled
                              ? t("agent.workbench.hideMapPreviewTitle")
                              : t("agent.workbench.showMapPreviewTitle")
                          }
                          className={cn(
                            "h-7 px-2.5 rounded-full text-xs font-medium inline-flex items-center gap-1.5 shadow-overlay transition-colors cursor-pointer",
                            mapPreviewEnabled
                              ? "bg-sp-accent-fill text-white"
                              : "bg-sp-surface/90 text-sp-text-secondary hover:text-sp-text"
                          )}
                        >
                          {mapPreviewEnabled ? (
                            <Eye className="h-3.5 w-3.5" />
                          ) : (
                            <EyeOff className="h-3.5 w-3.5" />
                          )}
                          <span>
                            {mapPreviewEnabled
                              ? t("agent.workbench.mapPreviewOn")
                              : t("agent.workbench.mapPreviewOff")}
                          </span>
                        </button>
                      )}
                    </>
                  }
                />

                {/* 地图底部统一数据坞收拢胶囊（点击展开同屏/全屏数据坞） */}
                {effectiveScenes.length > 0 && dockMode === "collapsed" && (
                  <div
                    data-testid="agent-map-bottom-dock"
                    className="absolute bottom-8 left-1/2 -translate-x-1/2 z-[740] flex items-center gap-2 rounded-full border border-black/10 dark:border-white/15 bg-white/90 dark:bg-zinc-900/90 px-3 py-1 text-xs text-sp-text shadow-overlay backdrop-blur-md select-none max-w-[calc(100%-24px)]"
                  >
                    <Layers className="h-3.5 w-3.5 shrink-0 text-sp-text-secondary" />
                    <span className="truncate font-medium tabular-nums">
                      {effectiveBatchCollection.batches.length > 1
                        ? t("agent.multiBatch.dockSummaryMulti", {
                            datasets: effectiveBatchCollection.batches.length,
                            count:
                              effectiveBatchCollection.combinedTotalCount ||
                              effectiveScenes.length,
                          })
                        : t("agent.multiBatch.dockSummarySingle", {
                            count: effectiveBatchTotalCount ?? effectiveScenes.length,
                          })}
                    </span>
                    {sessionTaskProgress.activeTaskCount > 0 && (
                      <span className="shrink-0 text-sp-text-muted tabular-nums">
                        {t("agent.workbench.activeDownloadingBadge", {
                          count: sessionTaskProgress.activeTaskCount,
                        })}
                      </span>
                    )}
                    <button
                      type="button"
                      data-testid="btn-expand-map-split-dock"
                      onClick={() => setDockMode("split")}
                      className="inline-flex items-center gap-0.5 rounded-full bg-sp-hover px-2 py-0.5 font-medium text-sp-text hover:opacity-85 cursor-pointer shrink-0"
                    >
                      <span>{t("agent.multiBatch.expandSplitList")}</span>
                      <ChevronUp className="h-3 w-3" />
                    </button>
                  </div>
                )}
              </div>

              {/* 三态统一底部数据坞：split (可调分屏) / full (全屏覆盖地图框，保留地图状态) */}
              {(dockMode === "split" || dockMode === "full") && (
                <div
                  data-testid="agent-map-split-results-pane"
                  style={
                    dockMode === "split"
                      ? { height: `${Math.round(dockSplitRatio * 100)}%` }
                      : undefined
                  }
                  className={cn(
                    "flex flex-col border-t border-sp-line bg-sp-card",
                    dockMode === "full"
                      ? "flex-1 min-h-0 pt-12 border-t-0"
                      : "min-h-[210px] max-h-[75%] shrink-0"
                  )}
                >
                  {dockMode === "split" && (
                    <div
                      data-testid="agent-dock-resize-handle"
                      role="separator"
                      aria-orientation="horizontal"
                      aria-label={t("agent.workbench.resizeDockAria")}
                      onPointerDown={(e) => {
                        const container = splitContainerColumnRef.current;
                        if (!container) return;
                        e.preventDefault();
                        const rect = container.getBoundingClientRect();
                        if (rect.height <= 0) return;
                        const onMove = (ev: PointerEvent) => {
                          const nextRatio = (rect.bottom - ev.clientY) / rect.height;
                          setDockSplitRatio(Math.max(0.25, Math.min(0.75, nextRatio)));
                        };
                        const onUp = () => {
                          window.removeEventListener("pointermove", onMove);
                          window.removeEventListener("pointerup", onUp);
                        };
                        window.addEventListener("pointermove", onMove);
                        window.addEventListener("pointerup", onUp);
                      }}
                      className="h-1.5 w-full shrink-0 cursor-row-resize flex items-center justify-center bg-sp-surface-muted/60 hover:bg-sp-hover transition-colors"
                    >
                      <span className="h-0.5 w-8 rounded-full bg-sp-line" />
                    </div>
                  )}
                  {effectiveScenes.length === 0 ? (
                    <div className="flex flex-1 items-center justify-center p-4">
                      {dockMode === "split" ? (
                        <EmptyState
                          kind="first-use"
                          size="card"
                          visual={<EmptyStateIcon name="layers" size={22} />}
                          title={t("agent.workbench.noResults")}
                          className="w-full max-w-md"
                        />
                      ) : (
                        <EmptyState
                          kind="first-use"
                          size="page"
                          visual={<EmptyStateWireframe variant="scene-list" showAccentAction />}
                          title={t("agent.workbench.noResults")}
                          description={t("agent.workbench.emptyResultsGuide")}
                        />
                      )}
                    </div>
                  ) : (
                    <AgentUnifiedResultsDeck
                      scenes={effectiveScenes}
                      totalCount={effectiveBatchTotalCount}
                      batches={effectiveBatchCollection.batches}
                      activeFilter={effectiveBatchCollection.activeFilter}
                      combinedCount={
                        effectiveBatchCollection.combinedTotalCount ||
                        effectiveBatchCollection.combinedSceneCount ||
                        effectiveScenes.length
                      }
                      sourceNames={sourceNames}
                      batchLabelsById={batchLabelsById}
                      onSelectFilter={handleSelectDatasetFilter}
                      defaultBatchId={effectiveActiveBatchId}
                      resolveBatchIdForScene={resolveBatchIdForScene}
                      rowStates={effectiveRowStates}
                      tasksById={tasksById}
                      sessionTasks={agentTasks.filter((tk) =>
                        agentCreatedTaskIds(events).includes(tk.task_id)
                      )}
                      onTaskAction={onTaskAction}
                      onOpenDownloadCenter={onOpenDownloadCenter}
                      selectedSceneId={selectedSceneId}
                      locateRequest={locateRequest}
                      onSelectScene={(id) => {
                        onSelectScene?.(id);
                        const target = id
                          ? effectiveScenes.find((s) => s.scene_id === id)
                          : null;
                        if (target?.footprint_bbox) {
                          triggerFocusBounds(target.footprint_bbox);
                        }
                      }}
                      onHoverSceneChange={setHoveredSceneId}
                      checkedSceneIds={checkedSceneIds}
                      onToggleCheckScene={onToggleCheckScene}
                      onToggleCheckAll={onToggleCheckAll}
                      onRowAction={onRowAction}
                      isPreviewPinnedFor={(id) => mapPreviewEnabled && pinnedPreviewIds.has(id)}
                      onTogglePinPreview={(sc) => {
                        togglePinPreviewScene(sc);
                        setMapPreviewEnabled(true);
                        if (dockMode === "full") {
                          setDockMode("split");
                        }
                      }}
                      onPreviewScene={(sc) => {
                        togglePinPreviewScene(sc);
                        setMapPreviewEnabled(true);
                        if (dockMode === "full") {
                          setDockMode("split");
                        }
                      }}
                      creatingItemIds={creatingItemIds}
                      onDownloadChecked={onDownloadChecked}
                      downloadBusy={downloadBusy}
                      onOpenInMapWorkbench={
                        onOpenInMapWorkbench
                          ? () =>
                              void onOpenInMapWorkbench(
                                effectiveActiveResourceId,
                                effectiveActiveBatchId
                              )
                          : undefined
                      }
                      isFullscreenDock={dockMode === "full"}
                      onToggleFullscreenDock={() =>
                        setDockMode((prev) => (prev === "full" ? "split" : "full"))
                      }
                      onCollapseSplit={() => setDockMode("collapsed")}
                    />
                  )}
                </div>
              )}
            </div>
          )}

          {panelTab === "results" && (
            <div className="flex flex-col flex-1 min-h-0 w-full overflow-hidden rounded-lg bg-sp-card border border-sp-line">
              {effectiveScenes.length === 0 ? (
                <div className="flex flex-1 items-center justify-center p-6">
                  <EmptyState
                    kind="first-use"
                    size="page"
                    visual={<EmptyStateWireframe variant="scene-list" showAccentAction />}
                    title={t("agent.workbench.noResults")}
                    description={t("agent.workbench.emptyResultsGuide")}
                  />
                </div>
              ) : (
                <AgentUnifiedResultsDeck
                  scenes={effectiveScenes}
                  totalCount={effectiveBatchTotalCount}
                  batches={effectiveBatchCollection.batches}
                  activeFilter={effectiveBatchCollection.activeFilter}
                  combinedCount={
                    effectiveBatchCollection.combinedTotalCount ||
                    effectiveBatchCollection.combinedSceneCount ||
                    effectiveScenes.length
                  }
                  sourceNames={sourceNames}
                  batchLabelsById={batchLabelsById}
                  onSelectFilter={handleSelectDatasetFilter}
                  defaultBatchId={effectiveActiveBatchId}
                  resolveBatchIdForScene={resolveBatchIdForScene}
                  rowStates={effectiveRowStates}
                  tasksById={tasksById}
                  sessionTasks={agentTasks.filter((tk) =>
                    agentCreatedTaskIds(events).includes(tk.task_id)
                  )}
                  onTaskAction={onTaskAction}
                  onOpenDownloadCenter={onOpenDownloadCenter}
                  selectedSceneId={selectedSceneId}
                  locateRequest={locateRequest}
                  onSelectScene={onSelectScene}
                  onHoverSceneChange={setHoveredSceneId}
                  checkedSceneIds={checkedSceneIds}
                  onToggleCheckScene={onToggleCheckScene}
                  onToggleCheckAll={onToggleCheckAll}
                  onRowAction={onRowAction}
                  isPreviewPinnedFor={(id) => mapPreviewEnabled && pinnedPreviewIds.has(id)}
                  onTogglePinPreview={(sc) => {
                    togglePinPreviewScene(sc);
                    setMapPreviewEnabled(true);
                  }}
                  onPreviewScene={(sc) => {
                    togglePinPreviewScene(sc);
                    setMapPreviewEnabled(true);
                  }}
                  creatingItemIds={creatingItemIds}
                  onDownloadChecked={onDownloadChecked}
                  downloadBusy={downloadBusy}
                  onOpenInMapWorkbench={
                    onOpenInMapWorkbench
                      ? () =>
                          void onOpenInMapWorkbench(
                            effectiveActiveResourceId,
                            effectiveActiveBatchId
                          )
                      : undefined
                  }
                />
              )}
            </div>
          )}

          {panelTab === "task" && (
            <div className="flex flex-col flex-1 min-h-0 w-full overflow-hidden rounded-lg bg-sp-card border border-sp-line">
              {effectiveBatchCollection.batches.length > 0 && (
                <div
                  data-testid="agent-task-datasets-summary"
                  className="shrink-0 border-b border-sp-divider bg-sp-surface p-3 space-y-2"
                >
                  <div className="flex items-center justify-between text-xs">
                    <span className="font-medium text-sp-text">
                      {t("agent.multiBatch.sessionDeliverablesTitle", {
                        count: effectiveBatchCollection.batches.length,
                      })}
                    </span>
                    <button
                      type="button"
                      onClick={() => {
                        setPanelTab("map");
                        setDockMode("split");
                      }}
                      className="text-xs text-sp-text-secondary hover:text-sp-text underline underline-offset-2 cursor-pointer"
                    >
                      {t("agent.workbench.viewMapAndListSplit")}
                    </button>
                  </div>
                  <div className="flex flex-wrap gap-1.5">
                    {effectiveBatchCollection.batches.map((b) => {
                      const label = resolveAgentBatchShortName(
                        b,
                        sourceNames,
                        batchLabelsById
                      );
                      const count = Math.max(b.totalCount, b.scenes.length);
                      return (
                        <button
                          key={b.batchId}
                          type="button"
                          onClick={() => {
                            handleSelectDatasetFilter(b.batchId);
                            setPanelTab("map");
                            setDockMode("split");
                          }}
                          className="inline-flex items-center gap-1.5 rounded-full border border-sp-line bg-sp-surface-muted px-2.5 py-1 text-xs text-sp-text hover:bg-sp-hover cursor-pointer tabular-nums"
                        >
                          <span className="font-medium">{label}</span>
                          <span className="text-sp-text-muted">
                            {t("agent.workbench.scenesShortCount", { count })}
                          </span>
                        </button>
                      );
                    })}
                  </div>
                </div>
              )}
              <div className="shrink-0 border-b border-sp-divider px-3 py-2">
                <div
                  role="tablist"
                  aria-label={t("agent.workbench.taskSubviewAria")}
                  className="inline-flex items-center gap-0.5 rounded-sp-md bg-sp-hover p-0.5"
                >
                  <button
                    type="button"
                    role="tab"
                    aria-selected={taskPanelSubview === "progress"}
                    data-testid="agent-task-subview-progress"
                    onClick={() => setTaskPanelSubview("progress")}
                    className={cn(
                      "flex h-7 items-center rounded-sp-sm px-2.5 text-xs outline-none transition-colors duration-fast ease-sp cursor-pointer",
                      taskPanelSubview === "progress"
                        ? "bg-sp-surface font-medium text-sp-text shadow-card dark:bg-sp-card"
                        : "text-sp-text-secondary hover:text-sp-text"
                    )}
                  >
                    {t("downloads.drawer.tabProgress")}
                  </button>
                  <button
                    type="button"
                    role="tab"
                    aria-selected={taskPanelSubview === "artifacts"}
                    data-testid="agent-task-subview-artifacts"
                    onClick={() => setTaskPanelSubview("artifacts")}
                    className={cn(
                      "flex h-7 items-center rounded-sp-sm px-2.5 text-xs outline-none transition-colors duration-fast ease-sp cursor-pointer",
                      taskPanelSubview === "artifacts"
                        ? "bg-sp-surface font-medium text-sp-text shadow-card dark:bg-sp-card"
                        : "text-sp-text-secondary hover:text-sp-text"
                    )}
                  >
                    {t("downloads.drawer.tabArtifacts")}
                  </button>
                </div>
              </div>
              {taskPanelSubview === "artifacts" ? (
                <TaskArtifactsVerificationPanel
                  tasks={agentTasks}
                  onOpenFolder={openOutputDirectory}
                  onOpenTask={onOpenDownloadCenter ? (taskId) => onOpenDownloadCenter(taskId) : undefined}
                  compact
                />
              ) : (
                <AgentWorkPanel
                  work={sessionWork}
                  tasks={agentTasks}
                  onOpenTask={onOpenDownloadCenter ? (taskId) => onOpenDownloadCenter(taskId) : undefined}
                />
              )}
            </div>
          )}
        </div>
      </aside>
    );
  };

  return (
    <div className="relative flex h-full w-full overflow-hidden bg-sp-surface select-none">
      {/* 主体区（无左侧会话栏，会话历史移至主导航「历史」页） */}
      <div className="flex flex-1 min-w-0 h-full flex-col overflow-hidden">
        {/* 全局通知横幅 (演示模式 / 错误) */}
        {demoNotice && (
          <div className="flex items-center justify-between px-3 py-1.5 bg-sp-hover border-b border-sp-line text-xs text-sp-accent-strong z-30 shrink-0">
            <span>{demoNotice}</span>
            <button
              type="button"
              aria-label={t("agent.workbench.closeNotice")}
              title={t("agent.workbench.closeNotice")}
              onClick={() => setDemoNotice(null)}
              className="p-1 hover:opacity-75 cursor-pointer"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
        )}
        {actionError && (
          <div className="flex items-center justify-between px-3 py-1.5 bg-sp-danger-bg border-b border-sp-line text-xs text-sp-danger-text z-30 shrink-0">
            <span>{actionError}</span>
            <button
              type="button"
              aria-label={t("agent.workbench.closeError")}
              title={t("agent.workbench.closeError")}
              onClick={() => setActionError(null)}
              className="p-1 hover:opacity-75 cursor-pointer"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
        )}

        {/* 主体分栏：对话区与右侧面板 (.agent-split) */}
        <div
          ref={splitContainerRef}
          className={cn(
            "agent-split relative flex-1 min-h-0 w-full overflow-hidden",
            isDraggingSeparator && "select-none"
          )}
          style={
            isPanelOpen && panelMode === "docked" && effectivePanelWidth !== null
              ? {
                  display: "grid",
                  gridTemplateColumns: `minmax(0, 1fr) auto ${effectivePanelWidth}px`,
                  gridTemplateRows: "minmax(0, 1fr)",
                }
              : {
                  display: "grid",
                  gridTemplateColumns: "minmax(0, 1fr)",
                  gridTemplateRows: "minmax(0, 1fr)",
                }
          }
        >
          {!activeSessionId ? (
            <div className="agent-empty relative flex h-full w-full items-center justify-center p-6 overflow-auto bg-sp-surface bg-white dark:bg-[#18181b]">
              <div className="hero flex w-full max-w-[660px] flex-col gap-3.5 -mt-4">
                {/* EOStage Hero Row: EOStage Wordmark on left, Real Map-Split Segmented Pill on right */}
                <div className="flex min-h-7 items-center justify-between gap-2 pl-1.5 pr-1">
                  <div className="flex items-center gap-2.5">
                    <svg
                      aria-hidden="true"
                      style={{ height: 28, width: 32 }}
                      xmlns="http://www.w3.org/2000/svg"
                      viewBox="-1 1 117 98"
                      fill="currentColor"
                      className="text-[#141413] dark:text-[#f4f4f5] shrink-0"
                    >
                      <path d="M59 5 C28.5 32.5, 10.5 70, 18.2 84.8 C25.2 96.8, 54.5 82, 77.8 51.2 L74.4 47.8 C53.2 75.2, 27.2 87.2, 22.5 79 C17.5 70, 33 34, 59 5 Z" />
                      <path d="M75 4 C76.4 30, 84.5 46.8, 114 50 C84.5 53.2, 76.4 70, 75 96 C73.6 70, 69.2 54.5, 63 50 C69.2 45.5, 73.6 30, 75 4 Z" />
                      <polygon points="41.00,20.00 52.06,18.90 75,18.90 75,21.10 52.06,21.10" />
                      <polygon points="50.00,26.00 58.50,24.95 75,24.95 75,27.05 58.50,27.05" />
                      <polygon points="58.00,32.00 63.78,31.00 75,31.00 75,33.00 63.78,33.00" />
                      <polygon points="45.00,38.00 55.20,36.85 75,36.85 75,39.15 55.20,39.15" />
                      <polygon points="23.00,44.00 35.79,42.70 75,42.70 75,45.30 35.79,45.30" />
                      <polygon points="1.00,50.00 15.90,48.40 75,48.40 75,51.60 15.90,51.60" />
                      <polygon points="23.00,56.00 35.79,54.70 75,54.70 75,57.30 35.79,57.30" />
                      <polygon points="45.00,62.00 55.20,60.85 75,60.85 75,63.15 55.20,63.15" />
                      <polygon points="58.00,68.00 63.78,67.00 75,67.00 75,69.00 63.78,69.00" />
                      <polygon points="50.00,74.00 58.50,72.95 75,72.95 75,75.05 58.50,75.05" />
                      <polygon points="41.00,80.00 52.06,78.90 75,78.90 75,81.10 52.06,81.10" />
                    </svg>
                    <span className="text-[22px] font-bold tracking-[-0.03em] text-[#18181b] dark:text-[#f4f4f5]">
                      EOStage
                    </span>
                  </div>
                </div>

                <div className="w-full" data-testid="agent-home-composer-shell">
                  <AgentComposer
                    eyebrow={
                      <div className="flex items-center justify-between gap-2">
                        <div className="flex items-center gap-2 min-w-0">
                          <span
                            className={cn(
                              "h-2 w-2 shrink-0 rounded-full",
                              !channelReadiness.ready
                                ? "bg-sp-status-warning"
                                : workbenchAoi
                                  ? "bg-sp-status-success"
                                  : "bg-sp-text-muted"
                            )}
                          />
                          {!channelReadiness.ready ? (
                            <span
                              data-testid="agent-unconfigured-eyebrow-hint"
                              className="truncate text-sp-text-secondary"
                            >
                              {t("agent.composer.unconfiguredModelShort")}
                            </span>
                          ) : (
                            <span className="truncate text-sp-text-secondary">
                              {workbenchAoi
                                ? t("agent.composer.aoiBoundEyebrow", { aoi: aoiLabel || t("agent.composer.aoiCustomFallback") })
                                : t("agent.composer.aoiGlobalEyebrow")}
                            </span>
                          )}
                        </div>
                        <div className="flex items-center gap-2.5 shrink-0 whitespace-nowrap">
                          <button
                            type="button"
                            data-testid="eyebrow-admin-region-btn"
                            onClick={() => onLocateAdminBoundary?.()}
                            className="font-medium text-sp-accent hover:underline cursor-pointer"
                          >
                            {t("agent.composer.regionLink")}
                          </button>
                          <span className="text-sp-line">·</span>
                          <button
                            type="button"
                            onClick={() => onOpenSettings?.("models")}
                            className="font-medium text-sp-accent hover:underline cursor-pointer"
                          >
                            {!channelReadiness.ready ? t("agent.composer.configureModel") : t("agent.composer.modelsAndCredentials")}
                          </button>
                        </div>
                      </div>
                    }
                    value={inputText}
                    onInputChange={setInputText}
                    onSubmit={handleSendMessage}
                    busy={Boolean(activeRunId)}
                    isRunning={Boolean(activeRunId)}
                    onStop={handleCancelRun}
                    attachments={composerAttachments}
                    onAddAttachments={handleAddComposerAttachments}
                    onRemoveAttachment={handleRemoveComposerAttachment}
                    attachmentError={attachmentError}
                    channelReadiness={channelReadiness}
                    channels={channels}
                    sessionId={activeSessionId || null}
                    onModelChange={handleModelSelectionChange}
                    onChannelGoSettings={() => onOpenSettings?.("models")}
                    onOpenPerfSettings={() => onOpenSettings?.("performance_usage")}
                    contextRuns={calibratedContextRuns}
                    contextEvents={events}
                    permissionView={displayedPermissionViewValue}
                    permissionStatus={permissionStatus}
                    permissionPending={pendingSessionMode !== null}
                    onPermissionSelect={handlePermissionSelect}
                    onPermissionSetGlobal={handlePermissionSetGlobal}
                    onPermissionClearSession={handlePermissionClearSession}
                    onDrawRect={handleDrawRectAction}
                    onOpenAdminRegion={onLocateAdminBoundary}
                    onVectorFile={handleVectorFile}
                    vectorBusy={vectorBusy}
                    vectorError={vectorError}
                    placeholder={t("agent.composer.defaultPlaceholder")}
                  />
                </div>

                {/* EOStage 典型遥感任务快捷建议（低噪 1px 细线胶囊卡片 2×2 网格） */}
                <div className="grid grid-cols-1 sm:grid-cols-2 gap-2 pt-1">
                  {[
                    {
                      tag: t("agent.homeStarters.sarTag"),
                      title: t("agent.homeStarters.sarTitle"),
                      prompt: t("agent.homeStarters.sarPrompt"),
                    },
                    {
                      tag: t("agent.homeStarters.opticalTag"),
                      title: t("agent.homeStarters.opticalTitle"),
                      prompt: t("agent.homeStarters.opticalPrompt"),
                    },
                    {
                      tag: t("agent.homeStarters.demTag"),
                      title: t("agent.homeStarters.demTitle"),
                      prompt: t("agent.homeStarters.demPrompt"),
                    },
                    {
                      tag: t("agent.homeStarters.insarTag"),
                      title: t("agent.homeStarters.insarTitle"),
                      prompt: t("agent.homeStarters.insarPrompt"),
                    },
                  ].map((starter) => (
                    <button
                      key={starter.tag}
                      type="button"
                      onClick={() => setInputText(starter.prompt)}
                      className="group flex flex-col justify-between gap-1.5 rounded-[10px] border border-[#e4e4e7] bg-[#fafafa]/80 p-3 text-left transition-colors hover:border-[#d4d4d8] hover:bg-[#f4f4f5] cursor-pointer dark:border-[#27272a] dark:bg-[#1e1e22]/70 dark:hover:border-[#3f3f46] dark:hover:bg-[#27272a]"
                    >
                      <div className="flex min-w-0 items-center justify-between gap-1.5">
                        <span className="min-w-0 truncate rounded-[4px] border border-[#e4e4e7] bg-white px-1.5 py-0.5 font-mono text-xs font-medium text-[#52525b] dark:border-[#323238] dark:bg-[#18181b] dark:text-[#a1a1aa]">
                          {starter.tag}
                        </span>
                        <span className="shrink-0 text-xs text-[#a1a1aa] transition-transform group-hover:translate-x-0.5 group-hover:text-[#18181b] dark:group-hover:text-white">
                          →
                        </span>
                      </div>
                      <span className="text-[12.5px] font-medium text-[#18181b] dark:text-[#f4f4f5] line-clamp-1">
                        {starter.title}
                      </span>
                      <span className="text-xs leading-snug text-[#71717a] dark:text-[#a1a1aa] line-clamp-2">
                        {starter.prompt}
                      </span>
                    </button>
                  ))}
                </div>
              </div>

              {/* Bottom-Right Readiness Callout Card: ONLY shown when core data accounts are missing */}
              {showOnboardingCallout &&
                credentialStatusLoaded &&
                channelReadiness.ready &&
                missingDataAccounts.length > 0 && (
                  <div
                    data-testid="onboarding-readiness-callout"
                    className="fixed right-5 bottom-5 z-30 w-[348px] rounded-[12px] border border-sp-line bg-sp-surface p-4 text-sp-text shadow-modal"
                  >
                    <div className="flex items-start justify-between gap-2">
                      <div className="flex items-center gap-2">
                        <span className="flex h-4 w-4 shrink-0 items-center justify-center rounded-full bg-sp-status-success text-xs font-bold text-white">
                          E
                        </span>
                        <span className="text-[13px] font-semibold leading-tight text-sp-text">
                          {t("agent.workbench.missingAccountsTitle")}
                        </span>
                      </div>
                      <button
                        type="button"
                        onClick={() => dismissOnboardingCallout(dontNotifyAgain)}
                        aria-label={t("agent.workbench.closeNotice")}
                        className="rounded p-0.5 text-sp-text-muted transition-colors hover:text-sp-text cursor-pointer"
                      >
                        <X className="h-3.5 w-3.5" />
                      </button>
                    </div>

                    <p className="mt-1.5 pl-6 text-[12px] leading-snug text-sp-text-secondary">
                      {t("agent.workbench.missingAccountsBody", {
                        accounts: missingDataAccounts.join(t("agent.workbench.accountListSep")),
                      })}
                    </p>

                    <div className="mt-3.5 flex items-center justify-between pl-6">
                      <label className="flex cursor-pointer items-center gap-2 text-[12px] text-sp-text-secondary hover:text-sp-text">
                        <input
                          type="checkbox"
                          checked={dontNotifyAgain}
                          onChange={(e) => setDontNotifyAgain(e.target.checked)}
                          className="h-3.5 w-3.5 rounded-[3px] border border-sp-line bg-transparent accent-[#2563eb]"
                        />
                        <span>{t("agent.workbench.dontNotifyAgain")}</span>
                          </label>

                          <button
                            type="button"
                            onClick={() => {
                              dismissOnboardingCallout(dontNotifyAgain);
                              onOpenSettings?.("auth");
                            }}
                            className="rounded-[6px] bg-sp-accent-fill px-3 py-1 text-[12px] font-medium text-white transition-colors hover:opacity-90 cursor-pointer"
                          >
                            {t("agent.workbench.addAccountsBtn")}
                          </button>
                        </div>
                    </div>
                  )}
            </div>
          ) : (
            <section className="chat flex flex-1 flex-col h-full bg-sp-surface min-w-0 border-r border-sp-line overflow-hidden select-text">
              {/* 消息区域包装容器 (flex-1 min-h-0 relative) */}
              <div className="relative flex-1 min-h-0">
                <AgentTaskProgressCard
                  classifiedItems={classifiedItems}
                  activeRunId={activeRunId}
                  compactWhenSplit={isPanelOpen}
                  className="absolute top-3 right-5 z-20"
                />
                {/* 消息滚动流 */}
                <div
                  ref={scrollContainerRef}
                  onScroll={handleScroll}
                  className="cds-scroll-fade-y h-full overflow-y-auto"
                >
                <div className="w-full max-w-[760px] mx-auto px-6 pt-6 pb-12 flex flex-col gap-4">
                  {!isRealDesktop && !isFixtureMode ? (
                    <StatusStateCard
                      variant="not_connected"
                      title={t("agent.workbench.desktopTitle")}
                      description={t("agent.workbench.desktopDescription")}
                      className="h-full"
                    />
                  ) : classifiedItems.length === 0 && !runs.some((r) => r.status === "failed") ? (
                    loadingSessionHistory ? (
                      <div
                        data-testid="agent-history-skeleton-slot"
                        className="min-h-[240px] w-full py-2"
                      >
                        {showSessionHistorySkeleton ? <AgentMessagesSkeleton /> : null}
                      </div>
                    ) : (
                      <div className="flex min-h-64 items-center justify-center">
                        <EmptyState
                          kind="first-use"
                          size="page"
                          visual={
                            !channelReadiness.ready ? (
                              <EmptyStateWireframe variant="channel-setup" showAccentAction />
                            ) : (
                              <EmptyStateIcon name="layers" size={48} />
                            )
                          }
                          title={
                            !channelReadiness.ready
                              ? t("agent.workbench.noModelConnectedTitle")
                              : t("agent.workbench.startTitle")
                          }
                          description={
                            !channelReadiness.ready
                              ? t("agent.workbench.noModelConnectedDesc")
                              : t("agent.workbench.startSlashDesc")
                          }
                          primaryAction={
                            !channelReadiness.ready && onOpenSettings
                              ? {
                                  label: t("agent.composer.configureModel"),
                                  onClick: () => onOpenSettings("models"),
                                }
                              : undefined
                          }
                        />
                      </div>
                    )
                  ) : (
                    <>
                      {classifiedItems.map((item, idx) => {
                      const foldMeta = turnProcessMeta.byItemIdx.get(idx);
                      const foldToggle =
                        foldMeta && foldMeta.isFirst ? (
                          <UnifiedTurnProcessHeader
                            key={`turn-fold-${foldMeta.turnKey}`}
                            turnKey={foldMeta.turnKey}
                            isActive={foldMeta.isActive}
                            isExpanded={foldMeta.isExpanded}
                            onToggle={() => toggleTurnExpand(foldMeta.turnKey, foldMeta.isExpanded)}
                            startedAtMs={foldMeta.startedAtMs}
                            durationMs={foldMeta.durationMs}
                            toolStepCount={foldMeta.toolStepCount}
                            runningToolHint={foldMeta.runningToolHint}
                            liveSummaryPreview={
                              foldMeta.isActive &&
                              !foldMeta.isExpanded &&
                              liveDelta &&
                              liveDelta.thinkingText.trim().length > 0
                                ? liveDelta.thinkingText
                                    .trim()
                                    .split(/\r?\n/)
                                    .map((l) => l.trim())
                                    .filter(Boolean)
                                    .slice(-1)[0]
                                : null
                            }
                          />
                        ) : null;

                      if (foldMeta && !foldMeta.isExpanded) {
                        if (!foldToggle) return null;
                        return (
                          <div
                            key={`turn-fold-wrap-${foldMeta.turnKey}`}
                            data-testid="agent-turn-process-block"
                            className="flex flex-col gap-1 w-full"
                          >
                            {foldToggle}
                          </div>
                        );
                      }

                      const renderedItem = (() => {
                      switch (item.kind) {
                        case "message": {
                          const isUser = item.role === "user";
                          if (!isUser && !foldMeta) {
                            let hasLaterAssistantReplyInTurn = false;
                            let isWorkStepsCollapsed = stepDetailMode === "concise";
                            for (let nextIdx = idx + 1; nextIdx < classifiedItems.length; nextIdx++) {
                              const nextItem = classifiedItems[nextIdx];
                              if (nextItem.kind === "message" && nextItem.role === "user") {
                                break;
                              }
                              if (
                                nextItem.kind === "conclusion" ||
                                (nextItem.kind === "message" && nextItem.role === "assistant")
                              ) {
                                hasLaterAssistantReplyInTurn = true;
                              }
                              if (nextItem.kind === "tool_group") {
                                const nextGroupKey = `tool-grp-${nextItem.seq}-${nextIdx}`;
                                const hasSubUser = classifiedItems
                                  .slice(nextIdx + 1)
                                  .some((it) => it.kind === "message" && it.role === "user");
                                const nextDefaultExpanded = shouldExpandToolGroupByDefault({
                                  stepDetailMode,
                                  collapseTiming,
                                  itemStatus: nextItem.status,
                                  hasSubsequentUserInput: hasSubUser,
                                });
                                const nextExpanded =
                                  expandedGroups[nextGroupKey] !== undefined
                                    ? expandedGroups[nextGroupKey]
                                    : nextDefaultExpanded;
                                if (nextExpanded) {
                                  isWorkStepsCollapsed = false;
                                }
                              }
                            }
                            if (
                              shouldHideIntermediateReply({
                                stepDetailMode,
                                foldedReplies,
                                isWorkStepsCollapsed,
                                hasLaterAssistantReplyInTurn,
                              })
                            ) {
                              return null;
                            }
                          }
                          const rawContextAttachments =
                            isUser && item.context && typeof item.context === "object"
                              ? (item.context as Record<string, unknown>).attachments
                              : null;
                          const msgAttachments: WireAgentAttachment[] = Array.isArray(
                            rawContextAttachments
                          )
                            ? (rawContextAttachments as WireAgentAttachment[])
                            : [];
                          return isUser ? (
                            <div
                              key={idx}
                              id={`eo-stream-item-${idx}`}
                              className="group flex flex-col items-end gap-1.5 w-full select-text"
                            >
                              <div
                                data-testid="eo-user-bubble"
                                className="eo-user-bubble cds-user-bubble max-w-[min(80%,506px)] px-3.5 py-2 text-sm leading-[1.6] text-sp-text break-words select-text"
                              >
                                {msgAttachments.length > 0 && (
                                  <div
                                    data-testid="user-message-attachments"
                                    className="mb-1.5 flex flex-wrap items-center gap-1.5"
                                  >
                                    {msgAttachments.map((att, attIdx) => {
                                      const imgSrc =
                                        att.kind === "image" && typeof att.data_base64 === "string" && att.data_base64
                                          ? att.data_base64.startsWith("data:")
                                            ? att.data_base64
                                            : `data:${att.mime_type || "image/png"};base64,${att.data_base64}`
                                          : null;
                                      return imgSrc ? (
                                        <img
                                          key={`${att.name}-${attIdx}`}
                                          src={imgSrc}
                                          alt={att.name}
                                          title={att.name}
                                          className="h-14 w-14 rounded-[7px] border border-sp-line object-cover"
                                        />
                                      ) : (
                                        <span
                                          key={`${att.name}-${attIdx}`}
                                          className="inline-flex items-center gap-1 rounded-[6px] border border-sp-line bg-sp-surface/80 px-2 py-0.5 text-xs text-sp-text-secondary"
                                        >
                                          <FileText className="h-3 w-3 shrink-0" />
                                          <span className="max-w-[160px] truncate">{att.name}</span>
                                          {typeof att.size_bytes === "number" && att.size_bytes > 0 && (
                                            <span className="text-xs text-sp-text-muted">
                                              {formatAttachmentSize(att.size_bytes)}
                                            </span>
                                          )}
                                        </span>
                                      );
                                    })}
                                  </div>
                                )}
                                {item.text && <ChatTokenizedText text={item.text} />}
                              </div>
                              <AgentMessageActionBar
                                role="user"
                                content={item.text}
                                turnIndex={idx}
                                runId={item.run_id}
                                isRunning={Boolean(activeRunId)}
                                onEditResend={(newText) => handleEditResendTurn(item.seq, newText)}
                                onRevertTurn={() => handleRevertTurn(item.seq, item.text)}
                              />
                            </div>
                          ) : foldMeta ? (
                            <div
                              key={idx}
                              id={`eo-stream-item-${idx}`}
                              className="w-full px-1.5 py-0.5 text-xs leading-[1.65] text-sp-text-secondary break-words select-text"
                            >
                              <AgentMarkdownRenderer content={item.text} />
                            </div>
                          ) : (
                            <div
                              key={idx}
                              id={`eo-stream-item-${idx}`}
                              className="group flex flex-col w-full select-text"
                            >
                              <div
                                data-testid="agent-assistant-message"
                                className="flex-1 min-w-0 flex flex-col gap-2 text-sm leading-[1.65] text-sp-text break-words select-text"
                              >
                                <AgentThinkingCard
                                  status="done"
                                  thinkingText={item.thinking_text}
                                  durationMs={
                                    item.thinking_ms ??
                                    (lastTerminalIdxByRunId.get(item.run_id) === idx
                                      ? (turnDurationMsByRun.get(item.run_id) ?? null)
                                      : null)
                                  }
                                  showElapsedWithoutText={
                                    lastTerminalIdxByRunId.get(item.run_id) === idx &&
                                    !runsWithStandaloneThinkingCard.has(item.run_id)
                                  }
                                />
                                <AgentMarkdownRenderer content={item.text} />
                                <AgentMessageActionBar
                                  role="assistant"
                                  content={item.text}
                                  turnIndex={idx}
                                  runId={item.run_id}
                                  isRunning={activeRunId === item.run_id}
                                  onRetryTurn={() => handleRetryTurn(item.run_id)}
                                  onStopRun={() => handleCancelRun()}
                                  usageTotal={
                                    lastTerminalIdxByRunId.get(item.run_id) === idx
                                      ? (usageTotalByRun.get(item.run_id) ?? null)
                                      : null
                                  }
                                />
                              </div>
                            </div>
                          );
                        }

                        case "tool_group": {
                          const groupKey = `tool-grp-${item.seq}-${idx}`;
                          const hasSubsequentUserInput = classifiedItems
                            .slice(idx + 1)
                            .some((it) => it.kind === "message" && it.role === "user");
                          const defaultExpanded = foldMeta
                            ? true
                            : shouldExpandToolGroupByDefault({
                                stepDetailMode,
                                collapseTiming,
                                itemStatus: item.status,
                                hasSubsequentUserInput,
                              });
                          const isExpanded =
                            expandedGroups[groupKey] !== undefined
                              ? expandedGroups[groupKey]
                              : defaultExpanded;
                          const isLastToolGroupForRun = lastToolGroupIdxByRunId.get(item.run_id) === idx;
                          const shouldAppendResent = isLastToolGroupForRun && reasoningRefusedRunIds.has(item.run_id);
                          const searchCard = isSearchToolGroup(item.rounds)
                            ? projectSearchCard(
                                item,
                                searchCardContext(item.run_id, classifiedItems, sourceNames)
                              )
                            : null;
                          const focusTarget = extractToolGroupFocusTarget(item, events);

                          return (
                            <div
                              key={idx}
                              id={`eo-stream-item-${idx}`}
                              className="flex flex-col gap-3 w-full"
                            >
                              <AgentToolGroupCard
                                item={item}
                                options={shouldAppendResent ? { reasoningResent: true } : undefined}
                                resolveToolName={resolveAgentToolDisplayName}
                                headless={Boolean(foldMeta)}
                                isExpanded={isExpanded}
                                onToggleExpand={() => toggleGroupExpand(groupKey, isExpanded)}
                                stepDetailMode={stepDetailMode}
                                collapseTiming={collapseTiming}
                                hasSubsequentUserInput={hasSubsequentUserInput}
                                onLocateOnMap={() => void handleFocusScenesOnMap(focusTarget)}
                                onViewOnMap={() => void handleFocusScenesOnMap(focusTarget)}
                              />

                              {!foldMeta && searchCard && (
                                <div
                                  className="cds-decision-card eo-concentric-neutral w-full"
                                  aria-label={t("agent.workbench.searchAria")}
                                >
                                  <div className="cds-decision-header flex items-center gap-2 px-3 py-1.5">
                                    <Search className="h-3.5 w-3.5 text-sp-text-secondary shrink-0" />
                                    <span className="text-xs font-semibold text-sp-text flex-1">
                                      {t("agent.workbench.search")}
                                    </span>
                                    {searchCard.status === "done" && (
                                      <span className="px-2 py-0.5 rounded-full text-xs font-semibold bg-sp-success-bg text-sp-success-text">
                                        {t("agent.workbench.done")}
                                      </span>
                                    )}
                                    {searchCard.status === "failed" && (
                                      <span className="px-2 py-0.5 rounded-full text-xs font-semibold bg-sp-danger-bg text-sp-danger-text">
                                        {t("agent.workbench.failed")}
                                      </span>
                                    )}
                                    {searchCard.status === "running" && (
                                      <span className="px-2 py-0.5 rounded-full text-xs font-semibold bg-sp-hover text-sp-accent-strong">
                                        {t("agent.workbench.running")}
                                      </span>
                                    )}
                                  </div>

                                  {(searchCard.source || searchCard.conditionBadges.length > 0 || searchCard.resultCountZh) && (
                                    <div className="eo-concentric-inner p-3.5">
                                      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2 text-xs items-baseline m-0">
                                        {searchCard.source && (
                                          <>
                                            <dt className="text-sp-text-muted whitespace-nowrap">{t("agent.source")}</dt>
                                            <dd className="m-0 text-sp-text font-medium">{searchCard.source}</dd>
                                          </>
                                        )}
                                        {searchCard.conditionBadges.length > 0 && (
                                          <>
                                            <dt className="text-sp-text-muted whitespace-nowrap">{t("agent.conditions")}</dt>
                                            <dd className="m-0 text-sp-text flex flex-wrap gap-1.5 items-center">
                                              {searchCard.conditionBadges.map((badge, bIdx) => (
                                                <span
                                                  key={bIdx}
                                                  className="px-2 py-0.5 rounded-[5px] bg-sp-surface-muted border border-sp-line text-xs font-medium text-sp-text"
                                                >
                                                  {badge}
                                                </span>
                                              ))}
                                            </dd>
                                          </>
                                        )}
                                        {searchCard.resultCountZh && (
                                          <>
                                            <dt className="text-sp-text-muted whitespace-nowrap">{t("agent.results")}</dt>
                                            <dd className="m-0 text-sp-text font-semibold">{searchCard.resultCountZh}</dd>
                                          </>
                                        )}
                                      </dl>
                                    </div>
                                  )}
                                </div>
                              )}
                            </div>
                          );
                        }

                        case "plan_card": {
                          const terminalProgress = findTerminalProgressForPlan(
                            classifiedItems.slice(idx + 1),
                            item.plan_id
                          );
                          return (
                            <div key={idx} id={`eo-stream-item-${idx}`} className="w-full">
                              <AgentPlanCard
                                item={item}
                                sourceNames={sourceNames}
                                progressStatus={terminalProgress?.status ?? null}
                                progressCode={terminalProgress?.code ?? null}
                              />
                            </div>
                          );
                        }

                        case "confirm_card": {
                          const subsequentProgress = findTerminalProgressForPlan(
                            classifiedItems.slice(idx + 1),
                            item.plan_id
                          );

                          const taskIds = createdTaskIdsByPlan.get(item.plan_id) || [];
                          const outcome = autoCancelOutcomes[item.plan_id] || null;
                          const confirmFocus = extractConfirmFocusTarget(item);
                          const ctx = searchCardContext(item.run_id, classifiedItems, sourceNames);
                          const runSearchTool = [...classifiedItems]
                            .reverse()
                            .find(
                              (it): it is StreamToolGroupItem =>
                                it.kind === "tool_group" &&
                                it.run_id === item.run_id &&
                                isSearchToolGroup(it.rounds)
                            );
                          const extraConditionBadges = runSearchTool
                            ? projectSearchCard(runSearchTool, ctx).conditionBadges
                            : undefined;

                          return (
                            <div key={idx} id={`eo-stream-item-${idx}`} className="w-full">
                              <AgentConfirmCard
                                item={item}
                                sourceNames={sourceNames}
                                sourceLabel={confirmCardSource(item, classifiedItems, sourceNames)}
                                extraConditionBadges={extraConditionBadges}
                                approvedOutputDir={recallApprovedOutputDir(item.plan_id)}
                                subsequentProgress={subsequentProgress}
                                createdTaskIds={taskIds}
                                agentTasks={agentTasks}
                                onTaskAction={onTaskAction}
                                autoCancelOutcome={outcome}
                                defaultDownloadDir={defaultDownloadDir}
                                onPickDirectory={isFixtureMode ? undefined : pickDirectoryForCard}
                                onApprove={(planId, snapshot, outputDir, _includePreciseOrbit, opts) =>
                                  handleApprovePlan(planId, snapshot, outputDir, opts)
                                }
                                onReject={(planId) => handleRejectPlan(planId)}
                                onViewOnMap={() => void handleFocusScenesOnMap(confirmFocus)}
                                onCancelAuto={(runId, createdIds) =>
                                  handleCancelAuto(item.plan_id, runId, createdIds)
                                }
                              />
                            </div>
                          );
                        }

                        case "progress_card": {
                          if (allConfirmPlanIds.has(item.plan_id) || allPlanCardIds.has(item.plan_id)) {
                            return null;
                          }
                          const text = progressStatusLineZh(item);
                          return (
                            <div key={idx} className="text-xs text-sp-text-muted py-1">
                              {text}
                            </div>
                          );
                        }

                        case "conclusion": {
                          const isLastTerminalForRun = lastTerminalIdxByRunId.get(item.run_id) === idx;
                          return (
                            <div key={idx} id={`eo-stream-item-${idx}`} className="w-full flex flex-col gap-2 select-text">
                              <AgentConclusionCard
                                item={item}
                                agentTasks={agentTasks}
                                isEvidenceExpanded={expandedEvidences[item.run_id] || false}
                                onToggleEvidenceExpand={toggleEvidenceExpand}
                                onTaskAction={onTaskAction}
                                showActionBar
                                turnIndex={idx}
                                isRunning={activeRunId === item.run_id}
                                onRetryTurn={() => handleRetryTurn(item.run_id)}
                                onStopRun={() => handleCancelRun()}
                                fallbackDurationMs={turnDurationMsByRun.get(item.run_id) ?? null}
                                suppressThinkingHeader={
                                  runsWithStandaloneThinkingCard.has(item.run_id) ||
                                  turnProcessMeta.suppressedConclusionIndices.has(idx)
                                }
                                usageTotal={isLastTerminalForRun ? (usageTotalByRun.get(item.run_id) ?? null) : null}
                                batchLabelsById={batchLabelsById}
                                onSelectBatchRef={(bid) => void handleFocusScenesOnMap({ batchId: bid })}
                              />
                            </div>
                          );
                        }

                        case "error": {
                          return (
                            <div key={idx} id={`eo-stream-item-${idx}`} className="w-full flex flex-col gap-2 select-text">
                              <AgentErrorCard
                                item={item}
                                reasoningOffRefused={runs.some(
                                  (r) => r.run_id === item.run_id && r.reasoning_off_refused === true
                                )}
                                onRetry={(rid) => handleRetryTurn(rid)}
                                onOpenModelSettings={
                                  onOpenSettings ? () => onOpenSettings("models") : undefined
                                }
                                activeDownloadCount={activeDownloadCount}
                                onOpenTaskPanel={() => setPanelTab("task")}
                              />
                            </div>
                          );
                        }

                        case "stop_all": {
                          const summary = stopAllSummary(item.payload, agentTasks);
                          const line0 = summary.lines[0];
                          const text = line0 ? `${summary.title_zh}：${line0}` : summary.title_zh;
                          return (
                            <div
                              key={idx}
                              className="my-2 text-center text-xs text-sp-text-muted select-none"
                            >
                              {text}
                            </div>
                          );
                        }

                        case "thinking": {
                          return (
                            <div key={idx} id={`eo-stream-item-${idx}`} className="w-full select-text">
                              <AgentThinkingCard
                                status={item.status}
                                thinkingText={item.text}
                                startedAtMs={item.started_at_ms}
                                durationMs={item.thinking_ms}
                                embeddedInTurn={Boolean(foldMeta)}
                              />
                            </div>
                          );
                        }

                        case "question_card": {
                          if (pendingQuestionInfo && pendingQuestionInfo.idx === idx) {
                            return null;
                          }
                          const qKey = questionStateKey(item, idx);
                          let settled = answeredQuestionByKey[qKey] ?? null;
                          if (!settled) {
                            for (let j = idx + 1; j < classifiedItems.length; j++) {
                              const nextIt = classifiedItems[j];
                              if (nextIt.kind === "question_card") break;
                              if (nextIt.kind === "message" && nextIt.role === "user") {
                                const userReply = nextIt.text.trim();
                                const matchedOpt = item.options.find(
                                  (o) =>
                                    o.label_zh?.trim() === userReply ||
                                    o.value.trim() === userReply
                                );
                                if (matchedOpt) {
                                  settled = {
                                    value: matchedOpt.value,
                                    values: [matchedOpt.value],
                                    label: matchedOpt.label_zh || matchedOpt.value,
                                    passive: isPassiveDismissQuestionOption(
                                      matchedOpt.value,
                                      matchedOpt.label_zh
                                    ),
                                  };
                                } else {
                                  const parts = userReply
                                    .split(/[、,，]/)
                                    .map((p) => p.trim())
                                    .filter(Boolean);
                                  const matchedMany = item.options.filter(
                                    (o) =>
                                      parts.includes(o.label_zh?.trim() || "") ||
                                      parts.includes(o.value.trim())
                                  );
                                  if (matchedMany.length > 0) {
                                    const vals = matchedMany.map((o) => o.value);
                                    settled = {
                                      value: vals.join(","),
                                      values: vals,
                                      label: userReply,
                                      passive: false,
                                    };
                                  }
                                }
                                break;
                              }
                            }
                          }
                          return (
                            <div key={idx} id={`eo-stream-item-${idx}`} className="w-full select-text">
                              <AgentQuestionCard
                                questionZh={item.question_zh}
                                options={item.options}
                                runId={item.run_id}
                                multiSelect={Boolean(
                                  (item as unknown as { multi_select?: boolean }).multi_select
                                )}
                                selectedValue={settled?.value ?? null}
                                selectedValues={settled?.values}
                                dismissed={Boolean(settled?.passive)}
                                disabled={composerActivelyBusy}
                                onSelectOption={(val, opt) => {
                                  const label = opt.label_zh || val;
                                  const passive = isPassiveDismissQuestionOption(val, opt.label_zh);
                                  setAnsweredQuestionByKey((prev) => ({
                                    ...prev,
                                    [qKey]: { value: val, values: [val], label, passive },
                                  }));
                                  if (!passive) {
                                    void handleSendText(label);
                                  }
                                }}
                                onSubmitMulti={(vals, opts) => {
                                  const text =
                                    opts.map((o) => o.label_zh || o.value).join("、") ||
                                    vals.join("、");
                                  setAnsweredQuestionByKey((prev) => ({
                                    ...prev,
                                    [qKey]: {
                                      value: vals.join(","),
                                      values: vals,
                                      label: text,
                                      passive: false,
                                    },
                                  }));
                                  if (text.trim()) void handleSendText(text);
                                }}
                              />
                            </div>
                          );
                        }

                        case "gee_artifact": {
                          return (
                            <div key={idx} id={`eo-stream-item-${idx}`} className="w-full">
                              <GeeScriptConfirmCard
                                key={`${activeSessionId || "local"}:${item.run_id}:${item.call_id}:${item.seq}`}
                                toolId={item.tool_id}
                                toolArgs={item.tool_args}
                                summary={item.summary}
                                scriptPreview={item.script_preview}
                                onViewOnMap={() => void handleFocusScenesOnMap()}
                              />
                            </div>
                          );
                        }

                        case "workspace_sync": {
                          return null;
                        }
                        default:
                          return null;
                      }
                      })();

                      if (foldMeta) {
                        const prevHasFold =
                          idx > 0 &&
                          turnProcessMeta.byItemIdx.get(idx - 1)?.turnKey === foldMeta.turnKey;
                        const inlineConclusionThinkingNode =
                          foldMeta.inlineConclusionThinkingText ? (
                            <AgentThinkingCard
                              status="done"
                              thinkingText={foldMeta.inlineConclusionThinkingText}
                              durationMs={foldMeta.inlineConclusionThinkingMs}
                              embeddedInTurn
                            />
                          ) : null;
                        if (foldToggle) {
                          return (
                            <div
                              key={`turn-fold-wrap-${foldMeta.turnKey}`}
                              data-testid="agent-turn-process-block"
                              className="flex flex-col gap-1 w-full"
                            >
                              {foldToggle}
                              <div className="ml-[7px] border-l border-sp-line pl-2.5 flex flex-col gap-1 w-full">
                                {inlineConclusionThinkingNode}
                                {renderedItem}
                              </div>
                            </div>
                          );
                        }
                        return (
                          <div
                            key={idx}
                            className={cn(
                              "ml-[7px] border-l border-sp-line pl-2.5 w-full",
                              prevHasFold && "-mt-4 pt-1"
                            )}
                          >
                            {renderedItem}
                          </div>
                        );
                      }
                      return renderedItem;
                    })}
                    {runs
                      .filter(
                        (r) =>
                          r.status === "failed" &&
                          !classifiedItems.some(
                            (it) =>
                              (it.kind === "error" || it.kind === "conclusion") &&
                              it.run_id === r.run_id
                          )
                      )
                      .map((failedRun) => (
                        <div
                          key={`orphan-failed-${failedRun.run_id}`}
                          data-testid={`orphan-failed-run-${failedRun.run_id}`}
                          className="w-full flex flex-col gap-2 select-text"
                        >
                          <AgentErrorCard
                            item={{
                              kind: "error",
                              seq: 0,
                              run_id: failedRun.run_id,
                              code: "AGENT_RUN_FAILED",
                              status: "failed",
                              plan_id: null,
                              message_zh: t("agent.workbench.orphanRunFailedMsg"),
                            }}
                            reasoningOffRefused={failedRun.reasoning_off_refused === true}
                            onRetry={(rid) => handleRetryTurn(rid)}
                            onOpenModelSettings={
                              onOpenSettings ? () => onOpenSettings("models") : undefined
                            }
                            activeDownloadCount={activeDownloadCount}
                            onOpenTaskPanel={() => setPanelTab("task")}
                          />
                        </div>
                      ))}
                    </>
                  )}

                  {/* 实时流式思考与正文增量预览（尚未落盘为最终事件前；若所在轮次已由用户手动折叠且尚无正文，则保持折叠态不重新顶开） */}
                  {activeRunId &&
                    liveDelta &&
                    liveDelta.runId === activeRunId &&
                    (liveDelta.thinkingStatus === "thinking" ||
                      liveDelta.thinkingText.trim().length > 0 ||
                      liveDelta.assistantText.trim().length > 0) &&
                    !(
                      turnProcessMeta.collapsedActiveRunIds.has(liveDelta.runId) &&
                      liveDelta.assistantText.trim().length === 0
                    ) && (
                      <div
                        data-testid="agent-live-delta-preview"
                        style={{
                          minHeight:
                            liveThinkingExpanded &&
                            !turnProcessMeta.collapsedActiveRunIds.has(liveDelta.runId) &&
                            liveDeltaMinHeight > 0
                              ? `${liveDeltaMinHeight}px`
                              : undefined,
                        }}
                        className="flex flex-col w-full select-text"
                      >
                        <div ref={liveDeltaInnerRef} className="flex-1 min-w-0 flex flex-col gap-2 select-text">
                          {!turnProcessMeta.collapsedActiveRunIds.has(liveDelta.runId) &&
                            !classifiedItems.some(
                              (it) =>
                                it.kind === "thinking" &&
                                it.run_id === liveDelta.runId &&
                                it.step_index === liveDelta.stepIndex
                            ) && (
                              <AgentThinkingCard
                                status={liveDelta.thinkingStatus}
                                thinkingText={liveDelta.thinkingText}
                                startedAtMs={liveDelta.startedAtMs}
                                durationMs={liveDelta.durationMs}
                                onExpandedChange={(exp) => {
                                  setLiveThinkingExpanded(exp);
                                  if (!exp) setLiveDeltaMinHeight(0);
                                }}
                                embeddedInTurn={turnProcessMeta.activeRunIdsWithHeader.has(
                                  liveDelta.runId
                                )}
                              />
                            )}
                          {liveDelta.assistantText.trim().length > 0 && (
                            <AgentMarkdownRenderer content={liveDelta.assistantText} streaming />
                          )}
                        </div>
                      </div>
                    )}

                  {/* 首个思考/过程事件到达前的单行思考等待态（无 EO 头像块，唯一停止入口为输入框变形停止键） */}
                  {activeRunId &&
                    !turnProcessMeta.activeRunIdsWithHeader.has(activeRunId) &&
                    !(
                      liveDelta &&
                      liveDelta.runId === activeRunId &&
                      (liveDelta.thinkingStatus === "thinking" ||
                        liveDelta.thinkingText.trim().length > 0 ||
                        liveDelta.assistantText.trim().length > 0)
                    ) && (
                      <div data-testid="agent-live-pending-thinking" className="w-full">
                        <AgentThinkingCard
                          status="thinking"
                          startedAtMs={liveDelta?.startedAtMs ?? null}
                          durationMs={null}
                        />
                      </div>
                    )}
                </div>
              </div>

              {/* EOStage 右侧边缘 TimelineTicks Scrollspy 导航刻度条 */}
              {classifiedItems.length > 0 && (
                <div
                  data-testid="eo-timeline-ticks"
                  aria-label={t("agent.workbench.timelineTicksAria")}
                  className="pointer-events-none absolute right-1.5 top-1/2 -translate-y-1/2 z-10 flex flex-col items-end gap-1.5 py-2"
                >
                  {classifiedItems.map((item, idx) => {
                    const isUserMsg = item.kind === "message" && item.role === "user";
                    const isMajorCard =
                      item.kind === "plan_card" ||
                      item.kind === "confirm_card" ||
                      item.kind === "gee_artifact" ||
                      item.kind === "conclusion" ||
                      item.kind === "error";
                    if (!isUserMsg && !isMajorCard) return null;
                    const tickLabel = isUserMsg
                      ? item.text.slice(0, 28)
                      : item.kind === "plan_card"
                      ? t("agent.gee.timelinePlan")
                      : item.kind === "confirm_card"
                      ? t("agent.confirm.title")
                      : item.kind === "gee_artifact"
                      ? t("agent.gee.timelineTick")
                      : item.kind === "conclusion"
                      ? t("agent.gee.timelineConclusion")
                      : t("agent.gee.timelineError");
                    return (
                      <button
                        key={`tick-${idx}`}
                        type="button"
                        data-testid="eo-timeline-tick"
                        title={tickLabel}
                        aria-label={tickLabel}
                        onClick={() => {
                          const target = scrollContainerRef.current?.querySelector(
                            `#eo-stream-item-${idx}`
                          );
                          target?.scrollIntoView({ behavior: "smooth", block: "center" });
                        }}
                        className="pointer-events-auto relative group/tick flex items-center py-0.5 px-1 cursor-pointer outline-none"
                      >
                        <span className="pointer-events-none absolute right-full top-1/2 mr-1.5 -translate-y-1/2 whitespace-nowrap max-w-[160px] truncate rounded-md bg-sp-card px-2 py-0.5 text-xs text-sp-text shadow-overlay border border-sp-line opacity-0 group-hover/tick:opacity-100 group-focus-visible/tick:opacity-100 transition-opacity duration-fast">
                          {tickLabel}
                        </span>
                        <span
                          className={cn(
                            "h-0.5 rounded-full transition-all duration-fast",
                            isUserMsg
                              ? "w-3 bg-sp-text-secondary group-hover/tick:w-4 group-hover/tick:bg-sp-accent"
                              : "w-2 bg-sp-text-disabled group-hover/tick:w-3.5 group-hover/tick:bg-sp-text"
                          )}
                        />
                      </button>
                    );
                  })}
                </div>
              )}

              {/* 距底 >120px 时浮现「跳至最新」按钮 (基线 4b-2，锚定在消息滚动包装区底部) */}
              {showScrollBottomBtn && (
                <div className="absolute bottom-4 right-6 z-10">
                  <button
                    type="button"
                    onClick={() => scrollToBottom(true)}
                    className="flex items-center gap-1 rounded-full bg-sp-surface border border-sp-line px-3 py-1.5 text-xs font-semibold text-sp-text shadow-modal hover:bg-sp-hover transition-all duration-fast ease-sp cursor-pointer"
                  >
                    <ArrowDown className="h-3.5 w-3.5 text-sp-accent" />
                    <span>{t("agent.workbench.jumpLatest")}</span>
                  </button>
                </div>
              )}
            </div>

            {/* 底部输入框区（停靠待答 ask_user_question 卡 + EOStage 同心圆 InputShell） */}
              <div className="px-3 pb-3 pt-1 bg-sp-surface shrink-0">
                <div
                  className="max-w-[712px] mx-auto flex flex-col gap-2"
                  data-testid="agent-active-composer-shell"
                >
                  {pendingQuestionInfo && (
                    <AgentQuestionCard
                      variant="docked"
                      questionZh={pendingQuestionInfo.item.question_zh}
                      options={pendingQuestionInfo.item.options}
                      runId={pendingQuestionInfo.item.run_id}
                      multiSelect={Boolean(
                        (pendingQuestionInfo.item as unknown as { multi_select?: boolean }).multi_select
                      )}
                      disabled={composerActivelyBusy}
                      onSelectOption={(val, opt) => {
                        const label = opt.label_zh || val;
                        const passive = isPassiveDismissQuestionOption(val, opt.label_zh);
                        const key = pendingQuestionInfo.key;
                        setAnsweredQuestionByKey((prev) => ({
                          ...prev,
                          [key]: { value: val, label, passive },
                        }));
                        if (!passive) {
                          void handleSendText(label);
                        }
                      }}
                      onSubmitMulti={(vals, opts) => {
                        const text = opts.map((o) => o.label_zh || o.value).join("、") || vals.join("、");
                        const key = pendingQuestionInfo.key;
                        setAnsweredQuestionByKey((prev) => ({
                          ...prev,
                          [key]: { value: vals.join(","), values: vals, label: text, passive: false },
                        }));
                        if (text.trim()) void handleSendText(text);
                      }}
                    />
                  )}
                  <AgentComposer
                    compact
                    eyebrow={
                      <AgentComposerEyebrow
                        hasBoundAoi={Boolean(workbenchAoi)}
                        aoiLabel={aoiLabel}
                        onOpenMap={() => openSplitPanel("map")}
                        onSelectAdminRegion={onLocateAdminBoundary}
                        onClearAoi={onClearLayers}
                        sourceLabel={resolveAgentSessionSourceLabel({
                          batches: effectiveBatchCollection.batches,
                          activeFilter: effectiveBatchCollection.activeFilter,
                          scenes: effectiveScenes,
                          effectiveResourceId: effectiveActiveResourceId,
                          fallbackSourceId: sourceId,
                          sourceNames,
                          batchLabelsById,
                        })}
                        scenesCount={effectiveScenes.length}
                        checkedCount={checkedSceneIds.size}
                        onOpenResults={() => openSplitPanel("results")}
                        activeTaskCount={sessionTaskProgress.activeTaskCount}
                        completedTaskCount={sessionTaskProgress.completedTaskCount}
                        onOpenTasks={() => openSplitPanel("task")}
                        queuedMessages={queuedMessages}
                        isSteering={isSteering}
                        onSteerQueued={handleSteerQueued}
                        onEditQueued={handleEditQueued}
                        onRemoveQueued={handleRemoveQueued}
                      />
                    }
                    value={inputText}
                    onInputChange={setInputText}
                    onSubmit={handleSendMessage}
                    busy={composerActivelyBusy}
                    isRunning={Boolean(activeRunId)}
                    onStop={handleCancelRun}
                    allowSendWhileBusy
                    attachments={composerAttachments}
                    onAddAttachments={handleAddComposerAttachments}
                    onRemoveAttachment={handleRemoveComposerAttachment}
                    attachmentError={attachmentError}
                    sendTitleOverride={
                      composerActivelyBusy
                        ? busySendMode === "steer"
                          ? t("agent.queue.sendToSteerHint")
                          : t("agent.queue.sendToQueueHint")
                        : undefined
                    }
                    channelReadiness={channelReadiness}
                    channels={channels}
                    sessionId={activeSessionId || null}
                    onModelChange={handleModelSelectionChange}
                    onChannelGoSettings={() => onOpenSettings?.("models")}
                    onOpenPerfSettings={() => onOpenSettings?.("performance_usage")}
                    contextRuns={calibratedContextRuns}
                    contextEvents={events}
                    permissionView={displayedPermissionViewValue}
                    permissionStatus={permissionStatus}
                    permissionPending={pendingSessionMode !== null}
                    onPermissionSelect={handlePermissionSelect}
                    onPermissionSetGlobal={handlePermissionSetGlobal}
                    onPermissionClearSession={handlePermissionClearSession}
                    onDrawRect={handleDrawRectAction}
                    onOpenAdminRegion={onLocateAdminBoundary}
                    onVectorFile={handleVectorFile}
                    vectorBusy={vectorBusy}
                    vectorError={vectorError}
                    placeholder={
                      composerActivelyBusy
                        ? busySendMode === "steer"
                          ? t("agent.queue.sendToSteerHint")
                          : t("agent.queue.sendToQueueHint")
                        : t("agent.composer.continuePlaceholder")
                    }
                  />
                </div>
              </div>
            </section>
          )}

          {/* 对话区与面板可拖拽分割条（集成 CDS 双纹胶囊拖拽手柄） */}
          {isPanelOpen && panelMode === "docked" && effectivePanelWidth !== null && (
            <div
              role="separator"
              aria-orientation="vertical"
              aria-label={t("agent.workbench.resize")}
              tabIndex={0}
              aria-valuenow={Math.round(effectivePanelWidth)}
              onPointerDown={handleSeparatorPointerDown}
              onKeyDown={handleSeparatorKeyDown}
              onDoubleClick={handleResetPanelWidth}
              className="group relative z-20 flex h-full w-2 shrink-0 cursor-col-resize items-center justify-center outline-none select-none focus-visible:ring-1 focus-visible:ring-sp-accent"
            >
              <div
                className={cn(
                  "h-full w-px transition-colors duration-fast ease-sp",
                  isDraggingSeparator
                    ? "bg-sp-line"
                    : "bg-transparent group-hover:bg-sp-line group-focus-visible:bg-sp-line"
                )}
              />
              <span
                aria-hidden="true"
                data-testid="cds-split-pill"
                className={cn("cds-split-pill", isDraggingSeparator && "is-dragging")}
              >
                <span />
                <span />
              </span>
            </div>
          )}

          {/* 右侧面板 */}
          {renderSidePanel()}

          {/* 地图收起后的一键展开入口（带 Magnetic Peek 磁吸微探出） */}
          {!isPanelOpen && (
            <button
              type="button"
              data-testid="agent-expand-map-handle"
              onClick={() => {
                setPanelRequested(true);
                setPanelIntent("map-view");
                onPanelOpenChange?.(true);
              }}
              aria-label={t("shell.titlebar.expandMap")}
              title={t("shell.titlebar.expandMap")}
              className="sp-magnetic-peek-left absolute right-0 top-1/2 -translate-y-1/2 z-[1001] w-5 h-16 rounded-l-sp-md bg-sp-surface text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover shadow-md border-y border-l border-sp-line flex flex-col items-center justify-center cursor-pointer transition-colors group"
            >
              <ChevronLeft className="w-3.5 h-3.5 sp-peek-arrow" />
            </button>
          )}
        </div>
      </div>

      {/* 多要素矢量导入选择与合并弹窗 */}
      {pendingVectorPreview && (
        <VectorFeaturePickerModal
          open={true}
          preview={pendingVectorPreview.preview}
          busy={vectorBusy}
          onConfirm={handleConfirmVectorPicker}
          onCancel={() => {
            setPendingVectorPreview(null);
            setVectorError(null);
          }}
        />
      )}
    </div>
  );
}

interface AdaptivePanelPillNavbarProps {
  panelTab: PanelTab;
  onSelectTab: (tab: PanelTab) => void;
  mapLabel: string;
  resultsLabel: string;
  taskTitle: string;
  taskLabel: string;
}

function AdaptivePanelPillNavbar({
  panelTab,
  onSelectTab,
  mapLabel,
  resultsLabel,
  taskTitle,
  taskLabel,
}: AdaptivePanelPillNavbarProps) {
  const [hoveredTab, setHoveredTab] = useState<PanelTab | null>(null);
  const [focusedTab, setFocusedTab] = useState<PanelTab | null>(null);
  const navRef = useRef<HTMLDivElement | null>(null);
  const tabRefs = useRef<Record<PanelTab, HTMLButtonElement | null>>({
    map: null,
    results: null,
    task: null,
  });
  const labelRefs = useRef<Record<PanelTab, HTMLSpanElement | null>>({
    map: null,
    results: null,
    task: null,
  });

  const [labelWidths, setLabelWidths] = useState<Record<PanelTab, number>>({
    map: 28,
    results: 64,
    task: 76,
  });

  const items = useMemo(
    () =>
      [
        {
          key: "map" as const,
          icon: MapIcon,
          label: mapLabel,
          title: mapLabel,
          ariaLabel: mapLabel,
        },
        {
          key: "results" as const,
          icon: List,
          label: resultsLabel,
          title: resultsLabel,
          ariaLabel: resultsLabel,
        },
        {
          key: "task" as const,
          icon: ClipboardList,
          label: taskLabel,
          title: taskTitle,
          ariaLabel: taskLabel,
        },
      ] as const,
    [mapLabel, resultsLabel, taskTitle, taskLabel]
  );

  useEffect(() => {
    const measureLabels = () => {
      const next: Record<PanelTab, number> = {
        map: 28,
        results: 64,
        task: 76,
      };
      for (const item of items) {
        const el = labelRefs.current[item.key];
        if (el && el.scrollWidth > 0) {
          next[item.key] = Math.ceil(el.scrollWidth) + 2;
        } else {
          next[item.key] = Math.max(24, item.label.length * 11 + 4);
        }
      }
      setLabelWidths((prev) =>
        prev.map === next.map &&
        prev.results === next.results &&
        prev.task === next.task
          ? prev
          : next
      );
    };
    measureLabels();
    let rafId: number | null = null;
    if (typeof window !== "undefined" && typeof window.requestAnimationFrame === "function") {
      rafId = window.requestAnimationFrame(measureLabels);
    }
    return () => {
      if (rafId !== null && typeof window !== "undefined" && typeof window.cancelAnimationFrame === "function") {
        window.cancelAnimationFrame(rafId);
      }
    };
  }, [items]);

  // 单阶段确定性几何同步：不通过 ResizeObserver 追踪过渡中间态（消除双重弹簧滞后与回弹打架），
  // 在 hover/click 触发的第 0 帧即给出最终目标 left 与 width，与标签宽度用同一条 260ms 曲线同步插值。
  const indicatorRect = useMemo(() => {
    const NAV_PAD = 3;
    const NAV_GAP = 4;
    const BTN_CLOSED_W = 34; // 左右 padding 10px*2 + 图标 14px
    const LABEL_GAP = 6;
    let left = NAV_PAD;
    let width = BTN_CLOSED_W + LABEL_GAP + labelWidths[panelTab];
    for (const item of items) {
      const expanded =
        item.key === panelTab ||
        hoveredTab === item.key ||
        focusedTab === item.key;
      const itemW = expanded
        ? BTN_CLOSED_W + LABEL_GAP + labelWidths[item.key]
        : BTN_CLOSED_W;
      if (item.key === panelTab) {
        width = itemW;
        break;
      }
      left += itemW + NAV_GAP;
    }
    return { left, width };
  }, [panelTab, hoveredTab, focusedTab, labelWidths, items]);

  return (
    <div
      ref={navRef}
      role="tablist"
      data-testid="agent-adaptive-pill-nav"
      data-active-tab={panelTab}
      className="sp-adaptive-pill-nav shrink-0 whitespace-nowrap"
      onMouseLeave={() => setHoveredTab(null)}
    >
      <div
        aria-hidden="true"
        data-testid="agent-panel-pill-indicator"
        data-active-tab={panelTab}
        className="sp-adaptive-pill-indicator"
        style={{
          transform: `translate3d(${indicatorRect.left}px, 0, 0)`,
          width: `${indicatorRect.width}px`,
        }}
      />
      {items.map((item) => {
        const Icon = item.icon;
        const isActive = panelTab === item.key;
        const isExpanded = isActive || hoveredTab === item.key || focusedTab === item.key;
        return (
          <button
            key={item.key}
            ref={(el) => {
              tabRefs.current[item.key] = el;
            }}
            type="button"
            role="tab"
            data-testid={`agent-panel-tab-${item.key}`}
            data-expanded={isExpanded ? "true" : "false"}
            aria-selected={isActive}
            onClick={() => onSelectTab(item.key)}
            onMouseEnter={() => setHoveredTab(item.key)}
            onFocus={() => setFocusedTab(item.key)}
            onBlur={() => setFocusedTab((prev) => (prev === item.key ? null : prev))}
            title={item.title}
            aria-label={item.ariaLabel}
            className={cn(
              "sp-adaptive-pill-btn shrink-0 whitespace-nowrap text-xs font-medium",
              isActive && "is-active"
            )}
          >
            <Icon className="w-3.5 h-3.5 shrink-0" />
            <span
              data-testid={`agent-panel-tab-label-wrap-${item.key}`}
              style={
                {
                  "--sp-pill-label-w": `${labelWidths[item.key]}px`,
                } as React.CSSProperties
              }
              className={cn(
                "sp-adaptive-pill-label-wrap",
                isExpanded && "is-expanded"
              )}
            >
              <span
                ref={(el) => {
                  labelRefs.current[item.key] = el;
                }}
                className="sp-adaptive-pill-label whitespace-nowrap"
              >
                {item.label}
              </span>
            </span>
          </button>
        );
      })}
    </div>
  );
}
