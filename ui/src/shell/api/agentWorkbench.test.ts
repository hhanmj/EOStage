import test from "node:test";
import assert from "node:assert/strict";
import {
  getConfirmCardCoreFields,
  freezeApprovalSnapshot,
  isApprovalExpired,
  verifyClaimReferences,
  determineAgentWorkbenchLayout,
  getModelChannelReadiness,
  resolveAoiSync,
} from "./agentLogic.ts";
import type {
  AgentConfirmCardData,
  AgentClaim,
} from "./agentTypes.ts";
import type { Bbox } from "../../lib/bridge.ts";

test("Agent Workbench & Slice 5 Logic Suite", async (t) => {
  await t.test("1. 2档确认卡的数据恰好四项（数据源/范围/预计大小/保存位置）", () => {
    const cardData: AgentConfirmCardData = {
      source: "Sentinel-1 GRD/SLC (ASF)",
      scope: {
        aoi_name: "大同盆地试验区",
        area_sqkm: 1240.5,
        time_range: "2024-04-01 ~ 2024-04-30",
        product_or_orbit: "SLC · 升轨",
        bbox: { west: 112.5, east: 113.8, south: 39.5, north: 40.5 },
      },
      estimated_size: "约 4.8 GB (估算)",
      save_location: "/data/sar/downloads/sentinel1",
      plan_id: "plan-s1-001",
      plan_hash: "a9f3b8c2d1e04576",
      store_revision_seen: 12,
    };

    const fields = getConfirmCardCoreFields(cardData);
    assert.deepEqual(
      fields.sort(),
      ["data_source", "estimated_size", "save_location", "spatial_temporal_scope"].sort(),
      "必须恰好提取四项核心信息，不多不少"
    );
    assert.ok(cardData.estimated_size.includes("估算"), "预计大小必须显式标明「估算」");
  });

  await t.test("2. 批准后卡片冻结（快照不变，后续数据变化不改变已批准内容）", () => {
    const cardData: AgentConfirmCardData = {
      source: "Sentinel-1 GRD/SLC (ASF)",
      scope: {
        aoi_name: "大同盆地试验区",
        time_range: "2024-04-01 ~ 2024-04-30",
        product_or_orbit: "SLC · 升轨",
      },
      estimated_size: "约 4.8 GB (估算)",
      save_location: "/data/sar/downloads/sentinel1",
      plan_id: "plan-s1-001",
      plan_hash: "a9f3b8c2d1e04576",
      store_revision_seen: 12,
    };

    const approval = freezeApprovalSnapshot(cardData, "2026-10-24T09:00:00Z");
    assert.equal(approval.decision, "approved");
    assert.equal(approval.decided_at, "2026-10-24T09:00:00Z");
    assert.equal(approval.snapshot.save_location, "/data/sar/downloads/sentinel1");

    // 模拟原对象后续被外部修改或重绑
    cardData.save_location = "/mutated/another/path";
    cardData.estimated_size = "约 99.9 GB (估算)";

    // 冻结快照必须保持不变
    assert.equal(
      approval.snapshot.save_location,
      "/data/sar/downloads/sentinel1",
      "已批准的快照保存位置不可篡改"
    );
    assert.equal(
      approval.snapshot.estimated_size,
      "约 4.8 GB (估算)",
      "已批准的快照预计大小不可篡改"
    );
  });

  await t.test("3. Store revision 变化 → 状态变「已失效」", () => {
    const cardData: AgentConfirmCardData = {
      source: "Sentinel-1 GRD/SLC (ASF)",
      scope: {
        aoi_name: "大同盆地试验区",
        time_range: "2024-04-01 ~ 2024-04-30",
        product_or_orbit: "SLC · 升轨",
      },
      estimated_size: "约 4.8 GB (估算)",
      save_location: "/data/sar/downloads/sentinel1",
      plan_id: "plan-s1-001",
      plan_hash: "a9f3b8c2d1e04576",
      store_revision_seen: 12,
    };

    const approval = freezeApprovalSnapshot(cardData, "2026-10-24T09:00:00Z");

    // revision 相同且 hash 相同 -> 未失效
    assert.equal(isApprovalExpired(approval, 12, "a9f3b8c2d1e04576"), false);

    // Store revision 增加 -> 已失效
    assert.equal(isApprovalExpired(approval, 13, "a9f3b8c2d1e04576"), true);

    // plan_hash 变动 -> 已失效
    assert.equal(isApprovalExpired(approval, 12, "different_hash"), true);
  });

  await t.test("4. 无有效引用的 claim → 「未核验」；有有效引用 → 「已核验」", () => {
    const validKnowledge = {
      taskIds: ["task-101", "task-102"],
      batchIds: ["batch-s1-202404"],
      verifiedWebDomains: ["copernicus.eu", "asf.alaska.edu"],
    };

    const claimWithValidTask: AgentClaim = {
      statement: "已创建 Sentinel-1 下载任务 #task-101",
      refs: [{ type: "task", id: "task-101", verified: false }],
    };

    const claimWithInvalidTask: AgentClaim = {
      statement: "引用的任务不存在 #task-999",
      refs: [{ type: "task", id: "task-999", verified: false }],
    };

    const claimWithWebSearch: AgentClaim = {
      statement: "公开网页检索发现相关辅助信息",
      refs: [{ type: "web", id: "random-blog.com/post", title: "第三方博客", verified: false }],
    };

    const verified1 = verifyClaimReferences(claimWithValidTask, validKnowledge);
    assert.equal(verified1.refs[0].verified, true, "存在于已知任务列表中的引用应核验为 true");

    const verified2 = verifyClaimReferences(claimWithInvalidTask, validKnowledge);
    assert.equal(verified2.refs[0].verified, false, "不存在的引用应标为 false（未核验）");

    const verified3 = verifyClaimReferences(claimWithWebSearch, validKnowledge);
    assert.equal(verified3.refs[0].verified, false, "未列入权威域名的 web_search 引用应标为 false（未核验）");
  });

  await t.test("5. 920 布局判定函数：<=920 -> tabs，>920 -> 分栏", () => {
    assert.equal(determineAgentWorkbenchLayout(1920), "columns");
    assert.equal(determineAgentWorkbenchLayout(1280), "columns");
    assert.equal(determineAgentWorkbenchLayout(921), "columns");
    assert.equal(determineAgentWorkbenchLayout(920), "tabs");
    assert.equal(determineAgentWorkbenchLayout(800), "tabs");
    assert.equal(determineAgentWorkbenchLayout(600), "tabs");
  });

  await t.test("6. 渠道未配置 -> 输入禁用状态（readiness 逻辑返回禁用）", () => {
    // 未配置渠道
    const notConfigured = getModelChannelReadiness(false, 0);
    assert.equal(notConfigured.ready, false);
    assert.equal(notConfigured.disabled, true);
    assert.ok(notConfigured.reason_zh.includes("模型渠道"), "未配置时必须返回明确的提示");

    // 已配置但模型列表为空
    const emptyModels = getModelChannelReadiness(true, 0);
    assert.equal(emptyModels.ready, false);
    assert.equal(emptyModels.disabled, true);

    // 已配置且有可用模型
    const ready = getModelChannelReadiness(true, 2);
    assert.equal(ready.ready, true);
    assert.equal(ready.disabled, false);
  });

  await t.test("7. AOI 同步：未批准「更新工作区 AOI」时工作台 AOI 保持原值，批准后才采用 Agent 的 AOI", () => {
    const workbenchAoi: Bbox = { west: 112.0, east: 113.0, south: 39.0, north: 40.0 };
    const agentAoi: Bbox = { west: 115.0, east: 116.5, south: 38.0, north: 39.2 };

    // 用户未确认点击「更新工作区 AOI」
    const kept = resolveAoiSync(workbenchAoi, agentAoi, false);
    assert.deepEqual(kept, workbenchAoi, "未批准时必须严格保持原工作台 AOI，绝不静默覆盖");

    // 用户显式点击批准「更新工作区 AOI」
    const updated = resolveAoiSync(workbenchAoi, agentAoi, true);
    assert.deepEqual(updated, agentAoi, "用户确认后采用 Agent 提议的 AOI");
  });
});
