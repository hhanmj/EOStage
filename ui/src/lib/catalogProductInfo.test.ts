import assert from "node:assert/strict";
import test from "node:test";

import type { CatalogResultRow } from "./catalogPreview.ts";
import {
  buildSceneFullCopyText,
  buildSceneRowMetaDisplay,
  buildSceneSummaryRows,
  catalogProductInfoText,
  formatSceneRowDisplayTitle,
} from "./catalogProductInfo.ts";
import type { SceneRow } from "./bridge.ts";
import { localizedSourceDisplayName } from "./dataSourceCatalogLogic.ts";
import { setCurrentLocale } from "../i18n/index.ts";
import type { SarProduct } from "./generated/v3-contracts.ts";

const product: SarProduct = {
  product_key: "ASF::SENTINEL-1::P1",
  provider_product_key: "ASF::SENTINEL-1::P1",
  provider_product_id: "P1",
  canonical_product_key: null,
  identity_rule_id: "asf.sentinel-1.standard.v1",
  product_id: "P1",
  scene_id: "S1A_IW_GRDH_P1",
  provider: "ASF",
  dataset: "SENTINEL-1",
  platform: "S1A",
  instrument: "C-SAR",
  processing_level: "GRD_HD",
  product_family: "GRD",
  beam_mode: "IW",
  beam_swath: null,
  polarization: "VV+VH",
  flight_direction: "ASCENDING",
  look_direction: "RIGHT",
  relative_orbit: 142,
  absolute_orbit: 53000,
  frame: 90,
  asf_frame: null,
  path: 142,
  acquisition_start: "2026-01-02T03:04:05Z",
  acquisition_stop: null,
  footprint_geojson: null,
  footprint_bbox: { west: 110, east: 111, south: 30, north: 31, crs: "EPSG:4326" },
  download_url: "https://datapool.asf.alaska.edu/GRD/P1.zip",
  file_name: "P1.zip",
  file_size_remote: 2048,
  checksum: null,
  local_path: null,
  native_metadata: {
    properties: {
      provider_field: "preserved",
      centerLat: 30.5,
      centerLon: 110.5,
    },
  },
  burst: null,
  assets: [
    {
      asset_key: "P1.zip",
      stable_asset_key: "ASF::SENTINEL-1::P1::ASSET::P1.zip",
      access_descriptor: null,
      role: "primary",
      url: "https://datapool.asf.alaska.edu/GRD/P1.zip",
      file_name: "P1.zip",
      media_type: "application/zip",
      size_bytes: 2048,
    },
  ],
};

const row: CatalogResultRow = {
  provider_product_key: "ASF::SENTINEL-1::P1",
  display_name: "P1.zip",
  collection: "SENTINEL-1",
  provider: "ASF",
  platform: "S1A",
  processing_level: "GRD_HD",
  acquisition_time: "2026-01-02T03:04:05Z",
  footprint_available: true,
  preview_status: "not_requested",
  file_size_label: "2 KB",
  asset_summary: "1 asset",
  kind: "sar",
  domain_field_cells: [
    { label: "Direction", value: "ASCENDING" },
    { label: "Polarization", value: "VV+VH (DV)" },
    { label: "Path", value: "142" },
    { label: "Frame", value: "90" },
  ],
  selected: true,
  active: false,
};

test("catalog info includes exact identity and compact scientific fields", () => {
  const text = catalogProductInfoText(product, row);

  assert.match(text, /^scene_id: S1A_IW_GRDH_P1/m);
  assert.match(text, /^platform: S1A/m);
  assert.match(text, /^product_family: GRD/m);
  assert.match(text, /^processing_level: GRD_HD/m);
  assert.match(text, /^beam_mode: IW/m);
  assert.match(text, /^polarization: VV\+VH \(DV\)/m);
  assert.match(text, /^orbit_direction: ASCENDING/m);
  assert.match(text, /^relative_orbit: 142/m);
  assert.match(text, /^frame: 90/m);
  assert.match(text, /^absolute_orbit: 53000/m);
  assert.match(text, /^acquisition_start_utc: 2026-01-02T03:04:05Z/m);
  assert.match(text, /^compressed_size: 2 KB/m);
  assert.match(text, /^download_url: https:\/\/datapool\.asf\.alaska\.edu\/GRD\/P1\.zip/m);
  assert.match(text, /^footprint_bbox: W110\.000000 S30\.000000 E111\.000000 N31\.000000/m);
  assert.match(text, /^footprint_centroid: 110\.500000°E, 30\.500000°N/m);
  assert.match(text, /^footprint_crs: EPSG:4326/m);
  assert.doesNotMatch(text, /Native metadata|provider_field|Dataset:|Assets \(/);
  assert.doesNotMatch(text, /incidence|heading|uncompressed|orbit_availability/i);
});

test("catalog info emits typed checksum and omits unknown optional fields", () => {
  const text = catalogProductInfoText(
    {
      ...product,
      acquisition_stop: "2026-01-02T03:04:33Z",
      checksum: "md5:0123456789abcdef0123456789abcdef",
      frame: null,
      asf_frame: "0090",
    },
    {
      ...row,
      domain_field_cells: row.domain_field_cells.map((cell) =>
        cell.label === "Frame" ? { ...cell, value: "0090" } : cell,
      ),
    },
  );

  assert.match(text, /^asf_frame: 0090/m);
  assert.match(text, /^acquisition_end_utc: 2026-01-02T03:04:33Z/m);
  assert.match(text, /^md5_checksum: 0123456789abcdef0123456789abcdef/m);
  assert.doesNotMatch(text, /^frame:/m);
});

test("SEASAT catalogue detail labels HDF5 as primary and Browse as display-only", () => {
  const seasat = {
    ...product,
    product_key: "ASF::SEASAT::SS_01502_STD_F2536",
    provider_product_key: "ASF::SEASAT::SS_01502_STD_F2536",
    provider_product_id: "SS_01502_STD_F2536",
    product_id: "SS_01502_STD_F2536",
    scene_id: "SS_01502_STD_F2536",
    dataset: "SEASAT",
    platform: "SEASAT 1",
    processing_level: null,
    product_family: "OTHER",
    file_name: "SS_01502_STD_F2536.h5",
    file_size_remote: 425_004_910,
    checksum: "fd99f89b380163d20c50b4a26fa0db12",
    download_url: "https://cumulus.asf.earthdatacloud.nasa.gov/SEASAT/SS_01502_STD_F2536/SS_01502_STD_F2536.h5",
    assets: [
      { ...product.assets[0], role: "primary" as const, file_name: "SS_01502_STD_F2536.h5" },
      { ...product.assets[0], role: "derived" as const, file_name: "SS_01502_STD_F2536.tif" },
      { ...product.assets[0], role: "browse" as const, file_name: "SS_01502_STD_F2536.jpg" },
    ],
  };
  const text = catalogProductInfoText(seasat, { ...row, collection: "SEASAT" });
  assert.match(text, /^primary_hdf5_file: SS_01502_STD_F2536\.h5/m);
  assert.match(text, /^primary_hdf5_size: 425 MB/m);
  assert.match(text, /^md5_checksum: fd99f89b380163d20c50b4a26fa0db12/m);
  assert.match(text, /^preview: Browse JPEG \(仅显示\)/m);
  assert.match(text, /^derived_geotiff: 可用但不自动下载/m);
  assert.doesNotMatch(text, /^download_url:/m);
});

test("missing provider centroid never becomes a fabricated zero-zero coordinate", () => {
  const text = catalogProductInfoText(
    {
      ...product,
      native_metadata: { properties: {} },
    },
    row,
  );

  assert.doesNotMatch(text, /^footprint_centroid:/m);
  assert.doesNotMatch(text, /0\.000000°E, 0\.000000°N/);

  const malformed = catalogProductInfoText({
    ...product,
    native_metadata: { properties: { centerLat: false, centerLon: " " } },
  }, row);
  assert.doesNotMatch(malformed, /^footprint_centroid:/m);
});

test("Sentinel-2 catalogue detail keeps optical metadata and omits SAR-only fields and URLs", () => {
  const text = catalogProductInfoText(
    {
      ...product,
      product_key: "CDSE::S2::P1",
      provider_product_key: "CDSE::S2::P1",
      provider_product_id: "165ba4c9-997c-4cea-970f-33ab9bdb8084",
      product_id: "165ba4c9-997c-4cea-970f-33ab9bdb8084",
      scene_id: "S2A_MSIL2A_20260726T125321_N0512_R138_T27VXL_20260726T221014.SAFE",
      provider: "copernicus.dataspace.odata",
      dataset: "SENTINEL-2 L2A",
      platform: "S2A",
      instrument: "MSI",
      processing_level: "L2A",
      product_family: "S2MSI2A",
      beam_mode: null,
      polarization: null,
      flight_direction: null,
      relative_orbit: null,
      absolute_orbit: null,
      frame: null,
      asf_frame: null,
      path: null,
      file_name: "S2A_MSIL2A_20260726T125321_N0512_R138_T27VXL_20260726T221014.SAFE.zip",
      file_size_remote: 867907004,
      checksum: "md5:0123456789abcdef0123456789abcdef",
      acquisition_stop: "2026-07-26T12:53:31Z",
      download_url: null,
      native_metadata: {
        logical_dataset: "optical.sentinel-2.l2a",
        native_collection_id: "SENTINEL-2",
        source_kind: "sentinel2",
        product_kind: "optical_scene",
        properties: { online: true, cloud_cover: 4.2, product_type: "S2MSI2A" },
      },
    },
    {
      ...row,
      kind: "optical",
      platform: "S2A",
      processing_level: "L2A",
      acquisition_time: "2026-07-26T12:53:21Z",
      domain_field_cells: [
        { label: "Cloud", value: "4.2%" },
        { label: "Tile", value: "T27VXL" },
        { label: "Online", value: "Online" },
      ],
    },
  );

  assert.match(text, /^product_id: 165ba4c9-997c-4cea-970f-33ab9bdb8084/m);
  assert.match(text, /^product_name: S2A_MSIL2A_20260726T125321_N0512_R138_T27VXL_20260726T221014\.SAFE/m);
  assert.match(text, /^processing_level: L2A/m);
  assert.match(text, /^cloud_cover: 4.2%/m);
  assert.match(text, /^mgrs_tile: T27VXL/m);
  assert.match(text, /^archive_availability: Online/m);
  assert.match(text, /^acquisition_end_utc: 2026-07-26T12:53:31Z/m);
  assert.match(text, /^archive_file: S2A_MSIL2A_20260726T125321_N0512_R138_T27VXL_20260726T221014\.SAFE\.zip/m);
  assert.match(text, /^archive_size: 867\.9 MB/m);
  assert.match(text, /^archive_size_bytes: 867907004/m);
  assert.match(text, /^md5_checksum: 0123456789abcdef0123456789abcdef/m);
  assert.doesNotMatch(text, /^polarization:/m);
  assert.doesNotMatch(text, /^orbit_direction:/m);
  assert.doesNotMatch(text, /^download_url:/m);
});

test("Landsat catalogue detail keeps entity, product, WRS, category, and browse truth separate", () => {
  const text = catalogProductInfoText(
    {
      ...product,
      product_key: "USGS::LANDSAT::ENTITY",
      provider_product_key: "USGS::LANDSAT::ENTITY",
      provider_product_id: "LC91230452026001LGN00",
      product_id: "LC91230452026001LGN00",
      scene_id: "LC09_L2SP_123045_20260101_20260103_02_T1",
      provider: "usgs.eros.m2m",
      dataset: "LANDSAT 8-9 C2 L2",
      platform: "landsat_9",
      instrument: "OLI/TIRS",
      processing_level: "l2sp",
      product_family: "OLI_TIRS_L2SP",
      beam_mode: null,
      polarization: null,
      flight_direction: null,
      relative_orbit: null,
      absolute_orbit: null,
      frame: null,
      asf_frame: null,
      path: null,
      file_name: null,
      file_size_remote: null,
      checksum: null,
      download_url: null,
      native_metadata: {
        logical_dataset: "optical.landsat.c2.l2",
        native_collection_id: "landsat_ot_c2_l2",
        source_kind: "landsat",
        product_kind: "optical_scene",
        properties: {
          entity_id: "LC91230452026001LGN00",
          landsat_product_id: "LC09_L2SP_123045_20260101_20260103_02_T1",
          landsat_scene_id: "LC91230452026001LGN00",
          spacecraft: "landsat_9",
          processing_level: "l2sp",
          cloud_cover: 12.5,
          wrs_path: "123",
          wrs_row: "045",
          collection_category: "T1",
          browse_available: true,
        },
      },
      assets: [],
    },
    {
      ...row,
      kind: "optical",
      platform: "Landsat 9",
      processing_level: "L2SP",
      domain_field_cells: [
        { label: "Cloud", value: "12.5%" },
        { label: "WRS", value: "123/045" },
        { label: "Category", value: "T1" },
        { label: "完整包", value: "下载前解析" },
      ],
    },
  );

  assert.match(text, /^entity_id: LC91230452026001LGN00/m);
  assert.match(text, /^landsat_product_id: LC09_L2SP_123045_20260101_20260103_02_T1/m);
  assert.match(text, /^landsat_scene_id: LC91230452026001LGN00/m);
  assert.match(text, /^spacecraft: Landsat 9/m);
  assert.match(text, /^processing_level: L2SP/m);
  assert.match(text, /^cloud_cover: 12.5%/m);
  assert.match(text, /^wrs_path_row: 123\/045/m);
  assert.match(text, /^collection_category: T1/m);
  assert.match(text, /^browse: 可用/m);
  assert.match(text, /^full_bundle: 下载前解析/m);
  assert.doesNotMatch(text, /^polarization:|^orbit_direction:|^mgrs_tile:|^download_url:/m);
});

test("ALOS-2 detail keeps authoritative identity, exact archive size, and no-Browse truth", () => {
  const alos2 = {
    ...product,
    product_key: "PROVIDER::asf.nasa.earthdata::COLLECTION::C3315903479-ASF::ITEM::G3463311647-ASF",
    provider_product_key: "PROVIDER::asf.nasa.earthdata::COLLECTION::C3315903479-ASF::ITEM::G3463311647-ASF",
    provider_product_id: "G3463311647-ASF",
    product_id: "G3463311647-ASF",
    scene_id: "ALOS2478123150-230331-WBDR1.1__D",
    provider: "asf.nasa.earthdata",
    dataset: "sar.alos-2.palsar-2.scansar.l1.1.v1",
    platform: "ALOS-2",
    instrument: "PALSAR-2",
    processing_level: "1.1",
    product_family: "ALOS2_L1_PSR2",
    beam_mode: "WBD",
    polarization: "HH+HV",
    flight_direction: "DESCENDING",
    relative_orbit: null,
    absolute_orbit: 47812,
    frame: null,
    path: null,
    acquisition_start: "2023-03-31T19:20:46Z",
    acquisition_stop: "2023-03-31T19:21:36Z",
    download_url: null,
    file_name: "ALOS2478123150-230331-WBDR1.1__D.zip",
    file_size_remote: 60_018_405_388,
    checksum: "md5:0f558597e19d964f24ca3a4bff11696c",
    native_metadata: {
      properties: {
        logical_dataset: "sar.alos-2.palsar-2.scansar.l1.1.v1",
        collection_concept_id: "C3315903479-ASF",
        collection_version: "1",
        collection_profile_id: "asf.alos-2.palsar-2.scansar.l1.1.v1",
        identity_rule_id: "asf.alos-2.cmr-granule.v1",
        filename_rule_id: "asf.alos-2.scansar-zip.v1",
        resolver_policy_id: "asf.alos-2.earthdata-complete-zip.v1",
        cmr_native_id: "ALOS2478123150-230331-WBDR1.1__D",
        producer_granule_id: "ALOS2478123150-230331",
      },
    },
  } satisfies SarProduct;
  const text = catalogProductInfoText(alos2, {
    ...row,
    collection: "ALOS-2 PALSAR-2 ScanSAR L1.1",
    provider: "asf.nasa.earthdata",
    platform: "ALOS-2",
    processing_level: "Level 1.1",
    file_size_label: "60,018,405,388 B",
    asset_summary: "完整 ZIP",
    domain_field_cells: [
      { label: "Beam", value: "WBD" },
      { label: "Polarization", value: "HH+HV" },
    ],
  });

  assert.match(text, /^product_id: G3463311647-ASF/m);
  assert.match(text, /^cmr_native_id: ALOS2478123150-230331-WBDR1\.1__D/m);
  assert.match(text, /^producer_granule_id: ALOS2478123150-230331/m);
  assert.match(text, /^dataset: ALOS-2 PALSAR-2 ScanSAR L1\.1/m);
  assert.match(text, /^platform: ALOS-2/m);
  assert.match(text, /^instrument: PALSAR-2/m);
  assert.match(text, /^processing_level: 1\.1/m);
  assert.match(text, /^beam_mode: WBD/m);
  assert.match(text, /^polarization: HH\+HV/m);
  assert.match(text, /^archive_file: ALOS2478123150-230331-WBDR1\.1__D\.zip/m);
  assert.match(text, /^archive_size: 60\.0 GB/m);
  assert.match(text, /^archive_size_bytes: 60018405388/m);
  assert.match(text, /^md5_checksum: 0f558597e19d964f24ca3a4bff11696c/m);
  assert.match(text, /^preview: unavailable \(no authoritative Browse\)/m);
  assert.doesNotMatch(text, /^download_url:/m);
});

test("thematic SceneRow platform localizes dynamically across ja/en switches while keeping buildSceneFullCopyText strictly English", () => {
  const worldCoverScene = {
    product_key: "ESA_WorldCover_10m_2021_v200_N30E117_Map",
    product_id: "ESA_WorldCover_10m_2021_v200_N30E117_Map",
    provider: "microsoft.planetary-computer",
    dataset: "science.esa.worldcover.v1",
    file_name: "ESA_WorldCover_10m_2021_v200_N30E117_Map.tif",
    scene_id: "ESA_WorldCover_10m_2021_v200_N30E117_Map",
    platform: "ESA WorldCover 全球 10 米土地利用分类",
    product_type: "map",
    beam_mode: "",
    polarization: "",
    acquisition_datetime: "2021-01-01T00:00:00Z",
    absolute_orbit: null,
    relative_orbit: null,
    path: null,
    frame: null,
    orbit_direction: "",
    has_url: true,
  } as unknown as SceneRow;

  try {
    setCurrentLocale("ja");
    const jaMeta = buildSceneRowMetaDisplay(worldCoverScene);
    assert.strictEqual(jaMeta.platform, "ESA WorldCover 全球 10m 土地被覆分類");
    assert.strictEqual(jaMeta.platform?.includes("土地利用分类"), false);
    assert.strictEqual(jaMeta.platform?.includes("10 米"), false);

    const jaSummary = buildSceneSummaryRows(worldCoverScene);
    const platSummary = jaSummary.find((r) => r.label === "衛星 / レベル" || r.value.includes("WorldCover"));
    assert.ok(platSummary);
    assert.strictEqual(platSummary.value.includes("土地利用分类"), false);

    // Standard English metadata copy must never leak Japanese or Chinese even when UI locale is ja.
    const copyInJa = buildSceneFullCopyText(worldCoverScene);
    assert.match(copyInJa, /^Platform \/ Sensor: ESA WorldCover Global 10m Land Cover$/m);
    assert.strictEqual(/[\u3040-\u30ff\u4e00-\u9fff]/.test(copyInJa), false, `copy text leaked CJK/kana:\n${copyInJa}`);

    // Switching from cached Japanese platform string directly to English updates immediately.
    const cachedJaScene = {
      ...worldCoverScene,
      platform: "ESA WorldCover 全球 10m 土地被覆分類",
    } as SceneRow;
    setCurrentLocale("en");
    const enMeta = buildSceneRowMetaDisplay(cachedJaScene);
    assert.strictEqual(enMeta.platform, "ESA WorldCover Global 10m Land Cover");
  } finally {
    setCurrentLocale("zh-CN");
  }
});

test("buildSceneFullCopyText keeps Source and Platform strictly English even when open-geodata provider falls back to localized displayTitle", () => {
  const cases = [
    { dataset: "science.nasa.emit.l2a.v1", expectedSource: "NASA EMIT L2A" },
    { dataset: "event.usgs.earthquake.v1", expectedSource: "USGS Earthquake" },
    { dataset: "science.comet.licsar.v1", expectedSource: "COMET-LiCSAR" },
  ];

  try {
    for (const c of cases) {
      for (const locale of ["ja", "zh-CN"] as const) {
        setCurrentLocale(locale);
        const fallbackTitle = localizedSourceDisplayName(c.dataset, locale);
        assert.ok(
          /[\u3040-\u30ff\u4e00-\u9fff]/.test(fallbackTitle),
          `expected localized ${locale} fallbackTitle for ${c.dataset}, got: ${fallbackTitle}`,
        );
        const scene = {
          product_key: `${c.dataset}_sample`,
          product_id: `${c.dataset}_sample`,
          provider: fallbackTitle,
          dataset: c.dataset,
          file_name: `${c.dataset}_sample.nc`,
          scene_id: `${c.dataset}_sample`,
          platform: fallbackTitle,
          product_type: "L2A",
          beam_mode: "",
          polarization: "",
          acquisition_datetime: "2026-10-01T00:00:00Z",
          absolute_orbit: null,
          relative_orbit: null,
          path: null,
          frame: null,
          orbit_direction: "",
          has_url: true,
        } as unknown as SceneRow;

        const copied = buildSceneFullCopyText(scene);
        assert.match(copied, new RegExp(`^Source: ${c.expectedSource}$`, "m"));
        assert.strictEqual(
          /[\u3040-\u30ff\u4e00-\u9fff]/.test(copied),
          false,
          `copy text in ${locale} for ${c.dataset} leaked CJK/kana:\n${copied}`,
        );
      }
    }
  } finally {
    setCurrentLocale("zh-CN");
  }
});

test("USGS Earthquake depth marker localizes per locale and stays strictly English in buildSceneFullCopyText", () => {
  const eqScene = {
    product_key: "USGS::EQ::eq1",
    product_id: "USGS_eq1_M6.5",
    title: "M6.5 · 50 km WSW of Example (深度 10.0km)",
    provider: "USGS Earthquake",
    dataset: "event.usgs.earthquake.v1",
    file_name: "M6.5 · 50 km WSW of Example (深度 10.0km)",
    scene_id: "USGS_eq1_M6.5",
    platform: "USGS Earthquake",
    product_type: "Event",
    depth_km: 10.0,
    place: "50 km WSW of Example",
    beam_mode: "",
    polarization: "",
    acquisition_datetime: "2026-10-01T00:00:00Z",
    absolute_orbit: null,
    relative_orbit: null,
    path: null,
    frame: null,
    orbit_direction: "",
    has_url: true,
  } as unknown as SceneRow;

  try {
    setCurrentLocale("zh-CN");
    assert.strictEqual(
      formatSceneRowDisplayTitle(eqScene),
      "M6.5 · 50 km WSW of Example (深度 10.0km)",
    );

    setCurrentLocale("en");
    assert.strictEqual(
      formatSceneRowDisplayTitle(eqScene),
      "M6.5 · 50 km WSW of Example (Depth 10.0 km)",
    );

    setCurrentLocale("ja");
    assert.strictEqual(
      formatSceneRowDisplayTitle(eqScene),
      "M6.5 · 50 km WSW of Example （震源深さ 10.0 km）",
    );
    const jaRows = buildSceneSummaryRows(eqScene);
    const jaIdRow = jaRows.find((r) => r.value.includes("M6.5"));
    assert.ok(jaIdRow);
    assert.strictEqual(jaIdRow.value.includes("深度"), false);
    assert.ok(jaIdRow.value.includes("（震源深さ 10.0 km）"));

    const copiedInJa = buildSceneFullCopyText(eqScene);
    assert.match(copiedInJa, /^Product ID: M6\.5 · 50 km WSW of Example \(Depth 10\.0 km\)$/m);
    assert.strictEqual(
      /[\u3040-\u30ff\u4e00-\u9fff]/.test(copiedInJa),
      false,
      `USGS Earthquake copy leaked CJK/kana:\n${copiedInJa}`,
    );
  } finally {
    setCurrentLocale("zh-CN");
  }
});
