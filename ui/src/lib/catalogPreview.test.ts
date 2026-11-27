import assert from "node:assert/strict";
import test from "node:test";

import type {
  ProviderAsset,
  ResolvedPreviewAsset,
  SarProduct,
} from "./generated/v3-contracts.ts";
import {
  PreviewRequestCoordinator,
  deriveCatalogPreview,
  deriveSarPreview,
  isDesktopControlledPreviewDataUrl,
  isPreviewActionAllowed,
  isRestrictedSarDataset,
  isReviewedPublicPreviewUrl,
  previewErrorMessage,
  projectSarCatalogRows,
  resolveCatalogPreviewBatchId,
  restrictedSarReason,
  saveReviewedPublicPreview,
  type PreviewRuntimeState,
} from "./catalogPreview.ts";

test("catalog preview batch binding prefers the exact frozen source for current and historical views", () => {
  assert.equal(resolveCatalogPreviewBatchId({
    scope: "current",
    currentBatchId: " current-batch ",
    taskSourceBatchId: "must-not-leak",
  }), "current-batch");
  assert.equal(resolveCatalogPreviewBatchId({
    scope: "task",
    currentBatchId: "must-not-leak",
    taskSourceBatchId: " historical-source ",
    taskBatchId: "task-batch",
    taskSnapshotBatchId: "legacy-task-batch",
  }), "historical-source");
  assert.equal(resolveCatalogPreviewBatchId({
    scope: "task",
    taskBatchId: "task-batch",
    taskSnapshotBatchId: "legacy-task-batch",
  }), "task-batch");
  assert.equal(resolveCatalogPreviewBatchId({
    scope: "task",
    taskSnapshotBatchId: "legacy-task-batch",
  }), "legacy-task-batch");
  assert.equal(resolveCatalogPreviewBatchId({ scope: "current" }), "");
  assert.equal(resolveCatalogPreviewBatchId({ scope: "task" }), "");
});

function asset(role: ProviderAsset["role"], url: string, options: Partial<ProviderAsset> = {}): ProviderAsset {
  return {
    asset_key: `${role}-asset`,
    stable_asset_key: `ASF::SENTINEL-1::P1::ASSET::${role}`,
    native_roles: [],
    access_descriptor: null,
    role,
    url,
    file_name: url.split("/").at(-1) ?? null,
    media_type: url.endsWith(".png") ? "image/png" : "image/jpeg",
    size_bytes: 1024,
    ...options,
  };
}

function product(overrides: Partial<SarProduct> = {}): SarProduct {
  return {
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
    relative_orbit: 42,
    absolute_orbit: 53000,
    frame: 100,
    asf_frame: null,
    path: 42,
    acquisition_start: "2026-01-02T03:04:05Z",
    acquisition_stop: null,
    footprint_geojson: null,
    footprint_bbox: { west: 110, east: 111, south: 30, north: 31, crs: "EPSG:4326" },
    download_url: "https://datapool.asf.alaska.edu/GRD/P1.zip",
    file_name: "P1.zip",
    file_size_remote: 2048,
    checksum: null,
    local_path: null,
    native_metadata: {},
    burst: null,
    assets: [],
    ...overrides,
  };
}

function alos2Product(): SarProduct {
  const fileName = "ALOS2478123150-230331-WBDR1.1__D.zip";
  return product({
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
    file_name: fileName,
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
      },
    },
    assets: [asset("primary", `https://datapool.asf.alaska.edu/ALOS2/${fileName}`, {
      url: null,
      file_name: fileName,
      media_type: "application/zip",
      size_bytes: 60_018_405_388,
      access_descriptor: {
        resolver_id: "earthdata_https",
        scheme: "earthdata_https",
        stable_href: `https://datapool.asf.alaska.edu/ALOS2/${fileName}`,
        requires_credentials: true,
        credential_profile: "earthdata-default",
        resolver_parameters: {},
      },
    })],
  });
}

function seasatProduct(): SarProduct {
  const scene = "SS_01502_STD_F2536";
  return product({
    product_key: `ASF::SEASAT::${scene}`,
    provider_product_key: `ASF::SEASAT::${scene}`,
    provider_product_id: scene,
    product_id: scene,
    scene_id: scene,
    dataset: "SEASAT",
    platform: "SEASAT 1",
    processing_level: null,
    product_family: "OTHER",
    file_name: `${scene}.h5`,
    file_size_remote: 425_004_910,
    checksum: "fd99f89b380163d20c50b4a26fa0db12",
    download_url: `https://cumulus.asf.earthdatacloud.nasa.gov/SEASAT/${scene}/${scene}.h5`,
    assets: [
      asset("primary", `https://cumulus.asf.earthdatacloud.nasa.gov/SEASAT/${scene}/${scene}.h5`, {
        file_name: `${scene}.h5`, media_type: "application/x-hdf5", size_bytes: 425_004_910,
      }),
      asset("derived", `https://cumulus.asf.earthdatacloud.nasa.gov/SEASAT/${scene}/${scene}.tif`, {
        file_name: `${scene}.tif`, media_type: "image/tiff", size_bytes: 287_076_789,
      }),
      asset("thumbnail", `https://cumulus.asf.earthdatacloud.nasa.gov/BROWSE/SEASAT/${scene}/${scene}_THUMBNAIL.jpg`),
      asset("browse", `https://cumulus.asf.earthdatacloud.nasa.gov/BROWSE/SEASAT/${scene}/${scene}.jpg`),
      asset("metadata", `https://cumulus.asf.earthdatacloud.nasa.gov/SEASAT/${scene}/${scene}.xml`, {
        media_type: "application/xml",
      }),
      asset("auxiliary", `https://cumulus.asf.earthdatacloud.nasa.gov/SEASAT/${scene}/${scene}.qc_report`, {
        media_type: "text/plain",
      }),
    ],
  });
}

test("SEASAT preview uses the main Browse JPEG and row summary keeps one HDF5 science primary", () => {
  const seasat = seasatProduct();
  const preview = deriveSarPreview(seasat);
  assert.equal(preview.kind, "candidate");
  if (preview.kind !== "candidate") return;
  assert.equal(preview.asset.role, "browse");
  assert.match(preview.url, /SS_01502_STD_F2536\.jpg$/);
  assert.doesNotMatch(preview.url, /\.h5$|\.tif$/);

  const [row] = projectSarCatalogRows([seasat], new Set(), seasat.product_key);
  assert.equal(row.asset_summary, "HDF5 主数据");
  assert.equal(row.display_name, "SS_01502_STD_F2536.h5");
  assert.equal(row.file_size_label, "425 MB");
});

test("catalog rows preserve provider identity while active and download selection stay independent", () => {
  const first = product();
  const second = product({
    product_key: "ASF::ERS::P1",
    provider_product_key: "ASF::ERS::P1",
    dataset: "ERS",
    product_id: "P1",
    scene_id: "SAME DISPLAY ALIAS",
  });

  const rows = projectSarCatalogRows(
    [first, second],
    new Set([second.product_key]),
    first.product_key,
  );

  assert.deepEqual(rows.map((row) => row.provider_product_key), [
    "ASF::SENTINEL-1::P1",
    "ASF::ERS::P1",
  ]);
  assert.equal(rows[0].active, true);
  assert.equal(rows[0].selected, false);
  assert.equal(rows[1].active, false);
  assert.equal(rows[1].selected, true);
  assert.deepEqual(rows[0].domain_field_cells, [
    { label: "Direction", value: "ASCENDING" },
    { label: "Polarization", value: "VV+VH (DV)" },
    { label: "Path", value: "42" },
    { label: "Frame", value: "100" },
  ]);
  assert.equal(rows[0].kind, "sar");
  assert.equal(rows[0].platform, "S1A");
});

test("Sentinel-2 rows expose optical fields and materialize quicklooks through the desktop path", async () => {
  const optical = product({
    product_key: "PROVIDER::copernicus.dataspace.odata::COLLECTION::SENTINEL-2::ITEM::165ba4c9-997c-4cea-970f-33ab9bdb8084",
    provider_product_key: "PROVIDER::copernicus.dataspace.odata::COLLECTION::SENTINEL-2::ITEM::165ba4c9-997c-4cea-970f-33ab9bdb8084",
    provider_product_id: "165ba4c9-997c-4cea-970f-33ab9bdb8084",
    product_id: "165ba4c9-997c-4cea-970f-33ab9bdb8084",
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
    path: null,
    scene_id: "S2A_MSIL2A_20260726T125321_N0512_R138_T27VXL_20260726T221014.SAFE",
    file_name: "S2A_MSIL2A_20260726T125321_N0512_R138_T27VXL_20260726T221014.SAFE.zip",
    file_size_remote: 867907004,
    download_url: null,
    native_metadata: {
      logical_dataset: "optical.sentinel-2.l2a",
      native_collection_id: "SENTINEL-2",
      source_kind: "sentinel2",
      product_kind: "optical_scene",
      properties: { online: true, cloud_cover: 4.2, product_type: "S2MSI2A" },
    },
    assets: [
      asset("primary", "https://example.invalid/archive.zip", {
        url: null,
        file_name: null,
        media_type: null,
        size_bytes: 867907004,
      }),
    ],
  });
  const [row] = projectSarCatalogRows([optical], new Set([optical.product_key]), optical.product_key);
  assert.equal(row?.kind, "optical");
  assert.deepEqual(row?.domain_field_cells, [
    { label: "Cloud", value: "4.2%" },
    { label: "Tile", value: "T27VXL" },
  ]);

  const source = deriveCatalogPreview(optical);
  assert.equal(source.kind, "cdse_quicklook");
  if (source.kind !== "cdse_quicklook") return;
  assert.equal(source.descriptor.media_type, "image/*");
  assert.deepEqual(source.descriptor.source_asset_keys, [optical.assets?.[0]?.stable_asset_key]);
  const published: PreviewRuntimeState[] = [];
  const coordinator = new PreviewRequestCoordinator((state) => published.push(state));
  let remoteLoads = 0;
  const state = await coordinator.requestCdseQuicklook(
    source,
    async () => {
      remoteLoads += 1;
      return { dataUrl: "data:image/jpeg;base64,cHJldmlldw==", mediaType: "image/jpeg" };
    },
    () => undefined,
  );
  assert.equal(remoteLoads, 1);
  assert.equal(state.status, "available");
  assert.equal(state.url, "data:image/jpeg;base64,cHJldmlldw==");
  assert.deepEqual(published.map((entry) => entry.status), ["loading", "available"]);
});

test("Microsoft Sentinel-2 remains an optical search result with Preview unavailable", () => {
  const key = "PROVIDER::microsoft.planetary-computer::COLLECTION::sentinel-2-l2a::ITEM::S2B_MSIL2A_20250129T022849_R046_T51RUH_20250129T040545";
  const searchOnly = product({
    product_key: key,
    provider_product_key: key,
    provider_product_id: "S2B_MSIL2A_20250129T022849_R046_T51RUH_20250129T040545",
    product_id: "S2B_MSIL2A_20250129T022849_R046_T51RUH_20250129T040545",
    provider: "microsoft.planetary-computer",
    dataset: "SENTINEL-2 L2A",
    platform: "S2B",
    instrument: "MSI",
    processing_level: "L2A",
    product_family: "S2MSI2A",
    beam_mode: null,
    polarization: null,
    flight_direction: null,
    relative_orbit: null,
    absolute_orbit: null,
    frame: null,
    path: null,
    scene_id: "S2B_MSIL2A_20250129T022849_N0511_R046_T51RUH_20250129T040545.SAFE",
    file_name: null,
    file_size_remote: null,
    download_url: null,
    native_metadata: {
      logical_dataset: "optical.sentinel-2.l2a",
      native_collection_id: "sentinel-2-l2a",
      collection_profile_id: "planetary-computer.sentinel-2-l2a.scene.v1",
      identity_rule_id: "planetary-computer.sentinel-2-l2a.identity.v1",
      filename_rule_id: "planetary-computer.sentinel-2-l2a.product-uri.v1",
      resolver_policy_id: "planetary-computer.sentinel-2-l2a.search-only.v1",
      source_kind: "sentinel2",
      product_kind: "optical_scene",
      properties: { cloud_cover: 43.816283, product_type: "S2MSI2A" },
      assets: [],
    },
    assets: [],
  });

  const [row] = projectSarCatalogRows([searchOnly], new Set(), key);
  assert.equal(row?.kind, "optical");
  assert.deepEqual(row?.domain_field_cells, [
    { label: "Cloud", value: "43.816283%" },
    { label: "Tile", value: "T51RUH" },
  ]);
  assert.equal(row?.asset_summary, "按需解析");

  const source = deriveCatalogPreview(searchOnly);
  assert.equal(source.kind, "unavailable");
  if (source.kind === "unavailable") {
    assert.match(source.reason, /Microsoft Sentinel-2.*仅提供检索结果/);
  }
});

test("Landsat rows expose native quality fields and browse uses the desktop path", async () => {
  const key = "PROVIDER::usgs.eros.m2m::COLLECTION::landsat_ot_c2_l2::ITEM::LC91230452026001LGN00";
  const landsat = product({
    product_key: key,
    provider_product_key: key,
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
    footprint_geojson: {
      type: "Polygon",
      coordinates: [[[110, 30], [111, 30], [111, 31], [110, 30]]],
    },
    footprint_bbox: null,
    file_name: null,
    file_size_remote: null,
    download_url: null,
    native_metadata: {
      logical_dataset: "optical.landsat.c2.l2",
      native_collection_id: "landsat_ot_c2_l2",
      source_kind: "landsat",
      product_kind: "optical_scene",
      properties: {
        entity_id: "LC91230452026001LGN00",
        landsat_product_id: "LC09_L2SP_123045_20260101_20260103_02_T1",
        spacecraft: "landsat_9",
        processing_level: "l2sp",
        collection_category: "T1",
        cloud_cover: 12.5,
        wrs_path: "123",
        wrs_row: "045",
        browse_available: true,
      },
    },
    assets: [],
  });

  const [row] = projectSarCatalogRows([landsat], new Set(), key);
  assert.equal(row?.kind, "optical");
  assert.equal(row?.platform, "Landsat 9");
  assert.equal(row?.processing_level, "L2SP");
  assert.equal(row?.file_size_label, "—");
  assert.equal(row?.asset_summary, "");
  assert.deepEqual(row?.domain_field_cells, [
    { label: "Cloud", value: "12.5%" },
    { label: "WRS", value: "123/045" },
    { label: "Category", value: "T1" },
  ]);

  const source = deriveCatalogPreview(landsat);
  assert.equal(source.kind, "landsat_browse");
  if (source.kind !== "landsat_browse") return;
  assert.equal(source.descriptor.render_profile_id, "usgs.landsat.c2.l2.exact-scene-browse.v1");
  assert.deepEqual(source.descriptor.source_asset_keys, [key]);

  const published: PreviewRuntimeState[] = [];
  const coordinator = new PreviewRequestCoordinator((state) => published.push(state));
  const state = await coordinator.requestDesktopPreview(
    source,
    async () => ({ dataUrl: "data:image/jpeg;base64,cHJldmlldw==", mediaType: "image/jpeg" }),
    () => undefined,
  );
  assert.equal(state.status, "available");
  assert.equal(state.product_key, key);
  assert.deepEqual(published.map((entry) => entry.status), ["loading", "available"]);

  const wrongProvider = product({ ...landsat, provider: "unreviewed.provider" });
  assert.equal(projectSarCatalogRows([wrongProvider], new Set(), key)[0]?.kind, "sar");
  assert.equal(deriveCatalogPreview(wrongProvider).kind, "unavailable");
});

test("catalog rows show file size and count only multiple non-Preview downloads", () => {
  const remoteSize = Math.round(4.6 * 1024 ** 3);
  const [singleDownload] = projectSarCatalogRows(
    [product({
      file_size_remote: remoteSize,
      assets: [
        asset("primary", "https://datapool.asf.alaska.edu/GRD/P1.zip", {
          media_type: "application/zip",
          size_bytes: remoteSize,
        }),
        asset("browse", "https://datapool.asf.alaska.edu/BROWSE/P1.jpg"),
      ],
    })],
    new Set(),
    null,
  );

  assert.equal(singleDownload.file_size_label, "4.9 GB");
  assert.equal(singleDownload.asset_summary, "");

  const [multipleDownloads] = projectSarCatalogRows(
    [product({
      assets: [
        asset("primary", "https://datapool.asf.alaska.edu/GRD/P1.zip", {
          media_type: "application/zip",
        }),
        asset("metadata", "https://datapool.asf.alaska.edu/GRD/P1.xml", {
          media_type: "application/xml",
        }),
        asset("thumbnail", "https://datapool.asf.alaska.edu/BROWSE/P1.png"),
      ],
    })],
    new Set(),
    null,
  );

  assert.equal(multipleDownloads.asset_summary, "2 个下载文件");
});

test("ALOS-2 no-Browse is unavailable without loading or promoting the complete ZIP", async () => {
  const alos2 = alos2Product();
  const source = deriveCatalogPreview(alos2);
  assert.deepEqual(source, {
    kind: "unavailable",
    reason: "该 ALOS-2 产品没有权威 Browse 资产；完整 ZIP 不能作为 Preview。",
  });

  const published: PreviewRuntimeState[] = [];
  let loads = 0;
  const coordinator = new PreviewRequestCoordinator((state) => published.push(state));
  const state = await coordinator.request(source, async () => {
    loads += 1;
  });
  assert.equal(loads, 0);
  assert.equal(state.status, "unavailable");
  assert.equal(state.url, null);
  assert.equal(state.descriptor, null);
  assert.deepEqual(published.map((entry) => entry.status), ["unavailable"]);
});

test("SAR Preview prefers a thumbnail, then browse, and never promotes a primary archive", () => {
  const preview = deriveSarPreview(product({
    assets: [
      asset("primary", "https://example.org/P1.zip", { media_type: "application/zip" }),
      asset("browse", "https://example.org/P1-browse.jpg"),
      asset("thumbnail", "https://example.org/P1-thumb.png"),
    ],
  }));

  assert.equal(preview.kind, "candidate");
  if (preview.kind !== "candidate") return;
  assert.equal(preview.url, "https://example.org/P1-thumb.png");
  assert.equal(preview.descriptor.preview_kind, "thumbnail");
  assert.deepEqual(preview.descriptor.source_asset_keys, [
    "ASF::SENTINEL-1::P1::ASSET::thumbnail",
  ]);

  const browse = deriveSarPreview(product({
    assets: [asset("browse", "https://example.org/P1-browse.jpg")],
  }));
  assert.equal(browse.kind, "candidate");
  if (browse.kind === "candidate") {
    assert.equal(browse.descriptor.preview_kind, "quicklook");
    assert.equal(
      browse.descriptor.render_profile_id,
      "asf.sentinel-1.grd-browse-geographic.v1",
    );
  }

  const archiveOnly = deriveSarPreview(product({
    assets: [asset("primary", "https://example.org/P1.zip", { media_type: "application/zip" })],
  }));
  assert.deepEqual(archiveOnly, {
    kind: "unavailable",
    reason: "该产品没有可用的缩略图或浏览图资产。",
  });
});

test("only reviewed Sentinel-1 GRD browse is eligible for map placement", () => {
  const alos = deriveSarPreview(product({
    dataset: "ALOS PALSAR",
    platform: "ALOS",
    identity_rule_id: "asf.alos-palsar.v2",
    product_family: "L1.1",
    processing_level: "L1.1",
    assets: [asset("browse", "https://datapool.asf.alaska.edu/BROWSE/ALOS.jpg")],
  }));
  assert.equal(alos.kind, "candidate");
  if (alos.kind === "candidate") {
    assert.equal(alos.descriptor.render_profile_id, "sar.provider-browse.v1");
  }

  const slc = deriveSarPreview(product({
    product_family: "SLC",
    processing_level: "SLC",
    assets: [asset("browse", "https://datapool.asf.alaska.edu/BROWSE/SLC.jpg")],
  }));
  assert.equal(slc.kind, "candidate");
  if (slc.kind === "candidate") {
    assert.equal(slc.descriptor.render_profile_id, "sar.provider-browse.v1");
  }
});

test("Preview save/open actions accept only reviewed public HTTPS hosts", () => {
  assert.equal(
    isReviewedPublicPreviewUrl("https://datapool.asf.alaska.edu/BROWSE/P1.jpg"),
    true,
  );
  assert.equal(
    isReviewedPublicPreviewUrl("https://api.daac.asf.alaska.edu/services/search/P1.png"),
    true,
  );
  assert.equal(isReviewedPublicPreviewUrl("http://datapool.asf.alaska.edu/P1.jpg"), false);
  assert.equal(isReviewedPublicPreviewUrl("https://asf.alaska.edu.evil.example/P1.jpg"), false);
  assert.equal(isReviewedPublicPreviewUrl("https://example.org/P1.jpg"), false);
});

test("desktop-controlled Preview data URLs are allowed only for bounded image payloads", () => {
  const valid = "data:image/jpeg;base64,cHJldmlldw==";
  assert.equal(isDesktopControlledPreviewDataUrl(valid), true);
  assert.equal(isPreviewActionAllowed(valid), true);
  assert.equal(isDesktopControlledPreviewDataUrl("data:image/svg+xml;base64,PHN2Zz4="), false);
  assert.equal(isPreviewActionAllowed("data:image/jpeg;base64,not base64"), false);
});

test("Preview save fetches anonymously and downloads the decoded public image", async () => {
  const calls: unknown[] = [];
  await saveReviewedPublicPreview(
    "https://datapool.asf.alaska.edu/BROWSE/P1.jpg",
    "S1A_GRD_HD.jpg",
    {
      fetch: async (url, init) => {
        calls.push([url, init]);
        return new Response(new Blob(["preview"], { type: "image/jpeg" }), { status: 200 });
      },
      createObjectUrl: (blob) => {
        calls.push(["blob", blob.type]);
        return "blob:preview";
      },
      revokeObjectUrl: (url) => calls.push(["revoke", url]),
      clickDownload: (url, fileName) => calls.push(["download", url, fileName]),
    },
  );

  assert.deepEqual(calls[0], [
    "https://datapool.asf.alaska.edu/BROWSE/P1.jpg",
    { credentials: "omit", cache: "no-store", signal: undefined },
  ]);
  assert.deepEqual(calls.at(-2), ["download", "blob:preview", "S1A_GRD_HD.jpg"]);
  assert.deepEqual(calls.at(-1), ["revoke", "blob:preview"]);
});

test("Preview save rejects an unreviewed host before any network request", async () => {
  let fetched = false;
  await assert.rejects(
    saveReviewedPublicPreview(
      "https://evil.example/P1.jpg",
      "P1.jpg",
      {
        fetch: async () => {
          fetched = true;
          return new Response();
        },
        createObjectUrl: () => "blob:preview",
        revokeObjectUrl: () => undefined,
        clickDownload: () => undefined,
      },
    ),
    /not an approved public Preview host/,
  );
  assert.equal(fetched, false);
});

test("credentialed and unsafe Preview assets are not started as public images", () => {
  const credentialed = deriveSarPreview(product({
    assets: [asset("browse", "https://datapool.asf.alaska.edu/BROWSE/P1.jpg", {
      access_descriptor: {
        resolver_id: "earthdata_https",
        scheme: "earthdata_https",
        stable_href: "https://datapool.asf.alaska.edu/BROWSE/P1.jpg",
        requires_credentials: true,
        credential_profile: "earthdata.default",
        resolver_parameters: {},
      },
    })],
  }));
  assert.equal(credentialed.kind, "candidate");
  if (credentialed.kind === "candidate") {
    assert.equal(credentialed.descriptor.credential_requirement, "provider_credentials");
  }

  assert.equal(
    deriveSarPreview(product({
      assets: [asset("browse", "http://example.org/unsafe.jpg")],
    })).kind,
    "unavailable",
  );
});

test("shared resolved Preview path consumes one generated descriptor and typed errors", async () => {
  const published: PreviewRuntimeState[] = [];
  const coordinator = new PreviewRequestCoordinator((state) => published.push(state));
  const resolved: ResolvedPreviewAsset = {
    product_key: "ASF::SENTINEL-1::GRD",
    descriptor: {
      preview_kind: "quicklook",
      source_asset_keys: ["ASF::SENTINEL-1::GRD::browse"],
      render_profile_id: "asf.sentinel-1.grd-browse-geographic.v1",
      media_type: "image/jpeg",
      credential_requirement: "none",
      cache_policy: "session",
      provenance: "ASF official Browse",
    },
    source_asset_role: "browse",
    source_asset_key: "ASF::SENTINEL-1::GRD::browse",
    media_type: "image/jpeg",
    data_url: "data:image/jpeg;base64,AQID",
    byte_length: 3,
  };
  const state = await coordinator.requestResolvedPreview(
    resolved.product_key,
    async () => resolved,
    () => undefined,
  );
  assert.equal(state.status, "available");
  assert.equal(state.descriptor?.render_profile_id, resolved.descriptor.render_profile_id);
  assert.equal(state.url, resolved.data_url);
  assert.deepEqual(published.map((entry) => entry.status), ["loading", "available"]);

  assert.match(previewErrorMessage("provider_no_preview"), /Provider/);
  assert.match(previewErrorMessage("preview_http_403"), /403/);
  assert.match(previewErrorMessage("preview_http_404"), /404/);
  assert.match(previewErrorMessage("preview_timeout"), /超时/);
  assert.match(previewErrorMessage("preview_invalid_media_type"), /媒体类型/);
  assert.match(previewErrorMessage("preview_decode_failed"), /解码/);
});

function deferred() {
  let resolve!: () => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<void>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

test("Preview coordinator cancels the old generation and never publishes a late result", async () => {
  const published: PreviewRuntimeState[] = [];
  const coordinator = new PreviewRequestCoordinator((state) => published.push(state));
  const firstLoad = deferred();
  const secondLoad = deferred();
  const first = deriveSarPreview(product({
    assets: [asset("browse", "https://example.org/first.jpg")],
  }));
  const secondProduct = product({
    product_key: "ASF::SENTINEL-1::P2",
    provider_product_key: "ASF::SENTINEL-1::P2",
    product_id: "P2",
  });
  const second = deriveSarPreview({
    ...secondProduct,
    assets: [asset("browse", "https://example.org/second.jpg")],
  });

  const firstResult = coordinator.request(first, () => firstLoad.promise);
  const secondResult = coordinator.request(second, () => secondLoad.promise);
  firstLoad.resolve();
  secondLoad.resolve();

  assert.equal((await firstResult).status, "stale");
  assert.equal((await secondResult).status, "available");
  assert.deepEqual(published.map((state) => state.status), [
    "loading",
    "loading",
    "available",
  ]);
  assert.equal(published.at(-1)?.product_key, "ASF::SENTINEL-1::P2");
});

test("Preview coordinator distinguishes unavailable, authentication, cancellation, and failure", async () => {
  const published: PreviewRuntimeState[] = [];
  const coordinator = new PreviewRequestCoordinator((state) => published.push(state));

  const unavailable = await coordinator.request(
    { kind: "unavailable", reason: "no browse" },
    () => Promise.reject(new Error("loader must not run")),
  );
  assert.equal(unavailable.status, "unavailable");

  const credentialed = deriveSarPreview(product({
    assets: [asset("browse", "https://example.org/private.jpg", {
      access_descriptor: {
        resolver_id: "earthdata_https",
        scheme: "earthdata_https",
        stable_href: "https://example.org/private.jpg",
        requires_credentials: true,
        credential_profile: "earthdata.default",
        resolver_parameters: {},
      },
    })],
  }));
  assert.equal((await coordinator.request(credentialed, () => Promise.resolve())).status, "authentication_required");

  const pending = deferred();
  const running = coordinator.request(
    deriveSarPreview(product({ assets: [asset("browse", "https://example.org/cancel.jpg")] })),
    () => pending.promise,
  );
  coordinator.cancel();
  pending.reject(new DOMException("aborted", "AbortError"));
  assert.equal((await running).status, "cancelled");

  const failed = await coordinator.request(
    deriveSarPreview(product({ assets: [asset("browse", "https://example.org/fail.jpg")] })),
    () => Promise.reject(new Error("image decode failed")),
  );
  assert.equal(failed.status, "failed");
  assert.match(failed.message, /image decode failed/);
});

test("CDSE quicklook cancellation invalidates the local generation and sends one application cancel", async () => {
  const optical = product({
    product_key: "PROVIDER::copernicus.dataspace.odata::COLLECTION::SENTINEL-2::ITEM::165ba4c9-997c-4cea-970f-33ab9bdb8084",
    provider_product_key: "PROVIDER::copernicus.dataspace.odata::COLLECTION::SENTINEL-2::ITEM::165ba4c9-997c-4cea-970f-33ab9bdb8084",
    provider_product_id: "165ba4c9-997c-4cea-970f-33ab9bdb8084",
    product_id: "165ba4c9-997c-4cea-970f-33ab9bdb8084",
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
    path: null,
    scene_id: "S2A_MSIL2A_20260726T125321_N0512_R138_T27VXL_20260726T221014.SAFE",
    file_name: "S2A_MSIL2A_20260726T125321_N0512_R138_T27VXL_20260726T221014.SAFE.zip",
    download_url: null,
    native_metadata: {
      logical_dataset: "optical.sentinel-2.l2a",
      native_collection_id: "SENTINEL-2",
      source_kind: "sentinel2",
      product_kind: "optical_scene",
      properties: { online: true, product_type: "S2MSI2A" },
    },
    assets: [asset("primary", "https://example.invalid/archive.zip", {
      url: null,
      file_name: null,
      media_type: null,
      size_bytes: 867907004,
    })],
  });
  const source = deriveCatalogPreview(optical);
  assert.equal(source.kind, "cdse_quicklook");
  if (source.kind !== "cdse_quicklook") return;

  let resolve!: (value: { dataUrl: string; mediaType: string }) => void;
  const delayed = new Promise<{ dataUrl: string; mediaType: string }>((next) => {
    resolve = next;
  });
  let remoteCancels = 0;
  const published: PreviewRuntimeState[] = [];
  const coordinator = new PreviewRequestCoordinator((state) => published.push(state));
  const request = coordinator.requestCdseQuicklook(
    source,
    () => delayed,
    () => {
      remoteCancels += 1;
    },
  );
  coordinator.cancel();
  coordinator.cancel();
  resolve({ dataUrl: "data:image/jpeg;base64,cHJldmlldw==", mediaType: "image/jpeg" });

  assert.equal((await request).status, "cancelled");
  assert.equal(remoteCancels, 1);
  assert.deepEqual(published.map((entry) => entry.status), ["loading", "cancelled"]);
});

test("ASF-removed legacy SAR rows carry the non-downloadable flag, open missions do not", () => {
  const jers = product({
    product_key: "PROVIDER::asf.catalog::COLLECTION::JERS-1::ITEM::JERS1_0291_0025_950612",
    provider_product_key: "PROVIDER::asf.catalog::COLLECTION::JERS-1::ITEM::JERS1_0291_0025_950612",
    provider_product_id: "JERS1_0291_0025_950612",
    product_id: "JERS1_0291_0025_950612",
    scene_id: "JERS1_0291_0025_950612",
    provider: "asf.catalog",
    dataset: "JERS-1",
    platform: "JERS-1",
    polarization: "HH",
  });
  const ers = product({ dataset: "ERS" });
  const radarsat = product({ dataset: "RADARSAT-1" });
  const seasat = product({ dataset: "SEASAT" });

  const [jersRow] = projectSarCatalogRows([jers], new Set(), jers.product_key);
  assert.equal(jersRow.download_restricted, true, "JERS-1 rows are marked restricted");

  const [ersRow] = projectSarCatalogRows([ers], new Set(), ers.product_key);
  assert.equal(ersRow.download_restricted, true, "ERS was removed from ASF searchable interfaces");

  const [radarsatRow] = projectSarCatalogRows([radarsat], new Set(), radarsat.product_key);
  assert.equal(
    radarsatRow.download_restricted,
    true,
    "RADARSAT-1 was removed from ASF searchable interfaces",
  );

  const [seasatRow] = projectSarCatalogRows([seasat], new Set(), seasat.product_key);
  assert.equal(seasatRow.download_restricted, false, "SEASAT keeps its download path");

  assert.equal(isRestrictedSarDataset("jers-1 "), true, "the check is case- and trim-insensitive");
  assert.equal(isRestrictedSarDataset("radarsat-1"), true);
  assert.equal(isRestrictedSarDataset("SEASAT"), false);
  assert.equal(isRestrictedSarDataset(null), false);
  assert.equal(isRestrictedSarDataset(42), false);

  assert.equal(restrictedSarReason("ERS"), "ASF 已从检索接口下线该数据集");
  assert.equal(restrictedSarReason("RADARSAT-1"), "ASF 已从检索接口下线该数据集");
  assert.equal(
    restrictedSarReason("JERS-1"),
    "JERS-1 是受限数据集，仅支持检索，不能创建下载任务。有关数据访问，请联系 ASF 用户支持。",
  );
});
