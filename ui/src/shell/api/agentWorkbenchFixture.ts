/**
 * __fixture
 * Agent 工作台演示模式数据源（仅供 DEV 开发与 Harness 演示使用，严禁流入生产构建）
 */

import type {
  AgentStreamEvent,
  AgentSessionSummary,
  AgentRunSummary,
  AgentRunDetail,
  AgentChannelView,
  AgentPermissionModeView,
  AgentStopAllResult,
} from "../../lib/generated/v3-contracts.ts";

export function createFixtureEvents__fixture(
  subscene: string | null,
  sessionId?: string | null
): AgentStreamEvent[] {
  const baseEvents: AgentStreamEvent[] = [
    {
      seq: 1,
      kind: "message",
      payload: {
        role: "user",
        run_id: "fixture-run-01",
        text: "请检索大同盆地试验区 2024 年 4 月的 Sentinel-1 升轨数据并准备下载。",
        context: null,
      },
    },
    {
      seq: 2,
      kind: "tool_round",
      payload: {
        run_id: "fixture-run-01",
        call_id: "call-1",
        tool_id: "resolve_aoi",
        status: "done",
      },
    },
    {
      seq: 3,
      kind: "tool_round",
      payload: {
        run_id: "fixture-run-01",
        call_id: "call-2",
        tool_id: "application_sar_search_and_freeze",
        status: "done",
      },
    },
    {
      seq: 4,
      kind: "message",
      payload: {
        role: "assistant",
        run_id: "fixture-run-01",
        text: "已完成试验区地理范围解析与权威端点元数据检索。已冻结批次并生成执行计划：",
        context: null,
      },
    },
  ];

  if (subscene === "conversation-full" || (!subscene && sessionId === "fixture-sess-01")) {
    return [
      {
        seq: 1,
        kind: "message",
        payload: {
          role: "user",
          run_id: "fixture-run-01",
          text: "检索四川茂县叠溪镇 2024 年 5 月的 Sentinel-1 SLC 数据并创建下载。",
          context: null,
        },
      },
      {
        seq: 2,
        kind: "tool_round",
        payload: {
          run_id: "fixture-run-01",
          call_id: "call-1",
          tool_id: "working_aoi_get",
          status: "done",
          started_at: "2026-10-26T10:00:00Z",
          duration_ms: 120,
        },
      },
      {
        seq: 3,
        kind: "tool_round",
        payload: {
          run_id: "fixture-run-01",
          call_id: "call-2",
          tool_id: "search_start",
          status: "done",
          started_at: "2026-10-26T10:00:01Z",
          duration_ms: 1500,
          summary_zh: "已向 ASF 发起检索请求",
        },
      },
      {
        seq: 4,
        kind: "tool_round",
        payload: {
          run_id: "fixture-run-01",
          call_id: "call-3",
          tool_id: "search_progress",
          status: "done",
          started_at: "2026-10-26T10:00:03Z",
          duration_ms: 820,
          result_count: 5,
          summary_zh: "检索完成，保留 5 景产品",
        },
      },
      {
        seq: 5,
        kind: "tool_round",
        payload: {
          run_id: "fixture-run-01",
          call_id: "call-4",
          tool_id: "batch_page",
          status: "done",
          started_at: "2026-10-26T10:00:04Z",
          duration_ms: 350,
          result_count: 5,
        },
      },
      {
        seq: 6,
        kind: "message",
        payload: {
          role: "assistant",
          run_id: "fixture-run-01",
          text: "已完成空间范围解析与权威端点元数据检索。已冻结批次并生成执行计划：",
          context: null,
        },
      },
      {
        seq: 7,
        kind: "plan_card",
        payload: {
          run_id: "fixture-run-01",
          plan_id: "plan-s1-002-plan",
          plan_hash: "a1b2c3d4e5f60001",
          max_tier: 2,
          steps: [
            {
              tool_id: "working_aoi_get",
              label_zh: "解析空间范围与投影",
              tier: 0,
              status: "done",
            },
            {
              tool_id: "search_start",
              label_zh: "检索 Sentinel-1 雷达影像",
              tier: 1,
              status: "done",
              disclosure: {
                resource_id: "science.sentinel-1.standard.v1",
                display_name_zh: "Sentinel-1 雷达卫星",
                aoi_name: "茂县叠溪",
                time_range: { start: "2024-05-01", end: "2024-05-31" },
              },
            },
            {
              tool_id: "create_task",
              label_zh: "创建下载任务项",
              tier: 2,
              status: "pending",
            },
          ],
        },
      },
      {
        seq: 8,
        kind: "confirm_card",
        payload: {
          run_id: "fixture-run-01",
          plan_id: "plan-s1-002",
          plan_hash: "b2c3d4e5f6a1b2c3",
          snapshot: {
            source: "Sentinel-1 SLC (ASF)",
            range: {
              aoi_name: "茂县叠溪",
              area_km2: 145.2,
              time_range: { start: "2024-05-01", end: "2024-05-31" },
              product: "SLC · 降轨",
            },
            estimated_bytes: 4404019200,
            output_dir: "/data/sar/downloads/maoxian",
            output_dir_source: "default",
            result_count: 5,
            already_queued_count: 0,
            orbit_directions: ["Descending"],
          },
        },
      },
      {
        seq: 9,
        kind: "conclusion",
        payload: {
          run_id: "fixture-run-01",
          text: "已完成批次检索与下载任务准备，确认后即可启动。",
          claims: [
            {
              statement: "已冻结茂县叠溪试验区 Sentinel-1 降轨批次（共 5 景）",
              verified: true,
              refs: [
                {
                  kind: "batch",
                  id: "batch-maoxian-202405",
                  label: "批次 #batch-maoxian-202405",
                  trace_sequence: 5,
                  response_count: 5,
                },
              ],
            },
            {
              statement: "公开科学数据端点 ASF 验证服务状态正常",
              verified: false,
              refs: [
                {
                  kind: "web",
                  id: "asf.alaska.edu",
                  label: "ASF 数据源门户",
                  trace_sequence: 1,
                  response_count: 1,
                },
              ],
            },
          ],
        },
      },
    ];
  }

  if (subscene === "plan-pending") {
    baseEvents.push({
      seq: 5,
      kind: "plan_card",
      payload: {
        run_id: "fixture-run-01",
        plan_id: "plan-s1-001",
        plan_hash: "a9f3b8c2d1e04576",
        max_tier: 2,
        steps: [
          { tool_id: "resolve_aoi", label_zh: "解析空间范围与投影", tier: 0, status: "done" },
          { tool_id: "application_sar_search_and_freeze", label_zh: "检索并冻结批次", tier: 1, status: "done" },
          { tool_id: "application_create_download_task", label_zh: "创建下载任务项", tier: 2, status: "pending" },
        ],
      },
    });
    return baseEvents;
  }

  // 确认卡基础快照
  const confirmSnapshot = {
    source: "Sentinel-1 GRD/SLC (ASF)",
    range: {
      aoi_name: "大同盆地试验区",
      area_km2: 1240.5,
      time_range: { start: "2024-04-01", end: "2024-04-30" },
      product: "SLC · 升轨",
    },
    estimated_bytes: 5153960755, // 约 4.8 GB
    output_dir: "/data/sar/downloads/sentinel1",
  };

  const confirmCardEvent: AgentStreamEvent = {
    seq: 5,
    kind: "confirm_card",
    payload: {
      run_id: "fixture-run-01",
      plan_id: "plan-s1-001",
      plan_hash: "a9f3b8c2d1e04576",
      snapshot: confirmSnapshot,
    },
  };

  if (subscene === "confirm-pending") {
    baseEvents.push(confirmCardEvent);
    return baseEvents;
  }

  if (subscene === "confirm-approved") {
    baseEvents.push(confirmCardEvent);
    baseEvents.push({
      seq: 6,
      kind: "progress_card",
      payload: {
        run_id: "fixture-run-01",
        plan_id: "plan-s1-001",
        step_index: 2,
        tool_id: "application_create_download_task",
        status: "approved",
        code: "plan_approved",
      },
    });
    return baseEvents;
  }

  if (subscene === "confirm-expired") {
    baseEvents.push(confirmCardEvent);
    baseEvents.push({
      seq: 6,
      kind: "progress_card",
      payload: {
        run_id: "fixture-run-01",
        plan_id: "plan-s1-001",
        step_index: 2,
        tool_id: "application_create_download_task",
        status: "expired",
        code: "plan_expired",
      },
    });
    return baseEvents;
  }

  if (subscene === "auto-executed") {
    baseEvents.push({
      seq: 5,
      kind: "confirm_card",
      payload: {
        run_id: "fixture-run-01",
        plan_id: "plan-s1-001",
        plan_hash: "a9f3b8c2d1e04576",
        snapshot: confirmSnapshot,
        approval: {
          decision: "auto_by_mode",
          mode: "full_access",
          card_reasons: [],
        },
        auto_executed: true,
      },
    });
    baseEvents.push({
      seq: 6,
      kind: "progress_card",
      payload: {
        run_id: "fixture-run-01",
        plan_id: "plan-s1-001",
        step_index: 2,
        tool_id: "application_create_download_task",
        status: "approved",
        code: "plan_approved",
        created_task_ids: ["mock-task-c1-running-1"],
      },
    });
    return baseEvents;
  }

  if (subscene === "card-reasons") {
    baseEvents.push({
      seq: 5,
      kind: "confirm_card",
      payload: {
        run_id: "fixture-run-01",
        plan_id: "plan-s1-001",
        plan_hash: "a9f3b8c2d1e04576",
        snapshot: { ...confirmSnapshot, estimated_bytes: null },
        approval: {
          decision: "manual",
          mode: "auto_approve",
          card_reasons: ["size_unknown", "reopens_existing_task"],
        },
        auto_executed: false,
      },
    });
    return baseEvents;
  }

  if (subscene === "card-dir-default") {
    baseEvents.push({
      seq: 5,
      kind: "confirm_card",
      payload: {
        run_id: "fixture-run-01",
        plan_id: "plan-s1-001",
        plan_hash: "a9f3b8c2d1e04576",
        snapshot: {
          source: "Sentinel-1 雷达卫星",
          range: { ...confirmSnapshot.range },
          estimated_bytes: 5153960755,
          output_dir: "D:\\EOStage\\Downloads",
          output_dir_source: "default",
          result_count: 5,
          already_queued_count: 0,
          orbit_directions: ["Ascending"],
        },
        approval: {
          decision: "manual",
          mode: "confirm_changes",
          card_reasons: [],
        },
      },
    });
    return baseEvents;
  }

  if (subscene === "card-dir-empty") {
    baseEvents.push({
      seq: 5,
      kind: "confirm_card",
      payload: {
        run_id: "fixture-run-01",
        plan_id: "plan-s1-001",
        plan_hash: "a9f3b8c2d1e04576",
        snapshot: {
          source: "Sentinel-1 雷达卫星",
          range: { ...confirmSnapshot.range },
          estimated_bytes: 5153960755,
          output_dir: null,
          output_dir_source: null,
          result_count: 5,
          already_queued_count: 0,
          orbit_directions: ["Ascending"],
        },
        approval: {
          decision: "manual",
          mode: "full_access",
          card_reasons: ["output_dir_unknown"],
        },
      },
    });
    return baseEvents;
  }

  if (subscene === "card-dir-model") {
    baseEvents.push({
      seq: 5,
      kind: "confirm_card",
      payload: {
        run_id: "fixture-run-01",
        plan_id: "plan-s1-001",
        plan_hash: "a9f3b8c2d1e04576",
        snapshot: {
          source: "Sentinel-1 雷达卫星",
          range: { ...confirmSnapshot.range },
          estimated_bytes: 5153960755,
          output_dir: "E:\\SAR\\茂县叠溪",
          output_dir_source: "model_suggested",
          result_count: 5,
          already_queued_count: 0,
          orbit_directions: ["Ascending"],
        },
        approval: {
          decision: "manual",
          mode: "auto_approve",
          card_reasons: ["output_dir_unverified"],
        },
      },
    });
    return baseEvents;
  }

  if (subscene === "card-dir-message") {
    baseEvents.push({
      seq: 5,
      kind: "confirm_card",
      payload: {
        run_id: "fixture-run-01",
        plan_id: "plan-s1-001",
        plan_hash: "a9f3b8c2d1e04576",
        snapshot: {
          source: "Sentinel-1 雷达卫星",
          range: { ...confirmSnapshot.range },
          estimated_bytes: 5153960755,
          output_dir: "D:\\data\\maoxian",
          output_dir_source: "from_message",
          result_count: 5,
          already_queued_count: 0,
          orbit_directions: ["Ascending"],
        },
        approval: {
          decision: "manual",
          mode: "auto_approve",
          card_reasons: [],
        },
      },
    });
    return baseEvents;
  }

  if (subscene === "message-dir-ref") {
    return [
      {
        seq: 1,
        kind: "message",
        payload: {
          role: "user",
          run_id: "fixture-run-01",
          text: "请检索四川茂县叠溪镇 2024 年 5 月的 Sentinel-1 SLC 数据并准备下载。",
          context: null,
        },
      },
      {
        seq: 2,
        kind: "message",
        payload: {
          role: "assistant",
          run_id: "fixture-run-01",
          text: "数据将下载到 [local-reference:message_dir]，检索完成后请确认。原始文件也将保留在 [local-reference:message_dir] 中以供后续解压。",
          context: null,
        },
      },
    ];
  }

  if (subscene === "plan-disclosure") {
    baseEvents.push({
      seq: 5,
      kind: "plan_card",
      payload: {
        run_id: "fixture-run-01",
        plan_id: "plan-s1-disc",
        plan_hash: "d15c105e00000001",
        steps: [
          {
            tool_id: "search_start",
            label_zh: "检索 Sentinel-1 数据",
            tier: 1,
            status: "done",
            disclosure: {
              resource_id: "science.sentinel-1.standard.v1",
              display_name_zh: "Sentinel-1 雷达卫星",
              aoi_name: "大同盆地试验区",
              time_range: { start: "2024-04-01", end: "2024-04-30" },
            },
          },
          {
            tool_id: "create_task",
            label_zh: "创建下载任务",
            tier: 2,
            status: "pending",
          },
        ],
        approval: {
          decision: "auto_by_mode",
          mode: "confirm_changes",
          card_reasons: [],
        },
        auto_executed: true,
      },
    });
    return baseEvents;
  }

  if (subscene === "forbidden") {
    baseEvents.push({
      seq: 5,
      kind: "error",
      payload: {
        run_id: "fixture-run-01",
        code: "plan_refused_forbidden",
        status: "failed",
        plan_id: null,
        approval: {
          decision: "refused_forbidden",
          mode: "full_access",
          card_reasons: [],
        },
      },
    });
    return baseEvents;
  }

  // W3e 时期写下的旧事件：approval.mode 为 per_item / within_quota、带 exceeded（界面显示「未知模式」并忽略 exceeded）
  if (subscene === "legacy-events") {
    baseEvents.push({
      seq: 5,
      kind: "confirm_card",
      payload: {
        run_id: "fixture-run-01",
        plan_id: "plan-s1-001",
        plan_hash: "a9f3b8c2d1e04576",
        snapshot: confirmSnapshot,
        approval: { decision: "manual", mode: "within_quota", exceeded: ["max_bytes", "output_dir"] },
        auto_executed: false,
      },
    });
    baseEvents.push({
      seq: 6,
      kind: "error",
      payload: {
        run_id: "fixture-run-01",
        code: "quota_exceeded_actual",
        status: "failed",
        plan_id: "plan-s1-001",
        approval: { decision: "auto_by_mode", mode: "within_quota", exceeded: ["max_bytes_actual"] },
      },
    });
    return baseEvents;
  }

  if (subscene === "model-truncated") {
    return [
      baseEvents[0],
      baseEvents[1],
      baseEvents[2],
      {
        seq: 4,
        kind: "error",
        payload: {
          run_id: "fixture-run-01",
          code: "model_truncated",
          status: "failed",
          plan_id: null,
        },
      },
    ];
  }

  if (subscene === "reasoning-refused") {
    baseEvents.push(confirmCardEvent);
    baseEvents.push({
      seq: 6,
      kind: "progress_card",
      payload: {
        run_id: "fixture-run-01",
        plan_id: "plan-s1-001",
        step_index: 2,
        tool_id: "application_create_download_task",
        status: "done",
        code: "plan_approved",
      },
    });
    baseEvents.push({
      seq: 7,
      kind: "conclusion",
      payload: {
        run_id: "fixture-run-01",
        text: "已完成批次检索与下载任务创建，数据准备就绪。",
        claims: [
          {
            statement: "已创建 Sentinel-1 下载任务项并写入本地账本",
            verified: true,
            refs: [{ kind: "task", id: "task-s1-001", label: "任务 #task-s1-001" }],
          },
          {
            statement: "成功检索并冻结批次，包含 8 景 SLC 影像",
            verified: true,
            refs: [{ kind: "batch", id: "batch-s1-202404", label: "批次 #batch-s1-202404" }],
          },
          {
            statement: "公开科学数据端点 ASF 验证服务状态正常",
            verified: false,
            refs: [{ kind: "web", id: "asf.alaska.edu", label: "ASF 数据源门户" }],
          },
        ],
        reasoning_off_refused: true,
      },
    });
    return baseEvents;
  }

  if (subscene === "conclusion") {
    baseEvents.push(confirmCardEvent);
    baseEvents.push({
      seq: 6,
      kind: "progress_card",
      payload: {
        run_id: "fixture-run-01",
        plan_id: "plan-s1-001",
        step_index: 2,
        tool_id: "application_create_download_task",
        status: "done",
        code: "plan_approved",
      },
    });
    baseEvents.push({
      seq: 7,
      kind: "conclusion",
      payload: {
        run_id: "fixture-run-01",
        text: "已完成批次检索与下载任务创建，数据准备就绪。",
        claims: [
          {
            statement: "已创建 Sentinel-1 下载任务项并写入本地账本",
            verified: true,
            refs: [{ kind: "task", id: "task-s1-001", label: "任务 #task-s1-001" }],
          },
          {
            statement: "成功检索并冻结批次，包含 8 景 SLC 影像",
            verified: true,
            refs: [{ kind: "batch", id: "batch-s1-202404", label: "批次 #batch-s1-202404" }],
          },
          {
            statement: "公开科学数据端点 ASF 验证服务状态正常",
            verified: false,
            refs: [{ kind: "web", id: "asf.alaska.edu", label: "ASF 数据源门户" }],
          },
        ],
      },
    });
    return baseEvents;
  }

  // 默认完整态
  baseEvents.push(confirmCardEvent);
  baseEvents.push({
    seq: 6,
    kind: "progress_card",
    payload: {
      run_id: "fixture-run-01",
      plan_id: "plan-s1-001",
      step_index: 2,
      tool_id: "application_create_download_task",
      status: "done",
      code: "plan_approved",
    },
  });
  baseEvents.push({
    seq: 7,
    kind: "conclusion",
    payload: {
      run_id: "fixture-run-01",
      text: "已完成大同盆地试验区数据检索与准备工作。",
      claims: [
        {
          statement: "已建立下载任务",
          verified: true,
          refs: [{ kind: "task", id: "task-s1-001", label: "任务 #task-s1-001" }],
        },
      ],
    },
  });
  return baseEvents;
}

export function createFixtureSessions__fixture(): AgentSessionSummary[] {
  const now = new Date();
  const today1 = new Date(now.getFullYear(), now.getMonth(), now.getDate(), 14, 30).toISOString();
  const today2 = new Date(now.getFullYear(), now.getMonth(), now.getDate(), 10, 15).toISOString();
  const yesterday = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1, 16, 45).toISOString();
  const earlier1 = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 3, 11, 20).toISOString();
  const earlier2 = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 6, 9, 5).toISOString();

  return [
    {
      session_id: "fixture-sess-01",
      title: "检索四川茂县叠溪镇 2024 年 5 月的 Sentinel-1 SLC 数据并创建下载。",
      created_at: today1,
      updated_at: today1,
      run_ids: ["fixture-run-01"],
    },
    {
      session_id: "fixture-sess-02",
      title: "九寨沟 NISAR L 波段 GSLC，2026 年 9 月以来",
      created_at: today2,
      updated_at: today2,
      run_ids: ["fixture-run-02"],
    },
    {
      session_id: "fixture-sess-03",
      title: "台湾花莲 ALOS-2 PALSAR-2 升轨数据检索",
      created_at: yesterday,
      updated_at: yesterday,
      run_ids: ["fixture-run-03"],
    },
    {
      session_id: "fixture-sess-04",
      title: "青藏高原 Landsat-8/9 地表反射率数据下载",
      created_at: earlier1,
      updated_at: earlier1,
      run_ids: ["fixture-run-04"],
    },
    {
      session_id: "fixture-sess-05",
      title: "大同煤田 Sentinel-1 降轨干涉宽幅 SLC 检索",
      created_at: earlier2,
      updated_at: earlier2,
      run_ids: ["fixture-run-05"],
    },
  ];
}

export function createFixtureRuns__fixture(sessionId: string): AgentRunSummary[] {
  return [
    {
      run_id: "fixture-run-01",
      session_id: sessionId,
      status: "completed",
      started_at: "2026-10-24 20:00:00",
      ended_at: "2026-10-24 20:01:15",
      model_route: "OpenAI Chat",
    },
    {
      run_id: "fixture-run-02",
      session_id: sessionId,
      status: "failed",
      started_at: "2026-10-24 20:02:00",
      ended_at: "2026-10-24 20:02:45",
      model_route: "OpenAI Chat",
      reasoning_off_refused: true,
      truncated: true,
    },
  ];
}

export function createFixtureTraceDetail__fixture(runId: string, sessionId: string): AgentRunDetail {
  return {
    run: {
      run_id: runId,
      session_id: sessionId,
      status: "completed",
      started_at: "2026-10-24 20:00:00",
      ended_at: "2026-10-24 20:01:15",
      model_route: "OpenAI Chat",
    },
    events: [
      { sequence: 1, previous_sha256: "0000", kind: "message", payload: {}, sha256: "1111" },
      { sequence: 2, previous_sha256: "1111", kind: "tool_round", payload: {}, sha256: "2222" },
      { sequence: 3, previous_sha256: "2222", kind: "conclusion", payload: {}, sha256: "3333" },
    ],
  };
}

export function createFixtureChannelViews__fixture(): AgentChannelView[] {
  return [
    {
      id: "fixture-openai-default",
      protocol: "chat_completions",
      base_url: "https://api.openai.com/v1",
      model: "gpt-4o",
      reasoning_mode: "default",
      key_ready: true,
      is_default: true,
    },
    {
      id: "fixture-anthropic-backup",
      protocol: "anthropic_messages",
      base_url: "https://api.anthropic.com/v1",
      model: "claude-3-5-sonnet",
      key_ready: false,
      is_default: false,
    },
  ];
}

export function createFixturePermissionView__fixture(): AgentPermissionModeView {
  return {
    effective: "confirm_changes",
    session_override: null,
    global_default: "confirm_changes",
  };
}

export function createFixtureStopAllResult__fixture(): AgentStopAllResult {
  return {
    cancelled_runs: ["fixture-run-01"],
    failed_runs: [],
    cancelled_tasks: ["fixture-task-01"],
    failed_tasks: [],
  };
}
