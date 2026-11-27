import registryJson from "../../../../contracts/v3/resource-registry.v1.json" with { type: "json" };

export interface SarProductTypeOption {
  label: string;
  levels: string[];
}

export interface SarPolarizationOption {
  key: string;
  options: string[];
  multiple: boolean;
}

export interface SarKeyFilter {
  key: string;
  min?: number | null;
  max?: number | null;
  platformLimits?: Array<{ platform: string; minimum: number; maximum: number }>;
}

export interface SarFrameFilter {
  key: string;
  valueType: string;
  exactWidth?: number | null;
  min?: number | null;
  max?: number | null;
  platformLimits?: Array<{ platform: string; minimum: number; maximum: number }>;
}

/**
 * 依据所选卫星平台收缩 Path / Frame 有效边界
 */
export function resolveEffectiveNumericBounds(
  filter: {
    min?: number | null;
    max?: number | null;
    platformLimits?: Array<{ platform: string; minimum: number; maximum: number }>;
  } | null | undefined,
  selectedPlatforms: string[] = [],
): { min: number | null; max: number | null } {
  if (!filter) return { min: null, max: null };
  let min = filter.min ?? null;
  let max = filter.max ?? null;
  const limits = filter.platformLimits;
  if (limits && limits.length > 0 && selectedPlatforms.length > 0) {
    let narrowMin: number | null = min;
    let narrowMax: number | null = max;
    const upperSelected = new Set(selectedPlatforms.map((p) => p.toUpperCase()));
    const matching = limits.filter((l) => upperSelected.has(l.platform.toUpperCase()));
    if (matching.length > 0) {
      for (const l of matching) {
        narrowMin = narrowMin === null ? l.minimum : Math.max(narrowMin, l.minimum);
        narrowMax = narrowMax === null ? l.maximum : Math.min(narrowMax, l.maximum);
      }
    }
    return { min: narrowMin, max: narrowMax };
  }
  return { min, max };
}

export interface FilterFieldDependency {
  field: string;
  value: string;
  allow_values: string[];
}

export interface FilterFieldSchema {
  id: string;
  label: string;
  control: "Select" | "CheckboxGroup" | "RadioGroup" | "NumberInput" | "RangeSlider";
  options?: Array<{ value: string; label: string }>;
  min?: number;
  max?: number;
  tier: "common" | "advanced" | "fixed";
  depends_on?: FilterFieldDependency;
  status: "verified" | "pending_backend";
}

export interface SarFilterProfile {
  kind: "legacy" | "typed";
  selector: string;
  platforms: string[];
  productTypes: SarProductTypeOption[];
  polarization: SarPolarizationOption | null;
  beamModes: SarPolarizationOption | null;
  lookDirection: SarPolarizationOption | null;
  flightDirection: boolean;
  relativeOrbit: SarKeyFilter | null;
  path: SarKeyFilter | null;
  frame: SarFrameFilter | null;
  fields: FilterFieldSchema[];
}

export interface SarFilterInput {
  platforms?: string[];
  productType?: string | null;
  polarizations?: string[];
  beamModes?: string[];
  lookDirection?: string | null;
  orbitDirection?: string | null;
  relativeOrbit?: number | string | null;
  path?: number | string | null;
  frame?: number | string | null;
}

function buildDeclarativeFields(params: {
  selector: string;
  platforms: string[];
  productTypes: SarProductTypeOption[];
  polarization: SarPolarizationOption | null;
  beamModes: SarPolarizationOption | null;
  lookDirection: SarPolarizationOption | null;
  flightDirection: boolean;
  relativeOrbit: SarKeyFilter | null;
  path: SarKeyFilter | null;
  frame: SarFrameFilter | null;
}): FilterFieldSchema[] {
  const fields: FilterFieldSchema[] = [];

  if (params.productTypes.length > 0) {
    const isCommonProductType = params.selector === "SENTINEL-1";
    fields.push({
      id: "productType",
      label: "mapWorkbench.conditions.productType",
      control: "RadioGroup",
      options: params.productTypes.map((t) => ({ value: t.label, label: t.label })),
      tier: params.productTypes.length === 1 && !isCommonProductType ? "fixed" : isCommonProductType ? "common" : "advanced",
      status: "verified",
    });
  } else if (params.selector === "SLC-BURST") {
    fields.push({
      id: "productType",
      label: "mapWorkbench.conditions.productType",
      control: "RadioGroup",
      options: [{ value: "BURST", label: "Burst" }],
      tier: "fixed",
      status: "verified",
    });
  }

  if (params.polarization && params.polarization.options.length > 0) {
    fields.push({
      id: "polarizations",
      label: "mapWorkbench.conditions.polarization",
      control: params.polarization.multiple ? "CheckboxGroup" : "RadioGroup",
      options: params.polarization.options.map((opt) => ({ value: opt, label: opt })),
      tier: "common",
      depends_on:
        params.selector === "ALOS-2"
          ? {
              field: "beamModes",
              value: "WBS",
              allow_values: ["HH"],
            }
          : undefined,
      status: "verified",
    });
  }

  if (params.flightDirection) {
    fields.push({
      id: "orbitDirection",
      label: "mapWorkbench.conditions.orbitDirection",
      control: "RadioGroup",
      options: [
        { value: "ascending", label: "mapWorkbench.conditions.ascending" },
        { value: "descending", label: "mapWorkbench.conditions.descending" },
      ],
      tier: "common",
      status: "verified",
    });
  }

  if (params.beamModes && params.beamModes.options.length > 0) {
    fields.push({
      id: "beamModes",
      label: "mapWorkbench.conditions.beamMode",
      control: params.beamModes.multiple ? "CheckboxGroup" : "RadioGroup",
      options: params.beamModes.options.map((opt) => ({ value: opt, label: opt })),
      tier: params.beamModes.options.length === 1 && params.selector === "NISAR" ? "fixed" : "advanced",
      status: "verified",
    });
  }

  if (params.lookDirection && params.lookDirection.options.length > 0) {
    fields.push({
      id: "lookDirection",
      label: "mapWorkbench.conditions.lookDirection",
      control: "RadioGroup",
      options: params.lookDirection.options.map((opt) => ({
        value: opt,
        label: opt === "RIGHT" ? "mapWorkbench.conditions.lookRight" : "mapWorkbench.conditions.lookLeft",
      })),
      tier: "advanced",
      status: "verified",
    });
  }

  if (params.platforms.length > 0) {
    fields.push({
      id: "platforms",
      label: "mapWorkbench.conditions.platforms",
      control: "CheckboxGroup",
      options: params.platforms.map((p) => ({ value: p, label: p })),
      tier: params.platforms.length > 1 ? "advanced" : "fixed",
      status: "verified",
    });
  }

  if (params.relativeOrbit) {
    fields.push({
      id: "relativeOrbit",
      label: "mapWorkbench.conditions.relativeOrbit",
      control: "NumberInput",
      min: params.relativeOrbit.min ?? 1,
      max: params.relativeOrbit.max ?? 175,
      tier: "advanced",
      status: "verified",
    });
  }

  if (params.path) {
    fields.push({
      id: "path",
      label: "mapWorkbench.conditions.pathNumber",
      control: "NumberInput",
      min: params.path.min ?? 1,
      max: params.path.max ?? 671,
      tier: "advanced",
      status: "verified",
    });
  }

  if (params.frame) {
    fields.push({
      id: "frame",
      label: "mapWorkbench.conditions.frameNumber",
      control: "NumberInput",
      min: params.frame.min ?? 0,
      max: params.frame.max ?? 7200,
      tier: "advanced",
      status: "verified",
    });
  }

  if (params.selector === "NISAR") {
    fields.push(
      {
        id: "lookDirection",
        label: "mapWorkbench.conditions.lookDirection",
        control: "RadioGroup",
        options: [
          { value: "RIGHT", label: "mapWorkbench.conditions.lookRight" },
          { value: "LEFT", label: "mapWorkbench.conditions.lookLeft" },
        ],
        tier: "advanced",
        status: "pending_backend",
      },
      {
        id: "rangeBandwidth",
        label: "mapWorkbench.conditions.rangeBandwidth",
        control: "Select",
        tier: "advanced",
        status: "pending_backend",
      },
      {
        id: "jointObservation",
        label: "mapWorkbench.conditions.jointObservation",
        control: "RadioGroup",
        tier: "advanced",
        status: "pending_backend",
      },
      {
        id: "frameCoverage",
        label: "mapWorkbench.conditions.frameCoverage",
        control: "RadioGroup",
        tier: "advanced",
        status: "pending_backend",
      }
    );
  }

  return fields;
}

/**
 * 返回对用户可见（已验证且非固定值）的声明式条件字段
 */
export function visibleFilterFields(
  profile: SarFilterProfile | null,
  tier?: "common" | "advanced"
): FilterFieldSchema[] {
  if (!profile) return [];
  return profile.fields.filter(
    (field) =>
      field.status === "verified" &&
      field.tier !== "fixed" &&
      (tier === undefined || field.tier === tier)
  );
}

/**
 * 依据当前波束模式（如 ALOS-2 选 WBS 时只允许 HH）返回当前可选的极化值列表
 */
export function allowedPolarizationsForBeamMode(
  profile: SarFilterProfile | null,
  beamModes?: string[]
): string[] {
  if (!profile?.polarization) return [];
  const baseOptions = profile.polarization.options;
  const polField = profile.fields.find((f) => f.id === "polarizations");
  const dep = polField?.depends_on;
  if (
    dep &&
    dep.field === "beamModes" &&
    Array.isArray(beamModes) &&
    beamModes.includes(dep.value) &&
    beamModes.length === 1
  ) {
    return baseOptions.filter((opt) => dep.allow_values.includes(opt));
  }
  return baseOptions;
}

/**
 * 依据 resource-registry.v1.json 解析各 SAR 数据源的过滤参数支持能力
 */
export function sarFilterProfile(resourceId: string): SarFilterProfile | null {
  if (!resourceId) return null;
  const normalizedId =
    resourceId === "science.alos-2.scansar.v1"
      ? "science.alos-2.palsar-2.scansar-l1.1.v1"
      : resourceId;
  const resource = (registryJson.resources as any[]).find(
    (r) => r.common?.resource_id === normalizedId
  );
  if (!resource) return null;

  const sarSearch = resource.science?.sar_search;
  if (!sarSearch) return null;

  const projectionKind = sarSearch.projection_kind;
  if (projectionKind === "legacy_asf_search") {
    const platforms: string[] = Array.isArray(sarSearch.platforms)
      ? [...sarSearch.platforms]
      : [];

    const productFamilies: string[] = Array.isArray(sarSearch.product_families)
      ? sarSearch.product_families
      : [];
    const processingLevels: string[] = Array.isArray(sarSearch.processing_levels)
      ? sarSearch.processing_levels
      : [];

    const productTypes: SarProductTypeOption[] = [];
    for (const f of productFamilies) {
      const levels = processingLevels.filter(
        (lvl) => lvl === f || lvl.startsWith(f + "_")
      );
      if (levels.length > 0) {
        productTypes.push({ label: f, levels });
      }
    }

    let polarization: SarPolarizationOption | null = null;
    if (Array.isArray(sarSearch.polarizations) && sarSearch.polarizations.length > 0) {
      polarization = {
        key: "polarizations",
        options: [...sarSearch.polarizations],
        multiple: true,
      };
    }

    let beamModes: SarPolarizationOption | null = null;
    if (Array.isArray(sarSearch.beam_modes) && sarSearch.beam_modes.length > 0) {
      const rawModes: string[] = sarSearch.beam_modes;
      const options =
        sarSearch.selector === "SENTINEL-1"
          ? ["IW", "EW", "SM", "WV"].filter((m) => rawModes.includes(m))
          : [...rawModes];
      beamModes = {
        key: "beam_modes",
        options,
        multiple: true,
      };
    }

    const lookDirection: SarPolarizationOption | null = sarSearch.supports_look_direction
      ? {
          key: "look_direction",
          options: ["RIGHT", "LEFT"],
          multiple: false,
        }
      : null;

    const flightDirection = Boolean(sarSearch.supports_flight_direction);
    const trackLimits: Array<{ platform: string; minimum: number; maximum: number }> = Array.isArray(
      sarSearch.track_filter?.platform_limits
    )
      ? sarSearch.track_filter.platform_limits
      : [];
    const frameLimits: Array<{ platform: string; minimum: number; maximum: number }> = Array.isArray(
      sarSearch.frame_filter?.platform_limits
    )
      ? sarSearch.frame_filter.platform_limits
      : [];
    const relativeOrbit = sarSearch.supports_relative_orbit
      ? {
          key: "relative_orbit",
          min: sarSearch.track_filter?.minimum ?? null,
          max: sarSearch.track_filter?.maximum ?? null,
          platformLimits: trackLimits,
        }
      : null;
    // crates/insar-provider/src/asf/query.rs:1300-1320:
    // relative_orbit 与 path 映射到同一后台参数且不得冲突；
    // profile.path 仅在没有 relativeOrbit 时才暴露（如 ALOS PALSAR, ERS, JERS-1, RADARSAT-1, SEASAT）
    const path =
      !relativeOrbit && sarSearch.supports_path && sarSearch.track_filter?.supported
        ? {
            key: "path",
            min: sarSearch.track_filter?.minimum ?? null,
            max: sarSearch.track_filter?.maximum ?? null,
            platformLimits: trackLimits,
          }
        : null;

    let frame: SarFrameFilter | null = null;
    if (sarSearch.supports_frame && sarSearch.frame_filter?.supported) {
      const key =
        sarSearch.frame_filter.parameter === "asfframe" ? "asf_frame" : "frame";
      frame = {
        key,
        valueType: sarSearch.frame_filter.value_type ?? "integer",
        exactWidth: sarSearch.frame_filter.exact_width ?? null,
        min: sarSearch.frame_filter.minimum ?? null,
        max: sarSearch.frame_filter.maximum ?? null,
        platformLimits: frameLimits,
      };
    }

    const fields = buildDeclarativeFields({
      selector: sarSearch.selector,
      platforms,
      productTypes,
      polarization,
      beamModes,
      lookDirection,
      flightDirection,
      relativeOrbit,
      path,
      frame,
    });

    return {
      kind: "legacy",
      selector: sarSearch.selector,
      platforms,
      productTypes,
      polarization,
      beamModes,
      lookDirection,
      flightDirection,
      relativeOrbit,
      path,
      frame,
      fields,
    };
  }

  if (projectionKind === "typed_catalog_profile") {
    const sourceFilters = sarSearch.source_filters ?? {};
    const supportedKeys: string[] = Array.isArray(sourceFilters.supported_keys)
      ? sourceFilters.supported_keys
      : [];
    const optionsMap = sourceFilters.options ?? {};

    let polarization: SarPolarizationOption | null = null;
    const candidates: Array<{ key: string; multiple: boolean }> = [
      { key: "polarizations", multiple: true },
      { key: "polarization", multiple: false },
      { key: "frequency_a_polarization", multiple: false },
    ];
    for (const cand of candidates) {
      if (supportedKeys.includes(cand.key)) {
        // crates/insar-provider/src/asf/query.rs:203-218:
        // ALOS-2 只接受 ["HH"] 或 ["HH", "HV"]（已排序），单独的 "HV" 被后端 catalogue_filter_error 拒绝。
        // 将有效组合显式呈现为 ["HH", "HH+HV"]
        if (sarSearch.selector === "ALOS-2" && cand.key === "polarizations") {
          polarization = {
            key: cand.key,
            options: ["HH", "HH+HV"],
            multiple: true,
          };
          break;
        }

        const options =
          optionsMap[cand.key] ??
          (Array.isArray(sarSearch.polarizations) ? sarSearch.polarizations : []);
        polarization = {
          key: cand.key,
          options: [...options],
          multiple: cand.multiple,
        };
        break;
      }
    }

    let beamModes: SarPolarizationOption | null = null;
    if (supportedKeys.includes("beam_mode")) {
      const rawBeamOptions: string[] = Array.isArray(optionsMap.beam_mode)
        ? optionsMap.beam_mode
        : Array.isArray(sarSearch.beam_modes)
          ? sarSearch.beam_modes
          : [];
      const orderedOptions =
        sarSearch.selector === "ALOS-2" &&
        rawBeamOptions.includes("WBS") &&
        rawBeamOptions.includes("WBD")
          ? ["WBS", "WBD"]
          : [...rawBeamOptions];
      if (orderedOptions.length > 0) {
        beamModes = {
          key: "beam_mode",
          options: orderedOptions,
          multiple: false,
        };
      }
    }

    const lookDirection: SarPolarizationOption | null =
      sarSearch.selector !== "NISAR" &&
      Boolean(sarSearch.supports_look_direction) &&
      supportedKeys.includes("look_direction")
        ? {
            key: "look_direction",
            options: ["RIGHT", "LEFT"],
            multiple: false,
          }
        : null;

    const flightDirection =
      Boolean(sarSearch.supports_flight_direction) &&
      supportedKeys.includes("flight_direction");
    const typedTrackLimits: Array<{ platform: string; minimum: number; maximum: number }> = Array.isArray(
      sarSearch.track_filter?.platform_limits
    )
      ? sarSearch.track_filter.platform_limits
      : [];
    const typedFrameLimits: Array<{ platform: string; minimum: number; maximum: number }> = Array.isArray(
      sarSearch.frame_filter?.platform_limits
    )
      ? sarSearch.frame_filter.platform_limits
      : [];
    const relativeOrbit =
      sarSearch.supports_relative_orbit && supportedKeys.includes("relative_orbit")
        ? {
            key: "relative_orbit",
            min: sarSearch.track_filter?.minimum ?? null,
            max: sarSearch.track_filter?.maximum ?? null,
            platformLimits: typedTrackLimits,
          }
        : null;
    const path =
      !relativeOrbit && sarSearch.supports_path && supportedKeys.includes("path")
        ? {
            key: "path",
            min: sarSearch.track_filter?.minimum ?? null,
            max: sarSearch.track_filter?.maximum ?? null,
            platformLimits: typedTrackLimits,
          }
        : null;
    const frame =
      sarSearch.supports_frame && supportedKeys.includes("frame")
        ? {
            key: "frame",
            valueType: sarSearch.frame_filter?.value_type ?? "integer",
            exactWidth: sarSearch.frame_filter?.exact_width ?? null,
            min: sarSearch.frame_filter?.minimum ?? null,
            max: sarSearch.frame_filter?.maximum ?? null,
            platformLimits: typedFrameLimits,
          }
        : null;

    const fields = buildDeclarativeFields({
      selector: sarSearch.selector,
      platforms: [],
      productTypes: [],
      polarization,
      beamModes,
      lookDirection,
      flightDirection,
      relativeOrbit,
      path,
      frame,
    });

    return {
      kind: "typed",
      selector: sarSearch.selector,
      platforms: [],
      productTypes: [],
      polarization,
      beamModes,
      lookDirection,
      flightDirection,
      relativeOrbit,
      path,
      frame,
      fields,
    };
  }

  return null;
}

/**
 * 依据 profile 与输入构建严格合规的 filters 对象
 */
export function buildSarFilters(
  profile: SarFilterProfile | null,
  input: SarFilterInput
): Record<string, unknown> {
  if (!profile) return {};
  const filters: Record<string, unknown> = {};

  if (profile.kind === "legacy") {
    filters.dataset = profile.selector;

    if (input.platforms && input.platforms.length > 0) {
      const validPlatforms = input.platforms.filter((p) =>
        profile.platforms.includes(p)
      );
      if (validPlatforms.length > 0) {
        filters.platforms = validPlatforms;
      }
    }

    if (input.productType) {
      const matchedType = profile.productTypes.find(
        (t) => t.label === input.productType
      );
      if (matchedType && matchedType.levels.length > 0) {
        let levels = matchedType.levels;

        // crates/insar-provider/src/asf/query.rs:1153-1180 validate_sentinel_grd_polarizations:
        // 当 dataset 为 SENTINEL-1 且产品类型为 GRD 时：
        // 若指定了极化筛选：
        //   双极化（含 '+'，或 'DV'/'DH'）-> 收窄为双极化专属级别 [GRD_HD, GRD_MD]
        //   单极化（'VV'/'HH'/'SV'/'SH'）-> 收窄为单极化专属级别 [GRD_HS, GRD_MS]
        // 未指定极化 -> 保留全量级别 [GRD_HD, GRD_HS, GRD_MD, GRD_MS]
        if (
          profile.selector === "SENTINEL-1" &&
          input.productType === "GRD" &&
          input.polarizations &&
          input.polarizations.length > 0
        ) {
          const hasDual = input.polarizations.some(
            (p) => p.includes("+") || p === "DV" || p === "DH"
          );
          const hasSingle = input.polarizations.some(
            (p) => p === "SV" || p === "SH" || p === "VV" || p === "HH"
          );
          if (hasDual && !hasSingle) {
            levels = levels.filter(
              (lvl) => lvl === "GRD_HD" || lvl === "GRD_MD" || lvl === "GRD_FD"
            );
          } else if (hasSingle && !hasDual) {
            levels = levels.filter(
              (lvl) => lvl === "GRD_HS" || lvl === "GRD_MS"
            );
          }
        }

        filters.processing_levels = levels;
      }
    }

    if (
      profile.polarization &&
      input.polarizations &&
      input.polarizations.length > 0
    ) {
      const validPol = input.polarizations.filter((p) =>
        profile.polarization!.options.includes(p)
      );
      if (validPol.length > 0) {
        filters.polarizations = validPol;
      }
    }

    if (
      profile.beamModes &&
      input.beamModes &&
      input.beamModes.length > 0
    ) {
      const validBeams = input.beamModes.filter((mode) =>
        profile.beamModes!.options.includes(mode)
      );
      if (validBeams.length > 0) {
        filters[profile.beamModes.key] = validBeams;
      }
    }

    if (profile.lookDirection && input.lookDirection) {
      const look = input.lookDirection.trim().toUpperCase();
      if (look === "LEFT" || look === "RIGHT") {
        filters[profile.lookDirection.key] = look;
      }
    }

    if (profile.flightDirection && input.orbitDirection) {
      const dir = input.orbitDirection.trim().toUpperCase();
      if (dir === "ASCENDING" || dir === "DESCENDING") {
        filters.flight_direction = dir;
      }
    }

    if (
      profile.relativeOrbit &&
      input.relativeOrbit !== undefined &&
      input.relativeOrbit !== null &&
      input.relativeOrbit !== ""
    ) {
      const num =
        typeof input.relativeOrbit === "number"
          ? input.relativeOrbit
          : parseInt(String(input.relativeOrbit), 10);
      if (Number.isInteger(num)) {
        filters.relative_orbit = num;
      }
    }

    if (
      profile.path &&
      input.path !== undefined &&
      input.path !== null &&
      input.path !== ""
    ) {
      const num =
        typeof input.path === "number"
          ? input.path
          : parseInt(String(input.path), 10);
      if (Number.isInteger(num)) {
        filters.path = num;
      }
    }

    if (
      profile.frame &&
      input.frame !== undefined &&
      input.frame !== null &&
      input.frame !== ""
    ) {
      if (profile.frame.key === "asf_frame") {
        const raw = String(input.frame).trim();
        if (profile.frame.valueType === "padded_string" && profile.frame.exactWidth) {
          filters.asf_frame = raw.padStart(profile.frame.exactWidth, "0");
        } else {
          filters.asf_frame = raw;
        }
      } else {
        const num =
          typeof input.frame === "number"
            ? input.frame
            : parseInt(String(input.frame).trim(), 10);
        if (Number.isInteger(num)) {
          filters.frame = num;
        }
      }
    }

    return filters;
  }

  if (profile.kind === "typed") {
    // 绝不输出 dataset, processing_levels, platforms

    if (
      profile.beamModes &&
      input.beamModes &&
      input.beamModes.length > 0
    ) {
      const validBeams = input.beamModes.filter((mode) =>
        profile.beamModes!.options.includes(mode)
      );
      if (validBeams.length > 0) {
        if (profile.beamModes.multiple) {
          filters[profile.beamModes.key] = validBeams;
        } else {
          filters[profile.beamModes.key] = validBeams[0];
        }
      }
    }

    if (profile.lookDirection && input.lookDirection) {
      const look = input.lookDirection.trim().toUpperCase();
      if (look === "LEFT" || look === "RIGHT") {
        filters[profile.lookDirection.key] = look;
      }
    }

    if (profile.flightDirection && input.orbitDirection) {
      const dir = input.orbitDirection.trim().toUpperCase();
      if (dir === "ASCENDING" || dir === "DESCENDING") {
        filters.flight_direction = dir;
      }
    }

    if (
      profile.relativeOrbit &&
      input.relativeOrbit !== undefined &&
      input.relativeOrbit !== null &&
      input.relativeOrbit !== ""
    ) {
      const num =
        typeof input.relativeOrbit === "number"
          ? input.relativeOrbit
          : parseInt(String(input.relativeOrbit), 10);
      if (Number.isInteger(num)) {
        filters[profile.relativeOrbit.key] = num;
      }
    }

    if (
      profile.path &&
      input.path !== undefined &&
      input.path !== null &&
      input.path !== ""
    ) {
      const num =
        typeof input.path === "number"
          ? input.path
          : parseInt(String(input.path), 10);
      if (Number.isInteger(num)) {
        filters[profile.path.key] = num;
      }
    }

    if (
      profile.frame &&
      input.frame !== undefined &&
      input.frame !== null &&
      input.frame !== ""
    ) {
      const num =
        typeof input.frame === "number"
          ? input.frame
          : parseInt(String(input.frame), 10);
      if (Number.isInteger(num)) {
        filters[profile.frame.key] = num;
      }
    }

    if (
      profile.polarization &&
      input.polarizations &&
      input.polarizations.length > 0
    ) {
      const allowedOptions = allowedPolarizationsForBeamMode(
        profile,
        typeof filters.beam_mode === "string"
          ? [filters.beam_mode]
          : input.beamModes
      );
      if (profile.selector === "ALOS-2" && profile.polarization.key === "polarizations") {
        // crates/insar-provider/src/asf/query.rs:203-218:
        // ALOS-2 选 "HH+HV" 时发射 ["HH", "HV"]；选 "HH" 时发射 ["HH"]；单独的 "HV" 绝不发射。
        // 若波束模式为单极化 WBS，则自动收窄为 ["HH"]。
        if (
          allowedOptions.includes("HH+HV") &&
          input.polarizations.includes("HH+HV")
        ) {
          filters.polarizations = ["HH", "HV"];
        } else if (
          input.polarizations.includes("HH") ||
          input.polarizations.includes("HH+HV")
        ) {
          filters.polarizations = ["HH"];
        }
      } else {
        const validPol = input.polarizations.filter((p) =>
          allowedOptions.includes(p)
        );
        if (validPol.length > 0) {
          if (profile.polarization.multiple) {
            filters[profile.polarization.key] = validPol;
          } else {
            filters[profile.polarization.key] = validPol[0];
          }
        }
      }
    }

    return filters;
  }

  return filters;
}

/**
 * 切换数据源时清洗旧过滤参数，剔除新 profile 不支持的字段
 */
export function cleanStaleFilterValues(
  _oldProfile: SarFilterProfile | null,
  newProfile: SarFilterProfile | null,
  currentValues: SarFilterInput
): SarFilterInput {
  if (!newProfile) {
    return {
      platforms: [],
      productType: null,
      polarizations: [],
      beamModes: [],
      lookDirection: null,
      orbitDirection: null,
      relativeOrbit: null,
      path: null,
      frame: null,
    };
  }

  const next: SarFilterInput = { ...currentValues };

  // platforms
  if (currentValues.platforms && currentValues.platforms.length > 0) {
    next.platforms = currentValues.platforms.filter((p) =>
      newProfile.platforms.includes(p)
    );
  } else {
    next.platforms = [];
  }

  // productType (NISAR 官方 9 级产品保留合法级别；其余沿用 profile.productTypes 白名单)
  if (newProfile.selector.toUpperCase() === "NISAR") {
    if (
      currentValues.productType &&
      !isValidNisarProductLevel(currentValues.productType)
    ) {
      next.productType = null;
    }
  } else if (
    currentValues.productType &&
    !newProfile.productTypes.some((t) => t.label === currentValues.productType)
  ) {
    next.productType = null;
  }

  // beamModes
  if (
    newProfile.beamModes &&
    currentValues.beamModes &&
    currentValues.beamModes.length > 0
  ) {
    const validBeams = currentValues.beamModes.filter((mode) =>
      newProfile.beamModes!.options.includes(mode)
    );
    next.beamModes = newProfile.beamModes.multiple
      ? validBeams
      : validBeams.slice(0, 1);
  } else {
    next.beamModes = [];
  }

  // lookDirection
  if (newProfile.lookDirection && currentValues.lookDirection) {
    const look = currentValues.lookDirection.trim().toUpperCase();
    next.lookDirection =
      look === "LEFT" || look === "RIGHT" ? look : null;
  } else {
    next.lookDirection = null;
  }

  // orbitDirection
  if (!newProfile.flightDirection) {
    next.orbitDirection = null;
  }

  // relativeOrbit
  if (!newProfile.relativeOrbit) {
    next.relativeOrbit = null;
  }

  // path
  if (!newProfile.path) {
    next.path = null;
  }

  // frame
  if (!newProfile.frame) {
    next.frame = null;
  }

  // polarizations (含 ALOS-2 WBS 极化联动清洗)
  if (newProfile.polarization && currentValues.polarizations) {
    const allowedPol = allowedPolarizationsForBeamMode(
      newProfile,
      next.beamModes
    );
    const mapped = currentValues.polarizations.map((p) =>
      newProfile.selector === "ALOS-2" &&
      p === "HH+HV" &&
      !allowedPol.includes("HH+HV") &&
      allowedPol.includes("HH")
        ? "HH"
        : p
    );
    next.polarizations = Array.from(
      new Set(mapped.filter((p) => allowedPol.includes(p)))
    );
  } else {
    next.polarizations = [];
  }

  return next;
}

export type NisarProductLevelCode =
  | "RSLC"
  | "GSLC"
  | "GCOV"
  | "GUNW"
  | "GOFF"
  | "RIFG"
  | "RUNW"
  | "ROFF"
  | "SME2";

export interface NisarProductLevelOption {
  value: NisarProductLevelCode;
  label: string;
  scienceLevel: "L1" | "L2" | "L3";
  descriptionZh: string;
  lBandActive: boolean;
  sBandActive: boolean;
}

export const NISAR_OFFICIAL_PRODUCT_LEVELS: readonly NisarProductLevelOption[] = [
  {
    value: "RSLC",
    label: "RSLC",
    scienceLevel: "L1",
    descriptionZh: "距离-多普勒单视复数影像 (L1)",
    lBandActive: true,
    sBandActive: true,
  },
  {
    value: "GSLC",
    label: "GSLC",
    scienceLevel: "L2",
    descriptionZh: "地理编码单视复数影像 (L2)",
    lBandActive: true,
    sBandActive: true,
  },
  {
    value: "GCOV",
    label: "GCOV",
    scienceLevel: "L2",
    descriptionZh: "极化协方差矩阵地面产品 (L2)",
    lBandActive: true,
    sBandActive: true,
  },
  {
    value: "GUNW",
    label: "GUNW",
    scienceLevel: "L2",
    descriptionZh: "地理编码解缠干涉图 (L2)",
    lBandActive: true,
    sBandActive: true,
  },
  {
    value: "GOFF",
    label: "GOFF",
    scienceLevel: "L2",
    descriptionZh: "地理编码偏移量跟踪场 (L2)",
    lBandActive: true,
    sBandActive: true,
  },
  {
    value: "RIFG",
    label: "RIFG",
    scienceLevel: "L1",
    descriptionZh: "距离-多普勒缠绕干涉图 (L1)",
    lBandActive: true,
    sBandActive: true,
  },
  {
    value: "RUNW",
    label: "RUNW",
    scienceLevel: "L1",
    descriptionZh: "距离-多普勒解缠干涉图 (L1)",
    lBandActive: true,
    sBandActive: true,
  },
  {
    value: "ROFF",
    label: "ROFF",
    scienceLevel: "L1",
    descriptionZh: "距离-多普勒像素偏移量 (L1)",
    lBandActive: true,
    sBandActive: true,
  },
  {
    value: "SME2",
    label: "SME2",
    scienceLevel: "L3",
    descriptionZh: "表层土壤湿度产品 (L3 · 200m)",
    lBandActive: true,
    sBandActive: false,
  },
];

const NISAR_PRODUCT_LEVEL_SET: ReadonlySet<string> = new Set(
  NISAR_OFFICIAL_PRODUCT_LEVELS.map((o) => o.value)
);

export function isNisarLogicalResourceId(resourceId?: string | null): boolean {
  if (!resourceId) return false;
  const norm = resourceId.trim().toLowerCase();
  return (
    norm === "science.nisar.l.v1" ||
    norm === "science.nisar.s.v1" ||
    norm === "science.nisar.l.rslc.provisional.v1" ||
    norm === "science.nisar.s.ssar.rslc.v1" ||
    norm === "science.nisar.s.rslc.v1" ||
    norm.startsWith("sar.nisar.")
  );
}

export function isNisarSBandResourceId(resourceId?: string | null): boolean {
  if (!resourceId) return false;
  const norm = resourceId.trim().toLowerCase();
  return norm.includes(".s.") || norm.includes(".ssar.");
}

export function isValidNisarProductLevel(level?: string | null): level is NisarProductLevelCode {
  if (!level) return false;
  return NISAR_PRODUCT_LEVEL_SET.has(level.trim().toUpperCase());
}

export function resolveNisarProductLevel(level?: string | null): NisarProductLevelCode {
  if (!level) return "RSLC";
  const upper = level.trim().toUpperCase();
  return NISAR_PRODUCT_LEVEL_SET.has(upper) ? (upper as NisarProductLevelCode) : "RSLC";
}

export function nisarProductLevelOptions(_resourceId?: string | null): readonly NisarProductLevelOption[] {
  return NISAR_OFFICIAL_PRODUCT_LEVELS;
}
