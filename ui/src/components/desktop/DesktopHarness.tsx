import React, { useState, useEffect, useMemo } from "react";
import {
  Sun,
  Moon,
  AlertCircle,
  Loader2,
  Lock,
  ShieldAlert,
  ArrowDownCircle,
  RotateCcw,
  LogIn,
  Layers,
  FolderOpen,
  Info,
  CheckCircle2,
  Clock,
  Pause,
  Play,
  Activity,
  FileText,
  Globe,
  Cpu,
  KeyRound,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";
import type { ApplicationTask, TaskLifecycleStatus, AccountState } from "@/shell/api/types";
import { partitionTasks, getAvailableTaskActions } from "@/shell/api/taskLogic";
import { getTasks } from "@/shell/api";
import { DesktopDownloadDrawer, StatusBadge } from "./DesktopDownloadDrawer";
import { DesktopAgentWorkbench } from "./DesktopAgentWorkbench";
import { AgentHistoryPage } from "./AgentHistoryPage";
import { InplaceLoginCard } from "@/shell/components/InplaceLoginCard";
import { SettingsModal, type SettingsCategory } from "@/components/SettingsModal";
import { TopCenterToastHost } from "@/components/ui/TopCenterToast";
import { TEST_ENDPOINTS } from "@/shell/api/networkSettingsLogic";
import { ACCOUNT_METAS, getAccountStateLabel } from "@/shell/api/accountLogic";
import { StatusStateCard } from "@/shell/components/StatusStateCard";

export interface DesktopHarnessProps {
  dark?: boolean;
  onToggleDark?: () => void;
}

export function DesktopHarness({
  dark: propDark,
  onToggleDark: propToggleDark,
}: DesktopHarnessProps) {
  const { locale, t } = useI18n();
  // 从 URL hash 解析参数，确保主题优先以 URL 参数为准（避免 localStorage 残留造成浅色截图变深色）
  const getUrlTheme = (): "light" | "dark" | null => {
    if (typeof window === "undefined") return null;
    const raw = window.location.hash.replace(/^#harness\??/, "");
    const params = new URLSearchParams(raw);
    const t = params.get("theme");
    return t === "light" || t === "dark" ? t : null;
  };

  const [urlTheme, setUrlTheme] = useState<"light" | "dark" | null>(getUrlTheme);
  const [internalDark, setInternalDark] = useState<boolean>(() => {
    const t = getUrlTheme();
    if (t) return t === "dark";
    return false;
  });
  const [scene, setScene] = useState<string>("all");
  const [taskCount, setTaskCount] = useState<number>(8);
  const [resultCount, setResultCount] = useState<number>(8);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [showInplaceLogin, setShowInplaceLogin] = useState(false);
  const [harnessSettingsCategory, setHarnessSettingsCategory] = useState<SettingsCategory | null>(null);
  const [agentSubscene, setAgentSubscene] = useState<
    | "plan-pending"
    | "confirm-pending"
    | "confirm-approved"
    | "confirm-expired"
    | "conclusion"
    | "reasoning-refused"
    | "model-truncated"
    | "no-channel"
    | "conversation-full"
    | "card-dir-message"
    | "message-dir-ref"
    | null
  >(null);

  // 解析 URL 参数，支持仅靠 URL 直达任何场景与模式
  useEffect(() => {
    if (typeof window === "undefined") return;
    const raw = window.location.hash.replace(/^#harness\??/, "");
    const params = new URLSearchParams(raw);

    const themeParam = params.get("theme");
    if (themeParam === "dark") {
      setUrlTheme("dark");
      setInternalDark(true);
      document.documentElement.classList.add("dark");
    } else if (themeParam === "light") {
      setUrlTheme("light");
      setInternalDark(false);
      document.documentElement.classList.remove("dark");
    }

    if (params.has("scene")) {
      setScene(params.get("scene")!);
    }
    if (params.has("agent")) {
      setAgentSubscene(params.get("agent") as any);
    }
    if (params.has("settings")) {
      setHarnessSettingsCategory((params.get("settings") as SettingsCategory) || "models");
    }
    if (params.has("tasks")) {
      setTaskCount(Number(params.get("tasks")));
    }
    if (params.has("results")) {
      setResultCount(Number(params.get("results")));
    }
    if (params.has("drawer")) {
      setDrawerOpen(params.get("drawer") === "true");
    }
  }, []);

  // 严格规则：URL theme 存在时必须以 URL 为准；不存在时才回退到 propDark 或 internalDark
  const isDark = urlTheme !== null ? urlTheme === "dark" : (propDark ?? internalDark);

  useEffect(() => {
    if (isDark) {
      document.documentElement.classList.add("dark");
    } else {
      document.documentElement.classList.remove("dark");
    }
  }, [isDark]);

  const toggleDark = () => {
    const next = !isDark;
    setUrlTheme(next ? "dark" : "light");
    setInternalDark(next);
    if (next) {
      document.documentElement.classList.add("dark");
    } else {
      document.documentElement.classList.remove("dark");
    }
    propToggleDark?.();
  };

  // 生成不同条数（0, 1, 8, 32）的场景数据
  const mockTasks = useMemo(() => {
    if (taskCount === 0) return [];
    const list: ApplicationTask[] = [];
    const statuses: TaskLifecycleStatus[] = [
      "running",
      "queued",
      "paused",
      "completed",
      "failed",
      "running",
      "completed",
      "failed",
    ];

    for (let i = 0; i < taskCount; i++) {
      const isDem = i % 3 === 0;
      const status = statuses[i % statuses.length];
      const hasTotal = status === "completed" || (status === "running" && i % 2 === 0);
      const totalBytes = hasTotal ? 2000000000 + i * 500000000 : null;
      const doneBytes =
        status === "completed"
          ? totalBytes || 1000000000
          : status === "queued"
            ? 0
            : 800000000;

      list.push({
        task_id: `harness-task-${i + 1}`,
        revision: 1,
        category: isDem ? "dem_convert" : "scene_download",
        name:
          scene === "overflow"
            ? `S1A_IW_SLC__1SDV_20240410T100122_20240410T100149_053370_06771F_C726_VERY_LONG_NAME_TEST_PADDING_1234567890_OVERFLOW_TEST_FOR_TRUNCATION_${i + 1}`
            : isDem
              ? t("shell.harness.demTaskName", { index: i + 1 })
              : t("shell.harness.sceneTaskName", { index: i + 1 }),
        provider: isDem ? "Copernicus" : i % 2 === 0 ? "NASA Earthdata" : "ASF",
        dataset: isDem ? "COP30 DEM" : "Sentinel-1",
        status,
        done_bytes: doneBytes,
        total_bytes: totalBytes,
        speed_formatted: status === "running" ? "12.4 MB/s" : undefined,
        output_dir:
          scene === "overflow"
            ? "F:\\Very\\Long\\Path\\With\\Many\\Nested\\Directories\\InSAR_Data_Storage_Root_Repository_20261024\\Sentinel1_Interferometry_Project_Files_For_Earthquake_Deformation_Mapping\\"
            : `F:/InSAR_Data/batch_${i + 1}`,
        error:
          status === "failed"
            ? t("shell.harness.fixtureError")
            : null,
        created_at: "2026-10-24T02:00:00Z",
        updated_at: "2026-10-24T02:30:00Z",
      });
    }
    return list;
  }, [taskCount, scene, t]);

  // 生成不同条数（0, 1, 8, 32）的检索结果数据
  const mockResults = useMemo(() => {
    if (resultCount === 0) return [];
    return Array.from({ length: resultCount }).map((_, idx) => ({
      scene_id:
        scene === "overflow"
          ? `S1A_IW_SLC__1SDV_20240410T100122_20240410T100149_053370_06771F_OVERFLOW_LONG_PRODUCT_NAME_HASH_ABCDEF1234567890_${idx + 1}`
          : `S1A_IW_SLC__1SDV_20240410T100122_${idx + 1}`,
      date: `2024-04-${String(10 + (idx % 15)).padStart(2, "0")}`,
      orbit: 53370 + idx,
      size_mb: 4200,
    }));
  }, [resultCount, scene]);

  return (
    <div
      className={cn(
        "sp-modern min-h-screen w-full bg-sp-canvas text-sp-text select-none",
        isDark && "dark"
      )}
    >
      {/* 顶部显著标注「演示数据」横幅 */}
      <header className="sticky top-0 z-40 flex flex-wrap items-center justify-between border-b border-sp-line bg-sp-surface px-4 py-2 shadow-card gap-3">
        <div className="flex items-center gap-3 shrink-0">
          <div className="flex items-center gap-2 px-3 py-1 rounded-full bg-sp-warning-bg text-sp-warning-text border border-sp-warning-text/20 shrink-0">
            <span className="h-2 w-2 rounded-full bg-sp-warning-text" />
            <span className="text-xs font-bold tracking-wide whitespace-nowrap">
              {t("shell.harness.banner")}
            </span>
          </div>
          <span className="text-xs text-sp-text-muted whitespace-nowrap hidden xl:inline">
            {t("shell.harness.fixtureNote")}
          </span>
        </div>

        {/* 控制区：场景选择、数据量切换、主题切换 (自适应折行，无横向溢出) */}
        <div className="flex flex-wrap items-center gap-2 text-xs min-w-0">
          {/* 场景直达按钮组 */}
          <div className="flex flex-wrap items-center rounded-sp-md border border-sp-line bg-sp-canvas p-0.5 shrink-0">
            {[
              { id: "all", label: t("shell.harness.scenes.all") },
              { id: "tasks", label: t("shell.harness.scenes.tasks") },
              { id: "results", label: t("shell.harness.scenes.results") },
              { id: "overflow", label: t("shell.harness.scenes.overflow") },
              { id: "states", label: t("shell.harness.scenes.states") },
              { id: "slice4", label: t("shell.harness.scenes.slice4") },
              { id: "slice5", label: t("shell.harness.scenes.slice5") },
              { id: "w52-history", label: t("shell.harness.scenes.history") },
            ].map((item) => (
              <button
                key={item.id}
                type="button"
                onClick={() => setScene(item.id)}
                className={cn(
                  "px-2.5 py-1 rounded-sp-sm font-medium transition-colors duration-fast ease-sp whitespace-nowrap shrink-0",
                  scene === item.id
                    ? "bg-sp-surface text-sp-text shadow-xs"
                    : "text-sp-text-secondary hover:text-sp-text"
                )}
              >
                {item.label}
              </button>
            ))}
          </div>

          {/* 任务数切换 */}
          <div className="flex items-center gap-1 text-xs font-mono shrink-0 flex-nowrap">
            <span className="text-sp-text-muted">{t("shell.harness.tasks")}</span>
            {[0, 1, 8, 32].map((cnt) => (
              <button
                key={cnt}
                type="button"
                onClick={() => setTaskCount(cnt)}
                className={cn(
                  "px-1.5 py-0.5 rounded-sm border shrink-0",
                  taskCount === cnt
                    ? "border-sp-accent bg-blue-50 text-sp-accent-strong dark:bg-blue-950/60"
                    : "border-sp-line bg-sp-surface text-sp-text-muted hover:text-sp-text"
                )}
              >
                {cnt}
              </button>
            ))}
          </div>

          {/* 结果数切换 */}
          <div className="flex items-center gap-1 text-xs font-mono shrink-0 flex-nowrap">
            <span className="text-sp-text-muted">{t("shell.harness.results")}</span>
            {[0, 1, 8, 32].map((cnt) => (
              <button
                key={cnt}
                type="button"
                onClick={() => setResultCount(cnt)}
                className={cn(
                  "px-1.5 py-0.5 rounded-sm border shrink-0",
                  resultCount === cnt
                    ? "border-sp-accent bg-blue-50 text-sp-accent-strong dark:bg-blue-950/60"
                    : "border-sp-line bg-sp-surface text-sp-text-muted hover:text-sp-text"
                )}
              >
                {cnt}
              </button>
            ))}
          </div>

          {/* 演示模态与工具入口组 (收纳在同一分组内，支持窄屏换行) */}
          <div className="flex items-center gap-1.5 shrink-0 flex-wrap">
            {/* 打开抽屉 */}
            <button
              type="button"
              onClick={() => setDrawerOpen(true)}
              className="flex h-7 items-center gap-1 px-2 rounded-sp-sm font-medium bg-sp-surface border border-sp-line hover:bg-sp-hover text-sp-text transition-colors duration-fast ease-sp whitespace-nowrap shrink-0 cursor-pointer"
            >
              <Activity className="h-3.5 w-3.5 text-sp-accent" />
              {t("shell.harness.downloadDrawer")}
            </button>

            {/* 打开就地登录 */}
            <button
              type="button"
              onClick={() => setShowInplaceLogin(true)}
              className="flex h-7 items-center gap-1 px-2 rounded-sp-sm font-medium bg-sp-surface border border-sp-line hover:bg-sp-hover text-sp-text transition-colors duration-fast ease-sp whitespace-nowrap shrink-0 cursor-pointer"
            >
              <KeyRound className="h-3.5 w-3.5 text-sp-accent" />
              {t("shell.harness.inplaceLogin")}
            </button>

            {/* 打开设置模态窗 */}
            <button
              type="button"
              onClick={() => setHarnessSettingsCategory("auth")}
              className="flex h-7 items-center gap-1 px-2 rounded-sp-sm font-medium bg-sp-surface border border-sp-line hover:bg-sp-hover text-sp-text transition-colors duration-fast ease-sp whitespace-nowrap shrink-0 cursor-pointer"
            >
              <Globe className="h-3.5 w-3.5 text-sp-accent" />
              {t("shell.harness.preferences")}
            </button>

            {/* 主题切换 */}
            <button
              type="button"
              onClick={toggleDark}
              className="flex h-7 w-7 items-center justify-center rounded-sp-sm border border-sp-line bg-sp-surface hover:bg-sp-hover text-sp-text transition-colors duration-fast ease-sp shrink-0 cursor-pointer"
              title={isDark ? t("shell.harness.lightTheme") : t("shell.harness.darkTheme")}
              aria-label={isDark ? t("shell.harness.lightTheme") : t("shell.harness.darkTheme")}
            >
              {isDark ? <Sun className="h-3.5 w-3.5 text-sp-warning-text" /> : <Moon className="h-3.5 w-3.5 text-sp-text-secondary" />}
            </button>
          </div>
        </div>
      </header>

      {/* 主体演示区域 */}
      <main className="max-w-7xl mx-auto p-6 space-y-8">
        {/* 区域 A：基线 §4 第 8 条五种固定状态 */}
        {(scene === "all" || scene === "states") && (
          <section className="space-y-3">
            <div className="flex items-center gap-2">
              <h2 className="text-sm font-bold text-sp-text">
                {t("shell.harness.statesTitle")}
              </h2>
              <span className="text-xs text-sp-text-muted">{t("shell.harness.statesDescription")}</span>
            </div>

            <div className="grid grid-cols-1 md:grid-cols-3 lg:grid-cols-5 gap-4">
              <StatusStateCard
                variant="empty"
                title={t("shell.harness.emptyResultsTitle")}
                description={t("shell.harness.emptyResultsDescription")}
                action={{ label: t("shell.harness.resetFilters"), onClick: () => {} }}
                className="rounded-card bg-sp-surface shadow-card border border-sp-line min-h-45"
              />
              <StatusStateCard
                variant="loading"
                title={t("shell.harness.searchingTitle")}
                description={t("shell.harness.searchingDescription")}
                action={{ label: t("shell.harness.cancelSearch"), onClick: () => {} }}
                className="rounded-card bg-sp-surface shadow-card border border-sp-line min-h-45"
              />
              <StatusStateCard
                variant="error"
                title={t("shell.harness.timeoutTitle")}
                description={t("shell.harness.timeoutDescription")}
                action={{ label: t("shell.harness.retry"), onClick: () => {}, variant: "secondary" }}
                className="rounded-card bg-sp-surface shadow-card border border-sp-line min-h-45"
              />
              <StatusStateCard
                variant="not_connected"
                title={t("shell.harness.connectingTitle")}
                description={t("shell.harness.connectingDescription")}
                className="rounded-card bg-sp-surface shadow-card border border-sp-line min-h-45"
              />
              <StatusStateCard
                variant="login_required"
                title={t("shell.harness.loginTitle")}
                description={t("shell.harness.loginDescription")}
                action={{ label: t("shell.harness.inplaceLogin"), onClick: () => {}, variant: "primary" }}
                className="rounded-card bg-sp-surface shadow-card border border-sp-line min-h-45"
              />
            </div>
          </section>
        )}

        {/* 区域 B：任务列表卡片展示 (含两区、DEM转换与影像下载) */}
        {(scene === "all" || scene === "tasks" || scene === "overflow") && (
          <section className="space-y-3">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <h2 className="text-sm font-bold text-sp-text">
                  {t("shell.harness.tasksTitle", { count: mockTasks.length })}
                </h2>
                <span className="text-xs text-sp-text-muted">
                  {t("shell.harness.tasksDescription")}
                </span>
              </div>
            </div>

            {mockTasks.length === 0 ? (
              <div className="rounded-card bg-sp-surface p-8 text-center border border-sp-line">
                <ArrowDownCircle className="h-10 w-10 text-sp-text-disabled stroke-[1.2] mx-auto mb-2" />
                <div className="text-xs font-medium text-sp-text">{t("shell.harness.emptyTasks")}</div>
                <div className="text-xs text-sp-text-muted mt-0.5">{t("shell.harness.emptyTasksHint")}</div>
              </div>
            ) : (
              <div className="sp-grid-cards">
                {mockTasks.map((task) => (
                  <div
                    key={task.task_id}
                    className="rounded-card bg-sp-surface p-4 shadow-card border border-sp-line space-y-2.5"
                  >
                    <div className="flex items-start justify-between gap-2">
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-1.5 mb-1">
                          <span className="rounded-full bg-slate-100 px-2 py-0.5 text-xs font-medium text-sp-text-secondary dark:bg-slate-800">
                            {task.category === "dem_convert" ? t("shell.harness.demConvert") : t("shell.harness.sceneDownload")}
                          </span>
                          <span className="text-xs text-sp-text-muted truncate">
                            {task.provider}
                          </span>
                        </div>
                        <h3 className="text-xs font-bold text-sp-text truncate" title={task.name}>
                          {task.name}
                        </h3>
                      </div>
                      <StatusBadge status={task.status} category={task.category} />
                    </div>

                    {/* 进度条 */}
                    <div className="w-full h-1.5 rounded-full bg-slate-100 overflow-hidden dark:bg-slate-800">
                      {task.status === "completed" ? (
                        <div className="h-full w-full bg-sp-success-text rounded-full" />
                      ) : task.total_bytes ? (
                        <div
                          className="h-full bg-sp-accent-strong rounded-full"
                          style={{
                            width: `${Math.min(100, Math.round((task.done_bytes / task.total_bytes) * 100))}%`,
                          }}
                        />
                      ) : task.status === "running" ? (
                        <div className="h-full w-1/3 bg-sp-accent rounded-full progress-indeterminate-bar" />
                      ) : null}
                    </div>

                    <div className="flex items-center justify-between text-xs text-sp-text-muted font-mono truncate">
                      <span>
                        {task.total_bytes
                          ? `${(task.done_bytes / 1024 / 1024).toFixed(1)} MB / ${(task.total_bytes / 1024 / 1024).toFixed(1)} MB`
                          : t("shell.harness.transferredUnknown", { amount: (task.done_bytes / 1024 / 1024).toFixed(1) })}
                      </span>
                      <span>{task.speed_formatted || "—"}</span>
                    </div>

                    {task.output_dir && (
                      <div
                        className="text-xs font-mono text-sp-text-disabled truncate"
                        title={task.output_dir}
                      >
                        {t("shell.harness.path")} {task.output_dir}
                      </div>
                    )}
                  </div>
                ))}
              </div>
            )}
          </section>
        )}

        {/* 区域 C：检索结果列表展示 */}
        {(scene === "all" || scene === "results" || scene === "overflow") && (
          <section className="space-y-3">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <h2 className="text-sm font-bold text-sp-text">
                  {t("shell.harness.resultsTitle", { count: mockResults.length })}
                </h2>
                <span className="text-xs text-sp-text-muted">
                  {t("shell.harness.resultsDescription")}
                </span>
              </div>
            </div>

            {mockResults.length === 0 ? (
              <div className="rounded-card bg-sp-surface p-8 text-center border border-sp-line">
                <Layers className="h-10 w-10 text-sp-text-disabled stroke-[1.2] mx-auto mb-2" />
                <div className="text-xs font-medium text-sp-text">{t("shell.harness.emptyResults")}</div>
              </div>
            ) : (
              <div className="rounded-card bg-sp-surface shadow-card border border-sp-line overflow-hidden">
                <div className="max-h-80 overflow-y-auto overflow-x-auto">
                  <table className="w-full text-left text-xs border-collapse">
                    <thead className="sticky top-0 bg-sp-surface-muted text-sp-text-secondary border-b border-sp-line">
                      <tr>
                        <th className="py-2.5 px-3 font-semibold">{t("shell.harness.productId")}</th>
                        <th className="py-2.5 px-3 font-semibold">{t("shell.harness.date")}</th>
                        <th className="py-2.5 px-3 font-semibold">{t("shell.harness.absoluteOrbit")}</th>
                        <th className="py-2.5 px-3 font-semibold">{t("shell.harness.size")}</th>
                        <th className="py-2.5 px-3 font-semibold text-right">{t("shell.harness.action")}</th>
                      </tr>
                    </thead>
                    <tbody className="divide-y divide-sp-divider">
                      {mockResults.map((r, idx) => (
                        <tr key={idx} className="hover:bg-sp-hover transition-colors duration-fast ease-sp">
                          <td className="py-2.5 px-3 font-mono text-sp-text max-w-xs truncate" title={r.scene_id}>
                            {r.scene_id}
                          </td>
                          <td className="py-2.5 px-3 font-mono text-sp-text-secondary">{r.date}</td>
                          <td className="py-2.5 px-3 font-mono text-sp-text-secondary">{r.orbit}</td>
                          <td className="py-2.5 px-3 font-mono text-sp-text-secondary">{r.size_mb} MB</td>
                          <td className="py-2.5 px-3 text-right">
                            <button
                              type="button"
                              className="px-2 py-1 rounded-sp-sm text-xs font-medium border border-sp-line hover:bg-sp-hover text-sp-text"
                            >
                              {t("shell.harness.download")}
                            </button>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </div>
            )}
          </section>
        )}

        {/* 区域 D：工作台调试 专用场景（五态账号卡、测试连接逐端点结果、就地登录） */}
        {(scene === "all" || scene === "slice4") && (
          <section className="space-y-6">
            <div className="flex items-center justify-between border-b border-sp-divider pb-2">
              <div className="flex items-center gap-2">
                <h2 className="text-sm font-bold text-sp-text">
                  {t("shell.harness.slice4Title")}
                </h2>
                <span className="rounded-full bg-blue-50 px-2 py-0.5 text-xs font-semibold text-sp-accent-strong dark:bg-blue-950/60">
                  {t("shell.harness.unifiedContract")}
                </span>
              </div>
              <div className="flex items-center gap-2">
                <button
                  type="button"
                  onClick={() => setShowInplaceLogin(true)}
                  className="cred-action-btn"
                >
                  <KeyRound className="h-3.5 w-3.5" />
                  <span>{t("shell.harness.loginGuide")}</span>
                </button>
              </div>
            </div>

            {/* 子区 1：五态账号卡展示 */}
            <div className="space-y-2">
              <div className="text-xs font-semibold text-sp-text">
                {t("shell.harness.accountStatesTitle")}
              </div>
              <div className="sp-grid-cards">
                {[
                  { state: "logged_in" as AccountState, name: "Copernicus Data Space (CDSE)", desc: t("shell.harness.loggedInDescription") },
                  { state: "login_required" as AccountState, name: "ISRO Bhoonidhi Portal", desc: t("shell.harness.nisarLoginDescription") },
                  { state: "credentials_missing" as AccountState, name: "USGS EarthExplorer / M2M", desc: t("shell.harness.landsatCredentialsDescription") },
                  { state: "service_unavailable" as AccountState, name: t("shell.harness.gateway"), desc: t("shell.harness.gatewayDescription") },
                  { state: "not_required" as AccountState, name: "Microsoft Planetary Computer (MPC)", desc: t("shell.harness.anonymousDescription") },
                ].map((item, idx) => (
                  <div
                    key={idx}
                    className="rounded-card bg-sp-card p-4 shadow-card border border-sp-line space-y-2"
                  >
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-xs font-bold text-sp-text truncate">{item.name}</span>
                      {item.state === "logged_in" && (
                        <span className="inline-flex items-center gap-1 rounded-full bg-sp-success-bg px-2 py-0.5 text-xs font-semibold text-sp-success-text">
                          <CheckCircle2 className="h-3 w-3" />
                          {t("shell.harness.loggedIn")}
                        </span>
                      )}
                      {item.state === "login_required" && (
                        <span className="inline-flex items-center gap-1 rounded-full bg-sp-warning-bg px-2 py-0.5 text-xs font-semibold text-sp-warning-text">
                          <LogIn className="h-3 w-3" />
                          {t("shell.harness.loginRequired")}
                        </span>
                      )}
                      {item.state === "credentials_missing" && (
                        <span className="inline-flex items-center gap-1 rounded-full bg-slate-100 px-2 py-0.5 text-xs font-semibold text-sp-text-secondary dark:bg-slate-800">
                          <KeyRound className="h-3 w-3" />
                          {t("shell.harness.credentialsMissing")}
                        </span>
                      )}
                      {item.state === "service_unavailable" && (
                        <span className="inline-flex items-center gap-1 rounded-full bg-sp-danger-bg px-2 py-0.5 text-xs font-semibold text-sp-danger-text">
                          <AlertCircle className="h-3 w-3" />
                          {t("shell.harness.unavailable")}
                        </span>
                      )}
                      {item.state === "not_required" && (
                        <span className="inline-flex items-center gap-1 rounded-full bg-slate-100 px-2 py-0.5 text-xs font-semibold text-sp-text-secondary dark:bg-slate-800">
                          <Globe className="h-3 w-3" />
                          {t("shell.harness.noAuthentication")}
                        </span>
                      )}
                    </div>
                    <p className="text-xs text-sp-text-muted">{item.desc}</p>
                  </div>
                ))}
              </div>
            </div>

            {/* 子区 2：测试连接逐端点结果态展示 */}
            <div className="space-y-2">
              <div className="text-xs font-semibold text-sp-text">
                {t("shell.harness.endpointTestTitle")}
              </div>
              <div className="rounded-card bg-sp-card p-4 shadow-card border border-sp-line space-y-2">
                {TEST_ENDPOINTS.map((ep, idx) => {
                  const isBhoonidhi = ep.id === "bhoonidhi";
                  return (
                    <div
                      key={ep.id}
                      className="flex items-center justify-between rounded-sp-md bg-sp-surface p-3 text-xs border border-sp-line"
                    >
                      <div className="min-w-0 pr-2">
                        <div className="font-semibold text-sp-text truncate">{ep.name}</div>
                        <div className="font-mono text-xs text-sp-text-muted truncate">
                          {ep.site}
                        </div>
                      </div>
                      <div className="flex items-center gap-3 shrink-0">
                        <span className="font-mono text-xs text-sp-text-secondary">
                          {isBhoonidhi ? 1500 : 42 + idx * 18} ms
                        </span>
                        {isBhoonidhi ? (
                          <span className="inline-flex items-center gap-1 rounded-full bg-sp-danger-bg px-2 py-0.5 text-xs font-medium text-sp-danger-text">
                            <AlertCircle className="h-3 w-3" />
                            {t("shell.harness.failed")}
                          </span>
                        ) : (
                          <span className="inline-flex items-center gap-1 rounded-full bg-sp-success-bg px-2 py-0.5 text-xs font-medium text-sp-success-text">
                            <CheckCircle2 className="h-3 w-3" />
                            {t("shell.harness.succeeded")}
                          </span>
                        )}
                      </div>
                    </div>
                  );
                })}
              </div>
            </div>
          </section>
        )}

        {/* 切片 5：Agent 工作台与助手栏场景 (真实组件渲染) */}
        {(scene === "all" || scene === "slice5") && (
          <section className="space-y-4">
            <div className="flex flex-wrap items-center justify-between gap-2 border-b border-sp-line pb-2">
              <div className="flex items-center gap-2">
                <span className="text-sm font-bold text-sp-text">
                  {t("shell.harness.slice5Title")}
                </span>
                <span className="rounded-full bg-blue-50 px-2 py-0.5 text-xs font-semibold text-sp-accent-strong dark:bg-blue-950/60">
                  {t("shell.harness.layoutContract")}
                </span>
              </div>

              {/* 子场景切换栏 (URL agent 参数对应) */}
              <div className="flex flex-wrap items-center gap-1.5 text-xs">
                {[
                  { id: null, label: t("shell.harness.standardScene") },
                  { id: "conversation-full", label: t("shell.harness.conversationFull") },
                  { id: "plan-pending", label: t("shell.harness.planPending") },
                  { id: "confirm-pending", label: t("shell.harness.confirmPending") },
                  { id: "confirm-approved", label: t("shell.harness.confirmApproved") },
                  { id: "confirm-expired", label: t("shell.harness.confirmExpired") },
                  { id: "card-dir-message", label: t("shell.harness.confirmDirectory") },
                  { id: "message-dir-ref", label: t("shell.harness.directoryPlaceholder") },
                  { id: "conclusion", label: t("shell.harness.conclusion") },
                  { id: "reasoning-refused", label: t("shell.harness.reasoningRefused") },
                  { id: "model-truncated", label: t("shell.harness.truncated") },
                  { id: "no-channel", label: t("shell.harness.noChannel") },
                ].map((sub) => (
                  <button
                    key={sub.label}
                    type="button"
                    onClick={() => setAgentSubscene(sub.id as any)}
                    className={cn(
                      "px-2.5 py-1 rounded-sm text-xs font-medium transition-colors duration-fast ease-sp cursor-pointer",
                      agentSubscene === sub.id
                        ? "bg-sp-accent-fill text-white shadow-card"
                        : "bg-sp-surface text-sp-text-secondary border border-sp-line hover:bg-sp-hover"
                    )}
                  >
                    {sub.label}
                  </button>
                ))}

                <button
                  type="button"
                  onClick={() => setHarnessSettingsCategory("models")}
                  className="px-2.5 py-1 rounded-sm text-xs font-semibold bg-sp-surface text-sp-text border border-sp-line hover:bg-sp-hover cursor-pointer"
                >
                  {t("shell.harness.overlaySettings")}
                </button>
              </div>
            </div>

            {/* 真实整屏/大视窗渲染 Agent 工作台 */}
            <div className="h-[720px] w-full rounded-card border border-sp-line overflow-hidden shadow-card bg-sp-canvas">
              <DesktopAgentWorkbench
                forcedSubscene={agentSubscene}
                onOpenDownloadCenter={() => setDrawerOpen(true)}
                onOpenSettings={(cat) => setHarnessSettingsCategory(cat as SettingsCategory)}
              />
            </div>
          </section>
        )}

        {/* W5-2：历史页（真实组件） */}
        {(scene === "all" || scene === "w52-history") && (
          <section className="space-y-4">
            <div className="flex flex-col gap-2">
              <div className="flex items-center gap-2">
                <h2 className="text-base font-bold text-sp-text m-0">
                  {t("shell.harness.historyTitle")}
                </h2>
                <span className="rounded-full bg-blue-50 px-2 py-0.5 text-xs font-semibold text-sp-accent-strong dark:bg-blue-950/60 border border-sp-line">
                  {t("shell.harness.historyBadge")}
                </span>
              </div>
            </div>

            <div className="h-[720px] w-full rounded-card border border-sp-line overflow-hidden shadow-card">
              <AgentHistoryPage onOpenSession={() => {}} />
            </div>
          </section>
        )}
      </main>


      {/* 挂载真实下载中心抽屉 */}
      <DesktopDownloadDrawer
        open={drawerOpen}
        onClose={() => setDrawerOpen(false)}
        harnessData={mockTasks}
      />

      {/* 挂载就地登录卡 */}
      {showInplaceLogin && (
        <InplaceLoginCard
          error={{
            code: "LOGIN_REQUIRED",
            message_zh: t("shell.harness.loginFixtureError"),
            retryable: true,
            needs_account: "earthdata",
            needs_user_confirmation: false,
          }}
          onClose={() => setShowInplaceLogin(false)}
          onSuccess={() => {
            setShowInplaceLogin(false);
          }}
          onOpenSettings={() => {
            setShowInplaceLogin(false);
            setHarnessSettingsCategory("auth");
          }}
        />
      )}

      {/* 挂载偏好设置模态窗 */}
      <SettingsModal
        open={harnessSettingsCategory !== null}
        onClose={() => setHarnessSettingsCategory(null)}
        initialCategory={harnessSettingsCategory || "auth"}
      />
      <TopCenterToastHost />
    </div>
  );
}
