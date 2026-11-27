import type { Locale } from "../i18n/types.ts";

export const WORKBENCH_LOCALE_TEXT: Record<
  Locale,
  {
    modes: {
      search: string;
      demConvert: string;
      gee: string;
    };
    sourceDelivery: {
      title: string;
      s2CdseLabel: string;
      s2CdseSub: string;
      s2CogSub: string;
      s2GeeLabel: string;
      s2GeeSub: string;
      s2L1cCdseLabel: string;
      s2L1cCdseSub: string;
      s2L1cGeeSub: string;
      l89MpcSub: string;
      l89UsgsLabel: string;
      l89UsgsSub: string;
      l89GeeLabel: string;
      l89GeeSub: string;
      demAwsSub: string;
      demOtSub: string;
      demGeeLabel: string;
      demGeeSub: string;
    };
    demTable: {
      datumEgm2008: string;
      datumEgm96: string;
      datumEgm2008Bare: string;
      datumEgm96Bare: string;
      datumEgm96Surface: string;
      datumWgs84Ellipsoid: string;
      channelAws: string;
      channelOt: string;
      channelFabdem: string;
      channelGem15: string;
      channelAsf: string;
      channelEarthdata: string;
      sourceDatumLabel: string;
      geoidModelLabel: string;
      channelLabel: string;
      autoConvertHint: string;
      estimatedTilesCount: string;
      bindAoiFirst: string;
      calculating: string;
      previewPlanBtn: string;
      boundsPrefix: string;
      datumPlanPrefix: string;
      mosaicArtifactPrefix: string;
    };
    sar: {
      processingLevel: string;
      s2L2aTitle: string;
      s2L1cTitle: string;
      s1RetiredHint: string;
      s1RecentSwitchHint: string;
      nisarLevelTitle: string;
      onlyLBandNote: string;
      releaseTierTitle: string;
      validatedTitle: string;
      provisionalTitle: string;
      sBandLevelsNote: string;
      nisarSBandConnected: string;
      nisarLBandConnected: string;
      nisarLBandRslcOnly: string;
      polLabel: string;
      polClearDefault: string;
      iwDualPolHint: string;
      includeVvVhBtn: string;
      subsetBtnLabel: string;
      subsetTooltipReady: string;
      subsetTooltipEmpty: string;
      completeDownloadBtn: string;
      capTruncatedNotice: string;
    };
    orbitMode: {
      modeLabel: string;
      byAoiBtn: string;
      localFolderBtn: string;
      localInputPlaceholder: string;
      pickScriptTitle: string;
      pickScriptFilter: string;
      pickScriptBtnTitle: string;
      scriptBtn: string;
      localScanDesc: string;
      aoiQueryDesc: string;
      batchCandidateCount: string;
      orbitActionHint: string;
      matchSourceLabel: string;
      localSlcFolderOpt: string;
      currentResultsOpt: string;
      localSlcDirectoryLabel: string;
      localSlcPlaceholder: string;
    };
    convMode: {
      datumFallbackNote: string;
      formatSupportNote: string;
      sourceDatumLabel: string;
      activeDatumPrefix: string;
      autoOption: string;
      egm2008Option: string;
      egm96Option: string;
      ellipsoidOption: string;
      targetFormatLabel: string;
      formatSarscapeTitle: string;
      formatSarscapeSub: string;
      formatTifTitle: string;
      formatTifSub: string;
      outputDirLabel: string;
      outputDirPlaceholder: string;
      pickDirTitle: string;
      resultHeading: string;
      previewHeading: string;
      identifying: string;
      previewBtn: string;
      writtenBadge: string;
      readyBadge: string;
      datumConversionPrefix: string;
      builtinGeoidNote: string;
      ellipsoidTifPrefix: string;
    };
    gacos: {
      bindAoiNotice: string;
      spanPrefix: string;
      spanTooLarge: string;
      spanTooSmall: string;
      spanValid: string;
      datesHeading: string;
      batchCapBadge: string;
      pickDateAria: string;
      addDateBtn: string;
      pathAria: string;
      pathPlaceholder: string;
      pickSlcDirTitle: string;
      dirBtnTitle: string;
      dirBtn: string;
      pickScriptFilter: string;
      pickScriptTitle: string;
      scriptBtn: string;
      scanFailed: string;
      noYmdFound: string;
      extractedDatesToast: string;
      scanningBtn: string;
      extractDatesBtn: string;
      textareaAria: string;
      textareaPlaceholder: string;
      invalidDateFmt: string;
      validDatesSummary: string;
      splitBatchesNote: string;
      autoSplitNote: string;
      utcHourLabel: string;
      utcHourAria: string;
      utcHourUnit: string;
      utcMinLabel: string;
      utcMinAria: string;
      utcMinUnit: string;
      formatLabel: string;
      formatAria: string;
      asyncEmailNotice: string;
      batchPreviewHeading: string;
      syncSubmitLabel: string;
      footerCountLabel: string;
    };
    xyz: {
      sourcePickerHint: string;
      sourcePickerAria: string;
      customLabel: string;
      customUrlLabel: string;
      customUrlPlaceholder: string;
      placeholderHelp: string;
      crsLabel: string;
      crsAuto: string;
      crsWgs84: string;
      crsGcj02: string;
      crsAria: string;
      zoomHeading: string;
      selectedPrefix: string;
      noneSelected: string;
      zoomUnsupportedTooltip: string;
      clearBtn: string;
      zoomCappedTooltip: string;
      estimateHeading: string;
      zoomCount: string;
      selectAoiFirst: string;
      selectZoomFirst: string;
      tileCountCumulative: string;
      tileCountSingle: string;
      perPeriodCap: string;
      gridSizeLabel: string;
      estTrafficLabel: string;
      estFileSizeLabel: string;
      estMemoryLabel: string;
      streamingMemValue: string;
      bigTiffNote: string;
      cogStagingSuffix: string;
      limitExceededErr: string;
      jpegLimitErr: string;
      zoomSupportNote: string;
      exportFormatTitle: string;
      exportFormatLabel: string;
      exportFormatAria: string;
      stitchToggleTitle: string;
      stitchToggleDesc: string;
      compressionTitle: string;
      compressionLabel: string;
      compressionAria: string;
      buildOverviewsLabel: string;
      downloadPeriodsBtn: string;
      downloadMosaicBtn: string;
    };
    geeInline: {
      allCatalogOption: string;
      themeCatalogOption: string;
      customAssetSuffix: string;
      publicCatalogHeading: string;
      selectAssetAria: string;
      customAssetLabel: string;
      customAssetPlaceholder: string;
      customAssetNote: string;
      allCatalogSummarySuffix: string;
      defaultBandsOption: string;
      maxCloudLabel: string;
      polarizationLabel: string;
      polarizationAria: string;
      temporalReducerLabel: string;
      temporalReducerAria: string;
      staticAssetNote: string;
      targetBandLabel: string;
      targetBandAria: string;
      openGeeWorkbenchBtn: string;
      polAll: string;
      polVv: string;
      polVh: string;
      polHh: string;
      polHv: string;
      reducerMedian: string;
      reducerMean: string;
      reducerMax: string;
      reducerMin: string;
      cloudOpt10: string;
      cloudOpt20: string;
      cloudOpt30: string;
      cloudOpt50: string;
      cloudOpt80: string;
      cloudOpt100: string;
    };
    resultsExtra: {
      showPreviewTitle: string;
      hidePreviewTitle: string;
      mapPreviewBadge: string;
      mapPreviewHiddenBadge: string;
      clearPreviewsAria: string;
      clearResultsAria: string;
      clearBtn: string;
      listLoadFailedTitle: string;
      listLoadFailedDesc: string;
      searchingTitle: string;
      searchIncompleteTitle: string;
      searchIncompleteFallback: string;
      retryBtn: string;
      noMatchTitle: string;
      noMatchDesc: string;
      resetFilterBtn: string;
      emptyLocatedAoi: string;
      emptySelectedAoi: string;
      emptySearchTitle: string;
      emptySearchWithAoiDesc: string;
      emptySearchNoAoiDesc: string;
      startSearchBtn: string;
      drawAoiBtn: string;
    };
    bottomSummary: {
      localSlcFolder: string;
      chooseLocalSlc: string;
      restrictedAoi: string;
      customAoiName: string;
      globalOrView: string;
      gacosSummary: string;
      approxDemTiles: string;
      elapsedSeconds: string;
      cancelling: string;
      abort: string;
      deriveRegisteredNote: string;
      enmapSearchOnlyNotice: string;
    };
    categoryBadge: {
      sar: string;
      orbit: string;
      convert: string;
      atmo: string;
      geodesy: string;
      thematic: string;
      optical: string;
      aux: string;
    };
    topicOptions: {
      productVersionLabel: string;
      productVersionAria: string;
      wcV200: string;
      wcV100: string;
      waterLayerLabel: string;
      waterLayerAria: string;
      jrcOccurrence: string;
      jrcExtent: string;
      jrcSeasonality: string;
      jrcChange: string;
      alosBandLabel: string;
      alosBandAria: string;
      alosHh: string;
      alosHv: string;
      alosMask: string;
      modisBandLabel: string;
      modisBandAria: string;
      modisNdvi: string;
      modisEvi: string;
      modisQa: string;
      s5pLabel: string;
      s5pAria: string;
      s5pNo2: string;
      s5pCh4: string;
      s5pSo2: string;
      s5pCo: string;
      s5pO3: string;
      s5pAer: string;
      operaLabel: string;
      operaAria: string;
      operaDisp: string;
      operaCslc: string;
      operaRtc: string;
      era5Label: string;
      era5Aria: string;
      era5HourlyInsar: string;
      era5HourlyFull: string;
      era5Daily: string;
      eqLabel: string;
      eqAria: string;
      eqMw3: string;
      eqMw45: string;
      eqMw55: string;
      eqMw65: string;
      igsLabel: string;
      igsAria: string;
      igsCod: string;
      igsIgs: string;
      igsJpl: string;
      nisarOeLabel: string;
      nisarOeAria: string;
      nisarOeAll: string;
      nisarOePoe: string;
      nisarOeMoe: string;
      nisarOeNoe: string;
      nisarOeFoe: string;
      nisarOeHint: string;
      podLabel: string;
      podAria: string;
      podAll: string;
      podHint: string;
      bmLabel: string;
      bmAria: string;
      bmA3: string;
      bmA4: string;
      bmA2: string;
      emitLabel: string;
      emitAria: string;
      emitL2a: string;
      emitL2b: string;
      ecoLabel: string;
      ecoAria: string;
      ecoLst: string;
      ecoEmis: string;
      ecoEt: string;
      enmapNotePrefix: string;
      enmapNoteSuffix: string;
      s1GrdLabel: string;
      s1GrdAria: string;
      s1GrdVv: string;
      s1GrdVh: string;
      s1GrdHh: string;
      s1GrdHv: string;
      noaaLabel: string;
      noaaAria: string;
      noaaGfs: string;
      noaaGoes19: string;
      noaaGoes18: string;
      overtureLabel: string;
      overtureAria: string;
      ovBuildings: string;
      ovTransport: string;
      ovPlaces: string;
      ovDivisions: string;
      ovAddresses: string;
      ovBase: string;
    };
  }
> = {
  "zh-CN": {
    modes: {
      search: "数据检索",
      demConvert: "DEM 转换",
      gee: "GEE 云端计算与制图",
    },
    sourceDelivery: {
      title: "数据来源与格式",
      s2CdseLabel: "ESA CDSE (官方原始 SAFE 包)",
      s2CdseSub: "欧空局官方原档 · 适于几何校正与科研 · 需 CDSE 账号",
      s2CogSub: "微软行星计算机与 AWS Earth Search 云优化单波段 COG · 免账号",
      s2GeeLabel: "Google Earth Engine (裁剪 GeoTIFF)",
      s2GeeSub: "云原生按 AOI 裁剪 · COPERNICUS/S2_SR_HARMONIZED",
      s2L1cCdseLabel: "ESA CDSE (官方原始 SAFE 包)",
      s2L1cCdseSub: "欧空局官方公开目录检索 · 下载需 CDSE 账号",
      s2L1cGeeSub: "云原生按 AOI 裁剪 · COPERNICUS/S2_HARMONIZED",
      l89MpcSub: "微软行星计算机单波段 COG 展开集 · 免账号",
      l89UsgsLabel: "USGS M2M (官方原始 .tar)",
      l89UsgsSub: "公开目录统一检索 · 下载交付 USGS 官方完整 .tar（需 USGS 账号）",
      l89GeeLabel: "Google Earth Engine (裁剪 GeoTIFF)",
      l89GeeSub: "云端按 AOI 裁剪地表反射率 · LANDSAT/LC09/C02/T1_L2",
      demAwsSub: "亚马逊公开数据 GLO-30 原始瓦片 · 免账号",
      demOtSub: "按范围聚合 GeoTIFF · 需 API Key",
      demGeeLabel: "Google Earth Engine (高程裁剪)",
      demGeeSub: "云端按 AOI 裁剪高程 · COPERNICUS/DEM/GLO30",
    },
    demTable: {
      datumEgm2008: "EGM2008 正高",
      datumEgm96: "EGM96 正高",
      datumEgm2008Bare: "EGM2008 裸地正高",
      datumEgm96Bare: "EGM96 裸地正高",
      datumEgm96Surface: "EGM96 地表正高",
      datumWgs84Ellipsoid: "WGS84 椭球高",
      channelAws: "AWS 公开桶 · 免账号直连",
      channelOt: "OpenTopography · 需 API Key",
      channelFabdem: "公开 COG（CC BY-NC-SA 4.0 非商用）",
      channelGem15: "Google Cloud 公开 COG（CC BY 4.0 · 不含中国）",
      channelAsf: "ASF Earthdata · 需账号",
      channelEarthdata: "NASA LP DAAC · 需 Earthdata 账号",
      sourceDatumLabel: "原始垂直基准：",
      geoidModelLabel: "水准面模型：",
      channelLabel: "分发通道：",
      autoConvertHint: "下载拼接后自动执行 {geoid} 正高 → WGS84 椭球高转换，并生成 SARscape 标准 (_dem + .hdr + .sml) 与椭球高 GeoTIFF。",
      estimatedTilesCount: "预计覆盖 {count} 块 1°×1° 瓦片",
      bindAoiFirst: "请先在上方绑定研究区 AOI",
      calculating: "计算中…",
      previewPlanBtn: "预览瓦片方案",
      boundsPrefix: "范围：",
      datumPlanPrefix: "基准方案：",
      mosaicArtifactPrefix: "拼接产物：",
    },
    sar: {
      processingLevel: "处理级别 (Processing Level)",
      s2L2aTitle: "L2A地表反射率",
      s2L1cTitle: "L1C 表观反射率",
      s1RetiredHint: "S1B 已于 2021 年 12 月停止成像，近期时间段建议同时勾选 S1A / S1C / S1D 或保持全不选（默认全选）。",
      s1RecentSwitchHint: "提示：近期轨道多由 S1C / S1D 接替成像，若结果较少可勾选 S1C / S1D 或切换为「近1年」。",
      nisarLevelTitle: "产品级别 (Product Level)",
      onlyLBandNote: " — 该级别仅在 L-Band 提供",
      releaseTierTitle: "发布版本 (Release Tier)",
      validatedTitle: "Validated：NASA / ASF 官方校验通过的正式科学发布版本",
      provisionalTitle: "Provisional：NASA / ASF 早期试运行与定标验证期发布的暂定版本",
      sBandLevelsNote: "S-Band 当前开放 RSLC / GSLC / GCOV / GUNW / GOFF / RIFG / RUNW / ROFF 8 级官方产品；SME2（土壤湿度）仅在 L-Band 提供。",
      nisarSBandConnected: "：已连接 ISRO Bhoonidhi 实名检索，支持完整 .h5 下载与基于 HDF5 HTTP Range 的在线 AOI 裁剪。",
      nisarLBandConnected: "：已启用 ASF CMR 检索、完整 .h5 下载与基于 HDF5 HTTP Range 的在线 AOI 裁剪。",
      nisarLBandRslcOnly: "L-Band RSLC 支持 ASF 检索与基于 HDF5 HTTP Range 的在线 AOI 裁剪。",
      polLabel: "极化方式 (Polarization)",
      polClearDefault: "清空 (默认全选)",
      iwDualPolHint: "陆地 IW 场景多为双极化产品 (VV+VH)",
      includeVvVhBtn: "+ 包含 VV+VH",
      subsetBtnLabel: "AOI 裁剪下载",
      subsetTooltipReady: "打开 NISAR AOI 裁剪下载工作区（选择/绘制裁剪区域、Frequency 与极化）",
      subsetTooltipEmpty: "请先勾选至少一景 NISAR 影像以进行 AOI 裁剪下载",
      completeDownloadBtn: "完整产品下载",
      capTruncatedNotice: "当前展示最新 2,000 景（首批 250 景即时呈递，已达单次检索后台加载上限），缩短时间或缩小范围可获取更早影像",
    },
    orbitMode: {
      modeLabel: "轨道获取方式",
      byAoiBtn: "按 AOI/日期主动下载",
      localFolderBtn: "匹配本地影像文件夹轨道",
      localInputPlaceholder: "SLC 目录或 ASF .py 脚本路径",
      pickScriptTitle: "选择 ASF Sentinel-1 下载脚本或清单",
      pickScriptFilter: "ASF 脚本与清单 (*.py;*.metalink;*.meta4;*.txt;*.urls;*.csv;*.geojson;*.json)",
      pickScriptBtnTitle: "选择 ASF 下载脚本或清单文件（.py / .metalink / .txt 等）",
      scriptBtn: "脚本",
      localScanDesc: "自动扫描本地目录或 ASF 下载脚本（.py / .metalink / .txt 等）内 S1A/S1B/S1C/S1D 影像与时刻，批量匹配并补齐缺失的 .EOF 精密轨道。",
      aoiQueryDesc: "支持按当前 AOI 与时间窗主动查询下载精密轨道（也可在 Sentinel-1 检索下载时勾选「精密轨道」自动随附）。",
      batchCandidateCount: "当前批次共 {count} 景候选场景",
      orbitActionHint: "若已在右侧列表勾选场景，优先为已勾选场景下载对应精密轨道（POEORB 优先，未发布时自动回退 RESORB）。",
      matchSourceLabel: "匹配来源",
      localSlcFolderOpt: "本地 SLC 影像目录",
      currentResultsOpt: "当前检索结果",
      localSlcDirectoryLabel: "本地 SLC 目录",
      localSlcPlaceholder: "请选择本地 Sentinel-1 目录",
    },
    convMode: {
      datumFallbackNote: "（文件名未含基准标签，默认按 EGM2008 识别，可手动切换）",
      formatSupportNote: "支持选择本地 .tif / .tiff / .hgt / .dem 高程文件，内置大地水准面差距模型离线计算。",
      sourceDatumLabel: "源 DEM 垂直高程基准",
      activeDatumPrefix: "当前生效：{datum}",
      autoOption: "自动识别 (Copernicus→EGM2008 / SRTM→EGM96)",
      egm2008Option: "EGM2008 正高 (Copernicus DEM 30m/90m)",
      egm96Option: "EGM96 正高 (SRTM / NASADEM / AW3D30)",
      ellipsoidOption: "已是 WGS84 椭球高 (仅导出 SARscape 格式)",
      targetFormatLabel: "目标输出产物格式",
      formatSarscapeTitle: "SARscape + 椭球高 TIF",
      formatSarscapeSub: "生成 _dem/.hdr/.sml 与 _ellipsoid.tif",
      formatTifTitle: "仅 WGS84 椭球高 TIF",
      formatTifSub: "生成标准 GeoTIFF (_ellipsoid.tif)",
      outputDirLabel: "输出目录（留空默认输出至原文件所在目录）",
      outputDirPlaceholder: "默认与输入 DEM 同目录",
      pickDirTitle: "选择高程转换输出目录",
      resultHeading: "高程转换结果",
      previewHeading: "转换方案预览",
      identifying: "识别中…",
      previewBtn: "预览方案",
      writtenBadge: "已落盘",
      readyBadge: "预检就绪",
      datumConversionPrefix: "基准转换：",
      builtinGeoidNote: "（内置 {model} 水准面模型）",
      ellipsoidTifPrefix: "椭球高 TIF：",
    },
    gacos: {
      bindAoiNotice: "请先设定研究区矩形/边界 (GACOS 单次申请经纬度跨度须在 0.05°–10° 之间)",
      spanPrefix: "跨度 {lon}° × {lat}°",
      spanTooLarge: "超出 GACOS 官方单次最大范围限制（经纬度跨度均须 ≤ 10°），请缩小研究区",
      spanTooSmall: "范围过小（经纬度跨度建议 ≥ 0.05° 且 ≤ 10°）",
      spanValid: "符合 GACOS 范围限制 (0.05°–10°)",
      datesHeading: "影像日期列表（8 位 YYYYMMDD）",
      batchCapBadge: "单批上限 20 个",
      pickDateAria: "选择要添加的 GACOS 日期",
      addDateBtn: "+ 添加",
      pathAria: "本地 SLC 目录或 ASF 脚本路径",
      pathPlaceholder: "目录或 ASF .py 脚本路径",
      pickSlcDirTitle: "选择包含 SLC 影像的目录",
      dirBtnTitle: "浏览本地 SLC 目录",
      dirBtn: "目录",
      pickScriptFilter: "ASF 脚本与清单 (*.py;*.metalink;*.meta4;*.txt;*.urls;*.csv;*.geojson;*.json)",
      pickScriptTitle: "选择 ASF 下载脚本或清单文件",
      scriptBtn: "脚本",
      scanFailed: "未能扫描该目录下的 SLC 影像日期",
      noYmdFound: "未在该目录中识别出含 8 位日期 (YYYYMMDD) 的 SLC 文件名",
      extractedDatesToast: "已提取 {count} 个影像日期",
      scanningBtn: "识别中...",
      extractDatesBtn: "提取日期",
      textareaAria: "GACOS 8 位 YYYYMMDD 日期列表",
      textareaPlaceholder: "每行输入一个 8 位日期 YYYYMMDD，例如：\n20260501\n20260513",
      invalidDateFmt: "存在无效日期格式：{tokens}（须为 8 位有效公历日期 YYYYMMDD）",
      validDatesSummary: "有效日期 {count} 个（单批上限 20 个；{suffix}）",
      splitBatchesNote: "将自动拆分为 {count} 批提交",
      autoSplitNote: "超出自动按每批 20 个拆分",
      utcHourLabel: "UTC 小时 (00–23)",
      utcHourAria: "UTC 小时",
      utcHourUnit: "{h} 时",
      utcMinLabel: "UTC 分钟 (00–59)",
      utcMinAria: "UTC 分钟",
      utcMinUnit: "{m} 分",
      formatLabel: "GACOS 数据格式",
      formatAria: "GACOS 数据格式",
      asyncEmailNotice: "说明：GACOS 为官方邮件队列异步分发模式，提交后由 GACOS 服务器推送下载链接至配置邮箱，无需预设本地保存目录。",
      batchPreviewHeading: "GACOS 批量申请预览",
      syncSubmitLabel: "GACOS 同步申请",
      footerCountLabel: "GACOS {count} 次申请",
    },
    xyz: {
      sourcePickerHint: "图源选择（内置 15 种 · 含 Wayback 历史存档）",
      sourcePickerAria: "图源选择",
      customLabel: "自定义图源 · XYZ / TMS 瓦片服务模板",
      customUrlLabel: "自定义 XYZ / TMS 瓦片 URL 模板",
      customUrlPlaceholder: "如 https://mt1.google.com/vt/lyrs=s&x={x}&y={y}&z={z}",
      placeholderHelp: "支持占位符：",
      crsLabel: "图源坐标系与偏移纠偏",
      crsAuto: "自动识别 (含 autonavi/amap 自动按 GCJ-02 纠偏)",
      crsWgs84: "WGS84 / CGCS2000 国际标准坐标 (无偏移)",
      crsGcj02: "GCJ-02 国测局火星坐标 (自动反算纠偏至 WGS84)",
      crsAria: "图源坐标系",
      zoomHeading: "瓦片缩放层级 (当前图源支持 Z1–Z{maxZoom} 多选)",
      selectedPrefix: "已选 {zooms}",
      noneSelected: "未选择",
      zoomUnsupportedTooltip: "当前图源最高仅支持 Z{maxZoom}，不可一键选至 Z{presetMax}",
      clearBtn: "清空",
      zoomCappedTooltip: "当前图源原生最高仅支持 Z{maxZoom}（不可下载 Z{z}）",
      estimateHeading: "瓦片体积预估",
      zoomCount: "{count} 个层级",
      selectAoiFirst: "请先在地图或行政区划中选定研究区 AOI，即可实时估算瓦片数与体积。",
      selectZoomFirst: "请至少选择一个缩放层级 (Z1–Z{maxZoom})。",
      tileCountCumulative: "瓦片数 ({count} 期累计)",
      tileCountSingle: "瓦片数",
      perPeriodCap: "{shown} / 单期上限 {cap}",
      gridSizeLabel: "网格 W × H",
      estTrafficLabel: "预计下载流量",
      estFileSizeLabel: "预计输出文件大小",
      estMemoryLabel: "预计峰值内存",
      streamingMemValue: "约 {mem} · 流式写入",
      bigTiffNote: "引擎按条带流式写入并启用 BigTIFF；单幅栅格拼接与 VRT 因需解码全幅像素并控制句柄/IPC 负载，单次上限为 50,000 张；MBTiles 或取消栅格拼接的纯瓦片包上限为 200,000 张{cog}",
      cogStagingSuffix: "（COG 暂存优先使用 SSD 临时工作目录，避免占满系统盘）",
      limitExceededErr: "超出当前导出模式上限：单期共需 {total} 张瓦片，最大允许 {cap} 张，已禁止创建任务。请缩小研究区、降低缩放层级，或切换至 MBTiles / 取消栅格拼接。",
      jpegLimitErr: "超出 JPEG 格式单边 65,535 px 极限（当前为 {w} × {h} px），已禁止创建任务。请缩小研究区或改选 GeoTIFF / COG / MBTiles。",
      zoomSupportNote: "当前图源支持 Z1–Z{maxZoom}；高于上限的层级已自动禁用。若高倍率局部区域无原生切片，支持范围内的父级高清瓦片会自动回退裁剪补全。",
      exportFormatTitle: "导出栅格格式：{label}",
      exportFormatLabel: "导出栅格 / 归档格式",
      exportFormatAria: "导出栅格格式",
      stitchToggleTitle: "自动无缝拼接为单张带坐标栅格大图",
      stitchToggleDesc: "勾选则调用内置 GDAL 引擎生成带 EPSG:3857 坐标配准的整幅大图（单次上限 50,000 张）；取消勾选则仅下载原始瓦片或纯 ZIP 归档（上限 200,000 张）。",
      compressionTitle: "压缩算法：{label}",
      compressionLabel: "压缩算法",
      compressionAria: "压缩算法",
      buildOverviewsLabel: "构建内嵌多级金字塔 (2x / 4x / 8x / 16x Overviews，加速 GIS 缩放)",
      downloadPeriodsBtn: "批量下载 {count} 期历史影像与拼接大图",
      downloadMosaicBtn: "下载配准瓦片与拼接大图",
    },
    geeInline: {
      allCatalogOption: "全部公共遥感目录（默认全选 · {total} 项）",
      themeCatalogOption: "当前专题全部公共数据集（默认全选 · {count} 项）",
      customAssetSuffix: "{id} · 自定义 GEE 资产",
      publicCatalogHeading: "GEE 公共遥感目录 ({shown}/{total})",
      selectAssetAria: "选择 GEE 公共数据集",
      customAssetLabel: "高阶选项：手动填写自有/私有 Asset ID",
      customAssetPlaceholder: "如 COPERNICUS/S2_SR_HARMONIZED 或 projects/...",
      customAssetNote: "默认直接列出全部公共数据集；配置 GEE 云项目凭据后可下载自有 projects/... 资产。",
      allCatalogSummarySuffix: "全部公共遥感目录 (默认全选 · {total} 项) · 按 AOI 在线裁剪",
      defaultBandsOption: "默认推荐波段",
      maxCloudLabel: "最大云量上限",
      polarizationLabel: "极化通道 (Polarization)",
      polarizationAria: "极化通道",
      temporalReducerLabel: "时间聚合统计方法",
      temporalReducerAria: "时间聚合统计方法",
      staticAssetNote: "静态无时相基准数据，已自动隐藏日期与云量过滤，按 AOI 范围检索并裁剪导出。",
      targetBandLabel: "目标波段 / 物理变量",
      targetBandAria: "目标波段或物理变量",
      openGeeWorkbenchBtn: "前往 GEE 云端计算与制图 →",
      polAll: "全部极化 (VV / VH / HH / HV)",
      polVv: "VV 同极化 (Sentinel-1)",
      polVh: "VH 交叉极化 (Sentinel-1)",
      polHh: "HH 同极化 (ALOS PALSAR)",
      polHv: "HV 交叉极化 (ALOS PALSAR)",
      reducerMedian: "中值合成 (Median · 抗异常值)",
      reducerMean: "均值统计 (Mean · 平均状态)",
      reducerMax: "最大值合成 (Max · 峰值极端)",
      reducerMin: "最小值合成 (Min · 谷值基线)",
      cloudOpt10: "<= 10% (极高晴空)",
      cloudOpt20: "<= 20% (少云)",
      cloudOpt30: "<= 30% (推荐默认)",
      cloudOpt50: "<= 50% (适中)",
      cloudOpt80: "<= 80% (宽松)",
      cloudOpt100: "不限制云量 (100%)",
    },
    resultsExtra: {
      showPreviewTitle: "显示地图影像框内快视图",
      hidePreviewTitle: "隐藏地图影像框内快视图",
      mapPreviewBadge: "地图预览 ({count})",
      mapPreviewHiddenBadge: "预览已隐藏 ({count})",
      clearPreviewsAria: "清空全部地图预览图层",
      clearResultsAria: "清空检索结果",
      clearBtn: "清空",
      listLoadFailedTitle: "影像列表加载失败",
      listLoadFailedDesc: "当前目录读取未能完成，已设定的研究区和条件均未丢失。",
      searchingTitle: "正在检索…",
      searchIncompleteTitle: "影像检索未能完成",
      searchIncompleteFallback: "与数据源连接中断，已设定的研究区和条件均未丢失。",
      retryBtn: "重试",
      noMatchTitle: "当前条件下未找到匹配影像",
      noMatchDesc: "可尝试放宽云量上限、扩大时间跨度或切换数据源。",
      resetFilterBtn: "重置筛选条件",
      emptyLocatedAoi: "已定位 · {label}",
      emptySelectedAoi: "已选定研究区",
      emptySearchTitle: "尚未开始影像检索",
      emptySearchWithAoiDesc: "确认左侧传感器与时间条件后，即可获取可用影像。",
      emptySearchNoAoiDesc: "先在地图选定研究区并设置传感器与时间，即可获取可用影像。",
      startSearchBtn: "开始检索",
      drawAoiBtn: "框选研究区",
    },
    bottomSummary: {
      localSlcFolder: "本地影像目录：{name}",
      chooseLocalSlc: "本地扫描：请选择 Sentinel-1 影像目录",
      restrictedAoi: "已限定：{aoi}",
      customAoiName: "自定义范围",
      globalOrView: "空间范围：全球或当前视图",
      gacosSummary: "{dates} 个日期 · {h}:{m} UTC · {fmt}",
      approxDemTiles: "约 {count} 块瓦片",
      elapsedSeconds: "{label} · 已用时 {sec}s",
      cancelling: "正在中止",
      abort: "中止",
      deriveRegisteredNote: "已配准/地理编码的标准干涉成品，已内含几何配准结果，无需额外下载或等待原始 SLC 精密轨道。",
      enmapSearchOnlyNotice: "DLR EnMAP L2A 仅支持公开目录检索；原始影像归档需前往 https://eoweb.dlr.de 登录下载。",
    },
    categoryBadge: {
      sar: "雷达 SAR",
      orbit: "轨道",
      convert: "高程转换",
      atmo: "大气",
      geodesy: "形变/事件",
      thematic: "专题/瓦片",
      optical: "光学",
      aux: "辅助",
    },
    topicOptions: {
      productVersionLabel: "产品版本选择",
      productVersionAria: "WorldCover 产品版本",
      wcV200: "WorldCover v200 (2021 最新版 · 10m)",
      wcV100: "WorldCover v100 (2020 基准版 · 10m)",
      waterLayerLabel: "水体专题图层 (Asset Layer)",
      waterLayerAria: "JRC 水体专题图层",
      jrcOccurrence: "occurrence · 历史水体出现频率 (0–100%)",
      jrcExtent: "extent · 最大水体覆盖范围掩膜",
      jrcSeasonality: "seasonality · 永久水体与季节性水体分类",
      jrcChange: "change · 水体出现频率多年变化强度",
      alosBandLabel: "极化与掩膜波段",
      alosBandAria: "ALOS PALSAR 极化波段",
      alosHh: "HH · 水平同极化后向散射栅格 (.tif)",
      alosHv: "HV · 交叉极化后向散射栅格 (.tif)",
      alosMask: "mask · 雷达叠掩/阴影/水体几何掩膜 (.tif)",
      modisBandLabel: "植被指数波段",
      modisBandAria: "MODIS 植被指数波段",
      modisNdvi: "250m_16_days_NDVI · 归一化植被指数 (NDVI)",
      modisEvi: "250m_16_days_EVI · 增强型植被指数 (EVI)",
      modisQa: "250m_16_days_VI_Quality · 质量控制掩膜",
      s5pLabel: "大气痕量气体产品 (TROPOMI L2)",
      s5pAria: "Sentinel-5P 气体产品",
      s5pNo2: "NO2 · 二氧化氮对流层柱浓度",
      s5pCh4: "CH4 · 甲烷温室气体干空气混合比",
      s5pSo2: "SO2 · 二氧化硫柱浓度 (火山监测)",
      s5pCo: "CO · 一氧化碳总柱浓度",
      s5pO3: "O3 · 臭氧总柱浓度",
      s5pAer: "AER_AI · 紫外气溶胶指数",
      operaLabel: "OPERA 产品子类型",
      operaAria: "OPERA 产品子类型",
      operaDisp: "OPERA_L3_DISP-S1_V1 · 地表视线向位移时间序列",
      operaCslc: "OPERA_L2_CSLC-S1_V1 · 地理编码配准单视复数影像 (CSLC)",
      operaRtc: "OPERA_L2_RTC-S1_V1 · 地形辐射校正后向散射 (RTC)",
      era5Label: "重分析变量集 (Variables)",
      era5Aria: "ERA5 重分析变量集",
      era5HourlyInsar: "逐小时对流层与温湿压参数 (InSAR 改正 · 推荐)",
      era5HourlyFull: "全量逐小时气象再分析格网 (温湿压风云降水)",
      era5Daily: "逐日综合统计再分析",
      eqLabel: "最小矩震级阈值 (Mw)",
      eqAria: "最小矩震级阈值",
      eqMw3: "Mw ≥ 3.0 (有感微震与小震)",
      eqMw45: "Mw ≥ 4.5 (中等及以上地震 · 推荐)",
      eqMw55: "Mw ≥ 5.5 (显著形变同震事件)",
      eqMw65: "Mw ≥ 6.5 (强震与大地震)",
      igsLabel: "电离层分析中心 (Analysis Center)",
      igsAria: "电离层分析中心",
      igsCod: "COD · 瑞士伯尔尼大学 CODE 最终解",
      igsIgs: "IGS · 国际 GNSS 服务组织综合解",
      igsJpl: "JPL · NASA 喷气推进实验室解",
      nisarOeLabel: "NISAR 轨道历书等级 (NISAR_OE)",
      nisarOeAria: "NISAR 轨道历书等级",
      nisarOeAll: "ALL · 全部轨道历书 (POE/MOE/NOE/FOE)",
      nisarOePoe: "POE · 精密定轨星历 (Science 最高精度)",
      nisarOeMoe: "MOE · 中精度定轨星历",
      nisarOeNoe: "NOE · 标称实时定轨星历",
      nisarOeFoe: "FOE · 预报定轨星历",
      nisarOeHint: "全球全轨状态向量 (.xml)，无需框选地图范围，按起止日期检索；下载需 NASA Earthdata 账号。",
      podLabel: "哥白尼定轨任务族 (CDSE POD)",
      podAria: "哥白尼精密定轨任务族",
      podAll: "ALL · Sentinel-2 + Sentinel-3 全部精密轨道",
      podHint: "全球全轨精密星历，无需框选地图范围，按起止日期检索；下载需 CDSE 账号。",
      bmLabel: "Black Marble 夜光产品周期 (500m HDF5)",
      bmAria: "Black Marble 夜光产品周期",
      bmA3: "VNP46A3 · 月度无云合成夜光辐射度（推荐）",
      bmA4: "VNP46A4 · 年度无云合成夜光辐射度",
      bmA2: "VNP46A2 · 逐日月光/BRDF 校正夜光（震后停电评估）",
      emitLabel: "EMIT 高光谱科学级别 (60m · 285 波段)",
      emitAria: "EMIT 高光谱科学级别",
      emitL2a: "EMITL2ARFL · L2A 地表反射率与不确定性 (.nc)",
      emitL2b: "EMITL2BMIN · L2B 矿物识别与丰度反演 (.nc)",
      ecoLabel: "ECOSTRESS 热红外与蒸散发图层 (70m COG)",
      ecoAria: "ECOSTRESS 热红外图层",
      ecoLst: "ECO_L2T_LSTE · L2T 地表温度 LST (_LST.tif · 推荐)",
      ecoEmis: "ECO_L2T_LSTE · L2T 宽波段地表发射率 (_EmisWB.tif)",
      ecoEt: "ECO_L3T_JET · L3T JPL 日蒸散发量 (_ETdaily.tif)",
      enmapNotePrefix: "德国宇航中心 EnMAP L2A 公开 STAC 目录支持免账号检索空间足迹与云量；原始 224 波段归档受 DLR EOC CAS 网页单点登录保护，检索确认景号后请前往",
      enmapNoteSuffix: "门户登录下载。",
      s1GrdLabel: "优先极化通道 (AWS COG)",
      s1GrdAria: "Sentinel-1 GRD 极化通道",
      s1GrdVv: "VV · 双极化主通道（默认推荐）",
      s1GrdVh: "VH · 交叉极化（植被与水体）",
      s1GrdHh: "HH · 单/双极化水平同极化",
      s1GrdHv: "HV · 水平垂直交叉极化",
      noaaLabel: "气象预报与观测模式",
      noaaAria: "NOAA AWS 气象模式",
      noaaGfs: "NOAA GFS 0.25° (.idx 按变量 HTTP Range 子集)",
      noaaGoes19: "GOES-19 East ABI L2 (.nc 云辐射扫描)",
      noaaGoes18: "GOES-18 West ABI L2 (.nc 云辐射扫描)",
      overtureLabel: "Overture 开放矢量主题",
      overtureAria: "Overture 开放矢量主题",
      ovBuildings: "buildings · 全球建筑轮廓足迹 (GeoParquet)",
      ovTransport: "transportation · 道路交通路网",
      ovPlaces: "places · 全球兴趣点 POI",
      ovDivisions: "divisions · 行政边界划分",
      ovAddresses: "addresses · 开源门址数据",
      ovBase: "base · 地表水系与土地基础要素",
    },
  },
  en: {
    modes: {
      search: "Data Search",
      demConvert: "DEM Convert",
      gee: "GEE Cloud Studio",
    },
    sourceDelivery: {
      title: "Delivery Source & Format",
      s2CdseLabel: "ESA CDSE (Official SAFE Archive)",
      s2CdseSub: "Official ESA archive · Geometric calibration ready · CDSE required",
      s2CogSub: "Microsoft Planetary Computer & AWS Earth Search COGs · Free",
      s2GeeLabel: "Google Earth Engine (Clipped GeoTIFF)",
      s2GeeSub: "Cloud AOI clip · COPERNICUS/S2_SR_HARMONIZED",
      s2L1cCdseLabel: "ESA CDSE (Official SAFE Archive)",
      s2L1cCdseSub: "Public ESA catalog search · CDSE account required for download",
      s2L1cGeeSub: "Cloud AOI clip · COPERNICUS/S2_HARMONIZED",
      l89MpcSub: "Planetary Computer expanded single-band COGs · No account required",
      l89UsgsLabel: "USGS M2M (Official .tar Archive)",
      l89UsgsSub: "Unified catalog search · Official complete .tar archive (USGS account required)",
      l89GeeLabel: "Google Earth Engine (Clipped GeoTIFF)",
      l89GeeSub: "Cloud AOI surface reflectance clip · LANDSAT/LC09/C02/T1_L2",
      demAwsSub: "Amazon Open Data GLO-30 tiles · Free",
      demOtSub: "Subset GeoTIFF by AOI · API Key required",
      demGeeLabel: "Google Earth Engine (Elevation Clip)",
      demGeeSub: "Cloud AOI elevation clip · COPERNICUS/DEM/GLO30",
    },
    demTable: {
      datumEgm2008: "EGM2008 Orthometric",
      datumEgm96: "EGM96 Orthometric",
      datumEgm2008Bare: "EGM2008 Bare-Earth",
      datumEgm96Bare: "EGM96 Bare-Earth",
      datumEgm96Surface: "EGM96 Surface",
      datumWgs84Ellipsoid: "WGS84 Ellipsoidal",
      channelAws: "AWS Public Bucket · Direct Free",
      channelOt: "OpenTopography · API Key",
      channelFabdem: "Public COG (CC BY-NC-SA 4.0 Non-Commercial)",
      channelGem15: "Google Cloud COG (CC BY 4.0 · Excludes China)",
      channelAsf: "ASF Earthdata · Account Required",
      channelEarthdata: "NASA LP DAAC · Earthdata Account Required",
      sourceDatumLabel: "Native Datum: ",
      geoidModelLabel: "Geoid Model: ",
      channelLabel: "Channel: ",
      autoConvertHint: "Automatically converts {geoid} orthometric heights to WGS84 ellipsoid after download and writes SARscape (_dem/.hdr/.sml) + GeoTIFF.",
      estimatedTilesCount: "Est. ~{count} 1°×1° tiles",
      bindAoiFirst: "Bind a study AOI above first",
      calculating: "Calculating…",
      previewPlanBtn: "Preview Tile Plan",
      boundsPrefix: "Extent: ",
      datumPlanPrefix: "Datum Plan: ",
      mosaicArtifactPrefix: "Mosaic Output: ",
    },
    sar: {
      processingLevel: "Processing Level",
      s2L2aTitle: "L2A Surface Reflectance",
      s2L1cTitle: "L1C Top-of-Atmosphere",
      s1RetiredHint: "S1B ceased imaging in Dec 2021. For recent windows, also select S1A / S1C / S1D or leave all unchecked (all included).",
      s1RecentSwitchHint: "Tip: Recent acquisitions are often flown by S1C / S1D. Check S1C / S1D or widen to Last 1Y.",
      nisarLevelTitle: "Product Level",
      onlyLBandNote: " — L-Band only",
      releaseTierTitle: "Release Tier",
      validatedTitle: "Validated: official NASA / ASF science-quality release",
      provisionalTitle: "Provisional: NASA / ASF early commissioning & cal-val release",
      sBandLevelsNote: "S-Band supports 8 official levels (RSLC/GSLC/GCOV/GUNW/GOFF/RIFG/RUNW/ROFF); SME2 is L-Band only.",
      nisarSBandConnected: ": ISRO Bhoonidhi catalog ready, supporting full .h5 & HDF5 HTTP Range AOI subsetting.",
      nisarLBandConnected: ": ASF CMR catalog ready, supporting full .h5 & HDF5 HTTP Range AOI subsetting.",
      nisarLBandRslcOnly: "L-Band RSLC supports ASF search & HDF5 HTTP Range AOI subsetting.",
      polLabel: "Polarization",
      polClearDefault: "Clear (All)",
      iwDualPolHint: "Land IW scenes are usually dual-pol (VV+VH)",
      includeVvVhBtn: "+ Add VV+VH",
      subsetBtnLabel: "AOI Subset",
      subsetTooltipReady: "Open NISAR AOI Subset Workspace (draw region, Frequency & Polarization)",
      subsetTooltipEmpty: "Check at least 1 NISAR scene to open AOI Subset",
      completeDownloadBtn: "Download Full",
      capTruncatedNotice: "Showing newest 2,000 scenes (250 instant, backend per-search cap reached). Narrow date or AOI for earlier scenes.",
    },
    orbitMode: {
      modeLabel: "Orbit Mode",
      byAoiBtn: "Search by AOI / Date",
      localFolderBtn: "Match Local Folder",
      localInputPlaceholder: "SLC directory or ASF .py script path",
      pickScriptTitle: "Choose ASF Sentinel-1 download script or manifest",
      pickScriptFilter: "ASF scripts & manifests (*.py;*.metalink;*.meta4;*.txt;*.urls;*.csv;*.geojson;*.json)",
      pickScriptBtnTitle: "Select ASF script or manifest (.py / .metalink / .txt)",
      scriptBtn: "Script",
      localScanDesc: "Scans a local folder or ASF script for S1A/S1B/S1C/S1D timestamps and matches/downloads missing .EOF precise orbits.",
      aoiQueryDesc: "Queries and downloads precise orbits for your active AOI and date window (or attach during Sentinel-1 download).",
      batchCandidateCount: "{count} candidate scenes in current batch",
      orbitActionHint: "If scenes are checked on the right, downloads their precise orbits first (POEORB preferred, RESORB fallback).",
      matchSourceLabel: "Match Source",
      localSlcFolderOpt: "Local SLC Directory",
      currentResultsOpt: "Current Results",
      localSlcDirectoryLabel: "Local SLC Directory",
      localSlcPlaceholder: "Choose local Sentinel-1 directory",
    },
    convMode: {
      datumFallbackNote: " (Inferred EGM2008 from filename; adjustable manually)",
      formatSupportNote: "Supports local .tif / .tiff / .hgt / .dem files with offline geoid undulation computation.",
      sourceDatumLabel: "Source Vertical Datum",
      activeDatumPrefix: "Active: {datum}",
      autoOption: "Auto-detect (Copernicus→EGM2008 / SRTM→EGM96)",
      egm2008Option: "EGM2008 Orthometric (Copernicus 30m/90m)",
      egm96Option: "EGM96 Orthometric (SRTM / NASADEM / AW3D30)",
      ellipsoidOption: "Already WGS84 Ellipsoidal (Export SARscape only)",
      targetFormatLabel: "Target Output Format",
      formatSarscapeTitle: "SARscape + Ellipsoid TIF",
      formatSarscapeSub: "Write _dem/.hdr/.sml & _ellipsoid.tif",
      formatTifTitle: "WGS84 Ellipsoid TIF Only",
      formatTifSub: "Write GeoTIFF (_ellipsoid.tif)",
      outputDirLabel: "Output Directory (blank = source DEM folder)",
      outputDirPlaceholder: "Same folder as input DEM",
      pickDirTitle: "Select Elevation Output Directory",
      resultHeading: "Elevation Conversion Result",
      previewHeading: "Conversion Plan Preview",
      identifying: "Detecting…",
      previewBtn: "Preview Plan",
      writtenBadge: "Saved",
      readyBadge: "Ready",
      datumConversionPrefix: "Datum: ",
      builtinGeoidNote: " (Built-in {model} geoid)",
      ellipsoidTifPrefix: "Ellipsoid TIF: ",
    },
    gacos: {
      bindAoiNotice: "Set a study AOI first (GACOS requires lon/lat span between 0.05° and 10°)",
      spanPrefix: "Span {lon}° × {lat}°",
      spanTooLarge: "Exceeds GACOS 10° max span per request; narrow your study AOI",
      spanTooSmall: "AOI too small (recommended lon/lat span 0.05°–10°)",
      spanValid: "Within GACOS limits (0.05°–10°)",
      datesHeading: "Acquisition Dates (YYYYMMDD)",
      batchCapBadge: "Max 20/batch",
      pickDateAria: "Pick a GACOS acquisition date",
      addDateBtn: "+ Add",
      pathAria: "Local SLC folder or ASF script path",
      pathPlaceholder: "Folder or ASF .py script path",
      pickSlcDirTitle: "Choose folder with SLC scenes",
      dirBtnTitle: "Browse local SLC folder",
      dirBtn: "Folder",
      pickScriptFilter: "ASF scripts & manifests (*.py;*.metalink;*.meta4;*.txt;*.urls;*.csv;*.geojson;*.json)",
      pickScriptTitle: "Choose ASF script or manifest",
      scriptBtn: "Script",
      scanFailed: "Could not extract SLC dates from the selected directory",
      noYmdFound: "No YYYYMMDD dates recognized in filenames",
      extractedDatesToast: "Extracted {count} acquisition dates",
      scanningBtn: "Scanning...",
      extractDatesBtn: "Extract",
      textareaAria: "GACOS YYYYMMDD date list",
      textareaPlaceholder: "One 8-digit YYYYMMDD per line, e.g.:\n20260501\n20260513",
      invalidDateFmt: "Invalid dates: {tokens} (must be valid YYYYMMDD)",
      validDatesSummary: "{count} valid dates (max 20/batch; {suffix})",
      splitBatchesNote: "will split into {count} batches",
      autoSplitNote: "auto-splits into batches of 20",
      utcHourLabel: "UTC Hour (00–23)",
      utcHourAria: "UTC Hour",
      utcHourUnit: "{h}h",
      utcMinLabel: "UTC Minute (00–59)",
      utcMinAria: "UTC Minute",
      utcMinUnit: "{m}m",
      formatLabel: "GACOS Format",
      formatAria: "GACOS Format",
      asyncEmailNotice: "Note: GACOS delivers results asynchronously via email link to your configured address; no local download folder is needed now.",
      batchPreviewHeading: "GACOS Batch Request Preview",
      syncSubmitLabel: "Companion GACOS Request",
      footerCountLabel: "GACOS ({count} req)",
    },
    xyz: {
      sourcePickerHint: "Tile Source (15 Built-in incl. Wayback + Custom)",
      sourcePickerAria: "Tile Source",
      customLabel: "Custom · XYZ / TMS Tile URL Template",
      customUrlLabel: "Custom XYZ / TMS Tile URL Template",
      customUrlPlaceholder: "e.g. https://mt1.google.com/vt/lyrs=s&x={x}&y={y}&z={z}",
      placeholderHelp: "Supported placeholders:",
      crsLabel: "Tile CRS & Offset Alignment",
      crsAuto: "Auto-detect (applies GCJ-02 correction for AMap)",
      crsWgs84: "WGS84 / CGCS2000 (Standard, no offset)",
      crsGcj02: "GCJ-02 (Auto-deoffset to WGS84)",
      crsAria: "Tile Source CRS",
      zoomHeading: "Zoom Levels (Supports Z1–Z{maxZoom})",
      selectedPrefix: "Selected: {zooms}",
      noneSelected: "None",
      zoomUnsupportedTooltip: "Source max native zoom is Z{maxZoom} (cannot select up to Z{presetMax})",
      clearBtn: "Clear",
      zoomCappedTooltip: "Source max zoom is Z{maxZoom} (cannot download Z{z})",
      estimateHeading: "Tile Size Estimate",
      zoomCount: "{count} zooms",
      selectAoiFirst: "Bind a study AOI to estimate tile count and file size.",
      selectZoomFirst: "Select at least one zoom level (Z1–Z{maxZoom}).",
      tileCountCumulative: "Tiles ({count} periods)",
      tileCountSingle: "Tiles",
      perPeriodCap: "{shown} / {cap} max per period",
      gridSizeLabel: "Grid W × H",
      estTrafficLabel: "Est. Network Traffic",
      estFileSizeLabel: "Est. Output Size",
      estMemoryLabel: "Est. Peak Memory",
      streamingMemValue: "~{mem} · Streaming",
      bigTiffNote: "Streams via BigTIFF bands; stitched mosaic/VRT cap is 50,000 tiles per run; MBTiles or un-stitched ZIP cap is 200,000 tiles{cog}",
      cogStagingSuffix: " (COG staging prefers fast SSD workspace)",
      limitExceededErr: "Requires {total} tiles, exceeding limit of {cap}. Narrow AOI, lower zoom, or switch to MBTiles/tiles only.",
      jpegLimitErr: "Exceeds JPEG 65,535 px limit ({w} × {h} px). Narrow AOI or choose GeoTIFF/COG/MBTiles.",
      zoomSupportNote: "Supports Z1–Z{maxZoom}; zooms above cap fall back or remain disabled.",
      exportFormatTitle: "Output Format: {label}",
      exportFormatLabel: "Output Raster / Archive Format",
      exportFormatAria: "Output Format",
      stitchToggleTitle: "Auto-stitch into georeferenced raster mosaic",
      stitchToggleDesc: "Builds EPSG:3857 georeferenced mosaic via GDAL (cap 50,000 tiles); uncheck for raw tiles or ZIP (cap 200,000).",
      compressionTitle: "Compression: {label}",
      compressionLabel: "Compression",
      compressionAria: "Compression",
      buildOverviewsLabel: "Build internal Overviews (2x / 4x / 8x / 16x)",
      downloadPeriodsBtn: "Download {count} Historical Periods",
      downloadMosaicBtn: "Download Tiles & Mosaic",
    },
    geeInline: {
      allCatalogOption: "All Public Catalogs (Default · {total} datasets)",
      themeCatalogOption: "All Datasets in Theme (Default · {count})",
      customAssetSuffix: "{id} · Custom GEE Asset",
      publicCatalogHeading: "GEE Public Catalog ({shown}/{total})",
      selectAssetAria: "Select GEE Public Dataset",
      customAssetLabel: "Advanced: Custom / Private Asset ID",
      customAssetPlaceholder: "e.g. COPERNICUS/S2_SR_HARMONIZED or projects/...",
      customAssetNote: "Lists public collections by default; configure GEE cloud project to export private assets.",
      allCatalogSummarySuffix: "All Public Collections (Default · {total}) · AOI Clip",
      defaultBandsOption: "Recommended Default Bands",
      maxCloudLabel: "Max Cloud Cover",
      polarizationLabel: "Polarization",
      polarizationAria: "Polarization",
      temporalReducerLabel: "Temporal Reducer",
      temporalReducerAria: "Temporal Reducer",
      staticAssetNote: "Static baseline collection: date and cloud filters hidden automatically.",
      targetBandLabel: "Target Band / Variable",
      targetBandAria: "Target Band / Variable",
      openGeeWorkbenchBtn: "Open GEE Cloud Studio →",
      polAll: "All Polarizations (VV / VH / HH / HV)",
      polVv: "VV Co-pol (Sentinel-1)",
      polVh: "VH Cross-pol (Sentinel-1)",
      polHh: "HH Co-pol (ALOS PALSAR)",
      polHv: "HV Cross-pol (ALOS PALSAR)",
      reducerMedian: "Median (Cloud-resistant)",
      reducerMean: "Mean",
      reducerMax: "Max",
      reducerMin: "Min",
      cloudOpt10: "<= 10% (Clear sky)",
      cloudOpt20: "<= 20% (Low cloud)",
      cloudOpt30: "<= 30% (Recommended)",
      cloudOpt50: "<= 50% (Moderate)",
      cloudOpt80: "<= 80% (Relaxed)",
      cloudOpt100: "No cloud limit (100%)",
    },
    resultsExtra: {
      showPreviewTitle: "Show map footprint quicklook",
      hidePreviewTitle: "Hide map footprint quicklook",
      mapPreviewBadge: "Preview ({count})",
      mapPreviewHiddenBadge: "Hidden ({count})",
      clearPreviewsAria: "Clear all map previews",
      clearResultsAria: "Clear search results",
      clearBtn: "Clear",
      listLoadFailedTitle: "Could not load scene list",
      listLoadFailedDesc: "Catalog retrieval failed; your study area and filters are preserved.",
      searchingTitle: "Searching…",
      searchIncompleteTitle: "Search did not complete",
      searchIncompleteFallback: "Connection interrupted; your study area and filters remain intact.",
      retryBtn: "Retry",
      noMatchTitle: "No matching imagery found",
      noMatchDesc: "Try raising the cloud limit, widening the time window, or switching sources.",
      resetFilterBtn: "Reset Filters",
      emptyLocatedAoi: "Located · {label}",
      emptySelectedAoi: "Study Area Selected",
      emptySearchTitle: "No search started yet",
      emptySearchWithAoiDesc: "Review sensor and date parameters on the left to fetch scenes.",
      emptySearchNoAoiDesc: "Bind a study area on the map and set sensor/time to start.",
      startSearchBtn: "Start Search",
      drawAoiBtn: "Draw Study Area",
    },
    bottomSummary: {
      localSlcFolder: "Local SLC: {name}",
      chooseLocalSlc: "Local Scan: Choose Sentinel-1 directory",
      restrictedAoi: "Extent: {aoi}",
      customAoiName: "Custom AOI",
      globalOrView: "Extent: Global or current view",
      gacosSummary: "{dates} dates · {h}:{m} UTC · {fmt}",
      approxDemTiles: "~{count} tiles",
      elapsedSeconds: "{label} · {sec}s",
      cancelling: "Cancelling",
      abort: "Stop",
      deriveRegisteredNote: "Pre-coregistered geocoded interferometric product; no separate raw SLC precise-orbit fetch required.",
      enmapSearchOnlyNotice: "DLR EnMAP L2A supports public catalog search only; download raw archives at https://eoweb.dlr.de after search.",
    },
    categoryBadge: {
      sar: "SAR",
      orbit: "ORBIT",
      convert: "CONVERT",
      atmo: "ATMOS",
      geodesy: "GEODESY",
      thematic: "THEMATIC",
      optical: "OPTICAL",
      aux: "AUX",
    },
    topicOptions: {
      productVersionLabel: "Product Version",
      productVersionAria: "WorldCover Product Version",
      wcV200: "WorldCover v200 (2021 Latest · 10m)",
      wcV100: "WorldCover v100 (2020 Baseline · 10m)",
      waterLayerLabel: "Surface Water Layer",
      waterLayerAria: "JRC Surface Water Layer",
      jrcOccurrence: "occurrence · Historical Water Frequency (0–100%)",
      jrcExtent: "extent · Maximum Water Mask",
      jrcSeasonality: "seasonality · Permanent vs Seasonal Water",
      jrcChange: "change · Multi-year Occurrence Change",
      alosBandLabel: "Polarization & Mask Band",
      alosBandAria: "ALOS PALSAR Band",
      alosHh: "HH · Horizontal Backscatter (.tif)",
      alosHv: "HV · Cross-Pol Backscatter (.tif)",
      alosMask: "mask · Layover / Shadow / Water Mask (.tif)",
      modisBandLabel: "Vegetation Index Band",
      modisBandAria: "MODIS Index Band",
      modisNdvi: "250m_16_days_NDVI · Normalized Difference Vegetation Index",
      modisEvi: "250m_16_days_EVI · Enhanced Vegetation Index",
      modisQa: "250m_16_days_VI_Quality · QA Mask",
      s5pLabel: "Atmospheric Trace Gas Product (TROPOMI L2)",
      s5pAria: "Sentinel-5P Gas Product",
      s5pNo2: "NO2 · Tropospheric Nitrogen Dioxide",
      s5pCh4: "CH4 · Methane Mixing Ratio",
      s5pSo2: "SO2 · Sulfur Dioxide (Volcanic)",
      s5pCo: "CO · Carbon Monoxide Column",
      s5pO3: "O3 · Total Ozone Column",
      s5pAer: "AER_AI · UV Aerosol Index",
      operaLabel: "OPERA Sub-product Type",
      operaAria: "OPERA Product Subtype",
      operaDisp: "OPERA_L3_DISP-S1_V1 · Surface LOS Displacement",
      operaCslc: "OPERA_L2_CSLC-S1_V1 · Coregistered Geocoded SLC",
      operaRtc: "OPERA_L2_RTC-S1_V1 · Terrain-Corrected Backscatter",
      era5Label: "Reanalysis Variable Preset",
      era5Aria: "ERA5 Variables",
      era5HourlyInsar: "Hourly Tropospheric T/q/P (InSAR · Recommended)",
      era5HourlyFull: "Full Hourly Atmospheric Grid (Wind/Cloud/Precip)",
      era5Daily: "Daily Aggregated Reanalysis",
      eqLabel: "Minimum Magnitude (Mw)",
      eqAria: "Minimum Magnitude",
      eqMw3: "Mw ≥ 3.0 (Light & Felt Events)",
      eqMw45: "Mw ≥ 4.5 (Moderate+ · Recommended)",
      eqMw55: "Mw ≥ 5.5 (Significant Coseismic Deformation)",
      eqMw65: "Mw ≥ 6.5 (Major Earthquakes)",
      igsLabel: "Ionosphere Analysis Center",
      igsAria: "Ionosphere Analysis Center",
      igsCod: "COD · CODE Final Solution (Univ. Bern)",
      igsIgs: "IGS · IGS Combined Solution",
      igsJpl: "JPL · NASA Jet Propulsion Laboratory",
      nisarOeLabel: "NISAR Ephemeris Type (NISAR_OE)",
      nisarOeAria: "NISAR Orbit Ephemeris",
      nisarOeAll: "ALL · All Ephemerides (POE/MOE/NOE/FOE)",
      nisarOePoe: "POE · Precision Orbit Ephemeris (Highest)",
      nisarOeMoe: "MOE · Medium Orbit Ephemeris",
      nisarOeNoe: "NOE · Nominal Real-Time Ephemeris",
      nisarOeFoe: "FOE · Forecast Orbit Ephemeris",
      nisarOeHint: "Global state vectors (.xml) queried by date range without AOI polygon; download requires Earthdata.",
      podLabel: "Copernicus POD Family",
      podAria: "Copernicus POD Family",
      podAll: "ALL · Sentinel-2 + Sentinel-3 Precise Orbits",
      podHint: "Global precise ephemerides queried by date range without AOI polygon; download requires CDSE.",
      bmLabel: "Black Marble Product Cadence (500m HDF5)",
      bmAria: "Black Marble Cadence",
      bmA3: "VNP46A3 · Monthly Cloud-Free Composite (Recommended)",
      bmA4: "VNP46A4 · Annual Cloud-Free Composite",
      bmA2: "VNP46A2 · Daily Gap-Filled BRDF (Outages)",
      emitLabel: "EMIT Hyperspectral Level (60m · 285 bands)",
      emitAria: "EMIT Scientific Level",
      emitL2a: "EMITL2ARFL · L2A Surface Reflectance (.nc)",
      emitL2b: "EMITL2BMIN · L2B Mineral Identification (.nc)",
      ecoLabel: "ECOSTRESS Thermal / ET Layer (70m COG)",
      ecoAria: "ECOSTRESS Layer",
      ecoLst: "ECO_L2T_LSTE · Land Surface Temp LST (Recommended)",
      ecoEmis: "ECO_L2T_LSTE · Wideband Emissivity (_EmisWB.tif)",
      ecoEt: "ECO_L3T_JET · JPL Daily Evapotranspiration (_ETdaily.tif)",
      enmapNotePrefix: "DLR EnMAP L2A open STAC allows account-free footprint and cloud search. Raw 224-band archives require DLR EOC CAS SSO at",
      enmapNoteSuffix: ".",
      s1GrdLabel: "Preferred Polarization (AWS COG)",
      s1GrdAria: "Sentinel-1 GRD Polarization",
      s1GrdVv: "VV · Primary Channel (Recommended)",
      s1GrdVh: "VH · Cross-Pol (Vegetation & Water)",
      s1GrdHh: "HH · Co-Pol Horizontal",
      s1GrdHv: "HV · Cross-Pol Horizontal/Vertical",
      noaaLabel: "Forecast & Observation Mode",
      noaaAria: "NOAA AWS Mode",
      noaaGfs: "NOAA GFS 0.25° (.idx Byte-Range Variable Subset)",
      noaaGoes19: "GOES-19 East ABI L2 (.nc Cloud & Radiance)",
      noaaGoes18: "GOES-18 West ABI L2 (.nc Cloud & Radiance)",
      overtureLabel: "Overture Open Vector Theme",
      overtureAria: "Overture Vector Theme",
      ovBuildings: "buildings · Global Building Footprints (GeoParquet)",
      ovTransport: "transportation · Road Networks",
      ovPlaces: "places · Global Points of Interest",
      ovDivisions: "divisions · Administrative Boundaries",
      ovAddresses: "addresses · Open Address Points",
      ovBase: "base · Land & Hydrography Base Layers",
    },
  },
  ja: {
    modes: {
      search: "データ検索",
      demConvert: "DEM 変換",
      gee: "GEE クラウド作図",
    },
    sourceDelivery: {
      title: "取得元と形式",
      s2CdseLabel: "ESA CDSE (公式 SAFE アーカイブ)",
      s2CdseSub: "ESA 公式原本 · 幾何補正・研究向け · CDSE アカウント要",
      s2CogSub: "Microsoft Planetary Computer & AWS Earth Search 単バンド COG · 無料",
      s2GeeLabel: "Google Earth Engine (切り抜き GeoTIFF)",
      s2GeeSub: "クラウド AOI 切り抜き · COPERNICUS/S2_SR_HARMONIZED",
      s2L1cCdseLabel: "ESA CDSE (公式 SAFE アーカイブ)",
      s2L1cCdseSub: "ESA 公開カタログ検索 · 取得に CDSE アカウント要",
      s2L1cGeeSub: "クラウド AOI 切り抜き · COPERNICUS/S2_HARMONIZED",
      l89MpcSub: "Planetary Computer 単バンド COG 展開セット · アカウント不要",
      l89UsgsLabel: "USGS M2M (公式 .tar アーカイブ)",
      l89UsgsSub: "公開カタログ検索 · USGS 公式完全 .tar 納品（USGS アカウント要）",
      l89GeeLabel: "Google Earth Engine (切り抜き GeoTIFF)",
      l89GeeSub: "クラウド AOI 地表面反射率出力 · LANDSAT/LC09/C02/T1_L2",
      demAwsSub: "AWS Open Data GLO-30 原本タイル · 無料",
      demOtSub: "AOI 統合 GeoTIFF · API Key 要",
      demGeeLabel: "Google Earth Engine (標高切り抜き)",
      demGeeSub: "クラウド AOI 標高出力 · COPERNICUS/DEM/GLO30",
    },
    demTable: {
      datumEgm2008: "EGM2008 正標高",
      datumEgm96: "EGM96 正標高",
      datumEgm2008Bare: "EGM2008 裸地正標高",
      datumEgm96Bare: "EGM96 裸地正標高",
      datumEgm96Surface: "EGM96 表層正標高",
      datumWgs84Ellipsoid: "WGS84 楕円体高",
      channelAws: "AWS 公開バケット · アカウント不要",
      channelOt: "OpenTopography · API Key 要",
      channelFabdem: "公開 COG（CC BY-NC-SA 4.0 非商用）",
      channelGem15: "Google Cloud 公開 COG（CC BY 4.0 · 中国本土除く）",
      channelAsf: "ASF Earthdata · アカウント要",
      channelEarthdata: "NASA LP DAAC · Earthdata アカウント要",
      sourceDatumLabel: "元鉛直基準：",
      geoidModelLabel: "ジオイドモデル：",
      channelLabel: "配信元：",
      autoConvertHint: "取得後に自動で {geoid} 正標高 → WGS84 楕円体高へ変換し、SARscape 標準 (_dem/.hdr/.sml) と GeoTIFF を生成します。",
      estimatedTilesCount: "対象約 {count} 枚の 1°×1° タイル",
      bindAoiFirst: "上部の地図または検索から AOI を設定してください",
      calculating: "計算中…",
      previewPlanBtn: "タイル計画をプレビュー",
      boundsPrefix: "範囲：",
      datumPlanPrefix: "基準変換：",
      mosaicArtifactPrefix: "結合成果：",
    },
    sar: {
      processingLevel: "処理レベル (Processing Level)",
      s2L2aTitle: "L2A 地表面反射率",
      s2L1cTitle: "L1C 大気上端反射率",
      s1RetiredHint: "S1B は 2021年12月に運用終了しました。近年の検索では S1A / S1C / S1D の併用か全未選択（全対象）を推奨します。",
      s1RecentSwitchHint: "ヒント：最近の軌道は S1C / S1D が引き継いでいます。少ない場合は S1C / S1D や「1年」をお試しください。",
      nisarLevelTitle: "プロダクトレベル",
      onlyLBandNote: " — L-Band 専用レベル",
      releaseTierTitle: "公開ティア (Release Tier)",
      validatedTitle: "Validated：NASA / ASF 公式検証済みサイエンス版",
      provisionalTitle: "Provisional：初期試運用・校正段階の暫定版",
      sBandLevelsNote: "S-Band は RSLC/GSLC/GCOV/GUNW/GOFF/RIFG/RUNW/ROFF の 8 種類に対応。SME2 は L-Band 専用です。",
      nisarSBandConnected: "：ISRO Bhoonidhi 検索に接続済み。完全版 .h5 および HTTP Range による AOI 切り抜きに対応。",
      nisarLBandConnected: "：ASF CMR 検索に接続済み。完全版 .h5 および HTTP Range による AOI 切り抜きに対応。",
      nisarLBandRslcOnly: "L-Band RSLC は ASF 検索および HTTP Range 切り抜きに対応しています。",
      polLabel: "偏波 (Polarization)",
      polClearDefault: "クリア (全選択)",
      iwDualPolHint: "陸域 IW シーンは通常 2 偏波 (VV+VH) です",
      includeVvVhBtn: "+ VV+VH を追加",
      subsetBtnLabel: "AOI 切り抜き",
      subsetTooltipReady: "NISAR AOI 切り抜きワークスペースを開く",
      subsetTooltipEmpty: "切り抜き対象となる NISAR シーンを選択してください",
      completeDownloadBtn: "完全版を取得",
      capTruncatedNotice: "最新 2,000 シーンを表示中（単一検索の上限到達）。過去の画像は期間や範囲を絞って検索してください。",
    },
    orbitMode: {
      modeLabel: "軌道取得モード",
      byAoiBtn: "AOI / 日付で検索取得",
      localFolderBtn: "ローカルフォルダーと照合",
      localInputPlaceholder: "SLC フォルダーまたは ASF .py パス",
      pickScriptTitle: "ASF Sentinel-1 ダウンロードスクリプトまたはリストを選択",
      pickScriptFilter: "ASF スクリプト・リスト (*.py;*.metalink;*.meta4;*.txt;*.urls;*.csv;*.geojson;*.json)",
      pickScriptBtnTitle: "ASF スクリプト/リストを選択 (.py / .metalink / .txt)",
      scriptBtn: "スクリプト",
      localScanDesc: "ローカル SLC フォルダーや ASF スクリプト内の S1A/S1B/S1C/S1D 日時を自動解析し、不足している .EOF 精密軌道を照合・取得します。",
      aoiQueryDesc: "現在の AOI と期間から精密軌道を検索・ダウンロードします（Sentinel-1 画像取得時の自動同梱も可能です）。",
      batchCandidateCount: "現在のバッチ候補：{count} シーン",
      orbitActionHint: "右の一覧でシーンを選択している場合、該当シーンの精密軌道を優先取得します（POEORB 優先・RESORB 代替）。",
      matchSourceLabel: "照合ソース",
      localSlcFolderOpt: "ローカル SLC 画像フォルダー",
      currentResultsOpt: "現在の検索結果",
      localSlcDirectoryLabel: "ローカル SLC ディレクトリー",
      localSlcPlaceholder: "ローカル Sentinel-1 フォルダーを選択",
    },
    convMode: {
      datumFallbackNote: "（ファイル名に基準表記がないため EGM2008 として判定）",
      formatSupportNote: ".tif / .tiff / .hgt / .dem ファイルに対応。内蔵ジオイドモデルでオフライン変換します。",
      sourceDatumLabel: "入力 DEM 鉛直基準",
      activeDatumPrefix: "現在の適用：{datum}",
      autoOption: "自動判定 (Copernicus→EGM2008 / SRTM→EGM96)",
      egm2008Option: "EGM2008 正標高 (Copernicus 30m/90m)",
      egm96Option: "EGM96 正標高 (SRTM / NASADEM / AW3D30)",
      ellipsoidOption: "変換済み WGS84 楕円体高 (SARscape 形式出力のみ)",
      targetFormatLabel: "出力形式",
      formatSarscapeTitle: "SARscape + 楕円体高 TIF",
      formatSarscapeSub: "_dem/.hdr/.sml と _ellipsoid.tif を生成",
      formatTifTitle: "WGS84 楕円体高 TIF のみ",
      formatTifSub: "標準 GeoTIFF (_ellipsoid.tif) のみ生成",
      outputDirLabel: "出力先フォルダー（空欄で元ファイルと同場所）",
      outputDirPlaceholder: "入力 DEM と同じフォルダー",
      pickDirTitle: "標高変換の出力先フォルダーを選択",
      resultHeading: "標高変換結果",
      previewHeading: "変換プランプレビュー",
      identifying: "判定中…",
      previewBtn: "プラン確認",
      writtenBadge: "保存済み",
      readyBadge: "検証完了",
      datumConversionPrefix: "基準変換：",
      builtinGeoidNote: "（内蔵 {model} ジオイドモデル）",
      ellipsoidTifPrefix: "楕円体高 TIF：",
    },
    gacos: {
      bindAoiNotice: "先に地図または行政区検索で AOI を設定してください（緯度経度幅 0.05°–10° 以内）",
      spanPrefix: "幅 {lon}° × {lat}°",
      spanTooLarge: "GACOS 1回の最大範囲 (≤ 10°) を超過しています。範囲を縮小してください",
      spanTooSmall: "範囲が小さすぎます（推奨幅 0.05°〜10°）",
      spanValid: "GACOS の範囲条件を満たしています (0.05°–10°)",
      datesHeading: "撮像日リスト（8 桁 YYYYMMDD）",
      batchCapBadge: "1回最大 20件",
      pickDateAria: "追加する GACOS 日付を選択",
      addDateBtn: "+ 追加",
      pathAria: "ローカル SLC フォルダーまたは ASF スクリプト",
      pathPlaceholder: "フォルダーまたは ASF .py パス",
      pickSlcDirTitle: "SLC 画像を含むフォルダーを選択",
      dirBtnTitle: "ローカル SLC フォルダーを参照",
      dirBtn: "参照",
      pickScriptFilter: "ASF スクリプト・リスト (*.py;*.metalink;*.meta4;*.txt;*.urls;*.csv;*.geojson;*.json)",
      pickScriptTitle: "ASF スクリプト・リストを選択",
      scriptBtn: "スクリプト",
      scanFailed: "指定ディレクトリから SLC 日付を読み取れませんでした",
      noYmdFound: "8 桁日付 (YYYYMMDD) を含むファイル名が見つかりませんでした",
      extractedDatesToast: "{count} 件の撮像日を抽出しました",
      scanningBtn: "抽出中...",
      extractDatesBtn: "日付抽出",
      textareaAria: "GACOS 8 桁日付リスト",
      textareaPlaceholder: "1 行につき 1 つの YYYYMMDD を入力：\n20260501\n20260513",
      invalidDateFmt: "無効な日付形式：{tokens}（有効な YYYYMMDD を入力してください）",
      validDatesSummary: "有効な日付 {count} 件（1 回最大 20 件 · {suffix}）",
      splitBatchesNote: "{count} バッチに自動分割して送信",
      autoSplitNote: "20 件ごとに自動分割",
      utcHourLabel: "UTC 時 (00–23)",
      utcHourAria: "UTC 時",
      utcHourUnit: "{h} 時",
      utcMinLabel: "UTC 分 (00–59)",
      utcMinAria: "UTC 分",
      utcMinUnit: "{m} 分",
      formatLabel: "GACOS 出力形式",
      formatAria: "GACOS 出力形式",
      asyncEmailNotice: "補足：GACOS はメール通知による非同期配信のため、設定済みメールアドレスへ結果リンクが送信されます（保存先指定は不要です）。",
      batchPreviewHeading: "GACOS 一括申請プレビュー",
      syncSubmitLabel: "GACOS 同時申請",
      footerCountLabel: "GACOS {count} 件申請",
    },
    xyz: {
      sourcePickerHint: "タイルソース（内蔵 15 種 · Wayback 含む）",
      sourcePickerAria: "タイルソース選択",
      customLabel: "カスタムソース · XYZ / TMS テンプレート",
      customUrlLabel: "カスタム XYZ / TMS タイル URL テンプレート",
      customUrlPlaceholder: "例: https://mt1.google.com/vt/lyrs=s&x={x}&y={y}&z={z}",
      placeholderHelp: "利用可能な変数：",
      crsLabel: "座標系・オフセット補正",
      crsAuto: "自動判別 (autonavi/amap は GCJ-02 自動補正)",
      crsWgs84: "WGS84 / CGCS2000 標準座標 (補正なし)",
      crsGcj02: "GCJ-02 座標を WGS84 へ逆算補正",
      crsAria: "タイルソース座標系",
      zoomHeading: "ズームレベル (Z1〜Z{maxZoom} 複数選択)",
      selectedPrefix: "選択済み：{zooms}",
      noneSelected: "未選択",
      zoomUnsupportedTooltip: "現在のソースの最大ズームは Z{maxZoom} です (Z{presetMax} 不可)",
      clearBtn: "クリア",
      zoomCappedTooltip: "現在のソースは最大 Z{maxZoom} まで対応（Z{z} は不可）",
      estimateHeading: "タイル容量見積",
      zoomCount: "{count} レベル",
      selectAoiFirst: "先に地図または検索で AOI を設定すると容量を見積もれます。",
      selectZoomFirst: "少なくとも 1 つのズームレベル (Z1–Z{maxZoom}) を選択してください。",
      tileCountCumulative: "タイル枚数 ({count} 時期合計)",
      tileCountSingle: "タイル枚数",
      perPeriodCap: "{shown} / 1時期上限 {cap}",
      gridSizeLabel: "グリッド幅×高",
      estTrafficLabel: "推定通信量",
      estFileSizeLabel: "推定出力サイズ",
      estMemoryLabel: "推定ピークメモリ",
      streamingMemValue: "約 {mem} · ストリーム書込",
      bigTiffNote: "BigTIFF ストリーム出力対応。単一結合・VRT 上限は 50,000 枚、MBTiles や未結合 ZIP は 200,000 枚{cog}",
      cogStagingSuffix: "（COG 変換の一時領域には高速 SSD を優先使用）",
      limitExceededErr: "現在の出力モードの上限を超えています（必要 {total} 枚 / 上限 {cap} 枚）。範囲やズームを下げるか MBTiles に切り替えてください。",
      jpegLimitErr: "JPEG の 1 辺 65,535 px 上限を超えています（{w} × {h} px）。GeoTIFF / COG / MBTiles をお選びください。",
      zoomSupportNote: "最大 Z{maxZoom} まで対応。高倍率タイルが欠損する領域は上位レベルから自動補間されます。",
      exportFormatTitle: "出力形式：{label}",
      exportFormatLabel: "出力ラスタ / アーカイブ形式",
      exportFormatAria: "出力形式",
      stitchToggleTitle: "位置座標付き単一ラスタ画像へ自動結合",
      stitchToggleDesc: "内蔵 GDAL で EPSG:3857 幾何補正済み画像を生成（上限 50,000 枚）。オフ時は元タイルまたは ZIP 保存（上限 200,000 枚）。",
      compressionTitle: "圧縮方式：{label}",
      compressionLabel: "圧縮方式",
      compressionAria: "圧縮方式",
      buildOverviewsLabel: "内部ピラミッド構築 (2x / 4x / 8x / 16x Overviews)",
      downloadPeriodsBtn: "{count} 時期の履歴画像を一括ダウンロード",
      downloadMosaicBtn: "タイル取得＆結合画像出力",
    },
    geeInline: {
      allCatalogOption: "全公開リモートセンシングカタログ（既定・{total} 件）",
      themeCatalogOption: "現在テーマの全データセット（既定・{count} 件）",
      customAssetSuffix: "{id} · カスタム GEE アセット",
      publicCatalogHeading: "GEE 公開カタログ ({shown}/{total})",
      selectAssetAria: "GEE 公開データセットを選択",
      customAssetLabel: "詳細設定：独自・プライベート Asset ID",
      customAssetPlaceholder: "例: COPERNICUS/S2_SR_HARMONIZED または projects/...",
      customAssetNote: "既定では公開コレクションを表示します。GEE プロジェクトを設定するとプライベート資産も出力できます。",
      allCatalogSummarySuffix: "全公開カタログ（既定 {total} 件）· AOI オンライン切り抜き",
      defaultBandsOption: "推奨デフォルトバンド",
      maxCloudLabel: "最大雲量上限",
      polarizationLabel: "偏波チャネル (Polarization)",
      polarizationAria: "偏波チャネル",
      temporalReducerLabel: "時系列集約メソッド",
      temporalReducerAria: "時系列集約メソッド",
      staticAssetNote: "静的な無時相データセットのため、日付・雲量フィルターは非表示です。",
      targetBandLabel: "対象バンド / 物理量",
      targetBandAria: "対象バンドまたは物理量",
      openGeeWorkbenchBtn: "GEE クラウド計算画面へ →",
      polAll: "すべて (VV / VH / HH / HV)",
      polVv: "VV 水平垂直共偏波 (Sentinel-1)",
      polVh: "VH 交差偏波 (Sentinel-1)",
      polHh: "HH 水平共偏波 (ALOS PALSAR)",
      polHv: "HV 交差偏波 (ALOS PALSAR)",
      reducerMedian: "中央値合成 (Median · 外れ値に強い)",
      reducerMean: "平均値 (Mean)",
      reducerMax: "最大値合成 (Max)",
      reducerMin: "最小値合成 (Min)",
      cloudOpt10: "<= 10% (極めて晴天)",
      cloudOpt20: "<= 20% (少雲)",
      cloudOpt30: "<= 30% (推奨デフォルト)",
      cloudOpt50: "<= 50% (標準)",
      cloudOpt80: "<= 80% (緩やか)",
      cloudOpt100: "雲量無制限 (100%)",
    },
    resultsExtra: {
      showPreviewTitle: "地図上のクイックルック表示を有効にする",
      hidePreviewTitle: "地図上のクイックルック表示を隠す",
      mapPreviewBadge: "地図プレビュー ({count})",
      mapPreviewHiddenBadge: "プレビュー非表示 ({count})",
      clearPreviewsAria: "地図プレビューをすべて消去",
      clearResultsAria: "検索結果を消去",
      clearBtn: "クリア",
      listLoadFailedTitle: "画像リストを読み込めませんでした",
      listLoadFailedDesc: "カタログの取得に失敗しました。設定済みの AOI と条件は保持されています。",
      searchingTitle: "検索中…",
      searchIncompleteTitle: "画像検索が完了しませんでした",
      searchIncompleteFallback: "ソースとの接続が中断されました。設定済みの AOI と条件は保持されています。",
      retryBtn: "再試行",
      noMatchTitle: "条件に一致する画像が見つかりませんでした",
      noMatchDesc: "雲量上限の緩和、期間の拡大、またはソースの切り替えをお試しください。",
      resetFilterBtn: "条件をリセット",
      emptyLocatedAoi: "位置設定済み · {label}",
      emptySelectedAoi: "研究対象領域を選択済み",
      emptySearchTitle: "画像検索はまだ開始されていません",
      emptySearchWithAoiDesc: "左側のセンサー・期間条件を確認して画像を検索してください。",
      emptySearchNoAoiDesc: "先に地図上で研究領域を設定し、センサーと期間を指定してください。",
      startSearchBtn: "検索を開始",
      drawAoiBtn: "対象範囲を描画",
    },
    bottomSummary: {
      localSlcFolder: "ローカル画像フォルダー：{name}",
      chooseLocalSlc: "ローカル走査：Sentinel-1 フォルダーを選択",
      restrictedAoi: "範囲指定：{aoi}",
      customAoiName: "カスタム範囲",
      globalOrView: "範囲：全球または現在の表示域",
      gacosSummary: "{dates} 日分 · {h}:{m} UTC · {fmt}",
      approxDemTiles: "約 {count} タイル",
      elapsedSeconds: "{label} · {sec}秒",
      cancelling: "中止中",
      abort: "中止",
      deriveRegisteredNote: "位置合わせ・幾何補正済みの干渉生成物のため、元の SLC 精密軌道を別途取得する必要はありません。",
      enmapSearchOnlyNotice: "DLR EnMAP L2A は公開カタログ検索のみ対応しています。原版画像は https://eoweb.dlr.de から取得してください。",
    },
    categoryBadge: {
      sar: "レーダー SAR",
      orbit: "軌道",
      convert: "標高変換",
      atmo: "大気",
      geodesy: "測地・災害",
      thematic: "主題図",
      optical: "光学",
      aux: "補助",
    },
    topicOptions: {
      productVersionLabel: "プロダクトバージョン",
      productVersionAria: "WorldCover バージョン",
      wcV200: "WorldCover v200 (2021 最新版 · 10m)",
      wcV100: "WorldCover v100 (2020 基準版 · 10m)",
      waterLayerLabel: "表層水レイヤー (Asset Layer)",
      waterLayerAria: "JRC 表層水レイヤー",
      jrcOccurrence: "occurrence · 過去の水域出現頻度 (0–100%)",
      jrcExtent: "extent · 最大水域マスク",
      jrcSeasonality: "seasonality · 恒久・季節水域分類",
      jrcChange: "change · 水域出現頻度の経年変化",
      alosBandLabel: "偏波・マスクバンド",
      alosBandAria: "ALOS PALSAR バンド",
      alosHh: "HH · 水平同偏波後方散乱 (.tif)",
      alosHv: "HV · 交差偏波後方散乱 (.tif)",
      alosMask: "mask · 倒れ込み/影/水域幾何マスク (.tif)",
      modisBandLabel: "植生指標バンド",
      modisBandAria: "MODIS バンド",
      modisNdvi: "250m_16_days_NDVI · 正規化植生指標 (NDVI)",
      modisEvi: "250m_16_days_EVI · 拡張植生指標 (EVI)",
      modisQa: "250m_16_days_VI_Quality · 品質管理マスク",
      s5pLabel: "大気微量ガス製品 (TROPOMI L2)",
      s5pAria: "Sentinel-5P ガス製品",
      s5pNo2: "NO2 · 対流圏二酸化窒素柱濃度",
      s5pCh4: "CH4 · メタン混合比",
      s5pSo2: "SO2 · 二酸化硫黄柱濃度 (火山監視)",
      s5pCo: "CO · 一酸化炭素総柱濃度",
      s5pO3: "O3 · オゾン総柱濃度",
      s5pAer: "AER_AI · 紫外線エアロゾル指数",
      operaLabel: "OPERA サブプロダクト種別",
      operaAria: "OPERA サブプロダクト",
      operaDisp: "OPERA_L3_DISP-S1_V1 · 地表視線方向変位時系列",
      operaCslc: "OPERA_L2_CSLC-S1_V1 · 位置合わせ済み複素画像 (CSLC)",
      operaRtc: "OPERA_L2_RTC-S1_V1 · 地形放射補正後方散乱 (RTC)",
      era5Label: "再解析変数セット",
      era5Aria: "ERA5 変数セット",
      era5HourlyInsar: "毎時 対流圏温湿圧パラメータ (InSAR 補正 · 推奨)",
      era5HourlyFull: "毎時 気象再解析全グリッド (温湿圧・風・降水)",
      era5Daily: "日次総合統計再解析",
      eqLabel: "最小マグニチュード (Mw)",
      eqAria: "最小マグニチュード",
      eqMw3: "Mw ≥ 3.0 (有感微小・小規模地震)",
      eqMw45: "Mw ≥ 4.5 (中規模以上 · 推奨)",
      eqMw55: "Mw ≥ 5.5 (顕著な地震時地殻変動)",
      eqMw65: "Mw ≥ 6.5 (大地震)",
      igsLabel: "電離層解析センター",
      igsAria: "電離層解析センター",
      igsCod: "COD · ベルン大学 CODE 最終解",
      igsIgs: "IGS · 国際 GNSS 統合解",
      igsJpl: "JPL · NASA ジェット推進研究所解",
      nisarOeLabel: "NISAR 軌道暦グレード (NISAR_OE)",
      nisarOeAria: "NISAR 軌道暦グレード",
      nisarOeAll: "ALL · 全軌道暦 (POE/MOE/NOE/FOE)",
      nisarOePoe: "POE · 精密決定軌道暦 (最高精度)",
      nisarOeMoe: "MOE · 中精度軌道暦",
      nisarOeNoe: "NOE · 公称リアルタイム軌道暦",
      nisarOeFoe: "FOE · 予報軌道暦",
      nisarOeHint: "全球軌道ベクトル (.xml)。AOI 枠指定は不要で、期間により検索できます（取得に Earthdata 要）。",
      podLabel: "Copernicus POD 衛星群",
      podAria: "Copernicus POD 衛星群",
      podAll: "ALL · Sentinel-2 + Sentinel-3 全精密軌道",
      podHint: "全球精密軌道暦。AOI 枠指定不要で期間検索でき、取得には CDSE アカウントが必要です。",
      bmLabel: "Black Marble 夜間光周期 (500m HDF5)",
      bmAria: "Black Marble 周期",
      bmA3: "VNP46A3 · 月次雲除去合成夜間光（推奨）",
      bmA4: "VNP46A4 · 年次雲除去合成夜間光",
      bmA2: "VNP46A2 · 日次月光/BRDF 補正夜間光（停電評価）",
      emitLabel: "EMIT ハイパースペクトルレベル (60m · 285 バンド)",
      emitAria: "EMIT レベル",
      emitL2a: "EMITL2ARFL · L2A 地表面反射率 (.nc)",
      emitL2b: "EMITL2BMIN · L2B 鉱物同定・存在比 (.nc)",
      ecoLabel: "ECOSTRESS 熱赤外・蒸発散レイヤー (70m COG)",
      ecoAria: "ECOSTRESS レイヤー",
      ecoLst: "ECO_L2T_LSTE · L2T 地表面温度 LST（推奨）",
      ecoEmis: "ECO_L2T_LSTE · 広帯域射出率 (_EmisWB.tif)",
      ecoEt: "ECO_L3T_JET · JPL 日蒸発散量 (_ETdaily.tif)",
      enmapNotePrefix: "DLR EnMAP L2A 公開 STAC ではアカウント不要でフットプリントと雲量を検索可能。原本 224 バンドの取得は",
      enmapNoteSuffix: "より DLR SSO ログインで行ってください。",
      s1GrdLabel: "優先偏波チャネル (AWS COG)",
      s1GrdAria: "Sentinel-1 GRD 偏波",
      s1GrdVv: "VV · 主チャネル（推奨デフォルト）",
      s1GrdVh: "VH · 交差偏波（植生・水域向け）",
      s1GrdHh: "HH · 水平同偏波",
      s1GrdHv: "HV · 水平垂直交差偏波",
      noaaLabel: "気象予報・観測モード",
      noaaAria: "NOAA AWS モード",
      noaaGfs: "NOAA GFS 0.25° (.idx 変数別 Range 切り出し)",
      noaaGoes19: "GOES-19 East ABI L2 (.nc 雲・放射観測)",
      noaaGoes18: "GOES-18 West ABI L2 (.nc 雲・放射観測)",
      overtureLabel: "Overture オープンベクターテーマ",
      overtureAria: "Overture テーマ",
      ovBuildings: "buildings · 全球建物フットプリント (GeoParquet)",
      ovTransport: "transportation · 道路ネットワーク",
      ovPlaces: "places · 全球 POI データ",
      ovDivisions: "divisions · 行政区画境界",
      ovAddresses: "addresses · オープン住所データ",
      ovBase: "base · 水系・基盤土地地物",
    },
  },
};

export const XYZ_EXPORT_FORMAT_LOCALE: Record<Locale, Record<string, string>> = {
  "zh-CN": {
    gtiff: "GeoTIFF 标准大图 (.tif)",
    cog: "COG 云优化 GeoTIFF (.tif)",
    mbtiles: "MBTiles 离线瓦片库 (.mbtiles)",
    zip: "ZIP 标准瓦片包 (.zip)",
    png: "PNG 整幅大图 + .pgw/.prj",
    jpg: "JPEG 整幅大图 + .jgw/.prj",
    envi: "ENVI 标准栅格 (.bin + .hdr)",
    vrt: "GDAL VRT 虚拟镶嵌 (.vrt)",
  },
  en: {
    gtiff: "GeoTIFF Mosaic (.tif)",
    cog: "Cloud-Optimized GeoTIFF (.tif)",
    mbtiles: "MBTiles Offline Tile DB (.mbtiles)",
    zip: "ZIP Raw Tile Archive (.zip)",
    png: "PNG Full Mosaic + .pgw/.prj",
    jpg: "JPEG Full Mosaic + .jgw/.prj",
    envi: "ENVI Standard Raster (.bin + .hdr)",
    vrt: "GDAL VRT Virtual Mosaic (.vrt)",
  },
  ja: {
    gtiff: "GeoTIFF 標準大判 (.tif)",
    cog: "COG クラウド最適化 GeoTIFF (.tif)",
    mbtiles: "MBTiles オフラインタイル (.mbtiles)",
    zip: "ZIP タイルパッケージ (.zip)",
    png: "PNG 一括画像 + .pgw/.prj",
    jpg: "JPEG 一括画像 + .jgw/.prj",
    envi: "ENVI 標準ラスタ (.bin + .hdr)",
    vrt: "GDAL VRT 仮想モザイク (.vrt)",
  },
};

export const XYZ_COMPRESSION_LOCALE: Record<Locale, Record<string, string>> = {
  "zh-CN": {
    DEFLATE: "DEFLATE (无损高压缩·推荐)",
    LZW: "LZW (通用 GIS 无损压缩)",
    JPEG: "JPEG (有损高倍率压缩)",
    WEBP: "WEBP 高倍率有损/无损压缩",
    NONE: "NONE (不压缩)",
  },
  en: {
    DEFLATE: "DEFLATE (Lossless · Recommended)",
    LZW: "LZW (Standard GIS Lossless)",
    JPEG: "JPEG (High-ratio Lossy)",
    WEBP: "WEBP (Lossy/Lossless)",
    NONE: "NONE (Uncompressed)",
  },
  ja: {
    DEFLATE: "DEFLATE（可逆高圧縮・推奨）",
    LZW: "LZW（汎用 GIS 可逆圧縮）",
    JPEG: "JPEG（非可逆高倍率圧縮）",
    WEBP: "WEBP（高効率可逆・非可逆）",
    NONE: "NONE（無圧縮）",
  },
};

export const XYZ_PRESET_BASEMAPS_LOCALE: Record<Locale, Record<string, string>> = {
  "zh-CN": {
    arcgis_satellite: "Esri World Imagery · 全球高清卫星影像",
    esri_wayback: "Esri Wayback · 历史影像存档 (2014–至今)",
    google_satellite: "Google Satellite · 全球高清卫星影像",
    google_hybrid: "Google Hybrid · 卫星影像+地名路网标注",
    gaode_satellite: "高德卫星影像 · 国内高清 (GCJ-02 自动纠偏)",
    tianditu_satellite: "天地图卫星影像 · 国家地理信息公共服务平台",
    arcgis_topo: "Esri World Topographic · 全球地形底图",
    opentopomap: "OpenTopoMap · 全球等高线地形图",
    arcgis_street: "Esri World Street · 全球街道地图",
    google_map: "Google Maps · 全球电子地图",
    gaode_map: "高德电子地图 · 国内路网 (GCJ-02 自动纠偏)",
    tianditu_vector: "天地图矢量底图 · CGCS2000 权威矢量",
    osm: "OpenStreetMap · 标准开源街道图",
    carto_light: "Carto Light · 极简浅色科研制图底图",
    carto_dark: "Carto Dark · 极简深色科研制图底图",
  },
  en: {
    arcgis_satellite: "Esri World Imagery · Global Hi-Res Satellite",
    esri_wayback: "Esri Wayback · Historical Satellite Archive (2014–Present)",
    google_satellite: "Google Satellite · Global Hi-Res Satellite",
    google_hybrid: "Google Hybrid · Satellite + Road & Place Labels",
    gaode_satellite: "AMap Satellite · China Hi-Res (Auto GCJ-02 Offset)",
    tianditu_satellite: "Tianditu Satellite · National Geomatics Platform",
    arcgis_topo: "Esri World Topographic · Global Topo Map",
    opentopomap: "OpenTopoMap · Global Contour Map",
    arcgis_street: "Esri World Street · Global Street Map",
    google_map: "Google Maps · Global Vector Basemap",
    gaode_map: "AMap Street · China Road Map (Auto GCJ-02 Offset)",
    tianditu_vector: "Tianditu Vector · CGCS2000 Official Map",
    osm: "OpenStreetMap · Open Street Map",
    carto_light: "Carto Light · Minimal Light Basemap",
    carto_dark: "Carto Dark · Minimal Dark Basemap",
  },
  ja: {
    arcgis_satellite: "Esri World Imagery · 全球高精細衛星画像",
    esri_wayback: "Esri Wayback · 過去衛星画像アーカイブ (2014年〜現在)",
    google_satellite: "Google Satellite · 全球衛星画像",
    google_hybrid: "Google Hybrid · 衛星画像＋地名・道路ラベル",
    gaode_satellite: "高徳衛星画像 · 中国本土高精細 (GCJ-02 自動補正)",
    tianditu_satellite: "天地図衛星画像 · 国家地理情報プラットフォーム",
    arcgis_topo: "Esri World Topographic · 全球地形図",
    opentopomap: "OpenTopoMap · 全球等高線地形図",
    arcgis_street: "Esri World Street · 全球市街地図",
    google_map: "Google Maps · 全球電子地図",
    gaode_map: "高徳電子地図 · 中国本土道路網 (GCJ-02 自動補正)",
    tianditu_vector: "天地図ベクター背景図 · CGCS2000",
    osm: "OpenStreetMap · 標準オープン地図",
    carto_light: "Carto Light · シンプルライト背景図",
    carto_dark: "Carto Dark · シンプルダーク背景図",
  },
};
