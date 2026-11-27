//! Research-only ISCE3 geometry gate for controlled NISAR live acceptance.
//!
//! The production worker remains independent of Python, WSL, and ISCE3. This
//! module is compiled only with the explicit live-acceptance feature. It emits
//! a small raw-geometry package from the already validator-bound metadata
//! workspace and admits science Range requests only after a separately run
//! ISCE3 oracle result proves that the production window contains the
//! independent reference window.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use hdf5::types::{TypeDescriptor, VarLenAscii, VarLenUnicode};
use hdf5::Dataset;
use insar_core::{NisarSubsetFrequency, NisarSubsetRequestManifestV1};
use insar_provider::asf::nisar_subset::normalize_nisar_subset_aoi;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::nisar_subset_writer::{nisar_science_root_for_oracle, NisarSubsetMetadataSnapshotV1};

const INPUT_SCHEMA: &str = "NisarIsce3Geo2rdrOracleInputV1";
const OUTPUT_SCHEMA: &str = "NisarIsce3Geo2rdrOracleOutputV1";
const COMPARISON_SCHEMA: &str = "NisarStudioIsce3GeometryComparisonV1";
const SAMPLE_STRATEGY: &str = "polygon_boundary_interior_lattice_v1";
const REQUIRED_ISCE3_VERSION: &str = "0.25.12";
const SPEED_OF_LIGHT_MPS: f64 = 299_792_458.0;
const MAX_ORACLE_JSON_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct NisarIsce3LiveGeometryGateV1 {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub comparison_path: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NisarIsce3GeometryGateError {
    InputInvalid,
    InputWriteFailed,
    OracleRequired,
    OracleInvalid,
    Undercoverage,
}

impl NisarIsce3GeometryGateError {
    pub(crate) const fn code(self) -> &'static str {
        match self {
            Self::InputInvalid => "NISAR_ISCE3_ORACLE_INPUT_INVALID",
            Self::InputWriteFailed => "NISAR_ISCE3_ORACLE_INPUT_WRITE_FAILED",
            Self::OracleRequired => "NISAR_ISCE3_ORACLE_REQUIRED",
            Self::OracleInvalid => "NISAR_ISCE3_ORACLE_INVALID",
            Self::Undercoverage => "NISAR_ISCE3_REFERENCE_UNDERCOVERED",
        }
    }
}

#[derive(Debug, Serialize)]
struct OracleInputV1 {
    schema: &'static str,
    granule_id: String,
    aoi: Value,
    aoi_sha256: String,
    sample_strategy: SampleStrategyV1,
    selection: SelectionV1,
    geometry: GeometryV1,
    validation_nodes: Vec<ValidationNodeV1>,
}

#[derive(Debug, Serialize)]
struct SampleStrategyV1 {
    kind: &'static str,
    max_segment_degrees: f64,
    heights_m: Vec<f64>,
}

#[derive(Debug, Serialize)]
struct SelectionV1 {
    frequency: String,
    polarization: String,
    swath: String,
}

#[derive(Debug, Serialize)]
struct GeometryV1 {
    orbit: OrbitV1,
    radar_grid: RadarGridV1,
}

#[derive(Debug, Serialize)]
struct OrbitV1 {
    reference_epoch: String,
    time_s: Vec<f64>,
    position_ecef_m: Vec<[f64; 3]>,
    velocity_ecef_mps: Vec<[f64; 3]>,
}

#[derive(Debug, Serialize)]
struct RadarGridV1 {
    zero_doppler_time_s: Vec<f64>,
    slant_range_m: Vec<f64>,
    line_count: usize,
    sample_count: usize,
    sensing_start_s: f64,
    sensing_stop_s: f64,
    wavelength_m: f64,
    look_side: String,
}

#[derive(Clone, Debug, Serialize)]
struct ValidationNodeV1 {
    lon: f64,
    lat: f64,
    height_m: f64,
    expected_zero_doppler_time_s: f64,
    expected_slant_range_m: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OracleOutputV1 {
    schema: String,
    granule_id: String,
    aoi_sha256: String,
    isce3_version: String,
    sample_strategy: Value,
    reference_window: ReferenceWindowV1,
    success_point_count: u64,
    failure_point_count: u64,
    residual: ResidualV1,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReferenceWindowV1 {
    line_start: u64,
    line_end_exclusive: u64,
    sample_start: u64,
    sample_end_exclusive: u64,
    target_count: u64,
}

#[derive(Debug, Deserialize, Serialize)]
struct ResidualV1 {
    status: String,
    #[serde(default)]
    max_abs_azimuth_time_error_seconds: Option<f64>,
    #[serde(default)]
    max_abs_slant_range_error_m: Option<f64>,
    #[serde(default)]
    validation_node_count: u64,
}

#[derive(Debug, Serialize)]
struct GeometryComparisonV1<'a> {
    schema: &'static str,
    granule_id: &'a str,
    aoi_sha256: &'a str,
    isce3_version: &'a str,
    isce3_reference_window: ReferenceWindowV1,
    studio_window: StudioWindowV1,
    contains_reference: bool,
    line_halo: AxisHaloV1,
    sample_halo: AxisHaloV1,
    line_overcoverage_ratio: f64,
    sample_overcoverage_ratio: f64,
    area_ratio: f64,
    success_point_count: u64,
    failure_point_count: u64,
    residual: &'a ResidualV1,
    oracle_sample_strategy: &'a Value,
}

#[derive(Clone, Copy, Debug, Serialize)]
struct StudioWindowV1 {
    line_start: u64,
    line_end_exclusive: u64,
    sample_start: u64,
    sample_end_exclusive: u64,
}

#[derive(Clone, Copy, Debug, Serialize)]
struct AxisHaloV1 {
    before: u64,
    after: u64,
}

pub(crate) fn enforce_isce3_geometry_gate(
    gate: &NisarIsce3LiveGeometryGateV1,
    request: &NisarSubsetRequestManifestV1,
    metadata: &NisarSubsetMetadataSnapshotV1,
    sparse_source_path: &Path,
    aoi: &Map<String, Value>,
) -> Result<(), NisarIsce3GeometryGateError> {
    let input = build_input(request, metadata, sparse_source_path, aoi)?;
    write_json_atomic(&gate.input_path, &input)?;
    if !gate.output_path.is_file() {
        return Err(NisarIsce3GeometryGateError::OracleRequired);
    }
    let bytes =
        fs::read(&gate.output_path).map_err(|_| NisarIsce3GeometryGateError::OracleInvalid)?;
    if bytes.is_empty() || bytes.len() > MAX_ORACLE_JSON_BYTES {
        return Err(NisarIsce3GeometryGateError::OracleInvalid);
    }
    let output: OracleOutputV1 =
        serde_json::from_slice(&bytes).map_err(|_| NisarIsce3GeometryGateError::OracleInvalid)?;
    validate_output(request, &output)?;
    let window = request
        .mapping
        .windows
        .iter()
        .find(|window| {
            window.frequency == request.request.frequency
                && window.polarization == request.request.polarization
                && window.swath == request.request.swath
        })
        .ok_or(NisarIsce3GeometryGateError::InputInvalid)?;
    let studio = StudioWindowV1 {
        line_start: window.line_start,
        line_end_exclusive: window.line_end_exclusive,
        sample_start: window.sample_start,
        sample_end_exclusive: window.sample_end_exclusive,
    };
    let reference = output.reference_window;
    let contains = studio.line_start <= reference.line_start
        && studio.line_end_exclusive >= reference.line_end_exclusive
        && studio.sample_start <= reference.sample_start
        && studio.sample_end_exclusive >= reference.sample_end_exclusive;
    let reference_lines = reference.line_end_exclusive - reference.line_start;
    let reference_samples = reference.sample_end_exclusive - reference.sample_start;
    let studio_lines = studio.line_end_exclusive - studio.line_start;
    let studio_samples = studio.sample_end_exclusive - studio.sample_start;
    let comparison = GeometryComparisonV1 {
        schema: COMPARISON_SCHEMA,
        granule_id: &output.granule_id,
        aoi_sha256: &output.aoi_sha256,
        isce3_version: &output.isce3_version,
        isce3_reference_window: reference,
        studio_window: studio,
        contains_reference: contains,
        line_halo: AxisHaloV1 {
            before: reference.line_start.saturating_sub(studio.line_start),
            after: studio
                .line_end_exclusive
                .saturating_sub(reference.line_end_exclusive),
        },
        sample_halo: AxisHaloV1 {
            before: reference.sample_start.saturating_sub(studio.sample_start),
            after: studio
                .sample_end_exclusive
                .saturating_sub(reference.sample_end_exclusive),
        },
        line_overcoverage_ratio: studio_lines as f64 / reference_lines as f64,
        sample_overcoverage_ratio: studio_samples as f64 / reference_samples as f64,
        area_ratio: (studio_lines as f64 * studio_samples as f64)
            / (reference_lines as f64 * reference_samples as f64),
        success_point_count: output.success_point_count,
        failure_point_count: output.failure_point_count,
        residual: &output.residual,
        oracle_sample_strategy: &output.sample_strategy,
    };
    write_json_atomic(&gate.comparison_path, &comparison)?;
    if !contains {
        return Err(NisarIsce3GeometryGateError::Undercoverage);
    }
    Ok(())
}

fn build_input(
    request: &NisarSubsetRequestManifestV1,
    metadata: &NisarSubsetMetadataSnapshotV1,
    sparse_source_path: &Path,
    aoi: &Map<String, Value>,
) -> Result<OracleInputV1, NisarIsce3GeometryGateError> {
    let evidence = metadata.oracle_evidence();
    if metadata.request_manifest_sha256()
        != request
            .sha256()
            .map_err(|_| NisarIsce3GeometryGateError::InputInvalid)?
        || evidence.source_granule_id
            != request
                .source
                .file_name
                .strip_suffix(".h5")
                .ok_or(NisarIsce3GeometryGateError::InputInvalid)?
        || evidence.geolocation_epsg != 4326
    {
        return Err(NisarIsce3GeometryGateError::InputInvalid);
    }
    let science_root = nisar_science_root_for_oracle(&request.request.swath)
        .ok_or(NisarIsce3GeometryGateError::InputInvalid)?;
    let orbit_path = format!("{science_root}/RSLC/metadata/orbit");
    let identification_path = format!("{science_root}/identification");
    let swaths_path = format!("{science_root}/RSLC/swaths");
    let file = crate::hdf5_file::open(sparse_source_path)
        .map_err(|_| NisarIsce3GeometryGateError::InputInvalid)?;
    let orbit_time = file
        .dataset(&format!("{orbit_path}/time"))
        .map_err(|_| NisarIsce3GeometryGateError::InputInvalid)?;
    let radar_time = file
        .dataset(&format!("{swaths_path}/zeroDopplerTime"))
        .map_err(|_| NisarIsce3GeometryGateError::InputInvalid)?;
    let orbit_epoch = reference_epoch_from_units(&read_ascii_attribute(&orbit_time, "units")?)?;
    let radar_epoch = reference_epoch_from_units(&read_ascii_attribute(&radar_time, "units")?)?;
    if orbit_epoch != radar_epoch {
        return Err(NisarIsce3GeometryGateError::InputInvalid);
    }
    let look_side = read_fixed_ascii_dataset(
        &file
            .dataset(&format!("{identification_path}/lookDirection"))
            .map_err(|_| NisarIsce3GeometryGateError::InputInvalid)?,
    )?;
    let look_side = look_side.to_ascii_uppercase();
    if !matches!(look_side.as_str(), "LEFT" | "RIGHT") {
        return Err(NisarIsce3GeometryGateError::InputInvalid);
    }
    let frequency = match request.request.frequency {
        NisarSubsetFrequency::A => "A",
        NisarSubsetFrequency::B => "B",
    };
    let center_frequency = file
        .dataset(&format!(
            "{swaths_path}/frequency{frequency}/processedCenterFrequency"
        ))
        .and_then(|dataset| dataset.read_scalar::<f64>())
        .map_err(|_| NisarIsce3GeometryGateError::InputInvalid)?;
    let wavelength = SPEED_OF_LIGHT_MPS / center_frequency;
    let slant_range = evidence
        .slant_ranges
        .get(frequency)
        .cloned()
        .ok_or(NisarIsce3GeometryGateError::InputInvalid)?;
    let sensing_start = *evidence
        .zero_doppler_time
        .first()
        .ok_or(NisarIsce3GeometryGateError::InputInvalid)?;
    let sensing_stop = *evidence
        .zero_doppler_time
        .last()
        .ok_or(NisarIsce3GeometryGateError::InputInvalid)?;
    if !center_frequency.is_finite()
        || center_frequency <= 0.0
        || !wavelength.is_finite()
        || wavelength <= 0.0
        || sensing_start >= sensing_stop
    {
        return Err(NisarIsce3GeometryGateError::InputInvalid);
    }
    let nodes = validation_nodes(metadata, &request.request.swath)?;
    let normalized_aoi =
        normalize_nisar_subset_aoi(aoi).map_err(|_| NisarIsce3GeometryGateError::InputInvalid)?;
    if normalized_aoi.sha256 != request.request.aoi_sha256 {
        return Err(NisarIsce3GeometryGateError::InputInvalid);
    }
    let aoi_value = Value::Object(normalized_aoi.canonical_geojson);
    Ok(OracleInputV1 {
        schema: INPUT_SCHEMA,
        granule_id: evidence.source_granule_id.clone(),
        aoi: aoi_value,
        aoi_sha256: request.request.aoi_sha256.clone(),
        sample_strategy: SampleStrategyV1 {
            kind: SAMPLE_STRATEGY,
            max_segment_degrees: 0.005,
            heights_m: evidence.height_above_ellipsoid.clone(),
        },
        selection: SelectionV1 {
            frequency: frequency.to_owned(),
            polarization: request.request.polarization.clone(),
            swath: request.request.swath.clone(),
        },
        geometry: GeometryV1 {
            orbit: OrbitV1 {
                reference_epoch: orbit_epoch,
                time_s: evidence.orbit_time.clone(),
                position_ecef_m: evidence.orbit_position.clone(),
                velocity_ecef_mps: evidence.orbit_velocity.clone(),
            },
            radar_grid: RadarGridV1 {
                zero_doppler_time_s: evidence.zero_doppler_time.clone(),
                slant_range_m: slant_range.clone(),
                line_count: evidence.zero_doppler_time.len(),
                sample_count: slant_range.len(),
                sensing_start_s: sensing_start,
                sensing_stop_s: sensing_stop,
                wavelength_m: wavelength,
                look_side,
            },
        },
        validation_nodes: nodes,
    })
}

fn validation_nodes(
    metadata: &NisarSubsetMetadataSnapshotV1,
    swath: &str,
) -> Result<Vec<ValidationNodeV1>, NisarIsce3GeometryGateError> {
    let evidence = metadata.oracle_evidence();
    let [height_count, line_count, sample_count] = evidence.geolocation_shape;
    if height_count < 3 || line_count < 3 || sample_count < 3 {
        return Err(NisarIsce3GeometryGateError::InputInvalid);
    }
    let geolocation_time = metadata
        .oracle_geolocation_axis(swath, true)
        .map_err(|_| NisarIsce3GeometryGateError::InputInvalid)?;
    let geolocation_range = metadata
        .oracle_geolocation_axis(swath, false)
        .map_err(|_| NisarIsce3GeometryGateError::InputInvalid)?;
    if geolocation_time.len() != line_count || geolocation_range.len() != sample_count {
        return Err(NisarIsce3GeometryGateError::InputInvalid);
    }
    let layer_indices = [0, height_count / 2, height_count - 1];
    let node_indices = [
        (line_count / 4, sample_count / 4),
        (line_count / 2, sample_count / 2),
        (3 * line_count / 4, 3 * sample_count / 4),
    ];
    let mut nodes = Vec::with_capacity(9);
    for layer in layer_indices {
        for (line, sample) in node_indices {
            let index = layer
                .checked_mul(line_count)
                .and_then(|value| value.checked_add(line))
                .and_then(|value| value.checked_mul(sample_count))
                .and_then(|value| value.checked_add(sample))
                .ok_or(NisarIsce3GeometryGateError::InputInvalid)?;
            nodes.push(ValidationNodeV1 {
                lon: *evidence
                    .geolocation_x
                    .get(index)
                    .ok_or(NisarIsce3GeometryGateError::InputInvalid)?,
                lat: *evidence
                    .geolocation_y
                    .get(index)
                    .ok_or(NisarIsce3GeometryGateError::InputInvalid)?,
                height_m: evidence.height_above_ellipsoid[layer],
                expected_zero_doppler_time_s: geolocation_time[line],
                expected_slant_range_m: geolocation_range[sample],
            });
        }
    }
    Ok(nodes)
}

fn validate_output(
    request: &NisarSubsetRequestManifestV1,
    output: &OracleOutputV1,
) -> Result<(), NisarIsce3GeometryGateError> {
    let expected_granule = request
        .source
        .file_name
        .strip_suffix(".h5")
        .ok_or(NisarIsce3GeometryGateError::OracleInvalid)?;
    let window = output.reference_window;
    let residual_valid = output.residual.status == "compared_to_declared_raw_validation_nodes"
        && output.residual.validation_node_count == 9
        && output
            .residual
            .max_abs_azimuth_time_error_seconds
            .is_some_and(|value| value.is_finite() && value <= 1.0e-7)
        && output
            .residual
            .max_abs_slant_range_error_m
            .is_some_and(|value| value.is_finite() && value <= 1.0e-3);
    if output.schema != OUTPUT_SCHEMA
        || output.granule_id != expected_granule
        || output.aoi_sha256 != request.request.aoi_sha256
        || output.isce3_version != REQUIRED_ISCE3_VERSION
        || output.success_point_count == 0
        || window.target_count != output.success_point_count
        || window.line_start >= window.line_end_exclusive
        || window.sample_start >= window.sample_end_exclusive
        || output.failure_point_count > output.success_point_count
        || !residual_valid
    {
        return Err(NisarIsce3GeometryGateError::OracleInvalid);
    }
    Ok(())
}

fn read_ascii_attribute(
    dataset: &Dataset,
    name: &str,
) -> Result<String, NisarIsce3GeometryGateError> {
    let attribute = dataset
        .attr(name)
        .map_err(|_| NisarIsce3GeometryGateError::InputInvalid)?;
    let descriptor = attribute
        .dtype()
        .and_then(|datatype| datatype.to_descriptor())
        .map_err(|_| NisarIsce3GeometryGateError::InputInvalid)?;
    let value = match descriptor {
        TypeDescriptor::FixedAscii(capacity) | TypeDescriptor::FixedUnicode(capacity) => {
            if capacity == 0 || capacity > 512 {
                return Err(NisarIsce3GeometryGateError::InputInvalid);
            }
            let mut bytes = vec![0_u8; capacity];
            let datatype = attribute
                .dtype()
                .map_err(|_| NisarIsce3GeometryGateError::InputInvalid)?;
            let status = hdf5::sync::sync(|| unsafe {
                hdf5_sys::h5a::H5Aread(attribute.id(), datatype.id(), bytes.as_mut_ptr().cast())
            });
            if status < 0 {
                return Err(NisarIsce3GeometryGateError::InputInvalid);
            }
            let end = bytes
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(bytes.len());
            String::from_utf8(bytes[..end].to_vec())
                .map_err(|_| NisarIsce3GeometryGateError::InputInvalid)?
        }
        TypeDescriptor::VarLenAscii => attribute
            .read_scalar::<VarLenAscii>()
            .map(|value| value.as_str().to_owned())
            .map_err(|_| NisarIsce3GeometryGateError::InputInvalid)?,
        TypeDescriptor::VarLenUnicode => attribute
            .read_scalar::<VarLenUnicode>()
            .map(|value| value.as_str().to_owned())
            .map_err(|_| NisarIsce3GeometryGateError::InputInvalid)?,
        _ => return Err(NisarIsce3GeometryGateError::InputInvalid),
    };
    if value.is_empty() || value.len() > 512 || value.contains('\0') {
        return Err(NisarIsce3GeometryGateError::InputInvalid);
    }
    Ok(value)
}

fn read_fixed_ascii_dataset(dataset: &Dataset) -> Result<String, NisarIsce3GeometryGateError> {
    let descriptor = dataset
        .dtype()
        .and_then(|datatype| datatype.to_descriptor())
        .map_err(|_| NisarIsce3GeometryGateError::InputInvalid)?;
    let TypeDescriptor::FixedAscii(capacity) = descriptor else {
        return Err(NisarIsce3GeometryGateError::InputInvalid);
    };
    if capacity == 0 || capacity > 512 {
        return Err(NisarIsce3GeometryGateError::InputInvalid);
    }
    let mut bytes = vec![0_u8; capacity];
    let datatype = dataset
        .dtype()
        .map_err(|_| NisarIsce3GeometryGateError::InputInvalid)?;
    let status = hdf5::sync::sync(|| unsafe {
        hdf5_sys::h5d::H5Dread(
            dataset.id(),
            datatype.id(),
            hdf5_sys::h5s::H5S_ALL,
            hdf5_sys::h5s::H5S_ALL,
            hdf5_sys::h5p::H5P_DEFAULT,
            bytes.as_mut_ptr().cast(),
        )
    });
    if status < 0 {
        return Err(NisarIsce3GeometryGateError::InputInvalid);
    }
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    String::from_utf8(bytes[..end].to_vec()).map_err(|_| NisarIsce3GeometryGateError::InputInvalid)
}

fn reference_epoch_from_units(units: &str) -> Result<String, NisarIsce3GeometryGateError> {
    let normalized = units.trim();
    let prefix = "seconds since ";
    if normalized.len() <= prefix.len() || !normalized[..prefix.len()].eq_ignore_ascii_case(prefix)
    {
        return Err(NisarIsce3GeometryGateError::InputInvalid);
    }
    let mut raw_epoch = normalized[prefix.len()..].trim().to_owned();
    if raw_epoch.to_ascii_uppercase().ends_with("UTC") {
        raw_epoch.truncate(raw_epoch.len() - 3);
        raw_epoch = raw_epoch.trim_end().to_owned();
    }
    let mut epoch = raw_epoch.replacen(' ', "T", 1);
    if !epoch.ends_with('Z') {
        epoch.push('Z');
    }
    if epoch.len() < 20 || !epoch.is_ascii() {
        return Err(NisarIsce3GeometryGateError::InputInvalid);
    }
    Ok(epoch)
}

fn write_json_atomic(
    path: &Path,
    value: &impl Serialize,
) -> Result<(), NisarIsce3GeometryGateError> {
    let parent = path
        .parent()
        .ok_or(NisarIsce3GeometryGateError::InputWriteFailed)?;
    fs::create_dir_all(parent).map_err(|_| NisarIsce3GeometryGateError::InputWriteFailed)?;
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|_| NisarIsce3GeometryGateError::InputWriteFailed)?;
    if bytes.is_empty() || bytes.len() > MAX_ORACLE_JSON_BYTES {
        return Err(NisarIsce3GeometryGateError::InputWriteFailed);
    }
    let temporary = path.with_extension("json.part");
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temporary)
        .map_err(|_| NisarIsce3GeometryGateError::InputWriteFailed)?;
    file.write_all(&bytes)
        .and_then(|_| file.write_all(b"\n"))
        .and_then(|_| file.sync_all())
        .map_err(|_| NisarIsce3GeometryGateError::InputWriteFailed)?;
    drop(file);
    fs::rename(&temporary, path).map_err(|_| NisarIsce3GeometryGateError::InputWriteFailed)
}

#[cfg(test)]
mod tests {
    use crate::nisar_subset_writer::nisar_science_root_for_oracle;

    #[test]
    fn oracle_science_root_is_closed_to_reviewed_rslc_swaths() {
        assert_eq!(
            nisar_science_root_for_oracle("LSAR_RSLC"),
            Some("/science/LSAR")
        );
        assert_eq!(
            nisar_science_root_for_oracle("SSAR_RSLC"),
            Some("/science/SSAR")
        );
        assert_eq!(nisar_science_root_for_oracle("SLC"), None);
        assert_eq!(nisar_science_root_for_oracle(""), None);
    }
}
