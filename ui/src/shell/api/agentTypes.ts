/**
 * Agent 工作台核心类型定义
 * 依据 EOStage Agent 会话状态规范
 */

import type { Bbox } from "../../lib/bridge.ts";

export type AgentTier = 0 | 1 | 2;

export interface AgentToolCatalogItem {
  tool_id: string;
  name_zh: string;
  tier: AgentTier;
  description_zh: string;
}

export type AgentPlanStatus =
  | "draft"
  | "proposed"
  | "approved"
  | "expired"
  | "rejected"
  | "executing"
  | "completed"
  | "failed";

export interface AgentPlanStep {
  step_id: string;
  tool_id: string;
  tool_name_zh: string;
  tier: AgentTier;
  description_zh: string;
  status: "pending" | "running" | "completed" | "failed";
}

export interface AgentPlan {
  plan_id: string;
  session_id: string;
  run_id: string;
  plan_hash: string;
  max_tier: AgentTier;
  status: AgentPlanStatus;
  steps: AgentPlanStep[];
  store_revision_seen: number;
  created_at: string;
}

export interface AgentConfirmScope {
  aoi_name: string;
  area_sqkm?: number;
  time_range: string;
  product_or_orbit: string;
  bbox?: Bbox;
}

export interface AgentConfirmCardData {
  source: string;
  scope: AgentConfirmScope;
  estimated_size: string; // 必须标「估算」
  save_location: string;
  plan_id: string;
  plan_hash: string;
  store_revision_seen: number;
}

export interface AgentApprovalRecord {
  approval_id: string;
  plan_id: string;
  plan_hash_seen: string;
  store_revision_seen: number;
  snapshot: AgentConfirmCardData;
  decision: "approved" | "rejected";
  decided_at: string;
}

export type AgentClaimRefType = "task" | "batch" | "web";

export interface AgentClaimRef {
  type: AgentClaimRefType;
  id: string;
  title?: string;
  url?: string;
  verified: boolean;
}

export interface AgentClaim {
  statement: string;
  refs: AgentClaimRef[];
}

export interface AgentExecutionRound {
  round_id: string;
  tool_calls: Array<{
    tool_id: string;
    tool_name_zh: string;
    params_summary: string;
    result_summary: string;
    status: "success" | "error";
  }>;
}

export interface AgentTaskProgressCardData {
  task_id: string;
  task_title: string;
  status: "queued" | "running" | "paused" | "failed" | "completed";
  status_zh: string;
  progress_label: string;
  percent: number;
  speed_label?: string;
}

export interface AgentPlanResourceSummary {
  batch_id: string;
  source_title: string;
  scene_count_label: string;
  task_id: string;
  task_status_zh: string;
  output_dir: string;
}

export interface AgentMessage {
  id: string;
  sender: "user" | "agent";
  text: string;
  timestamp: string;
  plan?: AgentPlan;
  confirmCard?: AgentConfirmCardData;
  approvalRecord?: AgentApprovalRecord;
  claims?: AgentClaim[];
  executionRounds?: AgentExecutionRound[];
  taskIdRef?: string;
  progressCard?: AgentTaskProgressCardData;
}

export interface AgentSession {
  id: string;
  title: string;
  created_at: string;
  updated_at: string;
  messages: AgentMessage[];
}

export interface ModelChannelReadiness {
  ready: boolean;
  disabled: boolean;
  reason_zh: string;
}
