import assert from "node:assert/strict";
import test from "node:test";

import { JSDOM } from "jsdom";
import React, { act } from "react";
import { frozenNisarItem, geometry } from "@/lib/nisarAppendTestFixture";

import {
  clearTaskDetailTestBridge,
  registerTaskDetailTestBridge,
  type TaskDetailTestBridgeDependencies,
} from "@/lib/taskDetailTestBridge";


const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://127.0.0.1/",
});
Object.assign(globalThis, {
  window: dom.window,
  document: dom.window.document,
  HTMLElement: dom.window.HTMLElement,
  Node: dom.window.Node,
  MutationObserver: dom.window.MutationObserver,
  getComputedStyle: dom.window.getComputedStyle,
  IS_REACT_ACT_ENVIRONMENT: true,
});
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: dom.window.navigator,
});

const { TaskDetailWorkbench } = await import("./TaskDetailWorkbench");
const { finishSourceMutation, getSourceMutationGate } = await import("@/lib/taskSourceMutationGate");
const { cleanup, render, screen, within } = await import("@testing-library/react");
const userEvent = (await import("@testing-library/user-event")).default;

test.afterEach(() => {
  cleanup();
  clearTaskDetailTestBridge();
  dom.window.localStorage.clear();
});

test("JERS task detail replaces the generic failed label with restricted-access guidance", async () => {
  const opened: string[] = [];
  render(
    <TaskDetailWorkbench
      taskId="jers-task"
      revision={4}
      title="JERS-1 下载任务"
      status="failed"
      datasetLabel="JERS-1"
      concurrency={1}
      resumeSupported
      outputDir="C:/downloads"
      completedCount={0}
      activeCount={0}
      waitingCount={0}
      pausedCount={0}
      failedCount={1}
      transferredBytes={0}
      totalBytes={null}
      bytesPerSecond={0}
      transferActiveMillis={0}
      page={{
        taskId: "jers-task",
        revision: 4,
        pageIndex: 0,
        pageSize: 24,
        totalCount: 1,
        items: [{
          itemId: "J1_TEST",
          displayName: "J1_TEST.zip",
          status: "failed",
          bytesCompleted: 0,
          totalBytes: 100,
          percentage: 0,
          bytesPerSecond: 0,
          destination: "C:/downloads/J1_TEST.zip",
          failure: {
            code: "ASF_DATASET_ACCESS_RESTRICTED",
            reasonZh: "JERS-1 是受限数据集。有关如何访问这些数据的信息，请联系 ASF 用户支持。",
            retryable: false,
          },
          retryable: false,
          terminal: true,
        }],
      }}
      events={[]}
      onClose={() => undefined}
      onOpenOutput={() => undefined}
      onOpenRestrictedSupport={(url) => { opened.push(url); }}
      onPageIndexChange={() => undefined}
    />,
  );

  assert.ok(screen.getAllByText("🔒 访问受限").length >= 1);
  assert.ok(screen.getAllByText("JERS-1 是受限数据集。有关如何访问这些数据的信息，请联系 ASF 用户支持。").length >= 1);
  await userEvent.click(screen.getByRole("button", { name: "ASF 用户支持" }));
  assert.deepEqual(opened, ["https://asf.alaska.edu/asf/contact/"]);
});

test("a queued JERS item does not revive a retained restricted-access failure", async () => {
  registerTaskDetailTestBridge({
    invoke: async (command) => {
      if (command === "application_load_task_source_batch_page") {
        return {
          task_id: "jers-paused-task",
          source_batch_id: "jers-source-batch",
          task_revision: 5,
          task_status: "paused",
          page_index: 0,
          page_size: 24,
          total_count: 0,
          items: [],
        };
      }
      return {
        readiness: { contract: "application.v1", status: "ready", store_schema_version: 28 },
        core: { tasks: [{ task_id: String((args?.request as Record<string, unknown> | undefined)?.task_id ?? (args?.request as Record<string, unknown> | undefined)?.new_task_id ?? "reuse-task"), revision: 5 }], history: [], workbench: null },
      };
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  });
  const view = render(
    <TaskDetailWorkbench
      taskId="jers-paused-task"
      revision={5}
      title="JERS-1 下载任务"
      status="paused"
      datasetLabel="JERS-1"
      concurrency={1}
      resumeSupported
      outputDir="C:/downloads"
      completedCount={0}
      activeCount={0}
      waitingCount={1}
      pausedCount={0}
      failedCount={0}
      transferredBytes={0}
      totalBytes={100}
      bytesPerSecond={0}
      transferActiveMillis={0}
      page={{
        taskId: "jers-paused-task",
        revision: 5,
        pageIndex: 0,
        pageSize: 24,
        totalCount: 1,
        items: [{
          itemId: "J1_TEST",
          displayName: "J1_TEST.zip",
          status: "queued",
          bytesCompleted: 0,
          totalBytes: 100,
          percentage: 0,
          bytesPerSecond: 0,
          destination: "C:/downloads/J1_TEST.zip.part",
          failure: {
            code: "ASF_DATASET_ACCESS_RESTRICTED",
            reasonZh: "JERS-1 是受限数据集。有关如何访问这些数据的信息，请联系 ASF 用户支持。",
            retryable: false,
            stage: "未知阶段",
            transferredBytes: 0,
            attempt: 1,
          },
          attempt: 1,
          retryable: false,
          terminal: false,
          metadata: {},
        }],
      }}
      events={[]}
      onClose={() => undefined}
      onOpenOutput={() => undefined}
      onOpenRestrictedSupport={() => undefined}
      onPageIndexChange={() => undefined}
    />,
  );
  await act(async () => {
    await Promise.resolve();
  });

  assert.equal(screen.queryAllByText("🔒 访问受限").length, 0);
  assert.ok(screen.queryByRole("button", { name: "ASF 用户支持" }) === null);
  assert.ok(view.container.querySelector("[data-task-failure-summary]") === null);
});

test("Burst historical context uses task-scoped typed append and terminal successor commands", async () => {
  for (const [status, expectedCommand] of [
    ["paused", "application_append_burst_asset_task"],
    ["failed", "application_create_historical_burst_asset_task"],
  ] as const) {
    const calls: Array<[string, Record<string, unknown> | undefined]> = [];
    const dependencies: TaskDetailTestBridgeDependencies = {
      invoke: async (command, args) => {
        calls.push([command, args]);
        if (command === "application_load_task_source_batch_page") {
          return {
            task_id: String((args?.request as Record<string, unknown> | undefined)?.task_id ?? "burst-task"),
            source_batch_id: "burst-source-batch",
            task_revision: calls.some(([c]) => c === expectedCommand) ? 12 : 11,
            task_status: status,
            page_index: 0,
            page_size: Number((args?.request as Record<string, unknown> | undefined)?.page_size ?? 24),
            total_count: 1,
            items: [{
              ordinal: 0,
              item: {
                item_id: "BURST-B",
                provider_id: "asf.catalog",
                display_name: "BURST-B",
                acquisition_datetime: "2026-08-20T00:00:00Z",
                footprint_geojson: null,
                metadata: { dataset: "SLC-BURST" },
                payload: {},
              },
              belongs_to_task: calls.some(([c]) => c === expectedCommand),
              task_status: null,
              checked: false,
              bytes_completed: 0,
              total_bytes: null,
              bytes_per_second: 0,
              failure: null,
            }],
          };
        }
        return {
          readiness: { contract: "application.v1", status: "ready", store_schema_version: 19 },
          core: { tasks: [{ task_id: String((args?.request as Record<string, unknown> | undefined)?.task_id ?? (args?.request as Record<string, unknown> | undefined)?.new_task_id ?? "reuse-task"), revision: 5 }], history: [], workbench: null },
        };
      },
      getCurrentWindow: () => ({
        minimize: async () => undefined,
        toggleMaximize: async () => undefined,
        close: async () => undefined,
        innerSize: async () => ({ width: 100, height: 100 }),
        startResizeDragging: async () => undefined,
      }),
      open: async () => null,
    };
    registerTaskDetailTestBridge(dependencies);
    render(
      <TaskDetailWorkbench
        taskId="burst-task"
        revision={11}
        title="Burst task"
        status={status}
        datasetLabel="SLC-BURST"
        concurrency={2}
        newTaskPolicyReady={status !== "failed"}
        resumeSupported
        outputDir="C:/downloads"
        completedCount={0}
        activeCount={0}
        waitingCount={0}
        pausedCount={0}
        failedCount={status === "failed" ? 1 : 0}
        transferredBytes={0}
        totalBytes={null}
        bytesPerSecond={0}
        transferActiveMillis={0}
        page={null}
        events={[]}
        onClose={() => undefined}
        onOpenOutput={() => undefined}
        onOpenRestrictedSupport={() => undefined}
        onPageIndexChange={() => undefined}
      />,
    );

    assert.ok(screen.getByRole("button", { name: "概览" }));
    await userEvent.click(screen.getByRole("button", { name: "来源结果" }));
    await userEvent.click(await screen.findByRole("checkbox", { name: "选择 BURST-B" }));
    await userEvent.click(screen.getByRole("button", {
      name: status === "paused" ? "追加到当前任务" : "创建新下载任务",
    }));
    if (status === "failed") {
      assert.equal(calls.some(([command]) => command === expectedCommand), false);
      assert.ok(screen.getByRole("dialog", { name: "确认下载任务" }));
      assert.equal(screen.getByLabelText("目的目录").getAttribute("value"), "C:/downloads");
      await userEvent.click(screen.getByRole("button", { name: "创建下载任务" }));
    }
    await screen.findByRole("status");

    const mutation = calls.find(([command]) => command === expectedCommand);
    assert.ok(mutation, `${status} must use ${expectedCommand}`);
    const request = mutation[1]?.request as Record<string, unknown>;
    assert.deepEqual(request.selected_product_ids, ["BURST-B"]);
    assert.equal(
      request[status === "paused" ? "expected_task_revision" : "expected_source_task_revision"],
      11,
    );
    assert.equal(
      calls.some(([command]) => command === "application_append_task_from_source_batch"
        || command === "application_create_task_from_historical_source_batch"),
      false,
    );
    cleanup();
    clearTaskDetailTestBridge();
  dom.window.localStorage.clear();
  }
});


test("NISAR task info projects per-item lifecycle and retries only the failed item", async () => {
  const calls: Array<[string, Record<string, unknown> | undefined]> = [];
  let refreshes = 0;
  const completedId = "nisar-completed-scene";
  const failedId = "nisar-failed-scene";
  const preparedEnvelope = {
    envelope_contract: "NisarSubsetPreparedIntentEnvelopeV2",
    schema_version: 2,
    canonical_intent_json: JSON.stringify({
      requested_geometry: { canonical_geojson: {
        type: "Polygon", coordinates: [[[110, 30], [111, 30], [111, 31], [110, 30]]],
      } },
      selected_frequency: "A", selected_polarizations: ["HH"],
    }),
    source_batch_id: "nisar-batch",
    source_item_id: completedId,
    workbench_revision: 12,
    intent: { selected_frequency: "A", selected_polarizations: ["HH"] },
  };
  const page = {
    taskId: "nisar-batch-task",
    revision: 13,
    pageIndex: 0,
    pageSize: 24,
    totalCount: 2,
    items: [
      {
        itemId: completedId,
        displayName: "NISAR_COMPLETED.h5",
        status: "completed",
        bytesCompleted: 2048,
        totalBytes: 2048,
        percentage: 100,
        bytesPerSecond: 0,
        destination: "E:/nisar-output/NISAR_COMPLETED.h5",
        failure: null,
        attempt: 1,
        retryable: false,
        terminal: true,
        metadata: {
          delivery_kind: "remote_derived_subset_v2",
          nisar_subset_prepared_intent_v2_envelope: preparedEnvelope,
          nisar_subset_plan_v2_envelope: { plan_contract: "nisar_subset_plan_v2" },
        },
      },
      {
        itemId: failedId,
        displayName: "NISAR_FAILED.h5",
        status: "failed",
        bytesCompleted: 512,
        totalBytes: 4096,
        percentage: 13,
        bytesPerSecond: 0,
        destination: "E:/nisar-output/NISAR_FAILED.h5.part",
        failure: {
          code: "NISAR_SUBSET_TRANSFER_INTERRUPTED",
          reasonZh: "范围下载中断",
          retryable: true,
          stage: "未知阶段",
          transferredBytes: 512,
          attempt: 3,
        },
        attempt: 3,
        retryable: true,
        terminal: true,
        metadata: {
          delivery_kind: "remote_derived_subset_v2",
          nisar_subset_prepared_intent_v2_envelope: {
            ...preparedEnvelope,
            source_item_id: failedId,
          },
        },
      },
    ],
  };
  const dependencies: TaskDetailTestBridgeDependencies = {
    invoke: async (command, args) => {
      calls.push([command, args]);
      if (command === "application_load_task_source_batch_page") {
        return {
          task_id: "nisar-batch-task",
          source_batch_id: "nisar-batch",
          task_revision: 13,
          task_status: "failed",
          page_index: 0,
          page_size: 24,
          total_count: 2,
          items: [
            {
              ordinal: 0,
              item: {
                item_id: completedId,
                provider_id: "asf.nasa.earthdata",
                display_name: "FROZEN_SEARCH_A.h5",
                acquisition_datetime: "2026-06-18T10:00:00Z",
                footprint_geojson: null,
                metadata: { logical_dataset: "NISAR_L_RSLC", delivery_kind: "remote_derived_subset_v2" },
                payload: {},
                asset_access: null,
                execution_policy: null,
              },
              belongs_to_task: true,
              task_status: "completed",
              checked: true,
              bytes_completed: 2048,
              total_bytes: 2048,
              bytes_per_second: 0,
              failure: null,
            },
            {
              ordinal: 0,
              item: {
                item_id: failedId,
                provider_id: "asf.nasa.earthdata",
                display_name: "FROZEN_SEARCH_B.h5",
                acquisition_datetime: "2026-06-18T10:02:00Z",
                footprint_geojson: null,
                metadata: { logical_dataset: "NISAR_L_RSLC", delivery_kind: "remote_derived_subset_v2" },
                payload: {},
                asset_access: null,
                execution_policy: null,
              },
              belongs_to_task: true,
              task_status: "failed",
              checked: true,
              bytes_completed: 512,
              total_bytes: 4096,
              bytes_per_second: 0,
              failure: null,
            },
          ],
        };
      }
      return {
        readiness: { contract: "application.v1", status: "ready", store_schema_version: 19 },
        core: { tasks: [{ task_id: String((args?.request as Record<string, unknown> | undefined)?.task_id ?? (args?.request as Record<string, unknown> | undefined)?.new_task_id ?? "reuse-task"), revision: 5 }], history: [], workbench: null },
      };
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  };
  registerTaskDetailTestBridge(dependencies);
  const detail = (
    <TaskDetailWorkbench
      taskId="nisar-batch-task"
      taskMutable
      revision={13}
      title="NISAR 批量裁剪任务"
      status="failed"
      datasetLabel="NISAR RSLC"
      concurrency={2}
      resumeSupported
      outputDir="E:/nisar-output"
      completedCount={1}
      activeCount={0}
      waitingCount={0}
      pausedCount={0}
      failedCount={1}
      transferredBytes={2560}
      totalBytes={6144}
      bytesPerSecond={0}
      transferActiveMillis={0}
      page={page}
      events={[]}
      onClose={() => undefined}
      onOpenOutput={() => undefined}
      onOpenRestrictedSupport={() => undefined}
      onPageIndexChange={() => undefined}
      onHistoricalMutationApplied={() => { refreshes += 1; }}
    />
  );
  const view = render(detail);

  assert.ok(screen.getByRole("region", { name: "任务总体进度" }));
  assert.ok(screen.getByText("NISAR_FAILED.h5"));
  assert.ok(screen.getByText("NISAR_SUBSET_TRANSFER_INTERRUPTED"));
  const failureSummary = view.container.querySelector("[data-task-failure-summary]");
  assert.ok(failureSummary);
  assert.ok(within(failureSummary as HTMLElement).getByText("范围下载中断"));
  assert.doesNotMatch(failureSummary.textContent ?? "", /未知阶段/);

  await userEvent.click(screen.getByRole("button", { name: "来源结果" }));
  assert.ok(await screen.findByText("来源批次 nisar-batch"));
  assert.ok(screen.getByText("FROZEN_SEARCH_A.h5"));
  assert.ok(screen.getByText("FROZEN_SEARCH_B.h5"));
  assert.ok(screen.getByRole("region", { name: "任务冻结 AOI" }));
  assert.ok(screen.getByText("Frequency A · HH"));

  await userEvent.click(screen.getByRole("button", { name: "概览" }));
  assert.ok(screen.getByText("NISAR_SUBSET_TRANSFER_INTERRUPTED"));
  await userEvent.click(screen.getByRole("button", { name: "下载文件" }));
  assert.ok(screen.getByText("NISAR_COMPLETED.h5"));
  assert.ok(screen.getAllByText("NISAR_FAILED.h5").length >= 2);
  for (const stage of ["准备", "规划", "下载", "生成", "校验", "输出"]) {
    assert.ok(screen.getAllByText(stage).length >= 1, "missing lifecycle stage: " + stage);
  }
  assert.ok(screen.getByText("E:/nisar-output/NISAR_COMPLETED.h5"));
  const failureDiagnostic = view.container.querySelector("[data-task-failure-diagnostic]");
  assert.ok(failureDiagnostic);
  assert.ok(within(failureDiagnostic as HTMLElement).getByText("范围下载中断"));
  assert.doesNotMatch(failureDiagnostic.textContent ?? "", /未知阶段/);
  assert.ok(screen.getAllByText("NISAR_SUBSET_TRANSFER_INTERRUPTED").length >= 2);
  assert.equal(screen.queryByText(/已传输：512 B/), null,
    "subset item bytes must not be relabeled as cumulative network transfer");
  assert.ok(screen.getByText(/尝试次数：3/));
  assert.ok(screen.getByText(/可恢复：是/));

  const retryButtons = screen.getAllByRole("button", { name: "重试此影像" });
  assert.equal(retryButtons.length, 1);
  await userEvent.click(retryButtons[0]!);
  await new Promise<void>((resolve) => setTimeout(resolve, 0));
  const retry = calls.find(([command]) => command === "application_update_download_task");
  assert.ok(retry, "retry must use application_update_download_task");
  const retryRequest = retry?.[1]?.request as Record<string, unknown>;
  assert.equal(retryRequest.task_id, "nisar-batch-task");
  assert.equal(retryRequest.expected_revision, 13);
  assert.deepEqual(retryRequest.command, { kind: "retry_items", item_ids: [failedId] });
  assert.equal(typeof retryRequest.event_id, "string");
  assert.equal(typeof retryRequest.occurred_at, "string");
  assert.deepEqual((retryRequest.command as Record<string, unknown>).item_ids, [failedId]);
  assert.equal((retryRequest.command as Record<string, unknown>).item_ids.includes(completedId), false);
  assert.equal(refreshes, 1);
  view.rerender(React.cloneElement(detail, { taskMutable: false }));
  assert.ok(screen.queryByRole("button", { name: "重试此影像" }) === null,
    "archived or unconfirmed task mutability must not dispatch a doomed retry");
  assert.ok(screen.getByText("此任务当前不可直接重试；已归档记录与下载断点保持保留。"));
  assert.equal(calls.filter(([command]) => command === "application_update_download_task").length, 1);
});

test("NISAR subset detail reports persisted transfer facts without source-byte progress or ETA", async () => {
  const commonItem = {
    failure: null,
    attempt: 1,
    retryable: false,
    metadata: { delivery_kind: "remote_derived_subset_v2" },
  };
  const detail = (
    <TaskDetailWorkbench
      taskId="nisar-three-item-subset"
      revision={18}
      title="NISAR 三景裁剪任务"
      status="running"
      taskType="nisar_subset"
      datasetLabel="NISAR RSLC"
      concurrency={2}
      resumeSupported
      outputDir="E:/nisar-output"
      completedCount={1}
      activeCount={1}
      waitingCount={0}
      pausedCount={1}
      failedCount={0}
      transferredBytes={185_300_000}
      totalBytes={46_500_000_000}
      sourceTotalBytes={46_500_000_000}
      networkBytesReceived={185_300_000}
      reusableVerifiedBytes={null}
      reusedVerifiedBytes={0}
      bytesPerSecond={3_000_000}
      transferActiveMillis={62_000}
      page={{
        taskId: "nisar-three-item-subset",
        revision: 18,
        pageIndex: 0,
        pageSize: 24,
        totalCount: 3,
        items: [
          { ...commonItem, itemId: "active", displayName: "NISAR_ACTIVE.h5", status: "active",
            bytesCompleted: 185_300_000, totalBytes: 15_500_000_000, percentage: 1,
            bytesPerSecond: 3_000_000, destination: "E:/nisar-output/NISAR_ACTIVE.h5.part", terminal: false },
          { ...commonItem, itemId: "paused", displayName: "NISAR_PAUSED.h5", status: "paused",
            bytesCompleted: 0, totalBytes: 15_500_000_000, percentage: 0,
            bytesPerSecond: 0, destination: "E:/nisar-output/NISAR_PAUSED.h5.part", terminal: false },
          { ...commonItem, itemId: "completed", displayName: "NISAR_COMPLETED_OUTPUT.h5", status: "completed",
            bytesCompleted: 100_488_907, totalBytes: 100_488_907, percentage: 100,
            bytesPerSecond: 0, destination: "E:/nisar-output/NISAR_COMPLETED_OUTPUT.h5", terminal: true },
        ],
      }}
      events={[]}
      onClose={() => undefined}
      onOpenOutput={() => undefined}
      onOpenRestrictedSupport={() => undefined}
      onPageIndexChange={() => undefined}
    />
  );
  const view = render(detail);

  const header = screen.getByRole("region", { name: "任务总体进度" });
  assert.match(header.textContent ?? "", /网络已接收\s*185\.3 MB/);
  assert.match(header.textContent ?? "", /源文件总量（参考）\s*46\.5 GB/);
  assert.doesNotMatch(header.textContent ?? "", /185\.3 MB\s*\/\s*46\.5 GB|\b0%|\b1%/);
  for (const etaField of screen.getAllByText("预计剩余")) {
    assert.match(etaField.parentElement?.textContent ?? "", /不可用/);
  }

  const overview = screen.getByRole("button", { name: "概览" });
  await userEvent.click(overview);
  const summary = view.container.querySelector("[data-task-overview-summary]");
  assert.ok(summary);
  assert.match(summary.textContent ?? "", /网络已接收\s*185\.3 MB/);
  assert.match(summary.textContent ?? "", /源文件总量（参考）\s*46\.5 GB/);
  const resumeDetails = summary.querySelector("details");
  assert.ok(resumeDetails);
  assert.equal(resumeDetails.open, false);
  assert.match(resumeDetails.textContent ?? "", /续传详情/);
  assert.match(resumeDetails.textContent ?? "", /已保存并校验的数据\s*不可用/);
  assert.match(resumeDetails.textContent ?? "", /本次续传跳过的下载量\s*0 B/);
  assert.match(resumeDetails.textContent ?? "", /仅统计影像数据块，不含预读的元数据；任务级汇总/);
  assert.doesNotMatch(summary.textContent ?? "", /185\.3 MB\s*\/\s*46\.5 GB|\b0%|\b1%/);

  await userEvent.click(screen.getByRole("button", { name: "下载文件" }));
  const activeRow = screen.getByText("NISAR_ACTIVE.h5").closest("article");
  const pausedRow = screen.getByText("NISAR_PAUSED.h5").closest("article");
  const completedRow = screen.getByText("NISAR_COMPLETED_OUTPUT.h5").closest("article");
  assert.ok(activeRow && pausedRow && completedRow);
  assert.match(activeRow.textContent ?? "", /输出字节\s*待完成/);
  assert.match(pausedRow.textContent ?? "", /输出字节\s*待完成/);
  assert.match(completedRow.textContent ?? "", /输出字节\s*100\.5 MB/);
  for (const row of [activeRow, pausedRow, completedRow]) {
    assert.match(row.textContent ?? "", /源文件总量（参考）\s*不可用/);
    assert.doesNotMatch(row.textContent ?? "", /任务网络已接收|科学数据可复用|科学数据本次已复用|续传统计范围/);
    assert.doesNotMatch(row.textContent ?? "", /\/\s*15\.5 GB\s*·\s*\d+%/);
  }

  await act(async () => {
    view.rerender(React.cloneElement(detail, {
      status: "paused",
      networkBytesReceived: 250_000_000,
      reusableVerifiedBytes: 49_686_192,
      reusedVerifiedBytes: 47_876_062,
    }));
  });
  const pausedHeader = screen.getByRole("region", { name: "任务总体进度" });
  assert.match(pausedHeader.textContent ?? "", /网络已接收\s*250 MB/);
  const pausedDetails = pausedHeader.querySelector("details");
  assert.ok(pausedDetails);
  assert.equal(pausedDetails.open, false);
  assert.match(pausedDetails.textContent ?? "", /已保存并校验的数据\s*49\.7 MB/);
  assert.match(pausedDetails.textContent ?? "", /本次续传跳过的下载量\s*47\.9 MB/);
  for (const row of [activeRow, pausedRow, completedRow]) {
    assert.doesNotMatch(row.textContent ?? "", /任务网络已接收|科学数据可复用|科学数据本次已复用/);
  }

  await act(async () => {
    view.rerender(React.cloneElement(detail, {
      status: "running",
      networkBytesReceived: 310_000_000,
      reusableVerifiedBytes: 80_000_000,
      reusedVerifiedBytes: 75_000_000,
    }));
  });
  const resumedHeader = screen.getByRole("region", { name: "任务总体进度" });
  assert.match(resumedHeader.textContent ?? "", /网络已接收\s*310 MB/);
  const resumedDetails = resumedHeader.querySelector("details");
  assert.ok(resumedDetails);
  assert.equal(resumedDetails.open, false);
  assert.match(resumedDetails.textContent ?? "", /已保存并校验的数据\s*80 MB/);
  assert.match(resumedDetails.textContent ?? "", /本次续传跳过的下载量\s*75 MB/);
});

test("paused NISAR recovery observation is not rendered as an execution failure", async () => {
  registerTaskDetailTestBridge({
    invoke: async (command) => {
      if (command === "application_load_task_source_batch_page") {
        return {
          task_id: "nisar-paused-prefix",
          source_batch_id: "nisar-paused-source-batch",
          task_revision: 11,
          task_status: "paused",
          page_index: 0,
          page_size: 24,
          total_count: 0,
          items: [],
        };
      }
      return {
        readiness: { contract: "application.v1", status: "ready", store_schema_version: 28 },
        core: { tasks: [{ task_id: String((args?.request as Record<string, unknown> | undefined)?.task_id ?? (args?.request as Record<string, unknown> | undefined)?.new_task_id ?? "reuse-task"), revision: 5 }], history: [], workbench: null },
      };
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  });
  const view = render(
    <TaskDetailWorkbench
      taskId="nisar-paused-prefix"
      revision={11}
      title="NISAR S-band SSAR RSLC 下载任务"
      status="paused"
      taskType="nisar_subset"
      datasetLabel="NISAR S-band SSAR RSLC"
      concurrency={1}
      resumeSupported
      outputDir="E:/nisar-output"
      completedCount={0}
      activeCount={0}
      waitingCount={1}
      pausedCount={0}
      failedCount={0}
      transferredBytes={0}
      totalBytes={24_922_535_061}
      sourceTotalBytes={24_922_535_061}
      networkBytesReceived={161_156_694}
      reusableVerifiedBytes={null}
      reusedVerifiedBytes={null}
      bytesPerSecond={0}
      transferActiveMillis={38_092}
      page={{
        taskId: "nisar-paused-prefix",
        revision: 11,
        pageIndex: 0,
        pageSize: 24,
        totalCount: 1,
        items: [{
          itemId: "nisar-paused-item",
          displayName: "NISAR_PAUSED_PREFIX.h5",
          status: "queued",
          bytesCompleted: 0,
          totalBytes: 24_922_535_061,
          percentage: 0,
          bytesPerSecond: 0,
          destination: "E:/nisar-output/NISAR_PAUSED_PREFIX.h5.part",
          failure: {
            code: "NISAR_SUBSET_V2_PAUSED",
            reasonZh: "NISAR V2 execution was paused after preserving verified recovery state.",
            retryable: false,
            stage: "未知阶段",
            transferredBytes: 0,
            attempt: 2,
          },
          attempt: 2,
          retryable: false,
          terminal: false,
          metadata: { delivery_kind: "remote_derived_subset_v2" },
        }],
      }}
      events={[]}
      onClose={() => undefined}
      onOpenOutput={() => undefined}
      onOpenRestrictedSupport={() => undefined}
      onPageIndexChange={() => undefined}
    />,
  );
  await act(async () => {
    await Promise.resolve();
  });

  assert.ok(view.container.querySelector("[data-task-failure-summary]") === null,
    "a queued item under a paused task must not produce a red failure summary");
  assert.ok(screen.queryByText("NISAR_SUBSET_V2_PAUSED") === null);

  await userEvent.click(screen.getByRole("button", { name: "下载文件" }));
  assert.ok(screen.getByText("NISAR_PAUSED_PREFIX.h5"));
  assert.ok(view.container.querySelector("[data-task-failure-diagnostic]") === null,
    "a preserved pause observation must not be presented as a current item failure");
  assert.ok(screen.queryByText("NISAR_SUBSET_V2_PAUSED") === null);
});

test("persisted subset identity survives an empty detail page and keeps missing facts unavailable", () => {
  render(
    <TaskDetailWorkbench
      taskId="empty-subset"
      revision={4}
      title="Empty subset page"
      status="paused"
      taskType="nisar_subset"
      datasetLabel="NISAR RSLC"
      concurrency={1}
      resumeSupported
      outputDir="E:/nisar-output"
      completedCount={0}
      activeCount={0}
      waitingCount={3}
      pausedCount={0}
      failedCount={0}
      transferredBytes={999}
      totalBytes={46_500_000_000}
      networkBytesReceived={null}
      reusableVerifiedBytes={null}
      reusedVerifiedBytes={null}
      bytesPerSecond={10_000_000}
      transferActiveMillis={0}
      page={{ taskId: "empty-subset", revision: 4, pageIndex: 0, pageSize: 24, totalCount: 3, items: [] }}
      events={[]}
      onClose={() => undefined}
      onOpenOutput={() => undefined}
      onOpenRestrictedSupport={() => undefined}
      onPageIndexChange={() => undefined}
    />,
  );
  const header = screen.getByRole("region", { name: "任务总体进度" });
  assert.match(header.textContent ?? "", /网络已接收\s*不可用/);
  const resumeDetails = header.querySelector("details");
  assert.ok(resumeDetails);
  assert.equal(resumeDetails.open, false);
  assert.match(resumeDetails.textContent ?? "", /已保存并校验的数据\s*不可用/);
  assert.doesNotMatch(header.textContent ?? "", /999 B|46\.5 GB.*%/);
  for (const etaField of screen.getAllByText("预计剩余")) {
    assert.match(etaField.parentElement?.textContent ?? "", /不可用/);
  }
});

test("frozen subset page metadata is a compatibility fallback but explicit full-download identity wins", async () => {
  const subsetItem = {
    itemId: "frozen-item",
    displayName: "NISAR_FULL_OR_SUBSET.h5",
    status: "active",
    bytesCompleted: 1_000_000_000,
    totalBytes: 2_000_000_000,
    percentage: 50,
    bytesPerSecond: 100_000_000,
    destination: "E:/downloads/NISAR_FULL_OR_SUBSET.h5.part",
    failure: null,
    attempt: 1,
    retryable: false,
    terminal: false,
    metadata: { delivery_kind: "remote_derived_subset_v2" },
  };
  const baseProps = {
    taskId: "compatibility-task", revision: 2, title: "NISAR task", status: "running",
    datasetLabel: "NISAR RSLC", concurrency: 1, resumeSupported: true, outputDir: "E:/downloads",
    completedCount: 0, activeCount: 1, waitingCount: 0, pausedCount: 0, failedCount: 0,
    transferredBytes: 1_000_000_000, totalBytes: 2_000_000_000, bytesPerSecond: 100_000_000,
    transferActiveMillis: 10_000,
    page: { taskId: "compatibility-task", revision: 2, pageIndex: 0, pageSize: 24, totalCount: 1, items: [subsetItem] },
    events: [], onClose: () => undefined, onOpenOutput: () => undefined,
    onOpenRestrictedSupport: () => undefined, onPageIndexChange: () => undefined,
  };
  const view = render(<TaskDetailWorkbench {...baseProps} taskType={null}
    networkBytesReceived={null} reusableVerifiedBytes={null} reusedVerifiedBytes={null} />);
  assert.match(screen.getByRole("region", { name: "任务总体进度" }).textContent ?? "", /网络已接收\s*不可用/);

  view.rerender(<TaskDetailWorkbench {...baseProps} taskType="full_download"
    networkBytesReceived={null} reusableVerifiedBytes={null} reusedVerifiedBytes={null} />);
  const ordinaryHeader = screen.getByRole("region", { name: "任务总体进度" });
  assert.match(ordinaryHeader.textContent ?? "", /1\.0 GB\s*\/\s*2\.0 GB/);
  assert.match(ordinaryHeader.textContent ?? "", /50%/);
  for (const etaField of screen.getAllByText("预计剩余")) {
    assert.match(etaField.parentElement?.textContent ?? "", /10 秒/);
  }
  await userEvent.click(screen.getByRole("button", { name: "下载文件" }));
  const row = screen.getByText("NISAR_FULL_OR_SUBSET.h5").closest("article");
  assert.ok(row);
  assert.match(row.textContent ?? "", /1\.0 GB\s*\/\s*2\.0 GB\s*·\s*50%/);
});


for (const scenario of [
  { initialStatus: "running", modeChange: false, startFails: false },
  { initialStatus: "completed", modeChange: false, startFails: false },
  { initialStatus: "running", modeChange: true, startFails: false },
  { initialStatus: "completed", modeChange: false, startFails: true },
] as const) {
  const { initialStatus, modeChange, startFails } = scenario;
  const finalStatus = modeChange ? "completed" : initialStatus;
  test(`NISAR ${initialStatus} append workspace: mode change ${modeChange}, start failure ${startFails}`, async () => {
    const calls: Array<[string, Record<string, unknown> | undefined]> = [];
    const sourceItems = [1, 2, 3, 4].map(frozenNisarItem);
    let contextReads = 0; let currentContextRevision = 9;
    const snapshot = { readiness: { contract: "application.v1", status: "ready", store_schema_version: 19 },
      core: { tasks: [], history: [], workbench: null } };
    registerTaskDetailTestBridge({
      invoke: async (command, args) => {
        calls.push([command, args]);
        const request = args?.request as Record<string, unknown>;
        if (command === "application_load_task_source_batch_page") return {
          task_id: "nisar-subset-task", source_batch_id: "nisar-source-batch", task_revision: currentContextRevision,
          task_status: initialStatus, page_index: 0, page_size: request.page_size, total_count: 4,
          items: sourceItems.map((item, ordinal) => ({ ordinal, item, belongs_to_task: ordinal === 0,
            task_status: ordinal === 0 ? initialStatus : null, checked: ordinal === 0,
            bytes_completed: 0, total_bytes: null, bytes_per_second: 0, failure: null })),
        };
        if (command === "application_read_nisar_subset_append_context_v2") return {
          task_id: "nisar-subset-task", source_batch_id: "nisar-source-batch",
          task_revision: (currentContextRevision = [9, 12, 17][Math.min(contextReads++, 2)]!), task_status: modeChange && contextReads >= 3 ? "completed" : initialStatus,
          frozen_aoi_geojson: geometry, selected_frequency: "A", selected_polarizations: ["HH", "HV"],
          output_root: "E:/nisar-output", layout: "direct", concurrency: 2,
        };
        if (command === "application_preflight_nisar_subset_append_v2") return {
          task_id: "nisar-subset-task", source_batch_id: "nisar-source-batch",
          task_revision: request.expected_task_revision,
          eligible_items: sourceItems.slice(1, 3).map((item) => ({ source_item_id: item.item_id,
            effective_geometry: geometry, available_selections: [{ frequency: "A", polarizations: ["HH", "HV"] }] })),
          excluded_items: [{ source_item_id: sourceItems[3]!.item_id, reason: "no_overlap" }],
          common_selections: [{ frequency: "A", polarizations: ["HH", "HV"] }],
        };
        if (command === "application_preview_asset") throw { code: "provider_no_preview", message_key: "provider_no_preview" };
        if (startFails && command === "application_start_newly_confirmed_task") throw new Error("start temporarily unavailable");
        return { ...snapshot, core: { ...snapshot.core, tasks: [{ task_id: String(request?.task_id ?? request?.new_task_id ?? args?.taskId ?? "nisar-subset-task"), revision: 18 }] } };
      },
      getCurrentWindow: () => ({ minimize: async () => undefined, toggleMaximize: async () => undefined,
        close: async () => undefined, innerSize: async () => ({ width: 100, height: 100 }), startResizeDragging: async () => undefined }),
      open: async () => null,
    });
    render(<TaskDetailWorkbench taskId="nisar-subset-task" revision={9} title="NISAR AOI 裁剪任务"
      status={initialStatus} datasetLabel="NISAR RSLC" concurrency={2} resumeSupported outputDir="E:/nisar-output"
      completedCount={initialStatus === "completed" ? 1 : 0} activeCount={initialStatus === "running" ? 1 : 0}
      waitingCount={0} pausedCount={0} failedCount={0} transferredBytes={1024} totalBytes={null}
      bytesPerSecond={64} transferActiveMillis={1000}
      page={{ taskId: "nisar-subset-task", revision: 9, pageIndex: 0, pageSize: 24, totalCount: 1,
        items: [{ itemId: sourceItems[0]!.item_id, displayName: "Original subset", status: initialStatus === "running" ? "active" : "completed",
          bytesCompleted: 1024, totalBytes: null, percentage: null, bytesPerSecond: 64,
          destination: "E:/nisar-output/subset.h5", failure: null, attempt: 1, retryable: false,
          terminal: initialStatus === "completed", metadata: { delivery_kind: "remote_derived_subset_v2" } }] }}
      events={[]} onClose={() => undefined} onOpenOutput={() => undefined} onPageIndexChange={() => undefined} />);

    assert.ok(screen.getByRole("button", { name: "概览" }));
    await userEvent.click(screen.getByRole("button", { name: "来源结果" }));

    if (initialStatus === "running" && !modeChange) {
      await userEvent.click(await screen.findByRole("button", { name: `预览 ${sourceItems[0]!.display_name}` }));
      await screen.findByText("无预览图");
      const request = calls.find(([command]) => command === "application_preview_asset")?.[1]?.request as Record<string, unknown>;
      assert.equal(request.batch_id, "nisar-source-batch");
      assert.equal(request.item_id, sourceItems[0]!.item_id);
      await userEvent.click(screen.getByRole("button", { name: "关闭产品预览" }));
    }
    await userEvent.click(await screen.findByRole("button", { name: "追加数据" }));
    const dialog = within(await screen.findByRole("dialog", { name: "NISAR RSLC AOI 裁剪下载" }));
    const confirmName = initialStatus === "running" ? "确认追加" : "确认创建新裁剪任务";
    const confirm = await dialog.findByRole("button", { name: confirmName });
    await dialog.findByText("与 AOI 无交集，将排除");
    assert.equal(dialog.getAllByRole("checkbox", { name: /选择影像/ }).length, 3);
    for (const name of ["Frequency A", "Polarization HH", "Polarization HV"]) {
      const input = dialog.getByRole(name.startsWith("Frequency") ? "radio" : "checkbox", { name }) as HTMLInputElement;
      assert.equal(input.disabled, true);
      assert.equal(input.checked, true);
    }
    if (initialStatus === "running" && !modeChange) {
      const selections = dialog.getAllByRole("checkbox", { name: /选择影像/ }).map(input => (input as HTMLInputElement).checked);
      await userEvent.click(dialog.getAllByRole("button", { name: /预览影像/ })[0]!);
      await dialog.findByText("无预览图");
      const request = calls.filter(([command]) => command === "application_preview_asset").at(-1)?.[1]?.request as Record<string, unknown>;
      assert.equal(request.batch_id, "nisar-source-batch");
      assert.equal(request.item_id, sourceItems[1]!.item_id);
      assert.deepEqual(dialog.getAllByRole("checkbox", { name: /选择影像/ }).map(input => (input as HTMLInputElement).checked), selections);
      await userEvent.click(dialog.getByRole("button", { name: "关闭产品预览" }));
    }
    assert.equal(dialog.queryByRole("button", { name: "绘制矩形" }), null);
    assert.ok(dialog.getByText(/E:\/nisar-output/));
    assert.equal(calls.some(([command]) => /application_(append_nisar|create_nisar_subset_successor)/.test(command)), false);
    await userEvent.click(confirm);
    if (modeChange) {
      await dialog.findByText("原任务状态已变化，请查看更新后的操作并重新确认。");
      assert.equal(calls.some(([command]) => /application_(append_nisar|create_nisar_subset_successor)/.test(command)), false);
      await userEvent.click(dialog.getByRole("button", { name: "确认创建新裁剪任务" }));
    }
    await screen.findByRole("status");
    if (startFails) {
      await screen.findByText(/新裁剪任务已创建并暂停，启动失败/);
      assert.equal(screen.queryByRole("dialog", { name: "NISAR RSLC AOI 裁剪下载" }), null);
      assert.equal(calls.filter(([command]) => command === "application_create_nisar_subset_successor_task_v2").length, 1);
    }
    const mutationName = finalStatus === "running" ? "application_append_nisar_subset_batch_task_v2" : "application_create_nisar_subset_successor_task_v2";
    const mutation = calls.find(([command]) => command === mutationName);
    assert.ok(mutation);
    const request = mutation[1]?.request as Record<string, unknown>;
    assert.equal(request.expected_task_revision, 17);
    assert.deepEqual(request.source_item_ids, sourceItems.slice(1, 3).map((item) => item.item_id));
    assert.equal(calls.some(([command]) => command === "application_append_task_from_source_batch"), false);
    if (finalStatus === "running") {
      assert.equal(request.task_id, "nisar-subset-task");
      assert.equal(calls.some(([command]) => command === "application_create_nisar_subset_successor_task_v2"), false);
    } else {
      assert.equal(request.source_task_id, "nisar-subset-task");
      assert.notEqual(request.new_task_id, request.source_task_id);
      assert.equal(request.output_root, "E:/nisar-output");
      assert.equal(calls.some(([command]) => command === "application_append_nisar_subset_batch_task_v2"), false);
      assert.ok(calls.some(([command, args]) => command === "application_start_newly_confirmed_task" && args?.taskId === request.new_task_id));
    }
  });
}

for (const negativeCase of [
  {
    label: "ok response missing target task",
    taskId: "nisar-append-missing-target",
    responseTasks: [] as Array<{ task_id: string; revision: number }>,
  },
  {
    label: "ok response containing target task with stale revision below minRevision",
    taskId: "nisar-append-stale-revision",
    responseTasks: [{ task_id: "nisar-append-stale-revision", revision: 17 }],
  },
] as const) {
  test(`NISAR running append retains gate and shows pending verification when ${negativeCase.label}`, async () => {
    const { taskId, responseTasks } = negativeCase;
    const sourceItems = [1, 2, 3, 4].map(frozenNisarItem);
    let contextReads = 0;
    let currentContextRevision = 9;
    const snapshot = {
      readiness: { contract: "application.v1", status: "ready", store_schema_version: 19 },
      core: { tasks: [], history: [], workbench: null },
    };
    registerTaskDetailTestBridge({
      invoke: async (command, args) => {
        const request = args?.request as Record<string, unknown>;
        if (command === "application_load_task_source_batch_page") return {
          task_id: taskId, source_batch_id: "nisar-source-batch", task_revision: currentContextRevision,
          task_status: "running", page_index: 0, page_size: request.page_size, total_count: 4,
          items: sourceItems.map((item, ordinal) => ({ ordinal, item, belongs_to_task: ordinal === 0,
            task_status: ordinal === 0 ? "running" : null, checked: ordinal === 0,
            bytes_completed: 0, total_bytes: null, bytes_per_second: 0, failure: null })),
        };
        if (command === "application_read_nisar_subset_append_context_v2") return {
          task_id: taskId, source_batch_id: "nisar-source-batch",
          task_revision: (currentContextRevision = [9, 12, 17][Math.min(contextReads++, 2)]!), task_status: "running",
          frozen_aoi_geojson: geometry, selected_frequency: "A", selected_polarizations: ["HH", "HV"],
          output_root: "E:/nisar-output", layout: "direct", concurrency: 2,
        };
        if (command === "application_preflight_nisar_subset_append_v2") return {
          task_id: taskId, source_batch_id: "nisar-source-batch",
          task_revision: request.expected_task_revision,
          eligible_items: sourceItems.slice(1, 3).map((item) => ({ source_item_id: item.item_id,
            effective_geometry: geometry, available_selections: [{ frequency: "A", polarizations: ["HH", "HV"] }] })),
          excluded_items: [{ source_item_id: sourceItems[3]!.item_id, reason: "no_overlap" }],
          common_selections: [{ frequency: "A", polarizations: ["HH", "HV"] }],
        };
        if (command === "application_query_nisar_append_status_v2") return {
          outcome: "not_yet_visible",
          target_task_id: taskId,
          target_revision: null,
        };
        return { ...snapshot, core: { ...snapshot.core, tasks: responseTasks } };
      },
      getCurrentWindow: () => ({ minimize: async () => undefined, toggleMaximize: async () => undefined,
        close: async () => undefined, innerSize: async () => ({ width: 100, height: 100 }), startResizeDragging: async () => undefined }),
      open: async () => null,
    });
    try {
      render(<TaskDetailWorkbench taskId={taskId} revision={9} title="NISAR AOI 裁剪任务"
        status="running" datasetLabel="NISAR RSLC" concurrency={2} resumeSupported outputDir="E:/nisar-output"
        completedCount={0} activeCount={1} waitingCount={0} pausedCount={0} failedCount={0}
        transferredBytes={1024} totalBytes={null} bytesPerSecond={64} transferActiveMillis={1000}
        page={{ taskId, revision: 9, pageIndex: 0, pageSize: 24, totalCount: 1,
          items: [{ itemId: sourceItems[0]!.item_id, displayName: "Original subset", status: "active",
            bytesCompleted: 1024, totalBytes: null, percentage: null, bytesPerSecond: 64,
            destination: "E:/nisar-output/subset.h5", failure: null, attempt: 1, retryable: false,
            terminal: false, metadata: { delivery_kind: "remote_derived_subset_v2" } }] }}
        events={[]} onClose={() => undefined} onOpenOutput={() => undefined} onPageIndexChange={() => undefined} />);

      await userEvent.click(screen.getByRole("button", { name: "来源结果" }));
      await userEvent.click(await screen.findByRole("button", { name: "追加数据" }));
      const dialog = within(await screen.findByRole("dialog", { name: "NISAR RSLC AOI 裁剪下载" }));
      const confirm = await dialog.findByRole("button", { name: "确认追加" });
      await dialog.findByText("与 AOI 无交集，将排除");
      await userEvent.click(confirm);

      await dialog.findByText(/响应未确认目标任务；请重试原请求。/);
      const gate = getSourceMutationGate(taskId);
      assert.ok(gate, "source mutation gate must be retained when direct response does not confirm expected revision");
      assert.equal(gate.pending, false);
      assert.equal(gate.minRevision, 18);

      await userEvent.click(dialog.getByRole("button", { name: "取消" }));
      await screen.findByText(`未确认结果，新来源操作已暂停。目标任务：${taskId}`);
      assert.ok(screen.getByRole("button", { name: "重试原请求" }));
    } finally {
      const gate = getSourceMutationGate(taskId);
      if (gate) finishSourceMutation(gate, true);
    }
  });
}

function reusedItemPage(taskId: string, revision: number) {
  return {
    taskId,
    revision,
    pageIndex: 0,
    pageSize: 24,
    totalCount: 1,
    items: [
      {
        itemId: "reused-scene",
        displayName: "S1A_REUSED.zip",
        status: "skipped",
        bytesCompleted: 4096,
        totalBytes: 4096,
        percentage: 100,
        bytesPerSecond: 0,
        destination: "E:/sar-output/S1A_REUSED.zip",
        failure: null,
        attempt: 0,
        retryable: false,
        terminal: true,
        metadata: {},
        localReuse: {
          disposition: "reused" as const,
          grade: "size_and_format" as const,
          conflict_reason: null,
          conflict_file_name: null,
        },
      },
    ],
  };
}

function reacquireBridge(calls: Array<[string, Record<string, unknown> | undefined]>): TaskDetailTestBridgeDependencies {
  return {
    invoke: async (command, args) => {
      calls.push([command, args]);
      return {
        readiness: { contract: "application.v1", status: "ready", store_schema_version: 31 },
        core: { tasks: [{ task_id: String((args?.request as Record<string, unknown> | undefined)?.task_id ?? (args?.request as Record<string, unknown> | undefined)?.new_task_id ?? "reuse-task"), revision: 5 }], history: [], workbench: null },
      };
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  };
}

function reusedDetail(taskMutable: boolean, onApplied: () => void, datasetLabel = "SENTINEL-1") {
  return (
    <TaskDetailWorkbench
      taskId="reuse-task"
      taskMutable={taskMutable}
      revision={4}
      title="Sentinel-1 任务"
      status={taskMutable ? "running" : "completed"}
      datasetLabel={datasetLabel}
      concurrency={1}
      resumeSupported
      outputDir="E:/sar-output"
      completedCount={1}
      waitingCount={0}
      pausedCount={0}
      failedCount={0}
      transferredBytes={0}
      totalBytes={4096}
      bytesPerSecond={0}
      transferActiveMillis={0}
      page={reusedItemPage("reuse-task", 4)}
      events={[]}
      onClose={() => undefined}
      onOpenOutput={() => undefined}
      onOpenRestrictedSupport={() => undefined}
      onPageIndexChange={() => undefined}
      onHistoricalMutationApplied={onApplied}
    />
  );
}

test("a reuse without content evidence is labelled and can be downloaded again in place (downloader D3)", async () => {
  const calls: Array<[string, Record<string, unknown> | undefined]> = [];
  let applied = 0;
  registerTaskDetailTestBridge(reacquireBridge(calls));
  render(reusedDetail(true, () => { applied += 1; }));
  await userEvent.click(screen.getByRole("button", { name: "下载文件" }));
  assert.ok(screen.getAllByText("复用已有文件（未做内容校验）").length >= 1);
  await userEvent.click(screen.getByRole("button", { name: "重新下载" }));
  const dialog = screen.getByRole("dialog", { name: "重新下载" });
  assert.ok(within(dialog).getByText(
    "将重新下载此项。原文件会另存为 S1A_REUSED.zip.eostage-conflict-…，下载完成后由您决定是否删除。",
  ));
  assert.equal(calls.filter(([command]) => command === "application_update_download_task").length, 0,
    "nothing is sent before the confirmation");
  await userEvent.click(within(dialog).getByRole("button", { name: "重新下载" }));
  await new Promise<void>((resolve) => setTimeout(resolve, 0));
  const update = calls.find(([command]) => command === "application_update_download_task");
  assert.ok(update, "an active task reacquires in place");
  const request = update?.[1]?.request as Record<string, unknown>;
  assert.equal(request.task_id, "reuse-task");
  assert.equal(request.expected_revision, 4);
  assert.deepEqual(request.command, { kind: "reacquire_items", item_ids: ["reused-scene"] });
  assert.equal(applied, 1);
});

test("a terminal task downloads a reused item again through a successor in the same folder", async () => {
  const calls: Array<[string, Record<string, unknown> | undefined]> = [];
  registerTaskDetailTestBridge(reacquireBridge(calls));
  render(reusedDetail(false, () => undefined));
  await userEvent.click(screen.getByRole("button", { name: "下载文件" }));
  await userEvent.click(screen.getByRole("button", { name: "重新下载" }));
  const dialog = screen.getByRole("dialog", { name: "重新下载" });
  await userEvent.click(within(dialog).getByRole("button", { name: "重新下载" }));
  await new Promise<void>((resolve) => setTimeout(resolve, 0));
  assert.equal(calls.filter(([command]) => command === "application_update_download_task").length, 0);
  const successor = calls.find(([command]) => command === "application_create_task_from_historical_source_batch");
  assert.ok(successor, "a terminal task creates a successor");
  const request = successor?.[1]?.request as Record<string, unknown>;
  assert.equal(request.source_task_id, "reuse-task");
  assert.equal(request.expected_source_task_revision, 4);
  assert.deepEqual(request.item_ids, ["reused-scene"]);
  assert.deepEqual(request.reacquire_item_ids, ["reused-scene"]);
  assert.equal(request.output_root, "E:/sar-output", "the existing file is in the original folder");
});

test("a terminal Burst task does not offer 重新下载 through the source-batch successor", async () => {
  const calls: Array<[string, Record<string, unknown> | undefined]> = [];
  registerTaskDetailTestBridge(reacquireBridge(calls));
  render(reusedDetail(false, () => undefined, "SLC-BURST"));
  await userEvent.click(screen.getByRole("button", { name: "下载文件" }));
  assert.ok(screen.getAllByText("复用已有文件（未做内容校验）").length >= 1, "the label stays honest");
  assert.equal(screen.queryByRole("button", { name: "重新下载" }), null);
});
