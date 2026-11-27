import type { Locale } from "../i18n/types.ts";
import type { CatalogItemMetadata } from "./dataSourceCatalogLogic.ts";
import type { GeeCatalogTheme, GeePublicDatasetEntry } from "./geePublicCatalogIndex.ts";

export interface CatalogItemTranslation {
  name: string;
  subtitle: string;
  band?: string;
  era?: string;
  provider?: string;
  satellites?: string;
  timeRange?: string;
  accountText: string;
  description: string;
  filters: string[];
  companionDownloads?: string[];
  usageNotes?: string[];
}

export const CATALOG_ITEM_TRANSLATIONS: Record<
  string,
  Partial<Record<Exclude<Locale, "zh-CN">, CatalogItemTranslation>>
> = {
  "science.sentinel-1.standard.v1": {
    en: {
      name: "Sentinel-1",
      subtitle: "C-band · 2014–Present · Earthdata",
      band: "C-band (5.405 GHz)",
      timeRange: "2014-10 to Present",
      accountText: "Earthdata",
      description:
        "All-weather C-band SAR imagery widely used for surface deformation, flood mapping, and change detection.",
      filters: ["Product Type", "Polarization", "Orbit Direction", "Beam Mode", "Platform", "Path", "Frame"],
      companionDownloads: ["Precise Orbit", "GACOS Tropospheric Delay", "ERA5 Reanalysis"],
    },
    ja: {
      name: "Sentinel-1",
      subtitle: "Cバンド · 2014年〜現在 · Earthdata",
      band: "Cバンド (5.405 GHz)",
      timeRange: "2014年10月〜現在",
      accountText: "Earthdata",
      description: "雲や雨の影響を受けない全天候型 C バンド SAR。地殻変動・洪水・変化抽出に広く利用されます。",
      filters: ["プロダクトタイプ", "偏波", "軌道方向", "ビームモード", "衛星", "相対軌道 (Path)", "Frame"],
      companionDownloads: ["精密軌道", "GACOS 大気遅延", "ERA5 大気再解析"],
    },
  },
  "science.sentinel-1.slc-burst.v1": {
    en: {
      name: "Sentinel-1 SLC Burst",
      subtitle: "C-band · Single Burst · Earthdata",
      band: "C-band (5.405 GHz)",
      timeRange: "2014-10 to Present",
      accountText: "Earthdata",
      description: "Sub-scene single-burst SLC extraction for burst-level InSAR, greatly reducing download size and storage.",
      filters: ["Polarization", "Orbit Direction", "Beam Mode", "Platform"],
      companionDownloads: ["Precise Orbit", "GACOS Tropospheric Delay", "ERA5 Reanalysis"],
    },
    ja: {
      name: "Sentinel-1 SLC Burst",
      subtitle: "Cバンド · 単一バースト · Earthdata",
      band: "Cバンド (5.405 GHz)",
      timeRange: "2014年10月〜現在",
      accountText: "Earthdata",
      description: "単一バースト単位の SLC 取得モード。バースト干渉解析向けに通信量と保存容量を大幅に削減します。",
      filters: ["偏波", "軌道方向", "ビームモード", "衛星"],
      companionDownloads: ["精密軌道", "GACOS 大気遅延", "ERA5 大気再解析"],
    },
  },
  "science.opera.disp-s1.v1": {
    en: {
      name: "NASA OPERA Surface Displacement & Coregistered SLC (DISP-S1 / CSLC)",
      subtitle: "C-band · 2016–Present · ASF / Earthdata · Pre-coregistered products",
      band: "C-band (5.405 GHz)",
      timeRange: "2016 to Present",
      accountText: "Earthdata",
      description:
        "NASA JPL OPERA Sentinel-1 coregistered SLC (CSLC-S1), radiometric terrain-corrected backscatter (RTC-S1), and line-of-sight surface displacement (DISP-S1).",
      filters: ["Product Subtype (DISP-S1 / CSLC-S1 / RTC-S1)", "Burst / Frame ID", "Orbit Direction"],
      companionDownloads: ["ERA5 Reanalysis", "GACOS Tropospheric Delay"],
    },
    ja: {
      name: "NASA OPERA 地表変位・位置合わせ済み SLC (DISP-S1 / CSLC)",
      subtitle: "Cバンド · 2016年〜現在 · ASF / Earthdata · 解析済み干渉プロダクト",
      band: "Cバンド (5.405 GHz)",
      timeRange: "2016年〜現在",
      accountText: "Earthdata",
      description:
        "NASA JPL OPERA による Sentinel-1 位置合わせ済み複素画像 (CSLC-S1)、地形放射補正 (RTC-S1)、視線方向変位プロダクト (DISP-S1) を直接取得できます。",
      filters: ["サブタイプ (DISP-S1 / CSLC-S1 / RTC-S1)", "Burst ID / Frame ID", "軌道方向"],
      companionDownloads: ["ERA5 大気再解析", "GACOS 大気遅延"],
    },
  },
  "science.comet.licsar.v1": {
    en: {
      name: "COMET-LiCSAR Global Interferograms & Coherence",
      subtitle: "C-band · 2014–Present · COMET (Univ. of Leeds) · Account-free",
      band: "C-band (Wrapped / Unwrapped Phase & Coherence)",
      timeRange: "2014-10 to Present",
      accountText: "No account required",
      description:
        "Automated Sentinel-1 standard-frame InSAR interferograms (.geo.unw.tif / .geo.cc.tif) across tectonic belts by COMET, ready for rapid fault, volcano, and landslide analysis.",
      filters: ["LiCSAR Frame ID", "Interferometric Date Pair", "Output Layer"],
      companionDownloads: ["GACOS Tropospheric Delay", "ERA5 Reanalysis"],
    },
    ja: {
      name: "COMET-LiCSAR 全球干渉画像・コヒーレンス",
      subtitle: "Cバンド · 2014年〜現在 · COMET (リーズ大学) · アカウント不要",
      band: "Cバンド (干渉位相 / アンラップ位相 / コヒーレンス)",
      timeRange: "2014年10月〜現在",
      accountText: "アカウント不要",
      description:
        "英国 COMET が自動生成する変動帯 Sentinel-1 標準フレーム干渉プロダクト (.geo.unw.tif / .geo.cc.tif)。アカウント不要で断層・火山・地すべり監視に最適です。",
      filters: ["LiCSAR Frame ID", "干渉ペア日付", "出力レイヤー"],
      companionDownloads: ["GACOS 大気遅延", "ERA5 大気再解析"],
    },
  },
  "science.nisar.l.rslc.provisional.v1": {
    en: {
      name: "NISAR L-band SAR",
      subtitle: "L-band · 2025–Present · Earthdata · Online Subset Ready",
      band: "L-band (1.25 GHz)",
      timeRange: "2025 to Present",
      accountText: "Earthdata",
      description:
        "NASA-ISRO L-band SAR suite (RSLC / GSLC / GCOV / GUNW / GOFF / RIFG / RUNW / ROFF / SME2) with strong canopy penetration and HTTP Range AOI subsetting.",
      filters: ["Product Level", "Polarization", "Orbit Direction", "Path", "Frame", "AOI Subset"],
    },
    ja: {
      name: "NISAR Lバンド SAR",
      subtitle: "Lバンド · 2025年〜現在 · Earthdata · オンライン切り抜き対応",
      band: "Lバンド (1.25 GHz)",
      timeRange: "2025年〜現在",
      accountText: "Earthdata",
      description:
        "NASA-ISRO 合成開口レーダー L バンド全レベル (RSLC/GSLC/GCOV/GUNW/GOFF/RIFG/RUNW/ROFF/SME2)。完全版および HTTP Range による AOI 部分取得に対応。",
      filters: ["プロダクトレベル", "偏波", "軌道方向", "相対軌道 (Path)", "Frame", "オンライン AOI 切り抜き"],
    },
  },
  "science.nisar.s.ssar.rslc.v1": {
    en: {
      name: "NISAR S-band SAR",
      subtitle: "S-band · 2025–Present · Bhoonidhi · Online Subset Ready",
      band: "S-band (3.2 GHz)",
      timeRange: "2025 to Present",
      accountText: "Bhoonidhi",
      description:
        "ISRO S-band SAR products (RSLC / GSLC / GCOV / GUNW / GOFF / RIFG / RUNW / ROFF) for agriculture, soil moisture, and near-surface scattering, supporting AOI subsetting.",
      filters: ["Polarization", "Orbit Direction", "AOI Subset"],
    },
    ja: {
      name: "NISAR Sバンド SAR",
      subtitle: "Sバンド · 2025年〜現在 · Bhoonidhi · オンライン切り抜き対応",
      band: "Sバンド (3.2 GHz)",
      timeRange: "2025年〜現在",
      accountText: "Bhoonidhi",
      description:
        "ISRO 提供の S バンド SAR 製品群。農地・土壌水分・表層散乱観測向けで、完全版ダウンロードとオンライン AOI 切り抜きに対応します。",
      filters: ["偏波", "軌道方向", "オンライン AOI 切り抜き"],
    },
  },
  "science.alos-2.palsar-2.scansar-l1.1.v1": {
    en: {
      name: "ALOS-2 PALSAR-2 ScanSAR",
      subtitle: "L-band · 2014–Present · Earthdata",
      band: "L-band (1.27 GHz)",
      timeRange: "2014-05 to Present",
      accountText: "Earthdata",
      description: "JAXA wide-swath L-band SAR with high temporal coherence over vegetated terrain for regional deformation monitoring.",
      filters: ["Polarization", "Orbit Direction", "Beam Mode", "Look Direction", "Path", "Frame"],
    },
    ja: {
      name: "ALOS-2 PALSAR-2 ScanSAR",
      subtitle: "Lバンド · 2014年〜現在 · Earthdata",
      band: "Lバンド (1.27 GHz)",
      timeRange: "2014年5月〜現在",
      accountText: "Earthdata",
      description: "JAXA の広域 L バンド SAR。植生域でも高いコヒーレンスを保ち、広域の地殻・地盤変動解析に適しています。",
      filters: ["偏波", "軌道方向", "ビームモード", "視線方向", "Path", "Frame"],
    },
  },
  "science.alos.palsar.v1": {
    en: {
      name: "ALOS PALSAR",
      subtitle: "L-band · 2006–2011 · Earthdata",
      band: "L-band",
      timeRange: "2006–2011",
      accountText: "Earthdata",
      description: "Historical L-band SAR archive from JAXA's ALOS satellite for retrospective deformation studies.",
      filters: ["Polarization", "Orbit Direction", "Look Direction", "Path", "Frame"],
    },
    ja: {
      name: "ALOS PALSAR",
      subtitle: "Lバンド · 2006–2011年 · Earthdata",
      band: "Lバンド",
      timeRange: "2006–2011年",
      accountText: "Earthdata",
      description: "初代だいち (ALOS) の L バンド SAR アーカイブ。過去の長期変位レトロスペクティブ解析に適しています。",
      filters: ["偏波", "軌道方向", "視線方向", "Path", "Frame"],
    },
  },
  "science.seasat.sar.v1": {
    en: {
      name: "SEASAT",
      subtitle: "L-band · 1978 · Earthdata",
      band: "L-band",
      timeRange: "1978",
      accountText: "Earthdata",
      description: "First civilian spaceborne synthetic aperture radar oceanography mission (1978 historical baseline).",
      filters: ["Polarization", "Orbit Direction"],
    },
    ja: {
      name: "SEASAT",
      subtitle: "Lバンド · 1978年 · Earthdata",
      band: "Lバンド",
      timeRange: "1978年",
      accountText: "Earthdata",
      description: "世界初の民生用合成開口レーダー搭載衛星。1978 年の貴重な歴史的基準アーカイブです。",
      filters: ["偏波", "軌道方向"],
    },
  },
  "science.sentinel-2.l2a.v1": {
    en: {
      name: "Sentinel-2",
      subtitle: "MSI Multispectral · L2A BOA / L1C TOA · CDSE / MPC / Earth Search",
      band: "MSI Multispectral (13 bands · 10m–60m)",
      timeRange: "2015-06 to Present",
      accountText: "CDSE Account / MPC · Earth Search Free",
      description:
        "Copernicus Sentinel-2 multispectral imagery (13 bands, 10m–60m) with toggleable L2A / L1C processing levels and CDSE SAFE or account-free COG channels.",
      filters: ["L2A / L1C Level", "Max Cloud Cover", "Satellite Platform"],
    },
    ja: {
      name: "Sentinel-2",
      subtitle: "MSI マルチスペクトル · L2A 地表面反射率 / L1C TOA · CDSE / MPC / Earth Search",
      band: "MSI マルチスペクトル (13 バンド · 10m–60m)",
      timeRange: "2015年6月〜現在",
      accountText: "CDSE アカウント / MPC · Earth Search 不要",
      description:
        "Copernicus Sentinel-2 マルチスペクトル光学画像 (10m–60m)。検索欄から L2A と L1C、CDSE 原版 SAFE とアカウント不要の COG を切替できます。",
      filters: ["L2A / L1C レベル", "最大雲量", "衛星プラットフォーム"],
    },
  },
  "science.sentinel-2.l1c.v1": {
    en: {
      name: "Sentinel-2 L1C",
      subtitle: "MSI Multispectral · Level-1C Top-of-Atmosphere · CDSE",
      band: "MSI Multispectral (13 bands · 10m–60m)",
      timeRange: "2015-06 to Present",
      accountText: "CDSE Account",
      description:
        "Sentinel-2 L1C orthorectified Top-of-Atmosphere (TOA) reflectance with radiometric fidelity for custom atmospheric correction.",
      filters: ["L1C TOA Level", "Max Cloud Cover", "Satellite Platform"],
    },
    ja: {
      name: "Sentinel-2 L1C",
      subtitle: "MSI マルチスペクトル · L1C 大気上端反射率 (TOA) · CDSE",
      band: "MSI マルチスペクトル (13 バンド · 10m–60m)",
      timeRange: "2015年6月〜現在",
      accountText: "CDSE アカウント",
      description: "幾何オルソ補正済みの Sentinel-2 L1C TOA プロダクト。独自の大気補正や放射校正向けで AOI 切り抜きに対応します。",
      filters: ["L1C TOA 反射率", "最大雲量", "衛星プラットフォーム"],
    },
  },
  "science.landsat.c2-l2.v1": {
    en: {
      name: "Landsat 4-9 C2-L2",
      subtitle: "Optical Multispectral · 1982–Present · USGS / MPC",
      band: "Optical (30m resolution)",
      timeRange: "1982-07 to Present",
      accountText: "MPC Free / USGS Account",
      description:
        "USGS Collection 2 Level-2 surface reflectance (30m), supporting account-free MPC cloud-native bands with AOI clipping and official .tar archives.",
      filters: ["Max Cloud Cover", "Satellite Platform", "Product Type"],
    },
    ja: {
      name: "Landsat 4-9 C2-L2",
      subtitle: "光学マルチスペクトル · 1982年〜現在 · USGS / MPC",
      band: "光学 (解像度 30m)",
      timeRange: "1982年7月〜現在",
      accountText: "MPC アカウント不要 / USGS アカウント",
      description:
        "USGS Collection 2 Level-2 地表面反射率 (30m)。アカウント不要の MPC 単バンド COG (AOI 切り抜き対応) と USGS 公式 .tar の両方をサポートします。",
      filters: ["最大雲量", "衛星プラットフォーム", "プロダクトタイプ"],
    },
  },
  "science.sentinel-3.standard.v1": {
    en: {
      name: "Sentinel-3 OLCI / SLSTR / SRAL",
      subtitle: "Ocean & Land Observation · 2016–Present · Copernicus CDSE",
      band: "Optical / Thermal IR / Altimetry",
      timeRange: "2016-02 to Present",
      accountText: "CDSE Account",
      description: "ESA ocean and land mission carrying OLCI ocean/land color, SLSTR thermal radiometer, and SRAL radar altimeter.",
      filters: ["Max Cloud Cover", "Satellite Platform", "Product Type"],
    },
    ja: {
      name: "Sentinel-3 OLCI / SLSTR / SRAL",
      subtitle: "海洋・陸域統合観測 · 2016年〜現在 · Copernicus CDSE",
      band: "光学 / 熱赤外 / 高度計",
      timeRange: "2016年2月〜現在",
      accountText: "CDSE アカウント",
      description: "OLCI 海色・陸域センサー、SLSTR 熱赤外放射計、SRAL レーダー高度計を搭載した欧州環境観測衛星です。",
      filters: ["最大雲量", "衛星プラットフォーム", "プロダクトタイプ"],
    },
  },
  "science.hls.l30.v2": {
    en: {
      name: "HLS L30 (Landsat)",
      subtitle: "Optical Multispectral · 2013–Present · NASA LP DAAC / MPC",
      band: "Optical (30m MGRS grid)",
      timeRange: "2013 to Present",
      accountText: "MPC Free",
      description:
        "Harmonized Landsat 8-9 30m surface reflectance forming a 2–3 day revisit series with S30, with online AOI clipping and mosaic export.",
      filters: ["Max Cloud Cover"],
    },
    ja: {
      name: "HLS L30 (Landsat)",
      subtitle: "光学マルチスペクトル · 2013年〜現在 · NASA LP DAAC / MPC",
      band: "光学 (30m グリッド)",
      timeRange: "2013年〜現在",
      accountText: "MPC アカウント不要",
      description: "S30 と統合された 30m Landsat 8-9 地表面反射率データセット。AOI 切り抜きと近同時期モザイクに対応します。",
      filters: ["最大雲量"],
    },
  },
  "science.hls.s30.v2": {
    en: {
      name: "HLS S30 (Sentinel-2)",
      subtitle: "Optical Multispectral · 2015–Present · NASA LP DAAC / MPC",
      band: "Optical (30m MGRS grid)",
      timeRange: "2015 to Present",
      accountText: "MPC Free",
      description:
        "Harmonized Sentinel-2 30m surface reflectance with atmospheric correction and co-registration, supporting online AOI clipping and mosaic export.",
      filters: ["Max Cloud Cover"],
    },
    ja: {
      name: "HLS S30 (Sentinel-2)",
      subtitle: "光学マルチスペクトル · 2015年〜現在 · NASA LP DAAC / MPC",
      band: "光学 (30m グリッド)",
      timeRange: "2015年〜現在",
      accountText: "MPC アカウント不要",
      description: "大気補正と幾何整合済みの 30m Sentinel-2 地表面反射率データセット。AOI 切り抜きとモザイク合成に対応します。",
      filters: ["最大雲量"],
    },
  },
  "dem.cop30.v1": {
    en: {
      name: "Copernicus DEM GLO-30 (30m)",
      subtitle: "30m Resolution · EGM2008 Orthometric · Account-free",
      band: "30m (1 arcsec · EGM2008 Orthometric)",
      satellites: "TanDEM-X derived",
      timeRange: "Global (2011–2015 TanDEM-X baseline)",
      accountText: "No account required",
      description:
        "ESA TanDEM-X-derived global 30m DEM (EGM2008 orthometric height, auto-converted to WGS84 ellipsoid height on demand) — premier reference DEM for InSAR.",
      filters: ["Spatial Extent", "Convert to WGS84 Ellipsoidal Height"],
    },
    ja: {
      name: "Copernicus DEM GLO-30（30m）",
      subtitle: "解像度 30m · EGM2008 正標高 · アカウント不要",
      band: "30m (1秒角 · EGM2008 正標高)",
      satellites: "TanDEM-X 派生",
      timeRange: "全球対応 (2011–2015年 TanDEM-X 基準)",
      accountText: "アカウント不要",
      description:
        "TanDEM-X 干渉計測に基づく 30m 全球 DEM。WGS84 楕円体高への自動変換に対応し、InSAR 地形位相除去の第一候補です。",
      filters: ["空間範囲", "WGS84 楕円体高変換"],
    },
  },
  "dem.cop90.v1": {
    en: {
      name: "Copernicus DEM GLO-90 (90m)",
      subtitle: "90m Resolution · EGM2008 Orthometric · Account-free",
      band: "90m (3 arcsec · EGM2008 Orthometric)",
      satellites: "TanDEM-X derived",
      timeRange: "Global (2011–2015 TanDEM-X baseline)",
      accountText: "No account required",
      description: "Global 90m aggregated Copernicus DEM with optional WGS84 ellipsoidal conversion for continental-scale processing.",
      filters: ["Spatial Extent", "Convert to WGS84 Ellipsoidal Height"],
    },
    ja: {
      name: "Copernicus DEM GLO-90（90m）",
      subtitle: "解像度 90m · EGM2008 正標高 · アカウント不要",
      band: "90m (3秒角 · EGM2008 正標高)",
      satellites: "TanDEM-X 派生",
      timeRange: "全球対応 (2011–2015年 TanDEM-X 基準)",
      accountText: "アカウント不要",
      description: "GLO-30 を集約した 90m 全球 DEM。広域の高速処理解析や背景地形補正に適しています。",
      filters: ["空間範囲", "WGS84 楕円体高変換"],
    },
  },
  "dem.nasadem.v1": {
    en: {
      name: "NASADEM Global 30m",
      subtitle: "30m Resolution · EGM96 Orthometric · Earthdata / OpenTopography",
      band: "30m (1 arcsec · EGM96 Orthometric)",
      satellites: "SRTM / ASTER fusion",
      timeRange: "60°N – 56°S",
      accountText: "OpenTopography API Key / Earthdata",
      description: "NASA reprocessed SRTM 30m void-filled global elevation fused with ASTER GDEM and ICESat laser altimetry.",
      filters: ["Spatial Extent", "Convert to WGS84 Ellipsoidal Height"],
    },
    ja: {
      name: "NASADEM Global 30m",
      subtitle: "解像度 30m · EGM96 正標高 · Earthdata / OpenTopography",
      band: "30m (1秒角 · EGM96 正標高)",
      satellites: "SRTM / ASTER 統合",
      timeRange: "北緯 60°〜南緯 56°",
      accountText: "OpenTopography API Key / Earthdata",
      description: "SRTM を再処理し ASTER・ICESat で欠損補間した NASA の 30m シームレス全球 DEM です。",
      filters: ["空間範囲", "WGS84 楕円体高変換"],
    },
  },
  "dem.aw3d30.v1": {
    en: {
      name: "ALOS World 3D 30m (Orthometric)",
      subtitle: "30m Resolution · EGM96 Orthometric · OpenTopography",
      band: "30m (1 arcsec · EGM96 Orthometric)",
      timeRange: "Global Land (82°N–82°S)",
      accountText: "OpenTopography API Key",
      description: "JAXA global 30m DSM derived from ALOS PRISM optical stereo pairs (EGM96 orthometric height).",
      filters: ["Spatial Extent", "Convert to WGS84 Ellipsoidal Height"],
    },
    ja: {
      name: "ALOS World 3D 30m（正標高）",
      subtitle: "解像度 30m · EGM96 正標高 · OpenTopography",
      band: "30m (1秒角 · EGM96 正標高)",
      timeRange: "全球陸域 (82°N–82°S)",
      accountText: "OpenTopography API Key",
      description: "JAXA が ALOS PRISM ステレオペアから作成した 30m 全球数値表層モデル (EGM96 正標高) です。",
      filters: ["空間範囲", "WGS84 楕円体高変換"],
    },
  },
  "dem.aw3d30-ellipsoidal.v1": {
    en: {
      name: "ALOS World 3D 30m (Ellipsoidal)",
      subtitle: "30m Resolution · WGS84 Ellipsoid · OpenTopography",
      band: "30m (1 arcsec · WGS84 Ellipsoidal)",
      timeRange: "Global Land (82°N–82°S)",
      accountText: "OpenTopography API Key",
      description: "Native WGS84 ellipsoidal height version of JAXA ALOS World 3D 30m, ready directly for InSAR geometric coregistration.",
      filters: ["Spatial Extent"],
    },
    ja: {
      name: "ALOS World 3D 30m（楕円体高）",
      subtitle: "解像度 30m · WGS84 楕円体高 · OpenTopography",
      band: "30m (1秒角 · WGS84 楕円体高)",
      timeRange: "全球陸域 (82°N–82°S)",
      accountText: "OpenTopography API Key",
      description: "JAXA AW3D30 のネイティブ WGS84 楕円体高版。変換なしでそのまま InSAR 幾何位置合わせに利用できます。",
      filters: ["空間範囲"],
    },
  },
  "dem.nisar-dem.v1": {
    en: {
      name: "NISAR Reference DEM (Ellipsoidal · 30m)",
      subtitle: "30m Resolution · WGS84 Ellipsoid · Earthdata",
      band: "30m (1 arcsec · WGS84 Ellipsoidal)",
      timeRange: "Global",
      accountText: "Earthdata",
      description: "Official NASA/ISRO NISAR calibration DEM pre-converted from Copernicus GLO-30 to WGS84 ellipsoid with void filling.",
      filters: ["Spatial Extent"],
    },
    ja: {
      name: "NISAR 科学参照 DEM（楕円体高 · 30m）",
      subtitle: "解像度 30m · WGS84 楕円体高 · Earthdata",
      band: "30m (1秒角 · WGS84 楕円体高)",
      timeRange: "全球",
      accountText: "Earthdata",
      description: "Copernicus GLO-30 を基に WGS84 楕円体高へ事前変換・欠損補間した NISAR 公式参照 DEM です。",
      filters: ["空間範囲"],
    },
  },
  "dem.fabdem.v1": {
    en: {
      name: "FABDEM V1-2 30m (Bare-earth DTM)",
      subtitle: "30m Resolution · EGM2008 Bare-earth · Account-free",
      band: "30m (1 arcsec · Forests/Buildings Removed)",
      timeRange: "60°S–80°N Land (2023)",
      accountText: "No account required",
      description: "Bare-earth 30m DTM removing forest canopy and buildings from Copernicus GLO-30 (EGM2008 orthometric).",
      filters: ["Spatial Extent", "Convert to WGS84 Ellipsoidal Height"],
      usageNotes: [
        "No tiles above 80°N, below 60°S, or over open oceans.",
        "Licensed under CC BY-NC-SA 4.0 (non-commercial research and education only; use Copernicus GLO-30 for commercial projects).",
      ],
    },
    ja: {
      name: "FABDEM V1-2 30m（裸地 DTM · 30m）",
      subtitle: "解像度 30m · EGM2008 裸地正標高 · アカウント不要",
      band: "30m (1秒角 · 樹冠・建物除去)",
      timeRange: "南緯 60°〜北緯 80° 陸域 (2023年)",
      accountText: "アカウント不要",
      description: "Copernicus GLO-30 から森林樹冠と建築物高を機械学習で除去した 30m 裸地数値地形モデルです。",
      filters: ["空間範囲", "WGS84 楕円体高変換"],
      usageNotes: [
        "欠損範囲：北緯 80° 以北・南緯 60° 以南および海域にはタイルがありません。",
        "ライセンス制限：CC BY-NC-SA 4.0（非商用研究・教育限定）。商用利用時は Copernicus DEM GLO-30 を選択してください。",
      ],
    },
  },
  "dem.gem15-dtm.v1": {
    en: {
      name: "Google GEM-15 DTM 15m (Bare-earth)",
      subtitle: "15m (0.5 arcsec) · EGM96 Bare-earth · Account-free",
      band: "15m (0.5 arcsec · Bare-earth DTM)",
      timeRange: "61°S–84°N (2018–2025)",
      accountText: "No account required",
      description:
        "Google Research 15m bare-earth DTM v1 with automatic UTM-to-WGS84 mosaicking and optional EGM96-to-ellipsoid conversion.",
      filters: ["Spatial Extent", "Convert to WGS84 Ellipsoidal Height"],
      usageNotes: [
        "Official v1 excludes Mainland China, Russia, Mongolia, North Korea, Greenland, and Antarctica; use Copernicus GLO-30 or FABDEM there.",
        "CC BY 4.0 license supporting commercial and research use (attribute Google).",
      ],
    },
    ja: {
      name: "Google GEM-15 DTM 15m（裸地正標高 · 15m）",
      subtitle: "解像度 15m (0.5秒角) · EGM96 裸地正標高 · アカウント不要",
      band: "15m (0.5秒角 · 裸地 DTM)",
      timeRange: "61°S–84°N (2018–2025年合成)",
      accountText: "アカウント不要",
      description: "Google Research による 15m 裸地地形モデル。UTM→WGS84 再投影および楕円体高変換に自動対応します。",
      filters: ["空間範囲", "WGS84 楕円体高変換"],
      usageNotes: [
        "提供範囲制限：公式 v1 には中国本土、ロシア、モンゴル、北朝鮮、グリーンランド、南極は含まれません。",
        "ライセンス：CC BY 4.0（商用・研究利用可、要 Google クレジット表記）。",
      ],
    },
  },
  "dem.gem15-dsm.v1": {
    en: {
      name: "Google GEM-15 DSM 15m (Surface)",
      subtitle: "15m (0.5 arcsec) · EGM96 Surface DSM · Account-free",
      band: "15m (0.5 arcsec · Surface DSM)",
      timeRange: "61°S–84°N (2018–2025)",
      accountText: "No account required",
      description:
        "Google Research 15m Digital Surface Model (preserving canopy/buildings) with automatic UTM-to-WGS84 reprojection and EGM96 conversion.",
      filters: ["Spatial Extent", "Convert to WGS84 Ellipsoidal Height"],
      usageNotes: [
        "Official v1 excludes Mainland China, Russia, Mongolia, North Korea, Greenland, and Antarctica.",
        "CC BY 4.0 license supporting commercial and research use (attribute Google).",
      ],
    },
    ja: {
      name: "Google GEM-15 DSM 15m（表層正標高 · 15m）",
      subtitle: "解像度 15m (0.5秒角) · EGM96 表層正標高 · アカウント不要",
      band: "15m (0.5秒角 · 表層 DSM)",
      timeRange: "61°S–84°N (2018–2025年合成)",
      accountText: "アカウント不要",
      description: "樹木・建築物高を含む Google Research の 15m 数値表層モデル。UTM→WGS84 変換と楕円体高補正に自動対応します。",
      filters: ["空間範囲", "WGS84 楕円体高変換"],
      usageNotes: [
        "提供範囲制限：公式 v1 には中国本土、ロシア、モンゴル、北朝鮮、グリーンランド、南極は含まれません。",
        "ライセンス：CC BY 4.0（商用・研究利用可、要 Google クレジット表記）。",
      ],
    },
  },
  "dem.srtm-gl1.v1": {
    en: {
      name: "SRTM GL1 Global 30m (Orthometric)",
      subtitle: "30m Resolution · EGM96 Orthometric · OpenTopography",
      band: "30m (1 arcsec · EGM96 Orthometric)",
      timeRange: "60°N – 56°S",
      accountText: "OpenTopography API Key",
      description: "NASA SRTM Version 3 global 30m elevation baseline (EGM96 orthometric, auto-convertible to WGS84 ellipsoid).",
      filters: ["Spatial Extent", "Convert to WGS84 Ellipsoidal Height"],
    },
    ja: {
      name: "SRTM GL1 Global 30m（正標高）",
      subtitle: "解像度 30m · EGM96 正標高 · OpenTopography",
      band: "30m (1秒角 · EGM96 正標高)",
      timeRange: "北緯 60°〜南緯 56°",
      accountText: "OpenTopography API Key",
      description: "NASA SRTM v3 による 30m 全球地形データ (EGM96 正標高)。WGS84 楕円体高への変換に対応しています。",
      filters: ["空間範囲", "WGS84 楕円体高変換"],
    },
  },
  "dem.srtm-gl1-ellipsoidal.v1": {
    en: {
      name: "SRTM GL1 Global 30m (Ellipsoidal)",
      subtitle: "30m Resolution · WGS84 Ellipsoid · OpenTopography",
      band: "30m (1 arcsec · WGS84 Ellipsoidal)",
      timeRange: "60°N – 56°S",
      accountText: "OpenTopography API Key",
      description: "WGS84 ellipsoidal height version of SRTM GL1 30m with EGM96 geoid undulation removed for InSAR radar coregistration.",
      filters: ["Spatial Extent"],
    },
    ja: {
      name: "SRTM GL1 Global 30m（楕円体高）",
      subtitle: "解像度 30m · WGS84 楕円体高 · OpenTopography",
      band: "30m (1秒角 · WGS84 楕円体高)",
      timeRange: "北緯 60°〜南緯 56°",
      accountText: "OpenTopography API Key",
      description: "EGM96 ジオイド高を除去した SRTM GL1 の WGS84 楕円体高版。InSAR 位置合わせに直接利用できます。",
      filters: ["空間範囲"],
    },
  },
  "dem.srtm-gl3.v1": {
    en: {
      name: "SRTM GL3 Global 90m",
      subtitle: "90m Resolution · EGM96 Orthometric · OpenTopography",
      band: "90m (3 arcsec · EGM96 Orthometric)",
      timeRange: "60°N – 56°S",
      accountText: "OpenTopography API Key",
      description: "Lightweight global 90m SRTM v3 elevation grid covering 60°N to 56°S.",
      filters: ["Spatial Extent", "Convert to WGS84 Ellipsoidal Height"],
    },
    ja: {
      name: "SRTM GL3 Global 90m",
      subtitle: "解像度 90m · EGM96 正標高 · OpenTopography",
      band: "90m (3秒角 · EGM96 正標高)",
      timeRange: "北緯 60°〜南緯 56°",
      accountText: "OpenTopography API Key",
      description: "データ容量の小さい SRTM v3 の 90m 全球 DEM です。",
      filters: ["空間範囲", "WGS84 楕円体高変換"],
    },
  },
  "dem.aster-gdem.v3": {
    en: {
      name: "ASTER GDEM v3 Global 30m",
      subtitle: "30m Resolution · EGM96 Orthometric · NASA LP DAAC / Earthdata",
      band: "30m (1 arcsec · 83°N–83°S coverage)",
      timeRange: "83°N – 83°S",
      accountText: "Earthdata",
      description: "NASA/METI Terra ASTER GDEM v3 extending to ±83° high latitudes beyond SRTM coverage via your Earthdata account.",
      filters: ["Spatial Extent", "Convert to WGS84 Ellipsoidal Height"],
    },
    ja: {
      name: "ASTER GDEM v3 Global 30m",
      subtitle: "解像度 30m · EGM96 正標高 · NASA LP DAAC / Earthdata",
      band: "30m (1秒角 · 83°N–83°S 対応)",
      timeRange: "北緯 83°〜南緯 83°",
      accountText: "Earthdata",
      description: "SRTM の範囲を超える ±83° の高緯度域もカバーする NASA/経済産業省共同の 30m DEM (Earthdata 連携) です。",
      filters: ["空間範囲", "WGS84 楕円体高変換"],
    },
  },
  "orbit.sentinel-1.poeorb.v1": {
    en: {
      name: "Sentinel-1 Precise Orbit (POEORB/RESORB)",
      subtitle: "Centimeter Orbit · Active Search & Local Folder Matching · Account-free",
      band: "Precise Orbit",
      timeRange: "2014-10 to Present",
      accountText: "No account required",
      description:
        "ESA official Sentinel-1 orbit ephemerides (<5 cm accuracy; POEORB ~20 days post-acquisition, RESORB fallback). Supports active download, local folder matching, or companion attachment.",
      filters: ["AOI & Date Range", "Local SAR Folder Match", "Orbit Type (POEORB/RESORB)", "Platform"],
    },
    ja: {
      name: "Sentinel-1 精密軌道 (POEORB/RESORB)",
      subtitle: "cm 級精密軌道 · 条件検索＆ローカル照合 · アカウント不要",
      band: "精密軌道",
      timeRange: "2014年10月〜現在",
      accountText: "アカウント不要",
      description:
        "ESA 公式精密軌道ファイル (精度 5cm 未満)。AOI/期間指定の直接取得・ローカルフォルダ一括照合・Sentinel-1 との同時取得に対応します。",
      filters: ["AOI と期間", "ローカル SAR フォルダ照合", "軌道タイプ", "衛星"],
    },
  },
  "orbit.nisar.oe.v1": {
    en: {
      name: "NISAR Precision/Nominal Orbit Ephemeris (POE/MOE/NOE/FOE)",
      subtitle: "L/S-Band State Vectors · NASA ASF / JPL (NISAR_OE) · .xml",
      band: "Precise Orbit",
      timeRange: "2025-10 to Present",
      accountText: "Search free · Download requires Earthdata",
      description: "NISAR mission orbit ephemeris XML files (POE/MOE/NOE/FOE) queried by global time window for ISCE3 / COMPASS coregistration.",
      filters: ["Orbit Grade (POE/MOE/NOE/FOE)", "Start/End Date"],
    },
    ja: {
      name: "NISAR 精密・予報軌道暦 (POE/MOE/NOE/FOE)",
      subtitle: "L/S バンド軌道状態ベクトル · NASA ASF / JPL (NISAR_OE) · .xml",
      band: "精密軌道",
      timeRange: "2025年10月〜現在",
      accountText: "検索無料 · 取得に Earthdata 必要",
      description: "ISCE3 / COMPASS の幾何校正に用いる NISAR ミッション軌道暦 XML。期間指定で全球分を検索できます。",
      filters: ["軌道暦グレード (POE/MOE/NOE/FOE)", "期間"],
    },
  },
  "orbit.copernicus.pod.v1": {
    en: {
      name: "Sentinel-2 / Sentinel-3 Precise Orbit (POD POEORB)",
      subtitle: "Copernicus POD · S2 AUX_POEORB / S3 SR___POE_AX · CDSE",
      band: "Precise Orbit",
      timeRange: "2016-02 to Present",
      accountText: "Search free · Download requires CDSE",
      description: "Copernicus Precise Orbit Determination files for Sentinel-2 geometry refinement and Sentinel-3 altimetry.",
      filters: ["Mission Family (S2/S3)", "Start/End Date"],
    },
    ja: {
      name: "Sentinel-2 / Sentinel-3 精密軌道 (POD POEORB)",
      subtitle: "Copernicus 精密軌道決定 · S2 AUX_POEORB / S3 SR___POE_AX · CDSE",
      band: "精密軌道",
      timeRange: "2016年2月〜現在",
      accountText: "検索無料 · 取得に CDSE 必要",
      description: "Sentinel-2/3 向けの ESA Copernicus POD 公式精密軌道ファイルです。",
      filters: ["衛星ファミリー (S2/S3)", "期間"],
    },
  },
  "event.usgs.earthquake.v1": {
    en: {
      name: "USGS Global Earthquake Catalog & Coseismic AOI",
      subtitle: "Epicenter / Magnitude / ShakeMap · Real-time · Account-free",
      band: "Seismic Trigger & Coseismic Pairing",
      era: "Historical–Present",
      satellites: "Global Seismic Network (ANSS / NEIC)",
      timeRange: "Real-time & Archive",
      accountText: "No account required",
      description: "USGS FDSN earthquake catalog that auto-generates coseismic AOI buffers and pre/post-seismic SAR windows.",
      filters: ["Min Magnitude (Mw)", "Origin Time Window", "Buffer Radius"],
    },
    ja: {
      name: "USGS 全球地震カタログ・地震時領域抽出",
      subtitle: "震央・マグニチュード・ShakeMap · リアルタイム · アカウント不要",
      band: "地震トリガー・地震時ペア設定",
      era: "過去–現在",
      satellites: "全球地震観測網 (ANSS / NEIC)",
      timeRange: "リアルタイム・過去アーカイブ",
      accountText: "アカウント不要",
      description: "USGS FDSN 地震サービスから規模・時刻で検索し、地震前後の SAR 干渉ペア期間と AOI を生成します。",
      filters: ["最小規模 (Mw)", "発生期間", "バッファ半径"],
    },
  },
  "geodesy.ngl.gnss.v1": {
    en: {
      name: "NGL Global GNSS Displacement Time Series",
      subtitle: "Daily 3D Coordinates (ENU / IGS14) · 17,000+ Stations · Account-free",
      band: "Geodetic External Reference & Validation",
      timeRange: "1994 to Present",
      accountText: "No account required",
      description: "Nevada Geodetic Laboratory daily 3-component GNSS (.tenv3) time series across 17,000+ stations for InSAR reference datum and LOS validation.",
      filters: ["Stations in AOI", "Temporal Overlap", "LOS Projection"],
    },
    ja: {
      name: "NGL 全球 GNSS 基準局変位時系列",
      subtitle: "日別3次元座標 (ENU / IGS14) · 17,000+ 局 · アカウント不要",
      band: "測地基準拘束・検証",
      timeRange: "1994年〜現在",
      accountText: "アカウント不要",
      description: "ネバダ測地研究所 (NGL) の 17,000 局超の日別 GNSS 変位 (.tenv3)。InSAR 基準固定と LOS 精度検証に利用します。",
      filters: ["AOI 内観測局検索", "期間一致率", "LOS 投影"],
    },
  },
  "atmospheric.gacos.ztd.v1": {
    en: {
      name: "GACOS Atmospheric Delay Correction",
      subtitle: "Tropospheric ZTD Grids · Request by AOI/Date or Companion · Email needed",
      band: "Atmospheric Correction (~90m grid)",
      era: "Historical–Present",
      satellites: "ECMWF / HRES tropospheric model",
      timeRange: "Generated on request",
      accountText: "Notification email required",
      description:
        "Newcastle University Generic Atmospheric Correction Online Service (~90m ZTD grids). Submit requests directly by AOI/dates or attach as companion with Sentinel-1.",
      filters: ["AOI Extent", "Acquisition Dates & UTC Time", "Spatial Buffer"],
    },
    ja: {
      name: "GACOS 大気遅延補正",
      subtitle: "対流圏遅延補正 · AOI/日付指定申請＆同梱送信対応 · メール必要",
      band: "大気補正 (約 90m グリッド)",
      era: "過去–現在",
      satellites: "ECMWF / HRES 対流圏モデル",
      timeRange: "リクエスト生成",
      accountText: "通知メール要設定",
      description: "ニューカッスル大学の汎用大気遅延補正サービス。AOI と撮影日時を指定して申請でき、Sentinel-1 取得時の自動同梱にも対応します。",
      filters: ["空間範囲 (AOI)", "日時リスト", "空間バッファ"],
    },
  },
  "atmospheric.era5.reanalysis.v1": {
    en: {
      name: "ERA5 Atmospheric Reanalysis",
      subtitle: "Hourly 37 Pressure Levels · PyAPS / MintPy Input · Direct fetch",
      band: "Atmospheric (0.25° × 0.25° 3D T/q/P)",
      timeRange: "1940 to Present (hourly)",
      accountText: "ARCO-ERA5 Cloud Free / CDS",
      description: "ECMWF ERA5 3D pressure-level atmospheric reanalysis for direct PyAPS / MintPy / ISCE2 tropospheric integration.",
      filters: ["SAR Overpass Alignment", "37 Pressure Levels", "Spatial Buffer"],
    },
    ja: {
      name: "ERA5 大気再解析データ",
      subtitle: "毎時 37 気圧面 · PyAPS / MintPy 標準入力 · 直接取得",
      band: "大気補正 (0.25° 3次元温度・湿度・気圧)",
      timeRange: "1940年〜現在 (毎時)",
      accountText: "ARCO-ERA5 クラウド無料 / CDS",
      description: "ECMWF 第5世代再解析データ。待機キューなしで SAR 通過時刻に合わせた PyAPS / MintPy 用大気補正入力を取得できます。",
      filters: ["SAR 通過時刻整合", "37 気圧面", "空間バッファ"],
    },
  },
  "ionospheric.igs.gim.v1": {
    en: {
      name: "IGS / CODE Global Ionosphere Maps (IONEX GIM)",
      subtitle: "Global TEC Grids · Essential for L-band InSAR · Earthdata",
      band: "Ionospheric Correction (IONEX TEC)",
      provider: "NASA CDDIS / CODE (University of Bern)",
      timeRange: "1998 to Present",
      accountText: "Earthdata",
      description: "IGS and CODE global Total Electron Content IONEX files matched automatically by acquisition date to mitigate ALOS-2/NISAR L-band ionospheric ramps.",
      filters: ["Analysis Center (CODE / IGS / JPL)", "Acquisition DOY"],
    },
    ja: {
      name: "IGS / CODE 全球電離層マップ (IONEX GIM)",
      subtitle: "全球 TEC グリッド · Lバンド InSAR 必須 · Earthdata",
      band: "電離層補正 (IONEX TEC)",
      provider: "NASA CDDIS / CODE（ベルン大学）",
      timeRange: "1998年〜現在",
      accountText: "Earthdata",
      description: "IGS / CODE の全電子数 (TEC) 標準ファイル。ALOS-2 や NISAR の長波長電離層縞の補正に利用します。",
      filters: ["解析センター (CODE / IGS / JPL)", "撮像日 (DOY)"],
    },
  },
  "auxiliary.xyz-tiles.v1": {
    en: {
      name: "Global Hi-Res Satellite & Basemap Tiles (XYZ)",
      subtitle: "15 Built-in Sources (incl. Wayback Archive) + Custom URL · GeoTIFF/COG/MBTiles · Free",
      band: "Multi-source Basemaps, Wayback & Satellite Tiles (Z1–Z22)",
      era: "2014–Present archive + live basemaps",
      provider: "Esri / Esri Wayback / Google / Amap / Tianditu / OpenTopoMap / OSM / Carto / Custom",
      satellites: "Esri · Esri Wayback · Google · Amap · Tianditu · OpenTopoMap · OSM · Carto · Custom XYZ/TMS",
      timeRange: "2014–Present (Wayback) / Live Tiles",
      accountText: "No account required",
      description:
        "Esri Wayback historical imagery (2014–present), 14 live satellite/topo basemaps (with GCJ-02 correction), and custom XYZ/TMS templates with tiled mosaic export.",
      filters: ["15 Sources + Custom URL", "Zoom Z1–Z22", "GeoTIFF / COG / MBTiles / PNG Export", "Compression & Overviews"],
    },
    ja: {
      name: "全球高精細衛星・背景地図タイル (XYZ)",
      subtitle: "内蔵 15 ソース (Wayback 履歴含む) + カスタム URL · 結合エクスポート · 無料",
      band: "背景地図・過去衛星アーカイブ (Z1–Z22)",
      era: "2014–現在アーカイブ + リアルタイム地図",
      provider: "Esri / Esri Wayback / Google / 高徳 / 天地図 / OpenTopoMap / OSM / Carto / カスタム",
      satellites: "Esri · Esri Wayback · Google · 高徳 · 天地図 · OpenTopoMap · OSM · Carto · カスタム XYZ/TMS",
      timeRange: "2014年〜現在 (Wayback) / オンライン最新",
      accountText: "アカウント不要",
      description:
        "Esri Wayback 過去画像アーカイブ、14 種の最新背景地図・衛星画像、カスタム XYZ テンプレートを範囲指定でシームレス GeoTIFF/MBTiles 出力できます。",
      filters: ["15 種内蔵 + カスタム URL", "ズーム Z1–Z22", "GeoTIFF / COG / MBTiles 出力", "圧縮・ピラミッド"],
    },
  },
  "science.esa.worldcover.v1": {
    en: {
      name: "ESA WorldCover Global 10m Land Cover",
      subtitle: "10m Global 11 Classes · 2020–2021 · MPC Free",
      band: "10m Land Cover (Sentinel-1/2 fusion)",
      satellites: "Sentinel-1 · Sentinel-2 fusion",
      timeRange: "Static v100 (2020) · v200 (2021)",
      accountText: "MPC Free",
      description: "ESA 10m global land-cover map (11 classes) supporting online AOI clipping for InSAR masking and coherence classification.",
      filters: ["Spatial Extent", "Version (v200 / v100)", "Auto AOI Clip"],
    },
    ja: {
      name: "ESA WorldCover 全球 10m 土地被覆分類",
      subtitle: "10m 全球 11 クラス分類 · 2020–2021年 · MPC 無料",
      band: "10m 土地被覆 (Sentinel-1/2 融合)",
      satellites: "Sentinel-1 · Sentinel-2 統合",
      timeRange: "v100 (2020年) · v200 (2021年) 静的版",
      accountText: "MPC アカウント不要",
      description: "Sentinel-1/2 融合による 10m 全球土地被覆図。水域マスクやコヒーレンス分類向けに AOI 切り抜きに対応します。",
      filters: ["空間範囲", "バージョン (v200 / v100)", "AOI 自動切り抜き"],
    },
  },
  "science.jrc.gsw.v1": {
    en: {
      name: "JRC Global Surface Water (GSW)",
      subtitle: "30m Occurrence & Seasonality · 1984–Present · MPC Free",
      band: "30m Global Water Dynamics",
      satellites: "Landsat 5/7/8/9 full archive",
      timeRange: "1984 to Present",
      accountText: "MPC Free",
      description: "EC JRC 30m global surface water dynamics (Occurrence, Seasonality, Extent, Change) ideal for InSAR phase-unwrapping water masks.",
      filters: ["Spatial Extent", "Water Layer (occurrence / extent / seasonality / change)"],
    },
    ja: {
      name: "JRC 全球表層水動態 (GSW)",
      subtitle: "30m 水域出現頻度・季節性 · 1984年〜現在 · MPC 無料",
      band: "30m 全球水域マスク・長期変化",
      satellites: "Landsat 5/7/8/9 全履歴アーカイブ",
      timeRange: "1984年〜現在",
      accountText: "MPC アカウント不要",
      description: "EC JRC による約 40 年間の Landsat 水域統計データ。InSAR 位相アンラップ時の高精度水域マスクに最適です。",
      filters: ["空間範囲", "水域レイヤー (occurrence / extent / seasonality / change)"],
    },
  },
  "science.alos.palsar-mosaic.v1": {
    en: {
      name: "ALOS PALSAR 25m Annual Global Mosaic",
      subtitle: "L-band HH+HV Orthorectified · 2007–Present · JAXA / MPC Free",
      band: "L-band SAR (25m Mosaic)",
      timeRange: "2007 to Present (Annual)",
      accountText: "MPC Free",
      description: "JAXA 25m global annual terrain-corrected L-band HH/HV mosaics with local incidence mask via account-free Planetary Computer.",
      filters: ["Spatial Extent", "Annual Window", "Polarization Layer (HH / HV / mask)"],
    },
    ja: {
      name: "ALOS PALSAR 全球 25m 年別モザイク",
      subtitle: "Lバンド HH+HV オルソ補正済み · 2007年〜現在 · JAXA / MPC 無料",
      band: "Lバンド SAR (25m モザイク)",
      timeRange: "2007年〜現在 (年別)",
      accountText: "MPC アカウント不要",
      description: "JAXA 提供の 25m L バンド年別地形・放射補正モザイク (HH / HV / マスク)。アカウント不要で取得できます。",
      filters: ["空間範囲", "年次範囲", "偏波レイヤー (HH / HV / mask)"],
    },
  },
  "science.modis.ndvi.v1": {
    en: {
      name: "MODIS Global Vegetation Index (MOD13Q1)",
      subtitle: "250m 16-day NDVI / EVI · 2000–Present · NASA / MPC Free",
      band: "Vegetation Index (250m)",
      timeRange: "2000 to Present",
      accountText: "MPC Free",
      description: "NASA 250m 16-day NDVI/EVI vegetation series useful for assessing seasonal decorrelation in InSAR.",
      filters: ["Spatial Extent", "Time Window", "Index Band (NDVI / EVI)"],
    },
    ja: {
      name: "MODIS 全球植生指標 (MOD13Q1)",
      subtitle: "250m 16日間合成 NDVI / EVI · 2000年〜現在 · NASA / MPC 無料",
      band: "植生指標 (250m グリッド)",
      timeRange: "2000年〜現在",
      accountText: "MPC アカウント不要",
      description: "NASA Terra/Aqua MODIS 250m 16 日間合成 NDVI/EVI。InSAR 時系列の植生による非コヒーレンス評価に有用です。",
      filters: ["空間範囲", "期間", "指標バンド (NDVI / EVI)"],
    },
  },
  "science.aria.s1-gunw.v1": {
    en: {
      name: "NASA ARIA Sentinel-1 Standard Interferograms (S1-GUNW)",
      subtitle: "C-band Geocoded Unwrapped Phase & Coherence · 90m · Earthdata",
      band: "C-band GUNW (NetCDF)",
      timeRange: "2014 to Present",
      accountText: "Earthdata",
      description: "NASA JPL / Caltech ARIA standard geocoded unwrapped interferograms ready for MintPy and ARIA-tools time-series inversion.",
      filters: ["Spatial Extent", "Time Window", "Orbit Direction"],
    },
    ja: {
      name: "NASA ARIA Sentinel-1 標準アンラップ干渉画像 (S1-GUNW)",
      subtitle: "Cバンド 幾何補正済み干渉位相・コヒーレンス · 90m · Earthdata",
      band: "Cバンド GUNW (NetCDF)",
      timeRange: "2014年〜現在",
      accountText: "Earthdata",
      description: "MintPy / ARIA-tools へ直接入力できる NASA JPL ARIA の標準アンラップ済み干渉画像プロダクトです。",
      filters: ["空間範囲", "期間", "軌道方向"],
    },
  },
  "atmospheric.sentinel-5p.l2.v1": {
    en: {
      name: "Sentinel-5P Atmospheric Trace Gases (TROPOMI)",
      subtitle: "Daily Global NO2 / CH4 / SO2 / CO / O3 / Aerosol · 2018–Present · MPC Free",
      band: "Atmospheric Chemistry Column Densities",
      timeRange: "2018 to Present",
      accountText: "MPC Free",
      description: "Copernicus Sentinel-5P TROPOMI L2 trace gas and aerosol products via account-free Planetary Computer.",
      filters: ["Spatial Extent", "Time Window", "Gas Product"],
    },
    ja: {
      name: "Sentinel-5P 大気汚染・温室効果ガス (TROPOMI)",
      subtitle: "全球日別 NO2 / CH4 / SO2 / CO / O3 / エアロゾル · 2018年〜現在 · MPC 無料",
      band: "大気微量成分・ガス柱密度",
      timeRange: "2018年〜現在",
      accountText: "MPC アカウント不要",
      description: "アカウント不要で取得可能な Sentinel-5P TROPOMI L2 プロダクト。火山 SO2 やメタン排出監視に適しています。",
      filters: ["空間範囲", "期間", "ガス成分 (NO2 / CH4 / SO2 / CO / O3 / AER_AI)"],
    },
  },
  "science.gee.catalog.v1": {
    en: {
      name: "Google Earth Engine (GEE) Public Catalog",
      subtitle: "25 Built-in Public Collections + Custom Asset ID · Online AOI Clip",
      band: "Multi-sensor Cloud Data (GEE)",
      timeRange: "1972 to Present",
      accountText: "Account-free / GEE Account",
      description: "GEE public catalog with curated collections, AOI-clipped GeoTIFF export, and custom Asset ID support.",
      filters: ["Curated Themes", "Time Window", "AOI Clip"],
    },
    ja: {
      name: "Google Earth Engine (GEE) 公開カタログ",
      subtitle: "内蔵 25 公開データセット + カスタム Asset ID · オンライン切り抜き",
      band: "マルチセンサークラウドデータ (GEE)",
      timeRange: "1972年〜現在",
      accountText: "アカウント不要 / GEE アカウント",
      description: "GEE 公開リモートセンシングカタログ。テーマ別検索、任意 Asset ID 指定、AOI 切り抜き出力に対応します。",
      filters: ["テーマ・公開カタログ", "期間", "AOI 切り抜き"],
    },
  },
  "science.gee.s2-sr.v1": {
    en: {
      name: "GEE: Sentinel-2 L2A Surface Reflectance",
      subtitle: "10m MSI · COPERNICUS/S2_SR_HARMONIZED",
      band: "VNIR / Red Edge / SWIR",
      timeRange: "2017 to Present",
      accountText: "Cloud Free / GEE Account",
      description: "Harmonized atmospherically corrected Sentinel-2 MSI L2A surface reflectance clipped online to your AOI.",
      filters: ["AOI Clip", "Time Window", "Cloud Cover"],
    },
    ja: {
      name: "GEE: Sentinel-2 L2A 地表面反射率",
      subtitle: "10m MSI · COPERNICUS/S2_SR_HARMONIZED",
      band: "可視 / 近赤外 / レッドエッジ / 短波長赤外",
      timeRange: "2017年〜現在",
      accountText: "クラウド直接 / GEE アカウント",
      description: "大気補正済みの Sentinel-2 MSI L2A データセットを AOI に合わせてクラウド切り抜きできます。",
      filters: ["空間範囲 (AOI 切り抜き)", "期間", "雲量"],
    },
  },
  "science.gee.s1-grd.v1": {
    en: {
      name: "GEE: Sentinel-1 C-band SAR GRD",
      subtitle: "10m GRD Backscatter · COPERNICUS/S1_GRD",
      band: "C-band (5.405 GHz)",
      timeRange: "2014 to Present",
      accountText: "Cloud Free / GEE Account",
      description: "Orbit-restituted and radiometrically calibrated Sentinel-1 C-band GRD backscatter in GEE.",
      filters: ["Polarization (VV/VH/HH/HV)", "Orbit Direction", "Instrument Mode"],
    },
    ja: {
      name: "GEE: Sentinel-1 Cバンド SAR GRD",
      subtitle: "10m グランドレンジ強度 · COPERNICUS/S1_GRD",
      band: "Cバンド (5.405 GHz)",
      timeRange: "2014年〜現在",
      accountText: "クラウド直接 / GEE アカウント",
      description: "軌道および放射校正済みの Sentinel-1 C バンド GRD 後方散乱係数データセットです。",
      filters: ["偏波 (VV/VH/HH/HV)", "軌道方向", "観測モード"],
    },
  },
  "science.gee.landsat9.v1": {
    en: {
      name: "GEE: Landsat 9 C2-L2 Surface Reflectance",
      subtitle: "30m Optical/Thermal · LANDSAT/LC09/C02/T1_L2",
      band: "VNIR / SWIR / Thermal IR",
      timeRange: "2021 to Present",
      accountText: "Cloud Free / GEE Account",
      description: "Landsat 9 Collection 2 Level-2 surface reflectance and land surface temperature.",
      filters: ["Spatial Extent", "Time Window", "Cloud Cover"],
    },
    ja: {
      name: "GEE: Landsat 9 C2-L2 地表面反射率",
      subtitle: "30m 光学・熱赤外 · LANDSAT/LC09/C02/T1_L2",
      band: "可視 / 近赤外 / 短波長赤外 / 熱赤外",
      timeRange: "2021年〜現在",
      accountText: "クラウド直接 / GEE アカウント",
      description: "大気補正済みの Landsat 9 C2-L2 地表面反射率・地表面温度データセットです。",
      filters: ["空間範囲", "期間", "雲量"],
    },
  },
  "science.gee.landsat8.v1": {
    en: {
      name: "GEE: Landsat 8 C2-L2 Surface Reflectance",
      subtitle: "30m Optical/Thermal · LANDSAT/LC08/C02/T1_L2",
      band: "VNIR / SWIR / Thermal IR",
      timeRange: "2013 to Present",
      accountText: "Cloud Free / GEE Account",
      description: "Landsat 8 Collection 2 Level-2 surface reflectance and thermal temperature with QA mask.",
      filters: ["Spatial Extent", "Time Window", "Cloud Cover"],
    },
    ja: {
      name: "GEE: Landsat 8 C2-L2 地表面反射率",
      subtitle: "30m 光学・熱赤外 · LANDSAT/LC08/C02/T1_L2",
      band: "可視 / 近赤外 / 短波長赤外 / 熱赤外",
      timeRange: "2013年〜現在",
      accountText: "クラウド直接 / GEE アカウント",
      description: "雲・影品質マスクを含む Landsat 8 C2-L2 地表面反射率・熱赤外データセットです。",
      filters: ["空間範囲", "期間", "雲量"],
    },
  },
  "science.gee.dynamic-world.v1": {
    en: {
      name: "GEE: Dynamic World 10m Near-Real-Time LULC",
      subtitle: "10m Class Probabilities · GOOGLE/DYNAMICWORLD/V1",
      band: "9-Class Probability Layers",
      satellites: "Sentinel-2 (AI-derived)",
      timeRange: "2015 to Present",
      accountText: "Cloud Free / GEE Account",
      description: "Near-real-time global 10m 9-class land-cover probabilities derived from Sentinel-2.",
      filters: ["Spatial Extent", "Time Window", "Class Confidence"],
    },
    ja: {
      name: "GEE: Dynamic World 10m 準リアルタイム土地被覆",
      subtitle: "10m 土地利用確率 · GOOGLE/DYNAMICWORLD/V1",
      band: "水域 / 植生 / 都市 / 裸地 / 農地 確率",
      satellites: "Sentinel-2（AI 推論プロダクト）",
      timeRange: "2015年〜現在",
      accountText: "クラウド直接 / GEE アカウント",
      description: "Sentinel-2 による全球 10m 9 クラス準リアルタイム土地被覆確率データセットです。",
      filters: ["空間範囲", "期間", "分類信頼度"],
    },
  },
  "science.gee.worldcover.v1": {
    en: {
      name: "GEE: ESA WorldCover 10m Land Cover",
      subtitle: "10m 11-Class Map · ESA/WorldCover/v200",
      band: "11-Class Discrete Land Cover",
      satellites: "Sentinel-1 + Sentinel-2 fusion",
      timeRange: "2020–2021",
      accountText: "Cloud Free / GEE Account",
      description: "ESA Sentinel-1/2 global 10m discrete land cover dataset.",
      filters: ["Version (v200 / v100)", "Spatial Extent"],
    },
    ja: {
      name: "GEE: ESA WorldCover 10m 土地被覆分類",
      subtitle: "10m 11クラス分類 · ESA/WorldCover/v200",
      band: "全球 11 クラス土地被覆ラスタ",
      satellites: "Sentinel-1 + Sentinel-2 統合",
      timeRange: "2020–2021年",
      accountText: "クラウド直接 / GEE アカウント",
      description: "ESA Sentinel-1/2 融合による 10m 離散土地被覆プロダクトです。",
      filters: ["バージョン (v200 / v100)", "空間範囲"],
    },
  },
  "science.gee.gsw.v1": {
    en: {
      name: "GEE: JRC Global Surface Water (GSW)",
      subtitle: "30m Water Occurrence · JRC/GSW1_4/GlobalSurfaceWater",
      band: "Occurrence / Seasonality / Change",
      satellites: "Landsat 5 / 7 / 8 time series",
      timeRange: "1984–2021",
      accountText: "Cloud Free / GEE Account",
      description: "JRC 30m multi-decadal Landsat global water occurrence and dynamics.",
      filters: ["Water Layer", "Spatial Extent"],
    },
    ja: {
      name: "GEE: JRC 全球表層水動態 (GSW)",
      subtitle: "30m 水域出現頻度 · JRC/GSW1_4/GlobalSurfaceWater",
      band: "出現頻度 / 季節性 / 経年変化",
      satellites: "Landsat 5 / 7 / 8 時系列",
      timeRange: "1984–2021年",
      accountText: "クラウド直接 / GEE アカウント",
      description: "JRC による 30m 全球表層水出現頻度と長期変化データセットです。",
      filters: ["水域サブレイヤー", "空間範囲"],
    },
  },
  "science.gee.dem-glo30.v1": {
    en: {
      name: "GEE: Copernicus DEM GLO-30 (30m)",
      subtitle: "30m Copernicus DEM · COPERNICUS/DEM/GLO30",
      band: "Digital Elevation Model (DEM)",
      timeRange: "2015 Baseline",
      accountText: "Cloud Free / GEE Account",
      description: "Global 30m TanDEM-X elevation model hosted in GEE for cloud terrain analysis and AOI clipping.",
      filters: ["Spatial Extent (AOI Clip)"],
    },
    ja: {
      name: "GEE: Copernicus DEM GLO-30（30m）",
      subtitle: "30m Copernicus 正標高 DEM · COPERNICUS/DEM/GLO30",
      band: "数値標高モデル (DEM)",
      timeRange: "2015年基準",
      accountText: "クラウド直接 / GEE アカウント",
      description: "ESA TanDEM-X 全球 30m DEM の GEE クラウド版です。",
      filters: ["空間範囲 (AOI 切り抜き)"],
    },
  },
  "science.gee.nasadem.v1": {
    en: {
      name: "GEE: NASA NASADEM 30m",
      subtitle: "30m Reprocessed SRTM · NASA/NASADEM_HGT/001",
      band: "Digital Elevation Model (DEM)",
      timeRange: "2000 Baseline (2020 release)",
      accountText: "Cloud Free / GEE Account",
      description: "NASA JPL void-filled 30m global DEM reprocessed from SRTM.",
      filters: ["Spatial Extent (AOI Clip)"],
    },
    ja: {
      name: "GEE: NASA NASADEM 30m",
      subtitle: "30m 再処理 SRTM 改良版 · NASA/NASADEM_HGT/001",
      band: "数値標高モデル (DEM)",
      timeRange: "2000年基準 (2020年公開)",
      accountText: "クラウド直接 / GEE アカウント",
      description: "NASA JPL が SRTM を再処理・欠損補間した 30m 全球 DEM です。",
      filters: ["空間範囲 (AOI 切り抜き)"],
    },
  },
  "science.gee.gem-15.v1": {
    en: {
      name: "GEE: Google GEM-15 15m Elevation (DTM + DSM)",
      subtitle: "15m Bare-earth DTM & Surface DSM · projects/climate-and-sustainability/assets/gem-15_v1",
      band: "15m DTM + DSM (EGM96)",
      satellites: "Google multi-source stereo optical fusion",
      timeRange: "2018–2025 (2026 release)",
      accountText: "GEE Account / Direct GEM-15 also available",
      description: "Google Research 15m global DEM v1 providing both DTM and DSM bands in a single GEE collection.",
      filters: ["Spatial Extent (AOI Clip)"],
      usageNotes: [
        "Excludes Mainland China, Russia, Mongolia, North Korea, Greenland, and Antarctica; direct account-free GEM-15 is also available under Elevation DEM.",
        "CC BY 4.0 license (attribute Google).",
      ],
    },
    ja: {
      name: "GEE: Google GEM-15 15m 標高 (DTM + DSM)",
      subtitle: "15m 裸地 DTM・表層 DSM · projects/climate-and-sustainability/assets/gem-15_v1",
      band: "15m 裸地 DTM + 表層 DSM (EGM96 正標高)",
      satellites: "Google 複数ソース立体光学統合",
      timeRange: "2018–2025年 (2026年公開)",
      accountText: "GEE 切り抜き / 直接 GEM-15 も利用可",
      description: "Google Research による 15m 標高モデルの GEE 版。同一コレクションで裸地 DTM と表層 DSM を提供します。",
      filters: ["空間範囲 (AOI 切り抜き)"],
      usageNotes: [
        "公式 v1 は中国本土・ロシア・モンゴル・北朝鮮・グリーンランド・南極を含みません。",
        "CC BY 4.0 ライセンス（要 Google クレジット表記）。",
      ],
    },
  },
  "science.gee.modis-ndvi.v1": {
    en: {
      name: "GEE: MODIS 250m Vegetation Index (MOD13Q1)",
      subtitle: "250m 16-day NDVI / EVI · MODIS/061/MOD13Q1",
      band: "NDVI / EVI / NIR",
      timeRange: "2000 to Present",
      accountText: "Cloud Free / GEE Account",
      description: "Terra MODIS global 250m 16-day NDVI and EVI composite collection.",
      filters: ["Vegetation Index (NDVI / EVI)", "Time Window", "Spatial Extent"],
    },
    ja: {
      name: "GEE: MODIS 250m 植生指標 (MOD13Q1)",
      subtitle: "250m 16日間合成 NDVI / EVI · MODIS/061/MOD13Q1",
      band: "NDVI / EVI / 赤外反射率",
      timeRange: "2000年〜現在",
      accountText: "クラウド直接 / GEE アカウント",
      description: "Terra MODIS 全球 250m 16 日間合成 NDVI・EVI データセットです。",
      filters: ["植生指標 (NDVI / EVI)", "期間", "空間範囲"],
    },
  },
  "science.gee.era5-land.v1": {
    en: {
      name: "GEE: ECMWF ERA5-Land Hourly Reanalysis",
      subtitle: "9km Hourly Land Meteorology · ECMWF/ERA5_LAND/HOURLY",
      band: "Temperature / Pressure / Dewpoint / Precip / Radiation",
      satellites: "ECMWF numerical weather model",
      timeRange: "1950 to Present",
      accountText: "Cloud Free / GEE Account",
      description: "ECMWF 0.1° hourly land-surface climate reanalysis in GEE.",
      filters: ["Meteorological Bands", "Time Window", "Spatial Extent"],
    },
    ja: {
      name: "GEE: ECMWF ERA5-Land 毎時気象再解析",
      subtitle: "9km 毎時陸面気象 · ECMWF/ERA5_LAND/HOURLY",
      band: "気温 / 気圧 / 露点 / 降水量 / 放射",
      satellites: "ECMWF 数値予報モデル",
      timeRange: "1950年〜現在",
      accountText: "クラウド直接 / GEE アカウント",
      description: "ECMWF の 0.1 度毎時陸面再解析データセット。気温・気圧・降水などの連携解析に対応します。",
      filters: ["気象バンド", "期間", "空間範囲"],
    },
  },
  "science.gee.s5p-no2.v1": {
    en: {
      name: "GEE: Sentinel-5P Tropospheric NO2",
      subtitle: "Daily Air Quality Grid · COPERNICUS/S5P/OFFL/L3_NO2",
      band: "Tropospheric NO2 Column Density",
      timeRange: "2018 to Present",
      accountText: "Cloud Free / GEE Account",
      description: "Sentinel-5P TROPOMI daily tropospheric nitrogen dioxide grid for emissions and air-quality analysis.",
      filters: ["Spatial Extent", "Time Window", "Quality Flag"],
    },
    ja: {
      name: "GEE: Sentinel-5P 対流圏 NO2 柱濃度",
      subtitle: "日別大気質グリッド · COPERNICUS/S5P/OFFL/L3_NO2",
      band: "対流圏 NO2 柱濃度",
      timeRange: "2018年〜現在",
      accountText: "クラウド直接 / GEE アカウント",
      description: "大気汚染・排出源監視用の Sentinel-5P TROPOMI 日別 NO2 データセットです。",
      filters: ["空間範囲", "期間", "品質フラグ"],
    },
  },
  "science.whu.clcd.v1": {
    en: {
      name: "WHU Annual China 30m Land Cover (CLCD v1.0.4)",
      subtitle: "Annual 30m 9-Class China Land Cover · 1985–2023 · Zenodo CC-BY-4.0",
      band: "30m Annual Land Cover (9 classes)",
      provider: "Wuhan University (Yang & Huang) / Zenodo",
      timeRange: "1985–2023 Annual",
      accountText: "No account required (CC-BY-4.0)",
      description:
        "Wuhan University 30m annual China Land Cover Dataset (1985–2023, 9 Level-1 classes, ~79.3% accuracy).",
      filters: ["Target Year", "9-Class Land Cover", "Spatial Extent"],
    },
    ja: {
      name: "武漢大学 中国年別 30m 土地被覆 (CLCD v1.0.4)",
      subtitle: "中国年次 30m 土地利用分類 (9 クラス) · 1985–2023年 · CC-BY-4.0",
      band: "30m 年次土地被覆 (農地・森林・水域・不透水面等)",
      provider: "武漢大学 Yang & Huang / Zenodo",
      timeRange: "1985–2023年 年次連続",
      accountText: "アカウント不要 (CC-BY-4.0)",
      description: "武漢大学チーム公開の全 Landsat 系列に基づく 30m 中国年次土地被覆データセットです。",
      filters: ["対象年", "9 クラス分類", "空間範囲"],
    },
  },
  "science.tum.eot20.v1": {
    en: {
      name: "TUM EOT20 Global Empirical Ocean Tide Model",
      subtitle: "DGFI-TUM 0.125° Ocean & Shallow-Water Tides · SEANOE CC-BY-4.0",
      band: "17 Constituents Amplitudes & Phases (NetCDF)",
      era: "Multi-mission altimetry harmonic",
      timeRange: "Global Harmonic Static Model",
      accountText: "No account required (CC-BY-4.0)",
      description: "DGFI-TUM empirical ocean tide model (EOT20) with enhanced coastal/shelf accuracy for pyTMD InSAR ocean-tide loading correction.",
      filters: ["Tidal Constituents (M2/S2/K1/O1)", "Spatial Extent"],
    },
    ja: {
      name: "TUM EOT20 全球経験海洋潮汐モデル",
      subtitle: "DGFI-TUM 0.125° 海洋潮汐・浅海分潮 · SEANOE CC-BY-4.0",
      band: "17 主要分潮 振幅・位相 (NetCDF)",
      era: "複数ミッション衛星高度計調和",
      timeRange: "全球調和定数モデル",
      accountText: "アカウント不要 (CC-BY-4.0)",
      description: "ミュンヘン工科大による沿岸精度を高めた EOT20 モデル。pyTMD と連携した沿岸 InSAR 潮汐荷重補正に最適です。",
      filters: ["分潮成分 (M2/S2/K1/O1)", "空間範囲"],
    },
  },
  "science.gebco.bathymetry-2026.v1": {
    en: {
      name: "GEBCO Global 15-Arcsecond Bathymetry Grid",
      subtitle: "Global Seafloor Bathymetry & Land Topography (15\") · Public",
      band: "Bathymetry & Elevation (EPSG:4326)",
      era: "Latest global compilation",
      satellites: "Multibeam sonar · satellite altimetry gravity",
      timeRange: "Global Static Reference Grid",
      accountText: "No account required",
      description: "GEBCO 15-arcsecond (~450m) seamless global bathymetry/topography grid and Type Identifier (TID) for coastal and marine analysis.",
      filters: ["Elevation / Bathymetry Grid", "Spatial Extent"],
    },
    ja: {
      name: "GEBCO 全球 15 秒角海底地形グリッド",
      subtitle: "全球水深・陸上標高シームレス格子 (15 arc-second) · 公開データ",
      band: "海底水深・陸上標高 (EPSG:4326)",
      era: "最新の全球編纂アーカイブ",
      satellites: "マルチビームソナー・衛星重力高度計逆解析",
      timeRange: "全球静的参照グリッド",
      accountText: "アカウント不要",
      description: "GEBCO 公開の約 450m 解像度シームレス海陸地形グリッド。沿岸帯・潮汐境界・海陸マスク作成に利用します。",
      filters: ["標高・水深グリッド", "空間範囲"],
    },
  },
  "science.noaa.etopo-2022.v1": {
    en: {
      name: "NOAA ETOPO 2022 Global Relief Model",
      subtitle: "15-Arcsecond Bed & Ice Surface Grids · NOAA NCEI",
      band: "Bedrock / Ice Surface / Bathymetry",
      era: "2022 baseline compilation",
      satellites: "ICESat-2 · Copernicus DEM · bathymetric compilation",
      timeRange: "Global Static Model (2022)",
      accountText: "No account required",
      description: "NOAA NCEI 15-arcsecond global relief model combining Copernicus DEM, FABDEM, ICESat-2, and multibeam bathymetry.",
      filters: ["Layer Type (Bed / Surface)", "Spatial Extent"],
    },
    ja: {
      name: "NOAA ETOPO 2022 全球地形水深モデル",
      subtitle: "15 秒角 基盤岩 (Bed)・氷床表面 (Surface) 格子 · NOAA NCEI",
      band: "基盤標高 / 氷床表面 / 海底水深",
      era: "2022 基準編纂",
      satellites: "ICESat-2 · Copernicus DEM · 水深測量編纂",
      timeRange: "全球静的モデル (2022年版)",
      accountText: "アカウント不要",
      description: "Copernicus DEM・FABDEM・ICESat-2 を統合した NOAA NCEI の 15 秒角全球地形・水深モデルです。",
      filters: ["レイヤー種別 (Bed / Surface)", "空間範囲"],
    },
  },
  "science.nasa.black-marble.v1": {
    en: {
      name: "NASA Black Marble Nighttime Lights (VIIRS)",
      subtitle: "500m Nighttime Radiance · 2012–Present · NASA LAADS DAAC / Earthdata",
      band: "VIIRS DNB Nighttime Radiance (500m · HDF5)",
      timeRange: "2012-01 to Present",
      accountText: "Earthdata",
      description:
        "NASA VIIRS Black Marble moonlight/BRDF/atmospheric-corrected nighttime lights (VNP46A3 monthly, VNP46A4 annual, VNP46A2 daily) for outage & urban activity tracking.",
      filters: ["Product Cadence (VNP46A3 / VNP46A4 / VNP46A2)", "Spatial Extent", "Time Window"],
    },
    ja: {
      name: "NASA Black Marble 全球夜間光 (VIIRS)",
      subtitle: "500m 夜間光放射輝度 · 2012年〜現在 · NASA LAADS DAAC / Earthdata",
      band: "VIIRS DNB 夜間光 (500m · HDF5)",
      timeRange: "2012年1月〜現在",
      accountText: "Earthdata",
      description: "月光・大気補正済みの NASA VIIRS Black Marble 夜間光データ。停電影響や都市活動の長期評価に適しています。",
      filters: ["製品周期 (月次 VNP46A3 / 年次 VNP46A4 / 日次 VNP46A2)", "空間範囲", "期間"],
    },
  },
  "science.nasa.emit.l2a.v1": {
    en: {
      name: "NASA EMIT Hyperspectral Reflectance & Minerals",
      subtitle: "60m 285-Band Imaging Spectroscopy · 2022–Present · Earthdata",
      band: "Hyperspectral VSWIR (380–2500 nm · 285 bands · 60m)",
      timeRange: "2022-08 to Present",
      accountText: "Earthdata",
      description: "ISS-mounted NASA JPL EMIT 285-band hyperspectral L2A reflectance (EMITL2ARFL) and L2B mineral maps (EMITL2BMIN).",
      filters: ["Level (L2A Reflectance / L2B Minerals)", "Spatial Extent", "Time Window"],
    },
    ja: {
      name: "NASA EMIT ハイパースペクトル反射率・鉱物",
      subtitle: "60m 285バンド ハイパースペクトル · 2022年〜現在 · NASA LP DAAC / Earthdata",
      band: "ハイパースペクトル VSWIR (380–2500 nm · 285バンド · 60m)",
      timeRange: "2022年8月〜現在",
      accountText: "Earthdata",
      description: "ISS 搭載 NASA JPL EMIT の 285 バンド連続分光データ。L2A 地表面反射率および L2B 鉱物組成プロダクトを取得できます。",
      filters: ["レベル (L2A 反射率 / L2B 鉱物)", "空間範囲", "期間"],
    },
  },
  "science.dlr.enmap.l2a.v1": {
    en: {
      name: "DLR EnMAP Hyperspectral L2A Surface Reflectance",
      subtitle: "30m 224-Band Hyperspectral · 2022–Present · DLR EOC (Search Only)",
      band: "Hyperspectral VNIR+SWIR (420–2450 nm · 224 bands · 30m)",
      timeRange: "2022-04 to Present",
      accountText: "Public Search Only (Download via DLR EOWEB SSO)",
      description:
        "German Environmental Mapping and Analysis Program (EnMAP) L2A open STAC catalog (30m, 224 bands). Raw archives require interactive DLR EOC web portal login.",
      filters: ["Spatial Extent", "Time Window"],
      usageNotes: [
        "DLR EOC Geoservice STAC allows account-free catalog search and footprint inspection.",
        "Raw .TIF downloads are gated behind DLR CAS web SSO; copy scene IDs to https://eoweb.dlr.de after search.",
      ],
    },
    ja: {
      name: "DLR EnMAP ハイパースペクトル L2A 地表面反射率",
      subtitle: "30m 224バンド · 2022年〜現在 · ドイツ航空宇宙センター DLR (公開検索のみ)",
      band: "ハイパースペクトル VNIR+SWIR (420–2450 nm · 224バンド · 30m)",
      timeRange: "2022年4月〜現在",
      accountText: "検索無料（取得は DLR EOWEB ポータル SSO）",
      description:
        "EnMAP L2A 公開 STAC カタログ。アカウント不要でフットプリントと雲量を検索可能。元画像のダウンロードは DLR EOC Web SSO ポータルから行います。",
      filters: ["空間範囲", "期間"],
      usageNotes: [
        "DLR EOC Geoservice STAC によりアカウント不要でフットプリントと雲量を検索できます。",
        "原本 .TIF の取得は DLR CAS SSO 認証が必要なため、検索したシーン ID を https://eoweb.dlr.de で取得してください。",
      ],
    },
  },
  "science.nasa.ecostress.l2t.v1": {
    en: {
      name: "NASA ECOSTRESS Land Surface Temperature & ET",
      subtitle: "70m Thermal LST / ET · 2018–Present · NASA LP DAAC / Earthdata",
      band: "Thermal LST / Emissivity / Daily ET (70m COG)",
      timeRange: "2018-07 to Present",
      accountText: "Earthdata",
      description: "ISS NASA JPL ECOSTRESS 70m thermal COG datasets for Land Surface Temperature, emissivity, and daily evapotranspiration.",
      filters: ["Thermal Layer (LST / EmisWB / ETdaily)", "Spatial Extent", "Time Window"],
    },
    ja: {
      name: "NASA ECOSTRESS 地表面温度・蒸発散",
      subtitle: "70m 熱赤外 LST / ET · 2018年〜現在 · NASA LP DAAC / Earthdata",
      band: "熱赤外 LST / 放射率 / 日蒸発散量 (70m COG)",
      timeRange: "2018年7月〜現在",
      accountText: "Earthdata",
      description: "ISS 搭載 ECOSTRESS による高解像度 70m 熱赤外 COG。火山地熱・断層熱異常・干ばつ監視に適しています。",
      filters: ["熱赤外レイヤー (LST / EmisWB / ETdaily)", "空間範囲", "期間"],
    },
  },
  "science.aws.sentinel-1-grd.v1": {
    en: {
      name: "Sentinel-1 GRD (AWS Open Data)",
      subtitle: "C-band Multi-looked COG · Element 84 STAC · Account-free AWS S3",
      band: "C-band SAR (5.405 GHz · Cloud-Native COG)",
      timeRange: "2014-10 to Present",
      accountText: "No account required",
      description: "Sentinel-1 GRD Cloud-Optimized GeoTIFFs on AWS Open Data (Earth Search STAC), supporting account-free search and /vsicurl/ AOI clipping.",
      filters: ["Spatial Extent", "Time Window", "Polarization (VV / VH / HH / HV)"],
    },
    ja: {
      name: "Sentinel-1 GRD (AWS Open Data)",
      subtitle: "Cバンド多視処理済み COG · Element 84 STAC · アカウント不要 AWS S3",
      band: "Cバンド SAR (5.405 GHz · クラウドネイティブ COG)",
      timeRange: "2014年10月〜現在",
      accountText: "アカウント不要",
      description: "AWS Open Data 上の Sentinel-1 GRD COG。アカウント不要で検索・ダウンロードでき、AOI 切り抜きにも対応します。",
      filters: ["空間範囲", "期間", "偏波 (VV / VH / HH / HV)"],
    },
  },
  "science.noaa.aws-weather.v1": {
    en: {
      name: "NOAA GFS / GOES Weather (AWS Open Data)",
      subtitle: "GFS 0.25° .idx Subset & GOES ABI Geostationary · Account-free AWS S3",
      band: "Meteorology / Geostationary IR & Visible (GRIB2 / NetCDF4)",
      era: "Real-time – rolling 48h archive",
      timeRange: "Rolling Near-Real-Time & Archive",
      accountText: "No account required",
      description: "NOAA open weather suite on AWS S3: GFS 0.25° byte-range variable subsetting via .idx and GOES-19/18 ABI NetCDF.",
      filters: ["Mode (GFS / GOES)", "Variable Keyword", "Spatial Extent"],
    },
    ja: {
      name: "NOAA GFS / GOES 気象衛星 (AWS Open Data)",
      subtitle: "GFS 0.25° .idx サブセット＆静止気象衛星 GOES · アカウント不要 AWS S3",
      band: "数値予報変数 / 静止気象赤外・可視 (GRIB2 / NetCDF4)",
      era: "リアルタイム–直近 48 時間ローリング",
      timeRange: "準リアルタイム・アーカイブ",
      accountText: "アカウント不要",
      description: "AWS Open Data 上の NOAA 気象データ。GFS 0.25° の変数別 Range 取得や GOES-19/18 ABI データに直接アクセスできます。",
      filters: ["モード (GFS / GOES)", "気象変数キーワード", "空間範囲"],
    },
  },
  "science.overture.maps.v1": {
    en: {
      name: "Overture Maps Global Open Vectors (AWS Open Data)",
      subtitle: "Buildings / Roads / Places / Divisions GeoParquet · Account-free AWS S3",
      band: "Open Vector Geometry (GeoParquet / DuckDB GeoJSON)",
      era: "2024–Latest monthly release",
      timeRange: "Latest Monthly Release",
      accountText: "No account required",
      description: "Overture Maps Foundation global vector GeoParquet partitions matched by AOI, downloadable or exported cleanly via DuckDB Spatial.",
      filters: ["Theme (buildings / places / transportation / divisions)", "Spatial Extent"],
    },
    ja: {
      name: "Overture Maps 全球オープンベクター (AWS Open Data)",
      subtitle: "建物・道路網・POI・行政界 GeoParquet · アカウント不要 AWS S3",
      band: "オープンベクター幾何属性 (GeoParquet / GeoJSON)",
      era: "2024–最新月次リリース",
      timeRange: "最新月次リリース",
      accountText: "アカウント不要",
      description: "Overture Maps の全球標準ベクターデータ。AOI に該当する GeoParquet パーティションの取得と GeoJSON 変換に対応します。",
      filters: ["テーマ (buildings / places / transportation / divisions)", "空間範囲"],
    },
  },
};

function localizeCatalogEraFallback(era: string | undefined, locale: Locale): string | undefined {
  if (!era || locale === "zh-CN") return era;
  if (era.includes("–至今")) {
    return era.replace(/–至今/g, locale === "en" ? "–Present" : "–現在");
  }
  return era;
}

export function applyCatalogItemLocale(
  meta: CatalogItemMetadata,
  resourceId: string,
  locale: Locale,
): CatalogItemMetadata {
  if (locale === "zh-CN") return meta;
  const entry = CATALOG_ITEM_TRANSLATIONS[resourceId]?.[locale];
  if (!entry) return meta;
  return {
    ...meta,
    name: entry.name,
    subtitle: entry.subtitle,
    band: entry.band ?? meta.band,
    era: entry.era ?? localizeCatalogEraFallback(meta.era, locale),
    provider: entry.provider ?? meta.provider,
    satellites: entry.satellites ?? meta.satellites,
    timeRange: entry.timeRange ?? meta.timeRange,
    accountText: entry.accountText,
    description: entry.description,
    filters: entry.filters,
    companionDownloads: entry.companionDownloads ?? meta.companionDownloads,
    usageNotes: entry.usageNotes ?? meta.usageNotes,
  };
}

export function localizeCatalogDynamicAccountText(
  locale: Locale,
  kind: "gee_ok" | "gee_search" | "free" | "failed" | "missing",
  baseLabel = "",
): string {
  if (locale === "en") {
    switch (kind) {
      case "gee_ok":
        return "GEE Cloud Project (Search & computePixels download ready)";
      case "gee_search":
        return "Public catalog search · Configure GEE project for cloud exports";
      case "free":
        return "No account required";
      case "failed":
        return `${baseLabel} (Verification failed)`;
      case "missing":
        return `${baseLabel} (Pending credentials)`;
    }
  }
  if (locale === "ja") {
    switch (kind) {
      case "gee_ok":
        return "GEE プロジェクト（検索・computePixels ダウンロード対応）";
      case "gee_search":
        return "公開カタログ検索のみ · 切り抜き取得は GEE プロジェクト設定が必要";
      case "free":
        return "アカウント不要";
      case "failed":
        return `${baseLabel}（認証未確認）`;
      case "missing":
        return `${baseLabel}（認証情報未設定）`;
    }
  }
  switch (kind) {
    case "gee_ok":
      return "GEE 云项目（支持检索与 computePixels 下载）";
    case "gee_search":
      return "仅公共目录检索 · 官方裁剪下载需配置 GEE 项目凭据";
    case "free":
      return "免账号";
    case "failed":
      return `${baseLabel}（验证未通过）`;
    case "missing":
      return `${baseLabel}（待配置凭据）`;
  }
}

const GEE_THEME_LABELS: Record<Exclude<Locale, "zh-CN">, Record<GeeCatalogTheme, string>> = {
  en: {
    all: "All",
    optical: "Multispectral Optical",
    sar: "SAR Radar",
    dem: "Elevation / Terrain",
    landcover: "Land Cover",
    water: "Surface Water",
    climate: "Climate & Precip",
    ecology: "Vegetation & Ecology",
    atmosphere: "Atmosphere & Nightlights",
  },
  ja: {
    all: "すべて",
    optical: "光学マルチスペクトル",
    sar: "SAR レーダー",
    dem: "標高・地形",
    landcover: "土地被覆",
    water: "表層水",
    climate: "気象・降水",
    ecology: "植生・生態",
    atmosphere: "大気・夜間光",
  },
};

export function localizeGeeThemeLabel(theme: GeeCatalogTheme, fallbackZh: string, locale: Locale): string {
  if (locale === "zh-CN") return fallbackZh;
  return GEE_THEME_LABELS[locale][theme] ?? fallbackZh;
}

const GEE_DATASET_TITLES: Record<
  string,
  { en: { title: string; subtitle: string; coverageNote?: string }; ja: { title: string; subtitle: string; coverageNote?: string } }
> = {
  "COPERNICUS/S2_SR_HARMONIZED": {
    en: { title: "Sentinel-2 L2A Surface Reflectance", subtitle: "10m MSI BOA reflectance for cloud-free median composites & spectral indices" },
    ja: { title: "Sentinel-2 L2A 地表面反射率（クラウド合成）", subtitle: "10m MSI 地表面反射率。NDVI/MNDWI や雲除去中央値モザイクに最適" },
  },
  "LANDSAT/LC09/C02/T1_L2": {
    en: { title: "Landsat 9 C2-L2 Reflectance & LST", subtitle: "OLI-2 / TIRS-2 30m optical and thermal time series" },
    ja: { title: "Landsat 9 C2-L2 反射率・地表面温度", subtitle: "OLI-2 / TIRS-2 30m 光学・熱赤外長期モザイク" },
  },
  "LANDSAT/LC08/C02/T1_L2": {
    en: { title: "Landsat 8 C2-L2 Reflectance & LST", subtitle: "30m continuous optical & thermal archive since 2013" },
    ja: { title: "Landsat 8 C2-L2 反射率・地表面温度", subtitle: "2013年以降の 30m 連続光学・熱赤外アーカイブ" },
  },
  "LANDSAT/LT05/C02/T1_L2": {
    en: { title: "Landsat 5 TM C2-L2 Historical Archive", subtitle: "Classic 28-year 30m optical archive (1984–2012)" },
    ja: { title: "Landsat 5 TM C2-L2 過去長期アーカイブ", subtitle: "1984〜2012年の 28 年間 30m 光学履歴データ" },
  },
  "COPERNICUS/S1_GRD": {
    en: { title: "Sentinel-1 C-band SAR GRD", subtitle: "10m all-weather calibrated backscatter intensity (Sigma0)" },
    ja: { title: "Sentinel-1 Cバンド SAR GRD", subtitle: "10m 全天候型校正済み後方散乱強度 (Sigma0)" },
  },
  "JAXA/ALOS/PALSAR/YEARLY/SAR_EPOCH": {
    en: { title: "ALOS PALSAR L-band Annual Mosaic", subtitle: "25m L-band canopy-penetrating HH/HV annual mosaic" },
    ja: { title: "ALOS PALSAR Lバンド 年別モザイク", subtitle: "25m 森林透過 HH/HV レーダー年次合成データ" },
  },
  "projects/climate-and-sustainability/assets/gem-15_v1": {
    en: {
      title: "Google GEM-15 15m Elevation (DTM + DSM)",
      subtitle: "15m EGM96 stereo-photogrammetry DEM · Excludes Mainland China & poles",
      coverageNote: "Official v1 excludes Mainland China, Russia, Mongolia, North Korea, and poles.",
    },
    ja: {
      title: "Google GEM-15 15m 標高 (DTM + DSM)",
      subtitle: "15m EGM96 正標高 DEM · 中国本土および両極は除外",
      coverageNote: "公式 v1 は中国本土・ロシア・モンゴル・北朝鮮・極域を除きます。",
    },
  },
  "projects/sat-io/open-datasets/FABDEM": {
    en: { title: "FABDEM 30m Bare-earth Elevation", subtitle: "Forest and buildings removed from Copernicus GLO-30" },
    ja: { title: "FABDEM 30m 裸地標高 (建物・樹冠除去)", subtitle: "Copernicus GLO-30 から森林・建築物オフセットを除去した 30m 裸地 DEM" },
  },
  "COPERNICUS/DEM/GLO30": {
    en: { title: "Copernicus DEM GLO-30 Global 30m", subtitle: "TanDEM-X 30m global orthometric elevation reference" },
    ja: { title: "Copernicus DEM GLO-30 全球標高", subtitle: "TanDEM-X 30m 高精度正標高リファレンス" },
  },
  "NASA/NASADEM_HGT/001": {
    en: { title: "NASA NASADEM Global 30m", subtitle: "Void-filled SRTM elevation fused with ICESat & ASTER" },
    ja: { title: "NASA NASADEM 全球 30m 標高", subtitle: "ICESat と ASTER で欠損補間された再処理 SRTM DEM" },
  },
  "USGS/SRTMGL1_003": {
    en: { title: "SRTM Plus Global 30m (V3)", subtitle: "Classic 1-arcsecond elevation covering ±60° latitude" },
    ja: { title: "SRTM Plus 全球 30m 数値標高 (V3)", subtitle: "±60° 緯度帯の標準 1 秒角地形データ" },
  },
  "GOOGLE/DYNAMICWORLD/V1": {
    en: { title: "Dynamic World 10m Near-Real-Time LULC", subtitle: "Per-scene deep-learning 9-class probabilities" },
    ja: { title: "Dynamic World 10m 準リアルタイム土地利用", subtitle: "9 クラス土地被覆の深層学習シーン別確率データ" },
  },
  "ESA/WorldCover/v200": {
    en: { title: "ESA WorldCover v200 10m Land Cover", subtitle: "2021 ESA 11-class global 10m reference classification" },
    ja: { title: "ESA WorldCover v200 10m 土地被覆", subtitle: "ESA 11 クラス 2021年全球 10m 基準分類" },
  },
  "projects/sat-io/open-datasets/GHSL/GHSL_BUILT_H_AGBH_2018_100M": {
    en: { title: "GHSL Global Building Height 100m", subtitle: "EC JRC global average/total built height morphology grid" },
    ja: { title: "GHSL 全球都市建築高さ 100m", subtitle: "EU JRC による全球都市建物の平均・総高グリッド" },
  },
  "projects/sat-io/open-datasets/CLCD": {
    en: { title: "WHU Annual China 30m Land Cover (CLCD · Community)", subtitle: "1985–2023 annual 9-class 30m China land cover by Yang & Huang" },
    ja: { title: "武漢大学 中国年次 30m 土地被覆 CLCD (コミュニティ)", subtitle: "1985–2023年の中国年別 9 クラス 30m 土地被覆基準データ" },
  },
  "JRC/GSW1_4/GlobalSurfaceWater": {
    en: { title: "JRC Global Surface Water (Summary)", subtitle: "30m water occurrence, seasonality & long-term change" },
    ja: { title: "JRC 全球表層水動態 (統合)", subtitle: "30m 水域出現頻度・季節性・長期変化サマリー" },
  },
  "JRC/GSW1_4/MonthlyHistory": {
    en: { title: "JRC Monthly Surface Water History (1984–2021)", subtitle: "Monthly permanent/seasonal water detections at 30m" },
    ja: { title: "JRC 全球表層水月別履歴 (1984–2021)", subtitle: "洪水推移の月別集計に適した 30m 連続水域記録" },
  },
  "ECMWF/ERA5_LAND/HOURLY": {
    en: { title: "ECMWF ERA5-Land Hourly Reanalysis", subtitle: "~9km temperature, pressure, precipitation & soil moisture" },
    ja: { title: "ECMWF ERA5-Land 陸面気象再解析", subtitle: "約 9km 気温・地表気圧・総降水量・土壌水分グリッド" },
  },
  "UCSB-CHG/CHIRPS/DAILY": {
    en: { title: "CHIRPS v2 Daily High-Res Precipitation", subtitle: "5.5km satellite infrared + station blended daily rainfall" },
    ja: { title: "CHIRPS v2 日別高解像度降水量", subtitle: "5.5km 衛星赤外・地上観測融合の日降水量長期系列" },
  },
  "NASA/GPM_L3/IMERG_V07": {
    en: { title: "NASA GPM IMERG V07 Global Precipitation", subtitle: "0.1° half-hourly precipitation for storm & landslide tracking" },
    ja: { title: "NASA GPM IMERG V07 全球衛星降水", subtitle: "0.1° 30分間隔降水監視（豪雨・土砂災害解析向け）" },
  },
  "MODIS/061/MOD13Q1": {
    en: { title: "MODIS Terra 250m 16-Day Vegetation Index", subtitle: "NDVI / EVI for phenology and degradation studies" },
    ja: { title: "MODIS Terra 250m 16日植生指標", subtitle: "NDVI / EVI による生態劣化・フェノロジー監視" },
  },
  "MODIS/061/MOD11A1": {
    en: { title: "MODIS Terra Daily 1km Land Surface Temp (LST)", subtitle: "Day/night thermal anomalies and urban heat islands" },
    ja: { title: "MODIS Terra 日別 1km 地表面温度 (LST)", subtitle: "昼夜の地表熱異常およびヒートアイランド監視" },
  },
  "UMD/hansen/global_forest_change_2023_v1_11": {
    en: { title: "Hansen Global 30m Forest Cover & Loss", subtitle: "Annual deforestation year and 2000 baseline canopy cover (2000–2023)" },
    ja: { title: "Hansen 全球 30m 森林被覆・減少変化", subtitle: "2000–2023年の年次森林喪失年と基準樹冠被覆率" },
  },
  "COPERNICUS/S5P/OFFL/L3_NO2": {
    en: { title: "Sentinel-5P Tropospheric NO2", subtitle: "Emissions and regional air-quality tracking" },
    ja: { title: "Sentinel-5P 対流圏二酸化窒素 (NO2)", subtitle: "大気汚染排出および地域大気質モニタリング" },
  },
  "NOAA/VIIRS/DNB/MONTHLY_V1/VCMSLCFG": {
    en: { title: "VIIRS DNB Monthly Nighttime Lights", subtitle: "460m stray-light-corrected radiance for outage & economic analysis" },
    ja: { title: "VIIRS DNB 月次夜間光（迷光補正済み）", subtitle: "460m 地域経済活動・停電検知・光害研究向け放射輝度" },
  },
};

export function localizeGeeDatasetEntry(entry: GeePublicDatasetEntry, locale: Locale): GeePublicDatasetEntry {
  if (locale === "zh-CN") return entry;
  const tr = GEE_DATASET_TITLES[entry.assetId]?.[locale];
  if (!tr) return entry;
  return {
    ...entry,
    title: tr.title,
    subtitle: tr.subtitle,
    era: entry.era.replace("至今", locale === "en" ? "Present" : "現在").replace("基准产品", locale === "en" ? "Baseline" : "基準データ"),
    coverageNote: tr.coverageNote ?? entry.coverageNote,
  };
}
