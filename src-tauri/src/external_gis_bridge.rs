use insar_gis::SubprocessScratchBudget;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const PREFS_FILE_NAME: &str = "external_gis_prefs.v1.json";
const MAX_OUTPUT_BYTES: usize = 16 * 1024;
const MAX_LIST_ALGORITHMS_OUTPUT_BYTES: usize = 256 * 1024;
const DEFAULT_EXECUTE_TIMEOUT_SECS: u64 = 120;
const VERSION_PROBE_TIMEOUT_SECS: u64 = 3;

const ALLOWED_TOOLCHAINS: &[&str] = &["rustgis", "geolibre", "qgis", "arcgis_pro"];
const ALLOWED_EXECUTION_TOOLCHAINS: &[&str] =
    &["rustgis", "geolibre", "qgis", "arcgis_pro", "conda"];
const ALLOWED_ACTIONS: &[&str] = &[
    "version",
    "list_algorithms",
    "run_algorithm",
    "deliver_project",
];

const GEO_CONDA_PACKAGES: &[&str] = &[
    "gdal",
    "rasterio",
    "geopandas",
    "shapely",
    "pyproj",
    "xarray",
    "h5py",
    "netcdf4",
    "mintpy",
    "isce2",
    "scipy",
    "numpy",
    "fiona",
    "rioxarray",
    "pygmtsar",
];

/// Static isolated Conda/Python scientific runner executed via `python -I -c`
/// so parameters and input paths pass strictly via a host-generated JSON job file.
const CONDA_ISOLATED_RUNNER: &str = r#"import importlib, importlib.metadata, json, os, sys

job_path = sys.argv[1]
with open(job_path, "r", encoding="utf-8") as fh:
    job = json.load(fh)

def probe_packages():
    found = {}
    targets = [
        ("gdal", "osgeo.gdal"),
        ("rasterio", "rasterio"),
        ("geopandas", "geopandas"),
        ("shapely", "shapely"),
        ("pyproj", "pyproj"),
        ("xarray", "xarray"),
        ("h5py", "h5py"),
        ("netCDF4", "netCDF4"),
        ("mintpy", "mintpy"),
        ("scipy", "scipy"),
        ("numpy", "numpy"),
        ("fiona", "fiona"),
    ]
    for label, mod_name in targets:
        try:
            mod = importlib.import_module(mod_name)
            ver = getattr(mod, "__version__", None)
            if not ver:
                try:
                    ver = importlib.metadata.version(label)
                except Exception:
                    ver = "installed"
            found[label] = str(ver)
        except Exception:
            pass
    return found

def write_optional_output(payload):
    out_path = str(job.get("output_path") or "").strip()
    if out_path:
        parent = os.path.dirname(out_path)
        if parent:
            os.makedirs(parent, exist_ok=True)
        tmp = f"{out_path}.tmp.{os.getpid()}"
        with open(tmp, "w", encoding="utf-8") as out_fh:
            json.dump(payload, out_fh, ensure_ascii=False, indent=2)
        os.replace(tmp, out_path)

action = job.get("action")
if action == "version":
    print(json.dumps({
        "ok": True,
        "python": sys.version.split()[0],
        "conda_prefix": os.environ.get("CONDA_PREFIX"),
        "packages": probe_packages(),
    }, ensure_ascii=False))
elif action == "list_algorithms":
    print(json.dumps({
        "ok": True,
        "algorithms": [
            "package_probe",
            "raster_inspect",
            "raster_band_stats",
            "vector_inspect",
            "hdf5_inspect",
        ],
        "packages": probe_packages(),
    }, ensure_ascii=False))
elif action == "run_algorithm":
    alg = str(job.get("algorithm") or "").strip()
    params = job.get("params") or {}
    if not isinstance(params, dict):
        raise ValueError("params must be an object")
    inputs = [str(p).strip() for p in (job.get("input_paths") or []) if str(p).strip()]
    first_input = inputs[0] if inputs else str(params.get("input") or "").strip()

    if alg == "package_probe":
        result = {"ok": True, "algorithm": alg, "python": sys.version.split()[0], "packages": probe_packages()}
    elif alg == "raster_inspect":
        if not first_input:
            raise ValueError("raster_inspect requires input raster path")
        try:
            import rasterio
            with rasterio.open(first_input) as ds:
                result = {
                    "ok": True,
                    "algorithm": alg,
                    "backend": "rasterio",
                    "driver": ds.driver,
                    "width": ds.width,
                    "height": ds.height,
                    "count": ds.count,
                    "dtypes": list(ds.dtypes),
                    "crs": ds.crs.to_string() if ds.crs else None,
                    "bounds": [ds.bounds.left, ds.bounds.bottom, ds.bounds.right, ds.bounds.top],
                    "nodata": ds.nodata,
                }
        except ImportError:
            from osgeo import gdal
            ds = gdal.Open(first_input, gdal.GA_ReadOnly)
            if ds is None:
                raise RuntimeError(f"GDAL failed to open {first_input}")
            gt = ds.GetGeoTransform()
            b1 = ds.GetRasterBand(1)
            result = {
                "ok": True,
                "algorithm": alg,
                "backend": "osgeo.gdal",
                "driver": ds.GetDriver().ShortName if ds.GetDriver() else None,
                "width": ds.RasterXSize,
                "height": ds.RasterYSize,
                "count": ds.RasterCount,
                "geotransform": list(gt) if gt else None,
                "nodata": b1.GetNoDataValue() if b1 else None,
            }
    elif alg == "raster_band_stats":
        if not first_input:
            raise ValueError("raster_band_stats requires input raster path")
        band_idx = int(params.get("band") or 1)
        import numpy as np
        import rasterio
        with rasterio.open(first_input) as ds:
            arr = ds.read(band_idx, masked=True)
            valid = arr.compressed()
            if valid.size == 0:
                stats = {"valid_count": 0, "min": None, "max": None, "mean": None, "std": None}
            else:
                stats = {
                    "valid_count": int(valid.size),
                    "min": float(np.nanmin(valid)),
                    "max": float(np.nanmax(valid)),
                    "mean": float(np.nanmean(valid)),
                    "std": float(np.nanstd(valid)),
                }
            result = {"ok": True, "algorithm": alg, "band": band_idx, "stats": stats}
    elif alg == "vector_inspect":
        if not first_input:
            raise ValueError("vector_inspect requires input vector path")
        import geopandas as gpd
        gdf = gpd.read_file(first_input)
        bounds = [float(x) for x in gdf.total_bounds.tolist()] if len(gdf) > 0 else None
        result = {
            "ok": True,
            "algorithm": alg,
            "feature_count": int(len(gdf)),
            "crs": gdf.crs.to_string() if gdf.crs else None,
            "bounds": bounds,
            "columns": [str(c) for c in gdf.columns],
        }
    elif alg == "hdf5_inspect":
        if not first_input:
            raise ValueError("hdf5_inspect requires input HDF5 path")
        import h5py
        datasets = []
        with h5py.File(first_input, "r") as f:
            def visitor(name, obj):
                if len(datasets) >= 64:
                    return
                if isinstance(obj, h5py.Dataset):
                    datasets.append({
                        "path": name,
                        "shape": list(obj.shape),
                        "dtype": str(obj.dtype),
                    })
            f.visititems(visitor)
        result = {"ok": True, "algorithm": alg, "dataset_count": len(datasets), "datasets": datasets}
    else:
        raise ValueError(f"unsupported conda recipe: {alg}")

    write_optional_output(result)
    print(json.dumps(result, ensure_ascii=False))
else:
    raise ValueError(f"unsupported conda action: {action}")
"#;

/// Static isolated ArcGIS Pro runner executed with `python -I -c` so user
/// parameters travel strictly via a JSON job file and never via string interpolation.
const ARCPY_ISOLATED_RUNNER: &str = r#"import inspect, json, os, sys
job_path = sys.argv[1]
with open(job_path, "r", encoding="utf-8") as fh:
    job = json.load(fh)
import arcpy
action = job.get("action")
if action == "version":
    info = arcpy.GetInstallInfo()
    print(json.dumps({
        "ok": True,
        "python": sys.version.split()[0],
        "product": info.get("ProductName"),
        "version": info.get("Version"),
    }, ensure_ascii=False))
elif action == "list_algorithms":
    tools = list(arcpy.ListTools("*"))[:500]
    print(json.dumps({"ok": True, "count": len(tools), "algorithms": tools}, ensure_ascii=False))
elif action == "run_algorithm":
    alg = str(job.get("algorithm") or "").strip()
    if not alg:
        raise ValueError("missing algorithm name")
    parts = [p for p in alg.split(".") if p]
    if parts and parts[0] == "arcpy":
        parts = parts[1:]
    if not parts or any(p.startswith("_") for p in parts):
        raise ValueError("unsafe or invalid arcpy tool symbol")
    target = arcpy
    for seg in parts:
        target = getattr(target, seg)
    if not callable(target):
        raise ValueError("resolved arcpy target is not callable")
    params = dict(job.get("params") or {})
    controlled_out = str(job.get("controlled_output_path") or "").strip()
    default_out_param = str(job.get("default_output_param") or "out_feature_class").strip()
    if controlled_out:
        out_candidates = (
            "out_rasterdataset",
            "out_raster",
            "out_feature_class",
            "out_dataset",
            "out_table",
            "out_polygon_features",
            "out_point_features",
            "out_polyline_features",
            "out_file",
            "out_folder",
            "out_workspace",
            "OUTPUT",
        )
        lower_existing = {k.lower() for k in params.keys()}
        if not any(c.lower() in lower_existing for c in out_candidates) and "out_path" not in lower_existing and "output_location" not in lower_existing:
            sig_by_lower = {}
            try:
                sig_params = inspect.signature(target).parameters
                sig_by_lower = {k.lower(): k for k in sig_params.keys()}
            except Exception:
                sig_by_lower = {}
            split_pairs = (
                ("out_path", "out_name"),
                ("output_location", "raster_dataset_name_with_extension"),
            )
            matched_pair = None
            for d_key, n_key in split_pairs:
                if d_key in sig_by_lower and n_key in sig_by_lower:
                    matched_pair = (sig_by_lower[d_key], sig_by_lower[n_key])
                    break
            if matched_pair is None and "+" in default_out_param:
                d_key, n_key = default_out_param.split("+", 1)
                matched_pair = (
                    sig_by_lower.get(d_key.lower(), d_key),
                    sig_by_lower.get(n_key.lower(), n_key),
                )
            if matched_pair is not None:
                out_dir_key, out_name_key = matched_pair
                params[out_dir_key] = os.path.dirname(controlled_out)
                params[out_name_key] = os.path.basename(controlled_out)
            else:
                chosen = None
                for cand in (default_out_param,) + out_candidates:
                    if cand.lower() in sig_by_lower:
                        chosen = sig_by_lower[cand.lower()]
                        break
                params[chosen or default_out_param] = controlled_out
    result = target(**params)
    messages = ""
    try:
        messages = result.getMessages()
    except Exception:
        messages = str(result)
    print(json.dumps({"ok": True, "algorithm": alg, "messages": messages}, ensure_ascii=False))
else:
    raise ValueError(f"unsupported arcpy action: {action}")
"#;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalGisToolchainInfo {
    pub id: String,
    pub display_name: String,
    pub installed: bool,
    pub source: String,
    pub executable_path: Option<String>,
    pub gui_path: Option<String>,
    pub version: Option<String>,
    pub mcp_capable: bool,
    pub headless_capable: bool,
    pub gui_capable: bool,
    pub capabilities: Vec<String>,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalCondaEnvInfo {
    pub id: String,
    pub name: String,
    pub source: String,
    pub env_prefix: String,
    pub python_path: String,
    pub python_version: Option<String>,
    pub geo_packages: Vec<String>,
    pub verified: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ExternalGisPrefs {
    #[serde(default)]
    pub custom_paths: BTreeMap<String, String>,
    #[serde(default)]
    pub preferred_toolchain: Option<String>,
    #[serde(default)]
    pub conda_custom_envs: BTreeMap<String, String>,
    #[serde(default)]
    pub preferred_conda_env: Option<String>,
    #[serde(default)]
    pub rtk_custom_path: Option<String>,
    #[serde(default)]
    pub dehydrator_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension_mirror_preset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension_registry_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalGisInspectReport {
    pub ok: bool,
    pub platform: String,
    pub prefs: ExternalGisPrefs,
    pub toolchains: Vec<ExternalGisToolchainInfo>,
    #[serde(default)]
    pub conda_environments: Vec<ExternalCondaEnvInfo>,
    #[serde(default)]
    pub log_dehydrator: crate::log_dehydrator::LogDehydratorInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExternalGisInspectRequest {
    #[serde(default)]
    pub probe_versions: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExternalGisExecuteRequest {
    pub toolchain: String,
    pub action: String,
    #[serde(default)]
    pub algorithm: Option<String>,
    #[serde(default)]
    pub conda_env: Option<String>,
    #[serde(default)]
    pub params: BTreeMap<String, Value>,
    #[serde(default)]
    pub input_paths: Vec<String>,
    #[serde(default)]
    pub output_path: Option<String>,
    #[serde(default)]
    pub open_gui: bool,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalGisExecuteResult {
    pub ok: bool,
    pub toolchain: String,
    pub action: String,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub output_path: Option<String>,
    pub project_path: Option<String>,
    pub catalog_path: Option<String>,
    pub gui_launched: bool,
    pub detail: String,
    #[serde(default)]
    pub artifacts: Vec<String>,
    #[serde(default)]
    pub dehydration: Option<crate::log_dehydrator::DehydrationSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CondaGeoInstallRequest {
    pub package_id: String,
    #[serde(default)]
    pub conda_env: Option<String>,
    #[serde(default)]
    pub confirmed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CondaGeoInstallResult {
    pub ok: bool,
    pub package_id: String,
    pub env_name: String,
    pub command_preview: String,
    pub stdout: String,
    pub stderr: String,
    pub detail: String,
    pub report: Option<ExternalGisInspectReport>,
}

fn validate_toolchain_id(raw: &str) -> Result<String, String> {
    let normalized = raw.trim().to_ascii_lowercase();
    if ALLOWED_TOOLCHAINS.contains(&normalized.as_str()) {
        Ok(normalized)
    } else {
        Err(format!(
            "不支持的外部 GIS 工具链 '{}', 仅支持: {}",
            raw.trim(),
            ALLOWED_TOOLCHAINS.join(", ")
        ))
    }
}

fn validate_execution_toolchain_id(raw: &str) -> Result<String, String> {
    let normalized = raw.trim().to_ascii_lowercase();
    if ALLOWED_EXECUTION_TOOLCHAINS.contains(&normalized.as_str()) {
        Ok(normalized)
    } else {
        Err(format!(
            "不支持的外部工具链 '{}', 仅支持: {}",
            raw.trim(),
            ALLOWED_EXECUTION_TOOLCHAINS.join(", ")
        ))
    }
}

fn validate_action_name(raw: &str) -> Result<String, String> {
    let normalized = raw.trim().to_ascii_lowercase();
    if ALLOWED_ACTIONS.contains(&normalized.as_str()) {
        Ok(normalized)
    } else {
        Err(format!(
            "不支持的外部 GIS 动作 '{}', 仅支持: {}",
            raw.trim(),
            ALLOWED_ACTIONS.join(", ")
        ))
    }
}

fn is_regular_executable(path: &Path) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    if meta.file_type().is_symlink() || !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn is_safe_dir(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|meta| !meta.file_type().is_symlink() && meta.is_dir())
        .unwrap_or(false)
}

fn resolve_qgis_bundle_binary(bundle_or_bin: &Path) -> Option<(PathBuf, Option<PathBuf>)> {
    if is_regular_executable(bundle_or_bin) {
        #[cfg(target_os = "macos")]
        {
            let app_bundle = bundle_or_bin
                .parent()
                .and_then(|p| p.parent())
                .and_then(|p| {
                    if p.file_name().and_then(|n| n.to_str()) == Some("Contents") {
                        p.parent()
                    } else {
                        None
                    }
                })
                .filter(|p| {
                    p.extension()
                        .and_then(|s| s.to_str())
                        .map(|ext| ext.eq_ignore_ascii_case("app"))
                        .unwrap_or(false)
                })
                .map(Path::to_path_buf);
            return Some((bundle_or_bin.to_path_buf(), app_bundle));
        }
        #[cfg(not(target_os = "macos"))]
        {
            return Some((bundle_or_bin.to_path_buf(), None));
        }
    }
    #[cfg(target_os = "macos")]
    {
        if is_safe_dir(bundle_or_bin)
            && bundle_or_bin
                .extension()
                .and_then(|s| s.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("app"))
                .unwrap_or(false)
        {
            for rel in [
                "Contents/MacOS/bin/qgis_process",
                "Contents/MacOS/qgis_process",
            ] {
                let candidate = bundle_or_bin.join(rel);
                if is_regular_executable(&candidate) {
                    return Some((candidate, Some(bundle_or_bin.to_path_buf())));
                }
            }
            for rel in ["Contents/MacOS/QGIS", "Contents/MacOS/QGIS-LTR"] {
                let gui_only = bundle_or_bin.join(rel);
                if is_regular_executable(&gui_only) {
                    return Some((gui_only, Some(bundle_or_bin.to_path_buf())));
                }
            }
            if let Ok(entries) = fs::read_dir(bundle_or_bin.join("Contents/MacOS")) {
                for entry in entries.flatten() {
                    let candidate = entry.path();
                    if let Some(name) = candidate.file_name().and_then(|n| n.to_str()) {
                        if name.starts_with("QGIS") && is_regular_executable(&candidate) {
                            return Some((candidate, Some(bundle_or_bin.to_path_buf())));
                        }
                    }
                }
            }
        }
    }
    None
}

fn validate_custom_executable_path(toolchain: &str, raw_path: &str) -> Result<PathBuf, String> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() || trimmed.contains('\0') {
        return Err(format!("{toolchain} 自定义路径不能为空或包含空字符"));
    }
    let path = PathBuf::from(trimmed);
    if !path.is_absolute() {
        return Err(format!("{toolchain} 自定义路径必须是绝对路径: {trimmed}"));
    }
    for comp in path.components() {
        if matches!(comp, Component::ParentDir) {
            return Err(format!("{toolchain} 自定义路径不允许包含 '..': {trimmed}"));
        }
    }
    let meta = fs::symlink_metadata(&path)
        .map_err(|err| format!("{toolchain} 自定义路径不可访问 ({trimmed}): {err}"))?;
    if meta.file_type().is_symlink() {
        return Err(format!(
            "{toolchain} 自定义路径不能是指向外部的符号链接: {trimmed}"
        ));
    }
    if toolchain == "qgis" {
        if let Some((exe, _)) = resolve_qgis_bundle_binary(&path) {
            return Ok(exe);
        }
    }
    if !meta.is_file() {
        return Err(format!("{toolchain} 自定义路径必须是可执行文件: {trimmed}"));
    }
    if !is_regular_executable(&path) {
        return Err(format!("{toolchain} 自定义路径缺少可执行权限: {trimmed}"));
    }
    Ok(path)
}

fn validate_conda_alias(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.len() > 64
        || !trimmed
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':'))
    {
        return Err(format!("Conda 环境名不合法: '{trimmed}'"));
    }
    Ok(trimmed.to_string())
}

fn validate_absolute_no_parent_traversal(path: &Path, label: &str) -> Result<(), String> {
    let text = path.to_string_lossy();
    if text.trim().is_empty() || text.contains('\0') {
        return Err(format!("{label} 路径不能为空或包含空字符"));
    }
    if !path.is_absolute() {
        return Err(format!("{label} 路径必须是绝对路径: {text}"));
    }
    for comp in path.components() {
        if matches!(comp, Component::ParentDir) {
            return Err(format!("{label} 路径不允许包含 '..': {text}"));
        }
    }
    Ok(())
}

/// Resolves either a Conda environment prefix directory (`/path/to/envs/insar`)
/// or an explicit Python executable inside it (`/path/to/envs/insar/bin/python3`)
/// safely, allowing standard intra-bin Conda Python symlinks (`python3 -> python3.12`)
/// after verifying the canonical target is a safe non-world-writable regular executable.
pub fn resolve_conda_env_python(prefix_or_bin: &Path) -> Result<(PathBuf, PathBuf), String> {
    validate_absolute_no_parent_traversal(prefix_or_bin, "Conda")?;
    let meta = fs::symlink_metadata(prefix_or_bin)
        .map_err(|err| format!("Conda 路径不可访问 ({}): {err}", prefix_or_bin.display()))?;

    if meta.is_dir() && !meta.file_type().is_symlink() {
        #[cfg(windows)]
        let candidates = [
            prefix_or_bin.join("python.exe"),
            prefix_or_bin.join("Scripts").join("python.exe"),
        ];
        #[cfg(not(windows))]
        let candidates = [
            prefix_or_bin.join("bin").join("python3"),
            prefix_or_bin.join("bin").join("python"),
        ];
        for cand in candidates {
            if let Ok(canonical) = fs::canonicalize(&cand) {
                if crate::log_dehydrator::is_safe_executable_file(&canonical) {
                    return Ok((prefix_or_bin.to_path_buf(), cand));
                }
            }
        }
        return Err(format!(
            "Conda 环境目录下未找到可执行 Python ({})",
            prefix_or_bin.display()
        ));
    }

    let canonical = fs::canonicalize(prefix_or_bin).map_err(|err| {
        format!(
            "无法解析 Conda Python 路径 ({}): {err}",
            prefix_or_bin.display()
        )
    })?;
    if !crate::log_dehydrator::is_safe_executable_file(&canonical) {
        return Err(format!(
            "Conda Python 不是安全的常规可执行文件: {}",
            canonical.display()
        ));
    }

    let parent = prefix_or_bin
        .parent()
        .ok_or_else(|| "无法推导 Conda 环境根目录".to_string())?;
    let prefix = match parent.file_name().and_then(|s| s.to_str()) {
        Some("bin") | Some("Scripts") => parent.parent().unwrap_or(parent).to_path_buf(),
        _ => parent.to_path_buf(),
    };
    Ok((prefix, prefix_or_bin.to_path_buf()))
}

const GITHUB_EXTENSION_REGISTRY_URL: &str =
    "https://raw.githubusercontent.com/hhanmj/eostage-hub/main/registry.json";

fn normalize_extension_mirror_prefs(
    preset: Option<&str>,
    url: Option<&str>,
    reject_invalid: bool,
) -> Result<(Option<String>, Option<String>), String> {
    let raw_preset = preset.map(str::trim).filter(|s| !s.is_empty());
    let raw_url = url.map(str::trim).filter(|s| !s.is_empty());
    if raw_preset.is_none() && raw_url.is_none() {
        return Ok((None, None));
    }
    if let Some(p) = raw_preset {
        if p.len() > 64 || p.chars().any(|c| c.is_control()) {
            if reject_invalid {
                return Err(
                    "EXTENSION_MIRROR_PREF_INVALID: 扩展镜像预设包含非法控制字符或长度超限"
                        .to_string(),
                );
            }
            return Ok((None, None));
        }
    }
    if let Some(u) = raw_url {
        if u.len() > 2048 || u.chars().any(|c| c.is_control()) {
            if reject_invalid {
                return Err(
                    "EXTENSION_MIRROR_PREF_INVALID: 扩展仓库地址包含非法控制字符或长度超限"
                        .to_string(),
                );
            }
            return Ok((None, None));
        }
    }
    if raw_preset == Some("cloudflare")
        || raw_url.is_some_and(|u| u.to_ascii_lowercase().contains("eostage-hub.pages.dev"))
    {
        return Ok((
            Some("gitcode".to_string()),
            Some(crate::application::DEFAULT_EXTENSION_REGISTRY_URL.to_string()),
        ));
    }
    let final_preset = match raw_preset {
        Some("gitcode") => "gitcode".to_string(),
        Some("github") => "github".to_string(),
        Some("local") => "local".to_string(),
        Some("custom") | None => match raw_url {
            Some(u) if u == crate::application::DEFAULT_EXTENSION_REGISTRY_URL => {
                "gitcode".to_string()
            }
            Some(GITHUB_EXTENSION_REGISTRY_URL) => "github".to_string(),
            Some("local") => "local".to_string(),
            _ => "custom".to_string(),
        },
        Some(other) => {
            if reject_invalid {
                return Err(format!(
                    "EXTENSION_MIRROR_PRESET_UNSUPPORTED: 不支持的扩展镜像预设 ({other})"
                ));
            }
            "custom".to_string()
        }
    };
    let final_url = match raw_url {
        Some(u) => u.to_string(),
        None => match final_preset.as_str() {
            "gitcode" => crate::application::DEFAULT_EXTENSION_REGISTRY_URL.to_string(),
            "github" => GITHUB_EXTENSION_REGISTRY_URL.to_string(),
            "local" => "local".to_string(),
            _ => crate::application::DEFAULT_EXTENSION_REGISTRY_URL.to_string(),
        },
    };
    Ok((Some(final_preset), Some(final_url)))
}

pub fn load_external_gis_prefs(app_data_dir: &Path) -> Result<ExternalGisPrefs, String> {
    let prefs_path = app_data_dir.join(PREFS_FILE_NAME);
    match fs::symlink_metadata(&prefs_path) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ExternalGisPrefs::default());
        }
        Err(err) => {
            return Err(format!(
                "无法读取外部 GIS 配置元数据 ({}): {err}",
                prefs_path.display()
            ));
        }
        Ok(meta) => {
            if meta.file_type().is_symlink() || !meta.is_file() {
                return Err(format!(
                    "拒绝读取不安全的外部 GIS 配置文件: {}",
                    prefs_path.display()
                ));
            }
        }
    }
    let raw = fs::read_to_string(&prefs_path)
        .map_err(|err| format!("读取外部 GIS 配置失败 ({}): {err}", prefs_path.display()))?;
    let mut parsed: ExternalGisPrefs = serde_json::from_str(&raw).map_err(|err| {
        format!(
            "解析外部 GIS 配置 JSON 失败 ({}): {err}",
            prefs_path.display()
        )
    })?;
    let mut cleaned = BTreeMap::new();
    for (k, v) in parsed.custom_paths {
        if let Ok(norm_k) = validate_toolchain_id(&k) {
            let trimmed_v = v.trim();
            if !trimmed_v.is_empty() {
                cleaned.insert(norm_k, trimmed_v.to_string());
            }
        }
    }
    parsed.custom_paths = cleaned;
    parsed.preferred_toolchain = parsed
        .preferred_toolchain
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| validate_toolchain_id(s).ok());
    let mut cleaned_conda = BTreeMap::new();
    for (k, v) in parsed.conda_custom_envs {
        if let Ok(alias) = validate_conda_alias(&k) {
            let trimmed_v = v.trim();
            if !trimmed_v.is_empty() {
                cleaned_conda.insert(alias, trimmed_v.to_string());
            }
        }
    }
    parsed.conda_custom_envs = cleaned_conda;
    parsed.preferred_conda_env = parsed
        .preferred_conda_env
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| validate_conda_alias(s).ok());
    parsed.rtk_custom_path = parsed
        .rtk_custom_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    parsed.dehydrator_mode = parsed
        .dehydrator_mode
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| crate::log_dehydrator::normalize_dehydrator_mode(Some(s)));
    let (norm_preset, norm_url) = normalize_extension_mirror_prefs(
        parsed.extension_mirror_preset.as_deref(),
        parsed.extension_registry_url.as_deref(),
        false,
    )?;
    parsed.extension_mirror_preset = norm_preset;
    parsed.extension_registry_url = norm_url;
    Ok(parsed)
}

pub fn save_external_gis_prefs(
    app_data_dir: &Path,
    prefs: &ExternalGisPrefs,
) -> Result<ExternalGisPrefs, String> {
    let mut sanitized = ExternalGisPrefs::default();
    for (raw_k, raw_v) in &prefs.custom_paths {
        let key = validate_toolchain_id(raw_k)?;
        let val = raw_v.trim();
        if val.is_empty() {
            continue;
        }
        let validated = validate_custom_executable_path(&key, val)?;
        sanitized
            .custom_paths
            .insert(key, validated.to_string_lossy().to_string());
    }
    if let Some(pref) = prefs.preferred_toolchain.as_deref() {
        let trimmed = pref.trim();
        if !trimmed.is_empty() {
            sanitized.preferred_toolchain = Some(validate_toolchain_id(trimmed)?);
        }
    }
    for (raw_alias, raw_path) in &prefs.conda_custom_envs {
        let alias = validate_conda_alias(raw_alias)?;
        let val = raw_path.trim();
        if val.is_empty() {
            continue;
        }
        let (prefix, _py) = resolve_conda_env_python(Path::new(val))?;
        sanitized
            .conda_custom_envs
            .insert(alias, prefix.to_string_lossy().to_string());
    }
    if let Some(pref_conda) = prefs.preferred_conda_env.as_deref() {
        let trimmed = pref_conda.trim();
        if !trimmed.is_empty() {
            sanitized.preferred_conda_env = Some(validate_conda_alias(trimmed)?);
        }
    }
    if let Some(rtk_path) = prefs.rtk_custom_path.as_deref() {
        let trimmed = rtk_path.trim();
        if !trimmed.is_empty() {
            let validated_rtk = crate::log_dehydrator::validate_custom_rtk_path(trimmed)?;
            sanitized.rtk_custom_path = Some(validated_rtk.to_string_lossy().to_string());
        }
    }
    if let Some(mode) = prefs.dehydrator_mode.as_deref() {
        let trimmed = mode.trim();
        if !trimmed.is_empty() {
            sanitized.dehydrator_mode = Some(crate::log_dehydrator::normalize_dehydrator_mode(
                Some(trimmed),
            ));
        }
    }
    let (norm_preset, norm_url) = normalize_extension_mirror_prefs(
        prefs.extension_mirror_preset.as_deref(),
        prefs.extension_registry_url.as_deref(),
        true,
    )?;
    if norm_preset.is_none() && norm_url.is_none() {
        if let Ok(existing) = load_external_gis_prefs(app_data_dir) {
            sanitized.extension_mirror_preset = existing.extension_mirror_preset;
            sanitized.extension_registry_url = existing.extension_registry_url;
        }
    } else {
        sanitized.extension_mirror_preset = norm_preset;
        sanitized.extension_registry_url = norm_url;
    }

    fs::create_dir_all(app_data_dir)
        .map_err(|err| format!("创建配置目录失败 ({}): {err}", app_data_dir.display()))?;
    let prefs_path = app_data_dir.join(PREFS_FILE_NAME);
    if let Ok(meta) = fs::symlink_metadata(&prefs_path) {
        if meta.file_type().is_symlink() {
            return Err(format!(
                "拒绝写入符号链接外部 GIS 配置文件: {}",
                prefs_path.display()
            ));
        }
    }

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_path = app_data_dir.join(format!(
        ".{PREFS_FILE_NAME}.tmp.{}.{nanos}",
        std::process::id()
    ));
    let payload = serde_json::to_string_pretty(&sanitized)
        .map_err(|err| format!("序列化外部 GIS 配置失败: {err}"))?;

    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    {
        let mut file = opts
            .open(&tmp_path)
            .map_err(|err| format!("创建临时配置文件失败 ({}): {err}", tmp_path.display()))?;
        file.write_all(payload.as_bytes())
            .and_then(|_| file.write_all(b"\n"))
            .and_then(|_| file.sync_all())
            .map_err(|err| {
                let _ = fs::remove_file(&tmp_path);
                format!("写入外部 GIS 配置失败: {err}")
            })?;
    }
    fs::rename(&tmp_path, &prefs_path).map_err(|err| {
        let _ = fs::remove_file(&tmp_path);
        format!("保存外部 GIS 配置失败 ({}): {err}", prefs_path.display())
    })?;
    Ok(sanitized)
}

fn find_in_system_path(candidates: &[&str]) -> Option<PathBuf> {
    let mut search_dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| {
            std::env::split_paths(&p)
                .filter(|d| d.is_absolute())
                .collect()
        })
        .unwrap_or_default();
    #[cfg(target_os = "macos")]
    {
        for extra in ["/opt/homebrew/bin", "/usr/local/bin"] {
            let extra_path = PathBuf::from(extra);
            if !search_dirs.iter().any(|d| d == &extra_path) {
                search_dirs.push(extra_path);
            }
        }
    }
    for dir in search_dirs {
        for name in candidates {
            #[cfg(windows)]
            let suffixes = ["", ".exe", ".bat", ".cmd"];
            #[cfg(not(windows))]
            let suffixes = [""];
            for ext in suffixes {
                let file_name = if ext.is_empty() || name.ends_with(ext) {
                    name.to_string()
                } else {
                    format!("{name}{ext}")
                };
                let candidate = dir.join(file_name);
                if is_regular_executable(&candidate) {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

fn component_candidate_dirs(
    runtime_root: &Path,
    app_data_dir: &Path,
    subdirs: &[&str],
) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for sub in subdirs {
        dirs.push(runtime_root.join("runtime").join(sub));
        dirs.push(runtime_root.join(sub));
        dirs.push(app_data_dir.join("runtime-components").join(sub));
        dirs.push(app_data_dir.join("components").join(sub));
    }
    dirs
}

fn apply_sanitized_subprocess_env(cmd: &mut Command, executable: Option<&Path>) {
    cmd.env_clear();
    #[cfg(unix)]
    const SAFE_KEYS: &[&str] = &["PATH", "HOME", "LANG", "LC_ALL", "LC_CTYPE", "TERM"];
    #[cfg(windows)]
    const SAFE_KEYS: &[&str] = &[
        "PATH",
        "SYSTEMROOT",
        "COMSPEC",
        "WINDIR",
        "PROGRAMFILES",
        "PROGRAMFILES(X86)",
        "COMMONPROGRAMFILES",
        "LOCALAPPDATA",
        "APPDATA",
        "USERPROFILE",
        "OS",
        "PROCESSOR_ARCHITECTURE",
        "PATHEXT",
        "QT_QPA_PLATFORM",
        "OSGEO4W_ROOT",
        "QGIS_PREFIX_PATH",
    ];
    for key in SAFE_KEYS {
        if let Some(val) = std::env::var_os(key) {
            cmd.env(key, val);
        }
    }
    #[cfg(windows)]
    if let Some(exe) = executable {
        if let Some(parent) = exe.parent() {
            let file_name = exe
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if file_name == "python.exe" && parent.join("conda-meta").is_dir() {
                cmd.env("CONDA_PREFIX", parent);
            }
        }
    }
    #[cfg(target_os = "macos")]
    if let Some(exe) = executable {
        if let Some(app_bundle) = exe.ancestors().find(|p| {
            p.extension()
                .and_then(|s| s.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("app"))
                .unwrap_or(false)
        }) {
            let qgis_resources = app_bundle.join("Contents/Resources/qgis");
            let proj_dir = qgis_resources.join("proj");
            if proj_dir.join("proj.db").is_file() {
                cmd.env("PROJ_DATA", &proj_dir);
                cmd.env("PROJ_LIB", &proj_dir);
            }
            let gdal_dir = qgis_resources.join("gdal");
            if gdal_dir.is_dir() {
                cmd.env("GDAL_DATA", &gdal_dir);
            }
        }
        let file_name = exe
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if file_name.starts_with("qgis_process") {
            cmd.env("QT_QPA_PLATFORM", "offscreen");
        }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    let _ = executable;
}

fn create_isolated_scratch_dir(app_data_dir: &Path, tag: &str) -> Result<ScratchDirGuard, String> {
    let base = app_data_dir.join("scratch").join("external-gis");
    fs::create_dir_all(&base)
        .map_err(|err| format!("创建外部 GIS 临时目录失败 ({}): {err}", base.display()))?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = base.join(format!("{tag}-{}-{nanos}", std::process::id()));
    fs::create_dir(&dir).map_err(|err| format!("创建隔离工作区失败 ({}): {err}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
    }
    Ok(ScratchDirGuard(dir))
}

struct ScratchDirGuard(PathBuf);

impl Drop for ScratchDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn redact_sensitive_text(raw: &str) -> String {
    redact_sensitive_text_with_limit(raw, MAX_OUTPUT_BYTES)
}

fn redact_sensitive_text_with_limit(raw: &str, max_bytes: usize) -> String {
    let mut cleaned = String::with_capacity(raw.len().min(max_bytes));
    for line in raw.lines() {
        let lower = line.to_ascii_lowercase();
        let redacted_line = if lower.contains("authorization:")
            || lower.contains("bearer ")
            || lower.contains("password=")
            || lower.contains("passwd=")
            || lower.contains("secret=")
            || lower.contains("api_key=")
            || lower.contains("apikey=")
            || lower.contains("access_token=")
            || lower.contains("token=")
        {
            redact_line_tokens(line)
        } else {
            redact_prefixed_tokens(line)
        };
        if !cleaned.is_empty() {
            cleaned.push('\n');
        }
        if cleaned.len() + redacted_line.len() > max_bytes {
            let remaining = max_bytes.saturating_sub(cleaned.len());
            let mut end = remaining.min(redacted_line.len());
            while end > 0 && !redacted_line.is_char_boundary(end) {
                end -= 1;
            }
            cleaned.push_str(&redacted_line[..end]);
            cleaned.push_str("\n... [truncated]");
            break;
        }
        cleaned.push_str(&redacted_line);
    }
    cleaned
}

fn compact_qgis_algorithms_json(raw: &str) -> Option<String> {
    let parsed: Value = serde_json::from_str(raw.trim()).ok()?;
    let providers = parsed.get("providers")?.as_object()?;
    let mut algorithms = Vec::new();
    for prov_val in providers.values() {
        if let Some(algs) = prov_val.get("algorithms").and_then(Value::as_object) {
            for alg_id in algs.keys() {
                algorithms.push(alg_id.clone());
            }
        }
    }
    algorithms.sort();
    serde_json::to_string(&json!({
        "gdal_version": parsed.get("gdal_version").cloned().unwrap_or(Value::Null),
        "proj_version": parsed.get("proj_version").cloned().unwrap_or(Value::Null),
        "count": algorithms.len(),
        "algorithms": algorithms,
    }))
    .ok()
}

fn redact_line_tokens(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for word in line.split_inclusive(char::is_whitespace) {
        let trimmed = word.trim_end();
        let ws = &word[trimmed.len()..];
        let lower = trimmed.to_ascii_lowercase();
        if let Some(eq_idx) = trimmed.find('=') {
            let key = lower[..eq_idx].trim();
            if key.contains("secret")
                || key.contains("password")
                || key.contains("passwd")
                || key.contains("token")
                || key.contains("api_key")
                || key.contains("apikey")
            {
                out.push_str(&trimmed[..=eq_idx]);
                out.push_str("[REDACTED]");
                out.push_str(ws);
                continue;
            }
        }
        out.push_str(&redact_prefixed_tokens(trimmed));
        out.push_str(ws);
    }
    out
}

fn redact_prefixed_tokens(input: &str) -> String {
    let mut text = input.to_string();
    for prefix in ["sk-", "ghp_", "gho_", "AIza", "AKIA"] {
        while let Some(pos) = text.find(prefix) {
            let end = text[pos..]
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
                .map(|rel| pos + rel)
                .unwrap_or(text.len());
            text.replace_range(pos..end, "[REDACTED]");
        }
    }
    if let Some(scheme_pos) = text.find("://") {
        if let Some(at_rel) = text[scheme_pos + 3..].find('@') {
            let creds_slice = &text[scheme_pos + 3..scheme_pos + 3 + at_rel];
            if creds_slice.contains(':') && !creds_slice.contains(' ') {
                text.replace_range(scheme_pos + 3..scheme_pos + 3 + at_rel, "[REDACTED]");
            }
        }
    }
    text
}

fn probe_rustgis_mcp_version(executable: &Path, app_data_dir: &Path) -> Option<String> {
    let scratch = create_isolated_scratch_dir(app_data_dir, "probe-mcp").ok()?;
    let mut cmd = Command::new(executable);
    apply_sanitized_subprocess_env(&mut cmd, Some(executable));
    cmd.current_dir(&scratch.0);
    cmd.env("CPL_TMPDIR", &scratch.0);
    cmd.env("TMPDIR", &scratch.0);
    cmd.env("TMP", &scratch.0);
    cmd.env("TEMP", &scratch.0);
    cmd.env("PROJ_NETWORK", "OFF");
    cmd.arg("mcp").arg("--no-plugins");
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let init_req = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"eostage\",\"version\":\"1.0\"}}}\n";
        let _ = stdin.write_all(init_req);
        let _ = stdin.flush();
    }

    let deadline = Instant::now() + Duration::from_secs(VERSION_PROBE_TIMEOUT_SECS);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                let mut stdout_buf = Vec::new();
                if let Some(mut out) = child.stdout.take() {
                    let _ = out
                        .by_ref()
                        .take(MAX_OUTPUT_BYTES as u64)
                        .read_to_end(&mut stdout_buf);
                }
                let stdout = String::from_utf8_lossy(&stdout_buf);
                for line in stdout.lines() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if let Ok(parsed) = serde_json::from_str::<Value>(trimmed) {
                        if let Some(ver) = parsed
                            .pointer("/result/serverInfo/version")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|v| !v.is_empty())
                        {
                            let name = parsed
                                .pointer("/result/serverInfo/name")
                                .and_then(Value::as_str)
                                .map(str::trim)
                                .filter(|n| !n.is_empty())
                                .unwrap_or("rustgis");
                            return Some(redact_sensitive_text(&format!("{name} {ver}")));
                        }
                    }
                }
                return None;
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                thread::sleep(Duration::from_millis(25));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

fn probe_cli_version(executable: &Path, toolchain: &str, app_data_dir: &Path) -> Option<String> {
    if toolchain == "rustgis" {
        if let Some(ver) = probe_rustgis_mcp_version(executable, app_data_dir) {
            return Some(ver);
        }
    }
    let scratch = create_isolated_scratch_dir(app_data_dir, "probe").ok()?;
    let budget = SubprocessScratchBudget {
        max_scratch_bytes: 32 * 1024 * 1024,
        max_scratch_files: 64,
        timeout: Duration::from_secs(VERSION_PROBE_TIMEOUT_SECS),
        poll_interval: Duration::from_millis(25),
    };
    let mut cmd = if toolchain == "arcgis_pro" {
        let job_path = scratch.0.join("arcpy_version.json");
        fs::write(&job_path, br#"{"action":"version"}"#).ok()?;
        let mut c = Command::new(executable);
        apply_sanitized_subprocess_env(&mut c, Some(executable));
        c.arg("-I")
            .arg("-c")
            .arg(ARCPY_ISOLATED_RUNNER)
            .arg(&job_path);
        c
    } else {
        let mut c = Command::new(executable);
        apply_sanitized_subprocess_env(&mut c, Some(executable));
        c.arg("--version");
        c
    };
    let outcome = budget.execute_command(&mut cmd, &scratch.0).ok()?;
    if outcome.status_code != Some(0) {
        return None;
    }
    let stdout = String::from_utf8_lossy(&outcome.stdout);
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return None;
    }
    if toolchain == "arcgis_pro" {
        if let Ok(parsed) = serde_json::from_str::<Value>(trimmed) {
            if let Some(ver) = parsed.get("version").and_then(Value::as_str) {
                return Some(redact_sensitive_text(ver));
            }
        }
    }
    trimmed
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| redact_sensitive_text(line.trim()))
}

fn discover_qgis(
    custom_path: Option<&str>,
    app_data_dir: &Path,
    probe_versions: bool,
) -> ExternalGisToolchainInfo {
    let mut executable_path: Option<PathBuf> = None;
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut gui_path: Option<PathBuf> = None;
    let mut source = "missing".to_string();
    let mut custom_err: Option<String> = None;

    if let Some(raw) = custom_path {
        match validate_custom_executable_path("qgis", raw) {
            Ok(exe) => {
                executable_path = Some(exe.clone());
                if let Some(app_bundle) = exe.ancestors().find(|p| {
                    p.extension()
                        .and_then(|s| s.to_str())
                        .map(|e| e.eq_ignore_ascii_case("app"))
                        .unwrap_or(false)
                }) {
                    gui_path = Some(app_bundle.to_path_buf());
                } else {
                    gui_path = Some(exe);
                }
                source = "custom_path".to_string();
            }
            Err(err) => custom_err = Some(err),
        }
    }

    if executable_path.is_none() {
        #[cfg(target_os = "macos")]
        {
            let mut app_roots = vec![PathBuf::from("/Applications")];
            if let Some(home) = std::env::var_os("HOME") {
                app_roots.push(PathBuf::from(home).join("Applications"));
            }
            let mut scanned_qgis: Vec<PathBuf> = Vec::new();
            for root in &app_roots {
                if let Ok(entries) = fs::read_dir(root) {
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                        if name.starts_with("qgis") && name.ends_with(".app") {
                            scanned_qgis.push(entry.path());
                        }
                    }
                }
            }
            // Prefer exact standard names, then descending versioned names (e.g., QGIS-final-4_2_3.app).
            scanned_qgis.sort_by(|a, b| b.cmp(a));
            let mut app_candidates = vec![
                PathBuf::from("/Applications/QGIS.app"),
                PathBuf::from("/Applications/QGIS-LTR.app"),
            ];
            for root in &app_roots {
                app_candidates.push(root.join("QGIS.app"));
                app_candidates.push(root.join("QGIS-LTR.app"));
            }
            for cand in scanned_qgis {
                if !app_candidates.iter().any(|existing| existing == &cand) {
                    app_candidates.push(cand);
                }
            }
            for app in app_candidates {
                if let Some((exe, gui)) = resolve_qgis_bundle_binary(&app) {
                    executable_path = Some(exe);
                    gui_path = gui;
                    source = "system_app".to_string();
                    break;
                }
            }
            if executable_path.is_none() {
                for bin in [
                    "/opt/homebrew/bin/qgis_process",
                    "/usr/local/bin/qgis_process",
                ] {
                    let p = Path::new(bin);
                    if let Some((exe, gui)) = resolve_qgis_bundle_binary(p) {
                        executable_path = Some(exe);
                        if gui_path.is_none() {
                            gui_path = gui;
                        }
                        source = "system_path".to_string();
                        break;
                    }
                }
            }
        }
        #[cfg(windows)]
        {
            let mut root_candidates: Vec<PathBuf> = Vec::new();
            if let Some(pf) = std::env::var_os("ProgramFiles") {
                if let Ok(entries) = fs::read_dir(Path::new(&pf)) {
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().to_ascii_uppercase();
                        if name.starts_with("QGIS") {
                            root_candidates.push(entry.path());
                        }
                    }
                }
            }
            if let Some(osgeo) = std::env::var_os("OSGEO4W_ROOT") {
                root_candidates.push(PathBuf::from(osgeo));
            }
            root_candidates.push(PathBuf::from(r"C:\OSGeo4W"));
            root_candidates.push(PathBuf::from(r"C:\OSGeo4W64"));
            for root in root_candidates {
                let bin_dir = root.join("bin");
                for name in [
                    "qgis_process.exe",
                    "qgis_process-qgis.bat",
                    "qgis_process.bat",
                ] {
                    let candidate = bin_dir.join(name);
                    if is_regular_executable(&candidate) {
                        executable_path = Some(candidate);
                        source = "system_app".to_string();
                        break;
                    }
                }
                for gui_name in ["qgis-bin.exe", "qgis-ltr-bin.exe"] {
                    let gui_candidate = bin_dir.join(gui_name);
                    if is_regular_executable(&gui_candidate) {
                        gui_path = Some(gui_candidate);
                        break;
                    }
                }
                if executable_path.is_some() {
                    break;
                }
            }
        }
    }

    if executable_path.is_none() {
        if let Some(found) = find_in_system_path(&["qgis_process"]) {
            executable_path = Some(found);
            source = "system_path".to_string();
        }
    }
    if gui_path.is_none() {
        if let Some(found) = find_in_system_path(&["qgis"]) {
            gui_path = Some(found);
        }
    }

    let installed = executable_path.is_some() || gui_path.is_some();
    let version = if probe_versions {
        executable_path
            .as_deref()
            .and_then(|exe| probe_cli_version(exe, "qgis", app_data_dir))
    } else {
        None
    };
    let detail = if let Some(err) = custom_err {
        format!("自定义路径无效: {err}")
    } else if installed {
        match (&executable_path, &gui_path) {
            (Some(cli), Some(gui)) => format!(
                "已检测到 QGIS（CLI: {}；桌面: {}），支持无头处理与 .qgs 工程交付",
                cli.display(),
                gui.display()
            ),
            (Some(cli), None) => format!("已检测到 qgis_process: {}", cli.display()),
            (None, Some(gui)) => format!("已检测到 QGIS 桌面程序: {}", gui.display()),
            (None, None) => unreachable!(),
        }
    } else {
        "未在本机检测到 QGIS；安装后即可直接打开 EOStage 交付工程或无头调用算法".to_string()
    };

    ExternalGisToolchainInfo {
        id: "qgis".to_string(),
        display_name: "QGIS".to_string(),
        installed,
        source,
        executable_path: executable_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string()),
        gui_path: gui_path.as_ref().map(|p| p.to_string_lossy().to_string()),
        version,
        mcp_capable: false,
        headless_capable: executable_path.is_some(),
        gui_capable: gui_path.is_some(),
        capabilities: vec![
            "version".to_string(),
            "list_algorithms".to_string(),
            "run_algorithm".to_string(),
            "project_delivery".to_string(),
            "gui_launch".to_string(),
        ],
        detail,
    }
}

fn discover_arcgis_pro(
    custom_path: Option<&str>,
    app_data_dir: &Path,
    probe_versions: bool,
) -> ExternalGisToolchainInfo {
    let mut executable_path: Option<PathBuf> = None;
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut gui_path: Option<PathBuf> = None;
    let mut source = if cfg!(windows) {
        "missing".to_string()
    } else {
        "unsupported_os".to_string()
    };
    let mut custom_err: Option<String> = None;

    if let Some(raw) = custom_path {
        match validate_custom_executable_path("arcgis_pro", raw) {
            Ok(exe) => {
                executable_path = Some(exe);
                source = "custom_path".to_string();
            }
            Err(err) => custom_err = Some(err),
        }
    }

    #[cfg(windows)]
    {
        let mut bases = Vec::new();
        if let Some(pf) = std::env::var_os("ProgramFiles") {
            bases.push(PathBuf::from(pf).join("ArcGIS/Pro"));
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            bases.push(PathBuf::from(local).join("Programs/ArcGIS/Pro"));
        }
        for base in bases {
            if executable_path.is_none() {
                for rel in [
                    "bin/Python/envs/arcgispro-py3/python.exe",
                    "bin/Python/Scripts/propy.bat",
                ] {
                    let cand = base.join(rel);
                    if is_regular_executable(&cand) {
                        executable_path = Some(cand);
                        source = "system_app".to_string();
                        break;
                    }
                }
            }
            if gui_path.is_none() {
                let exe = base.join("bin/ArcGISPro.exe");
                if is_regular_executable(&exe) {
                    gui_path = Some(exe);
                }
            }
        }
    }

    let installed = executable_path.is_some() || gui_path.is_some();
    let version = if probe_versions {
        executable_path
            .as_deref()
            .and_then(|exe| probe_cli_version(exe, "arcgis_pro", app_data_dir))
    } else {
        None
    };

    let detail = if let Some(err) = custom_err {
        format!("自定义路径无效: {err}")
    } else if installed {
        format!(
            "已检测到 ArcGIS Pro 运行环境 ({})",
            executable_path
                .as_ref()
                .or(gui_path.as_ref())
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default()
        )
    } else if !cfg!(windows) {
        "ArcGIS Pro 桌面与 arcpy 环境仅适用于 Windows；当前系统建议使用 Built-in GDAL/PROJ、QGIS、RustGIS 或 GeoLibre".to_string()
    } else {
        "未在 Windows 默认安装目录检测到 ArcGIS Pro；可通过自定义路径指定 arcgispro-py3 的 python.exe".to_string()
    };

    ExternalGisToolchainInfo {
        id: "arcgis_pro".to_string(),
        display_name: "ArcGIS Pro (arcpy)".to_string(),
        installed,
        source,
        executable_path: executable_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string()),
        gui_path: gui_path.as_ref().map(|p| p.to_string_lossy().to_string()),
        version,
        mcp_capable: false,
        headless_capable: executable_path.is_some(),
        gui_capable: gui_path.is_some(),
        capabilities: vec![
            "version".to_string(),
            "list_algorithms".to_string(),
            "run_algorithm".to_string(),
            "project_delivery".to_string(),
            "gui_launch".to_string(),
        ],
        detail,
    }
}

struct CliFirstToolchainSpec<'a> {
    id: &'a str,
    display_name: &'a str,
    component_subdirs: &'a [&'a str],
    bin_names: &'a [&'a str],
    macos_apps: &'a [&'a str],
    mcp_supported: bool,
}

fn discover_cli_first_toolchain(
    spec: CliFirstToolchainSpec<'_>,
    custom_path: Option<&str>,
    runtime_root: &Path,
    app_data_dir: &Path,
    probe_versions: bool,
) -> ExternalGisToolchainInfo {
    let CliFirstToolchainSpec {
        id,
        display_name,
        component_subdirs,
        bin_names,
        macos_apps,
        mcp_supported,
    } = spec;
    let mut executable_path: Option<PathBuf> = None;
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut gui_path: Option<PathBuf> = None;
    let mut source = "missing".to_string();
    let mut custom_err: Option<String> = None;

    if let Some(raw) = custom_path {
        match validate_custom_executable_path(id, raw) {
            Ok(exe) => {
                executable_path = Some(exe);
                source = "custom_path".to_string();
            }
            Err(err) => custom_err = Some(err),
        }
    }

    if executable_path.is_none() {
        for dir in component_candidate_dirs(runtime_root, app_data_dir, component_subdirs) {
            if let Some(bin) = crate::application::resolve_component_binary(&dir, bin_names) {
                if is_regular_executable(&bin) {
                    executable_path = Some(bin);
                    source = "bundled_component".to_string();
                    break;
                }
            }
        }
    }

    if executable_path.is_none() {
        if let Some(bin) = find_in_system_path(bin_names) {
            executable_path = Some(bin);
            source = "system_path".to_string();
        }
    }

    #[cfg(target_os = "macos")]
    {
        let mut app_candidates: Vec<PathBuf> = macos_apps.iter().map(PathBuf::from).collect();
        if let Some(home) = std::env::var_os("HOME") {
            let user_apps = PathBuf::from(home).join("Applications");
            for app in macos_apps {
                if let Some(file_name) = Path::new(app).file_name() {
                    app_candidates.push(user_apps.join(file_name));
                }
            }
        }
        for app_path in app_candidates {
            if is_safe_dir(&app_path) {
                gui_path = Some(app_path);
                break;
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = macos_apps;

    let installed = executable_path.is_some() || gui_path.is_some();
    let version = if probe_versions {
        executable_path
            .as_deref()
            .and_then(|exe| probe_cli_version(exe, id, app_data_dir))
    } else {
        None
    };

    let detail = if let Some(err) = custom_err {
        format!("自定义路径无效: {err}")
    } else if let Some(ref exe) = executable_path {
        if mcp_supported {
            format!(
                "已检测到 {} ({}, 支持 mcp --yes 与无头 CLI 调用)",
                display_name,
                exe.display()
            )
        } else if let Some(ref gui) = gui_path {
            format!(
                "已检测到 {} ({}, 支持无头 CLI；桌面端: {})",
                display_name,
                exe.display(),
                gui.display()
            )
        } else {
            format!(
                "已检测到 {} ({}, 支持无头 CLI 调用)",
                display_name,
                exe.display()
            )
        }
    } else if let Some(ref gui) = gui_path {
        format!("已检测到 {} 桌面程序 ({})", display_name, gui.display())
    } else {
        format!("未检测到 {display_name}；可通过共享运行组件下载或设置本地可执行文件绝对路径")
    };

    let mut capabilities = vec![
        "version".to_string(),
        "list_algorithms".to_string(),
        "run_algorithm".to_string(),
    ];
    if mcp_supported {
        capabilities.push("mcp_stdio".to_string());
    }
    if gui_path.is_some() {
        capabilities.push("gui_launch".to_string());
    }

    ExternalGisToolchainInfo {
        id: id.to_string(),
        display_name: display_name.to_string(),
        installed,
        source,
        executable_path: executable_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string()),
        gui_path: gui_path.as_ref().map(|p| p.to_string_lossy().to_string()),
        version,
        mcp_capable: mcp_supported,
        headless_capable: executable_path.is_some(),
        gui_capable: gui_path.is_some(),
        capabilities,
        detail,
    }
}

fn canonical_geo_pkg_name(raw: &str) -> Option<&'static str> {
    let lower = raw.trim().to_ascii_lowercase().replace('_', "-");
    let mapped = match lower.as_str() {
        "osgeo" | "libgdal" | "libgdal-core" | "gdal" => "gdal",
        "rasterio" => "rasterio",
        "geopandas" => "geopandas",
        "shapely" => "shapely",
        "pyproj" => "pyproj",
        "xarray" => "xarray",
        "h5py" => "h5py",
        "netcdf4" => "netcdf4",
        "mintpy" => "mintpy",
        "isce2" | "isce" => "isce2",
        "scipy" => "scipy",
        "numpy" => "numpy",
        "fiona" => "fiona",
        "rioxarray" => "rioxarray",
        "pygmtsar" => "pygmtsar",
        _ => return None,
    };
    if GEO_CONDA_PACKAGES.contains(&mapped) {
        Some(mapped)
    } else {
        None
    }
}

fn inspect_conda_geo_packages(env_prefix: &Path) -> Vec<String> {
    let mut found: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();

    // 1) Read `<env>/conda-meta/*.json` package names (`<pkg>-<ver>-<build>.json`).
    if let Ok(entries) = fs::read_dir(env_prefix.join("conda-meta")) {
        for entry in entries.flatten() {
            let fname = entry.file_name().to_string_lossy().to_string();
            if !fname.ends_with(".json") {
                continue;
            }
            if let Some(pkg) = fname.rsplitn(3, '-').nth(2) {
                if let Some(canon) = canonical_geo_pkg_name(pkg) {
                    found.insert(canon);
                }
            }
        }
    }

    // 2) Read site-packages directories (`lib/python*/site-packages` on POSIX, `Lib/site-packages` on Windows).
    let mut site_dirs = vec![env_prefix.join("Lib").join("site-packages")];
    if let Ok(lib_entries) = fs::read_dir(env_prefix.join("lib")) {
        for entry in lib_entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("python") {
                site_dirs.push(entry.path().join("site-packages"));
            }
        }
    }
    for site_dir in site_dirs {
        if let Ok(entries) = fs::read_dir(site_dir) {
            for entry in entries.flatten() {
                let raw = entry.file_name().to_string_lossy().to_string();
                let base = raw
                    .strip_suffix(".dist-info")
                    .or_else(|| raw.strip_suffix(".egg-info"))
                    .unwrap_or(&raw);
                let pkg = base.split('-').next().unwrap_or(base);
                if let Some(canon) = canonical_geo_pkg_name(pkg) {
                    found.insert(canon);
                }
            }
        }
    }

    found.into_iter().map(str::to_string).collect()
}

fn probe_conda_python_version(
    python_exe: &Path,
    env_prefix: &Path,
    app_data_dir: &Path,
) -> Option<String> {
    let scratch = create_isolated_scratch_dir(app_data_dir, "probe-conda").ok()?;
    let budget = SubprocessScratchBudget {
        max_scratch_bytes: 16 * 1024 * 1024,
        max_scratch_files: 32,
        timeout: Duration::from_secs(VERSION_PROBE_TIMEOUT_SECS),
        poll_interval: Duration::from_millis(25),
    };
    let mut cmd = Command::new(python_exe);
    apply_sanitized_subprocess_env(&mut cmd, Some(python_exe));
    cmd.env("CONDA_PREFIX", env_prefix);
    cmd.arg("-I")
        .arg("-c")
        .arg("import sys; print(sys.version.split()[0])");
    let outcome = budget.execute_command(&mut cmd, &scratch.0).ok()?;
    if outcome.status_code != Some(0) {
        return None;
    }
    String::from_utf8_lossy(&outcome.stdout)
        .lines()
        .map(str::trim)
        .find(|s| !s.is_empty())
        .map(redact_sensitive_text)
}

fn prepend_conda_env_path(cmd: &mut Command, env_prefix: &Path) {
    let mut new_paths: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    {
        new_paths.push(env_prefix.to_path_buf());
        new_paths.push(env_prefix.join("Library").join("bin"));
        new_paths.push(env_prefix.join("Scripts"));
    }
    #[cfg(not(windows))]
    {
        new_paths.push(env_prefix.join("bin"));
    }
    if let Some(existing) = std::env::var_os("PATH") {
        new_paths.extend(std::env::split_paths(&existing));
    }
    if let Ok(joined) = std::env::join_paths(new_paths) {
        cmd.env("PATH", joined);
    }
    cmd.env("CONDA_PREFIX", env_prefix);
}

fn discover_conda_environments(
    prefs: &ExternalGisPrefs,
    app_data_dir: &Path,
    probe_versions: bool,
) -> Vec<ExternalCondaEnvInfo> {
    let mut raw_candidates: Vec<(String, PathBuf, &'static str)> = Vec::new();

    // 1. User-registered custom Conda environments
    for (alias, raw_path) in &prefs.conda_custom_envs {
        raw_candidates.push((alias.clone(), PathBuf::from(raw_path), "custom_path"));
    }

    let home_dir = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);

    // 2. ~/.conda/environments.txt
    if let Some(ref home) = home_dir {
        let env_txt = home.join(".conda").join("environments.txt");
        if let Ok(meta) = fs::symlink_metadata(&env_txt) {
            if !meta.file_type().is_symlink() && meta.is_file() {
                if let Ok(contents) = fs::read_to_string(&env_txt) {
                    for line in contents.lines() {
                        let trimmed = line.trim();
                        if trimmed.is_empty() || trimmed.starts_with('#') {
                            continue;
                        }
                        let p = PathBuf::from(trimmed);
                        let name = p
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("conda")
                            .to_string();
                        raw_candidates.push((name, p, "environments_txt"));
                    }
                }
            }
        }
    }

    // 3. Standard Conda / Mamba / Miniforge / Pixi roots
    let mut standard_roots: Vec<PathBuf> = Vec::new();
    if let Some(ref home) = home_dir {
        for rel in ["miniforge3", "mambaforge", "miniconda3", "anaconda3"] {
            standard_roots.push(home.join(rel));
        }
        let pixi_envs = home.join(".pixi").join("envs");
        if let Ok(entries) = fs::read_dir(&pixi_envs) {
            for entry in entries.flatten() {
                let p = entry.path();
                if is_safe_dir(&p) {
                    let name = p
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("pixi")
                        .to_string();
                    raw_candidates.push((name, p, "pixi"));
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        for abs in [
            "/opt/homebrew/Caskroom/miniforge/base",
            "/opt/homebrew/Caskroom/miniconda/base",
            "/opt/anaconda3",
        ] {
            standard_roots.push(PathBuf::from(abs));
        }
    }
    #[cfg(windows)]
    {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let base = PathBuf::from(local);
            standard_roots.push(base.join("miniconda3"));
            standard_roots.push(base.join("miniforge3"));
            standard_roots.push(base.join("anaconda3"));
        }
    }

    for root in standard_roots {
        if !is_safe_dir(&root) {
            continue;
        }
        let envs_dir = root.join("envs");
        if let Ok(entries) = fs::read_dir(&envs_dir) {
            let mut sub_paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
            sub_paths.sort();
            for sub in sub_paths {
                if is_safe_dir(&sub) {
                    let name = sub
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("env")
                        .to_string();
                    raw_candidates.push((name, sub, "standard_root"));
                }
            }
        }
        raw_candidates.push(("base".to_string(), root, "standard_root"));
    }

    let mut seen_prefixes: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let mut used_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut discovered = Vec::new();

    for (raw_name, raw_path, source) in raw_candidates {
        if discovered.len() >= 24 {
            break;
        }
        let Ok((prefix, python_path)) = resolve_conda_env_python(&raw_path) else {
            continue;
        };
        let canonical_prefix = fs::canonicalize(&prefix).unwrap_or_else(|_| prefix.clone());
        if !seen_prefixes.insert(canonical_prefix) {
            continue;
        }

        let base_id = if source == "custom_path" {
            format!("custom:{raw_name}")
        } else {
            format!("conda:{raw_name}")
        };
        let mut final_id = base_id.clone();
        let mut suffix = 2usize;
        while !used_ids.insert(final_id.clone()) {
            final_id = format!("{base_id}-{suffix}");
            suffix += 1;
        }

        let geo_packages = inspect_conda_geo_packages(&prefix);
        let python_version = if probe_versions {
            probe_conda_python_version(&python_path, &prefix, app_data_dir)
        } else {
            None
        };
        let verified = !probe_versions || python_version.is_some();
        let pkg_note = if geo_packages.is_empty() {
            "未检测到地学扩展包（可执行基础 Python 检查）".to_string()
        } else {
            format!("地学科学包: {}", geo_packages.join(", "))
        };
        let detail = match (&python_version, &python_path) {
            (Some(ver), py) => format!("Python {ver} ({}) · {pkg_note}", py.display()),
            (None, py) => format!("已发现环境 ({}) · {pkg_note}", py.display()),
        };

        discovered.push(ExternalCondaEnvInfo {
            id: final_id,
            name: raw_name,
            source: source.to_string(),
            env_prefix: prefix.to_string_lossy().to_string(),
            python_path: python_path.to_string_lossy().to_string(),
            python_version,
            geo_packages,
            verified,
            detail,
        });
    }

    discovered
}

pub fn inspect_external_gis_toolchains(
    runtime_root: &Path,
    app_data_dir: &Path,
    probe_versions: bool,
) -> Result<ExternalGisInspectReport, String> {
    let prefs = load_external_gis_prefs(app_data_dir)?;
    let rustgis = discover_cli_first_toolchain(
        CliFirstToolchainSpec {
            id: "rustgis",
            display_name: "RustGIS",
            component_subdirs: &["rustgis-cli", "rustgis"],
            bin_names: &["rustgis-cli", "rustgis"],
            macos_apps: &["/Applications/RustGIS.app"],
            mcp_supported: true,
        },
        prefs.custom_paths.get("rustgis").map(String::as_str),
        runtime_root,
        app_data_dir,
        probe_versions,
    );
    let geolibre = discover_cli_first_toolchain(
        CliFirstToolchainSpec {
            id: "geolibre",
            display_name: "GeoLibre",
            component_subdirs: &["geolibre", "geolibre-cli"],
            bin_names: &["geolibre", "geolibre-cli"],
            macos_apps: &[
                "/Applications/GeoLibre Desktop.app",
                "/Applications/GeoLibre.app",
            ],
            mcp_supported: false,
        },
        prefs.custom_paths.get("geolibre").map(String::as_str),
        runtime_root,
        app_data_dir,
        probe_versions,
    );
    let qgis = discover_qgis(
        prefs.custom_paths.get("qgis").map(String::as_str),
        app_data_dir,
        probe_versions,
    );
    let arcgis_pro = discover_arcgis_pro(
        prefs.custom_paths.get("arcgis_pro").map(String::as_str),
        app_data_dir,
        probe_versions,
    );
    let conda_environments = discover_conda_environments(&prefs, app_data_dir, probe_versions);
    let log_dehydrator = crate::log_dehydrator::inspect_log_dehydrator(
        prefs.rtk_custom_path.as_deref(),
        prefs.dehydrator_mode.as_deref(),
        app_data_dir,
        probe_versions,
    );
    Ok(ExternalGisInspectReport {
        ok: true,
        platform: std::env::consts::OS.to_string(),
        prefs,
        toolchains: vec![rustgis, geolibre, qgis, arcgis_pro],
        conda_environments,
        log_dehydrator,
    })
}

fn validate_algorithm_symbol(toolchain: &str, raw: Option<&str>) -> Result<String, String> {
    let trimmed = raw.map(str::trim).unwrap_or("");
    if trimmed.is_empty() || trimmed.len() > 128 {
        return Err("算法名称不能为空且长度不能超过 128 个字符".to_string());
    }
    if toolchain == "arcgis_pro" {
        let valid_parts = !trimmed.starts_with('.')
            && !trimmed.ends_with('.')
            && trimmed.split('.').count() <= 3
            && trimmed.split('.').all(|seg| {
                !seg.is_empty()
                    && !seg.starts_with('_')
                    && seg
                        .bytes()
                        .next()
                        .map(|b| b.is_ascii_alphabetic())
                        .unwrap_or(false)
                    && seg.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            });
        if !valid_parts {
            return Err(format!("ArcGIS Pro 算法标识符不合法: {trimmed}"));
        }
        return Ok(trimmed.to_string());
    }
    if trimmed.contains("..")
        || !trimmed
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-' | b'/'))
    {
        return Err(format!("算法标识符包含不允许的字符: {trimmed}"));
    }
    Ok(trimmed.to_string())
}

fn validate_param_entries(
    params: &BTreeMap<String, Value>,
    allow_raw_cli_render: bool,
) -> Result<Vec<(String, String)>, String> {
    let mut rendered = Vec::with_capacity(params.len());
    for (key, value) in params {
        let trimmed_key = key.trim();
        if trimmed_key.is_empty()
            || trimmed_key.len() > 64
            || !trimmed_key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
        {
            return Err(format!("参数名不合法: '{key}'"));
        }
        if allow_raw_cli_render {
            let as_str = match value {
                Value::String(s) => {
                    if s.contains('\0') || s.contains('\n') || s.contains('\r') {
                        return Err(format!("参数 '{trimmed_key}' 包含非法控制字符"));
                    }
                    s.clone()
                }
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                Value::Array(items) => {
                    let mut parts = Vec::with_capacity(items.len());
                    for item in items {
                        match item {
                            Value::String(s) => {
                                if s.contains('\0') || s.contains('\n') || s.contains('\r') {
                                    return Err(format!("数组参数 '{trimmed_key}' 包含控制字符"));
                                }
                                parts.push(s.clone());
                            }
                            Value::Number(n) => parts.push(n.to_string()),
                            Value::Bool(b) => parts.push(b.to_string()),
                            _ => {
                                return Err(format!(
                                    "数组参数 '{trimmed_key}' 仅支持字符串、数字或布尔值"
                                ));
                            }
                        }
                    }
                    parts.join(",")
                }
                _ => {
                    return Err(format!(
                        "参数 '{trimmed_key}' 仅支持标量或简单数组，不支持嵌套对象"
                    ));
                }
            };
            rendered.push((trimmed_key.to_string(), as_str));
        } else {
            validate_json_scalar_or_array(trimmed_key, value)?;
        }
    }
    Ok(rendered)
}

fn validate_json_scalar_or_array(key: &str, value: &Value) -> Result<(), String> {
    match value {
        Value::String(s) => {
            if s.contains('\0') {
                return Err(format!("参数 '{key}' 包含空字符"));
            }
            Ok(())
        }
        Value::Number(_) | Value::Bool(_) | Value::Null => Ok(()),
        Value::Array(items) => {
            for item in items {
                match item {
                    Value::String(s) if !s.contains('\0') => {}
                    Value::Number(_) | Value::Bool(_) => {}
                    _ => return Err(format!("数组参数 '{key}' 仅支持标量值")),
                }
            }
            Ok(())
        }
        Value::Object(_) => Err(format!("参数 '{key}' 不支持嵌套对象")),
    }
}

fn verify_expected_output_exists(output_path: Option<&str>) -> Result<Option<String>, String> {
    let Some(raw) = output_path.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let path = Path::new(raw);
    let meta = fs::symlink_metadata(path)
        .map_err(|err| format!("外部 GIS 未能生成预期输出文件 ({raw}): {err}"))?;
    if meta.file_type().is_symlink() {
        return Err(format!("拒绝将符号链接视为有效输出: {raw}"));
    }
    if meta.is_file() {
        if meta.len() == 0 {
            return Err(format!("外部 GIS 生成的输出文件为空 (0 字节): {raw}"));
        }
        return Ok(Some(raw.to_string()));
    }
    if meta.is_dir() {
        let has_nonempty = fs::read_dir(path)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .any(|e| {
                e.metadata()
                    .map(|m| m.is_file() && m.len() > 0)
                    .unwrap_or(false)
            });
        if !has_nonempty {
            return Err(format!("外部 GIS 输出目录为空: {raw}"));
        }
        return Ok(Some(raw.to_string()));
    }
    Err(format!("外部 GIS 输出路径既非文件也非目录: {raw}"))
}

fn build_rustgis_run_command(
    cmd: &mut Command,
    alg: &str,
    request: &ExternalGisExecuteRequest,
) -> Result<(), String> {
    let rendered = validate_param_entries(&request.params, false)?;
    let _ = rendered;
    let first_input_from_params = ["input", "data"]
        .iter()
        .find_map(|key| request.params.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let first_input_path = request
        .input_paths
        .iter()
        .map(|s| s.trim())
        .find(|s| !s.is_empty())
        .map(str::to_string)
        .or(first_input_from_params);

    if alg == "info" {
        let input =
            first_input_path.ok_or_else(|| "RustGIS 'info' 需要指定输入数据路径".to_string())?;
        cmd.arg("info").arg(input);
        return Ok(());
    }

    if alg == "render" {
        let input = first_input_path
            .ok_or_else(|| "RustGIS 'render' 需要指定输入数据或工程路径".to_string())?;
        let out = request
            .output_path
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "RustGIS 'render' 需要指定 .png 输出路径".to_string())?;
        cmd.arg("render").arg(input).arg(out);
        for flag in ["basemap", "tiles", "width", "height", "dpi", "worldview"] {
            if let Some(val) = request.params.get(flag) {
                let s = match val {
                    Value::String(text) => text.trim().to_string(),
                    Value::Number(num) => num.to_string(),
                    Value::Bool(b) => b.to_string(),
                    _ => continue,
                };
                if !s.is_empty() {
                    cmd.arg(format!("--{flag}")).arg(s);
                }
            }
        }
        return Ok(());
    }

    cmd.arg("run").arg("--no-plugins");
    let mut loaded_data_paths: Vec<PathBuf> = Vec::new();
    let mut first_data_stem: Option<String> = None;
    for raw in &request.input_paths {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let path = Path::new(trimmed);
        let is_rgis = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("rgis"))
            .unwrap_or(false);
        if is_rgis {
            cmd.arg("--project").arg(trimmed);
        } else {
            cmd.arg("--data").arg(trimmed);
            loaded_data_paths.push(path.to_path_buf());
            if first_data_stem.is_none() {
                first_data_stem = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(str::to_string);
            }
        }
    }

    let mut params_obj = Map::new();
    for (k, v) in &request.params {
        params_obj.insert(k.trim().to_string(), v.clone());
    }
    if let Some(Value::String(input_val)) = params_obj.get("input").cloned() {
        let input_path = Path::new(input_val.trim());
        if input_path.is_file() {
            if !loaded_data_paths.iter().any(|p| p == input_path) {
                cmd.arg("--data").arg(input_path);
            }
            if let Some(stem) = input_path.file_stem().and_then(|s| s.to_str()) {
                params_obj.insert("input".to_string(), Value::String(stem.to_string()));
            }
        }
    } else if let Some(stem) = first_data_stem {
        params_obj.insert("input".to_string(), Value::String(stem));
    }

    if alg.contains('.') {
        let cmd_payload = serde_json::to_string(&Value::Object(params_obj))
            .map_err(|err| format!("序列化 RustGIS 命令参数失败: {err}"))?;
        cmd.arg("--cmd").arg(format!("{alg}={cmd_payload}"));
    } else {
        let gp_tool = alg.split(':').next_back().unwrap_or(alg);
        let gp_payload = serde_json::to_string(&json!({
            "tool": gp_tool,
            "params": params_obj,
        }))
        .map_err(|err| format!("序列化 RustGIS GP 任务失败: {err}"))?;
        cmd.arg("--cmd").arg(format!("gp.run={gp_payload}"));
    }

    if let Some(out) = request
        .output_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let ext = Path::new(out)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        match ext.as_deref() {
            Some("rgis") => {
                cmd.arg("--save").arg(out);
            }
            Some("png") => {
                let payload = json!({ "path": out }).to_string();
                cmd.arg("--cmd").arg(format!("share.exportMap={payload}"));
            }
            Some("tif") | Some("tiff") => {
                let payload = json!({ "path": out }).to_string();
                cmd.arg("--cmd")
                    .arg(format!("share.exportRaster={payload}"));
            }
            _ => {
                let payload = json!({ "path": out }).to_string();
                cmd.arg("--cmd").arg(format!("share.exportLayer={payload}"));
            }
        }
    }
    Ok(())
}

fn finalize_dehydrated_outputs(
    raw_stdout: &str,
    raw_stderr: &str,
    stdout_limit: usize,
    dehydrator: &crate::log_dehydrator::LogDehydratorInfo,
    app_data_dir: &Path,
) -> (String, String, crate::log_dehydrator::DehydrationSummary) {
    let (dehydrated_out, out_sum) = crate::log_dehydrator::apply_output_dehydration(
        raw_stdout,
        stdout_limit,
        dehydrator,
        app_data_dir,
    );
    let (dehydrated_err, err_sum) = crate::log_dehydrator::apply_output_dehydration(
        raw_stderr,
        MAX_OUTPUT_BYTES,
        dehydrator,
        app_data_dir,
    );
    let stdout = redact_sensitive_text_with_limit(&dehydrated_out, stdout_limit);
    let stderr = redact_sensitive_text(&dehydrated_err);
    let combined = crate::log_dehydrator::DehydrationSummary {
        engine: out_sum.engine,
        raw_bytes: out_sum.raw_bytes.saturating_add(err_sum.raw_bytes),
        kept_bytes: stdout.len().saturating_add(stderr.len()),
        raw_lines: out_sum.raw_lines.saturating_add(err_sum.raw_lines),
        kept_lines: stdout
            .lines()
            .count()
            .saturating_add(stderr.lines().count()),
        collapsed_progress_lines: out_sum
            .collapsed_progress_lines
            .saturating_add(err_sum.collapsed_progress_lines),
        collapsed_duplicate_warnings: out_sum
            .collapsed_duplicate_warnings
            .saturating_add(err_sum.collapsed_duplicate_warnings),
        head_tail_elided_lines: out_sum
            .head_tail_elided_lines
            .saturating_add(err_sum.head_tail_elided_lines),
    };
    (stdout, stderr, combined)
}

fn select_conda_environment<'a>(
    report: &'a ExternalGisInspectReport,
    requested: Option<&str>,
) -> Result<&'a ExternalCondaEnvInfo, String> {
    if report.conda_environments.is_empty() {
        return Err(
            "未在本机检测到可用的 Conda/Python 科研环境；请在「运行组件」页添加环境路径"
                .to_string(),
        );
    }
    let selector = requested
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            report
                .prefs
                .preferred_conda_env
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
        });
    if let Some(target) = selector {
        return report
            .conda_environments
            .iter()
            .find(|env| env.id == target || env.name == target)
            .ok_or_else(|| format!("未找到指定的 Conda 环境 '{target}'"));
    }
    Ok(report
        .conda_environments
        .iter()
        .find(|env| !env.geo_packages.is_empty())
        .unwrap_or(&report.conda_environments[0]))
}

fn execute_conda_action(
    app_data_dir: &Path,
    action: &str,
    request: &ExternalGisExecuteRequest,
    report: &ExternalGisInspectReport,
) -> Result<ExternalGisExecuteResult, String> {
    if action == "deliver_project" {
        return Err(
            "Conda 科学环境不支持 'deliver_project'，请改用 QGIS 或 ArcGIS Pro".to_string(),
        );
    }
    let env_info = select_conda_environment(report, request.conda_env.as_deref())?;
    let exe = Path::new(&env_info.python_path);
    let prefix = Path::new(&env_info.env_prefix);

    let mut job = Map::new();
    job.insert("action".to_string(), Value::String(action.to_string()));
    if action == "run_algorithm" {
        let alg = request.algorithm.as_deref().map(str::trim).unwrap_or("");
        if !matches!(
            alg,
            "package_probe"
                | "raster_inspect"
                | "raster_band_stats"
                | "vector_inspect"
                | "hdf5_inspect"
        ) {
            return Err(format!(
                "不支持的 Conda 受控算子 '{alg}'（仅允许 package_probe/raster_inspect/raster_band_stats/vector_inspect/hdf5_inspect）"
            ));
        }
        let _ = validate_param_entries(&request.params, false)?;
        let mut params_obj = Map::new();
        for (k, v) in &request.params {
            params_obj.insert(k.trim().to_string(), v.clone());
        }
        job.insert("algorithm".to_string(), Value::String(alg.to_string()));
        job.insert("params".to_string(), Value::Object(params_obj));
        job.insert(
            "input_paths".to_string(),
            Value::Array(
                request
                    .input_paths
                    .iter()
                    .map(|p| Value::String(p.trim().to_string()))
                    .filter(|v| !v.as_str().unwrap_or("").is_empty())
                    .collect(),
            ),
        );
        if let Some(out) = request
            .output_path
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            job.insert("output_path".to_string(), Value::String(out.to_string()));
        }
    }

    let scratch = create_isolated_scratch_dir(app_data_dir, "conda")?;
    let timeout_secs = request
        .timeout_seconds
        .unwrap_or(DEFAULT_EXECUTE_TIMEOUT_SECS)
        .clamp(1, 3600);
    let budget = SubprocessScratchBudget {
        max_scratch_bytes: 512 * 1024 * 1024,
        max_scratch_files: 1024,
        timeout: Duration::from_secs(timeout_secs),
        poll_interval: Duration::from_millis(25),
    };
    let job_path = scratch.0.join("conda_job.json");
    let serialized = serde_json::to_string_pretty(&Value::Object(job))
        .map_err(|err| format!("序列化 Conda 任务失败: {err}"))?;
    fs::write(&job_path, serialized.as_bytes())
        .map_err(|err| format!("写入 conda_job.json 失败: {err}"))?;

    let mut cmd = Command::new(exe);
    apply_sanitized_subprocess_env(&mut cmd, Some(exe));
    prepend_conda_env_path(&mut cmd, prefix);
    cmd.arg("-I")
        .arg("-c")
        .arg(CONDA_ISOLATED_RUNNER)
        .arg(&job_path);

    let outcome = budget
        .execute_command(&mut cmd, &scratch.0)
        .map_err(|err| format!("执行 Conda Python 进程失败: {err}"))?;
    let raw_stdout = String::from_utf8_lossy(&outcome.stdout);
    let raw_stderr = String::from_utf8_lossy(&outcome.stderr);
    let (stdout, stderr, dehydration) = finalize_dehydrated_outputs(
        &raw_stdout,
        &raw_stderr,
        if action == "list_algorithms" {
            MAX_LIST_ALGORITHMS_OUTPUT_BYTES
        } else {
            MAX_OUTPUT_BYTES
        },
        &report.log_dehydrator,
        app_data_dir,
    );
    if outcome.status_code != Some(0) {
        return Err(format!(
            "Conda ({}) '{}' 执行失败 (exit {:?}): {}",
            env_info.name,
            action,
            outcome.status_code,
            if stderr.is_empty() { &stdout } else { &stderr }
        ));
    }

    let verified_out = if action == "run_algorithm" {
        verify_expected_output_exists(request.output_path.as_deref())?
    } else {
        None
    };

    Ok(ExternalGisExecuteResult {
        ok: true,
        toolchain: "conda".to_string(),
        action: action.to_string(),
        exit_code: outcome.status_code,
        stdout,
        stderr,
        output_path: verified_out.clone(),
        project_path: None,
        catalog_path: None,
        gui_launched: false,
        detail: format!("Conda ({}) 命令执行完成", env_info.name),
        artifacts: verified_out.into_iter().collect(),
        dehydration: Some(dehydration),
    })
}

pub fn execute_external_gis_action(
    runtime_root: &Path,
    app_data_dir: &Path,
    request: &ExternalGisExecuteRequest,
) -> Result<ExternalGisExecuteResult, String> {
    let toolchain = validate_execution_toolchain_id(&request.toolchain)?;
    let action = validate_action_name(&request.action)?;
    if action == "deliver_project" && toolchain != "conda" {
        return execute_deliver_project(runtime_root, app_data_dir, &toolchain, request);
    }

    let report = inspect_external_gis_toolchains(runtime_root, app_data_dir, false)?;
    if toolchain == "conda" {
        return execute_conda_action(app_data_dir, &action, request, &report);
    }
    let log_dehydrator = report.log_dehydrator.clone();
    let info = report
        .toolchains
        .into_iter()
        .find(|t| t.id == toolchain)
        .ok_or_else(|| format!("未找到工具链元信息: {toolchain}"))?;
    let exe_str = info.executable_path.as_deref().ok_or_else(|| {
        format!(
            "{} 当前未就绪，无法无头执行 '{action}'：{}",
            info.display_name, info.detail
        )
    })?;
    let exe = Path::new(exe_str);
    if toolchain == "rustgis" && action == "version" {
        let ver = probe_cli_version(exe, "rustgis", app_data_dir)
            .ok_or_else(|| format!("{} 'version' 探测失败", info.display_name))?;
        let (stdout, _err, dehydration) =
            finalize_dehydrated_outputs(&ver, "", MAX_OUTPUT_BYTES, &log_dehydrator, app_data_dir);
        return Ok(ExternalGisExecuteResult {
            ok: true,
            toolchain,
            action,
            exit_code: Some(0),
            stdout,
            stderr: String::new(),
            output_path: None,
            project_path: None,
            catalog_path: None,
            gui_launched: false,
            detail: format!("{} 版本探测完成", info.display_name),
            artifacts: Vec::new(),
            dehydration: Some(dehydration),
        });
    }
    let scratch = create_isolated_scratch_dir(app_data_dir, &toolchain)?;
    let timeout_secs = request
        .timeout_seconds
        .unwrap_or(DEFAULT_EXECUTE_TIMEOUT_SECS)
        .clamp(1, 3600);
    let budget = SubprocessScratchBudget {
        max_scratch_bytes: 512 * 1024 * 1024,
        max_scratch_files: 1024,
        timeout: Duration::from_secs(timeout_secs),
        poll_interval: Duration::from_millis(25),
    };

    let mut cmd = Command::new(exe);
    apply_sanitized_subprocess_env(&mut cmd, Some(exe));

    if toolchain == "arcgis_pro" {
        let mut job = Map::new();
        job.insert("action".to_string(), Value::String(action.clone()));
        if action == "run_algorithm" {
            let alg = validate_algorithm_symbol(&toolchain, request.algorithm.as_deref())?;
            let _ = validate_param_entries(&request.params, false)?;
            let mut params_obj = Map::new();
            for (k, v) in &request.params {
                params_obj.insert(k.trim().to_string(), v.clone());
            }
            if let Some(out) = request
                .output_path
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                let has_explicit_out = params_obj.keys().any(|k| {
                    matches!(
                        k.to_ascii_lowercase().as_str(),
                        "out_feature_class"
                            | "out_raster"
                            | "out_rasterdataset"
                            | "out_raster_dataset"
                            | "out_dataset"
                            | "out_table"
                            | "out_polygon_features"
                            | "out_point_features"
                            | "out_polyline_features"
                            | "out_file"
                            | "out_folder"
                            | "out_workspace"
                            | "out_path"
                            | "out_dir"
                            | "out_location"
                            | "output_location"
                            | "output"
                    )
                });
                if !has_explicit_out {
                    job.insert(
                        "controlled_output_path".to_string(),
                        Value::String(out.to_string()),
                    );
                    job.insert(
                        "default_output_param".to_string(),
                        Value::String(arcpy_default_output_param_name(&alg).to_string()),
                    );
                }
            }
            job.insert("algorithm".to_string(), Value::String(alg));
            job.insert("params".to_string(), Value::Object(params_obj));
        }
        let job_path = scratch.0.join("arcpy_job.json");
        let serialized = serde_json::to_string_pretty(&Value::Object(job))
            .map_err(|err| format!("序列化 ArcGIS Pro 任务失败: {err}"))?;
        fs::write(&job_path, serialized.as_bytes())
            .map_err(|err| format!("写入 arcpy_job.json 失败: {err}"))?;
        cmd.arg("-I")
            .arg("-c")
            .arg(ARCPY_ISOLATED_RUNNER)
            .arg(&job_path);
    } else if toolchain == "rustgis" {
        match action.as_str() {
            "list_algorithms" => {
                cmd.arg("tools").arg("--no-plugins");
            }
            "run_algorithm" => {
                let alg = validate_algorithm_symbol(&toolchain, request.algorithm.as_deref())?;
                build_rustgis_run_command(&mut cmd, &alg, request)?;
            }
            _ => unreachable!(),
        }
    } else if toolchain == "geolibre" {
        match action.as_str() {
            "version" => {
                cmd.arg("--version");
            }
            "list_algorithms" => {
                cmd.arg("list");
            }
            "run_algorithm" => {
                let alg = validate_algorithm_symbol(&toolchain, request.algorithm.as_deref())?;
                let rendered = validate_param_entries(&request.params, true)?;
                cmd.arg(&alg);
                for (k, v) in rendered {
                    cmd.arg(format!("--{k}={v}"));
                }
                if let Some(out) = request
                    .output_path
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    let has_explicit_out = request
                        .params
                        .keys()
                        .any(|k| k.eq_ignore_ascii_case("output"));
                    if !has_explicit_out {
                        cmd.arg(format!("--output={out}"));
                    }
                }
            }
            _ => unreachable!(),
        }
    } else {
        match action.as_str() {
            "version" => {
                cmd.arg("--version");
            }
            "list_algorithms" => {
                cmd.arg("--json").arg("--skip-loading-plugins").arg("list");
            }
            "run_algorithm" => {
                let alg = validate_algorithm_symbol(&toolchain, request.algorithm.as_deref())?;
                let rendered = validate_param_entries(&request.params, true)?;
                cmd.arg("--json")
                    .arg("--skip-loading-plugins")
                    .arg("run")
                    .arg(&alg)
                    .arg("--");
                for (k, v) in rendered {
                    cmd.arg(format!("{k}={v}"));
                }
                if let Some(out) = request
                    .output_path
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    let has_explicit_out = request
                        .params
                        .keys()
                        .any(|k| k.eq_ignore_ascii_case("output"));
                    if !has_explicit_out {
                        cmd.arg(format!("OUTPUT={out}"));
                    }
                }
            }
            _ => unreachable!(),
        }
    }

    let outcome = budget
        .execute_command(&mut cmd, &scratch.0)
        .map_err(|err| format!("执行外部 GIS 进程失败: {err}"))?;
    let raw_stdout = String::from_utf8_lossy(&outcome.stdout);
    let raw_stderr = String::from_utf8_lossy(&outcome.stderr);
    let prepared_stdout = if action == "list_algorithms" && toolchain == "qgis" {
        compact_qgis_algorithms_json(&raw_stdout)
    } else {
        None
    };
    let stdout_source = prepared_stdout.as_deref().unwrap_or(&raw_stdout);
    let stdout_limit = if action == "list_algorithms" {
        MAX_LIST_ALGORITHMS_OUTPUT_BYTES
    } else {
        MAX_OUTPUT_BYTES
    };
    let (stdout, stderr, dehydration) = finalize_dehydrated_outputs(
        stdout_source,
        &raw_stderr,
        stdout_limit,
        &log_dehydrator,
        app_data_dir,
    );
    if outcome.status_code != Some(0) {
        return Err(format!(
            "{} '{}' 执行失败 (exit {:?}): {}",
            info.display_name,
            action,
            outcome.status_code,
            if stderr.is_empty() { &stdout } else { &stderr }
        ));
    }

    let verified_out = if action == "run_algorithm" {
        verify_expected_output_exists(request.output_path.as_deref())?
    } else {
        None
    };

    Ok(ExternalGisExecuteResult {
        ok: true,
        toolchain,
        action,
        exit_code: outcome.status_code,
        stdout,
        stderr,
        output_path: verified_out.clone(),
        project_path: None,
        catalog_path: None,
        gui_launched: false,
        detail: format!("{} 命令执行完成", info.display_name),
        artifacts: verified_out.into_iter().collect(),
        dehydration: Some(dehydration),
    })
}

fn is_supported_raster_ext(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "tif" | "tiff" | "vrt" | "img" | "dat" | "bin" | "slc"
    )
}

fn is_supported_vector_ext(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "geojson" | "shp" | "kml" | "kmz" | "gpkg"
    )
}

fn is_geojson_json_file(path: &Path, ext: &str) -> bool {
    if !ext.eq_ignore_ascii_case("json") {
        return false;
    }
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    if !meta.is_file() || meta.len() == 0 {
        return false;
    }
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let max_probe = 64 * 1024usize;
    let mut buf = Vec::with_capacity((meta.len() as usize).min(max_probe));
    if std::io::Read::by_ref(&mut file)
        .take(max_probe as u64)
        .read_to_end(&mut buf)
        .is_err()
    {
        return false;
    }
    if (meta.len() as usize) <= max_probe {
        if let Ok(Value::Object(obj)) = serde_json::from_slice::<Value>(&buf) {
            return matches!(
                obj.get("type").and_then(Value::as_str),
                Some("FeatureCollection" | "Feature" | "GeometryCollection" | "Topology")
            );
        }
        return false;
    }
    let head = String::from_utf8_lossy(&buf[..buf.len().min(4096)]);
    head.contains("\"FeatureCollection\"")
        || head.contains("\"GeometryCollection\"")
        || head.contains("\"Topology\"")
}

fn collect_delivery_inputs(raw_inputs: &[String]) -> Result<(Vec<PathBuf>, Vec<PathBuf>), String> {
    let mut rasters = Vec::new();
    let mut vectors = Vec::new();

    fn visit(
        path: &Path,
        depth: usize,
        rasters: &mut Vec<PathBuf>,
        vectors: &mut Vec<PathBuf>,
    ) -> Result<(), String> {
        if depth > 3 {
            return Ok(());
        }
        let meta = fs::symlink_metadata(path)
            .map_err(|err| format!("交付源路径不可读取 ({}): {err}", path.display()))?;
        if meta.file_type().is_symlink() {
            return Ok(());
        }
        if meta.is_file() {
            let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
                return Ok(());
            };
            let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if file_name.starts_with('.') || file_name == "eostage_catalog.geojson" {
                return Ok(());
            }
            if is_supported_raster_ext(ext) {
                rasters.push(path.to_path_buf());
            } else if is_supported_vector_ext(ext) || is_geojson_json_file(path, ext) {
                vectors.push(path.to_path_buf());
            }
            return Ok(());
        }
        if meta.is_dir() {
            let entries = fs::read_dir(path)
                .map_err(|err| format!("读取目录失败 ({}): {err}", path.display()))?;
            let mut paths: Vec<PathBuf> = entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| {
                    !p.file_name()
                        .and_then(|s| s.to_str())
                        .map(|n| n.starts_with('.'))
                        .unwrap_or(true)
                })
                .collect();
            paths.sort();
            for p in paths {
                visit(&p, depth + 1, rasters, vectors)?;
            }
        }
        Ok(())
    }

    for raw in raw_inputs {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        visit(Path::new(trimmed), 0, &mut rasters, &mut vectors)?;
    }

    if rasters.is_empty() && vectors.is_empty() {
        return Err("未找到可用于 GIS 工程交付的有效栅格或矢量文件".to_string());
    }
    Ok((rasters, vectors))
}

fn xml_escape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

fn is_plausible_wgs84_bounds(b: [f64; 4]) -> bool {
    b[0].is_finite()
        && b[1].is_finite()
        && b[2].is_finite()
        && b[3].is_finite()
        && b[0] >= -180.1
        && b[2] <= 180.1
        && b[1] >= -90.1
        && b[3] <= 90.1
        && b[0] < b[2]
        && b[1] < b[3]
}

fn write_atomic_utf8(path: &Path, content: &str) -> Result<(), String> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return Err(format!("拒绝覆盖符号链接文件: {}", path.display()));
        }
    }
    let parent = path
        .parent()
        .ok_or_else(|| format!("缺少父目录: {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|err| format!("创建交付目录失败 ({}): {err}", parent.display()))?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = parent.join(format!(
        ".{}.tmp.{}.{nanos}",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("delivery"),
        std::process::id()
    ));
    let mut file = File::create(&tmp)
        .map_err(|err| format!("创建临时交付文件失败 ({}): {err}", tmp.display()))?;
    file.write_all(content.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|err| {
            let _ = fs::remove_file(&tmp);
            format!("写入交付文件失败: {err}")
        })?;
    fs::rename(&tmp, path).map_err(|err| {
        let _ = fs::remove_file(&tmp);
        format!("原子落盘交付文件失败 ({}): {err}", path.display())
    })
}

struct DeliveredRasterMeta {
    id: String,
    name: String,
    path: String,
    driver: String,
    crs: String,
    bounds: Option<[f64; 4]>,
    width: i64,
    height: i64,
    count: i64,
}

fn render_qgis_srs_inner(crs: &str) -> String {
    let trimmed = crs.trim();
    let upper = trimmed.to_ascii_uppercase();
    if upper.starts_with("EPSG:") || upper.starts_with("OGC:") || upper.starts_with("ESRI:") {
        format!("<authid>{}</authid>", xml_escape(trimmed))
    } else {
        format!("<wkt>{}</wkt>", xml_escape(trimmed))
    }
}

fn execute_deliver_project(
    runtime_root: &Path,
    app_data_dir: &Path,
    toolchain: &str,
    request: &ExternalGisExecuteRequest,
) -> Result<ExternalGisExecuteResult, String> {
    let raw_out = request
        .output_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "deliver_project 必须提供 output_path（目录或 .qgs 路径）".to_string())?;
    let raw_out_path = PathBuf::from(raw_out);
    let (delivery_dir, project_path) = if raw_out_path
        .extension()
        .and_then(|s| s.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("qgs"))
        .unwrap_or(false)
    {
        let parent = raw_out_path
            .parent()
            .ok_or_else(|| format!("无效的工程输出路径: {raw_out}"))?
            .to_path_buf();
        (parent, raw_out_path)
    } else {
        (
            raw_out_path.clone(),
            raw_out_path.join("eostage_delivery.qgs"),
        )
    };
    let catalog_path = delivery_dir.join("eostage_catalog.geojson");

    let (rasters, vectors) = collect_delivery_inputs(&request.input_paths)?;
    let engine = crate::geodata_tools::discover_native_gis_engine();

    let mut features = Vec::new();
    let mut raster_layers = Vec::new();
    let mut wgs_union: Option<[f64; 4]> = None;

    for (idx, raster_path) in rasters.iter().enumerate() {
        let path_str = raster_path.to_string_lossy().to_string();
        let name = raster_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("raster")
            .to_string();
        let info = engine
            .inspect_raster_source(&path_str)
            .map_err(|err| format!("检查交付栅格元数据失败 ({path_str}): {err}"))?;
        let meta = &info["metadata"];
        let driver = meta["driver"].as_str().unwrap_or("GTiff").to_string();
        let crs = meta["crs"].as_str().unwrap_or("EPSG:4326").to_string();
        let width = meta["width"].as_i64().unwrap_or(0);
        let height = meta["height"].as_i64().unwrap_or(0);
        let count = meta["count"].as_i64().unwrap_or(1);
        let bounds = meta["bounds"].as_array().and_then(|arr| {
            if arr.len() == 4 {
                Some([
                    arr[0].as_f64()?,
                    arr[1].as_f64()?,
                    arr[2].as_f64()?,
                    arr[3].as_f64()?,
                ])
            } else {
                None
            }
        });

        let geometry = if let Some(b) = bounds.filter(|b| is_plausible_wgs84_bounds(*b)) {
            wgs_union = Some(match wgs_union {
                Some(u) => [
                    u[0].min(b[0]),
                    u[1].min(b[1]),
                    u[2].max(b[2]),
                    u[3].max(b[3]),
                ],
                None => b,
            });
            json!({
                "type": "Polygon",
                "coordinates": [[
                    [b[0], b[1]],
                    [b[2], b[1]],
                    [b[2], b[3]],
                    [b[0], b[3]],
                    [b[0], b[1]],
                ]]
            })
        } else {
            Value::Null
        };

        features.push(json!({
            "type": "Feature",
            "properties": {
                "kind": "raster",
                "name": name,
                "path": path_str,
                "driver": driver,
                "crs": crs,
                "width": width,
                "height": height,
                "bands": count,
                "bounds": bounds,
            },
            "geometry": geometry,
        }));
        raster_layers.push(DeliveredRasterMeta {
            id: format!("eostage_raster_{idx}"),
            name,
            path: path_str,
            driver,
            crs,
            bounds,
            width,
            height,
            count,
        });
    }

    for vec_path in &vectors {
        let path_str = vec_path.to_string_lossy().to_string();
        let name = vec_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("vector")
            .to_string();
        features.push(json!({
            "type": "Feature",
            "properties": {
                "kind": "vector",
                "name": name,
                "path": path_str,
            },
            "geometry": Value::Null,
        }));
    }

    let catalog_json = serde_json::to_string_pretty(&json!({
        "type": "FeatureCollection",
        "name": "eostage_catalog",
        "features": features,
    }))
    .map_err(|err| format!("序列化交付目录 GeoJSON 失败: {err}"))?;
    write_atomic_utf8(&catalog_path, &format!("{catalog_json}\n"))?;

    let canvas_extent = raster_layers
        .iter()
        .filter_map(|r| r.bounds)
        .reduce(|acc, b| {
            [
                acc[0].min(b[0]),
                acc[1].min(b[1]),
                acc[2].max(b[2]),
                acc[3].max(b[3]),
            ]
        })
        .or(wgs_union)
        .unwrap_or([-180.0, -90.0, 180.0, 90.0]);
    let primary_crs = raster_layers
        .first()
        .map(|r| r.crs.as_str())
        .unwrap_or("EPSG:4326");

    let mut qgs = String::new();
    qgs.push_str("<!DOCTYPE qgis PUBLIC 'http://mrcc.com/qgis.dtd' 'SYSTEM'>\n");
    qgs.push_str("<qgis version=\"4.2.3\" projectname=\"EOStage Delivery\">\n");
    qgs.push_str("  <title>EOStage GIS Delivery Project</title>\n");
    qgs.push_str("  <projectCrs>\n");
    qgs.push_str("    <spatialrefsys>\n");
    qgs.push_str(&format!("      {}\n", render_qgis_srs_inner(primary_crs)));
    qgs.push_str("    </spatialrefsys>\n");
    qgs.push_str("  </projectCrs>\n");
    qgs.push_str("  <mapcanvas name=\"theMapCanvas\">\n");
    qgs.push_str("    <extent>\n");
    qgs.push_str(&format!("      <xmin>{}</xmin>\n", canvas_extent[0]));
    qgs.push_str(&format!("      <ymin>{}</ymin>\n", canvas_extent[1]));
    qgs.push_str(&format!("      <xmax>{}</xmax>\n", canvas_extent[2]));
    qgs.push_str(&format!("      <ymax>{}</ymax>\n", canvas_extent[3]));
    qgs.push_str("    </extent>\n");
    qgs.push_str("  </mapcanvas>\n");
    qgs.push_str("  <layer-tree-group>\n");
    for layer in &raster_layers {
        qgs.push_str(&format!(
            "    <layer-tree-layer id=\"{}\" name=\"{}\" checked=\"Qt::Checked\" expanded=\"1\"/>\n",
            xml_escape(&layer.id),
            xml_escape(&layer.name)
        ));
    }
    for (idx, vec_path) in vectors.iter().enumerate() {
        let name = vec_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("vector");
        qgs.push_str(&format!(
            "    <layer-tree-layer id=\"eostage_vector_{idx}\" name=\"{}\" checked=\"Qt::Checked\" expanded=\"1\"/>\n",
            xml_escape(name)
        ));
    }
    qgs.push_str("    <layer-tree-layer id=\"eostage_catalog_geojson\" name=\"eostage_catalog\" checked=\"Qt::Checked\" expanded=\"1\"/>\n");
    qgs.push_str("  </layer-tree-group>\n");
    qgs.push_str("  <projectlayers>\n");
    for layer in &raster_layers {
        qgs.push_str("    <maplayer type=\"raster\" hasScaleBasedVisibilityFlag=\"0\">\n");
        qgs.push_str(&format!("      <id>{}</id>\n", xml_escape(&layer.id)));
        qgs.push_str(&format!(
            "      <datasource>{}</datasource>\n",
            xml_escape(&layer.path)
        ));
        qgs.push_str(&format!(
            "      <layername>{}</layername>\n",
            xml_escape(&layer.name)
        ));
        qgs.push_str("      <provider>gdal</provider>\n");
        qgs.push_str(&format!(
            "      <customproperties><eostage_driver>{}</eostage_driver><eostage_width>{}</eostage_width><eostage_height>{}</eostage_height><eostage_bands>{}</eostage_bands></customproperties>\n",
            xml_escape(&layer.driver),
            layer.width,
            layer.height,
            layer.count
        ));
        if let Some(b) = layer.bounds {
            qgs.push_str(&format!(
                "      <extent><xmin>{}</xmin><ymin>{}</ymin><xmax>{}</xmax><ymax>{}</ymax></extent>\n",
                b[0], b[1], b[2], b[3]
            ));
        }
        qgs.push_str(&format!(
            "      <srs><spatialrefsys>{}</spatialrefsys></srs>\n",
            render_qgis_srs_inner(&layer.crs)
        ));
        qgs.push_str("    </maplayer>\n");
    }
    for (idx, vec_path) in vectors.iter().enumerate() {
        let name = vec_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("vector");
        let path_str = vec_path.to_string_lossy();
        qgs.push_str("    <maplayer type=\"vector\" hasScaleBasedVisibilityFlag=\"0\">\n");
        qgs.push_str(&format!("      <id>eostage_vector_{idx}</id>\n"));
        qgs.push_str(&format!(
            "      <datasource>{}</datasource>\n",
            xml_escape(&path_str)
        ));
        qgs.push_str(&format!(
            "      <layername>{}</layername>\n",
            xml_escape(name)
        ));
        qgs.push_str("      <provider encoding=\"UTF-8\">ogr</provider>\n");
        qgs.push_str("    </maplayer>\n");
    }
    qgs.push_str("    <maplayer type=\"vector\" hasScaleBasedVisibilityFlag=\"0\">\n");
    qgs.push_str("      <id>eostage_catalog_geojson</id>\n");
    qgs.push_str(&format!(
        "      <datasource>{}</datasource>\n",
        xml_escape(&catalog_path.to_string_lossy())
    ));
    qgs.push_str("      <layername>eostage_catalog</layername>\n");
    qgs.push_str("      <provider encoding=\"UTF-8\">ogr</provider>\n");
    qgs.push_str("    </maplayer>\n");
    qgs.push_str("  </projectlayers>\n");
    qgs.push_str("</qgis>\n");
    write_atomic_utf8(&project_path, &qgs)?;

    let mut gui_launched = false;
    if request.open_gui {
        let report = inspect_external_gis_toolchains(runtime_root, app_data_dir, false)?;
        let info = report
            .toolchains
            .into_iter()
            .find(|t| t.id == toolchain)
            .ok_or_else(|| format!("未找到工具链: {toolchain}"))?;
        let launch_target = info
            .gui_path
            .or(info.executable_path)
            .ok_or_else(|| format!("未检测到可启动的 {} 桌面程序", info.display_name))?;
        let launch_path = Path::new(&launch_target);
        let arg_file = if toolchain == "qgis" {
            &project_path
        } else {
            &catalog_path
        };
        #[cfg(target_os = "macos")]
        let mut child_cmd = {
            let is_app = launch_path
                .extension()
                .and_then(|s| s.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("app"))
                .unwrap_or(false);
            if is_app {
                let mut c = Command::new("/usr/bin/open");
                apply_sanitized_subprocess_env(&mut c, None);
                c.arg("-a").arg(launch_path).arg(arg_file);
                c
            } else {
                let mut c = Command::new(launch_path);
                apply_sanitized_subprocess_env(&mut c, Some(launch_path));
                c.arg(arg_file);
                c
            }
        };
        #[cfg(not(target_os = "macos"))]
        let mut child_cmd = {
            let mut c = Command::new(launch_path);
            apply_sanitized_subprocess_env(&mut c, Some(launch_path));
            c.arg(arg_file);
            c
        };
        child_cmd
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| format!("启动 {} 桌面端失败: {err}", info.display_name))?;
        gui_launched = true;
    }

    let proj_str = project_path.to_string_lossy().to_string();
    let cat_str = catalog_path.to_string_lossy().to_string();
    Ok(ExternalGisExecuteResult {
        ok: true,
        toolchain: toolchain.to_string(),
        action: "deliver_project".to_string(),
        exit_code: Some(0),
        stdout: String::new(),
        stderr: String::new(),
        output_path: Some(proj_str.clone()),
        project_path: Some(proj_str.clone()),
        catalog_path: Some(cat_str.clone()),
        gui_launched,
        detail: format!(
            "已生成 GIS 交付工程（{} 个栅格图层，{} 个矢量图层）",
            raster_layers.len(),
            vectors.len()
        ),
        artifacts: vec![proj_str, cat_str],
        dehydration: None,
    })
}

pub fn install_conda_geo_package(
    runtime_root: &Path,
    app_data_dir: &Path,
    request: &CondaGeoInstallRequest,
) -> Result<CondaGeoInstallResult, String> {
    if !request.confirmed {
        return Err(
            "CONDA_INSTALL_CONFIRMATION_REQUIRED: 需要用户明确点击确认后才能安装科学计算包"
                .to_string(),
        );
    }
    let pkg = request.package_id.trim().to_ascii_lowercase();
    let (conda_pkg, pip_pkg, conda_only) = match pkg.as_str() {
        "isce2" => ("isce2", None, true),
        "mintpy" => ("mintpy", Some("mintpy"), false),
        "xdem" => ("xdem", Some("xdem"), false),
        "pytmd" => ("pytmd", Some("pyTMD"), false),
        "torchgeo" => ("torchgeo", Some("torchgeo"), false),
        _ => {
            return Err(format!(
                "CONDA_UNSUPPORTED_GEO_PACKAGE: 仅允许安装白名单地学科学包 (isce2 / mintpy / xdem / pytmd / torchgeo)，收到 '{}'",
                request.package_id
            ));
        }
    };

    let report = inspect_external_gis_toolchains(runtime_root, app_data_dir, false)?;
    let env_info = select_conda_environment(&report, request.conda_env.as_deref())?;
    let prefix = Path::new(&env_info.env_prefix);
    let exe = Path::new(&env_info.python_path);

    let conda_bin_candidates = [
        prefix.join("bin").join("conda"),
        prefix
            .parent()
            .and_then(|p| p.parent())
            .map(|r| r.join("bin").join("conda"))
            .unwrap_or_default(),
        prefix
            .parent()
            .and_then(|p| p.parent())
            .map(|r| r.join("condabin").join("conda"))
            .unwrap_or_default(),
        PathBuf::from("conda"),
    ];
    let resolved_conda = conda_bin_candidates.into_iter().find(|p| p.is_file());

    let (mut cmd, preview) = if let Some(conda_exe) = resolved_conda {
        let preview_str = format!(
            "conda install -y -p {} -c conda-forge {conda_pkg}",
            env_info.env_prefix
        );
        let mut c = Command::new(&conda_exe);
        apply_sanitized_subprocess_env(&mut c, Some(&conda_exe));
        c.args([
            "install",
            "-y",
            "-p",
            &env_info.env_prefix,
            "-c",
            "conda-forge",
            conda_pkg,
        ]);
        (c, preview_str)
    } else if !conda_only {
        let pip_target = pip_pkg.unwrap_or(conda_pkg);
        let preview_str = format!("{} -m pip install {pip_target}", env_info.python_path);
        let mut c = Command::new(exe);
        apply_sanitized_subprocess_env(&mut c, Some(exe));
        prepend_conda_env_path(&mut c, prefix);
        c.args(["-m", "pip", "install", pip_target]);
        (c, preview_str)
    } else {
        return Err(
            "CONDA_BINARY_REQUIRED_FOR_ISCE2: ISCE2 包含原生 C++/Fortran 模块，未在本机检测到 conda 可执行文件，请先激活或配置带 conda 的科学环境。".to_string(),
        );
    };

    let scratch = create_isolated_scratch_dir(app_data_dir, "conda_install")?;
    let budget = SubprocessScratchBudget {
        max_scratch_bytes: 512 * 1024 * 1024,
        max_scratch_files: 1024,
        timeout: Duration::from_secs(300),
        poll_interval: Duration::from_millis(50),
    };
    let outcome = budget
        .execute_command(&mut cmd, &scratch.0)
        .map_err(|err| format!("启动包安装进程失败: {err}"))?;
    let raw_stdout = String::from_utf8_lossy(&outcome.stdout);
    let raw_stderr = String::from_utf8_lossy(&outcome.stderr);
    let (stdout, stderr, _dehydration) = finalize_dehydrated_outputs(
        &raw_stdout,
        &raw_stderr,
        MAX_OUTPUT_BYTES,
        &report.log_dehydrator,
        app_data_dir,
    );
    if outcome.status_code != Some(0) {
        return Err(format!(
            "在环境 {} 安装 {pkg} 失败 (exit {:?}): {}",
            env_info.name,
            outcome.status_code,
            if stderr.trim().is_empty() {
                &stdout
            } else {
                &stderr
            }
        ));
    }

    let refreshed = inspect_external_gis_toolchains(runtime_root, app_data_dir, false).ok();
    Ok(CondaGeoInstallResult {
        ok: true,
        package_id: pkg,
        env_name: env_info.name.clone(),
        command_preview: preview,
        stdout,
        stderr,
        detail: format!("已在 {} 完成安装并刷新环境包清单", env_info.name),
        report: refreshed,
    })
}

fn arcpy_default_output_param_name(alg: &str) -> &'static str {
    let trimmed = alg.trim();
    let lower = trimmed.to_ascii_lowercase();
    let leaf = lower.rsplit('.').next().unwrap_or(lower.as_str());
    match leaf {
        "createtable"
        | "tabletotable"
        | "createfeatureclass"
        | "featureclasstofeatureclass" => "out_path+out_name",
        "mosaictonewraster" => "output_location+raster_dataset_name_with_extension",
        "copyraster"
        | "projectraster"
        | "resample"
        | "compositebands"
        | "polygontoraster"
        | "featuretoraster"
        | "pointtoraster" => "out_rasterdataset",
        "extractbymask"
        | "slope"
        | "aspect"
        | "hillshade"
        | "focalstatistics"
        | "zonalstatistics"
        | "reclassify" => "out_raster",
        "clip" if lower.starts_with("management.") || lower.starts_with("arcpy.management.") => {
            "out_raster"
        }
        "clip" => "out_feature_class",
        "copyrows"
        | "statistics"
        | "frequency"
        | "summarystatistics"
        | "zonalstatisticsastable"
        | "tabletoexcel" => "out_table",
        "rastertopolygon" => "out_polygon_features",
        "rastertopoint" => "out_point_features",
        "rastertopolyline" => "out_polyline_features",
        _ => "out_feature_class",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use insar_gis::{RasterProcessRequest, VectorProcessRequest};
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

    struct TestTempDir(PathBuf);

    impl TestTempDir {
        fn new() -> Self {
            let seq = TEST_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let dir = std::env::temp_dir().join(format!(
                "eostage-ext-gis-test-{}-{nanos}-{seq}",
                std::process::id()
            ));
            fs::create_dir_all(&dir).expect("create temp test dir");
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestTempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(unix)]
    fn write_executable_stub(dir: &Path, name: &str, script: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        fs::write(&path, script).expect("write stub");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod stub");
        path
    }

    #[test]
    fn external_gis_prefs_validation_and_persistence_work() {
        let dir = TestTempDir::new();
        let app_data = dir.path();

        let empty = load_external_gis_prefs(app_data).expect("empty load");
        assert!(empty.custom_paths.is_empty());

        let mut bad_prefs = ExternalGisPrefs::default();
        bad_prefs
            .custom_paths
            .insert("rustgis".to_string(), "relative/rustgis".to_string());
        assert!(save_external_gis_prefs(app_data, &bad_prefs).is_err());

        #[cfg(unix)]
        {
            let exe = write_executable_stub(
                app_data,
                "fake-rustgis",
                "#!/bin/sh\necho 'rustgis 0.9.1'\n",
            );
            let mut good = ExternalGisPrefs::default();
            good.custom_paths
                .insert("rustgis".to_string(), exe.to_string_lossy().to_string());
            good.preferred_toolchain = Some("rustgis".to_string());
            let saved = save_external_gis_prefs(app_data, &good).expect("save valid prefs");
            assert_eq!(saved.preferred_toolchain.as_deref(), Some("rustgis"));
            let reloaded = load_external_gis_prefs(app_data).expect("reload prefs");
            assert_eq!(reloaded, saved);
        }
    }

    #[cfg(unix)]
    #[test]
    fn custom_rustgis_stub_executes_headless_and_redacts_secrets() {
        let dir = TestTempDir::new();
        let app_data = dir.path().join("appdata");
        fs::create_dir_all(&app_data).unwrap();
        let runtime_root = dir.path().join("runtime_root");
        fs::create_dir_all(&runtime_root).unwrap();

        let stub = write_executable_stub(
            dir.path(),
            "rustgis-cli",
            r#"#!/bin/sh
if [ "$1" = "mcp" ]; then
  read -r _line
  echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"rustgis","title":"RustGIS","version":"0.8.2"}}}'
  exit 0
fi
if [ "$1" = "tools" ]; then
  echo '[{"id":"buffer","name":"Buffer"}]'
  exit 0
fi
if [ "$1" = "run" ]; then
  out=''
  for a in "$@"; do
    case "$a" in
      share.exportLayer=*)
        out=$(printf '%s' "$a" | sed -n 's/.*"path":"\([^"]*\)".*/\1/p')
        ;;
    esac
  done
  echo 'token=supersecret sk-testsecret123456'
  printf '{"type":"FeatureCollection","features":[]}' > "$out"
  exit 0
fi
exit 1
"#,
        );

        let mut prefs = ExternalGisPrefs::default();
        prefs
            .custom_paths
            .insert("rustgis".to_string(), stub.to_string_lossy().to_string());
        save_external_gis_prefs(&app_data, &prefs).expect("save prefs");

        let report =
            inspect_external_gis_toolchains(&runtime_root, &app_data, true).expect("inspect");
        let rustgis = report
            .toolchains
            .iter()
            .find(|t| t.id == "rustgis")
            .expect("rustgis entry");
        assert!(rustgis.installed);
        assert_eq!(rustgis.source, "custom_path");
        assert_eq!(rustgis.version.as_deref(), Some("rustgis 0.8.2"));

        let tools_res = execute_external_gis_action(
            &runtime_root,
            &app_data,
            &ExternalGisExecuteRequest {
                toolchain: "rustgis".to_string(),
                action: "list_algorithms".to_string(),
                algorithm: None,
                params: BTreeMap::new(),
                input_paths: vec![],
                output_path: None,
                conda_env: None,
                open_gui: false,
                timeout_seconds: Some(10),
            },
        )
        .expect("list rustgis tools");
        assert!(tools_res.stdout.contains("\"buffer\""));

        let out_file = dir.path().join("rustgis-out.geojson");
        let mut params = BTreeMap::new();
        params.insert("distance".to_string(), json!(250));
        let exec_res = execute_external_gis_action(
            &runtime_root,
            &app_data,
            &ExternalGisExecuteRequest {
                toolchain: "rustgis".to_string(),
                action: "run_algorithm".to_string(),
                algorithm: Some("vector:buffer".to_string()),
                params,
                input_paths: vec![],
                output_path: Some(out_file.to_string_lossy().to_string()),
                conda_env: None,
                open_gui: false,
                timeout_seconds: Some(10),
            },
        )
        .expect("execute rustgis stub");
        assert!(exec_res.ok);
        assert!(out_file.is_file());
        assert!(!exec_res.stdout.contains("supersecret"));
        assert!(!exec_res.stdout.contains("sk-testsecret"));
        assert!(exec_res.stdout.contains("[REDACTED]"));
    }

    #[cfg(unix)]
    #[test]
    fn arcgis_pro_rejects_injection_and_uses_isolated_dash_i_c_argv() {
        let dir = TestTempDir::new();
        let app_data = dir.path().join("appdata");
        fs::create_dir_all(&app_data).unwrap();
        let runtime_root = dir.path().join("runtime_root");
        fs::create_dir_all(&runtime_root).unwrap();

        let argv_capture = dir.path().join("captured_argv.txt");
        let job_capture = dir.path().join("captured_job.json");
        let fake_py = write_executable_stub(
            dir.path(),
            "fake-arcpy-python",
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$1\" \"$2\" > \"{}\"\ncp \"$4\" \"{}\"\necho '{{\"ok\":true,\"version\":\"3.3\"}}'\n",
                argv_capture.display(),
                job_capture.display()
            ),
        );

        let mut prefs = ExternalGisPrefs::default();
        prefs.custom_paths.insert(
            "arcgis_pro".to_string(),
            fake_py.to_string_lossy().to_string(),
        );
        save_external_gis_prefs(&app_data, &prefs).expect("save prefs");

        let injection_err = execute_external_gis_action(
            &runtime_root,
            &app_data,
            &ExternalGisExecuteRequest {
                toolchain: "arcgis_pro".to_string(),
                action: "run_algorithm".to_string(),
                algorithm: Some("Buffer_analysis; import os; os.system('id')".to_string()),
                params: BTreeMap::new(),
                input_paths: vec![],
                output_path: None,
                conda_env: None,
                open_gui: false,
                timeout_seconds: None,
            },
        )
        .expect_err("injection must be rejected");
        assert!(injection_err.contains("不合法"));

        let dunder_err = execute_external_gis_action(
            &runtime_root,
            &app_data,
            &ExternalGisExecuteRequest {
                toolchain: "arcgis_pro".to_string(),
                action: "run_algorithm".to_string(),
                algorithm: Some("arcpy.__import__".to_string()),
                params: BTreeMap::new(),
                input_paths: vec![],
                output_path: None,
                conda_env: None,
                open_gui: false,
                timeout_seconds: None,
            },
        )
        .expect_err("dunder must be rejected");
        assert!(dunder_err.contains("不合法"));

        let mut params = BTreeMap::new();
        params.insert("buffer_distance_or_field".to_string(), json!("500 Meters"));
        let res = execute_external_gis_action(
            &runtime_root,
            &app_data,
            &ExternalGisExecuteRequest {
                toolchain: "arcgis_pro".to_string(),
                action: "run_algorithm".to_string(),
                algorithm: Some("arcpy.analysis.Buffer".to_string()),
                params,
                input_paths: vec![],
                output_path: None,
                conda_env: None,
                open_gui: false,
                timeout_seconds: Some(10),
            },
        )
        .expect("isolated arcpy run succeeds");
        assert!(res.ok);
        let argv_lines = fs::read_to_string(&argv_capture).expect("argv captured");
        assert_eq!(argv_lines.lines().collect::<Vec<_>>(), vec!["-I", "-c"]);
        let captured_job = fs::read_to_string(&job_capture).expect("job captured");
        assert!(captured_job.contains("\"arcpy.analysis.Buffer\""));
    }

    #[test]
    #[ignore = "requires optional native GIS runtime (run scripts/run_native_gis_tests.sh with INSAR_GIS_RUNTIME_ROOT)"]
    fn deliver_project_builds_catalog_geojson_and_well_formed_qgs_xml() {
        let dir = TestTempDir::new();
        let app_data = dir.path().join("appdata");
        fs::create_dir_all(&app_data).unwrap();
        let runtime_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let vector_path = dir.path().join("aoi.geojson");
        fs::write(
            &vector_path,
            r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{"name":"Hefei"},"geometry":{"type":"Point","coordinates":[117.28,31.86]}}]}"#,
        )
        .unwrap();

        let buffered_geojson = dir.path().join("buffered.geojson");
        crate::geodata_tools::execute_gis_vector_process(&VectorProcessRequest {
            input_path: vector_path,
            output_path: buffered_geojson.clone(),
            operation: "buffer".to_string(),
            distance_m: Some(500.0),
            quad_segs: None,
        })
        .expect("vector buffer via built-in PROJ/OGR");
        assert!(buffered_geojson.is_file());

        // Create a real ENVI raster and convert to GeoTIFF with built-in GDAL runtime
        let raw_dat = dir.path().join("seed.dat");
        let raw_hdr = dir.path().join("seed.hdr");
        fs::write(&raw_dat, vec![0u8; 16]).unwrap();
        fs::write(
            &raw_hdr,
            "ENVI\nsamples = 2\nlines = 2\nbands = 1\nheader offset = 0\nfile type = ENVI Standard\ndata type = 4\ninterleave = bsq\nbyte order = 0\nmap info = {Geographic Lat/Lon, 1.0, 1.0, 117.0, 32.0, 0.01, 0.01, WGS-84, units=Degrees}\n",
        )
        .unwrap();
        let src_tif = dir.path().join("sample_raw.tif");
        crate::geodata_tools::execute_gis_raster_process(&RasterProcessRequest {
            sources: vec![raw_dat.to_string_lossy().to_string()],
            output_path: src_tif.clone(),
            operation: "translate".to_string(),
            output_format: Some("GTiff".to_string()),
            preserve_dtype: Some(true),
            ..RasterProcessRequest::default()
        })
        .expect("translate seed ENVI raster to GeoTIFF");
        assert!(src_tif.is_file());

        // Write a satellite metadata .json sidecar alongside valid inputs; deliver_project must ignore non-GeoJSON .json files
        let sidecar_mtl = dir.path().join("LC09_MTL.json");
        fs::write(
            &sidecar_mtl,
            r#"{"LANDSAT_METADATA_FILE":{"PRODUCT_CONTENTS":{}}}"#,
        )
        .unwrap();

        let delivery_dir = dir.path().join("delivery & export");
        let deliver_res = execute_external_gis_action(
            &runtime_root,
            &app_data,
            &ExternalGisExecuteRequest {
                toolchain: "qgis".to_string(),
                action: "deliver_project".to_string(),
                algorithm: None,
                params: BTreeMap::new(),
                input_paths: vec![
                    src_tif.to_string_lossy().to_string(),
                    buffered_geojson.to_string_lossy().to_string(),
                    sidecar_mtl.to_string_lossy().to_string(),
                ],
                output_path: Some(delivery_dir.to_string_lossy().to_string()),
                conda_env: None,
                open_gui: false,
                timeout_seconds: None,
            },
        )
        .expect("deliver_project succeeds");

        assert!(deliver_res.ok);
        let qgs_path = PathBuf::from(deliver_res.project_path.as_deref().unwrap());
        let cat_path = PathBuf::from(deliver_res.catalog_path.as_deref().unwrap());
        assert!(qgs_path.is_file());
        assert!(cat_path.is_file());
        let qgs_text = fs::read_to_string(&qgs_path).unwrap();
        assert!(qgs_text.contains("&amp; export"));
        assert!(qgs_text.contains("id=\"eostage_raster_0\""));
        assert!(qgs_text.contains("<layername>sample_raw</layername>"));
        assert!(qgs_text.contains("<layername>buffered</layername>"));
        assert!(!qgs_text.contains("LC09_MTL"));

        let cat_json: Value =
            serde_json::from_str(&fs::read_to_string(&cat_path).unwrap()).expect("valid geojson");
        assert_eq!(cat_json["type"].as_str(), Some("FeatureCollection"));
        assert_eq!(cat_json["features"].as_array().map(|a| a.len()), Some(2));
        assert_eq!(
            cat_json["features"][0]["geometry"]["type"].as_str(),
            Some("Polygon")
        );

        let xml_check = Command::new("python3")
            .arg("-c")
            .arg("import sys, xml.etree.ElementTree as ET; ET.parse(sys.argv[1])")
            .arg(&qgs_path)
            .status();
        if let Ok(status) = xml_check {
            assert!(status.success(), "generated .qgs must be valid XML");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires optional local QGIS / rustgis-cli / geolibre binaries on macOS"]
    fn live_installed_toolchains_discovery_and_versions() {
        let qgis_app = Path::new("/Applications/QGIS-final-4_2_3.app");
        let rustgis_bin = Path::new("/opt/homebrew/bin/rustgis-cli");
        let geolibre_bin = Path::new("/opt/homebrew/bin/geolibre");
        if !qgis_app.is_dir() || !rustgis_bin.is_file() || !geolibre_bin.is_file() {
            eprintln!("SKIPPED live_installed_toolchains_discovery_and_versions: binaries missing");
            return;
        }

        let dir = TestTempDir::new();
        let app_data = dir.path().join("appdata");
        fs::create_dir_all(&app_data).unwrap();
        let empty_runtime_root = dir.path().join("runtime_root");
        fs::create_dir_all(&empty_runtime_root).unwrap();

        let report =
            inspect_external_gis_toolchains(&empty_runtime_root, &app_data, true).expect("inspect");
        let rustgis = report
            .toolchains
            .iter()
            .find(|t| t.id == "rustgis")
            .expect("rustgis");
        assert!(rustgis.installed);
        assert!(rustgis.mcp_capable);
        assert!(
            rustgis.version.as_deref().unwrap_or("").contains("0.6.0"),
            "unexpected rustgis version: {:?}",
            rustgis.version
        );

        let geolibre = report
            .toolchains
            .iter()
            .find(|t| t.id == "geolibre")
            .expect("geolibre");
        assert!(geolibre.installed);
        assert!(!geolibre.mcp_capable);
        assert!(
            geolibre.version.as_deref().unwrap_or("").contains("1.8.2"),
            "unexpected geolibre version: {:?}",
            geolibre.version
        );
        if Path::new("/Applications/GeoLibre Desktop.app").is_dir() {
            assert_eq!(
                geolibre.gui_path.as_deref(),
                Some("/Applications/GeoLibre Desktop.app")
            );
            assert!(geolibre.gui_capable);
        }

        let qgis = report
            .toolchains
            .iter()
            .find(|t| t.id == "qgis")
            .expect("qgis");
        assert!(qgis.installed);
        assert!(qgis.headless_capable);
        assert!(qgis.gui_capable);
        assert!(
            qgis.version.as_deref().unwrap_or("").contains("4.2.3"),
            "unexpected qgis version: {:?}",
            qgis.version
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires optional native GIS runtime (run scripts/run_native_gis_tests.sh with INSAR_GIS_RUNTIME_ROOT)"]
    fn live_qgis_geolibre_rustgis_and_pyqgis_end_to_end() {
        let qgis_py = Path::new("/Applications/QGIS-final-4_2_3.app/Contents/MacOS/python3.12");
        let rustgis_bin = Path::new("/opt/homebrew/bin/rustgis-cli");
        let geolibre_bin = Path::new("/opt/homebrew/bin/geolibre");
        if !qgis_py.is_file() || !rustgis_bin.is_file() || !geolibre_bin.is_file() {
            eprintln!("SKIPPED live_qgis_geolibre_rustgis_and_pyqgis_end_to_end: binaries missing");
            return;
        }

        let dir = TestTempDir::new();
        let app_data = dir.path().join("appdata");
        fs::create_dir_all(&app_data).unwrap();
        let runtime_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");

        // Prepare real vector + real float32 DEM GeoTIFF via built-in GDAL
        let seed_pts = dir.path().join("hefei_pts.geojson");
        fs::write(
            &seed_pts,
            r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{"name":"Hefei"},"geometry":{"type":"Point","coordinates":[117.2830,31.8612]}}]}"#,
        )
        .unwrap();

        let raw_dat = dir.path().join("dem4x4.dat");
        let raw_hdr = dir.path().join("dem4x4.hdr");
        let mut dem_bytes = Vec::with_capacity(16 * 4);
        for i in 0..16 {
            let elev: f32 = 50.0 + (i as f32) * 12.5;
            dem_bytes.extend_from_slice(&elev.to_le_bytes());
        }
        fs::write(&raw_dat, dem_bytes).unwrap();
        fs::write(
            &raw_hdr,
            "ENVI\nsamples = 4\nlines = 4\nbands = 1\nheader offset = 0\nfile type = ENVI Standard\ndata type = 4\ninterleave = bsq\nbyte order = 0\nmap info = {Geographic Lat/Lon, 1.0, 1.0, 117.2, 31.9, 0.001, 0.001, WGS-84, units=Degrees}\n",
        )
        .unwrap();
        let dem_tif = dir.path().join("hefei_dem.tif");
        crate::geodata_tools::execute_gis_raster_process(&RasterProcessRequest {
            sources: vec![raw_dat.to_string_lossy().to_string()],
            output_path: dem_tif.clone(),
            operation: "translate".to_string(),
            output_format: Some("GTiff".to_string()),
            preserve_dtype: Some(true),
            ..RasterProcessRequest::default()
        })
        .expect("translate seed DEM");

        // 1) Live QGIS list_algorithms + native:buffer + deliver_project + PyQGIS load
        let qgis_list = execute_external_gis_action(
            &runtime_root,
            &app_data,
            &ExternalGisExecuteRequest {
                toolchain: "qgis".to_string(),
                action: "list_algorithms".to_string(),
                algorithm: None,
                params: BTreeMap::new(),
                input_paths: vec![],
                output_path: None,
                conda_env: None,
                open_gui: false,
                timeout_seconds: Some(30),
            },
        )
        .expect("qgis list_algorithms");
        assert!(qgis_list.stdout.contains("native:buffer"));

        let qgis_buf_out = dir.path().join("qgis_buffer.geojson");
        let mut qgis_params = BTreeMap::new();
        qgis_params.insert("INPUT".to_string(), json!(seed_pts.to_string_lossy()));
        qgis_params.insert("DISTANCE".to_string(), json!(0.01));
        qgis_params.insert("SEGMENTS".to_string(), json!(8));
        let qgis_buf = execute_external_gis_action(
            &runtime_root,
            &app_data,
            &ExternalGisExecuteRequest {
                toolchain: "qgis".to_string(),
                action: "run_algorithm".to_string(),
                algorithm: Some("native:buffer".to_string()),
                params: qgis_params,
                input_paths: vec![],
                output_path: Some(qgis_buf_out.to_string_lossy().to_string()),
                conda_env: None,
                open_gui: false,
                timeout_seconds: Some(60),
            },
        )
        .expect("qgis native:buffer");
        assert!(qgis_buf.ok && qgis_buf_out.is_file());
        assert!(fs::read_to_string(&qgis_buf_out)
            .unwrap()
            .contains("Polygon"));

        let delivery_dir = dir.path().join("qgis_delivery");
        let delivered = execute_external_gis_action(
            &runtime_root,
            &app_data,
            &ExternalGisExecuteRequest {
                toolchain: "qgis".to_string(),
                action: "deliver_project".to_string(),
                algorithm: None,
                params: BTreeMap::new(),
                input_paths: vec![
                    dem_tif.to_string_lossy().to_string(),
                    qgis_buf_out.to_string_lossy().to_string(),
                ],
                output_path: Some(delivery_dir.to_string_lossy().to_string()),
                conda_env: None,
                open_gui: false,
                timeout_seconds: None,
            },
        )
        .expect("qgis deliver_project");
        let qgs_file = delivered.project_path.as_deref().expect("qgs path");
        let qgs_contents = fs::read_to_string(qgs_file).unwrap();
        assert!(!qgs_contents.contains("<wkt>EPSG:"));
        assert!(
            qgs_contents.contains("<authid>EPSG:")
                || qgs_contents.contains("<wkt>GEOGCS")
                || qgs_contents.contains("<wkt>GEOGCRS"),
            "expected valid spatialrefsys in .qgs: {qgs_contents}"
        );

        let pyqgis_script = r#"import sys
from qgis.core import QgsApplication, QgsProject
QgsApplication.setPrefixPath('/Applications/QGIS-final-4_2_3.app/Contents/MacOS', False)
app = QgsApplication([], False)
app.initQgis()
proj = QgsProject.instance()
assert proj.read(sys.argv[1]), f"failed reading {sys.argv[1]}"
layers = proj.mapLayers()
assert len(layers) == 3, f"expected 3 layers, got {len(layers)}"
for lid, lyr in layers.items():
    assert lyr.isValid(), f"invalid layer {lid}"
print("PYQGIS_OK", len(layers))
"#;
        let py_out = Command::new(qgis_py)
            .env_clear()
            .env("HOME", dir.path())
            .env(
                "PYTHONPATH",
                "/Applications/QGIS-final-4_2_3.app/Contents/Resources/python3.12:/Applications/QGIS-final-4_2_3.app/Contents/Resources/python3.12/lib-dynload:/Applications/QGIS-final-4_2_3.app/Contents/Resources/python3.12/site-packages:/Applications/QGIS-final-4_2_3.app/Contents/Resources/qgis/python",
            )
            .env(
                "PROJ_DATA",
                "/Applications/QGIS-final-4_2_3.app/Contents/Resources/qgis/proj",
            )
            .env(
                "PROJ_LIB",
                "/Applications/QGIS-final-4_2_3.app/Contents/Resources/qgis/proj",
            )
            .env(
                "GDAL_DATA",
                "/Applications/QGIS-final-4_2_3.app/Contents/Resources/qgis/gdal",
            )
            .env("QT_QPA_PLATFORM", "offscreen")
            .arg("-c")
            .arg(pyqgis_script)
            .arg(qgs_file)
            .output()
            .expect("invoke PyQGIS 4.2.3");
        assert!(
            py_out.status.success(),
            "PyQGIS failed: stdout={}, stderr={}",
            String::from_utf8_lossy(&py_out.stdout),
            String::from_utf8_lossy(&py_out.stderr)
        );
        assert!(String::from_utf8_lossy(&py_out.stdout).contains("PYQGIS_OK 3"));

        // 2) Live GeoLibre list + slope on real GeoTIFF + native inspect verification
        let geo_list = execute_external_gis_action(
            &runtime_root,
            &app_data,
            &ExternalGisExecuteRequest {
                toolchain: "geolibre".to_string(),
                action: "list_algorithms".to_string(),
                algorithm: None,
                params: BTreeMap::new(),
                input_paths: vec![],
                output_path: None,
                conda_env: None,
                open_gui: false,
                timeout_seconds: Some(30),
            },
        )
        .expect("geolibre list_algorithms");
        assert!(geo_list.stdout.lines().count() > 500);
        assert!(geo_list.stdout.lines().any(|l| l.trim() == "slope"));

        let slope_out = dir.path().join("hefei_slope.tif");
        let mut geo_params = BTreeMap::new();
        geo_params.insert("input".to_string(), json!(dem_tif.to_string_lossy()));
        geo_params.insert("units".to_string(), json!("degrees"));
        let geo_res = execute_external_gis_action(
            &runtime_root,
            &app_data,
            &ExternalGisExecuteRequest {
                toolchain: "geolibre".to_string(),
                action: "run_algorithm".to_string(),
                algorithm: Some("slope".to_string()),
                params: geo_params,
                input_paths: vec![],
                output_path: Some(slope_out.to_string_lossy().to_string()),
                conda_env: None,
                open_gui: false,
                timeout_seconds: Some(30),
            },
        )
        .expect("geolibre slope");
        assert!(geo_res.ok && slope_out.is_file());
        let slope_meta = crate::geodata_tools::discover_native_gis_engine()
            .inspect_raster_source(&slope_out.to_string_lossy())
            .expect("inspect geolibre slope GeoTIFF");
        assert_eq!(slope_meta["metadata"]["width"].as_i64(), Some(4));
        assert_eq!(slope_meta["metadata"]["height"].as_i64(), Some(4));

        // 3) Live RustGIS real zip component install + version + tools + render PNG + buffer GeoJSON + .rgis
        let comp_root = dir.path().join("component_root");
        let comp_app_data = dir.path().join("comp_appdata");
        fs::create_dir_all(&comp_root).unwrap();
        fs::create_dir_all(&comp_app_data).unwrap();
        let pkg_zip = dir.path().join("rustgis-live-macos.zip");
        {
            use sha2::{Digest, Sha256};
            let file = fs::File::create(&pkg_zip).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .unix_permissions(0o755);
            zip.start_file("bin/rustgis-cli", options).unwrap();
            let bin_bytes = fs::read(rustgis_bin).unwrap();
            zip.write_all(&bin_bytes).unwrap();
            zip.start_file(
                "component-manifest.json",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(br#"{"version":"0.6.0"}"#).unwrap();
            zip.finish().unwrap();

            let zip_bytes = fs::read(&pkg_zip).unwrap();
            let mut hasher = Sha256::new();
            hasher.update(&zip_bytes);
            let sha = format!("{:x}", hasher.finalize());

            let installed = crate::application::install_runtime_component_at(
                &comp_root,
                &crate::application::RuntimeComponentInstallRequest {
                    component_id: "rustgis-cli".to_string(),
                    source: pkg_zip.to_string_lossy().to_string(),
                    expected_sha256: Some(sha),
                },
            )
            .expect("install real rustgis-cli component zip");
            assert!(installed.ok);
        }

        let comp_inspect = inspect_external_gis_toolchains(&comp_root, &comp_app_data, true)
            .expect("inspect comp");
        let comp_rustgis = comp_inspect
            .toolchains
            .iter()
            .find(|t| t.id == "rustgis")
            .unwrap();
        assert_eq!(comp_rustgis.source, "bundled_component");
        assert!(comp_rustgis
            .version
            .as_deref()
            .unwrap_or("")
            .contains("0.6.0"));

        let ver_res = execute_external_gis_action(
            &comp_root,
            &comp_app_data,
            &ExternalGisExecuteRequest {
                toolchain: "rustgis".to_string(),
                action: "version".to_string(),
                algorithm: None,
                params: BTreeMap::new(),
                input_paths: vec![],
                output_path: None,
                conda_env: None,
                open_gui: false,
                timeout_seconds: Some(15),
            },
        )
        .expect("rustgis version");
        assert!(ver_res.stdout.contains("0.6.0"));

        let tools_res = execute_external_gis_action(
            &comp_root,
            &comp_app_data,
            &ExternalGisExecuteRequest {
                toolchain: "rustgis".to_string(),
                action: "list_algorithms".to_string(),
                algorithm: None,
                params: BTreeMap::new(),
                input_paths: vec![],
                output_path: None,
                conda_env: None,
                open_gui: false,
                timeout_seconds: Some(15),
            },
        )
        .expect("rustgis tools");
        assert!(tools_res.stdout.contains("\"buffer\""));

        let render_png = dir.path().join("rustgis_render.png");
        let render_res = execute_external_gis_action(
            &comp_root,
            &comp_app_data,
            &ExternalGisExecuteRequest {
                toolchain: "rustgis".to_string(),
                action: "run_algorithm".to_string(),
                algorithm: Some("render".to_string()),
                params: BTreeMap::new(),
                input_paths: vec![seed_pts.to_string_lossy().to_string()],
                output_path: Some(render_png.to_string_lossy().to_string()),
                conda_env: None,
                open_gui: false,
                timeout_seconds: Some(30),
            },
        )
        .expect("rustgis render png");
        assert!(render_res.ok && render_png.is_file());
        let png_bytes = fs::read(&render_png).unwrap();
        assert!(png_bytes.starts_with(b"\x89PNG"));

        let rg_buf_out = dir.path().join("rustgis_buf.geojson");
        let mut rg_params = BTreeMap::new();
        rg_params.insert("distance".to_string(), json!(500));
        rg_params.insert("units".to_string(), json!("Meters"));
        let rg_buf = execute_external_gis_action(
            &comp_root,
            &comp_app_data,
            &ExternalGisExecuteRequest {
                toolchain: "rustgis".to_string(),
                action: "run_algorithm".to_string(),
                algorithm: Some("buffer".to_string()),
                params: rg_params.clone(),
                input_paths: vec![seed_pts.to_string_lossy().to_string()],
                output_path: Some(rg_buf_out.to_string_lossy().to_string()),
                conda_env: None,
                open_gui: false,
                timeout_seconds: Some(30),
            },
        )
        .expect("rustgis buffer geojson");
        assert!(rg_buf.ok && rg_buf_out.is_file());
        assert!(fs::read_to_string(&rg_buf_out).unwrap().contains("Polygon"));

        let rg_proj_out = dir.path().join("rustgis_proj.rgis");
        let rg_proj = execute_external_gis_action(
            &comp_root,
            &comp_app_data,
            &ExternalGisExecuteRequest {
                toolchain: "rustgis".to_string(),
                action: "run_algorithm".to_string(),
                algorithm: Some("buffer".to_string()),
                params: rg_params,
                input_paths: vec![seed_pts.to_string_lossy().to_string()],
                output_path: Some(rg_proj_out.to_string_lossy().to_string()),
                conda_env: None,
                open_gui: false,
                timeout_seconds: Some(30),
            },
        )
        .expect("rustgis save .rgis");
        assert!(rg_proj.ok && fs::metadata(&rg_proj_out).unwrap().len() > 0);
    }

    #[cfg(unix)]
    #[test]
    fn conda_discovery_handles_intra_bin_symlink_conda_meta_and_custom_prefs() {
        let dir = TestTempDir::new();
        let app_data = dir.path().join("appdata");
        fs::create_dir_all(&app_data).unwrap();
        let runtime_root = dir.path().join("runtime_root");
        fs::create_dir_all(&runtime_root).unwrap();

        let conda_env = dir.path().join("envs").join("insar_py");
        let bin_dir = conda_env.join("bin");
        let meta_dir = conda_env.join("conda-meta");
        fs::create_dir_all(&bin_dir).unwrap();
        fs::create_dir_all(&meta_dir).unwrap();
        fs::write(meta_dir.join("rasterio-1.3.10-py311h_0.json"), "{}").unwrap();
        fs::write(meta_dir.join("gdal-3.9.2-py311_0.json"), "{}").unwrap();
        fs::write(meta_dir.join("numpy-1.26.4-py311_0.json"), "{}").unwrap();

        // Simulate realistic Conda intra-bin symlink (`bin/python3 -> python3.12`)
        let py312 = write_executable_stub(
            &bin_dir,
            "python3.12",
            "#!/bin/sh\necho '{\"ok\":true,\"python\":\"3.12.7\",\"packages\":{\"gdal\":\"3.9.2\",\"rasterio\":\"1.3.10\"}}'\n",
        );
        std::os::unix::fs::symlink(&py312, bin_dir.join("python3")).unwrap();

        let mock_rtk = write_executable_stub(dir.path(), "rtk", "#!/bin/sh\necho 'rtk 0.25.0'\n");

        let mut prefs = ExternalGisPrefs::default();
        prefs.conda_custom_envs.insert(
            "insar_py".to_string(),
            conda_env.to_string_lossy().to_string(),
        );
        prefs.preferred_conda_env = Some("custom:insar_py".to_string());
        prefs.rtk_custom_path = Some(mock_rtk.to_string_lossy().to_string());
        prefs.dehydrator_mode = Some("auto".to_string());

        let saved = save_external_gis_prefs(&app_data, &prefs).expect("save extended prefs");
        assert_eq!(
            saved.preferred_conda_env.as_deref(),
            Some("custom:insar_py")
        );
        assert_eq!(saved.dehydrator_mode.as_deref(), Some("auto"));

        let report =
            inspect_external_gis_toolchains(&runtime_root, &app_data, true).expect("inspect");
        assert_eq!(report.toolchains.len(), 4);
        assert!(report.log_dehydrator.builtin_ready);
        assert!(report.log_dehydrator.shared_rtk.installed);
        assert_eq!(
            report.log_dehydrator.shared_rtk.version.as_deref(),
            Some("rtk 0.25.0")
        );

        let discovered = report
            .conda_environments
            .iter()
            .find(|e| e.id == "custom:insar_py")
            .expect("custom conda env discovered through symlinked python3");
        assert!(discovered.verified);
        assert!(discovered.geo_packages.contains(&"rasterio".to_string()));
        assert!(discovered.geo_packages.contains(&"gdal".to_string()));
        assert!(discovered.geo_packages.contains(&"numpy".to_string()));

        let res = execute_external_gis_action(
            &runtime_root,
            &app_data,
            &ExternalGisExecuteRequest {
                toolchain: "conda".to_string(),
                action: "run_algorithm".to_string(),
                algorithm: Some("package_probe".to_string()),
                conda_env: Some("custom:insar_py".to_string()),
                ..ExternalGisExecuteRequest::default()
            },
        )
        .expect("execute controlled conda recipe");
        assert!(res.ok);
        assert!(res.stdout.contains("rasterio"));
        let sum = res.dehydration.expect("dehydration summary attached");
        assert_eq!(sum.engine, "builtin_gis_dehydrator+shared_rtk");
    }

    #[cfg(unix)]
    #[test]
    fn execute_external_gis_action_dehydrates_noisy_stderr_and_retains_tail_error() {
        let dir = TestTempDir::new();
        let app_data = dir.path().join("appdata");
        fs::create_dir_all(&app_data).unwrap();
        let runtime_root = dir.path().join("runtime_root");
        fs::create_dir_all(&runtime_root).unwrap();

        let noisy_qgis = write_executable_stub(
            dir.path(),
            "noisy-qgis",
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "QGIS 4.2.3"
  exit 0
fi
i=0
while [ "$i" -lt 15 ]; do
  printf '/opt/py/mod.py:%d: FutureWarning: deprecated crs hint\n' "$i" >&2
  i=$((i+1))
done
printf '10%%|█|\r50%%|████|\r100%%|████████|\n' >&2
echo 'Traceback (most recent call last):' >&2
echo 'RuntimeError: final_gdal_clip_failure_tail' >&2
exit 2
"#,
        );

        let mut prefs = ExternalGisPrefs::default();
        prefs
            .custom_paths
            .insert("qgis".to_string(), noisy_qgis.to_string_lossy().to_string());
        prefs.dehydrator_mode = Some("builtin_only".to_string());
        save_external_gis_prefs(&app_data, &prefs).expect("save prefs");

        let err = execute_external_gis_action(
            &runtime_root,
            &app_data,
            &ExternalGisExecuteRequest {
                toolchain: "qgis".to_string(),
                action: "run_algorithm".to_string(),
                algorithm: Some("native:clip".to_string()),
                ..ExternalGisExecuteRequest::default()
            },
        )
        .expect_err("non-zero exit propagates dehydrated tail traceback");
        assert!(err.contains("已折叠 14 条同类重复告警"), "{err}");
        assert!(
            err.contains("RuntimeError: final_gdal_clip_failure_tail"),
            "{err}"
        );
        assert!(!err.contains("10%|█"), "{err}");
    }

    #[test]
    fn curated_conda_geo_installer_requires_explicit_confirmation_and_allowlists_packages() {
        let dir = TestTempDir::new();
        let runtime_root = dir.0.join("rt");
        let app_data = dir.0.join("app");
        std::fs::create_dir_all(&runtime_root).unwrap();
        std::fs::create_dir_all(&app_data).unwrap();

        let unconfirmed = install_conda_geo_package(
            &runtime_root,
            &app_data,
            &CondaGeoInstallRequest {
                package_id: "mintpy".into(),
                conda_env: None,
                confirmed: false,
            },
        )
        .expect_err("must reject unconfirmed package installation");
        assert!(
            unconfirmed.contains("CONDA_INSTALL_CONFIRMATION_REQUIRED"),
            "{unconfirmed}"
        );

        let unknown = install_conda_geo_package(
            &runtime_root,
            &app_data,
            &CondaGeoInstallRequest {
                package_id: "rm -rf /".into(),
                conda_env: None,
                confirmed: true,
            },
        )
        .expect_err("must reject unknown package");
        assert!(
            unknown.contains("CONDA_UNSUPPORTED_GEO_PACKAGE"),
            "{unknown}"
        );
    }

    #[test]
    fn external_gis_prefs_persists_extension_mirror_and_migrates_legacy_cloudflare() {
        let dir = TestTempDir::new();
        let app_data = dir.0.join("app");
        std::fs::create_dir_all(&app_data).unwrap();

        let github_url =
            "https://raw.githubusercontent.com/hhanmj/eostage-hub/main/registry.json".to_string();
        let prefs = ExternalGisPrefs {
            extension_mirror_preset: Some("github".to_string()),
            extension_registry_url: Some(github_url.clone()),
            ..ExternalGisPrefs::default()
        };

        let saved = save_external_gis_prefs(&app_data, &prefs).expect("save github mirror prefs");
        assert_eq!(saved.extension_mirror_preset.as_deref(), Some("github"));
        assert_eq!(
            saved.extension_registry_url.as_deref(),
            Some(github_url.as_str())
        );

        let reloaded = load_external_gis_prefs(&app_data).expect("reload mirror prefs");
        assert_eq!(reloaded.extension_mirror_preset.as_deref(), Some("github"));
        assert_eq!(
            reloaded.extension_registry_url.as_deref(),
            Some(github_url.as_str())
        );

        // Saving unrelated prefs with omitted mirror fields must preserve existing mirror choice
        let partial_save = ExternalGisPrefs {
            dehydrator_mode: Some("builtin_only".to_string()),
            ..ExternalGisPrefs::default()
        };
        let preserved =
            save_external_gis_prefs(&app_data, &partial_save).expect("preserve mirror on save");
        assert_eq!(preserved.extension_mirror_preset.as_deref(), Some("github"));
        assert_eq!(
            preserved.extension_registry_url.as_deref(),
            Some(github_url.as_str())
        );

        // Legacy Cloudflare config auto-migrates to GitCode on both load and save
        std::fs::write(
            app_data.join(PREFS_FILE_NAME),
            r#"{"extension_mirror_preset":"cloudflare","extension_registry_url":"https://eostage-hub.pages.dev/registry.json"}"#,
        )
        .unwrap();
        let migrated_load =
            load_external_gis_prefs(&app_data).expect("legacy cloudflare migrates on load");
        assert_eq!(
            migrated_load.extension_mirror_preset.as_deref(),
            Some("gitcode")
        );
        assert_eq!(
            migrated_load.extension_registry_url.as_deref(),
            Some(crate::application::DEFAULT_EXTENSION_REGISTRY_URL)
        );
    }

    #[test]
    fn arcpy_default_output_param_maps_raster_vector_and_table_tools() {
        assert_eq!(
            super::arcpy_default_output_param_name("management.CopyRaster"),
            "out_rasterdataset"
        );
        assert_eq!(
            super::arcpy_default_output_param_name("arcpy.management.ProjectRaster"),
            "out_rasterdataset"
        );
        assert_eq!(
            super::arcpy_default_output_param_name("analysis.Buffer"),
            "out_feature_class"
        );
        assert_eq!(
            super::arcpy_default_output_param_name("management.CopyRows"),
            "out_table"
        );
        assert_eq!(
            super::arcpy_default_output_param_name("management.Clip"),
            "out_raster"
        );
        assert_eq!(
            super::arcpy_default_output_param_name("analysis.Clip"),
            "out_feature_class"
        );
        assert_eq!(
            super::arcpy_default_output_param_name("conversion.RasterToPolygon"),
            "out_polygon_features"
        );
        assert_eq!(
            super::arcpy_default_output_param_name("management.CreateTable"),
            "out_path+out_name"
        );
        assert_eq!(
            super::arcpy_default_output_param_name("conversion.TableToTable"),
            "out_path+out_name"
        );
        assert_eq!(
            super::arcpy_default_output_param_name("management.CreateFeatureclass"),
            "out_path+out_name"
        );
        assert_eq!(
            super::arcpy_default_output_param_name("management.MosaicToNewRaster"),
            "output_location+raster_dataset_name_with_extension"
        );
    }
}
