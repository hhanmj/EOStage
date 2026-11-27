import type { Locale } from "@/i18n";
import { EO_ATLAS_CDN_CONFIGURED, resolveAtlasCdnUrl } from "./atlasCdnConfig";
import rawCatalog from "./atlasCatalog.json";

export type ContinentId =
  | "asia"
  | "europe"
  | "africa"
  | "north_america"
  | "south_america"
  | "oceania"
  | "polar";

export type EoModalityKey =
  | "optical"
  | "cir"
  | "swir"
  | "sar"
  | "sar_dp"
  | "dem"
  | "dem_hr"
  | "insar_ifg"
  | "insar_unw"
  | "lulc"
  | "thermal"
  | "water"
  | "nightlight_macro"
  | "nightlight"
  | "hist2014";

export const STAGE1_ASSET_VERSION = "21";
export const GLOBE_ASSET_VERSION = "21";

export const STAGE1_SPRITE_LAYERS = [
  `/onboarding-stage1/mosaic-layer-0.webp?v=${STAGE1_ASSET_VERSION}`,
  `/onboarding-stage1/mosaic-layer-1.webp?v=${STAGE1_ASSET_VERSION}`,
  `/onboarding-stage1/mosaic-layer-2.webp?v=${STAGE1_ASSET_VERSION}`,
  `/onboarding-stage1/mosaic-layer-3.webp?v=${STAGE1_ASSET_VERSION}`,
] as const;

export const STAGE1_HERO_SLOT_INDEX = 13;
export const STAGE1_HERO_TILE_SRC = `/onboarding-stage1/hero-tile.webp?v=${STAGE1_ASSET_VERSION}`;

export const GLOBE_ASSET_URLS = {
  day: `/onboarding-globe/earth-day-4k.webp?v=${GLOBE_ASSET_VERSION}`,
  night: `/onboarding-globe/earth-night.webp?v=${GLOBE_ASSET_VERSION}`,
  specular: `/onboarding-globe/earth-specular.webp?v=${GLOBE_ASSET_VERSION}`,
  clouds: `/onboarding-globe/earth-clouds.webp?v=${GLOBE_ASSET_VERSION}`,
  topology: `/onboarding-globe/earth-topology.webp?v=${GLOBE_ASSET_VERSION}`,
  countries: `/onboarding-globe/countries.json?v=${GLOBE_ASSET_VERSION}`,
} as const;

export const STAGE1_DECK_LAYERS: readonly (readonly [string, string, string, string])[] = [
  STAGE1_SPRITE_LAYERS,
  ...(EO_ATLAS_CDN_CONFIGURED
    ? ([
        [
          resolveAtlasCdnUrl("v2/stage1/deck-1-layer-0.webp?v=6"),
          resolveAtlasCdnUrl("v2/stage1/deck-1-layer-1.webp?v=6"),
          resolveAtlasCdnUrl("v2/stage1/deck-1-layer-2.webp?v=6"),
          resolveAtlasCdnUrl("v2/stage1/deck-1-layer-3.webp?v=6"),
        ],
        [
          resolveAtlasCdnUrl("v2/stage1/deck-2-layer-0.webp?v=6"),
          resolveAtlasCdnUrl("v2/stage1/deck-2-layer-1.webp?v=6"),
          resolveAtlasCdnUrl("v2/stage1/deck-2-layer-2.webp?v=6"),
          resolveAtlasCdnUrl("v2/stage1/deck-2-layer-3.webp?v=6"),
        ],
        [
          resolveAtlasCdnUrl("v2/stage1/deck-3-layer-0.webp?v=6"),
          resolveAtlasCdnUrl("v2/stage1/deck-3-layer-1.webp?v=6"),
          resolveAtlasCdnUrl("v2/stage1/deck-3-layer-2.webp?v=6"),
          resolveAtlasCdnUrl("v2/stage1/deck-3-layer-3.webp?v=6"),
        ],
        [
          resolveAtlasCdnUrl("v2/stage1/deck-4-layer-0.webp?v=6"),
          resolveAtlasCdnUrl("v2/stage1/deck-4-layer-1.webp?v=6"),
          resolveAtlasCdnUrl("v2/stage1/deck-4-layer-2.webp?v=6"),
          resolveAtlasCdnUrl("v2/stage1/deck-4-layer-3.webp?v=6"),
        ],
        [
          resolveAtlasCdnUrl("v2/stage1/deck-5-layer-0.webp?v=6"),
          resolveAtlasCdnUrl("v2/stage1/deck-5-layer-1.webp?v=6"),
          resolveAtlasCdnUrl("v2/stage1/deck-5-layer-2.webp?v=6"),
          resolveAtlasCdnUrl("v2/stage1/deck-5-layer-3.webp?v=6"),
        ],
        [
          resolveAtlasCdnUrl("v2/stage1/deck-6-layer-0.webp?v=6"),
          resolveAtlasCdnUrl("v2/stage1/deck-6-layer-1.webp?v=6"),
          resolveAtlasCdnUrl("v2/stage1/deck-6-layer-2.webp?v=6"),
          resolveAtlasCdnUrl("v2/stage1/deck-6-layer-3.webp?v=6"),
        ],
      ] as const)
    : []),
];

/** Exact count of distinct data modalities per (deckIdx, placeSlotIdx) so each square completes its cycle cleanly without consecutive repeats. */
export const STAGE1_DECK_SLOT_STEPS: readonly string[] = [
  "444444444444444444444444444444444444",
  "444444444444443444444434444444444444",
  "444444444444444444344344444444444444",
  "444344444434444444444444444444444444",
  "444444444444444444444444444444444444",
  "444444444444444444444444444444444444",
  "444444444444444444444444444444444444",
];

export interface Stage1MosaicSlot {
  id: string;
  col: number;
  row: number;
  delayMs: number;
}

export interface AtlasModalityFrame {
  key: EoModalityKey;
  label: Record<Locale, string>;
  badge: string;
  capturedAt?: string;
  src: string;
  kb: number;
  dotColor: string;
}

export interface AtlasPlaceEntry {
  id: string;
  lat: number;
  lon: number;
  label: Record<Locale, string>;
  blurb: Record<Locale, string>;
  modalities: readonly AtlasModalityFrame[];
}

export interface AtlasCountryEntry {
  iso3: string;
  continent: ContinentId;
  name: Record<Locale, string>;
  tagline: Record<Locale, string>;
  places: readonly AtlasPlaceEntry[];
}

export interface CountrySummaryRecord {
  iso3: string;
  nameEn: string;
  labelLon: number;
  labelLat: number;
}

export const MODALITY_DOT_COLORS: Record<EoModalityKey, string> = {
  optical: "#60a5fa",
  cir: "#fb7185",
  swir: "#a78bfa",
  sar: "#cbd5e1",
  sar_dp: "#f472b6",
  dem: "#34d399",
  dem_hr: "#10b981",
  insar_ifg: "#f43f5e",
  insar_unw: "#38bdf8",
  lulc: "#4ade80",
  thermal: "#fb923c",
  water: "#22d3ee",
  nightlight_macro: "#facc15",
  nightlight: "#facc15",
  hist2014: "#94a3b8",
};

export const CONTINENT_TABS: ReadonlyArray<{
  id: ContinentId;
  labels: Record<Locale, string>;
  defaultTagline: Record<Locale, string>;
}> = [
  {
    id: "asia",
    labels: { "zh-CN": "亚洲", en: "Asia", ja: "アジア" },
    defaultTagline: {
      "zh-CN": "亚太山海走廊 · 高原与大河三角洲",
      en: "ASIA-PACIFIC PLATEAUS & RIVER DELTAS",
      ja: "アジア太平洋の高原と河口デルタ",
    },
  },
  {
    id: "europe",
    labels: { "zh-CN": "欧洲", en: "Europe", ja: "ヨーロッパ" },
    defaultTagline: {
      "zh-CN": "欧洲海峡、峡湾与高山冰川",
      en: "EUROPEAN FJORDS, STRAITS & ALPINE SPINES",
      ja: "ヨーロッパの海峡とアルプス氷河",
    },
  },
  {
    id: "africa",
    labels: { "zh-CN": "非洲", en: "Africa", ja: "アフリカ" },
    defaultTagline: {
      "zh-CN": "东非裂谷、萨赫勒与沙漠绿洲",
      en: "RIFT VALLEYS, DELTAS & SAHARAN MASSIFS",
      ja: "大地溝帯とサハラ砂漠の緑洲",
    },
  },
  {
    id: "north_america",
    labels: { "zh-CN": "北美洲", en: "North America", ja: "北米" },
    defaultTagline: {
      "zh-CN": "落基山冰原、峡谷与海湾",
      en: "CORDILLERAS, CANYONS & TIDAL BAYS",
      ja: "ロッキー氷原と大峡谷・湾岸",
    },
  },
  {
    id: "south_america",
    labels: { "zh-CN": "南美洲", en: "South America", ja: "南米" },
    defaultTagline: {
      "zh-CN": "安第斯雪峰与亚马孙水脉",
      en: "ANDES PEAKS & AMAZONIAN WATERSHEDS",
      ja: "アンデス高峰とアマゾン流域",
    },
  },
  {
    id: "oceania",
    labels: { "zh-CN": "大洋洲", en: "Oceania", ja: "オセアニア" },
    defaultTagline: {
      "zh-CN": "南太平洋珊瑚环礁与火山弧",
      en: "CORAL ATOLLS & VOLCANIC ARCHIPELAGOS",
      ja: "南太平洋サンゴ環礁と火山弧",
    },
  },
  {
    id: "polar",
    labels: { "zh-CN": "极地", en: "Polar", ja: "極地" },
    defaultTagline: {
      "zh-CN": "极地冰盖、冻土峡湾与无冰干谷",
      en: "POLAR ICE SHELVES, FJORDS & DRY VALLEYS",
      ja: "極地氷床・フィヨルド・無氷谷",
    },
  },
];

const CURATED_TAGLINES: Partial<Record<string, Record<Locale, string>>> = {
  CHN: { "zh-CN": "山海都市、高原峡谷、大河湿地与宝岛纵谷", en: "PEAKS, DELTAS, PLATEAUS & TAIWAN RIFT", ja: "山岳・都市圏・長江湿地・台湾縦谷" },
  JPN: { "zh-CN": "活火山弧、海湾与海岸都市群", en: "VOLCANIC CONES & PACIFIC BAYS", ja: "活火山弧と臨海ベイエリア" },
  USA: { "zh-CN": "湾区盐田、德州圆盘农田、峡谷与冰川火山", en: "BAYS, TEXAS PIVOTS, CANYONS & KĪLAUEA", ja: "湾岸塩田・テキサス円形農地・峡谷・火山" },
  MEX: { "zh-CN": "中央高原古湖盆与波波卡特佩特雪火山", en: "MEXICAN BASIN & SNOW-CAPPED VOLCANO CONE", ja: "メキシコ盆地と雪冠活火山" },
  CAN: { "zh-CN": "落基山冰原与北极永久冻土湖群", en: "ROCKY ICEFIELDS & ARCTIC THERMOKARST", ja: "ロッキー氷原と極地湖沼デルタ" },
  EGY: { "zh-CN": "尼罗河绿色三角洲与新苏伊士运河", en: "NILE GREEN DELTA & SUEZ CROSSROADS", ja: "ナイル・デルタとスエズ運河" },
  ARE: { "zh-CN": "波斯湾海岸沙漠与人工岛屿群", en: "PERSIAN GULF ARCHIPELAGOS & COASTAL DUNES", ja: "ペルシャ湾海岸と人工島群" },
  TUR: { "zh-CN": "欧亚跨海咽喉与东安纳托利亚大断裂", en: "BOSPHORUS STRAIT & ANATOLIAN FAULTS", ja: "欧亜海峡と東アナトリア断層" },
  NLD: { "zh-CN": "莱茵河口填海巨港与温室玻璃田", en: "RECLAIMED POLDER DELTAS & EUROPORT", ja: "ライン河口干拓地と大温室群" },
  MRT: { "zh-CN": "撒哈拉之眼地质穹窿与阿尔金沙洲滩", en: "RICHAT RING DOME & ATLANTIC SHOALS", ja: "サハラの目と大西洋砂洲" },
  ISL: { "zh-CN": "大西洋中脊活裂谷熔岩与瓦特纳冰帽", en: "MID-ATLANTIC LAVA RIFTS & OUTWASH ICE CAPS", ja: "大西洋裂谷溶岩と氷帽" },
  CHL: { "zh-CN": "阿塔卡马盐沼、百内冰原与毛莱火山高原", en: "ANDEAN SALARS, VOLCANO LAKES & ICEFIELDS", ja: "塩湖・火山高原・パタゴニア氷河" },
  SGP: { "zh-CN": "赤道海峡岛屿与深水港湾", en: "EQUATORIAL STRAIT & DEEPWATER PORTS", ja: "赤道海峡と国際海港" },
  SAU: { "zh-CN": "阿拉伯圆盘绿洲与鲁卜哈利星形大沙海", en: "DESERT AQUIFERS & RUB' AL KHALI DUNES", ja: "円形灌漑と大砂丘海" },
  ETH: { "zh-CN": "青尼罗河大峡谷与埃尔塔阿雷熔岩裂谷", en: "BLUE NILE GORGE & ERTA ALE RIFT VALLEY", ja: "青ナイル大峡谷とアファール裂谷" },
  PER: { "zh-CN": "亚马孙蛇行牛轭湖与安第斯萨班卡亚活火山", en: "UCAYALI OXBOW LAKES & SABANCAYA VOLCANO", ja: "蛇行河川とアンデス活火山" },
  AUS: { "zh-CN": "大堡礁珊瑚环礁与澳洲腹地乌鲁鲁红岩", en: "GREAT BARRIER REEF & ULURU MONOLITHS", ja: "珊瑚環礁とウルル一枚岩" },
  NZL: { "zh-CN": "南阿尔卑斯冰湖与北岛鲁阿佩胡活火山链", en: "SOUTHERN ALPS GLACIERS & RUAPEHU VOLCANOES", ja: "氷河湖と北島活火山帯" },
  ITA: { "zh-CN": "西西里—坎帕尼亚双活火山与亚得里亚泻湖", en: "ACTIVE MEDITERRANEAN VOLCANOES & LAGOONS", ja: "地中海活火山群とヴェネツィア潟" },
  FRA: { "zh-CN": "大西洋潮汐孤岛、勃朗冰峰与留尼汪活火山", en: "TIDAL BAYS, MONT BLANC & FOURNAISE CALDERA", ja: "干潟・モンブラン・活火山" },
  GBR: { "zh-CN": "泰晤士河曲、港湾与不列颠都市走廊", en: "THAMES ESTUARY & ISLAND METROPOLIS", ja: "テムズ川湾曲部と都市帯" },
  NOR: { "zh-CN": "北欧千米峭壁峡湾与北极斯瓦尔巴冰原", en: "GLACIAL GORGE FJORDS & ARCTIC ARCHIPELAGOS", ja: "氷河大フィヨルドと極地諸島" },
  RUS: { "zh-CN": "贝加尔深水大裂谷与堪察加活火山链", en: "BAIKAL RIFT LAKE & KAMCHATKA VOLCANOES", ja: "バイカル裂谷湖とカムチャツカ火山" },
  IND: { "zh-CN": "恒河大平原、红树河口与卡奇盐沼", en: "GANGETIC PLAINS, MANGROVES & SALARS", ja: "ガンジス平原と大湿地・塩沼" },
  BRA: { "zh-CN": "马瑙斯黑白双河奇观与千湖白沙泻湖", en: "MEETING OF TWO RIVERS & DUNE LAGOONS", ja: "二大河川合流と砂丘湖群" },
  ARG: { "zh-CN": "南巴塔哥尼亚乌普萨拉与莫雷诺巨大冰舌", en: "PATAGONIAN OUTLET GLACIERS & PROGLACIAL LAKES", ja: "パタゴニア巨大氷河と氷河湖" },
  ATA: { "zh-CN": "麦克默多无冰干谷与西南极断裂冰架前缘", en: "ANTARCTIC DRY VALLEYS & CALVING SHELVES", ja: "無氷ドライバレーと棚氷前縁" },
  GRL: { "zh-CN": "北极崩解巨冰川峡湾与玄武岩冰盖边缘", en: "ARCTIC CALVING GLACIERS & GIANT FJORDS", ja: "崩壊大氷河と極地フィヨルド" },
  GRC: { "zh-CN": "爱琴海月牙沉陷火山岛与科林斯陡崖", en: "AEGEAN CALDERAS & CORINTH RIFT SCARPS", ja: "エーゲ海カルデラと地溝断崖" },
  IDN: { "zh-CN": "巽他—爪哇弧链活火山与热带岛海", en: "SUNDA VOLCANIC ARCS & CORAL SEAS", ja: "スンダ・ジャワ活火山列島" },
  PHL: { "zh-CN": "弧形破火山湖群与吕宋中央裂谷", en: "CALDERA LAKES & ARCHIPELAGO RIDGES", ja: "二重カルデラとルソン山脈" },
  IRN: { "zh-CN": "扎格罗斯背斜褶皱、盐穹与粉红盐湖", en: "ZAGROS FOLDS, SALT DIAPIRS & PLAYAS", ja: "ザグロス褶曲帯・岩塩ドーム" },
  MAR: { "zh-CN": "高阿特拉斯山脉红色折叠山谷与绿洲", en: "HIGH ATLAS GORGES & PIEDMONT OASES", ja: "アトラス山脈峡谷と緑洲" },
  COD: { "zh-CN": "尼拉贡戈熔岩湖火山口与刚果河网道", en: "NYIRAGONGO LAVA VOLCANO & CONGO BEND", ja: "ニラゴンゴ活火山とコンゴ川" },
};

// 9 x 4 grid slots with brisk scattered-organic Planet-like entrance (0..258ms)
export const STAGE1_MOSAIC_SLOTS: readonly Stage1MosaicSlot[] = Array.from({ length: 36 }, (_, idx) => {
  const col = idx % 9;
  const row = Math.floor(idx / 9);
  const distFromCenter = Math.hypot(col - 4, (row - 1.5) * 1.45);
  return {
    id: `slot-${String(idx + 1).padStart(2, "0")}`,
    col,
    row,
    delayMs: Math.min(260, Math.round(distFromCenter * 18 + ((idx * 17) % 11) * 16)),
  };
});

interface RawCatalogShape {
  iso3: string;
  continent: string;
  featured?: boolean;
  name: Record<Locale, string>;
  tagline?: Record<Locale, string>;
  places: Array<{
    id: string;
    lat: number;
    lon: number;
    label: Record<Locale, string>;
    blurb: Record<Locale, string>;
    modalities: Array<{
      key: string;
      label: Record<Locale, string>;
      badge: string;
      capturedAt?: string;
      src: string;
      kb: number;
    }>;
  }>;
}

const continentFallbackMap = new Map(CONTINENT_TABS.map((c) => [c.id, c.defaultTagline]));

export const ATLAS_COUNTRY_LIST: readonly AtlasCountryEntry[] = (rawCatalog as RawCatalogShape[]).map(
  (entry) => {
    const continent = (continentFallbackMap.has(entry.continent as ContinentId)
      ? entry.continent
      : "europe") as ContinentId;
    const tagline =
      entry.tagline ?? CURATED_TAGLINES[entry.iso3] ?? continentFallbackMap.get(continent)!;
    return {
      iso3: entry.iso3,
      continent,
      name: entry.name,
      tagline,
      places: entry.places.map((p) => ({
        id: p.id,
        lat: p.lat,
        lon: p.lon,
        label: p.label,
        blurb: p.blurb,
        modalities: p.modalities.map((m) => {
          const mKey = (m.key in MODALITY_DOT_COLORS ? m.key : "optical") as EoModalityKey;
          return {
            key: mKey,
            label: m.label,
            badge: m.badge,
            ...(m.capturedAt ? { capturedAt: m.capturedAt } : {}),
            src: resolveAtlasCdnUrl(m.kb ? `${m.src}?v=${m.kb}` : m.src),
            kb: m.kb,
            dotColor: MODALITY_DOT_COLORS[mKey],
          };
        }),
      })),
    };
  }
);

export const ATLAS_COUNTRIES_BY_ISO3: Readonly<Record<string, AtlasCountryEntry>> = Object.fromEntries(
  ATLAS_COUNTRY_LIST.map((c) => [c.iso3, c])
);

const ENGLISH_DISPLAY_SHORTENER: Record<string, string> = {
  "People's Republic of China": "China",
  "United States of America": "United States",
  "Democratic Republic of the Congo": "DR Congo",
  "Central African Republic": "Central African Rep.",
  "United Arab Emirates": "UAE",
  "Bosnia and Herzegovina": "Bosnia & Herz.",
  "Dominican Republic": "Dominican Rep.",
  "Republic of the Congo": "Congo",
  "Solomon Islands": "Solomon Is.",
  "Turkish Republic of Northern Cyprus": "N. Cyprus",
  "Equatorial Guinea": "Eq. Guinea",
};

export function formatLocalizedCountryName(
  iso3: string,
  englishFallback: string,
  locale: Locale
): string {
  const rec = ATLAS_COUNTRIES_BY_ISO3[iso3];
  if (rec) {
    return rec.name[locale] || rec.name.en;
  }
  return ENGLISH_DISPLAY_SHORTENER[englishFallback] ?? englishFallback;
}

export function classifyOceanName(lat: number, lon: number, locale: Locale): string {
  let en = "Pacific Ocean";
  let zh = "太平洋";
  let ja = "太平洋";

  if (lat > 66.5) {
    en = "Arctic Ocean";
    zh = "北冰洋";
    ja = "北極海";
  } else if (lat < -60) {
    en = "Southern Ocean";
    zh = "南大洋";
    ja = "南極海";
  } else if ((lon > -70 && lon < 20) || (lon > -98 && lon <= -70 && lat > 9 && lat < 32)) {
    en = "Atlantic Ocean";
    zh = "大西洋";
    ja = "大西洋";
  } else if (lon >= 20 && lat < 30 && ((lat >= -8 && lon < 105) || (lat < -8 && lon <= 146))) {
    en = "Indian Ocean";
    zh = "印度洋";
    ja = "インド洋";
  }

  return locale === "zh-CN" ? zh : locale === "ja" ? ja : en;
}

const COUNTRY_SEARCH_ALIASES: Record<string, string> = {
  usa: "USA",
  us: "USA",
  america: "USA",
  texas: "USA",
  德州: "USA",
  德克萨斯: "USA",
  uk: "GBR",
  britain: "GBR",
  england: "GBR",
  uae: "ARE",
  dubai: "ARE",
  holland: "NLD",
  netherlands: "NLD",
  korea: "KOR",
  russia: "RUS",
  china: "CHN",
  zhongguo: "CHN",
  taiwan: "CHN",
  台湾: "CHN",
  台北: "CHN",
  花莲: "CHN",
  阿里山: "CHN",
  tibet: "CHN",
  西藏: "CHN",
  "three gorges": "CHN",
  三峡: "CHN",
  "hong kong": "CHN",
  香港: "CHN",
  macau: "CHN",
  澳门: "CHN",
  australia: "AUS",
  澳大利亚: "AUS",
  japan: "JPN",
  fuji: "JPN",
  sahara: "MRT",
  richat: "MRT",
  iceland: "ISL",
  atacama: "CHL",
  amazon: "PER",
  reef: "AUS",
  egypt: "EGY",
  suez: "EGY",
  antarctica: "ATA",
  greenland: "GRL",
  mexico: "MEX",
  cdmx: "MEX",
  volcano: "ITA",
  etna: "ITA",
  kilauea: "USA",
  sakurajima: "JPN",
  fournaise: "FRA",
  agung: "IDN",
  merapi: "IDN",
  taal: "PHL",
};

export interface AtlasSearchHit {
  id: string;
  iso3: string;
  label: string;
  sublabel: string;
  lat: number;
  lon: number;
  placeIndex?: number;
}

export function searchAtlasTargets(
  query: string,
  globeCountries: readonly CountrySummaryRecord[],
  locale: Locale,
  limit = 8
): AtlasSearchHit[] {
  const q = query.trim().toLowerCase();
  if (!q) return [];

  const globeCenterMap = new Map(globeCountries.map((g) => [g.iso3, g]));
  const hits: AtlasSearchHit[] = [];
  const seenIds = new Set<string>();

  const aliasIso3 = COUNTRY_SEARCH_ALIASES[q];
  if (aliasIso3 && ATLAS_COUNTRIES_BY_ISO3[aliasIso3]) {
    const c = ATLAS_COUNTRIES_BY_ISO3[aliasIso3];
    const p0 = c.places[0];
    const gc = globeCenterMap.get(aliasIso3);
    hits.push({
      id: `country-${aliasIso3}`,
      iso3: aliasIso3,
      label: c.name[locale],
      sublabel: p0 ? p0.label[locale] : c.tagline[locale],
      lat: gc?.labelLat ?? p0?.lat ?? 0,
      lon: gc?.labelLon ?? p0?.lon ?? 0,
    });
    seenIds.add(`country-${aliasIso3}`);
  }

  for (const c of ATLAS_COUNTRY_LIST) {
    const cNames = `${c.iso3} ${c.name["zh-CN"]} ${c.name.en} ${c.name.ja}`.toLowerCase();
    const gc = globeCenterMap.get(c.iso3);
    const firstPlace = c.places[0];
    if (cNames.includes(q) && !seenIds.has(`country-${c.iso3}`)) {
      hits.push({
        id: `country-${c.iso3}`,
        iso3: c.iso3,
        label: c.name[locale],
        sublabel: firstPlace ? firstPlace.label[locale] : c.tagline[locale],
        lat: gc?.labelLat ?? firstPlace?.lat ?? 0,
        lon: gc?.labelLon ?? firstPlace?.lon ?? 0,
      });
      seenIds.add(`country-${c.iso3}`);
    }

    c.places.forEach((p, idx) => {
      const pText = `${p.id} ${p.label["zh-CN"]} ${p.label.en} ${p.label.ja}`.toLowerCase();
      if (pText.includes(q) && !seenIds.has(`place-${c.iso3}-${p.id}`)) {
        hits.push({
          id: `place-${c.iso3}-${p.id}`,
          iso3: c.iso3,
          label: p.label[locale],
          sublabel: c.name[locale],
          lat: p.lat,
          lon: p.lon,
          placeIndex: idx,
        });
        seenIds.add(`place-${c.iso3}-${p.id}`);
      }
    });
  }

  return hits.slice(0, limit);
}
