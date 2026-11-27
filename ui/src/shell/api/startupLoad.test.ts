import test from "node:test";
import assert from "node:assert/strict";
import { createStartupEffect, type StartupLoadDependencies } from "./startupLoad.ts";
import type { SourceDescriptor, SourceView, RowTaskState, Bbox } from "./types.ts";

function createDeferred<T>() {
  let resolve!: (val: T) => void;
  let reject!: (err?: any) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

test("W4 StartupLoad Suite - StrictMode dual-mount and lifecycle guards", async (t) => {
  // -------------------------------------------------------------
  // 1. StrictMode 双挂载用例（核心证据）
  // -------------------------------------------------------------
  await t.test("1. StrictMode dual-mount: run() -> cleanup() -> run() drops first run and commits second run", async () => {
    type Deferred<T> = ReturnType<typeof createDeferred<T>>;
    const listSourcesCalls: Deferred<SourceDescriptor[]>[] = [];
    const getSourceViewsCalls: Deferred<SourceView[]>[] = [];
    const getBatchPageCalls: Deferred<{ scenes: any[]; rows: RowTaskState[] }>[] = [];
    const getWorkingAoiCalls: Deferred<unknown>[] = [];
    const getCredentialStatusCalls: Deferred<unknown>[] = [];
    const getSarDownloadPolicyCalls: Deferred<any>[] = [];
    const getDemTaskSettingsCalls: Deferred<any>[] = [];

    const mockSources: SourceDescriptor[] = [
      {
        resource_id: "science.sentinel-1.standard.v1",
        group_id: "sentinel",
        display_name_zh: "哨兵1号",
        family: "sar",
        search_mode: "sar",
        filter_schema_id: null,
        filter_schema_revision: null,
        result_cap: 250,
        accounts: ["earthdata"],
        network_default: "direct",
        availability: "available",
        availability_reason_zh: null,
      },
    ];

    const mockViews: SourceView[] = [
      {
        resource_id: "science.sentinel-1.standard.v1",
        batch_id: "batch-s1-init",
        revision: 1,
        item_count: 1,
        searched_at: "2026-10-25T00:00:00Z",
        pending_request_id: null,
        last_error: null,
      },
    ];

    const mockBatchPage = {
      scenes: [{ scene_id: "S1A_IW_GRDH_1SDV_TEST" }],
      rows: [
        {
          item_id: "item-1",
          task_id: "task-1",
          task_status: "idle" as const,
          item_status: "idle" as const,
          actions: ["download" as const],
        },
      ],
    };

    const mockAoi = {
      geometry: { type: "Polygon", coordinates: [] },
      bbox: [100.0, 20.0, 105.0, 25.0],
      display_name: "四川盆地测试区",
    };

    const mockCreds = { earthdata: { configured: true, valid: true } };

    const mockPolicy = {
      ok: true,
      default_output_root: "D:\\ScenePortData\\StorageRoot",
      per_source: {},
    };

    const mockDem = {
      ok: true,
      max_concurrent_tasks: 4,
    };

    // 观察 setter 调用情况
    let availableSources: SourceDescriptor[] | null = null;
    let sourceViews: SourceView[] | null = null;
    let scenes: any[] | null = null;
    let rowStates: Map<string, RowTaskState> | null = null;
    let workingGeometry: Record<string, unknown> | null = null;
    let workingAoiState: Bbox | null = null;
    let aoiLabel: string | null = null;
    let creds: any = null;
    let storageRootPath: string | null = null;
    let sarDownloadPolicy: any = null;
    let demConcurrency: number | null = null;

    let totalSetterCalls = 0;

    const deps: StartupLoadDependencies = {
      listSources: () => {
        const d = createDeferred<SourceDescriptor[]>();
        listSourcesCalls.push(d);
        return d.promise;
      },
      getSourceViews: () => {
        const d = createDeferred<SourceView[]>();
        getSourceViewsCalls.push(d);
        return d.promise;
      },
      getBatchPage: () => {
        const d = createDeferred<{ scenes: any[]; rows: RowTaskState[] }>();
        getBatchPageCalls.push(d);
        return d.promise;
      },
      getWorkingAoi: () => {
        const d = createDeferred<unknown>();
        getWorkingAoiCalls.push(d);
        return d.promise;
      },
      getCredentialStatus: () => {
        const d = createDeferred<unknown>();
        getCredentialStatusCalls.push(d);
        return d.promise;
      },
      getSarDownloadPolicySettings: () => {
        const d = createDeferred<any>();
        getSarDownloadPolicyCalls.push(d);
        return d.promise;
      },
      getDemTaskSettings: () => {
        const d = createDeferred<any>();
        getDemTaskSettingsCalls.push(d);
        return d.promise;
      },
      isDesktopHost: () => true,

      setAvailableSources: (s) => {
        totalSetterCalls++;
        availableSources = s;
      },
      setSourceViews: (v) => {
        totalSetterCalls++;
        sourceViews = v;
      },
      setScenes: (sc) => {
        totalSetterCalls++;
        scenes = sc;
      },
      setRowStates: (r) => {
        totalSetterCalls++;
        rowStates = r;
      },
      setWorkingGeometry: (g) => {
        totalSetterCalls++;
        workingGeometry = g;
      },
      setWorkingAoiState: (b) => {
        totalSetterCalls++;
        workingAoiState = b;
      },
      setAoiLabel: (l) => {
        totalSetterCalls++;
        aoiLabel = l;
      },
      setCreds: (c) => {
        totalSetterCalls++;
        creds = c;
      },
      setStorageRootPath: (p) => {
        totalSetterCalls++;
        storageRootPath = p;
      },
      setSarDownloadPolicy: (pol) => {
        totalSetterCalls++;
        sarDownloadPolicy = pol;
      },
      setDemConcurrency: (c) => {
        totalSetterCalls++;
        demConcurrency = c;
      },
    };

    const run = createStartupEffect(deps);

    // 1. 模拟 React 19 StrictMode 挂载第 1 次
    const cleanup1 = run();
    assert.strictEqual(listSourcesCalls.length, 1, "mount 1 should initiate listSources");
    assert.strictEqual(getWorkingAoiCalls.length, 1, "mount 1 should initiate getWorkingAoi");

    // 2. React 19 StrictMode cleanup 卸载第 1 次
    cleanup1();

    // 3. React 19 StrictMode 立即挂载第 2 次
    const cleanup2 = run();

    // 在修复后，由于 cleanup1 复位了守卫，第 2 次挂载应该成功发起新的加载请求
    // 若守卫未复位（旧逻辑缺陷），listSourcesCalls.length 将依然为 1
    assert.strictEqual(
      listSourcesCalls.length,
      2,
      "mount 2 must initiate second load when guard resets properly"
    );
    assert.strictEqual(
      getWorkingAoiCalls.length,
      2,
      "mount 2 must initiate second host load when guard resets properly"
    );

    // 4. 先逐步 resolve 第 1 次挂载的所有 Promise（此时第 1 次已被 cleanup 卸载）
    listSourcesCalls[0].resolve(mockSources);
    getSourceViewsCalls[0].resolve(mockViews);

    // 宿主加载器按序 await 执行
    getWorkingAoiCalls[0].resolve(mockAoi);
    await new Promise((resolve) => setTimeout(resolve, 10));
    assert.strictEqual(getCredentialStatusCalls.length, 1, "mount 1 proceeds to getCredentialStatus");
    getCredentialStatusCalls[0].resolve(mockCreds);

    await new Promise((resolve) => setTimeout(resolve, 10));
    assert.strictEqual(getSarDownloadPolicyCalls.length, 1, "mount 1 proceeds to getSarDownloadPolicySettings");
    getSarDownloadPolicyCalls[0].resolve(mockPolicy);

    await new Promise((resolve) => setTimeout(resolve, 10));
    assert.strictEqual(getDemTaskSettingsCalls.length, 1, "mount 1 proceeds to getDemTaskSettings");
    getDemTaskSettingsCalls[0].resolve(mockDem);

    // 等待微任务处理完毕
    await new Promise((resolve) => setTimeout(resolve, 20));

    // 断言第 1 次挂载的所有结果都被 active 检查丢弃，没有任何 setter 被调用
    assert.strictEqual(
      totalSetterCalls,
      0,
      "all setters from unmounted first run must be discarded"
    );
    assert.strictEqual(storageRootPath, null);
    assert.strictEqual(availableSources, null);
    assert.strictEqual(workingAoiState, null);

    // 5. 然后逐步 resolve 第 2 次挂载的 Promise（活跃挂载）
    listSourcesCalls[1].resolve(mockSources);
    getSourceViewsCalls[1].resolve(mockViews);

    // 宿主加载器按序 await 执行
    getWorkingAoiCalls[1].resolve(mockAoi);
    await new Promise((resolve) => setTimeout(resolve, 10));
    assert.strictEqual(getCredentialStatusCalls.length, 2, "mount 2 proceeds to getCredentialStatus");
    getCredentialStatusCalls[1].resolve(mockCreds);

    await new Promise((resolve) => setTimeout(resolve, 10));
    assert.strictEqual(getSarDownloadPolicyCalls.length, 2, "mount 2 proceeds to getSarDownloadPolicySettings");
    getSarDownloadPolicyCalls[1].resolve(mockPolicy);

    await new Promise((resolve) => setTimeout(resolve, 10));
    assert.strictEqual(getDemTaskSettingsCalls.length, 2, "mount 2 proceeds to getDemTaskSettings");
    getDemTaskSettingsCalls[1].resolve(mockDem);

    // 批次页 Promise 在 views 解决后才发起
    await new Promise((resolve) => setTimeout(resolve, 20));
    assert.strictEqual(getBatchPageCalls.length, 1, "default view batch page loader called on active run");
    getBatchPageCalls[0].resolve(mockBatchPage);

    // 等待全部结算
    await new Promise((resolve) => setTimeout(resolve, 20));

    // 6. 严密断言第 2 次挂载的所有结果成功应用到状态中
    assert.strictEqual(storageRootPath, "D:\\ScenePortData\\StorageRoot", "storageRootPath must be loaded");
    assert.deepStrictEqual(sarDownloadPolicy, mockPolicy, "sarDownloadPolicy must be loaded");
    assert.strictEqual(demConcurrency, 4, "demConcurrency must be loaded");
    assert.deepStrictEqual(creds, mockCreds, "creds must be loaded");
    assert.deepStrictEqual(availableSources, mockSources, "availableSources must be loaded");
    assert.deepStrictEqual(sourceViews, mockViews, "sourceViews must be loaded");
    assert.deepStrictEqual(scenes, mockBatchPage.scenes, "batch scenes must be loaded");
    assert.ok(rowStates?.has("item-1"), "batch rowStates must contain item-1");
    assert.deepStrictEqual(workingGeometry, mockAoi.geometry, "workingGeometry must be loaded");
    assert.deepStrictEqual(
      workingAoiState,
      { west: 100.0, south: 20.0, east: 105.0, north: 25.0 },
      "workingAoiState bbox must be mapped correctly"
    );
    assert.strictEqual(aoiLabel, "四川盆地测试区", "aoiLabel must match display_name");

    cleanup2();
  });

  // -------------------------------------------------------------
  // 2. 单挂载防同实例重入用例
  // -------------------------------------------------------------
  await t.test("2. Single mount: prevent duplicate calls within same mount, allow reload after cleanup", async () => {
    let listCallCount = 0;
    const deps: StartupLoadDependencies = {
      listSources: async () => {
        listCallCount++;
        return [];
      },
      getSourceViews: async () => [],
      getBatchPage: async () => ({ scenes: [], rows: [] }),
      getWorkingAoi: async () => null,
      getCredentialStatus: async () => null,
      getSarDownloadPolicySettings: async () => null,
      getDemTaskSettings: async () => null,
      isDesktopHost: () => false,

      setAvailableSources: () => {},
      setSourceViews: () => {},
      setScenes: () => {},
      setRowStates: () => {},
      setWorkingGeometry: () => {},
      setWorkingAoiState: () => {},
      setAoiLabel: () => {},
      setCreds: () => {},
      setStorageRootPath: () => {},
      setSarDownloadPolicy: () => {},
      setDemConcurrency: () => {},
    };

    const run = createStartupEffect(deps);
    const cleanup1 = run();
    assert.strictEqual(listCallCount, 1, "first call runs");

    // 同一挂载周期内再次调用（重入防重）
    const cleanupReentrant = run();
    assert.strictEqual(listCallCount, 1, "reentrant call inside same mount must be guarded");

    cleanupReentrant();
    cleanup1();

    // 卸载后再调用允许重新加载
    const cleanup2 = run();
    assert.strictEqual(listCallCount, 2, "subsequent mount after cleanup is allowed");
    cleanup2();
  });

  // -------------------------------------------------------------
  // 3. 非桌面宿主用例
  // -------------------------------------------------------------
  await t.test("3. Non-desktop host: host state loaders strictly 0 calls", async () => {
    let hostAoiCalls = 0;
    let hostCredsCalls = 0;
    let hostPolicyCalls = 0;
    let hostDemCalls = 0;
    let c1SourceCalls = 0;

    const deps: StartupLoadDependencies = {
      listSources: async () => {
        c1SourceCalls++;
        return [];
      },
      getSourceViews: async () => [],
      getBatchPage: async () => ({ scenes: [], rows: [] }),
      getWorkingAoi: async () => {
        hostAoiCalls++;
        return null;
      },
      getCredentialStatus: async () => {
        hostCredsCalls++;
        return null;
      },
      getSarDownloadPolicySettings: async () => {
        hostPolicyCalls++;
        return null;
      },
      getDemTaskSettings: async () => {
        hostDemCalls++;
        return null;
      },
      isDesktopHost: () => false,

      setAvailableSources: () => {},
      setSourceViews: () => {},
      setScenes: () => {},
      setRowStates: () => {},
      setWorkingGeometry: () => {},
      setWorkingAoiState: () => {},
      setAoiLabel: () => {},
      setCreds: () => {},
      setStorageRootPath: () => {},
      setSarDownloadPolicy: () => {},
      setDemConcurrency: () => {},
    };

    const run = createStartupEffect(deps);
    const cleanup = run();
    await new Promise((resolve) => setTimeout(resolve, 20));

    assert.strictEqual(c1SourceCalls, 1, "C1 sources should be loaded even in web host");
    assert.strictEqual(hostAoiCalls, 0, "host AOI must not be called when isDesktopHost is false");
    assert.strictEqual(hostCredsCalls, 0, "host creds must not be called when isDesktopHost is false");
    assert.strictEqual(hostPolicyCalls, 0, "host policy must not be called when isDesktopHost is false");
    assert.strictEqual(hostDemCalls, 0, "host DEM must not be called when isDesktopHost is false");

    cleanup();
  });

  // -------------------------------------------------------------
  // 4. 局部失败容错用例
  // -------------------------------------------------------------
  await t.test("4. Partial loader failure does not block other loaders from applying", async () => {
    let appliedCreds: any = null;
    let appliedDemConcurrency: number | null = null;

    const originalWarn = console.warn;
    const warnLogs: any[] = [];
    console.warn = (...args: any[]) => {
      warnLogs.push(args);
    };

    try {
      const deps: StartupLoadDependencies = {
        listSources: async () => [],
        getSourceViews: async () => [],
        getBatchPage: async () => ({ scenes: [], rows: [] }),
        getWorkingAoi: async () => {
          throw new Error("AOI disk read failed");
        },
        getCredentialStatus: async () => ({ earthdata: true }),
        getSarDownloadPolicySettings: async () => {
          throw new Error("Policy file corrupted");
        },
        getDemTaskSettings: async () => ({ ok: true, max_concurrent_tasks: 3 }),
        isDesktopHost: () => true,

        setAvailableSources: () => {},
        setSourceViews: () => {},
        setScenes: () => {},
        setRowStates: () => {},
        setWorkingGeometry: () => {},
        setWorkingAoiState: () => {},
        setAoiLabel: () => {},
        setCreds: (c) => {
          appliedCreds = c;
        },
        setStorageRootPath: () => {},
        setSarDownloadPolicy: () => {},
        setDemConcurrency: (c) => {
          appliedDemConcurrency = c;
        },
      };

      const run = createStartupEffect(deps);
      const cleanup = run();
      await new Promise((resolve) => setTimeout(resolve, 30));

      assert.deepStrictEqual(appliedCreds, { earthdata: true }, "creds should be applied despite other errors");
      assert.strictEqual(appliedDemConcurrency, 3, "dem concurrency should be applied despite other errors");
      assert.ok(warnLogs.length >= 2, "warnings should be logged for failures");

      cleanup();
    } finally {
      console.warn = originalWarn;
    }
  });

  // -------------------------------------------------------------
  // 5. 纯 Node 环境无 DOM / 无 Vite 导入兼容
  // -------------------------------------------------------------
  await t.test("5. Pure Node environment: createStartupEffect callable without DOM or import.meta.env", () => {
    assert.strictEqual(typeof createStartupEffect, "function");
  });

  // -------------------------------------------------------------
  // 6. Fixture Working AOI 集中注入行为测试
  // -------------------------------------------------------------
  await t.test("6. Non-desktop + fixture provider calls setWorkingGeometry/setWorkingAoiState/setAoiLabel with fixture values", async () => {
    let appliedGeom: Record<string, unknown> | null = null;
    let appliedBbox: Bbox | null = null;
    let appliedLabel: string | null = null;

    const mockFixtureAoi = {
      label: "茂县叠溪",
      bbox: { west: 103.5, south: 31.8, east: 103.75, north: 32.0 },
      geometry: {
        type: "Polygon",
        coordinates: [
          [
            [103.5, 31.8],
            [103.75, 31.8],
            [103.75, 32.0],
            [103.5, 32.0],
            [103.5, 31.8],
          ],
        ],
      },
    };

    const deps: any = {
      listSources: async () => [],
      getSourceViews: async () => [],
      getBatchPage: async () => ({ scenes: [], rows: [] }),
      getWorkingAoi: async () => null,
      getCredentialStatus: async () => null,
      getSarDownloadPolicySettings: async () => null,
      getDemTaskSettings: async () => null,
      isDesktopHost: () => false,
      getFixtureWorkingAoi: () => mockFixtureAoi,

      setAvailableSources: () => {},
      setSourceViews: () => {},
      setScenes: () => {},
      setRowStates: () => {},
      setWorkingGeometry: (g: any) => {
        appliedGeom = g;
      },
      setWorkingAoiState: (b: any) => {
        appliedBbox = b;
      },
      setAoiLabel: (l: string) => {
        appliedLabel = l;
      },
      setCreds: () => {},
      setStorageRootPath: () => {},
      setSarDownloadPolicy: () => {},
      setDemConcurrency: () => {},
    };

    const run = createStartupEffect(deps);
    const cleanup = run();
    await new Promise((r) => setTimeout(r, 10));

    assert.ok(appliedGeom, "non-desktop + fixture provider must call setWorkingGeometry");
    assert.deepStrictEqual(appliedBbox, mockFixtureAoi.bbox, "setWorkingAoiState must be called with fixture bbox");
    assert.strictEqual(appliedLabel, "茂县叠溪", "setAoiLabel must be called with fixture label");

    cleanup();
  });

  await t.test("7. Non-desktop + provider returning null calls none of the AOI setters", async () => {
    let callCount = 0;

    const deps: any = {
      listSources: async () => [],
      getSourceViews: async () => [],
      getBatchPage: async () => ({ scenes: [], rows: [] }),
      getWorkingAoi: async () => null,
      getCredentialStatus: async () => null,
      getSarDownloadPolicySettings: async () => null,
      getDemTaskSettings: async () => null,
      isDesktopHost: () => false,
      getFixtureWorkingAoi: () => null,

      setAvailableSources: () => {},
      setSourceViews: () => {},
      setScenes: () => {},
      setRowStates: () => {},
      setWorkingGeometry: () => {
        callCount++;
      },
      setWorkingAoiState: () => {
        callCount++;
      },
      setAoiLabel: () => {
        callCount++;
      },
      setCreds: () => {},
      setStorageRootPath: () => {},
      setSarDownloadPolicy: () => {},
      setDemConcurrency: () => {},
    };

    const run = createStartupEffect(deps);
    const cleanup = run();
    await new Promise((r) => setTimeout(r, 10));

    assert.strictEqual(callCount, 0, "none of the AOI setters should be called when fixture provider returns null");
    cleanup();
  });

  await t.test("9. Startup surfaces interrupted batch delivery state and total_count via onBatchPage", async () => {
    let observedPage: any = null;
    let appliedScenes: any[] = [];

    const deps: StartupLoadDependencies = {
      listSources: async () => [],
      getSourceViews: async () => [
        {
          resource_id: "science.sentinel-1.standard.v1",
          batch_id: "batch-interrupted-restart",
          revision: 0,
          item_count: 50,
          searched_at: "2026-11-05T00:00:00Z",
          pending_request_id: null,
          last_error: null,
        },
      ],
      getBatchPage: async (batchId) => ({
        batch_id: batchId,
        total_count: 50,
        loaded_count: 50,
        remote_total: 120,
        state: "interrupted" as const,
        scenes: [{ scene_id: "S1A_INTERRUPTED_001" }],
        rows: [
          {
            item_id: "S1A_INTERRUPTED_001",
            task_id: null,
            task_status: "idle" as const,
            item_status: "idle" as const,
            actions: ["download" as const],
          },
        ],
      }),
      onBatchPage: (page) => {
        observedPage = page;
      },
      getWorkingAoi: async () => null,
      getCredentialStatus: async () => null,
      getSarDownloadPolicySettings: async () => null,
      getDemTaskSettings: async () => null,
      isDesktopHost: () => false,
      setAvailableSources: () => {},
      setSourceViews: () => {},
      setScenes: (s) => {
        appliedScenes = s;
      },
      setRowStates: () => {},
      setWorkingGeometry: () => {},
      setWorkingAoiState: () => {},
      setAoiLabel: () => {},
      setCreds: () => {},
      setStorageRootPath: () => {},
      setSarDownloadPolicy: () => {},
      setDemConcurrency: () => {},
    };

    const run = createStartupEffect(deps);
    const cleanup = run();
    await new Promise((r) => setTimeout(r, 20));

    assert.strictEqual(appliedScenes.length, 1);
    assert.ok(observedPage, "onBatchPage must be called for interrupted startup batch");
    assert.strictEqual(observedPage.batch_id, "batch-interrupted-restart");
    assert.strictEqual(observedPage.total_count, 50);
    assert.strictEqual(observedPage.state, "interrupted");
    cleanup();
  });

  await t.test("8. Desktop host preserves original getWorkingAoi loading and ignores getFixtureWorkingAoi", async () => {
    let desktopAoiCalled = false;
    let appliedLabel: string | null = null;

    const deps: any = {
      listSources: async () => [],
      getSourceViews: async () => [],
      getBatchPage: async () => ({ scenes: [], rows: [] }),
      getWorkingAoi: async () => {
        desktopAoiCalled = true;
        return {
          display_name: "Desktop Working AOI",
          bbox: [100, 20, 101, 21],
          geometry: { type: "Polygon", coordinates: [] },
        };
      },
      getCredentialStatus: async () => null,
      getSarDownloadPolicySettings: async () => null,
      getDemTaskSettings: async () => null,
      isDesktopHost: () => true,
      getFixtureWorkingAoi: () => ({
        label: "Should Be Ignored",
        bbox: { west: 103.5, south: 31.8, east: 103.75, north: 32.0 },
        geometry: {},
      }),

      setAvailableSources: () => {},
      setSourceViews: () => {},
      setScenes: () => {},
      setRowStates: () => {},
      setWorkingGeometry: () => {},
      setWorkingAoiState: () => {},
      setAoiLabel: (l: string) => {
        appliedLabel = l;
      },
      setCreds: () => {},
      setStorageRootPath: () => {},
      setSarDownloadPolicy: () => {},
      setDemConcurrency: () => {},
    };

    const run = createStartupEffect(deps);
    const cleanup = run();
    await new Promise((r) => setTimeout(r, 30));

    assert.strictEqual(desktopAoiCalled, true, "desktop host must call getWorkingAoi");
    assert.strictEqual(appliedLabel, "Desktop Working AOI", "desktop host must use native AOI instead of fixture");
    cleanup();
  });

  await test("Issue #8 尾项：冷启动恢复已保存批次查询时，极化、波束模式、视向与 Burst 策略完整回填到表单状态", async () => {
    let restoredPolarizations: string[] | null = null;
    let restoredBeamModes: string[] | null = null;
    let restoredLookDirection: string | null = null;
    let restoredBurstStrategy: "all" | "primary" | null = null;

    const deps = {
      listSources: async () => [],
      getSourceViews: async () => [
        {
          resource_id: "science.sentinel-1.standard.v1",
          batch_id: "batch-restore-8",
          revision: 3,
          total_count: 2,
          updated_at: "2026-11-05T00:00:00Z",
          query: {
            filters: {
              polarizations: ["VV", "VH"],
              beam_modes: ["IW", "EW"],
              look_direction: "right",
              burst_asset_policy: "primary_only",
            },
          },
        } as any,
      ],
      getBatchPage: async () => ({
        batch_id: "batch-restore-8",
        total_count: 2,
        scenes: [],
        rows: [],
      }),
      getWorkingAoi: async () => null,
      getCredentialStatus: async () => null,
      getSarDownloadPolicySettings: async () => null,
      getDemTaskSettings: async () => null,
      isDesktopHost: () => false,

      setAvailableSources: () => {},
      setSourceViews: () => {},
      setScenes: () => {},
      setRowStates: () => {},
      setWorkingGeometry: () => {},
      setWorkingAoiState: () => {},
      setAoiLabel: () => {},
      setCreds: () => {},
      setStorageRootPath: () => {},
      setSarDownloadPolicy: () => {},
      setDemConcurrency: () => {},
      setPolarizations: (pols: string[]) => {
        restoredPolarizations = pols;
      },
      setBeamModes: (modes: string[]) => {
        restoredBeamModes = modes;
      },
      setLookDirection: (dir: string | null) => {
        restoredLookDirection = dir;
      },
      setBurstStrategy: (strategy: "all" | "primary") => {
        restoredBurstStrategy = strategy;
      },
    };

    const run = createStartupEffect(deps);
    const cleanup = run();
    await new Promise((r) => setTimeout(r, 30));

    assert.deepStrictEqual(restoredPolarizations, ["VV", "VH"]);
    assert.deepStrictEqual(restoredBeamModes, ["IW", "EW"]);
    assert.strictEqual(restoredLookDirection, "RIGHT");
    assert.strictEqual(restoredBurstStrategy, "primary");
    cleanup();
  });
});

test("9. Startup restores SarDownloadPolicy even when default_output_root is empty", async () => {
  let appliedPolicy: any = null;
  let storageRootSetCount = 0;

  const deps: any = {
    listSources: async () => [],
    getSourceViews: async () => [],
    getBatchPage: async () => ({ scenes: [], rows: [] }),
    getWorkingAoi: async () => null,
    getCredentialStatus: async () => null,
    getSarDownloadPolicySettings: async () => ({
      ok: true,
      mode: "manual",
      organize_by_product: false,
      default_output_root: "",
      max_concurrent_items: 6,
    }),
    getDemTaskSettings: async () => ({ ok: true, max_concurrent_tasks: 6 }),
    isDesktopHost: () => true,
    setAvailableSources: () => {},
    setSourceViews: () => {},
    setScenes: () => {},
    setRowStates: () => {},
    setWorkingGeometry: () => {},
    setWorkingAoiState: () => {},
    setAoiLabel: () => {},
    setCreds: () => {},
    setStorageRootPath: () => {
      storageRootSetCount += 1;
    },
    setSarDownloadPolicy: (policy: any) => {
      appliedPolicy = policy;
    },
    setDemConcurrency: () => {},
  };

  const run = createStartupEffect(deps);
  const cleanup = run();
  await new Promise((r) => setTimeout(r, 30));

  assert.ok(appliedPolicy, "setSarDownloadPolicy must be called even if default_output_root is empty");
  assert.strictEqual(appliedPolicy.max_concurrent_items, 6);
  assert.strictEqual(storageRootSetCount, 0, "empty default_output_root must not overwrite storage root path");
  cleanup();
});

test("10. Startup restores persisted preferred activeResourceId and loads its initial batch", async () => {
  let restoredId: string | null = null;
  const loadedBatchIds: string[] = [];
  let loadedScenes: any[] = [];

  const deps: StartupLoadDependencies = {
    listSources: async () => [
      {
        resource_id: "science.sentinel-1.standard.v1",
        group_id: "sentinel",
        display_name_zh: "Sentinel-1",
        family: "sar",
        search_mode: "sar",
        filter_schema_id: null,
        filter_schema_revision: null,
        result_cap: 250,
        accounts: [],
        network_default: "direct",
      },
      {
        resource_id: "science.landsat.c2-l2.v1",
        group_id: "landsat",
        display_name_zh: "Landsat C2 L2",
        family: "optical",
        search_mode: "optical",
        filter_schema_id: null,
        filter_schema_revision: null,
        result_cap: 250,
        accounts: [],
        network_default: "direct",
      },
    ],
    getSourceViews: async () => [
      {
        resource_id: "science.sentinel-1.standard.v1",
        batch_id: "batch-s1",
        revision: 1,
        item_count: 1,
        searched_at: "2026-11-24T00:00:00Z",
        pending_request_id: null,
        last_error: null,
      },
      {
        resource_id: "science.landsat.c2-l2.v1",
        batch_id: "batch-landsat",
        revision: 2,
        item_count: 1,
        searched_at: "2026-11-24T01:00:00Z",
        pending_request_id: null,
        last_error: null,
      },
    ],
    getPreferredActiveResourceId: async () => "science.landsat.c2-l2.v1",
    onRestoreActiveResourceId: (id) => {
      restoredId = id;
    },
    getBatchPage: async (batchId) => {
      loadedBatchIds.push(batchId);
      return { scenes: [{ scene_id: "LC09_RESTORED_TEST" }], rows: [] };
    },
    getWorkingAoi: async () => null,
    getCredentialStatus: async () => null,
    getSarDownloadPolicySettings: async () => ({ ok: false }),
    getDemTaskSettings: async () => ({ ok: false }),
    isDesktopHost: () => false,
    setAvailableSources: () => {},
    setSourceViews: () => {},
    setScenes: (s) => {
      loadedScenes = s;
    },
    setRowStates: () => {},
    setWorkingGeometry: () => {},
    setWorkingAoiState: () => {},
    setAoiLabel: () => {},
    setCreds: () => {},
    setStorageRootPath: () => {},
    setSarDownloadPolicy: () => {},
    setDemConcurrency: () => {},
  };

  const run = createStartupEffect(deps);
  const cleanup = run();
  await new Promise((r) => setTimeout(r, 30));

  assert.strictEqual(restoredId, "science.landsat.c2-l2.v1");
  assert.deepStrictEqual(loadedBatchIds, ["batch-landsat"]);
  assert.strictEqual(loadedScenes[0]?.scene_id, "LC09_RESTORED_TEST");
  cleanup();

  // Unknown non-static ID falls back to default Sentinel-1
  let unknownRestored: string | null = null;
  const unknownLoadedBatches: string[] = [];
  const runUnknown = createStartupEffect({
    ...deps,
    getPreferredActiveResourceId: async () => "science.nonexistent.v99",
    onRestoreActiveResourceId: (id) => {
      unknownRestored = id;
    },
    getBatchPage: async (batchId) => {
      unknownLoadedBatches.push(batchId);
      return { scenes: [], rows: [] };
    },
  });
  const cleanupUnknown = runUnknown();
  await new Promise((r) => setTimeout(r, 30));
  assert.strictEqual(unknownRestored, null);
  assert.deepStrictEqual(unknownLoadedBatches, ["batch-s1"]);
  cleanupUnknown();
});
