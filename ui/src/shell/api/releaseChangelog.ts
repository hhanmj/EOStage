/**
 * releaseChangelog.ts
 * 应用内置更新日志数据源与未读版本红点追踪（供左上角消息中心「更新日志」页签展示）。
 */
import type { SimpleStorage } from "./shellRoutes.ts";

export interface ReleaseChangelogEntry {
  version: string;
  date: string;
  title: string;
  highlights: readonly string[];
}

export const RELEASE_CHANGELOG_STORAGE_KEY = "eostage.changelog.lastSeenVersion";

export const RELEASE_CHANGELOG_ENTRIES: readonly ReleaseChangelogEntry[] = [
  {
    version: "2026.11.19",
    date: "2026-11-19",
    title: "结果清单统一、NISAR 双版本与任务核验增强",
    highlights: [
      "地图工作台与智能体共享「同幅归组」、整组全选与统一排序；点击地图多景气泡可直接定位展开对应结果。",
      "NISAR L 波段支持区分官方「Validated」与「Provisional」发布版本（默认全选检索），并全面清理内部工程代号。",
      "增强多轮会话与下载链路可靠性：旧会话工具结果向后兼容、断点续传大小校验与限流避让、细粒度连接诊断（超时/DNS/TLS/代理/流中断）。",
      "任务中心计数按伴随折叠真实对齐，新增「成果与核验」视图独立展示已落盘目录与核验状态。",
      "模型失败精简展示真实上游报错理由，确保后台下载独立运行，并支持一键「重试」（认证类异常可直达模型设置）。",
      "支持主流平台浏览器 OAuth 授权登录并按需手动触发识别渠道模型，新增四色健康状态点（绿/红/灰/橙）。",
    ],
  },
  {
    version: "2026.11.18",
    date: "2026-11-18",
    title: "多平台账号门户与离线桌面巡检修复",
    highlights: [
      "完善各遥感数据源官方注册/认证入口与本地凭据安全存储。",
      "统一左侧栏收起与展开动效，消除图标与文字闪烁问题。",
      "增强 NISAR 与光学数据裁剪范围预估与错误提示可读性。",
    ],
  },
  {
    version: "2026.11.16",
    date: "2026-11-16",
    title: "智能体协同工作台与多数据源检索上线",
    highlights: [
      "支持同一会话跨 Sentinel-1、Sentinel-2、Landsat、NISAR 等多数据源并存检索与联合定位。",
      "支持「思考中」折叠时间线与流式输出，支持运行中继续发送排队/引导消息。",
    ],
  },
];

export function getLatestReleaseVersion(): string {
  return RELEASE_CHANGELOG_ENTRIES[0]?.version ?? "2026.11.19";
}

function getDefaultStorage(): SimpleStorage | null {
  try {
    return typeof window !== "undefined" && window.localStorage ? window.localStorage : null;
  } catch {
    return null;
  }
}

export function readLastSeenReleaseVersion(storage?: SimpleStorage | null): string | null {
  const s = storage ?? getDefaultStorage();
  if (!s) return null;
  try {
    const val = s.getItem(RELEASE_CHANGELOG_STORAGE_KEY);
    return val && val.trim() ? val.trim() : null;
  } catch {
    return null;
  }
}

export function hasUnreadReleaseNotice(storage?: SimpleStorage | null): boolean {
  const seen = readLastSeenReleaseVersion(storage);
  return seen !== getLatestReleaseVersion();
}

const changelogListeners = new Set<() => void>();

export function subscribeReleaseChangelog(cb: () => void): () => void {
  changelogListeners.add(cb);
  return () => {
    changelogListeners.delete(cb);
  };
}

export function markReleaseChangelogSeen(storage?: SimpleStorage | null): void {
  const s = storage ?? getDefaultStorage();
  if (!s) return;
  try {
    s.setItem(RELEASE_CHANGELOG_STORAGE_KEY, getLatestReleaseVersion());
    for (const cb of changelogListeners) cb();
  } catch {
    // Ignore quota or restricted storage errors
  }
}
