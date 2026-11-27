/**
 * Agent 业务逻辑与纯函数模块
 * 依据 EOStage Agent 会话状态规范
 */

import type { Bbox } from "../../lib/bridge.ts";
import { t } from "../../i18n/index.ts";
import type {
  AgentApprovalRecord,
  AgentClaim,
  AgentConfirmCardData,
  AgentToolCatalogItem,
  ModelChannelReadiness,
} from "./agentTypes.ts";

/**
 * A1 §3 + Agent 2.0 工具目录定义（18 个权威目录工具，名称随语言动态响应）
 */
export const AGENT_TOOL_CATALOG: Record<string, AgentToolCatalogItem> = {
  service_readiness: {
    tool_id: "service_readiness",
    get name_zh() {
      return t("agent.tools.serviceReadiness");
    },
    tier: 0,
    get description_zh() {
      return t("agent.tools.serviceReadiness");
    },
  },
  capabilities_get: {
    tool_id: "capabilities_get",
    get name_zh() {
      return t("agent.tools.capabilitiesGet");
    },
    tier: 0,
    get description_zh() {
      return t("agent.tools.capabilitiesGet");
    },
  },
  working_aoi_get: {
    tool_id: "working_aoi_get",
    get name_zh() {
      return t("agent.tools.workingAoiGet");
    },
    tier: 0,
    get description_zh() {
      return t("agent.tools.workingAoiGet");
    },
  },
  admin_boundary_search: {
    tool_id: "admin_boundary_search",
    get name_zh() {
      return t("agent.tools.adminBoundarySearch");
    },
    tier: 0,
    get description_zh() {
      return t("agent.tools.adminBoundarySearch");
    },
  },
  working_aoi_set: {
    tool_id: "working_aoi_set",
    get name_zh() {
      return t("agent.tools.workingAoiSet");
    },
    tier: 1,
    get description_zh() {
      return t("agent.tools.workingAoiSet");
    },
  },
  workbench_page: {
    tool_id: "workbench_page",
    get name_zh() {
      return t("agent.tools.workbenchPage");
    },
    tier: 0,
    get description_zh() {
      return t("agent.tools.workbenchPage");
    },
  },
  task_page: {
    tool_id: "task_page",
    get name_zh() {
      return t("agent.tools.taskPage");
    },
    tier: 0,
    get description_zh() {
      return t("agent.tools.taskPage");
    },
  },
  task_event_page: {
    tool_id: "task_event_page",
    get name_zh() {
      return t("agent.tools.taskEventPage");
    },
    tier: 0,
    get description_zh() {
      return t("agent.tools.taskEventPage");
    },
  },
  credential_readiness: {
    tool_id: "credential_readiness",
    get name_zh() {
      return t("agent.tools.credentialReadiness");
    },
    tier: 0,
    get description_zh() {
      return t("agent.tools.credentialReadiness");
    },
  },
  update_download_task: {
    tool_id: "update_download_task",
    get name_zh() {
      return t("agent.tools.updateDownloadTask");
    },
    tier: 2,
    get description_zh() {
      return t("agent.tools.updateDownloadTask");
    },
  },
  source_list: {
    tool_id: "source_list",
    get name_zh() {
      return t("agent.tools.sourceList");
    },
    tier: 0,
    get description_zh() {
      return t("agent.tools.sourceList");
    },
  },
  account_readiness: {
    tool_id: "account_readiness",
    get name_zh() {
      return t("agent.tools.accountReadiness");
    },
    tier: 0,
    get description_zh() {
      return t("agent.tools.accountReadiness");
    },
  },
  task_overview: {
    tool_id: "task_overview",
    get name_zh() {
      return t("agent.tools.taskOverview");
    },
    tier: 0,
    get description_zh() {
      return t("agent.tools.taskOverview");
    },
  },
  batch_page: {
    tool_id: "batch_page",
    get name_zh() {
      return t("agent.tools.batchPage");
    },
    tier: 0,
    get description_zh() {
      return t("agent.tools.batchPage");
    },
  },
  search_progress: {
    tool_id: "search_progress",
    get name_zh() {
      return t("agent.tools.searchProgress");
    },
    tier: 0,
    get description_zh() {
      return t("agent.tools.searchProgress");
    },
  },
  search_start: {
    tool_id: "search_start",
    get name_zh() {
      return t("agent.tools.searchStart");
    },
    tier: 1,
    get description_zh() {
      return t("agent.tools.searchStart");
    },
  },
  create_task: {
    tool_id: "create_task",
    get name_zh() {
      return t("agent.tools.createTask");
    },
    tier: 2,
    get description_zh() {
      return t("agent.tools.createTask");
    },
  },
  task_action: {
    tool_id: "task_action",
    get name_zh() {
      return t("agent.tools.taskAction");
    },
    tier: 2,
    get description_zh() {
      return t("agent.tools.taskAction");
    },
  },
};

export function resolveAgentToolDisplayName(toolId: string): string {
  const catalogEntry = AGENT_TOOL_CATALOG[toolId];
  if (catalogEntry) {
    return catalogEntry.name_zh;
  }
  if (toolId === "ask_user_question") {
    return t("agent.tools.askUserQuestion");
  }
  if (toolId === "map_workspace_action") {
    return t("agent.tools.mapWorkspaceAction");
  }
  switch (toolId) {
    case "gee_catalog_search":
      return t("agent.tools.geeCatalogSearch");
    case "gee_dataset_advisor":
      return t("agent.tools.geeDatasetAdvisor");
    case "gee_export_preflight":
      return t("agent.tools.geeExportPreflight");
    case "gee_check_availability":
      return t("agent.tools.geeCheckAvailability");
    case "gee_cloud_analyze":
      return t("agent.tools.geeCloudAnalyze");
    case "gee_chunked_download":
      return t("agent.tools.geeChunkedDownload");
    case "gee_script_inspect":
      return t("agent.tools.geeScriptInspect");
    case "gee_quota_status":
      return t("agent.tools.geeQuotaStatus");
    case "gis_toolchains_status":
      return t("agent.tools.gisToolchainsStatus");
    case "gis_local_process":
      return t("agent.tools.gisLocalProcess");
    case "gis_external_execute":
      return t("agent.tools.gisExternalExecute");
    case "skill_read":
      return t("agent.tools.skillRead");
    case "nisar_subset_preflight":
      return t("agent.tools.nisarSubsetPreflight");
    case "nisar_create_subset_task":
      return t("agent.tools.nisarCreateSubsetTask");
    case "propose_plan":
      return t("agent.tools.proposePlan");
    case "resolve_aoi":
      return t("agent.tools.workingAoiSet");
    case "global_disaster_search":
      return t("agent.tools.globalDisasterSearch");
    case "aoi_validate_boundary":
      return t("agent.tools.aoiValidateBoundary");
    case "application_sar_search_and_freeze":
      return t("agent.tools.searchStart");
    default: {
      if (toolId.startsWith("mcp_") || toolId.startsWith("mcp__")) {
        const cleaned = toolId
          .replace(/^mcp_+/, "")
          .replace(/_+/g, " ")
          .trim();
        return cleaned ? `MCP · ${cleaned}` : "MCP 外部连接器";
      }
      return toolId;
    }
  }
}

/**
 * 1. 2 档确认卡四项验证与提取函数
 * 确认卡恰好四项：数据源 / 范围 / 预计大小（标「估算」） / 保存位置
 */
export function getConfirmCardCoreFields(data: AgentConfirmCardData): string[] {
  const fields: string[] = [];
  if (data.source && data.source.trim().length > 0) {
    fields.push("data_source");
  }
  if (data.scope && (data.scope.aoi_name || data.scope.time_range || data.scope.product_or_orbit)) {
    fields.push("spatial_temporal_scope");
  }
  if (data.estimated_size && data.estimated_size.trim().length > 0) {
    fields.push("estimated_size");
  }
  if (data.save_location && data.save_location.trim().length > 0) {
    fields.push("save_location");
  }
  return fields;
}

/**
 * 2. 批准后卡片冻结
 * 批准后快照深拷贝冻结、不可纂改
 */
export function freezeApprovalSnapshot(
  cardData: AgentConfirmCardData,
  decisionTime: string
): AgentApprovalRecord {
  const deepCopiedSnapshot: AgentConfirmCardData = {
    source: cardData.source,
    scope: {
      aoi_name: cardData.scope.aoi_name,
      area_sqkm: cardData.scope.area_sqkm,
      time_range: cardData.scope.time_range,
      product_or_orbit: cardData.scope.product_or_orbit,
      bbox: cardData.scope.bbox ? { ...cardData.scope.bbox } : undefined,
    },
    estimated_size: cardData.estimated_size,
    save_location: cardData.save_location,
    plan_id: cardData.plan_id,
    plan_hash: cardData.plan_hash,
    store_revision_seen: cardData.store_revision_seen,
  };

  return {
    approval_id: `appr-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
    plan_id: cardData.plan_id,
    plan_hash_seen: cardData.plan_hash,
    store_revision_seen: cardData.store_revision_seen,
    snapshot: deepCopiedSnapshot,
    decision: "approved",
    decided_at: decisionTime,
  };
}

/**
 * 3. Store revision 变化与失效判定
 * 规则：执行前比对 plan_hash 与 store_revision，任一变化即失效
 */
export function isApprovalExpired(
  record: AgentApprovalRecord,
  currentStoreRevision: number,
  currentPlanHash?: string
): boolean {
  if (record.store_revision_seen !== currentStoreRevision) {
    return true;
  }
  if (currentPlanHash && record.plan_hash_seen !== currentPlanHash) {
    return true;
  }
  return false;
}

/**
 * 4. Claim 引用有效性核验
 * 有效引用（已知任务 ID、已知批次 ID、权威已知域名）显示「已核验」，否则灰色「未核验」
 */
export function verifyClaimReferences(
  claim: AgentClaim,
  validKnowledge: {
    taskIds: string[];
    batchIds: string[];
    verifiedWebDomains: string[];
  }
): AgentClaim {
  const verifiedRefs = claim.refs.map((ref) => {
    let isValid = false;
    if (ref.type === "task") {
      isValid = validKnowledge.taskIds.includes(ref.id);
    } else if (ref.type === "batch") {
      isValid = validKnowledge.batchIds.includes(ref.id);
    } else if (ref.type === "web") {
      const lowerId = (ref.url || ref.id).toLowerCase();
      isValid = validKnowledge.verifiedWebDomains.some((d) => lowerId.includes(d.toLowerCase()));
    }
    return {
      ...ref,
      verified: isValid,
    };
  });

  return {
    statement: claim.statement,
    refs: verifiedRefs,
  };
}

/**
 * 5. 920 宽响应式布局判定
 * 窗口 <=920 宽：中/右变为两个标签页「对话与确认」/「地图与结果」；>920 分栏
 */
export function determineAgentWorkbenchLayout(viewportWidth: number): "tabs" | "columns" {
  return viewportWidth <= 920 ? "tabs" : "columns";
}

/**
 * 6. 模型渠道就绪判定
 * 渠道未配置时：输入框禁用 + 一行提示 + 前往设置链接；绝不给假回复
 */
export function getModelChannelReadiness(
  configured: boolean,
  activeModelCount: number
): ModelChannelReadiness {
  if (!configured || activeModelCount <= 0) {
    return {
      ready: false,
      disabled: true,
      reason_zh: t("agent.channels.unconfigured"),
    };
  }
  return {
    ready: true,
    disabled: false,
    reason_zh: "",
  };
}

/**
 * 7. AOI 弱同步裁决
 * Agent 在另一区域检索的结果绝不改写工作台 AOI，除非用户在卡片上显式点击「更新工作区 AOI」
 */
export function resolveAoiSync(
  currentWorkbenchAoi: Bbox | null,
  agentProposedAoi: Bbox,
  userConfirmedUpdate: boolean
): Bbox | null {
  if (userConfirmedUpdate) {
    return agentProposedAoi;
  }
  return currentWorkbenchAoi;
}
