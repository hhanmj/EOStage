import type {
  ContractError as BridgeContractError,
  TaskEvent as BridgeTaskEvent,
} from "../bridge";
import type {
  ContractError,
  CatalogCollectionDescriptor,
  CatalogSearchQuery,
  CatalogSearchResponse,
  DataSourceKind,
  FrozenSearchStats,
  OrbitFileMetadata,
  OrbitMatchReport,
  ProviderMetadata,
  ProviderPlan,
  ProviderProduct,
  SidecarRequest,
  SidecarResponse,
  SidecarServiceInfo,
  StableAssetReference,
  TaskEvent,
} from "./v3-contracts";

const landsatSourceKind: DataSourceKind = "landsat";
void landsatSourceKind;

const frozenSearchStats = {
  matched_count: 4,
  fetched_record_count: 4,
  mapped_record_count: 4,
  provider_returned_count: 4,
  frozen_candidate_count: 4,
  truncated: false,
  source: "ASF_BATCH",
} satisfies FrozenSearchStats;
void frozenSearchStats;

const hostileFrozenSearchStats = {
  ...frozenSearchStats,
  // @ts-expect-error Frozen search statistics retain only canonical route labels.
  source: "https://example.invalid/search",
} satisfies FrozenSearchStats;
void hostileFrozenSearchStats;

const metadata = {
  provider_id: "fixture.provider",
  display_name: "Fixture Provider",
  source_kind: "sentinel1",
  version: "0.1.0",
  capabilities: ["search", "plan"],
} satisfies ProviderMetadata;

const product = {
  provider_id: metadata.provider_id,
  product_id: "FIXTURE-SCENE-001",
  display_name: "Fixture Scene 001",
  source_kind: "sentinel1",
  product_kind: "sar_slc",
  acquisition_datetime: null,
  footprint_geojson: null,
  footprint_bbox: null,
  assets: [],
  properties: {},
  payload: {},
} satisfies ProviderProduct;

const plan = {
  provider_id: metadata.provider_id,
  source_kind: "sentinel1",
  output_root: "E:/InSAR/测试/输出",
  items: [
    {
      product_id: product.product_id,
      target_path: "E:/InSAR/测试/输出/FIXTURE-SCENE-001.zip",
      role: "primary",
      expected_size_bytes: null,
      status: "planned",
      properties: {},
    },
  ],
  requires_credentials: false,
  manual_action_required: false,
  execution_mode: "direct_download",
  submission: {},
  legacy_plan: {},
  legacy_report: {},
} satisfies ProviderPlan;

const error = {
  code: "V3_FIXTURE_ERROR",
  user_message: "固定测试错误",
  debug_context: { provider_id: metadata.provider_id },
} satisfies ContractError;

const event = {
  event_id: "event-fixture-001",
  task_id: "task-fixture-001",
  item_id: null,
  sequence: 0,
  occurred_at: "2024-01-01T00:00:00Z",
  event_type: "state_changed",
  payload: { state: "queued" },
  error,
} satisfies TaskEvent;

const sidecarRequest = {
  protocol_major: 1,
  protocol_minor: 2,
  request_id: "请求-001",
  operation: "health",
  payload: { probe: "startup" },
} satisfies SidecarRequest;

const sidecarResponse = {
  protocol_major: 1,
  protocol_minor: 2,
  request_id: sidecarRequest.request_id,
  ok: true,
  result: { status: "healthy" },
  error: null,
  terminal: true,
} satisfies SidecarResponse;

const sidecarServiceInfo = {
  service_version: "0.1.0",
  contract_version: "3.1.0",
  protocol_major: 1,
  protocol_minor: 2,
  minimum_protocol_minor: 1,
  build_id: "development",
  capabilities: ["orbit_parse", "orbit_match"],
} satisfies SidecarServiceInfo;

const orbitFile = {
  file_name: "S1A_OPER_AUX_POEORB_OPOD_20240102T120000_V20240101T225942_20240103T005942.EOF",
  platform: "S1A",
  orbit_type: "POEORB",
  creation_unix_seconds: 1704196800,
  validity_start_unix_seconds: 1704150000,
  validity_stop_unix_seconds: 1704243600,
} satisfies OrbitFileMetadata;

const orbitReport = {
  total: 1,
  matched: 1,
  unmatched: 0,
  results: [],
  issues: [],
  summary: {
    orbit_files: 1,
    scenes: 1,
    matched: 1,
    unmatched: 0,
    orbit_types: [orbitFile.orbit_type],
  },
} satisfies OrbitMatchReport;

const stableAssetReference = {
  provider_id: "asf.catalog",
  collection_id: "ALOS PALSAR",
  provider_product_id: "ALOS-ITEM-001",
  provider_product_key: "PROVIDER::asf.catalog::COLLECTION::ALOS PALSAR::ITEM::ALOS-ITEM-001",
  canonical_product_key: null,
  asset_key: "ALOS-FILE-001",
  stable_asset_key: "PROVIDER::asf.catalog::COLLECTION::ALOS PALSAR::ITEM::ALOS-ITEM-001::ASSET::ALOS-FILE-001",
} satisfies StableAssetReference;

const catalogCollection = {
  logical_dataset: "optical.sentinel-2.l2a",
  provider_id: "copernicus.dataspace.odata",
  native_collection_id: "SENTINEL-2",
  collection_profile_id: "cdse.sentinel-2.l2a.collection.v1",
  identity_rule_id: "cdse.sentinel-2.l2a.identity.v1",
  filename_rule_id: "cdse.sentinel-2.l2a.archive-filename.v1",
  delivery_model: "archive_first",
  production_enabled: false,
} satisfies CatalogCollectionDescriptor;

const catalogQuery = {
  logical_dataset: catalogCollection.logical_dataset,
  provider_id: catalogCollection.provider_id,
  native_collection_id: catalogCollection.native_collection_id,
  aoi_geojson: {
    type: "Polygon",
    coordinates: [[[110, 30], [111, 30], [111, 31], [110, 30]]],
  },
  cursor: "fixture-page-2",
  result_limit: null,
  filters: { product_type: "S2MSI2A" },
} satisfies CatalogSearchQuery;

const catalogResponse = {
  collection: catalogCollection,
  products: [],
  stats: {
    requested_limit: 2,
    matched_count: 3,
    request_count: 1,
    transport_part_count: 1,
    fetched_record_count: 2,
    mapped_record_count: 2,
    profile_filtered_record_count: 0,
    exact_duplicate_record_count: 0,
    unique_provider_product_count: 2,
    exact_matched_count: 2,
    returned_count: 2,
    remote_exhausted: false,
    working_set_satisfied: true,
    elapsed_ms: 10,
    truncated: true,
  },
  next_cursor: catalogQuery.cursor,
  item_errors: [],
} satisfies CatalogSearchResponse;

const bridgeError: BridgeContractError = error;
const bridgeEvent: BridgeTaskEvent = event;

void plan;
void bridgeError;
void bridgeEvent;
void sidecarResponse;
void sidecarServiceInfo;
void orbitReport;
void stableAssetReference;
void catalogResponse;
