import React from "react";
import {
  Send,
  MessageSquare,
} from "lucide-react";
import type { Bbox, SceneRow } from "@/lib/bridge";
import { StatusStateCard } from "@/shell/components/StatusStateCard";

export interface InterferometricPairPreview {
  direction: "升轨" | "降轨";
  track: number;
  primaryDate: string;
  secondaryDate: string;
  bperp: number;
  btemp: number;
  coherenceEstimate: number;
}

export type ArtifactSource = "demo" | "agent_runtime" | "catalogue";

export interface AgentSpatialArtifact {
  aoiName: string;
  bbox: Bbox;
  recommendedSensor: string;
  recommendedOrbit: "ascending" | "descending" | "both";
  matchedCount: number;
  recommendedPairsCount: number;
  interferometricPairs?: InterferometricPairPreview[];
  recommendedDem?: string;
  recommendedOrbitProduct?: string;
  suggestedScenes?: SceneRow[];
  source: ArtifactSource;
}

export interface DesktopAgentDrawerProps {
  currentAoiName?: string;
  currentAoiBbox?: Bbox | null;
  onApplySpatialArtifact?: (artifact: AgentSpatialArtifact) => void;
  onFocusMapBbox?: (bbox: Bbox) => void;
  onOneClickPipelineExecute?: (artifact: AgentSpatialArtifact) => void;
}

export function DesktopAgentDrawer({
  currentAoiName = "全球视图",
}: DesktopAgentDrawerProps) {
  return (
    <div className="flex h-full w-[380px] shrink-0 flex-col bg-sp-surface-muted border-r border-sp-line select-none z-20">
      {/* 抽屉头部 */}
      <div className="flex h-12 shrink-0 items-center justify-between border-b border-sp-line bg-sp-surface px-4">
        <div className="flex items-center gap-2">
          <div className="flex h-7 w-7 items-center justify-center rounded-sm border border-sp-line bg-sp-surface-muted text-sp-text-secondary">
            <MessageSquare className="h-4 w-4" />
          </div>
          <div>
            <div className="flex items-center gap-1.5">
              <span className="text-xs font-bold text-sp-text">
                智能助手
              </span>
              <span className="rounded-full bg-slate-100 px-1.5 py-0.2 font-mono text-xs font-semibold text-sp-text-muted dark:bg-slate-800">
                未接入
              </span>
            </div>
            <div className="text-xs text-sp-text-muted truncate max-w-[180px]">
              当前工作区: {currentAoiName}
            </div>
          </div>
        </div>
      </div>

      {/* 诚实的状态展示区：采用共享五态组件 */}
      <div className="flex-1 overflow-y-auto p-4 flex flex-col items-center justify-center">
        <StatusStateCard
          variant="not_connected"
          title="Agent 尚未接入"
          description="智能助手核心服务接口正在适配中。请使用左侧「科学参数工作台」配置卫星参数并检索权威数据。"
          className="w-full rounded-card border border-sp-line bg-sp-surface shadow-card"
        />
      </div>

      {/* 底部输入框（禁用态） */}
      <div className="p-3 border-t border-sp-line bg-sp-surface">
        <div className="relative flex items-center">
          <input
            type="text"
            disabled
            placeholder="Agent 尚未接入，输入已禁用"
            className="h-sp-row w-full rounded-sm border border-sp-line bg-slate-50 px-3 pr-10 text-xs text-sp-text-disabled outline-none cursor-not-allowed dark:bg-slate-900"
          />
          <button
            type="button"
            disabled
            aria-label="发送消息"
            className="absolute right-1.5 flex h-6 w-6 items-center justify-center rounded-sm bg-slate-200 text-sp-text-disabled cursor-not-allowed dark:bg-slate-800"
          >
            <Send className="h-3.5 w-3.5" />
          </button>
        </div>
      </div>
    </div>
  );
}
