import { t } from "../../i18n/index.ts";
import type {
  NetworkProxySettings,
  GlobalProxyMode,
  SourceProxyOverride,
} from "./types.ts";

export const DEFAULT_GLOBAL_PROXY_MODE: GlobalProxyMode = "direct";

export interface SourceProxyMeta {
  id: string;
  name_zh: string;
  badge_zh?: string;
  default_override: SourceProxyOverride;
}

export const KNOWN_NETWORK_SOURCES: SourceProxyMeta[] = [
  {
    id: "cdse",
    name_zh: "Copernicus Data Space (CDSE)",
    default_override: "follow_global",
  },
  {
    id: "earthdata",
    name_zh: "NASA Earthdata (ASF DAAC)",
    default_override: "follow_global",
  },
  {
    id: "bhoonidhi",
    get name_zh() { return t("shared.networkSettings.bhoonidhiName"); },
    get badge_zh() { return t("shared.networkSettings.defaultExceptionBadge"); },
    default_override: "system",
  },
  {
    id: "usgs_m2m",
    name_zh: "USGS EarthExplorer / M2M",
    default_override: "follow_global",
  },
  {
    id: "mpc",
    name_zh: "Microsoft Planetary Computer (MPC)",
    default_override: "follow_global",
  },
];

export function getDefaultNetworkProxySettings(): NetworkProxySettings {
  const overrides: Record<string, SourceProxyOverride> = {};
  for (const src of KNOWN_NETWORK_SOURCES) {
    overrides[src.id] = src.default_override;
  }
  return {
    global_mode: DEFAULT_GLOBAL_PROXY_MODE,
    manual_host: "127.0.0.1",
    manual_port: 7890,
    source_overrides: overrides,
  };
}

export function validateProxyUrl(rawUrl: string): { ok: boolean; error_zh?: string } {
  const trimmed = rawUrl.trim();
  if (!trimmed) {
    return { ok: false, error_zh: t("shared.networkSettings.emptyProxyError") };
  }

  // 1. 检查凭据泄露风险 (user:pass@)
  if (trimmed.includes("@")) {
    return {
      ok: false,
      error_zh: t("shared.networkSettings.credentialsForbiddenError"),
    };
  }

  // 2. 检查协议及显式端口
  const urlCandidate =
    trimmed.startsWith("http://") || trimmed.startsWith("https://")
      ? trimmed
      : `http://${trimmed}`;

  try {
    const parsed = new URL(urlCandidate);
    if (!parsed.port) {
      return {
        ok: false,
        error_zh: t("shared.networkSettings.missingPortError"),
      };
    }
    const portNum = Number(parsed.port);
    if (isNaN(portNum) || portNum < 1 || portNum > 65535) {
      return { ok: false, error_zh: t("shared.networkSettings.invalidPortRangeError") };
    }
    return { ok: true };
  } catch (_e) {
    return { ok: false, error_zh: t("shared.networkSettings.invalidProxyFormatError") };
  }
}

export const PROXY_MODE_OPTIONS: { value: GlobalProxyMode; label_zh: string }[] = [
  { value: "direct", get label_zh() { return t("shared.networkSettings.proxyModes.direct"); } },
  { value: "system", get label_zh() { return t("shared.networkSettings.proxyModes.system"); } },
  { value: "manual", get label_zh() { return t("shared.networkSettings.proxyModes.manual"); } },
];

export const SOURCE_OVERRIDE_OPTIONS: {
  value: SourceProxyOverride;
  label_zh: string;
}[] = [
  { value: "follow_global", get label_zh() { return t("shared.networkSettings.sourceOverrides.follow_global"); } },
  { value: "system", get label_zh() { return t("shared.networkSettings.sourceOverrides.system"); } },
  { value: "direct", get label_zh() { return t("shared.networkSettings.sourceOverrides.direct"); } },
  { value: "manual", get label_zh() { return t("shared.networkSettings.sourceOverrides.manual"); } },
];

/// The three-way UI projection of the persisted two-field proxy state:
/// disabled = direct, enabled with an empty URL = automatic system proxy,
/// enabled with a URL = manual HTTP proxy.
export function projectProxyMode(settings: {
  proxy_enabled: boolean;
  proxy_url: string;
}): GlobalProxyMode {
  if (!settings.proxy_enabled) return "direct";
  if (!settings.proxy_url || !settings.proxy_url.trim()) return "system";
  return "manual";
}

/// The persisted payload fields for one UI mode selection. The automatic
/// system mode persists an enabled proxy with an empty URL; the backend keeps
/// that representation and builds system-proxy clients from it.
export function proxyFieldsForMode(
  mode: GlobalProxyMode,
  manual: { host: string; port: string },
): { proxy_enabled: boolean; proxy_url: string } {
  return {
    proxy_enabled: mode !== "direct",
    proxy_url:
      mode === "manual"
        ? `http://${manual.host.trim()}:${manual.port.trim()}`
        : "",
  };
}

export const PROBE_ERROR_REASONS: Record<string, string> = {
  get timeout() { return t("shared.networkSettings.probeReasons.timeout"); },
  get transport_unavailable() { return t("shared.networkSettings.probeReasons.transport_unavailable"); },
  get proxy_configuration_rejected() { return t("shared.networkSettings.probeReasons.proxy_configuration_rejected"); },
};


export const TEST_ENDPOINTS = [
  {
    id: "cdse",
    name: "Copernicus Data Space (CDSE)",
    site: "https://dataspace.copernicus.eu/",
  },
  {
    id: "earthdata",
    name: "NASA Earthdata / ASF DAAC",
    site: "https://search.asf.alaska.edu/",
  },
  {
    id: "bhoonidhi",
    name: "ISRO Bhoonidhi Portal",
    site: "https://bhoonidhi.nrsc.gov.in/",
  },
  {
    id: "usgs_m2m",
    name: "USGS EarthExplorer / M2M",
    site: "https://earthexplorer.usgs.gov/",
  },
  {
    id: "mpc",
    name: "Microsoft Planetary Computer (MPC)",
    site: "https://planetarycomputer.microsoft.com/",
  },
  {
    id: "opentopography",
    name: "OpenTopography",
    site: "https://portal.opentopography.org/",
  },
  {
    id: "gee",
    name: "Google Earth Engine (GEE)",
    site: "https://earthengine.google.com/",
  },
  {
    id: "gacos",
    name: "GACOS Generic Atmospheric Correction",
    site: "http://www.gacos.net/",
  },
  {
    id: "comet_licsar",
    name: "COMET-LiCSAR Interferometry Portal",
    site: "https://comet.nerc.ac.uk/COMET-LiCS-portal/",
  },
];
