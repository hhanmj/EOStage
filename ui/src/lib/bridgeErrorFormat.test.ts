import assert from "node:assert/strict";
import test from "node:test";

import { formatBridgeErrorValue as formatBridgeError } from "./bridgeErrorFormat.ts";

test("Bhoonidhi catalogue shows a bounded HTTP status without leaking provider diagnostics", () => {
  assert.match(formatBridgeError({ code: "BHOONIDHI_HTTP_ERROR", context: "http_status:422", error: "application_service.unavailable" }), /HTTP 422.*查询参数/);
  assert.match(formatBridgeError({ code: "BHOONIDHI_HTTP_ERROR", context: "http_status:413" }), /HTTP 413.*缩小/);
  const message = formatBridgeError({ code: "BHOONIDHI_HTTP_ERROR", context: "https://private/?token=secret" });
  assert.doesNotMatch(message, /secret|private|https:/);
  assert.match(message, /Bhoonidhi 目录请求失败/);
  assert.doesNotMatch(formatBridgeError({ code: "BHOONIDHI_HTTP_ERROR", context: "http_status:999" }), /HTTP 999/);
});

test("restart-limit errors explain recovery without exposing implementation jargon", () => {
  const message = formatBridgeError({
    ok: false,
    error: "Rust Sidecar 重启次数超过限制，本次会话已停用",
    code: "V3_SIDECAR_RESTART_LIMIT",
  });

  assert.match(message, /后台任务服务连续异常退出/);
  assert.match(message, /重启 EOStage/);
  assert.doesNotMatch(message, /Rust|Sidecar|V3_/);
});

test("timeout errors keep a possibly-running task instead of suggesting duplicate creation", () => {
  const message = formatBridgeError({
    ok: false,
    error: "后台任务服务响应超时",
    code: "V3_SIDECAR_TIMEOUT",
  });

  assert.match(message, /任务可能仍在运行/);
  assert.match(message, /不要重复创建/);
  assert.doesNotMatch(message, /V3_/);
});

test("Sentinel-2 authentication errors identify the required Copernicus account", () => {
  const message = formatBridgeError({
    ok: false,
    error: "CDSE OAuth login rejected",
    code: "CDSE_OAUTH_REJECTED",
  });

  assert.match(message, /Copernicus Data Space/);
  assert.match(message, /TOTP/);
  assert.doesNotMatch(message, /USGS token/);
});

test("new task admission requests provider login instead of a generic failure", () => {
  for (const [code, provider] of [
    ["CDSE_ACCOUNT_LOGIN_REQUIRED", "CDSE"],
    ["USGS_ACCOUNT_LOGIN_REQUIRED", "USGS"],
    ["EARTHDATA_ACCOUNT_LOGIN_REQUIRED", "Earthdata"],
    ["BHOONIDHI_ACCOUNT_LOGIN_REQUIRED", "Bhoonidhi"],
  ]) {
    const message = formatBridgeError({ code, error: "application_service.unavailable" });
    assert.match(message, new RegExp(`需要登录 ${provider} 后才能继续`));
    assert.doesNotMatch(message, /操作失败|application_service|https?:\/\/|fixture-secret/i);
  }
});

test("unavailable credential store rejects admission without asking to log in", () => {
  for (const [code, provider] of [
    ["CDSE_ACCOUNT_SERVICE_UNAVAILABLE", "CDSE"],
    ["USGS_ACCOUNT_SERVICE_UNAVAILABLE", "USGS"],
  ]) {
    const message = formatBridgeError({ code, error: "application_service.unavailable" });
    assert.match(message, new RegExp(provider));
    assert.match(message, /系统凭据|系统凭证|凭据管理器/);
    assert.doesNotMatch(message, /需要登录|保存.*账号|Application Token|application_service|fixture-secret/i);
  }
});

test("USGS M2M authentication errors preserve the real failure class", () => {
  const cases = [
    ["USGS_M2M_AUTH_INVALID", /Application Token|用户名/],
    ["USGS_M2M_AUTH_UNAUTHORIZED", /M2M 权限|Application Token/],
    ["USGS_M2M_TRANSPORT_FAILED", /网络|DNS|代理|TLS/],
    ["USGS_M2M_TIMEOUT", /超时|网络|代理/],
    ["USGS_M2M_RATE_LIMITED", /请求过多|稍后/],
  ] as const;

  for (const [code, expected] of cases) {
    const message = formatBridgeError({ code, error: "USGS_M2M_INTERNAL_DETAIL" });
    assert.match(message, expected, code);
    assert.doesNotMatch(message, /操作失败，请稍后重试/, code);
    assert.doesNotMatch(message, /USGS_M2M_INTERNAL_DETAIL/, code);
  }
});

test("generic application failures never invent a catalogue operation or an unrelated login cause", () => {
  const message = formatBridgeError({
    ok: false,
    error: "Tauri application service operation failed",
    code: "V3_APPLICATION_UNAVAILABLE",
  });

  assert.match(message, /无法确认|未能确认/);
  assert.match(message, /不要重复提交/);
  assert.doesNotMatch(message, /Sentinel-2|Copernicus|Landsat|USGS|没有加载|目录检索/);
});

test("a safe specific Chinese invalid-request reason is not hidden by a catch-all", () => {
  const result = {
    ok: false as const,
    error: "请先打开工作区并选择研究区",
    code: "V3_APPLICATION_INVALID_REQUEST",
  };

  const message = formatBridgeError(result);

  assert.equal(message, "请先打开工作区并选择研究区");
  assert.doesNotMatch(message, /AOI 不完整|确认输出目录/);
  assert.doesNotMatch(message, /V3_APPLICATION_INVALID_REQUEST/);
  assert.equal(result.code, "V3_APPLICATION_INVALID_REQUEST");
});

test("operation-specific prerequisite codes keep search, AOI, and download causes separate", () => {
  const search = formatBridgeError({ code: "CATALOG_SEARCH_INVALID_INPUT" });
  const aoi = formatBridgeError({ code: "CATALOG_SEARCH_AOI_REQUIRED" });
  const output = formatBridgeError({ code: "DOWNLOAD_OUTPUT_REQUIRED" });

  assert.match(search, /检索参数/);
  assert.doesNotMatch(search, /AOI|输出目录/);
  assert.match(aoi, /数据源检索|AOI/);
  assert.doesNotMatch(aoi, /输出目录/);
  assert.match(output, /下载|输出目录/);
  assert.doesNotMatch(output, /检索参数|AOI/);
});

test("OpenTopography area-limit errors explain the provider bound and the admitted AWS alternative", () => {
  const message = formatBridgeError({ code: "DEM_PROVIDER_AREA_LIMIT_EXCEEDED" });

  assert.match(message, /OpenTopography/);
  assert.match(message, /面积上限/);
  assert.match(message, /Copernicus AWS/);
  assert.doesNotMatch(message, /应用状态已变化|刷新页面/);
});

test("typed catalogue failures explain the failure class instead of showing the generic fallback", () => {
  const cases = [
    ["CATALOGUE_QUERY_INVALID", /检索条件|参数/],
    ["CATALOGUE_FILTER_INVALID", /筛选条件/],
    ["CATALOGUE_RESPONSE_INVALID", /目录服务|返回/],
    ["CATALOGUE_REQUEST_CANCELLED", /取消/],
    ["CATALOGUE_TIME_BUDGET_EXCEEDED", /超时|时间/],
    ["CATALOGUE_HTTP_CLIENT_FAILURE", /目录服务|筛选/],
    ["CATALOGUE_HTTP_SERVER_FAILURE", /目录服务|服务/],
    ["CATALOGUE_TRANSPORT_FAILURE", /网络|代理/],
    ["CATALOGUE_PROVIDER_TIMEOUT", /超时/],
    ["CATALOGUE_RESPONSE_TOO_LARGE", /安全上限|缩小/],
    ["CATALOGUE_RESPONSE_INVALID_JSON", /无法解析/],
    ["CATALOGUE_INVALID_POLYGON", /AOI|空间/],
    ["CATALOGUE_SPATIAL_PLAN_EXCEEDED", /AOI|拆分/],
    ["CATALOGUE_RESULT_LIMIT_EXCEEDED", /结果数量|范围/],
  ] as const;

  for (const [code, expected] of cases) {
    const message = formatBridgeError({ code });
    assert.match(message, expected, code);
    assert.doesNotMatch(message, /操作失败，请稍后重试/, code);
  }
});

test("generic catalogue failures never mislabel non-CMR providers as ASF/CMR", () => {
  for (const code of [
    "CATALOGUE_HTTP_CLIENT_FAILURE",
    "CATALOGUE_HTTP_SERVER_FAILURE",
    "CATALOGUE_TRANSPORT_FAILURE",
    "CATALOGUE_PROVIDER_TIMEOUT",
    "CATALOGUE_RESPONSE_TOO_LARGE",
    "CATALOGUE_RESPONSE_INVALID_JSON",
    "CATALOGUE_INVALID_POLYGON",
  ]) {
    assert.doesNotMatch(formatBridgeError({ code }), /CMR|ASF/, code);
  }
});

test("Sentinel-2 SAFE delivery errors explain CDSE re-resolution without exposing a locator", () => {
  const cases = [
    ["SENTINEL2_ARCHIVE_FROZEN_EVIDENCE_INVALID", /结果.*变化|重新选择/],
    ["SENTINEL2_ARCHIVE_DELIVERY_UNAVAILABLE", /Copernicus Data Space|SAFE.*归档/],
    ["SENTINEL2_ARCHIVE_REQUEST_DUPLICATE", /正在核对|等待/],
    ["SENTINEL2_ARCHIVE_CANCELLED", /取消|没有创建/],
  ] as const;

  for (const [code, expected] of cases) {
    const message = formatBridgeError({ code });
    assert.match(message, expected, code);
    assert.doesNotMatch(message, /https?:\/\/|token|uuid|locator/i, code);
  }
});

test("Bhoonidhi failures keep the S-band failure class instead of collapsing to the generic provider error", () => {
  const cases = [
    ["BHOONIDHI_UNAUTHORIZED", /登录会话|重新登录/],
    ["BHOONIDHI_FORBIDDEN", /拒绝|权限/],
    ["BHOONIDHI_RATE_LIMITED", /请求过多|稍后/],
    ["BHOONIDHI_NETWORK_ERROR", /连接|网络|DNS|代理/],
    ["BHOONIDHI_QUERY_INVALID", /S-band|检索条件|AOI/],
    ["BHOONIDHI_RESPONSE_INVALID", /返回|识别/],
    ["BHOONIDHI_PRODUCT_INVALID", /无效.*S-band.*产品/],
    ["BHOONIDHI_PRODUCT_IDENTITY_MISMATCH", /产品身份|检索范围/],
    ["BHOONIDHI_PRODUCT_SCOPE_MISMATCH", /产品属性|检索范围/],
  ] as const;

  for (const [code, expected] of cases) {
    const message = formatBridgeError({ code });
    assert.match(message, expected, code);
    assert.doesNotMatch(message, /操作失败，请稍后重试/, code);
  }
});

test("Bhoonidhi authentication timeout stays concise and typed", () => {
  const message = formatBridgeError({ code: "BHOONIDHI_AUTH_TIMEOUT" });

  assert.equal(message, "Bhoonidhi 认证超时，请稍后重试。");
  assert.doesNotMatch(message, /不能证明|密码错误|反复/);
});

test("Bhoonidhi query invalid surfaces bounded Chinese polygon guidance when provided", () => {
  const withDetail = formatBridgeError({
    code: "BHOONIDHI_QUERY_INVALID",
    error: "application_service.unavailable",
    context: "研究区多边形边界顶点过多（超过 Bhoonidhi 接口上限），请改用矩形或简化多边形框选",
  });
  assert.equal(
    withDetail,
    "研究区多边形边界顶点过多（超过 Bhoonidhi 接口上限），请改用矩形或简化多边形框选。",
  );

  const generic = formatBridgeError({ code: "BHOONIDHI_QUERY_INVALID" });
  assert.match(generic, /Bhoonidhi S-band 检索条件无效/);
});

test("an unknown provider failure keeps a sanitized provider message", () => {
  const message = formatBridgeError({
    code: "V3_PROVIDER_OPERATION_FAILED",
    error: "application_service.unavailable",
    context: "Bhoonidhi 网络请求失败",
  });

  assert.equal(message, "数据源操作失败：Bhoonidhi 网络请求失败。");
  assert.doesNotMatch(message, /V3_PROVIDER|application_service|操作失败，请稍后重试/);
});

test("typed Microsoft optical catalogue failures keep actionable failure classes", () => {
  const cases = [
    ["STAC_IDENTITY_MISMATCH", /产品身份字段|无法确认身份/],
    ["STAC_RESPONSE_INVALID", /产品元数据|必需字段/],
    ["STAC_TIMEOUT", /超时|缩小检索范围/],
    ["STAC_NETWORK_ERROR", /网络|DNS|代理/],
    ["PLANETARY_COMPUTER_RESPONSE_TOO_LARGE", /安全上限|结果数量/],
    ["PLANETARY_COMPUTER_TRANSPORT_FAILED", /Planetary Computer|网络|DNS|代理/],
  ] as const;

  for (const [code, expected] of cases) {
    const message = formatBridgeError({ code, error: "application_service.unavailable" });
    assert.match(message, expected, code);
    assert.doesNotMatch(message, /数据源操作失败：|V3_PROVIDER_OPERATION_FAILED/, code);
  }
});

test("typed Burst failures explain the failure class instead of showing the generic fallback", () => {
  const cases = [
    ["ASF002", /ASF|CMR|网络|代理/],
    ["ASF_QUERY_UNSUPPORTED", /Burst|筛选/],
    ["ASF_PAGINATION_INVALID", /Burst|分页|范围/],
    ["ASF_IDENTITY_CONFLICT", /Burst|产品标识/],
    ["ASF_RATE_LIMITED", /请求过多|稍后/],
    ["ASF_SERVER_ERROR", /服务|稍后/],
    ["ASF_TIMEOUT", /超时|范围/],
    ["ASF_TRANSPORT_FAILED", /网络|DNS|代理/],
    ["ASF_RESPONSE_TOO_LARGE", /安全上限|结果数量/],
  ] as const;

  for (const [code, expected] of cases) {
    const message = formatBridgeError({ code });
    assert.match(message, expected, code);
    assert.doesNotMatch(message, /操作失败，请稍后重试/, code);
  }
});

test("generic ASF query errors do not mislabel a non-Burst dataset", () => {
  const message = formatBridgeError(
    { code: "ASF_QUERY_INVALID", error: "当前查询无效" },
    { dataset: "ALOS-2" },
  );

  assert.equal(message, "当前 SAR 检索条件无效，请检查当前数据集的筛选条件。");
  assert.doesNotMatch(message, /Burst/);
});

test("Burst query errors keep the Burst-specific guidance when dataset context is explicit", () => {
  const message = formatBridgeError(
    { code: "ASF_QUERY_INVALID" },
    { dataset: "SLC-BURST" },
  );

  assert.match(message, /Burst/);
  assert.match(message, /时间|AOI|筛选参数/);
});

test("an internal invalid-request reason uses a neutral fallback without inventing AOI or output requirements", () => {
  const message = formatBridgeError({
    code: "V3_APPLICATION_INVALID_REQUEST",
    error: "workspace snapshot route requires internal owner",
  });

  assert.match(message, /请求参数无效/);
  assert.doesNotMatch(message, /workspace|snapshot|route|AOI|输出目录/i);
});

test("invalid-request context hides internal ownership details but preserves actionable DEM validation", () => {
  const internal = formatBridgeError({
    code: "V3_APPLICATION_INVALID_REQUEST",
    context: "workspace snapshot route requires internal owner",
  });
  assert.match(internal, /请求参数无效/);
  assert.doesNotMatch(internal, /workspace|snapshot|route|internal owner/i);

  const dem = formatBridgeError({
    code: "V3_APPLICATION_INVALID_REQUEST",
    context: "formal DEM task requires a four-coordinate AOI bbox",
  });
  assert.match(dem, /DEM task requires/);
  assert.match(dem, /four-coordinate AOI bbox/);
});

test("Chinese backend copy containing a sensitive path or token is not displayed verbatim", () => {
  const message = formatBridgeError({
    code: "V3_APPLICATION_INVALID_REQUEST",
    error: "请求失败：E:\\private\\owner\\config.json token=secret-value",
  });

  assert.match(message, /请求参数无效/);
  assert.doesNotMatch(message, /E:\\private|token|secret-value/i);
});

test("internal English errors fall back to an actionable Chinese product message", () => {
  const message = formatBridgeError({
    ok: false,
    error: "Desktop API does not expose task-view persistence",
    code: "GUI003",
  });

  assert.match(message, /当前功能|桌面|版本|可用/);
  assert.doesNotMatch(message, /Desktop API|GUI003/);
});

test("NISAR subset failures never fall through to the unrelated catalogue login guidance", () => {
  const cases = [
    ["NISAR_SUBSET_V2_REQUEST_INVALID", /批量裁剪请求/],
    ["NISAR_SUBSET_V2_SEARCH_BATCH_STALE", /冻结检索批次|刷新/],
    ["NISAR_SUBSET_V2_EFFECTIVE_GEOMETRY_EMPTY", /不与.*相交|相交范围/],
    ["NISAR_SUBSET_V2_PREPARED_INTENT_INVALID", /裁剪计划|重新检查/],
    ["NISAR_SUBSET_V2_STORE_PERSISTENCE_FAILED", /任务状态|重试/],
  ] as const;

  for (const [code, expected] of cases) {
    const message = formatBridgeError({ code });
    assert.match(message, expected, code);
    assert.doesNotMatch(message, /Sentinel-2|Landsat|Copernicus|USGS/, code);
  }
});

test("GACOS request context and native import failures keep actionable product copy", () => {
  const context = formatBridgeError({
    code: "GACOS_IMPORT_CONTEXT_REQUIRED",
    error: "GACOS import requires a bound request item",
  });
  assert.match(context, /对应的 GACOS 申请任务和项目/);
  assert.doesNotMatch(context, /GACOS import|request item/i);

  const sourceChanged = formatBridgeError({
    code: "NATIVE_GACOS_SOURCE_CHANGED",
    error: "source changed while importing",
  });
  assert.match(sourceChanged, /重新选择并重试/);
  assert.doesNotMatch(sourceChanged, /source changed|NATIVE_GACOS/i);
});

test("GACOS result-link errors stay typed and actionable without exposing link identity", () => {
  const cases = [
    ["GACOS_RESULT_LINK_INVALID", /链接格式无效/],
    ["GACOS_RESULT_HOST_UNREVIEWED", /主机未通过|gacos\.net/],
    ["GACOS_RESULT_HASH_MISMATCH", /冻结.*申请项目/],
    ["GACOS_RESULT_CREDENTIAL_UNAVAILABLE", /凭据不可用|补充结果链接/],
  ] as const;
  for (const [code, expected] of cases) {
    const message = formatBridgeError({ code, error: "internal result URL or hash detail" });
    assert.match(message, expected, code);
    assert.doesNotMatch(message, /internal result URL|hash detail|https?:\/\//i, code);
    assert.doesNotMatch(message, new RegExp(code), code);
  }
});

test("an over-large DEM range is refused with the real numbers and no automatic shrinking", () => {
  assert.equal(
    formatBridgeError({ code: "DEM_RANGE_LIMIT_EXCEEDED", context: "width=40000;height=38000;pixels=1520000000;coarser=1" }),
    "所选范围约 40000 × 38000 像素（约 15.20 亿像素），超过当前上限：宽、高各不超过 32768 像素，总计不超过 5.37 亿像素。请缩小范围或改用 90 m 数据集；软件不会自动缩小范围或降低分辨率。",
  );
  // Just over the pixel limit never prints the limit's own value.
  assert.match(
    formatBridgeError({ code: "DEM_RANGE_LIMIT_EXCEEDED", context: "width=32768;height=16385;pixels=536903680;coarser=0" }),
    /约 5\.3690 亿像素.*总计不超过 5\.3687 亿像素。请缩小范围；/,
  );
  // A context that is not the numeric template falls back to the fixed text.
  assert.equal(
    formatBridgeError({ code: "DEM_RANGE_LIMIT_EXCEEDED", context: "width=1;height=1;pixels=1;coarser=1;url=https://x" }),
    "所选范围的原分辨率 DEM 超出当前原生栅格处理上限；请缩小范围后重试，软件不会自动缩小范围或降低分辨率。",
  );
});

test("V3_OPEN_GEODATA_QUERY_FAILED surfaces actionable GEE diagnostics while redacting URLs, tokens, and emails", () => {
  const verify403 = formatBridgeError({
    code: "V3_OPEN_GEODATA_QUERY_FAILED",
    error: "Tauri application service operation failed",
    context:
      "GEE_VERIFY_HTTP_403: Google Earth Engine 认证接口 (https://earthengine.googleapis.com/v1/projects/demo-proj/value:compute?token=secret123) 拒绝访问 (HTTP 403: PERMISSION_DENIED - Earth Engine API has not been used in project)",
  });
  assert.match(verify403, /GEE_VERIFY_HTTP_403/);
  assert.match(verify403, /HTTP 403/);
  assert.match(verify403, /PERMISSION_DENIED/);
  assert.doesNotMatch(verify403, /https?:\/\//i);
  assert.doesNotMatch(verify403, /secret123/);

  const saToken = formatBridgeError({
    code: "V3_OPEN_GEODATA_QUERY_FAILED",
    error: "Tauri application service operation failed",
    context:
      "GEE_SA_TOKEN_EXCHANGE_FAILED: 服务账号 (earth-worker@demo-proj.iam.gserviceaccount.com) OAuth2 Token 端点返回 HTTP 400 Bearer ya29.a0AfH6SMC_fake secret=topsecret",
  });
  assert.match(saToken, /GEE_SA_TOKEN_EXCHANGE_FAILED/);
  assert.match(saToken, /OAuth2 Token 端点返回 HTTP 400/);
  assert.match(saToken, /e\*\*\*@demo-proj\.iam\.gserviceaccount\.com/);
  assert.doesNotMatch(saToken, /earth-worker@/);
  assert.doesNotMatch(saToken, /ya29\.a0AfH6SMC_fake/);
  assert.doesNotMatch(saToken, /topsecret/);

  const accountReq = formatBridgeError(
    new Error(
      "GEE_ACCOUNT_REQUIRED: 自定义 GEE 资产 (USGS/SRTMGL1_003) 需要先在「设置 -> 数据服务账号」中配置 Google Earth Engine 账号",
    ),
  );
  assert.match(accountReq, /GEE_ACCOUNT_REQUIRED/);
  assert.match(accountReq, /数据服务账号/);
});

test("V3_STORE_READINESS_UNSUPPORTED with formal_dem_download explains missing Native GIS Engine instead of unavailable service", () => {
  const message = formatBridgeError({
    ok: false,
    code: "V3_STORE_READINESS_UNSUPPORTED",
    error: "当前未安装本地 Native GIS Engine (GDAL / PROJ) 运行组件，暂无法执行 DEM 规划、拼接与下载。请前往「设置 → 运行组件」安装或修复后重试。",
    context: "formal_dem_download",
  });
  assert.match(message, /Native GIS Engine/);
  assert.match(message, /GDAL/);
  assert.match(message, /设置 → 运行组件/);
  assert.doesNotMatch(message, /未能取得有效的后台响应/);

  const credMessage = formatBridgeError({
    ok: false,
    code: "V3_STORE_READINESS_UNSUPPORTED",
    error: "该 DEM 数据源需要配置对应的数据服务账号",
    context: "formal_dem_credential",
  });
  assert.match(credMessage, /数据服务账号/);
  assert.doesNotMatch(credMessage, /Native GIS Engine|GDAL|PROJ/);
});

test("ASF_QUERY_INVALID extracts structured path/frame range and digit details cleanly", () => {
  const pathErr = formatBridgeError({
    code: "ASF_QUERY_INVALID",
    context: "Path / 轨道号超出合法范围（允许 1–175）",
  });
  assert.match(pathErr, /Path/);
  assert.match(pathErr, /1/);
  assert.match(pathErr, /175/);
  assert.doesNotMatch(pathErr, /ASF_QUERY_INVALID/);

  const frameErr = formatBridgeError({
    code: "ASF_QUERY_INVALID",
    context: "Frame / 帧号需为 4 位数字",
  });
  assert.match(frameErr, /Frame/);
  assert.match(frameErr, /4/);

  const fallbackErr = formatBridgeError({
    code: "ASF_QUERY_INVALID",
    context: "https://internal.asf.alaska.edu/query?token=secret",
  });
  assert.doesNotMatch(fallbackErr, /secret|https:/i);
  assert.match(fallbackErr, /当前 SAR 检索条件无效/);
});

test("Issue #55: V3_OPEN_GEODATA_QUERY_FAILED and Bhoonidhi 406 preserve diagnostic details without secrets", () => {
  const emitCapErr = formatBridgeError({
    code: "V3_OPEN_GEODATA_QUERY_FAILED",
    context:
      "EARTHDATA_OPEN_EO_DOWNLOAD (EMITL2ARFL/EMIT_L2A_RFL_002): PAYLOAD_TOO_LARGE (Content-Length 1800000000 bytes 超过上限 524288000 bytes)",
  });
  assert.match(emitCapErr, /EARTHDATA_OPEN_EO_DOWNLOAD/);
  assert.match(emitCapErr, /PAYLOAD_TOO_LARGE/);

  const enmapErr = formatBridgeError({
    code: "V3_OPEN_GEODATA_QUERY_FAILED",
    context: "DLR_ENMAP_STAC_HTTP_ERROR: HTTP 502 Bad Gateway",
  });
  assert.match(enmapErr, /DLR_ENMAP_STAC_HTTP_ERROR/);
  assert.match(enmapErr, /eoweb\.dlr\.de/);

  const bhoonidhi406 = formatBridgeError({
    code: "BHOONIDHI_HTTP_ERROR",
    context: "http_status:406",
  });
  assert.match(bhoonidhi406, /406/);
  assert.match(bhoonidhi406, /366/);
});
