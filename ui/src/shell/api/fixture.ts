/**
 * C1 Contract Mock Fixture 模块
 * 供切片 2 数据工作台数据流与交互测试使用
 */
import type {
  SourceDescriptor,
  SourceView,
  AccountStatus,
  RowTaskState,
  TaskRef,
  SearchProgress,
  SearchCancelOutcome,
  BatchPageResult,
  SearchStartParams,
  CreateTaskOptions,
  RowAction,
  TypedError,
  ApplicationTask,
  TaskRetryFallbackOutcome,
} from "./types.ts";
import type { SceneRow, Bbox } from "../../lib/bridge.ts";
import type {
  AgentApprovalRecord,
  AgentClaim,
  AgentConfirmCardData,
  AgentMessage,
  AgentSession,
  AgentPlanResourceSummary,
  AgentTaskProgressCardData,
} from "./agentTypes.ts";
import { freezeApprovalSnapshot } from "./agentLogic.ts";
import { RESULT_PAGE_SIZE } from "./batchDeliveryLogic.ts";
import { isTaskHistoryRow } from "./taskLogic.ts";

/**
 * 契约硬性规定：带 __fixture 标记，构建时可一键识别与切换到真实 bridge
 */
export const __fixture = true as const;

export const isFixtureBackend = (): boolean => true;

export const FIXTURE_SOURCES: SourceDescriptor[] = [
  {
    resource_id: "science.sentinel-1.standard.v1",
    group_id: "sentinel",
    display_name_zh: "Sentinel-1 雷达卫星",
    family: "sar",
    search_mode: "sar",
    filter_schema_id: "s1-sar-v1",
    filter_schema_revision: 1,
    result_cap: 250,
    accounts: ["earthdata"],
    network_default: "direct",
    availability: "available",
    availability_reason_zh: null,
  },
  {
    resource_id: "science.nisar.l.rslc.provisional.v1",
    group_id: "nisar",
    display_name_zh: "NISAR L波段 (ASF)",
    family: "sar",
    search_mode: "sar",
    filter_schema_id: "nisar-l-v1",
    filter_schema_revision: 1,
    result_cap: 250,
    accounts: ["earthdata"],
    network_default: "direct",
    availability: "available",
    availability_reason_zh: null,
  },
  {
    resource_id: "science.nisar.s.ssar.rslc.v1",
    group_id: "nisar",
    display_name_zh: "NISAR S波段 (Bhoonidhi)",
    family: "sar",
    search_mode: "sar",
    filter_schema_id: "nisar-s-v1",
    filter_schema_revision: 1,
    result_cap: 250,
    accounts: ["bhoonidhi"],
    network_default: "system_proxy",
    availability: "available",
    availability_reason_zh: null,
  },
  {
    resource_id: "science.alos-2.palsar-2.scansar-l1.1.v1",
    group_id: "alos",
    display_name_zh: "ALOS-2 PALSAR-2",
    family: "sar",
    search_mode: "sar",
    filter_schema_id: "alos2-v1",
    filter_schema_revision: 1,
    result_cap: 250,
    accounts: ["earthdata"],
    network_default: "direct",
    availability: "available",
    availability_reason_zh: null,
  },
  {
    resource_id: "science.sentinel-2.l2a.v1",
    group_id: "sentinel-2",
    display_name_zh: "Sentinel-2 多光谱",
    family: "optical",
    search_mode: "catalogue_intent",
    filter_schema_id: "search.optical.sentinel-2.l2a.v1",
    filter_schema_revision: 1,
    result_cap: 250,
    accounts: ["cdse"],
    network_default: "direct",
    availability: "degraded",
    availability_reason_zh: "光学检索接口适配中",
  },
  {
    resource_id: "dem.cop30.v1",
    group_id: "dem",
    display_name_zh: "Copernicus DEM 30m",
    family: "dem",
    search_mode: "auxiliary",
    filter_schema_id: null,
    filter_schema_revision: null,
    result_cap: 50,
    accounts: [],
    network_default: "direct",
    availability: "available",
    availability_reason_zh: null,
  },
];

// 初始预设场景数据（包含科学字段如实映射；缺字段保持 null，绝不伪造）
const INITIAL_S1_SCENES: SceneRow[] = [
  {
    scene_id: "S1A_IW_SLC__1SDV_20240408T100132_20240408T100159_053341_067676_8A12",
    platform: "Sentinel-1A",
    beam_mode: "IW",
    product_type: "SLC",
    polarization: "VV+VH",
    acquisition_datetime: "2024-04-08T10:01:32Z",
    absolute_orbit: 53341,
    relative_orbit: 69,
    path: 69,
    frame: 450, // __fixture: 供截图与分堆联动展示同轨同幅 (69:450)
    orbit_direction: "ascending",
    // __fixture: 围绕茂县叠溪 AOI 覆盖的升轨真实正交旋转矩形足迹 (cos(31.8°)=0.85 投影正交校准)
    footprint_bbox: { west: 102.38, south: 31.0, east: 104.86, north: 32.8 },
    footprint_geojson: {
      type: "Polygon",
      coordinates: [
        [
          [102.66, 31.0],
          [104.86, 31.3],
          [104.58, 32.8],
          [102.38, 32.5],
          [102.66, 31.0],
        ],
      ],
    },
    has_url: false,
    assets: [
      {
        role: "thumbnail",
        url: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAAAAADhZOFXAAAAK0lEQVR4nGNISM0uLKtp6mCA0r0MUHoyA5SeyQCl5zFA6SUMUHoVA5TeAAB5syHm1H7/bgAAAABJRU5ErkJggg==",
        stable_asset_key: "__fixture_thumbnail_0",
        file_name: "thumb.png",
        media_type: "image/png",
        size_bytes: 100,
      },
    ],
  },
  {
    scene_id: "S1A_IW_SLC__1SDV_20240327T100132_20240327T100159_053166_067092_F88B",
    platform: "Sentinel-1A",
    beam_mode: "IW",
    product_type: "SLC",
    polarization: "VV+VH",
    acquisition_datetime: "2024-03-27T10:01:32Z",
    absolute_orbit: 53166,
    relative_orbit: 69,
    path: 69,
    frame: 450, // __fixture: 供截图与分堆联动展示同轨同幅 (69:450)
    orbit_direction: "ascending",
    // __fixture: 与首景重合的同轨同幅正交矩形足迹
    footprint_bbox: { west: 102.39, south: 31.01, east: 104.87, north: 32.81 },
    footprint_geojson: {
      type: "Polygon",
      coordinates: [
        [
          [102.67, 31.01],
          [104.87, 31.31],
          [104.59, 32.81],
          [102.39, 32.51],
          [102.67, 31.01],
        ],
      ],
    },
    has_url: false,
  },
  {
    scene_id: "S1A_IW_SLC__1SDV_20240409T220815_20240409T220842_053363_06772D_1D02",
    platform: "Sentinel-1A",
    beam_mode: "IW",
    product_type: "SLC",
    polarization: "VV+VH",
    acquisition_datetime: "2024-04-09T22:08:15Z",
    absolute_orbit: 53363,
    relative_orbit: 84,
    path: 84,
    frame: null, // 真实数据缺少帧号时如实为 null，界面诚实展示「未知」
    orbit_direction: "descending",
    // __fixture: 降轨交叉过境正交旋转矩形足迹 (NNE->SSW 倾角正交校准)
    footprint_bbox: { west: 102.41, south: 31.0, east: 104.89, north: 32.8 },
    footprint_geojson: {
      type: "Polygon",
      coordinates: [
        [
          [102.69, 32.8],
          [104.89, 32.5],
          [104.61, 31.0],
          [102.41, 31.3],
          [102.69, 32.8],
        ],
      ],
    },
    has_url: false,
  },
  {
    scene_id: "S1A_IW_SLC__1SDV_20240328T220815_20240328T220842_053188_06714E_99B1",
    platform: "Sentinel-1A",
    beam_mode: "IW",
    product_type: "SLC",
    polarization: "VV+VH",
    acquisition_datetime: "2024-03-28T22:08:15Z",
    absolute_orbit: 53188,
    relative_orbit: 84,
    path: 84,
    frame: null, // 未经权威验证保持 null，诚实展示「未知」
    orbit_direction: "descending",
    // __fixture: 降轨正交旋转矩形足迹
    footprint_bbox: { west: 102.41, south: 31.0, east: 104.89, north: 32.8 },
    footprint_geojson: {
      type: "Polygon",
      coordinates: [
        [
          [102.69, 32.8],
          [104.89, 32.5],
          [104.61, 31.0],
          [102.41, 31.3],
          [102.69, 32.8],
        ],
      ],
    },
    has_url: false,
  },
  {
    scene_id: "S1B_IW_SLC__1SDV_20211215T100130_20211215T100157_030030_0396A1_7C18",
    platform: "Sentinel-1B",
    beam_mode: "IW",
    product_type: "SLC",
    polarization: "VV+VH",
    acquisition_datetime: "2021-12-15T10:01:30Z",
    absolute_orbit: 30030,
    relative_orbit: 69,
    path: 69,
    frame: null, // 审计 P0：不为历史样本编造 frame，如实展示「未知」
    orbit_direction: "ascending",
    // __fixture: 偏南升轨正交旋转矩形足迹
    footprint_bbox: { west: 102.48, south: 30.5, east: 104.96, north: 32.3 },
    footprint_geojson: {
      type: "Polygon",
      coordinates: [
        [
          [102.76, 30.5],
          [104.96, 30.8],
          [104.68, 32.3],
          [102.48, 32.0],
          [102.76, 30.5],
        ],
      ],
    },
    has_url: false,
  },
];

// __fixture: 8 景模式足迹定义（三条围绕茂县叠溪的交叉覆盖正交旋转矩形轨道）
// 轨道 1: 升轨 (Ascending, RO 11, Frame 65) 包含 scenes 0, 3, 6
const S1_TRACK1_FOOTPRINT__fixture = {
  footprint_bbox: { west: 102.38, south: 31.0, east: 104.86, north: 32.8 },
  footprint_geojson: {
    type: "Polygon" as const,
    coordinates: [
      [
        [102.66, 31.0],
        [104.86, 31.3],
        [104.58, 32.8],
        [102.38, 32.5],
        [102.66, 31.0],
      ],
    ],
  },
};

// 轨道 2: 降轨 (Descending, RO 135, Frame 471) 包含 scenes 1, 4, 7
const S1_TRACK2_FOOTPRINT__fixture = {
  footprint_bbox: { west: 102.41, south: 31.0, east: 104.89, north: 32.8 },
  footprint_geojson: {
    type: "Polygon" as const,
    coordinates: [
      [
        [102.69, 32.8],
        [104.89, 32.5],
        [104.61, 31.0],
        [102.41, 31.3],
        [102.69, 32.8],
      ],
    ],
  },
};

// 轨道 3: 升轨 (Ascending, RO 84, Frame 70) 包含 scenes 2, 5
const S1_TRACK3_FOOTPRINT__fixture = {
  footprint_bbox: { west: 101.98, south: 30.85, east: 104.46, north: 32.65 },
  footprint_geojson: {
    type: "Polygon" as const,
    coordinates: [
      [
        [102.26, 30.85],
        [104.46, 31.15],
        [104.18, 32.65],
        [101.98, 32.35],
        [102.26, 30.85],
      ],
    ],
  },
};

const FIXTURE_GRAY_THUMBNAIL_ASSETS: NonNullable<SceneRow["assets"]> = [
  {
    role: "thumbnail",
    url: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAAAAADhZOFXAAAAK0lEQVR4nGNISM0uLKtp6mCA0r0MUHoyA5SeyQCl5zFA6SUMUHoVA5TeAAB5syHm1H7/bgAAAABJRU5ErkJggg==",
    stable_asset_key: "__fixture_thumb",
    file_name: "thumb.png",
    media_type: "image/png",
    size_bytes: 100,
  },
];

export const MOCKUP_8_SCENES__fixture: SceneRow[] = [
  {
    scene_id: "S1A_IW_SLC__1SDV_20261022T103247_20261022T103314_066758_0849A2_3F1C",
    platform: "Sentinel-1A",
    beam_mode: "IW",
    product_type: "SLC",
    polarization: "VV+VH",
    acquisition_datetime: "2026-10-22T10:32:47Z",
    absolute_orbit: 66758,
    relative_orbit: 11,
    path: 11,
    frame: 65,
    orbit_direction: "ascending",
    file_size_remote: 4402341478, // 4.1 GB
    assets: FIXTURE_GRAY_THUMBNAIL_ASSETS,
    ...S1_TRACK1_FOOTPRINT__fixture,
    has_url: false,
  },
  {
    scene_id: "S1A_IW_SLC__1SDV_20261018T223105_20261018T223132_066707_084715_B20D",
    platform: "Sentinel-1A",
    beam_mode: "IW",
    product_type: "SLC",
    polarization: "VV+VH",
    acquisition_datetime: "2026-10-18T22:31:05Z",
    absolute_orbit: 66707,
    relative_orbit: 135,
    path: 135,
    frame: 471,
    orbit_direction: "descending",
    file_size_remote: 4617089843, // 4.3 GB
    assets: FIXTURE_GRAY_THUMBNAIL_ASSETS,
    ...S1_TRACK2_FOOTPRINT__fixture,
    has_url: false,
  },
  {
    scene_id: "S1A_IW_SLC__1SDV_20261015T104012_20261015T104039_066656_08452B_9C41",
    platform: "Sentinel-1A",
    beam_mode: "IW",
    product_type: "SLC",
    polarization: "VV+VH",
    acquisition_datetime: "2026-10-15T10:40:12Z",
    absolute_orbit: 66656,
    relative_orbit: 84,
    path: 84,
    frame: 70,
    orbit_direction: "ascending",
    file_size_remote: 4294967296, // 4.0 GB
    assets: [], // N/A (来源未提供)
    ...S1_TRACK3_FOOTPRINT__fixture,
    has_url: false,
  },
  {
    scene_id: "S1A_IW_SLC__1SDV_20261010T103247_20261010T103314_066583_0841F7_7A5E",
    platform: "Sentinel-1A",
    beam_mode: "IW",
    product_type: "SLC",
    polarization: "VV+VH",
    acquisition_datetime: "2026-10-10T10:32:47Z",
    absolute_orbit: 66583,
    relative_orbit: 11,
    path: 11,
    frame: 65,
    orbit_direction: "ascending",
    file_size_remote: 4402341478, // 4.1 GB
    assets: FIXTURE_GRAY_THUMBNAIL_ASSETS,
    ...S1_TRACK1_FOOTPRINT__fixture,
    has_url: false,
  },
  {
    scene_id: "S1A_IW_SLC__1SDV_20261006T223106_20261006T223133_066532_083F6A_C0E4",
    platform: "Sentinel-1A",
    beam_mode: "IW",
    product_type: "SLC",
    polarization: "VV+VH",
    acquisition_datetime: "2026-10-06T22:31:06Z",
    absolute_orbit: 66532,
    relative_orbit: 135,
    path: 135,
    frame: 471,
    orbit_direction: "descending",
    file_size_remote: 4509715660, // 4.2 GB
    assets: FIXTURE_GRAY_THUMBNAIL_ASSETS,
    ...S1_TRACK2_FOOTPRINT__fixture,
    has_url: false,
  },
  {
    scene_id: "S1A_IW_SLC__1SDV_20261003T104012_20261003T104039_066481_083D80_19A7",
    platform: "Sentinel-1A",
    beam_mode: "IW",
    product_type: "SLC",
    polarization: "VV+VH",
    acquisition_datetime: "2026-10-03T10:40:12Z",
    absolute_orbit: 66481,
    relative_orbit: 84,
    path: 84,
    frame: 70,
    orbit_direction: "ascending",
    file_size_remote: 4294967296, // 4.0 GB
    assets: FIXTURE_GRAY_THUMBNAIL_ASSETS,
    ...S1_TRACK3_FOOTPRINT__fixture,
    has_url: false,
  },
  {
    scene_id: "S1A_IW_SLC__1SDV_20260928T103248_20260928T103315_066408_083A4C_E5B2",
    platform: "Sentinel-1A",
    beam_mode: "IW",
    product_type: "SLC",
    polarization: "VV+VH",
    acquisition_datetime: "2026-09-28T10:32:48Z",
    absolute_orbit: 66408,
    relative_orbit: 11,
    path: 11,
    frame: 65,
    orbit_direction: "ascending",
    file_size_remote: 4402341478, // 4.1 GB
    assets: [], // N/A (来源未提供)
    ...S1_TRACK1_FOOTPRINT__fixture,
    has_url: false,
  },
  {
    scene_id: "S1A_IW_SLC__1SDV_20260924T223105_20260924T223132_066357_0837BF_6D09",
    platform: "Sentinel-1A",
    beam_mode: "IW",
    product_type: "SLC",
    polarization: "VV+VH",
    acquisition_datetime: "2026-09-24T22:31:05Z",
    absolute_orbit: 66357,
    relative_orbit: 135,
    path: 135,
    frame: 471,
    orbit_direction: "descending",
    file_size_remote: 4617089843, // 4.3 GB
    assets: FIXTURE_GRAY_THUMBNAIL_ASSETS,
    ...S1_TRACK2_FOOTPRINT__fixture,
    has_url: false,
  },
];

const INITIAL_NISAR_L_SCENES: SceneRow[] = [
  {
    scene_id: "NISAR_L_PR_SLC_001_024_A_005_20240501T120000_20240501T120030_M01",
    platform: "NISAR",
    beam_mode: "PR",
    product_type: "SLC",
    polarization: "HH+HV",
    acquisition_datetime: "2024-05-01T12:00:00Z",
    absolute_orbit: 1024,
    relative_orbit: 24,
    path: 24,
    frame: null, // 未经权威验证保持 null，诚实展示「未知」
    orbit_direction: "ascending",
    footprint_bbox: { west: 121.0, south: 23.4, east: 122.0, north: 24.4 },
    footprint_geojson: null,
    has_url: false,
  },
  {
    scene_id: "NISAR_L_PR_SLC_001_024_A_005_20240513T120000_20240513T120030_M01",
    platform: "NISAR",
    beam_mode: "PR",
    product_type: "SLC",
    polarization: "HH+HV",
    acquisition_datetime: "2024-05-13T12:00:00Z",
    absolute_orbit: 1196,
    relative_orbit: 24,
    path: 24,
    frame: null, // 未经权威验证保持 null，诚实展示「未知」
    orbit_direction: "ascending",
    footprint_bbox: { west: 121.0, south: 23.4, east: 122.0, north: 24.4 },
    footprint_geojson: null,
    has_url: false,
  },
];

const INITIAL_NISAR_S_SCENES: SceneRow[] = [
  {
    scene_id: "NISAR_S_PR_SLC_ISRO_002_048_D_010_20240502T040000_M01",
    platform: "NISAR",
    beam_mode: "PR",
    product_type: "SLC",
    polarization: "HH",
    acquisition_datetime: "2024-05-02T04:00:00Z",
    absolute_orbit: 1035,
    relative_orbit: 48,
    path: 48,
    frame: null, // 未经权威验证保持 null，诚实展示「未知」
    orbit_direction: "descending",
    footprint_bbox: { west: 121.3, south: 23.7, east: 121.9, north: 24.3 },
    footprint_geojson: null,
    has_url: false,
  },
];

// 内存数据库
class FixtureDatabase {
  private sources: Map<string, SourceDescriptor> = new Map();
  private sourceViews: Map<string, SourceView> = new Map();
  private batches: Map<string, { revision: number; scenes: SceneRow[]; rows: Map<string, RowTaskState> }> = new Map();
  private activeRequests: Map<string, { progress: SearchProgress; targetOutcome: "done" | "failed"; pollCount: number }> = new Map();
  private tasks: Map<string, ApplicationTask> = new Map();
  private nextSearchOutcome: "done" | "failed" = "done";
  private currentSubscene: ManualWorkbenchSubscene | null = null;

  constructor() {
    this.reset();
  }

  public getCurrentSubscene(): ManualWorkbenchSubscene | null {
    return this.currentSubscene;
  }

  public reset(): void {
    this.sources.clear();
    this.sourceViews.clear();
    this.batches.clear();
    this.activeRequests.clear();
    this.tasks.clear();
    this.nextSearchOutcome = "done";

    const subscene = currentManualWorkbenchSubscene();
    this.currentSubscene = subscene;

    for (const s of FIXTURE_SOURCES) {
      this.sources.set(s.resource_id, s);
    }

    if (subscene === "empty") {
      this.sourceViews.set("science.sentinel-1.standard.v1", {
        resource_id: "science.sentinel-1.standard.v1",
        batch_id: null,
        revision: 0,
        item_count: 0,
        searched_at: null,
        pending_request_id: null,
        last_error: null,
      });
    } else if (subscene === "results-8") {
      const s1BatchId = "batch-s1-results-8";
      const s1Rows = new Map<string, RowTaskState>();
      for (const sc of MOCKUP_8_SCENES__fixture) {
        s1Rows.set(sc.scene_id, {
          item_id: sc.scene_id,
          task_id: null,
          task_status: null,
          item_status: null,
          actions: ["download"],
        });
      }
      this.batches.set(s1BatchId, { revision: 1, scenes: MOCKUP_8_SCENES__fixture, rows: s1Rows });
      this.sourceViews.set("science.sentinel-1.standard.v1", {
        resource_id: "science.sentinel-1.standard.v1",
        batch_id: s1BatchId,
        revision: 1,
        item_count: 8,
        searched_at: new Date().toISOString(),
        pending_request_id: null,
        last_error: null,
      });
    } else if (subscene === "results-tasks") {
      const s1BatchId = "batch-s1-results-tasks";
      const s1Rows = new Map<string, RowTaskState>();

      // 1. 传输中 46% (Scene 0)
      const runningId = MOCKUP_8_SCENES__fixture[0].scene_id;
      s1Rows.set(runningId, {
        item_id: runningId,
        task_id: "task-s1-running-46",
        task_status: "running",
        item_status: "active",
        actions: ["pause", "open_task"],
      });
      this.tasks.set("task-s1-running-46", {
        task_id: "task-s1-running-46",
        revision: 1,
        category: "scene_download",
        name: "Sentinel-1 SLC 单景下载",
        provider: "ASF",
        dataset: "Sentinel-1",
        status: "running",
        done_bytes: 2025077080,
        total_bytes: 4402341478,
        bytes_per_second: 10485760,
        speed_formatted: "10 MB/s",
        output_dir: "F:/InSAR_Data/s1_slc",
        items_count: 1,
        completed_items_count: 0,
        failed_items_count: 0,
        scene_ids: [runningId],
        created_at: "2026-10-26T08:00:00Z",
        updated_at: "2026-10-26T08:00:30Z",
      });

      // 2. 已暂停 31% (Scene 1)
      const pausedId = MOCKUP_8_SCENES__fixture[1].scene_id;
      s1Rows.set(pausedId, {
        item_id: pausedId,
        task_id: "task-s1-paused-31",
        task_status: "paused",
        item_status: "paused",
        actions: ["resume"],
      });
      this.tasks.set("task-s1-paused-31", {
        task_id: "task-s1-paused-31",
        revision: 1,
        category: "scene_download",
        name: "Sentinel-1 SLC 单景下载",
        provider: "ASF",
        dataset: "Sentinel-1",
        status: "paused",
        done_bytes: 1431297852,
        total_bytes: 4617089843,
        speed_formatted: "已暂停",
        output_dir: "F:/InSAR_Data/s1_slc",
        pause_reason: { kind: "disk_low", free_bytes: 1288490188, threshold_bytes: 2147483648, drive: "F:" },
        items_count: 1,
        completed_items_count: 0,
        failed_items_count: 0,
        scene_ids: [pausedId],
        created_at: "2026-10-26T08:05:00Z",
        updated_at: "2026-10-26T08:05:30Z",
      });

      // 3. 失败 (Scene 2)
      const failedId = MOCKUP_8_SCENES__fixture[2].scene_id;
      const failReason = "下载连接中断，网络请求超时。请检查网络配置或代理设置后重试。";
      s1Rows.set(failedId, {
        item_id: failedId,
        task_id: "task-s1-failed-net",
        task_status: "failed",
        item_status: "failed",
        actions: ["retry"],
      });
      this.tasks.set("task-s1-failed-net", {
        task_id: "task-s1-failed-net",
        revision: 1,
        category: "scene_download",
        name: "Sentinel-1 SLC 单景下载",
        provider: "ASF",
        dataset: "Sentinel-1",
        status: "failed",
        done_bytes: 450000000,
        total_bytes: 4294967296,
        output_dir: "F:/InSAR_Data/s1_slc",
        error: failReason,
        failure: {
          codes: ["NET002"],
          message_zh: failReason,
          failed_items: 1,
          total_items: 1,
        },
        items_count: 1,
        completed_items_count: 0,
        failed_items_count: 1,
        scene_ids: [failedId],
        created_at: "2026-10-26T08:10:00Z",
        updated_at: "2026-10-26T08:10:30Z",
      });

      // 4. 已完成 (Scene 3)
      const completedId = MOCKUP_8_SCENES__fixture[3].scene_id;
      s1Rows.set(completedId, {
        item_id: completedId,
        task_id: "task-s1-completed-done",
        task_status: "completed",
        item_status: "completed",
        actions: [],
      });
      this.tasks.set("task-s1-completed-done", {
        task_id: "task-s1-completed-done",
        revision: 1,
        category: "scene_download",
        name: "Sentinel-1 SLC 单景下载",
        provider: "ASF",
        dataset: "Sentinel-1",
        status: "completed",
        done_bytes: 4402341478,
        total_bytes: 4402341478,
        speed_formatted: "完成",
        output_dir: "F:/InSAR_Data/s1_slc",
        items_count: 1,
        completed_items_count: 1,
        failed_items_count: 0,
        scene_ids: [completedId],
        created_at: "2026-10-26T08:15:00Z",
        updated_at: "2026-10-26T08:16:30Z",
      });

      // 其余 4 景无任务
      for (let i = 4; i < MOCKUP_8_SCENES__fixture.length; i++) {
        const sc = MOCKUP_8_SCENES__fixture[i];
        s1Rows.set(sc.scene_id, {
          item_id: sc.scene_id,
          task_id: null,
          task_status: null,
          item_status: null,
          actions: ["download"],
        });
      }

      this.batches.set(s1BatchId, { revision: 1, scenes: MOCKUP_8_SCENES__fixture, rows: s1Rows });
      this.sourceViews.set("science.sentinel-1.standard.v1", {
        resource_id: "science.sentinel-1.standard.v1",
        batch_id: s1BatchId,
        revision: 1,
        item_count: 8,
        searched_at: new Date().toISOString(),
        pending_request_id: null,
        last_error: null,
      });
    } else {
      // 默认情况：保留今天原有的 INITIAL_S1_SCENES 5 景初始化
      const s1BatchId = "batch-s1-init";
      const s1Rows = new Map<string, RowTaskState>();
      const representativeStates: RowTaskState[] = [
        {
          item_id: INITIAL_S1_SCENES[0].scene_id,
          task_id: null,
          task_status: null,
          item_status: null,
          actions: ["download"], // 未建任务
        },
        {
          item_id: INITIAL_S1_SCENES[1].scene_id,
          task_id: "task-s1-queued",
          task_status: "queued",
          item_status: "queued",
          actions: ["download"], // 排队中
        },
        {
          item_id: INITIAL_S1_SCENES[2].scene_id,
          task_id: "task-s1-running",
          task_status: "running",
          item_status: "active",
          actions: ["pause", "open_task"], // 下载中 (传输中)
        },
        {
          item_id: INITIAL_S1_SCENES[3].scene_id,
          task_id: "task-s1-paused",
          task_status: "paused",
          item_status: "paused",
          actions: ["resume"], // 已暂停
        },
        {
          item_id: INITIAL_S1_SCENES[4].scene_id,
          task_id: "task-s1-failed",
          task_status: "failed",
          item_status: "failed",
          actions: ["retry"], // 失败
        },
      ];
      for (const st of representativeStates) {
        s1Rows.set(st.item_id, st);
      }
      this.batches.set(s1BatchId, { revision: 1, scenes: INITIAL_S1_SCENES, rows: s1Rows });
      this.sourceViews.set("science.sentinel-1.standard.v1", {
        resource_id: "science.sentinel-1.standard.v1",
        batch_id: s1BatchId,
        revision: 1,
        item_count: INITIAL_S1_SCENES.length,
        searched_at: "2024-04-10T08:00:00Z",
        pending_request_id: null,
        last_error: null,
      });
    }

    // 初始化 NISAR L 批次与视图 (独立保留，切源不清空)
    const nisarLBatchId = "batch-nisar-l-init";
    this.createBatch(nisarLBatchId, 1, INITIAL_NISAR_L_SCENES);
    this.sourceViews.set("science.nisar.l.rslc.provisional.v1", {
      resource_id: "science.nisar.l.rslc.provisional.v1",
      batch_id: nisarLBatchId,
      revision: 1,
      item_count: INITIAL_NISAR_L_SCENES.length,
      searched_at: "2024-05-14T08:00:00Z",
      pending_request_id: null,
      last_error: null,
    });

    // 初始化 NISAR S 批次与视图 (S/L 同组独立保留)
    const nisarSBatchId = "batch-nisar-s-init";
    this.createBatch(nisarSBatchId, 1, INITIAL_NISAR_S_SCENES);
    this.sourceViews.set("science.nisar.s.ssar.rslc.v1", {
      resource_id: "science.nisar.s.ssar.rslc.v1",
      batch_id: nisarSBatchId,
      revision: 1,
      item_count: INITIAL_NISAR_S_SCENES.length,
      searched_at: "2024-05-03T08:00:00Z",
      pending_request_id: null,
      last_error: null,
    });

    // 初始化任务中心数据 (仅在非 results-tasks 子场景时填充默认 8 项任务，results-tasks 保留与行状态一致的 4 个任务)
    if (subscene !== "results-tasks") {
      this.tasks.clear();
      const defaultTasks: ApplicationTask[] = [
      {
        task_id: "mock-task-c1-running-1",
        revision: 1,
        category: "scene_download",
        name: "Sentinel-1 SLC 5 景批次",
        provider: "NASA Earthdata",
        dataset: "Sentinel-1",
        status: "running",
        done_bytes: 1450000000,
        total_bytes: 4200000000,
        speed_formatted: "18.5 MB/s",
        output_dir: "F:/InSAR_Data/s1_slc_202610",
        items_count: 5,
        completed_items_count: 1,
        failed_items_count: 0,
        scene_ids: [
          "S1A_IW_SLC__1SDV_20240410T100122_20240410T100149_053370_06771F_C726",
          "S1A_IW_SLC__1SDV_20240410T100147_20240410T100214_053370_06771F_E31A",
          "S1A_IW_SLC__1SDV_20240410T100212_20240410T100239_053370_06771F_98BC",
          "S1A_IW_SLC__1SDV_20240410T100237_20240410T100304_053370_06771F_2211",
          "S1A_IW_SLC__1SDV_20240410T100302_20240410T100329_053370_06771F_FA89",
        ],
        scene_items: [
          {
            scene_id: "S1A_IW_SLC__1SDV_20240410T100122_20240410T100149_053370_06771F_C726",
            status: "completed",
            done_bytes: 840000000,
            total_bytes: 840000000,
          },
          {
            scene_id: "S1A_IW_SLC__1SDV_20240410T100147_20240410T100214_053370_06771F_E31A",
            status: "running",
            done_bytes: 610000000,
            total_bytes: 840000000,
          },
          {
            scene_id: "S1A_IW_SLC__1SDV_20240410T100212_20240410T100239_053370_06771F_98BC",
            status: "queued",
            done_bytes: 0,
            total_bytes: 840000000,
          },
          {
            scene_id: "S1A_IW_SLC__1SDV_20240410T100237_20240410T100304_053370_06771F_2211",
            status: "queued",
            done_bytes: 0,
            total_bytes: 840000000,
          },
          {
            scene_id: "S1A_IW_SLC__1SDV_20240410T100302_20240410T100329_053370_06771F_FA89",
            status: "queued",
            done_bytes: 0,
            total_bytes: 840000000,
          },
        ],
        created_at: "2026-10-24T02:15:00Z",
        updated_at: "2026-10-24T02:30:00Z",
      },
      {
        task_id: "mock-task-c1-queued-2",
        revision: 1,
        category: "scene_download",
        name: "NISAR L波段 2 景批次",
        provider: "NASA Earthdata",
        dataset: "NISAR L",
        status: "queued",
        done_bytes: 0,
        total_bytes: 1800000000,
        output_dir: "F:/InSAR_Data/nisar_l",
        scene_ids: [
          "NISAR_L_PR_20240410T100122_001",
          "NISAR_L_PR_20240410T100147_002",
        ],
        scene_items: [
          {
            scene_id: "NISAR_L_PR_20240410T100122_001",
            status: "queued",
            done_bytes: 0,
            total_bytes: 900000000,
          },
          {
            scene_id: "NISAR_L_PR_20240410T100147_002",
            status: "queued",
            done_bytes: 0,
            total_bytes: 900000000,
          },
        ],
        items_count: 2,
        completed_items_count: 0,
        failed_items_count: 0,
        created_at: "2026-10-24T02:20:00Z",
        updated_at: "2026-10-24T02:20:00Z",
      },
      {
        task_id: "mock-task-c1-paused-3",
        revision: 1,
        category: "scene_download",
        name: "Sentinel-1 GRD 3 景批次",
        provider: "ASF",
        dataset: "Sentinel-1",
        status: "paused",
        done_bytes: 800000000,
        total_bytes: 2500000000,
        output_dir: "F:/InSAR_Data/s1_grd",
        scene_ids: [
          "S1A_IW_GRDH_1SDV_20240410T100122_053370_GRD1",
          "S1A_IW_GRDH_1SDV_20240410T100147_053370_GRD2",
          "S1A_IW_GRDH_1SDV_20240410T100212_053370_GRD3",
        ],
        scene_items: [
          {
            scene_id: "S1A_IW_GRDH_1SDV_20240410T100122_053370_GRD1",
            status: "completed",
            done_bytes: 800000000,
            total_bytes: 800000000,
          },
          {
            scene_id: "S1A_IW_GRDH_1SDV_20240410T100147_053370_GRD2",
            status: "paused",
            done_bytes: 0,
            total_bytes: 850000000,
          },
          {
            scene_id: "S1A_IW_GRDH_1SDV_20240410T100212_053370_GRD3",
            status: "paused",
            done_bytes: 0,
            total_bytes: 850000000,
          },
        ],
        items_count: 3,
        completed_items_count: 1,
        failed_items_count: 0,
        created_at: "2026-10-24T01:45:00Z",
        updated_at: "2026-10-24T02:00:00Z",
      },
      {
        // __fixture: 磁盘空间不足暂停（W2d pause_reason，演示数据）
        task_id: "mock-task-c1-disklow-5",
        revision: 2,
        category: "scene_download",
        name: "Sentinel-1 SLC 2 景批次",
        provider: "ASF",
        dataset: "Sentinel-1",
        status: "paused",
        done_bytes: 2100000000,
        total_bytes: 8400000000,
        output_dir: "D:/InSAR_Data/s1_slc_disk",
        scene_ids: [
          "S1A_IW_SLC__1SDV_20240411T120000_DISK1",
          "S1A_IW_SLC__1SDV_20240411T120025_DISK2",
        ],
        scene_items: [
          {
            scene_id: "S1A_IW_SLC__1SDV_20240411T120000_DISK1",
            status: "paused",
            done_bytes: 2100000000,
            total_bytes: 4200000000,
          },
          {
            scene_id: "S1A_IW_SLC__1SDV_20240411T120025_DISK2",
            status: "paused",
            done_bytes: 0,
            total_bytes: 4200000000,
          },
        ],
        items_count: 2,
        completed_items_count: 0,
        failed_items_count: 0,
        created_at: "2026-10-24T01:50:00Z",
        updated_at: "2026-10-24T02:10:00Z",
        pause_reason: { kind: "disk_low", free_bytes: 1288490188, threshold_bytes: 2147483648, drive: "D:" },
      },
      {
        task_id: "mock-task-c1-dem-running-4",
        revision: 1,
        category: "dem_convert",
        name: "Copernicus DEM 30m 高程切片处理",
        provider: "Copernicus",
        dataset: "COP30 DEM",
        status: "running",
        done_bytes: 156000000,
        total_bytes: null,
        speed_formatted: "3.2 MB/s",
        output_dir: "F:/InSAR_Data/dem_cop30",
        items_count: 12,
        completed_items_count: 4,
        failed_items_count: 0,
        created_at: "2026-10-24T02:25:00Z",
        updated_at: "2026-10-24T02:30:00Z",
      },
      {
        task_id: "mock-task-c1-cancelled-9",
        revision: 1,
        category: "scene_download",
        name: "Sentinel-1 SLC 1 景批次",
        provider: "ASF",
        dataset: "Sentinel-1",
        status: "cancelled",
        done_bytes: 0,
        total_bytes: 1200000000,
        output_dir: "F:/InSAR_Data/s1_cancelled",
        items_count: 1,
        completed_items_count: 0,
        failed_items_count: 0,
        created_at: "2026-10-24T02:26:00Z",
        updated_at: "2026-10-24T02:28:00Z",
      },
      {
        task_id: "mock-task-c1-completed-5",
        revision: 1,
        category: "scene_download",
        name: "ALOS-2 PALSAR-2 1 景批次",
        provider: "NASA Earthdata",
        dataset: "ALOS-2",
        status: "completed",
        done_bytes: 3100000000,
        total_bytes: 3100000000,
        speed_formatted: "完成",
        output_dir: "F:/InSAR_Data/alos2_202610",
        items_count: 1,
        completed_items_count: 1,
        failed_items_count: 0,
        scene_ids: ["ALOS2-PALSAR2-L11-20240315-OBS102"],
        created_at: "2026-10-24T00:10:00Z",
        updated_at: "2026-10-24T00:45:00Z",
      },
      {
        task_id: "mock-task-c1-failed-6",
        revision: 1,
        category: "scene_download",
        name: "Sentinel-1 SLC 批次",
        provider: "ASF",
        dataset: "Sentinel-1",
        status: "failed",
        done_bytes: 450000000,
        total_bytes: 2100000000,
        output_dir: "F:/InSAR_Data/s1_failed",
        error: "网络连接中断，下载端点响应超时（HTTP 504）",
        failure: {
          codes: ["DOWNLOAD_TRANSFER_TIMEOUT"],
          message_zh: "网络连接中断，下载端点响应超时（HTTP 504）。请检查网络设置或稍后重试。",
          failed_items: 1,
          total_items: 4,
        },
        frozen_search_params: {
          resource_id: "science.sentinel-1.standard.v1",
          aoi_name: "大同盆地测试区",
        },
        items_count: 4,
        completed_items_count: 1,
        failed_items_count: 1,
        scene_ids: [
          "S1A_IW_SLC__1SDV_20240410T100122_FAIL",
          "S1A_IW_SLC__1SDV_20240410T100147_FAIL",
        ],
        scene_items: [
          {
            scene_id: "S1A_IW_SLC__1SDV_20240410T100122_FAIL",
            status: "completed",
            done_bytes: 450000000,
            total_bytes: 450000000,
          },
          {
            scene_id: "S1A_IW_SLC__1SDV_20240410T100147_FAIL",
            status: "failed",
            done_bytes: 0,
            total_bytes: 550000000,
          },
          {
            scene_id: "S1A_IW_SLC__1SDV_20240410T100212_FAIL3",
            status: "queued",
            done_bytes: 0,
            total_bytes: 550000000,
          },
          {
            scene_id: "S1A_IW_SLC__1SDV_20240410T100237_FAIL4",
            status: "queued",
            done_bytes: 0,
            total_bytes: 550000000,
          },
        ],
        created_at: "2026-10-23T23:00:00Z",
        updated_at: "2026-10-23T23:20:00Z",
      },
      {
        task_id: "task-s1-failed",
        revision: 1,
        category: "scene_download",
        name: "Sentinel-1 SLC 单景下载",
        provider: "ASF",
        dataset: "Sentinel-1",
        status: "failed",
        done_bytes: 450000000,
        total_bytes: 2100000000,
        output_dir: "F:/InSAR_Data/s1_failed",
        error: "下载数据分片响应超时。上游数据服务未在预期窗口内返回有效字节流，请检查网络代理配置或稍后重试。",
        failure: {
          codes: ["DOWNLOAD_CHUNK_TIMEOUT"],
          message_zh: "下载数据分片响应超时。上游数据服务未在预期窗口内返回有效字节流，请检查网络代理配置或稍后重试。",
          failed_items: 1,
          total_items: 1,
        },
        items_count: 1,
        completed_items_count: 0,
        failed_items_count: 1,
        scene_ids: [INITIAL_S1_SCENES[4].scene_id],
        created_at: "2026-10-23T23:00:00Z",
        updated_at: "2026-10-23T23:20:00Z",
      },
      {
        task_id: "mock-task-c1-dem-failed-7",
        revision: 1,
        category: "dem_convert",
        name: "NASADEM 转换与投影重采样",
        provider: "NASA",
        dataset: "NASADEM",
        status: "failed",
        done_bytes: 0,
        total_bytes: null,
        error: "GDAL 坐标转换失败: 无效的地理仿射变换参数",
        frozen_search_params: {
          resource_id: "dem.cop30.v1",
          aoi_name: "大同盆地测试区",
        },
        items_count: 1,
        completed_items_count: 0,
        failed_items_count: 1,
        created_at: "2026-10-23T22:30:00Z",
        updated_at: "2026-10-23T22:32:00Z",
      },
      {
        task_id: "mock-task-c1-dem-completed-8",
        revision: 1,
        category: "dem_convert",
        name: "SRTM-1 DEM 转换批次",
        provider: "USGS",
        dataset: "SRTM",
        status: "completed",
        done_bytes: 85000000,
        total_bytes: 85000000,
        speed_formatted: "完成",
        output_dir: "F:/InSAR_Data/dem_srtm",
        items_count: 2,
        completed_items_count: 2,
        failed_items_count: 0,
        created_at: "2026-10-23T21:00:00Z",
        updated_at: "2026-10-23T21:15:00Z",
      },
    ];
      for (const t of defaultTasks) {
        this.tasks.set(t.task_id, t);
      }
    }
  }

  private createBatch(batchId: string, revision: number, scenes: SceneRow[]): void {
    const rowMap = new Map<string, RowTaskState>();
    for (const sc of scenes) {
      rowMap.set(sc.scene_id, {
        item_id: sc.scene_id,
        task_id: null,
        task_status: null,
        item_status: null,
        actions: ["download"], // 初始权威行级可用操作：仅允许下载
      });
    }
    this.batches.set(batchId, { revision, scenes, rows: rowMap });
  }

  public setNextSearchOutcome(outcome: "done" | "failed"): void {
    this.nextSearchOutcome = outcome;
  }

  public listSources(): SourceDescriptor[] {
    return Array.from(this.sources.values());
  }

  public getSourceViews(): SourceView[] {
    return Array.from(this.sourceViews.values());
  }

  public getSourceView(resourceId: string): SourceView | undefined {
    return this.sourceViews.get(resourceId);
  }

  public searchStart(params: SearchStartParams): SearchProgress {
    const reqId = `req-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
    const outcome = (params as any).simulate_outcome || this.nextSearchOutcome;

    const progress: SearchProgress = {
      request_id: reqId,
      resource_id: params.resource_id,
      stage: "requesting",
      requests_done: 1,
      items_kept: 0,
      items_cap: 250,
      elapsed_ms: 50,
      message_zh: "正在向数据源检索中心请求元数据...",
      terminal: false,
      error: null,
    };

    // 记录进行中的请求，并更新对应源的 pending_request_id
    this.activeRequests.set(reqId, { progress, targetOutcome: outcome, pollCount: 0 });
    const view = this.sourceViews.get(params.resource_id);
    if (view) {
      view.pending_request_id = reqId;
      view.last_error = null;
    }

    return { ...progress };
  }

  public getSearchProgress(requestId: string): SearchProgress {
    const entry = this.activeRequests.get(requestId);
    if (!entry) {
      return {
        request_id: requestId,
        resource_id: "",
        stage: "failed",
        requests_done: 0,
        items_kept: 0,
        items_cap: 0,
        elapsed_ms: 0,
        message_zh: "未找到活跃检索请求",
        terminal: true,
        error: { code: "NOT_FOUND", message_zh: "未找到活跃检索请求", retryable: false, needs_account: null, needs_user_confirmation: false },
      };
    }

    entry.pollCount += 1;
    entry.progress.elapsed_ms += 250;

    // 状态阶段演进：requesting -> filtering -> freezing -> (done / failed)
    if (entry.pollCount === 1) {
      entry.progress.stage = "filtering";
      entry.progress.requests_done = 2;
      entry.progress.items_kept = 4;
      entry.progress.message_zh = "正在根据空间范围与干涉几何约束过滤场景...";
    } else if (entry.pollCount >= 2) {
      if (entry.targetOutcome === "done") {
        entry.progress.stage = "done";
        entry.progress.terminal = true;
        entry.progress.items_kept = 6;
        entry.progress.message_zh = "检索完成，已锁定最新冻结批次";

        // 成功切换视图：创建新批次并更新对应源的 SourceView
        const resourceId = entry.progress.resource_id;
        const newBatchId = `batch-${resourceId}-${Date.now()}`;
        const newScenes: SceneRow[] = [
          ...INITIAL_S1_SCENES,
          {
            scene_id: `S1A_IW_SLC__NEW_${Date.now()}`,
            platform: "Sentinel-1A",
            beam_mode: "IW",
            product_type: "SLC",
            polarization: "VV+VH",
            acquisition_datetime: new Date().toISOString(),
            absolute_orbit: 54000,
            relative_orbit: 69,
            path: 69,
            frame: null,
            orbit_direction: "ascending",
            footprint_bbox: { west: 121.2, south: 23.5, east: 121.9, north: 24.5 },
            footprint_geojson: null,
            has_url: false,
          },
        ];

        this.createBatch(newBatchId, 1, newScenes);
        const view = this.sourceViews.get(resourceId);
        if (view) {
          view.batch_id = newBatchId;
          view.revision = (view.revision ?? 0) + 1;
          view.item_count = newScenes.length;
          view.searched_at = new Date().toISOString();
          view.pending_request_id = null;
          view.last_error = null;
        }
      } else {
        // 失败保留旧批次：不更新 batch_id，仅记录 last_error
        entry.progress.stage = "failed";
        entry.progress.terminal = true;
        const err: TypedError = {
          code: "UPSTREAM_TIMEOUT",
          message_zh: "数据源服务器连接超时，已保留上一批结果供继续浏览",
          retryable: true,
          needs_account: null,
          needs_user_confirmation: false,
        };
        entry.progress.error = err;
        entry.progress.message_zh = err.message_zh;

        const view = this.sourceViews.get(entry.progress.resource_id);
        if (view) {
          view.pending_request_id = null;
          view.last_error = err;
          // 注意：view.batch_id 保持不变！
        }
      }
    }

    return { ...entry.progress };
  }

  public cancelSearch(requestId: string): void {
    const entry = this.activeRequests.get(requestId);
    if (entry) {
      entry.progress.stage = "cancelled";
      entry.progress.terminal = true;
      entry.progress.message_zh = "用户已取消检索";
      const view = this.sourceViews.get(entry.progress.resource_id);
      if (view && view.pending_request_id === requestId) {
        view.pending_request_id = null;
      }
    }
  }

  public getBatchPage(batchId: string, revision: number, page = 0, pageSize = RESULT_PAGE_SIZE): BatchPageResult {
    const b = this.batches.get(batchId);
    if (!b) {
      return {
        batch_id: batchId,
        revision,
        page,
        page_size: pageSize,
        total_count: 0,
        scenes: [],
        rows: [],
      };
    }

    const start = page * pageSize;
    const end = start + pageSize;
    const sliceScenes = b.scenes.slice(start, end);
    const sliceRows = sliceScenes.map((sc) => b.rows.get(sc.scene_id) || {
      item_id: sc.scene_id,
      task_id: null,
      task_status: null,
      item_status: null,
      actions: ["download" as RowAction],
    });

    return {
      batch_id: batchId,
      revision: b.revision,
      page,
      page_size: pageSize,
      total_count: b.scenes.length,
      scenes: sliceScenes,
      rows: sliceRows,
    };
  }

  public createTask(batchId: string, _revision: number, itemIds: string[], options?: CreateTaskOptions): TaskRef {
    // 模拟缺账号报错：如果明确要求模拟凭据缺失，抛出 TypedError
    if ((options as any)?.simulate_auth_error) {
      const authErr: TypedError = {
        code: "AUTH_REQUIRED",
        message_zh: "需要登录 NASA Earthdata 账户以继续下载，登录成功后自动继续",
        retryable: false,
        needs_account: "earthdata",
        needs_user_confirmation: false,
      };
      throw authErr;
    }

    // 审计 P0 防线：纯内存模拟 TaskRef，绝对不调用后端原生下载链路
    const taskId = `mock-task-c1-${Date.now()}`;
    const b = this.batches.get(batchId);

    if (b) {
      for (const id of itemIds) {
        const row = b.rows.get(id);
        if (row) {
          row.task_id = taskId;
          row.task_status = "running";
          // 行内可用操作受控于后端状态：排队/下载中可暂停，可打开任务
          row.actions = ["pause", "open_task"];
        }
      }
    }

    const newTask: ApplicationTask = {
      task_id: taskId,
      revision: 0,
      category: "scene_download",
      name: `${itemIds.length} 景数据下载任务`,
      provider: "NASA Earthdata",
      dataset: "Sentinel-1",
      status: "queued",
      done_bytes: 0,
      total_bytes: itemIds.length * 800000000,
      speed_formatted: "等待分配调度...",
      output_dir: options?.output_root || (options as any)?.output_dir || "F:/InSAR_Data/downloads",
      items_count: itemIds.length,
      completed_items_count: 0,
      failed_items_count: 0,
      scene_ids: [...itemIds],
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
    };
    this.tasks.set(taskId, newTask);

    // 立即返回 TaskRef，状态为 queued/running，完全不阻塞界面
    return {
      task_id: taskId,
      revision: 0,
      status: "queued",
      created: true,
    };
  }

  public taskAction(taskId: string, _revision: number, action: RowAction | "pause" | "resume" | "retry" | "cancel" | "archive"): void {
    const task = this.tasks.get(taskId);
    if (task) {
      if (action === "pause") {
        task.status = "paused";
      } else if (action === "resume" || action === "retry") {
        task.status = "running";
        task.error = null;
      } else if (action === "cancel") {
        task.status = "cancelled";
      } else if (action === "archive") {
        this.tasks.delete(taskId);
      }
      task.updated_at = new Date().toISOString();
    }

    for (const b of this.batches.values()) {
      for (const row of b.rows.values()) {
        if (row.task_id === taskId) {
          if (action === "pause") {
            row.task_status = "paused";
            row.actions = ["resume", "open_task"];
          } else if (action === "resume" || action === "retry") {
            row.task_status = "running";
            row.actions = ["pause", "open_task"];
          }
        }
      }
    }
  }

  public getTasks(): ApplicationTask[] {
    return Array.from(this.tasks.values(), (t) => ({ ...t }));
  }

  // __fixture: 外部任务变更入口（供交互测试在两次轮询之间清空/创建/更新任务状态与修订号）
  public clearTasks__fixture(): void {
    this.tasks.clear();
  }

  public patchTask__fixture(taskId: string, patch: Partial<ApplicationTask>): ApplicationTask | null {
    const existing = this.tasks.get(taskId);
    if (!existing) return null;
    const updated: ApplicationTask = {
      ...existing,
      ...patch,
      task_id: existing.task_id,
    };
    this.tasks.set(taskId, updated);
    return { ...updated };
  }

  /**
   * __fixture 重新检索后重试两步演示流
   * 严格遵守 C1 契约规范与 TypedError 形状，演示数据绝不触达真实命令
   */
  public retryTaskFallback(
    taskId: string,
    confirmRefreshedBatchId: string | null = null
  ): TaskRetryFallbackOutcome {
    const task = this.tasks.get(taskId);
    if (!task) {
      const err: any = new Error(`任务不存在: ${taskId}`);
      err.code = "TASK_NOT_FOUND";
      err.message_zh = `任务不存在: ${taskId}`;
      throw err;
    }

    // 只有 failed 的任务可以重检兜底
    if (task.status !== "failed") {
      const err: any = new Error("只有失败的任务可以重检兜底");
      err.code = "RETRY_FALLBACK_TASK_NOT_FAILED";
      err.message_zh = "只有失败的任务可以重检兜底";
      throw err;
    }

    // 目前只支持 ASF 目录 SAR 产品任务，其他返回 RETRY_FALLBACK_UNSUPPORTED
    if (task.category === "dem_convert" || task.provider !== "ASF") {
      const err: any = new Error("该任务类型不支持重新检索重试");
      err.code = "RETRY_FALLBACK_UNSUPPORTED";
      err.message_zh = "该任务类型不支持重新检索重试（仅支持 ASF SAR 任务）";
      throw err;
    }

    const expectedBatchId = `fixture-batch-${task.task_id}`;

    // 第一次调用：confirmRefreshedBatchId 为 null，返回差异确认
    if (confirmRefreshedBatchId === null) {
      return {
        task: {
          task_id: task.task_id,
          revision: task.revision,
          status: task.status,
          created: false,
        },
        refreshed_batch_id: expectedBatchId,
        rebuilt_item_ids: task.scene_ids && task.scene_ids.length > 0 ? [task.scene_ids[0]] : ["item-fixture-rebuilt-1"],
        missing_product_ids: ["S1A_IW_SLC__1SDV_20240410T100122_MISSING_DEMO"],
        added_product_ids: ["S1A_IW_SLC__1SDV_20240410T100147_NEW_DEMO"],
        needs_user_confirmation: true,
      };
    }

    // 第二次调用：回传 refreshed_batch_id 进行两步确认
    if (confirmRefreshedBatchId !== expectedBatchId) {
      const err: any = new Error("刷新批次确认标识不匹配或已过期");
      err.code = "RETRY_FALLBACK_CONFIRMATION_MISMATCH";
      err.message_zh = "刷新批次确认标识不匹配或已过期";
      throw err;
    }

    // 确认成功：更新任务状态为 running，清空 error，递增 revision
    task.status = "running";
    task.error = null;
    task.revision = task.revision + 1;
    task.updated_at = new Date().toISOString();

    return {
      task: {
        task_id: task.task_id,
        revision: task.revision,
        status: task.status,
        created: false,
      },
      refreshed_batch_id: confirmRefreshedBatchId,
      rebuilt_item_ids: task.scene_ids && task.scene_ids.length > 0 ? [task.scene_ids[0]] : ["item-fixture-rebuilt-1"],
      missing_product_ids: [],
      added_product_ids: [],
      needs_user_confirmation: false,
    };
  }

  public clearTaskHistory(): void {
    for (const [id, task] of this.tasks.entries()) {
      if (isTaskHistoryRow(task)) {
        this.tasks.delete(id);
      }
    }
  }
}

let _db: FixtureDatabase | null = null;
export function getDb(): FixtureDatabase {
  if (!_db) {
    _db = new FixtureDatabase();
  } else {
    const cur = currentManualWorkbenchSubscene();
    if (_db.getCurrentSubscene() !== cur) {
      _db.reset();
    }
  }
  return _db;
}

// 导出与真实 bridge 一致的对外契约方法
export async function listSources(): Promise<SourceDescriptor[]> {
  return getDb().listSources();
}

export async function getSourceDescriptors(): Promise<SourceDescriptor[]> {
  return getDb().listSources();
}

export async function getSourceViews(): Promise<SourceView[]> {
  return getDb().getSourceViews();
}

export async function getSourceView(resourceId: string): Promise<SourceView | undefined> {
  return getDb().getSourceView(resourceId);
}

export async function searchStart(
  params: SearchStartParams,
  _options?: { bypassCache?: boolean },
): Promise<SearchProgress> {
  return getDb().searchStart(params);
}

export async function getSearchProgress(requestId: string): Promise<SearchProgress> {
  return getDb().getSearchProgress(requestId);
}

export async function cancelSearch(requestId: string): Promise<void> {
  return getDb().cancelSearch(requestId);
}

export async function cancelCatalogSearch(requestId: string): Promise<SearchCancelOutcome | null> {
  await getDb().cancelSearch(requestId);
  return null;
}

export async function getBatchPage(
  batchId: string,
  revision: number,
  page = 0,
  pageSize = RESULT_PAGE_SIZE
): Promise<BatchPageResult> {
  return getDb().getBatchPage(batchId, revision, page, pageSize);
}

export async function createTask(
  batchId: string,
  revision: number,
  itemIds: string[],
  options?: CreateTaskOptions
): Promise<TaskRef> {
  return getDb().createTask(batchId, revision, itemIds, options);
}

export async function previewAllMatchTask(
  request: import("./types.ts").BatchAllMatchPreviewRequest
): Promise<import("./types.ts").BatchAllMatchPreview> {
  const page = await getDb().getBatchPage(request.batch_id, request.expected_revision, 0, 250);
  const excludedSet = new Set(request.excluded_item_ids ?? []);
  const selectable = page.scenes.filter((s) => !excludedSet.has(s.scene_id));
  const knownBytes = selectable.reduce((sum, s) => sum + (s.file_size_remote ?? 0), 0);
  return {
    batch_id: request.batch_id,
    revision: page.revision,
    batch_total_count: page.total_count,
    matched_count: page.total_count,
    excluded_count: excludedSet.size,
    skipped_active_count: 0,
    rejected_count: 0,
    admissible_count: Math.max(0, page.total_count - excludedSet.size),
    known_bytes: knownBytes,
    unknown_size_count: 0,
    selection_digest: `fixture-digest:${request.batch_id}:${page.revision}:${excludedSet.size}`,
  };
}

export async function createAllMatchTask(
  request: import("./types.ts").BatchAllMatchCreateTaskRequest
): Promise<import("./types.ts").BatchAllMatchTaskOutcome> {
  const page = await getDb().getBatchPage(request.batch_id, request.expected_revision, 0, 250);
  const excludedSet = new Set(request.excluded_item_ids ?? []);
  const itemIds = page.scenes.map((s) => s.scene_id).filter((id) => !excludedSet.has(id));
  const ref = await getDb().createTask(request.batch_id, request.expected_revision, itemIds, {
    output_root: request.output_root,
    concurrency: request.concurrency,
    layout: request.layout,
  });
  return {
    ...ref,
    matched_count: page.total_count,
    created_count: itemIds.length,
    pending_count: 0,
    skipped_count: 0,
    rejected_count: 0,
  };
}

export async function taskAction(
  taskId: string,
  revision: number,
  action: RowAction | "pause" | "resume" | "retry" | "cancel" | "archive"
): Promise<void> {
  return getDb().taskAction(taskId, revision, action);
}

export async function getTasks(): Promise<ApplicationTask[]> {
  return getDb().getTasks();
}

export async function retryTaskFallback(
  taskId: string,
  confirmRefreshedBatchId: string | null = null
): Promise<TaskRetryFallbackOutcome> {
  return getDb().retryTaskFallback(taskId, confirmRefreshedBatchId);
}

export async function clearTaskHistory(): Promise<void> {
  return getDb().clearTaskHistory();
}

export const isAccountStatusesWired = (): boolean => true;

export async function getAccountStatuses(): Promise<AccountStatus[]> {
  return [
    {
      account_id: "cdse",
      state: "credentials_missing",
      checked_at: new Date().toISOString(),
      message_zh: "未配置 CDSE 账户凭据",
    },
    {
      account_id: "earthdata",
      state: "logged_in",
      checked_at: new Date().toISOString(),
      message_zh: "NASA Earthdata 已登录",
    },
    {
      account_id: "usgs_m2m",
      state: "credentials_missing",
      checked_at: new Date().toISOString(),
      message_zh: "未配置 USGS 凭据",
    },
    {
      account_id: "bhoonidhi",
      state: "login_required",
      checked_at: new Date().toISOString(),
      message_zh: "需要登录 ISRO Bhoonidhi 账户",
    },
    {
      account_id: "mpc",
      state: "not_required",
      checked_at: new Date().toISOString(),
      message_zh: "匿名签名 / 可用",
    },
    {
      account_id: "opentopography",
      state: "not_required",
      checked_at: new Date().toISOString(),
      message_zh: "公开服务 / 无需认证",
    },
  ];
}

export function __resetFixtureState(): void {
  getDb().reset();
  _fixtureChannelsConfigured = true;
}

// __fixture: 演示存储根（确认卡「使用默认目录」用；真实桌面经 getSarDownloadPolicySettings 读取）
export function getFixtureStorageRoot(): string {
  return "D:\\EOStage\\Downloads";
}

// __fixture: 演示工作区 AOI（茂县叠溪）
export function getFixtureWorkingAoi() {
  return {
    state: "working" as const,
    source_kind: "manual_polygon",
    display_name: "茂县叠溪",
    label: "茂县叠溪",
    bbox: { west: 103.5, south: 31.8, east: 103.75, north: 32.0 },
    geometry: {
      type: "Polygon" as const,
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
    crs: "EPSG:4326",
  };
}

// __fixture: 手动工作台演示子场景类型与解析器
export type ManualWorkbenchSubscene = "empty" | "results-8" | "results-tasks";

export function parseManualWorkbenchSubscene(
  hashOrQuery: string | null | undefined
): ManualWorkbenchSubscene | null {
  if (!hashOrQuery) return null;
  const match = hashOrQuery.match(/[?&]subscene=([^&#]+)/);
  const val = match ? match[1] : null;
  if (val === "empty" || val === "results-8" || val === "results-tasks") {
    return val;
  }
  return null;
}

let _testSubsceneOverride: ManualWorkbenchSubscene | null = null;
export function __setManualWorkbenchSubsceneForTest(subscene: ManualWorkbenchSubscene | null): void {
  _testSubsceneOverride = subscene;
}

export function currentManualWorkbenchSubscene(): ManualWorkbenchSubscene | null {
  if (_testSubsceneOverride !== null) return _testSubsceneOverride;
  if (typeof window === "undefined") return null;
  return parseManualWorkbenchSubscene(window.location.hash);
}

export function __setSearchOutcome(outcome: "done" | "failed"): void {
  getDb().setNextSearchOutcome(outcome);
}

let _fixtureChannelsConfigured = true;

export function __setFixtureChannelsConfigured(configured: boolean): void {
  _fixtureChannelsConfigured = configured;
}

export const isAgentWired = (): boolean => true;

export async function getModelChannelReadiness() {
  if (!_fixtureChannelsConfigured) {
    return {
      ready: false,
      disabled: true,
      reason_zh: "未配置可用模型渠道，请前往「设置 → 模型渠道」配置",
    };
  }
  return {
    ready: true,
    disabled: false,
    reason_zh: "",
  };
}

export function createFixtureAgentSessions(): AgentSession[] {
  const planHash1 = "7b2c9e4a1f8d30e5";
  const defaultAoi: Bbox = { west: 112.8, east: 113.6, south: 39.8, north: 40.4 };

  return [
    {
      id: "sess-001",
      title: "大同盆地 2024-04 Sentinel-1 检索与任务",
      created_at: "2026-10-24 08:30",
      updated_at: "2026-10-24 08:45",
      messages: [
        {
          id: "m-01",
          sender: "user",
          text: "帮我检索大同盆地 2024 年 4 月的 Sentinel-1 升轨数据并建立下载任务。",
          timestamp: "08:30",
        },
        {
          id: "m-02",
          sender: "agent",
          text: "收到请求。我将依次解析区域边界、调用官方端点检索 Sentinel-1 SLC 批次并编制下载计划：",
          timestamp: "08:31",
          executionRounds: [
            {
              round_id: "rnd-1",
              tool_calls: [
                {
                  tool_id: "resolve_aoi",
                  tool_name_zh: "解析空间范围",
                  params_summary: "地名: 大同盆地",
                  result_summary: "已解析为边界矩形 [112.80, 39.80, 113.60, 40.40]",
                  status: "success",
                },
                {
                  tool_id: "application_sar_search_and_freeze",
                  tool_name_zh: "检索并冻结 Sentinel-1 批次",
                  params_summary: "源: Sentinel-1 ASF, 时间: 2024-04-01 ~ 2024-04-30, 轨道: 升轨",
                  result_summary: "已检出 2 景有效 SLC 影像，批次号 batch-202404-dt",
                  status: "success",
                },
              ],
            },
          ],
          plan: {
            plan_id: "plan-dt-202404",
            session_id: "sess-001",
            run_id: "run-001",
            plan_hash: planHash1,
            max_tier: 2,
            status: "proposed",
            store_revision_seen: 42,
            created_at: "2026-10-24 08:31",
            steps: [
              {
                step_id: "s-1",
                tool_id: "resolve_aoi",
                tool_name_zh: "解析空间范围",
                tier: 0,
                description_zh: "确认大同盆地地理经纬度范围",
                status: "completed",
              },
              {
                step_id: "s-2",
                tool_id: "application_sar_search_and_freeze",
                tool_name_zh: "检索并冻结 Sentinel-1 批次",
                tier: 1,
                description_zh: "检索 2024 年 4 月升轨 SLC 影像并生成只读快照",
                status: "completed",
              },
              {
                step_id: "s-3",
                tool_id: "application_create_download_task",
                tool_name_zh: "创建下载任务",
                tier: 2,
                description_zh: "在下载队列中创建数据获取任务",
                status: "pending",
              },
              {
                step_id: "s-4",
                tool_id: "application_download_task_page",
                tool_name_zh: "读取任务状态",
                tier: 0,
                description_zh: "持续跟踪任务队列状态",
                status: "pending",
              },
            ],
          },
          confirmCard: {
            source: "Sentinel-1 GRD/SLC (ASF)",
            scope: {
              aoi_name: "大同盆地",
              area_sqkm: 1450.0,
              time_range: "2024-04-01 ~ 2024-04-30",
              product_or_orbit: "SLC · 升轨 (IW)",
              bbox: defaultAoi,
            },
            estimated_size: "约 4.6 GB (估算)",
            save_location: "D:\\EOStageData\\Downloads\\Sentinel-1",
            plan_id: "plan-dt-202404",
            plan_hash: planHash1,
            store_revision_seen: 42,
          },
          progressCard: {
            task_id: "task-dt-01",
            task_title: "数据下载任务 (#task-dt-01)",
            status: "running",
            status_zh: "下载中",
            progress_label: "2.3 GB / 4.6 GB (估算)",
            percent: 50,
            speed_label: "瞬时速率: 18.5 MB/s",
          },
        },
      ],
    },
    {
      id: "sess-002",
      title: "历史会话：太原周边 Sentinel-1 数据归档",
      created_at: "2026-10-23 15:10",
      updated_at: "2026-10-23 15:20",
      messages: [
        {
          id: "m-03",
          sender: "user",
          text: "查询太原市 2024 年第一季度可用影像。",
          timestamp: "15:10",
        },
        {
          id: "m-04",
          sender: "agent",
          text: "已完成目录检索，检索批次已保存。",
          timestamp: "15:12",
        },
      ],
    },
  ];
}

export async function getAgentSessions(): Promise<AgentSession[]> {
  return createFixtureAgentSessions();
}

export function getAgentSessionsSync(): AgentSession[] {
  return createFixtureAgentSessions();
}

export function getAgentSubsceneApprovalSync(
  subscene: string | null
): AgentApprovalRecord | null {
  if (subscene === "confirm-approved" || subscene === "confirm-expired") {
    const dummy: AgentConfirmCardData = {
      source: "Sentinel-1 GRD/SLC (ASF)",
      scope: {
        aoi_name: "大同盆地",
        area_sqkm: 1450.0,
        time_range: "2024-04-01 ~ 2024-04-30",
        product_or_orbit: "SLC · 升轨 (IW)",
        bbox: { west: 112.8, east: 113.6, south: 39.8, north: 40.4 },
      },
      estimated_size: "约 4.6 GB (估算)",
      save_location: "D:\\EOStageData\\Downloads\\Sentinel-1",
      plan_id: "plan-dt-202404",
      plan_hash: "7b2c9e4a1f8d30e5",
      store_revision_seen: subscene === "confirm-expired" ? 40 : 42,
    };
    return freezeApprovalSnapshot(dummy, "2026-10-24 08:35:10");
  }
  return null;
}

export async function getAgentSubsceneApproval(
  subscene: string | null
): Promise<AgentApprovalRecord | null> {
  return getAgentSubsceneApprovalSync(subscene);
}

export function getAgentConclusionsSync(): {
  claims: AgentClaim[];
  knowledge: {
    taskIds: string[];
    batchIds: string[];
    verifiedWebDomains: string[];
  };
} {
  return {
    claims: [
      {
        statement: "已成功冻结大同盆地 2024-04 的 2 景 Sentinel-1 SLC 批次，并创建下载队列任务。",
        refs: [
          { type: "batch", id: "batch-202404-dt", title: "检索批次", verified: false },
          { type: "task", id: "task-dt-01", title: "下载任务 #task-dt-01", verified: false },
        ],
      },
      {
        statement: "经外部网络检索，ASF 预计于本月底维护，请关注任务投递状态。",
        refs: [
          {
            type: "web",
            id: "copernicus-news-hub.info/maintenance",
            title: "第三方运维镜像通知",
            verified: false,
          },
        ],
      },
    ],
    knowledge: {
      taskIds: ["task-dt-01"],
      batchIds: ["batch-202404-dt"],
      verifiedWebDomains: ["copernicus.eu"],
    },
  };
}

export async function getAgentConclusions() {
  return getAgentConclusionsSync();
}

export function getFixtureNewSessionInitialMessageSync(): AgentMessage {
  return {
    id: `m-${Date.now()}`,
    sender: "agent",
    text: "你好！我是 EOStage 智能助手。我可以帮助你解析空间范围、检索 Sentinel-1 批次并规划下载任务。请输入你的需求。",
    timestamp: "刚刚",
  };
}

export async function getFixtureNewSessionInitialMessage(): Promise<AgentMessage> {
  return getFixtureNewSessionInitialMessageSync();
}

export function getFixtureAssistantGreetingSync(): {
  boundTitle: string;
  defaultAoi: string;
  defaultResource: string;
  greetingText: string;
} {
  return {
    boundTitle: "工作区上下文已绑定",
    defaultAoi: "全球视图",
    defaultResource: "Sentinel-1 (ASF)",
    greetingText:
      "你好！我是 EOStage 智能助手。你可以在此快速输入检索或数据准备需求，或点击右上角「放大」进入全功能 Agent 工作台。",
  };
}

export async function getFixtureAssistantGreeting() {
  return getFixtureAssistantGreetingSync();
}

export function getAgentPlanResourceSummarySync(
  _sessionId?: string
): AgentPlanResourceSummary | null {
  return {
    batch_id: "batch-202404-dt",
    source_title: "Sentinel-1 (ASF)",
    scene_count_label: "2 景 (SLC · IW)",
    task_id: "#task-dt-01",
    task_status_zh: "下载中 (50%)",
    output_dir: "D:\\EOStageData\\Downloads\\Sentinel-1",
  };
}

export async function getAgentPlanResourceSummary(
  sessionId?: string
): Promise<AgentPlanResourceSummary | null> {
  return getAgentPlanResourceSummarySync(sessionId);
}

export async function openOutputDirectory(_ref: {
  task_id: string;
  revision: number;
}): Promise<void> {
  // 演示模式下绝不触达本机文件系统
}
