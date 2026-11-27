use std::collections::{BTreeMap, BTreeSet};

use insar_core::{ContractError, ProviderResult};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use url::Url;

const MAX_RAW_UMM_BYTES: usize = 2 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 512;
const MAX_ADDITIONAL_ATTRIBUTES: usize = 128;
const MAX_ATTRIBUTE_VALUES: usize = 8;
const MAX_ARCHIVES: usize = 32;
const MAX_RELATED_URLS: usize = 128;
const MAX_RELATED_URL_BYTES: usize = 4_096;
const MAX_NISAR_RAW_RESOURCES: usize = 128;
const MAX_POLYGONS: usize = 64;
const MAX_POINTS_PER_POLYGON: usize = 4_096;
const NISAR_PROVISIONAL_SHORT_NAME: &str = "NISAR_L1_RSLC_PROVISIONAL_V1";
const NISAR_BETA_SHORT_NAME: &str = "NISAR_L1_RSLC_BETA_V1";

#[derive(Clone, Debug, PartialEq)]
pub struct CmrGranuleEnvelope {
    pub collection: Map<String, Value>,
    pub granule: Map<String, Value>,
}

impl TryFrom<&Value> for CmrGranuleEnvelope {
    type Error = ContractError;

    fn try_from(value: &Value) -> ProviderResult<Self> {
        if serde_json::to_vec(value)
            .map_err(|_| envelope_error("serialization"))?
            .len()
            > MAX_RAW_UMM_BYTES
        {
            return Err(envelope_error("record_size"));
        }
        let root = value
            .as_object()
            .ok_or_else(|| envelope_error("granule_envelope"))?;
        match (
            root.get("collection").and_then(Value::as_object),
            root.get("granule").and_then(Value::as_object),
        ) {
            (Some(collection), Some(granule)) => Ok(Self {
                collection: collection.clone(),
                granule: granule.clone(),
            }),
            _ => normalize_raw_cmr_umm(root),
        }
    }
}

fn normalize_raw_cmr_umm(root: &Map<String, Value>) -> ProviderResult<CmrGranuleEnvelope> {
    let meta = object(root, "meta")?;
    let umm = object(root, "umm")?;
    if text(meta, "concept-type")? != "granule"
        || text(meta, "provider-id")? != "ASF"
        || text(meta, "format")? != "application/vnd.nasa.cmr.umm+json"
    {
        return Err(collection_error("cmr_meta"));
    }
    let concept_id = text(meta, "concept-id")?;
    let native_id = text(meta, "native-id")?;
    let collection_concept_id = text(meta, "collection-concept-id")?;
    let collection_reference = object(umm, "CollectionReference")?;
    let short_name = text(collection_reference, "ShortName")?;
    let version = text(collection_reference, "Version")?;
    let granule_ur = text(umm, "GranuleUR")?;
    if granule_ur != native_id {
        return Err(envelope_error("native_identity"));
    }

    if matches!(
        short_name.as_str(),
        NISAR_PROVISIONAL_SHORT_NAME | NISAR_BETA_SHORT_NAME
    ) {
        return normalize_raw_nisar_cmr_umm(
            umm,
            concept_id,
            native_id,
            collection_concept_id,
            short_name,
            version,
        );
    }

    let attributes = additional_attributes(umm)?;
    let processing_level = attribute_one(&attributes, "PROCESSING_LEVEL")?;
    let beam_mode = attribute_one(&attributes, "BEAM_MODE")?;
    let polarizations = attribute_many(&attributes, "POLARIZATION")?;
    let flight_direction = attribute_one(&attributes, "ASCENDING_DESCENDING")?;
    let look_direction = attribute_one(&attributes, "LOOK_DIRECTION")?;
    let operational_mode = attribute_one(&attributes, "OPERATIONAL_MODE")?;
    let path = attribute_u64(&attributes, "PATH_NUMBER")?;
    let frame = attribute_u64(&attributes, "FRAME_NUMBER")?;
    let off_nadir_angle = attribute_f64(&attributes, "OFF_NADIR_ANGLE")?;

    let temporal = object(umm, "TemporalExtent")?;
    let range = object(temporal, "RangeDateTime")?;
    let acquisition_start = text(range, "BeginningDateTime")?;
    let acquisition_end = text(range, "EndingDateTime")?;
    let orbit_domains = array(umm, "OrbitCalculatedSpatialDomains", 8)?;
    let [orbit_domain] = orbit_domains else {
        return Err(envelope_error("orbit"));
    };
    let orbit = orbit_domain
        .as_object()
        .and_then(|value| value.get("OrbitNumber"))
        .and_then(Value::as_u64)
        .ok_or_else(|| envelope_error("orbit"))?;
    let (platform, instrument, platform_look_direction) = platform_identity(umm)?;
    if platform_look_direction
        .as_deref()
        .is_some_and(|value| value != look_direction)
    {
        return Err(envelope_error("look_direction_conflict"));
    }

    let producer_granule_id = producer_granule_id(umm)?;
    let footprint = footprint(umm)?;
    let (assets, browse) = archive_assets(umm)?;
    let collection = Map::from_iter([
        (
            "concept_id".to_owned(),
            Value::String(collection_concept_id),
        ),
        ("short_name".to_owned(), Value::String(short_name)),
        ("version".to_owned(), Value::String(version)),
        (
            "cmr_provider_id".to_owned(),
            Value::String("ASF".to_owned()),
        ),
    ]);
    let granule = Map::from_iter([
        ("concept_id".to_owned(), Value::String(concept_id)),
        ("native_id".to_owned(), Value::String(native_id)),
        (
            "producer_granule_id".to_owned(),
            Value::String(producer_granule_id),
        ),
        ("platform".to_owned(), Value::String(platform)),
        ("instrument".to_owned(), Value::String(instrument)),
        (
            "processing_level".to_owned(),
            Value::String(processing_level),
        ),
        ("beam_mode".to_owned(), Value::String(beam_mode)),
        (
            "operational_mode".to_owned(),
            Value::String(operational_mode),
        ),
        (
            "polarizations".to_owned(),
            Value::Array(polarizations.into_iter().map(Value::String).collect()),
        ),
        (
            "acquisition_start_utc".to_owned(),
            Value::String(acquisition_start),
        ),
        (
            "acquisition_end_utc".to_owned(),
            Value::String(acquisition_end),
        ),
        (
            "flight_direction".to_owned(),
            Value::String(flight_direction),
        ),
        ("look_direction".to_owned(), Value::String(look_direction)),
        ("off_nadir_angle".to_owned(), Value::from(off_nadir_angle)),
        ("orbit".to_owned(), Value::from(orbit)),
        ("path".to_owned(), Value::from(path)),
        ("frame".to_owned(), Value::from(frame)),
        ("footprint".to_owned(), Value::Object(footprint)),
        ("assets".to_owned(), Value::Array(assets)),
        ("browse".to_owned(), browse),
    ]);
    Ok(CmrGranuleEnvelope {
        collection,
        granule,
    })
}

fn normalize_raw_nisar_cmr_umm(
    umm: &Map<String, Value>,
    concept_id: String,
    native_id: String,
    collection_concept_id: String,
    short_name: String,
    version: String,
) -> ProviderResult<CmrGranuleEnvelope> {
    let attributes = additional_attributes(umm)?;
    let processing_level = attribute_one(&attributes, "PRODUCT_TYPE")?;
    let science_level = attribute_one(&attributes, "PROCESSING_LEVEL")?;
    let production_configuration = attribute_one(&attributes, "PRODUCTION_PIPELINE")?;
    let frame_coverage = match attribute_one(&attributes, "FULL_FRAME")?.as_str() {
        "TRUE" => "FULL",
        "FALSE" => "PARTIAL",
        _ => return Err(envelope_error("full_frame")),
    };
    let frequency_a = attribute_optional_one(&attributes, "FREQUENCY_A_POLARIZATION_CONCAT")?;
    let frequency_b = attribute_optional_one(&attributes, "FREQUENCY_B_POLARIZATION_CONCAT")?;
    let declared_frequencies = attribute_many(&attributes, "FREQUENCIES")?
        .into_iter()
        .collect::<BTreeSet<_>>();
    if declared_frequencies.is_empty()
        || declared_frequencies
            .iter()
            .any(|value| !matches!(value.as_str(), "A" | "B"))
        || declared_frequencies.contains("A") != frequency_a.is_some()
        || declared_frequencies.contains("B") != frequency_b.is_some()
    {
        return Err(envelope_error("frequency_declaration"));
    }
    let range_bandwidth = attribute_one(&attributes, "RANGE_BANDWIDTH_CONCAT")?;
    let joint_observation = attribute_bool(&attributes, "JOINT_OBSERVATION")?;
    let flight_direction = attribute_one(&attributes, "ASCENDING_DESCENDING")?;
    let track = attribute_u64(&attributes, "TRACK_NUMBER")?;
    let frame = attribute_u64(&attributes, "FRAME_NUMBER")?;

    let temporal = object(umm, "TemporalExtent")?;
    let range = object(temporal, "RangeDateTime")?;
    let acquisition_start = text(range, "BeginningDateTime")?;
    let acquisition_end = text(range, "EndingDateTime")?;
    let (platform, instrument, look_direction) = platform_identity(umm)?;
    let look_direction = look_direction.ok_or_else(|| envelope_error("look_direction"))?;
    if platform != "NISAR" || instrument != "L-SAR" {
        return Err(envelope_error("nisar_platform_identity"));
    }

    let producer_granule_id = producer_granule_id(umm)?;
    if producer_granule_id != native_id {
        return Err(envelope_error("native_producer_identity"));
    }
    let crid = optional_granule_identifier(umm, "CRID")?;
    let pge_version = optional_pge_version(umm)?;
    let footprint = footprint(umm)?;
    let assets = nisar_archive_assets(umm, &producer_granule_id)?;
    let quality_status = if short_name == NISAR_BETA_SHORT_NAME {
        "Beta"
    } else {
        "Provisional"
    };

    let collection = Map::from_iter([
        (
            "concept_id".to_owned(),
            Value::String(collection_concept_id),
        ),
        ("short_name".to_owned(), Value::String(short_name)),
        ("version".to_owned(), Value::String(version)),
        (
            "cmr_provider_id".to_owned(),
            Value::String("ASF".to_owned()),
        ),
    ]);
    let mut granule = Map::from_iter([
        ("concept_id".to_owned(), Value::String(concept_id)),
        ("native_id".to_owned(), Value::String(native_id)),
        (
            "producer_granule_id".to_owned(),
            Value::String(producer_granule_id),
        ),
        ("platform".to_owned(), Value::String(platform)),
        ("instrument".to_owned(), Value::String(instrument)),
        (
            "processing_level".to_owned(),
            Value::String(processing_level),
        ),
        ("science_level".to_owned(), Value::String(science_level)),
        (
            "quality_status".to_owned(),
            Value::String(quality_status.to_owned()),
        ),
        (
            "production_configuration".to_owned(),
            Value::String(production_configuration),
        ),
        ("band".to_owned(), Value::String("L".to_owned())),
        (
            "acquisition_start_utc".to_owned(),
            Value::String(acquisition_start),
        ),
        (
            "acquisition_end_utc".to_owned(),
            Value::String(acquisition_end),
        ),
        (
            "frame_coverage".to_owned(),
            Value::String(frame_coverage.to_owned()),
        ),
        ("frame".to_owned(), Value::from(frame)),
        ("track".to_owned(), Value::from(track)),
        ("relative_orbit".to_owned(), Value::from(track)),
        (
            "flight_direction".to_owned(),
            Value::String(flight_direction),
        ),
        ("look_direction".to_owned(), Value::String(look_direction)),
        ("range_bandwidth".to_owned(), Value::String(range_bandwidth)),
        (
            "joint_observation".to_owned(),
            Value::Bool(joint_observation),
        ),
        ("footprint".to_owned(), Value::Object(footprint)),
        ("assets".to_owned(), assets),
    ]);
    if let Some(value) = frequency_a {
        granule.insert("frequency_a_polarization".to_owned(), Value::String(value));
    }
    if let Some(value) = frequency_b {
        granule.insert("frequency_b_polarization".to_owned(), Value::String(value));
    }
    if let Some(value) = crid {
        granule.insert("crid".to_owned(), Value::String(value));
    }
    if let Some(value) = pge_version {
        granule.insert("pge_version".to_owned(), Value::String(value));
    }
    Ok(CmrGranuleEnvelope {
        collection,
        granule,
    })
}

#[derive(Debug, Default)]
struct NisarFormatDiagnostics {
    related_url_count: usize,
    retained_official_related_url_count: usize,
    matched_archive_related_url_count: usize,
    ignored_granule_information_count: usize,
    ignored_service_api_count: usize,
    url_only_visualization_count: usize,
    url_only_metadata_count: usize,
    unmatched_data_route_count: usize,
    unknown_relation_count: usize,
    archive_count: usize,
    recognized_format_count: usize,
    unrecognized_format_count: usize,
    unrecognized_format_sha256: BTreeSet<String>,
    unrecognized_format_byte_count: BTreeSet<usize>,
    unknown_on_expected_primary: bool,
    unknown_on_hdf5_extension: bool,
    related_url_trigger: Option<NisarRelatedUrlDiagnostic>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NisarRelatedUrlClass {
    ArchiveDataRoute,
    ArchiveVisualizationRoute,
    ArchiveMetadataRoute,
    GranuleInformation,
    ServiceApi,
    UnsupportedOrUnknown,
}

impl NisarRelatedUrlClass {
    fn label(self) -> &'static str {
        match self {
            Self::ArchiveDataRoute => "archive_data_route",
            Self::ArchiveVisualizationRoute => "archive_visualization_route",
            Self::ArchiveMetadataRoute => "archive_metadata_route",
            Self::GranuleInformation => "granule_information",
            Self::ServiceApi => "service_api",
            Self::UnsupportedOrUnknown => "unsupported_or_unknown",
        }
    }
}

#[derive(Debug)]
struct NisarRelatedUrlDiagnostic {
    relation_class: &'static str,
    subtype_class: &'static str,
    basename_sha256: Option<String>,
    basename_byte_count: Option<usize>,
    basename_extension_family: &'static str,
    format_family: String,
    mime_family: String,
    official_host_class: &'static str,
    scheme_class: &'static str,
    archive_match: bool,
    hdf5_like_basename: bool,
}

struct NisarRelatedUrlContext<'a> {
    relation_class: NisarRelatedUrlClass,
    subtype: Option<&'a str>,
    url: &'a Url,
    basename: Option<&'a str>,
    formats: &'a BTreeSet<String>,
    mime: Option<&'a NisarFormatToken>,
    archive_match: bool,
}

struct NisarRelatedRoute<'a> {
    relation: String,
    subtype: Option<String>,
    formats: &'a BTreeSet<String>,
    unknown_sha256: BTreeSet<String>,
    unknown_byte_count: BTreeSet<usize>,
    scheme: String,
    locator: String,
}

#[derive(Debug)]
struct NisarFormatToken {
    family: Option<String>,
    unknown_sha256: Option<String>,
    byte_count: usize,
}

#[derive(Debug)]
struct RawNisarAsset {
    name: String,
    archive_name: Option<String>,
    format_types: BTreeSet<String>,
    mime_types: BTreeSet<String>,
    hdf5_evidence: bool,
    size_bytes: Option<u64>,
    checksum_algorithm: Option<String>,
    checksum_value: Option<String>,
    url_types: BTreeSet<String>,
    url_subtypes: BTreeSet<String>,
    formats: BTreeSet<String>,
    unrecognized_format_sha256: BTreeSet<String>,
    unrecognized_format_byte_count: BTreeSet<usize>,
    routes: BTreeMap<(String, String), Value>,
}

fn nisar_archive_assets(
    umm: &Map<String, Value>,
    producer_granule_id: &str,
) -> ProviderResult<Value> {
    let data_granule = object(umm, "DataGranule")?;
    let archives = nisar_array(
        data_granule,
        "ArchiveAndDistributionInformation",
        MAX_ARCHIVES,
        "archive_count",
    )?;
    if archives.is_empty() {
        return Err(envelope_error("archives"));
    }
    let related = nisar_array(umm, "RelatedUrls", MAX_RELATED_URLS, "related_url_count")?;
    let raw_resource_count = archives
        .len()
        .checked_add(related.len())
        .ok_or_else(|| asset_limit_error("raw_resource_count"))?;
    if raw_resource_count > MAX_NISAR_RAW_RESOURCES {
        return Err(asset_limit_error("raw_resource_count"));
    }

    let expected_primary = format!("{producer_granule_id}.h5");
    let mut diagnostics = NisarFormatDiagnostics {
        related_url_count: related.len(),
        archive_count: archives.len(),
        ..NisarFormatDiagnostics::default()
    };
    let mut assets = Vec::with_capacity(archives.len() + 1);
    let mut archive_indices = BTreeMap::new();
    let mut url_only_indices = BTreeMap::new();
    for archive in archives {
        let archive = archive
            .as_object()
            .ok_or_else(|| envelope_error("archive"))?;
        let name = nisar_bounded_text(
            archive.get("Name").and_then(Value::as_str),
            "archive_name",
            MAX_TEXT_BYTES,
        )?;
        if archive_indices.insert(name.clone(), assets.len()).is_some() {
            return Err(envelope_error("duplicate_archive_name"));
        }
        let format_token =
            parse_nisar_format_token(archive.get("Format"), "archive_format", &mut diagnostics)?;
        let format_type_token = parse_nisar_format_token(
            archive.get("FormatType"),
            "archive_format_type",
            &mut diagnostics,
        )?;
        let mime_token = parse_nisar_mime_token(
            archive.get("MimeType"),
            "archive_mime_type",
            &mut diagnostics,
        )?;
        if name == expected_primary
            && [
                format_token.as_ref(),
                format_type_token.as_ref(),
                mime_token.as_ref(),
            ]
            .iter()
            .flatten()
            .any(|token| token.family.is_none())
        {
            diagnostics.unknown_on_expected_primary = true;
        }
        if is_hdf5_like_name(&name)
            && [
                format_token.as_ref(),
                format_type_token.as_ref(),
                mime_token.as_ref(),
            ]
            .iter()
            .flatten()
            .any(|token| token.family.is_none())
        {
            diagnostics.unknown_on_hdf5_extension = true;
        }
        let mut formats = BTreeSet::new();
        add_nisar_family(
            &mut formats,
            format_token
                .as_ref()
                .and_then(|token| token.family.as_deref()),
            "archive_format_conflict",
        )
        .map_err(|_| archive_error("archive_format_conflict", &diagnostics))?;
        add_nisar_family(
            &mut formats,
            mime_token
                .as_ref()
                .and_then(|token| token.family.as_deref()),
            "archive_mime_conflict",
        )
        .map_err(|_| archive_error("archive_mime_conflict", &diagnostics))?;
        let format_types = format_type_token
            .as_ref()
            .and_then(|token| token.family.clone())
            .into_iter()
            .collect::<BTreeSet<_>>();
        let mime_types = mime_token
            .as_ref()
            .and_then(|token| token.family.as_deref())
            .map(nisar_media_type)
            .map(str::to_owned)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let hdf5_evidence = format_token
            .as_ref()
            .and_then(|token| token.family.as_deref())
            .is_some_and(|family| family == "HDF5")
            || mime_token
                .as_ref()
                .and_then(|token| token.family.as_deref())
                .is_some_and(|family| family == "HDF5");
        let (size_bytes, checksum_algorithm, checksum_value) = nisar_archive_integrity(archive)?;
        if name == expected_primary
            && hdf5_evidence
            && (size_bytes.is_none() || checksum_algorithm.is_none() || checksum_value.is_none())
        {
            return Err(primary_missing_error());
        }
        let (unknown_sha256, unknown_byte_count) = collect_unknown_nisar_tokens(&[
            format_token.as_ref(),
            format_type_token.as_ref(),
            mime_token.as_ref(),
        ]);
        assets.push(RawNisarAsset {
            name: name.clone(),
            archive_name: Some(name),
            format_types,
            mime_types,
            hdf5_evidence,
            size_bytes,
            checksum_algorithm,
            checksum_value,
            url_types: BTreeSet::new(),
            url_subtypes: BTreeSet::new(),
            formats,
            unrecognized_format_sha256: unknown_sha256,
            unrecognized_format_byte_count: unknown_byte_count,
            routes: BTreeMap::new(),
        });
    }

    for related_url in related {
        let related_url = related_url
            .as_object()
            .ok_or_else(|| envelope_error("related_url"))?;
        let locator = nisar_bounded_text(
            related_url.get("URL").and_then(Value::as_str),
            "related_url_locator",
            MAX_RELATED_URL_BYTES,
        )?;
        let parsed = Url::parse(&locator).map_err(|_| envelope_error("related_url_locator"))?;
        if !retain_nisar_related_url(&parsed)? {
            continue;
        }
        diagnostics.retained_official_related_url_count += 1;

        let relation = normalize_nisar_relation_token(related_url.get("Type")).map_err(|_| {
            diagnostics.unknown_relation_count += 1;
            set_related_url_trigger(
                &mut diagnostics,
                NisarRelatedUrlContext {
                    relation_class: NisarRelatedUrlClass::UnsupportedOrUnknown,
                    subtype: None,
                    url: &parsed,
                    basename: None,
                    formats: &BTreeSet::new(),
                    mime: None,
                    archive_match: false,
                },
            );
            archive_error("related_url_type_unrecognized", &diagnostics)
        })?;
        let subtype = normalize_nisar_subtype_token(related_url.get("Subtype")).map_err(|_| {
            set_related_url_trigger(
                &mut diagnostics,
                NisarRelatedUrlContext {
                    relation_class: NisarRelatedUrlClass::UnsupportedOrUnknown,
                    subtype: None,
                    url: &parsed,
                    basename: None,
                    formats: &BTreeSet::new(),
                    mime: None,
                    archive_match: false,
                },
            );
            archive_error("related_url_role_conflict", &diagnostics)
        })?;
        let relation_class =
            classify_nisar_related_url(&relation, subtype.as_deref()).map_err(|reason| {
                if reason == "related_url_type_unrecognized" {
                    diagnostics.unknown_relation_count += 1;
                }
                set_related_url_trigger(
                    &mut diagnostics,
                    NisarRelatedUrlContext {
                        relation_class: NisarRelatedUrlClass::UnsupportedOrUnknown,
                        subtype: subtype.as_deref(),
                        url: &parsed,
                        basename: None,
                        formats: &BTreeSet::new(),
                        mime: None,
                        archive_match: false,
                    },
                );
                archive_error(reason, &diagnostics)
            })?;
        let related_format = parse_nisar_format_token(
            related_url.get("Format"),
            "related_url_format",
            &mut diagnostics,
        )?;
        let related_mime = parse_nisar_mime_token(
            related_url.get("MimeType"),
            "related_url_mime_type",
            &mut diagnostics,
        )?;
        let _description = optional_text(related_url, "Description")?;
        let scheme = parsed.scheme().to_owned();
        let mut related_formats = BTreeSet::new();
        add_nisar_family(
            &mut related_formats,
            related_format
                .as_ref()
                .and_then(|token| token.family.as_deref()),
            "related_url_format_conflict",
        )
        .map_err(|_| {
            set_related_url_trigger(
                &mut diagnostics,
                NisarRelatedUrlContext {
                    relation_class,
                    subtype: subtype.as_deref(),
                    url: &parsed,
                    basename: None,
                    formats: &related_formats,
                    mime: related_mime.as_ref(),
                    archive_match: false,
                },
            );
            archive_error("related_url_role_conflict", &diagnostics)
        })?;
        add_nisar_family(
            &mut related_formats,
            related_mime
                .as_ref()
                .and_then(|token| token.family.as_deref()),
            "related_url_format_conflict",
        )
        .map_err(|_| {
            set_related_url_trigger(
                &mut diagnostics,
                NisarRelatedUrlContext {
                    relation_class,
                    subtype: subtype.as_deref(),
                    url: &parsed,
                    basename: None,
                    formats: &related_formats,
                    mime: related_mime.as_ref(),
                    archive_match: false,
                },
            );
            archive_error("related_url_role_conflict", &diagnostics)
        })?;
        let (unknown_sha256, unknown_byte_count) =
            collect_unknown_nisar_tokens(&[related_format.as_ref(), related_mime.as_ref()]);

        if matches!(
            relation_class,
            NisarRelatedUrlClass::GranuleInformation | NisarRelatedUrlClass::ServiceApi
        ) {
            set_related_url_trigger(
                &mut diagnostics,
                NisarRelatedUrlContext {
                    relation_class,
                    subtype: subtype.as_deref(),
                    url: &parsed,
                    basename: None,
                    formats: &related_formats,
                    mime: related_mime.as_ref(),
                    archive_match: false,
                },
            );
            match relation_class {
                NisarRelatedUrlClass::GranuleInformation => {
                    diagnostics.ignored_granule_information_count += 1;
                }
                NisarRelatedUrlClass::ServiceApi => {
                    diagnostics.ignored_service_api_count += 1;
                }
                _ => unreachable!(),
            }
            continue;
        }

        let basename = nisar_related_basename(&parsed).map_err(|reason| {
            set_related_url_trigger(
                &mut diagnostics,
                NisarRelatedUrlContext {
                    relation_class,
                    subtype: subtype.as_deref(),
                    url: &parsed,
                    basename: None,
                    formats: &related_formats,
                    mime: related_mime.as_ref(),
                    archive_match: false,
                },
            );
            archive_error(reason, &diagnostics)
        })?;
        let archive_index = archive_indices.get(&basename).copied();
        set_related_url_trigger(
            &mut diagnostics,
            NisarRelatedUrlContext {
                relation_class,
                subtype: subtype.as_deref(),
                url: &parsed,
                basename: Some(&basename),
                formats: &related_formats,
                mime: related_mime.as_ref(),
                archive_match: archive_index.is_some(),
            },
        );

        match relation_class {
            NisarRelatedUrlClass::ArchiveDataRoute => {
                let Some(index) = archive_index else {
                    diagnostics.unmatched_data_route_count += 1;
                    return Err(archive_error("related_url_archive_identity", &diagnostics));
                };
                diagnostics.matched_archive_related_url_count += 1;
                attach_nisar_related_route(
                    &mut assets[index],
                    NisarRelatedRoute {
                        relation,
                        subtype,
                        formats: &related_formats,
                        unknown_sha256,
                        unknown_byte_count,
                        scheme,
                        locator,
                    },
                    &diagnostics,
                )?;
            }
            NisarRelatedUrlClass::ArchiveVisualizationRoute => {
                if related_format
                    .as_ref()
                    .and_then(|token| token.family.as_deref())
                    .is_some_and(|family| family == "OTHER")
                    || related_mime
                        .as_ref()
                        .and_then(|token| token.family.as_deref())
                        .is_some_and(|family| family == "OTHER")
                {
                    return Err(archive_error("related_url_role_conflict", &diagnostics));
                }
                let archive_formats = archive_index.map(|index| &assets[index].formats);
                let visualization_format =
                    nisar_visualization_format(&basename, &related_formats, archive_formats)
                        .map_err(|reason| archive_error(reason, &diagnostics))?;
                let index = if let Some(index) = archive_index {
                    diagnostics.matched_archive_related_url_count += 1;
                    index
                } else if let Some(index) = url_only_indices.get(&basename).copied() {
                    let asset: &RawNisarAsset = &assets[index];
                    if asset.formats.len() != 1 || !asset.formats.contains(visualization_format) {
                        return Err(archive_error("related_url_role_conflict", &diagnostics));
                    }
                    index
                } else {
                    let index = assets.len();
                    url_only_indices.insert(basename.clone(), index);
                    assets.push(RawNisarAsset {
                        name: basename.clone(),
                        archive_name: None,
                        format_types: BTreeSet::new(),
                        mime_types: BTreeSet::new(),
                        hdf5_evidence: false,
                        size_bytes: None,
                        checksum_algorithm: None,
                        checksum_value: None,
                        url_types: BTreeSet::new(),
                        url_subtypes: BTreeSet::new(),
                        formats: if related_formats.is_empty() {
                            BTreeSet::from([visualization_format.to_owned()])
                        } else {
                            related_formats.clone()
                        },
                        unrecognized_format_sha256: unknown_sha256.clone(),
                        unrecognized_format_byte_count: unknown_byte_count.clone(),
                        routes: BTreeMap::new(),
                    });
                    diagnostics.url_only_visualization_count += 1;
                    index
                };
                attach_nisar_related_route(
                    &mut assets[index],
                    NisarRelatedRoute {
                        relation,
                        subtype,
                        formats: &related_formats,
                        unknown_sha256,
                        unknown_byte_count,
                        scheme,
                        locator,
                    },
                    &diagnostics,
                )?;
            }
            NisarRelatedUrlClass::ArchiveMetadataRoute => {
                if let Some(index) = archive_index {
                    diagnostics.matched_archive_related_url_count += 1;
                    attach_nisar_related_route(
                        &mut assets[index],
                        NisarRelatedRoute {
                            relation,
                            subtype,
                            formats: &related_formats,
                            unknown_sha256,
                            unknown_byte_count,
                            scheme,
                            locator,
                        },
                        &diagnostics,
                    )?;
                } else if (relation == "EXTENDED METADATA" && related_formats.is_empty())
                    || is_supported_unmatched_metadata(&basename, &related_formats)
                {
                    diagnostics.url_only_metadata_count += 1;
                } else {
                    return Err(archive_error("related_url_role_conflict", &diagnostics));
                }
            }
            NisarRelatedUrlClass::GranuleInformation
            | NisarRelatedUrlClass::ServiceApi
            | NisarRelatedUrlClass::UnsupportedOrUnknown => unreachable!(),
        }
    }

    let complete_hdf5_names = assets
        .iter()
        .filter(|asset| {
            let lower_name = asset.name.to_ascii_lowercase();
            let is_qa_hdf5 = lower_name.ends_with("_qa_stats.h5") || lower_name.ends_with("_qa.h5");
            let has_data_role = asset
                .url_types
                .iter()
                .any(|value| matches!(value.as_str(), "GET DATA" | "GET DATA VIA DIRECT ACCESS"));
            asset.hdf5_evidence
                && is_hdf5_like_name(&asset.name)
                && !is_qa_hdf5
                && has_data_role
                && asset.size_bytes.is_some()
                && asset.checksum_algorithm.is_some()
                && asset.checksum_value.is_some()
        })
        .map(|asset| asset.name.clone())
        .collect::<BTreeSet<_>>();
    if complete_hdf5_names.len() > 1 {
        return Err(primary_ambiguous_error());
    }
    if assets.iter().any(|asset| {
        let lower_name = asset.name.to_ascii_lowercase();
        let is_qa_hdf5 = lower_name.ends_with("_qa_stats.h5") || lower_name.ends_with("_qa.h5");
        asset.hdf5_evidence
            && is_hdf5_like_name(&asset.name)
            && !is_qa_hdf5
            && asset.name != expected_primary
            && !complete_hdf5_names.contains(&asset.name)
    }) || complete_hdf5_names
        .iter()
        .any(|name| name != &expected_primary)
    {
        return Err(archive_error("unexpected_hdf5_archive", &diagnostics));
    }

    let records = assets
        .into_iter()
        .map(|asset| {
            let RawNisarAsset {
                name,
                archive_name,
                format_types,
                mime_types,
                hdf5_evidence,
                size_bytes,
                checksum_algorithm,
                checksum_value,
                url_types,
                url_subtypes,
                mut formats,
                unrecognized_format_sha256,
                unrecognized_format_byte_count,
                routes,
            } = asset;
            let lower_name = name.to_ascii_lowercase();
            let meaningful_formats = formats
                .iter()
                .filter(|format| format.as_str() != "OTHER")
                .cloned()
                .collect::<BTreeSet<_>>();
            if meaningful_formats.len() > 1 {
                return Err(archive_error("asset_format_conflict", &diagnostics));
            }
            formats = meaningful_formats;
            let format = formats
                .iter()
                .next()
                .cloned()
                .unwrap_or_else(|| "OTHER".to_owned());
            let is_qa_hdf5 = lower_name.ends_with("_qa_stats.h5") || lower_name.ends_with("_qa.h5");
            let has_data_role = url_types
                .iter()
                .any(|value| matches!(value.as_str(), "GET DATA" | "GET DATA VIA DIRECT ACCESS"));
            if is_hdf5_like_name(&name) && !is_qa_hdf5 && !hdf5_evidence {
                return Err(archive_error("unknown_hdf5_extension", &diagnostics));
            }
            let product_match = name == expected_primary
                && archive_name.is_some()
                && format == "HDF5"
                && hdf5_evidence
                && !is_qa_hdf5
                && has_data_role
                && size_bytes.is_some()
                && checksum_algorithm.is_some()
                && checksum_value.is_some();
            if format == "OTHER" && has_data_role {
                return Err(archive_error("unknown_data_route", &diagnostics));
            }
            let normalized_role = if product_match {
                "primary"
            } else {
                match format.as_str() {
                    "PNG" | "JPEG" if lower_name.contains("thumbnail") => "thumbnail",
                    "PNG" | "JPEG" if is_nisar_main_latlon_preview(&name, &expected_primary) => {
                        "preview"
                    }
                    "PNG" | "JPEG" => "browse",
                    "XML" | "YAML" | "JSON" | "TEXT" | "CSV" => "metadata",
                    "PDF" | "KML" | "HDF5" | "ZIP" | "OTHER" => "auxiliary",
                    _ => return Err(archive_error("archive_format", &diagnostics)),
                }
            };
            let asset_id = if name == expected_primary {
                "science-rslc-hdf5".to_owned()
            } else {
                name.clone()
            };
            let mut native_roles = BTreeSet::new();
            if archive_name.is_some() {
                native_roles.insert(format!("ARCHIVE:{format}"));
            }
            native_roles.extend(url_types.iter().cloned());
            native_roles.extend(url_subtypes.iter().cloned());
            let checksum = match (checksum_algorithm, checksum_value) {
                (Some(algorithm), Some(value)) => json!({
                    "algorithm": algorithm,
                    "value": value
                }),
                (None, None) => Value::Null,
                _ => return Err(archive_error("archive_checksum", &diagnostics)),
            };
            Ok(json!({
                "asset_id": asset_id,
                "native_roles": native_roles,
                "native_evidence": {
                    "archive_name": archive_name,
                    "product_match": product_match,
                    "url_types": url_types,
                    "url_subtypes": url_subtypes,
                    "formats": if format == "OTHER" { BTreeSet::from(["OTHER".to_owned()]) } else { formats },
                    "format_recognized": unrecognized_format_sha256.is_empty(),
                    "format_types": format_types,
                    "mime_types": mime_types,
                    "unrecognized_format_sha256": unrecognized_format_sha256,
                    "unrecognized_format_byte_count": unrecognized_format_byte_count
                },
                "normalized_role": normalized_role,
                "file_name": name,
                "media_type": nisar_media_type(&format),
                "size_bytes": size_bytes,
                "checksum": checksum,
                "routes": routes.into_values().collect::<Vec<_>>()
            }))
        })
        .collect::<ProviderResult<Vec<_>>>()?;
    Ok(json!({
        "raw_resource_count": raw_resource_count,
        "records": records
    }))
}

fn is_nisar_main_latlon_preview(name: &str, expected_primary: &str) -> bool {
    let Some(product_stem) = expected_primary.strip_suffix(".h5") else {
        return false;
    };
    ["png", "jpg", "jpeg"]
        .iter()
        .any(|extension| name.eq_ignore_ascii_case(&format!("{product_stem}_LATLON.{extension}")))
}

fn normalize_nisar_relation_token(value: Option<&Value>) -> Result<String, ()> {
    let value = value.and_then(Value::as_str).ok_or(())?.trim();
    if value.is_empty()
        || value.len() > MAX_TEXT_BYTES
        || !value.is_ascii()
        || value.chars().any(char::is_control)
    {
        return Err(());
    }
    Ok(value.to_ascii_uppercase())
}

fn normalize_nisar_subtype_token(value: Option<&Value>) -> Result<Option<String>, ()> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => normalize_nisar_relation_token(Some(value)).map(Some),
    }
}

fn classify_nisar_related_url(
    relation: &str,
    subtype: Option<&str>,
) -> Result<NisarRelatedUrlClass, &'static str> {
    match relation {
        "GET DATA" | "GET DATA VIA DIRECT ACCESS" => {
            if subtype.is_none() {
                Ok(NisarRelatedUrlClass::ArchiveDataRoute)
            } else {
                Err("related_url_role_conflict")
            }
        }
        "GET RELATED VISUALIZATION" => {
            if subtype.is_none() {
                Ok(NisarRelatedUrlClass::ArchiveVisualizationRoute)
            } else {
                Err("related_url_role_conflict")
            }
        }
        "EXTENDED METADATA" => {
            if subtype.is_none() {
                Ok(NisarRelatedUrlClass::ArchiveMetadataRoute)
            } else {
                Err("related_url_role_conflict")
            }
        }
        "VIEW RELATED INFORMATION" => match subtype {
            None => Ok(NisarRelatedUrlClass::GranuleInformation),
            Some(
                "PRODUCT QUALITY ASSESSMENT"
                | "QA STATISTICS"
                | "METADATA"
                | "PRODUCT METADATA"
                | "PRODUCT INFORMATION"
                | "DOCUMENTATION",
            ) => Ok(NisarRelatedUrlClass::ArchiveMetadataRoute),
            Some(
                "USER GUIDE"
                | "GENERAL INFORMATION"
                | "PROJECT INFORMATION"
                | "PROJECT HOME PAGE"
                | "LANDING PAGE"
                | "PRODUCT LANDING PAGE",
            ) => Ok(NisarRelatedUrlClass::GranuleInformation),
            Some(_) => Err("related_url_role_conflict"),
        },
        "PRODUCT LANDING PAGE" | "PROJECT HOME PAGE" | "GENERAL INFORMATION" => {
            if subtype.is_none() {
                Ok(NisarRelatedUrlClass::GranuleInformation)
            } else {
                Err("related_url_role_conflict")
            }
        }
        "USE SERVICE API" => {
            if subtype.is_none() {
                Ok(NisarRelatedUrlClass::ServiceApi)
            } else {
                Err("related_url_role_conflict")
            }
        }
        _ => Err("related_url_type_unrecognized"),
    }
}

fn nisar_related_basename(url: &Url) -> Result<String, &'static str> {
    let raw = url
        .path()
        .rsplit('/')
        .next()
        .filter(|value| !value.is_empty())
        .ok_or("related_url_basename")?;
    let decoded = percent_decode_nisar_path_segment(raw).ok_or("related_url_basename")?;
    if decoded.contains(['/', '\\']) || decoded == "." || decoded == ".." {
        return Err("related_url_basename");
    }
    nisar_bounded_text(Some(&decoded), "related_url_basename", MAX_TEXT_BYTES)
        .map_err(|_| "related_url_basename")
}

fn percent_decode_nisar_path_segment(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return None;
            }
            let high = hex_nibble(bytes[index + 1])?;
            let low = hex_nibble(bytes[index + 2])?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn nisar_visualization_format(
    basename: &str,
    related_formats: &BTreeSet<String>,
    archive_formats: Option<&BTreeSet<String>>,
) -> Result<&'static str, &'static str> {
    let expected = match nisar_extension_family(basename) {
        "png" => "PNG",
        "jpeg" => "JPEG",
        _ => return Err("related_url_role_conflict"),
    };
    let formats = if !related_formats.is_empty() {
        related_formats
    } else {
        archive_formats.ok_or("related_url_role_conflict")?
    };
    if formats.len() != 1 || !formats.contains(expected) {
        return Err("related_url_role_conflict");
    }
    Ok(expected)
}

fn is_supported_unmatched_metadata(basename: &str, related_formats: &BTreeSet<String>) -> bool {
    let extension_family = nisar_extension_family(basename);
    related_formats.len() == 1
        && related_formats.iter().next().is_some_and(|format| {
            let expected_extension = match format.as_str() {
                "XML" => "xml",
                "YAML" => "yaml",
                "JSON" => "json",
                "TEXT" => "text",
                "CSV" => "csv",
                "PDF" => "pdf",
                "KML" => "kml",
                _ => return false,
            };
            extension_family == expected_extension
        })
}

fn attach_nisar_related_route(
    asset: &mut RawNisarAsset,
    route: NisarRelatedRoute<'_>,
    diagnostics: &NisarFormatDiagnostics,
) -> ProviderResult<()> {
    asset.url_types.insert(route.relation);
    if let Some(value) = route.subtype {
        asset.url_subtypes.insert(value);
    }
    for family in route.formats {
        add_nisar_family(
            &mut asset.formats,
            Some(family.as_str()),
            "related_url_format_conflict",
        )
        .map_err(|_| archive_error("related_url_role_conflict", diagnostics))?;
    }
    asset.hdf5_evidence |= route.formats.contains("HDF5");
    asset
        .unrecognized_format_sha256
        .extend(route.unknown_sha256);
    asset
        .unrecognized_format_byte_count
        .extend(route.unknown_byte_count);
    asset.routes.insert(
        (route.scheme.clone(), route.locator.clone()),
        json!({"scheme": route.scheme, "locator": route.locator}),
    );
    Ok(())
}

fn set_related_url_trigger(
    diagnostics: &mut NisarFormatDiagnostics,
    context: NisarRelatedUrlContext<'_>,
) {
    let raw_basename = context
        .url
        .path()
        .rsplit('/')
        .next()
        .filter(|value| !value.is_empty());
    let candidate = context.basename.or(raw_basename);
    let (basename_sha256, basename_byte_count) = candidate.map_or((None, None), |value| {
        (
            Some(format!("{:x}", Sha256::digest(value.as_bytes()))),
            Some(value.len()),
        )
    });
    diagnostics.related_url_trigger = Some(NisarRelatedUrlDiagnostic {
        relation_class: context.relation_class.label(),
        subtype_class: nisar_subtype_class(context.subtype),
        basename_sha256,
        basename_byte_count,
        basename_extension_family: candidate.map(nisar_extension_family).unwrap_or("none"),
        format_family: safe_family_summary(
            context.formats,
            context.formats.is_empty() && context.mime.is_none(),
        ),
        mime_family: context
            .mime
            .and_then(|token| token.family.as_deref())
            .unwrap_or("unknown")
            .to_owned(),
        official_host_class: nisar_official_host_class(context.url),
        scheme_class: match context.url.scheme() {
            "https" => "https",
            "s3" => "s3",
            _ => "other",
        },
        archive_match: context.archive_match,
        hdf5_like_basename: candidate.is_some_and(is_hdf5_like_name),
    });
}

fn safe_family_summary(formats: &BTreeSet<String>, no_format: bool) -> String {
    if formats.is_empty() {
        return if no_format {
            "none".to_owned()
        } else {
            "unknown".to_owned()
        };
    }
    formats.iter().cloned().collect::<Vec<_>>().join("|")
}

fn nisar_subtype_class(subtype: Option<&str>) -> &'static str {
    match subtype {
        None => "none",
        Some("PRODUCT QUALITY ASSESSMENT" | "QA STATISTICS") => "quality",
        Some("METADATA" | "PRODUCT METADATA" | "PRODUCT INFORMATION" | "DOCUMENTATION") => {
            "metadata"
        }
        Some(
            "USER GUIDE"
            | "GENERAL INFORMATION"
            | "PROJECT INFORMATION"
            | "PROJECT HOME PAGE"
            | "LANDING PAGE"
            | "PRODUCT LANDING PAGE",
        ) => "granule_information",
        Some(_) => "unknown",
    }
}

fn nisar_extension_family(value: &str) -> &'static str {
    let lower = value.to_ascii_lowercase();
    if lower.ends_with(".h5") || lower.ends_with(".hdf5") || lower.ends_with(".he5") {
        "hdf5"
    } else if lower.ends_with(".png") {
        "png"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "jpeg"
    } else if lower.ends_with(".xml") {
        "xml"
    } else if lower.ends_with(".yaml") || lower.ends_with(".yml") {
        "yaml"
    } else if lower.ends_with(".json") {
        "json"
    } else if lower.ends_with(".txt") {
        "text"
    } else if lower.ends_with(".csv") {
        "csv"
    } else if lower.ends_with(".pdf") {
        "pdf"
    } else if lower.ends_with(".kml") {
        "kml"
    } else if lower.ends_with(".zip") {
        "zip"
    } else {
        "other"
    }
}

fn nisar_official_host_class(url: &Url) -> &'static str {
    match (url.scheme(), url.host_str()) {
        ("https", Some("nisar.asf.earthdatacloud.nasa.gov")) => "official_https",
        ("s3", Some("sds-n-cumulus-prod-nisar-products")) => "official_products_s3",
        ("s3", Some("sds-n-cumulus-prod-nisar-browse")) => "official_browse_s3",
        _ => "other",
    }
}

fn parse_nisar_format_token(
    value: Option<&Value>,
    reason: &'static str,
    diagnostics: &mut NisarFormatDiagnostics,
) -> ProviderResult<Option<NisarFormatToken>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = nisar_bounded_text(value.as_str(), reason, MAX_TEXT_BYTES)?;
    let family = canonical_nisar_format(&value).map(str::to_owned);
    let token = if family.is_some() {
        diagnostics.recognized_format_count += 1;
        NisarFormatToken {
            family,
            unknown_sha256: None,
            byte_count: value.len(),
        }
    } else {
        let digest = format!("{:x}", Sha256::digest(value.as_bytes()));
        diagnostics.unrecognized_format_count += 1;
        diagnostics
            .unrecognized_format_sha256
            .insert(digest.clone());
        diagnostics
            .unrecognized_format_byte_count
            .insert(value.len());
        NisarFormatToken {
            family: None,
            unknown_sha256: Some(digest),
            byte_count: value.len(),
        }
    };
    Ok(Some(token))
}

fn parse_nisar_mime_token(
    value: Option<&Value>,
    reason: &'static str,
    diagnostics: &mut NisarFormatDiagnostics,
) -> ProviderResult<Option<NisarFormatToken>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = nisar_bounded_text(value.as_str(), reason, MAX_TEXT_BYTES)?;
    let family = canonical_nisar_mime(&value).map(str::to_owned);
    let token = if family.is_some() {
        diagnostics.recognized_format_count += 1;
        NisarFormatToken {
            family,
            unknown_sha256: None,
            byte_count: value.len(),
        }
    } else {
        let digest = format!("{:x}", Sha256::digest(value.as_bytes()));
        diagnostics.unrecognized_format_count += 1;
        diagnostics
            .unrecognized_format_sha256
            .insert(digest.clone());
        diagnostics
            .unrecognized_format_byte_count
            .insert(value.len());
        NisarFormatToken {
            family: None,
            unknown_sha256: Some(digest),
            byte_count: value.len(),
        }
    };
    Ok(Some(token))
}

fn collect_unknown_nisar_tokens(
    tokens: &[Option<&NisarFormatToken>],
) -> (BTreeSet<String>, BTreeSet<usize>) {
    let mut hashes = BTreeSet::new();
    let mut byte_counts = BTreeSet::new();
    for token in tokens.iter().flatten() {
        if let Some(hash) = &token.unknown_sha256 {
            hashes.insert(hash.clone());
            byte_counts.insert(token.byte_count);
        }
    }
    (hashes, byte_counts)
}

fn add_nisar_family(
    formats: &mut BTreeSet<String>,
    family: Option<&str>,
    conflict_reason: &'static str,
) -> ProviderResult<()> {
    let Some(family) = family else {
        return Ok(());
    };
    if family == "OTHER" {
        if formats.is_empty() {
            formats.insert("OTHER".to_owned());
        }
        return Ok(());
    }
    if formats
        .iter()
        .any(|existing| existing != "OTHER" && existing != family)
    {
        return Err(envelope_error(conflict_reason));
    }
    formats.retain(|existing| existing == family || existing != "OTHER");
    formats.insert(family.to_owned());
    Ok(())
}

fn nisar_archive_integrity(
    archive: &Map<String, Value>,
) -> ProviderResult<(Option<u64>, Option<String>, Option<String>)> {
    let size_bytes = match archive.get("SizeInBytes") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_u64()
                .filter(|value| *value > 0 && *value <= i64::MAX as u64)
                .ok_or_else(|| envelope_error("archive_size"))?,
        ),
    };
    let Some(checksum) = archive.get("Checksum").filter(|value| !value.is_null()) else {
        return Ok((None, None, None));
    };
    let checksum = checksum
        .as_object()
        .ok_or_else(|| envelope_error("archive_checksum"))?;
    let algorithm = text(checksum, "Algorithm")?;
    if !matches!(algorithm.as_str(), "MD5" | "SHA-256" | "SHA256") {
        return Err(envelope_error("archive_checksum"));
    }
    let value = text(checksum, "Value")?;
    if size_bytes.is_none() {
        return Ok((None, None, None));
    }
    Ok((size_bytes, Some(algorithm), Some(value)))
}

fn is_hdf5_like_name(name: &str) -> bool {
    let lower_name = name.to_ascii_lowercase();
    lower_name.ends_with(".h5") || lower_name.ends_with(".hdf5") || lower_name.ends_with(".he5")
}

fn canonical_nisar_format(format: &str) -> Option<&'static str> {
    match format.to_ascii_uppercase().as_str() {
        "HDF5" => Some("HDF5"),
        "PNG" => Some("PNG"),
        "JPEG" | "JPG" => Some("JPEG"),
        "XML" => Some("XML"),
        "YAML" | "YML" => Some("YAML"),
        "CSV" => Some("CSV"),
        "KML" => Some("KML"),
        "PDF" => Some("PDF"),
        "JSON" => Some("JSON"),
        "TEXT" | "TXT" => Some("TEXT"),
        "ZIP" => Some("ZIP"),
        _ => None,
    }
}

fn canonical_nisar_mime(mime: &str) -> Option<&'static str> {
    match mime.to_ascii_lowercase().as_str() {
        "application/x-hdf5" | "application/hdf5" => Some("HDF5"),
        "image/png" => Some("PNG"),
        "image/jpeg" => Some("JPEG"),
        "application/xml" | "text/xml" => Some("XML"),
        "application/yaml" | "text/yaml" => Some("YAML"),
        "text/csv" => Some("CSV"),
        "application/vnd.google-earth.kml+xml" => Some("KML"),
        "application/pdf" => Some("PDF"),
        "application/json" => Some("JSON"),
        "text/plain" => Some("TEXT"),
        "application/zip" | "application/x-zip-compressed" => Some("ZIP"),
        "application/octet-stream" => Some("OTHER"),
        _ => None,
    }
}

fn nisar_media_type(format: &str) -> &'static str {
    match format {
        "HDF5" => "application/x-hdf5",
        "PNG" => "image/png",
        "JPEG" => "image/jpeg",
        "XML" => "application/xml",
        "YAML" => "application/yaml",
        "CSV" => "text/csv",
        "KML" => "application/vnd.google-earth.kml+xml",
        "PDF" => "application/pdf",
        "JSON" => "application/json",
        "TEXT" => "text/plain",
        "ZIP" => "application/zip",
        "OTHER" => "application/octet-stream",
        _ => "application/octet-stream",
    }
}

fn archive_error(reason: &'static str, diagnostics: &NisarFormatDiagnostics) -> ContractError {
    ContractError::new(
        "ASF_ASSET_CLASSIFICATION_INVALID",
        "ASF CMR/UMM 记录结构无效",
        Map::from_iter([
            ("reason".to_owned(), Value::String(reason.to_owned())),
            (
                "related_url_count".to_owned(),
                json!(diagnostics.related_url_count),
            ),
            (
                "retained_official_related_url_count".to_owned(),
                json!(diagnostics.retained_official_related_url_count),
            ),
            (
                "matched_archive_related_url_count".to_owned(),
                json!(diagnostics.matched_archive_related_url_count),
            ),
            (
                "ignored_granule_information_count".to_owned(),
                json!(diagnostics.ignored_granule_information_count),
            ),
            (
                "ignored_service_api_count".to_owned(),
                json!(diagnostics.ignored_service_api_count),
            ),
            (
                "url_only_visualization_count".to_owned(),
                json!(diagnostics.url_only_visualization_count),
            ),
            (
                "url_only_metadata_count".to_owned(),
                json!(diagnostics.url_only_metadata_count),
            ),
            (
                "unmatched_data_route_count".to_owned(),
                json!(diagnostics.unmatched_data_route_count),
            ),
            (
                "unknown_relation_count".to_owned(),
                json!(diagnostics.unknown_relation_count),
            ),
            ("archive_count".to_owned(), json!(diagnostics.archive_count)),
            (
                "recognized_format_count".to_owned(),
                json!(diagnostics.recognized_format_count),
            ),
            (
                "unrecognized_format_count".to_owned(),
                json!(diagnostics.unrecognized_format_count),
            ),
            (
                "unrecognized_format_sha256".to_owned(),
                json!(diagnostics.unrecognized_format_sha256),
            ),
            (
                "unrecognized_format_byte_count".to_owned(),
                json!(diagnostics.unrecognized_format_byte_count),
            ),
            (
                "unknown_on_expected_primary".to_owned(),
                json!(diagnostics.unknown_on_expected_primary),
            ),
            (
                "unknown_on_hdf5_extension".to_owned(),
                json!(diagnostics.unknown_on_hdf5_extension),
            ),
            (
                "related_url_trigger".to_owned(),
                diagnostics
                    .related_url_trigger
                    .as_ref()
                    .map_or(Value::Null, |trigger| {
                        json!({
                            "relation_class": trigger.relation_class,
                            "subtype_class": trigger.subtype_class,
                            "basename_sha256": trigger.basename_sha256,
                            "basename_byte_count": trigger.basename_byte_count,
                            "basename_extension_family": trigger.basename_extension_family,
                            "format_family": trigger.format_family,
                            "mime_family": trigger.mime_family,
                            "official_host_class": trigger.official_host_class,
                            "scheme_class": trigger.scheme_class,
                            "archive_match": trigger.archive_match,
                            "hdf5_like_basename": trigger.hdf5_like_basename
                        })
                    }),
            ),
        ]),
    )
    .expect("bounded NISAR archive error is safe")
}

fn primary_missing_error() -> ContractError {
    ContractError::new(
        "ASF_PRIMARY_ASSET_MISSING",
        "NISAR Granule 缺少完整主科学 HDF5",
        Map::new(),
    )
    .expect("bounded missing-primary error is safe")
}

fn primary_ambiguous_error() -> ContractError {
    ContractError::new(
        "ASF_PRIMARY_ASSET_AMBIGUOUS",
        "NISAR Granule 包含多个完整主科学 HDF5",
        Map::new(),
    )
    .expect("bounded ambiguous-primary error is safe")
}

fn retain_nisar_related_url(url: &Url) -> ProviderResult<bool> {
    let host = url
        .host_str()
        .ok_or_else(|| envelope_error("related_url_locator"))?;
    let lower_host = host.to_ascii_lowercase();
    let lower_path = url.path().to_ascii_lowercase();
    if lower_host.contains("nisar-jpl-private")
        || lower_path.contains("/private/")
        || lower_path.ends_with("/s3credentials")
    {
        return Ok(false);
    }
    let official_host = matches!(
        (url.scheme(), host),
        ("https", "nisar.asf.earthdatacloud.nasa.gov")
            | ("s3", "sds-n-cumulus-prod-nisar-products")
            | ("s3", "sds-n-cumulus-prod-nisar-browse")
    );
    let safe = official_host
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && match url.scheme() {
            "https" => url.port_or_known_default() == Some(443),
            "s3" => url.port().is_none(),
            _ => false,
        };
    if safe {
        Ok(true)
    } else {
        Err(envelope_error("related_url_locator"))
    }
}

fn nisar_array<'a>(
    map: &'a Map<String, Value>,
    key: &'static str,
    maximum: usize,
    limit_reason: &'static str,
) -> ProviderResult<&'a [Value]> {
    let values = map
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| envelope_error(key))?;
    if values.len() > maximum {
        return Err(asset_limit_error(limit_reason));
    }
    Ok(values)
}

fn nisar_bounded_text(
    value: Option<&str>,
    reason: &'static str,
    maximum: usize,
) -> ProviderResult<String> {
    let value = value
        .filter(|value| {
            !value.is_empty() && value == &value.trim() && !value.chars().any(char::is_control)
        })
        .ok_or_else(|| envelope_error(reason))?;
    if value.len() > maximum {
        return Err(asset_limit_error(reason));
    }
    Ok(value.to_owned())
}

fn additional_attributes(
    umm: &Map<String, Value>,
) -> ProviderResult<BTreeMap<String, Vec<String>>> {
    let values = array(umm, "AdditionalAttributes", MAX_ADDITIONAL_ATTRIBUTES)?;
    let mut attributes = BTreeMap::new();
    for value in values {
        let value = value
            .as_object()
            .ok_or_else(|| envelope_error("additional_attribute"))?;
        let name = text(value, "Name")?;
        let raw_values = array(value, "Values", MAX_ATTRIBUTE_VALUES)?;
        if raw_values.is_empty() {
            return Err(envelope_error("additional_attribute_values"));
        }
        let values = raw_values
            .iter()
            .map(|value| bounded_text(value.as_str(), "additional_attribute_value"))
            .collect::<ProviderResult<Vec<_>>>()?;
        if attributes.insert(name, values).is_some() {
            return Err(envelope_error("duplicate_additional_attribute"));
        }
    }
    Ok(attributes)
}

fn attribute_one(
    attributes: &BTreeMap<String, Vec<String>>,
    name: &'static str,
) -> ProviderResult<String> {
    let values = attributes
        .get(name)
        .ok_or_else(|| envelope_error("required_attribute"))?;
    let [value] = values.as_slice() else {
        return Err(envelope_error("single_attribute_value"));
    };
    Ok(value.clone())
}

fn attribute_optional_one(
    attributes: &BTreeMap<String, Vec<String>>,
    name: &'static str,
) -> ProviderResult<Option<String>> {
    let Some(values) = attributes.get(name) else {
        return Ok(None);
    };
    let [value] = values.as_slice() else {
        return Err(envelope_error("single_attribute_value"));
    };
    Ok(Some(value.clone()))
}

fn attribute_bool(
    attributes: &BTreeMap<String, Vec<String>>,
    name: &'static str,
) -> ProviderResult<bool> {
    match attribute_one(attributes, name)?.as_str() {
        "TRUE" => Ok(true),
        "FALSE" => Ok(false),
        _ => Err(envelope_error("boolean_attribute")),
    }
}

fn attribute_many(
    attributes: &BTreeMap<String, Vec<String>>,
    name: &'static str,
) -> ProviderResult<Vec<String>> {
    attributes
        .get(name)
        .filter(|values| !values.is_empty())
        .cloned()
        .ok_or_else(|| envelope_error("required_attribute"))
}

fn attribute_u64(
    attributes: &BTreeMap<String, Vec<String>>,
    name: &'static str,
) -> ProviderResult<u64> {
    attribute_one(attributes, name)?
        .parse()
        .map_err(|_| envelope_error("integer_attribute"))
}

fn attribute_f64(
    attributes: &BTreeMap<String, Vec<String>>,
    name: &'static str,
) -> ProviderResult<f64> {
    attribute_one(attributes, name)?
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| envelope_error("number_attribute"))
}

fn platform_identity(umm: &Map<String, Value>) -> ProviderResult<(String, String, Option<String>)> {
    let platforms = array(umm, "Platforms", 8)?;
    let [platform] = platforms else {
        return Err(envelope_error("platform"));
    };
    let platform = platform
        .as_object()
        .ok_or_else(|| envelope_error("platform"))?;
    let platform_name = text(platform, "ShortName")?;
    let instruments = array(platform, "Instruments", 8)?;
    let [instrument] = instruments else {
        return Err(envelope_error("instrument"));
    };
    let instrument = instrument
        .as_object()
        .ok_or_else(|| envelope_error("instrument"))?;
    let instrument_name = text(instrument, "ShortName")?;
    let look_direction = instrument
        .get("Characteristics")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .find(|value| value.get("Name").and_then(Value::as_str) == Some("LookDirection"))
        .and_then(|value| value.get("Value"))
        .and_then(Value::as_str)
        .map(|value| bounded_text(Some(value), "platform_look_direction"))
        .transpose()?;
    Ok((platform_name, instrument_name, look_direction))
}

fn producer_granule_id(umm: &Map<String, Value>) -> ProviderResult<String> {
    let data_granule = object(umm, "DataGranule")?;
    let identifiers = array(data_granule, "Identifiers", 32)?;
    let matches = identifiers
        .iter()
        .filter_map(Value::as_object)
        .filter(|value| {
            value.get("IdentifierType").and_then(Value::as_str) == Some("ProducerGranuleId")
        })
        .collect::<Vec<_>>();
    let [identifier] = matches.as_slice() else {
        return Err(envelope_error("producer_granule_id"));
    };
    text(identifier, "Identifier")
}

fn optional_granule_identifier(
    umm: &Map<String, Value>,
    identifier_type: &'static str,
) -> ProviderResult<Option<String>> {
    let data_granule = object(umm, "DataGranule")?;
    let identifiers = array(data_granule, "Identifiers", 32)?;
    let matches = identifiers
        .iter()
        .filter_map(Value::as_object)
        .filter(|value| {
            value.get("IdentifierType").and_then(Value::as_str) == Some(identifier_type)
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Ok(None),
        [identifier] => text(identifier, "Identifier").map(Some),
        _ => Err(envelope_error("duplicate_granule_identifier")),
    }
}

fn optional_pge_version(umm: &Map<String, Value>) -> ProviderResult<Option<String>> {
    match umm.get("PGEVersionClass") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(value)) => text(value, "PGEVersion").map(Some),
        Some(_) => Err(envelope_error("PGEVersionClass")),
    }
}

fn footprint(umm: &Map<String, Value>) -> ProviderResult<Map<String, Value>> {
    let polygons = umm
        .get("SpatialExtent")
        .and_then(Value::as_object)
        .and_then(|value| value.get("HorizontalSpatialDomain"))
        .and_then(Value::as_object)
        .and_then(|value| value.get("Geometry"))
        .and_then(Value::as_object)
        .and_then(|value| value.get("GPolygons"))
        .and_then(Value::as_array)
        .filter(|values| !values.is_empty() && values.len() <= MAX_POLYGONS)
        .ok_or_else(|| envelope_error("footprint"))?;
    let mut rings = Vec::with_capacity(polygons.len());
    for polygon in polygons {
        let points = polygon
            .as_object()
            .and_then(|value| value.get("Boundary"))
            .and_then(Value::as_object)
            .and_then(|value| value.get("Points"))
            .and_then(Value::as_array)
            .filter(|values| values.len() >= 3 && values.len() <= MAX_POINTS_PER_POLYGON)
            .ok_or_else(|| envelope_error("footprint_points"))?;
        let mut ring = points
            .iter()
            .map(|point| {
                let point = point
                    .as_object()
                    .ok_or_else(|| envelope_error("footprint_point"))?;
                let latitude = finite_number(point.get("Latitude"), "latitude")?;
                let longitude = finite_number(point.get("Longitude"), "longitude")?;
                if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
                    return Err(envelope_error("footprint_coordinate"));
                }
                Ok(json!([longitude, latitude]))
            })
            .collect::<ProviderResult<Vec<_>>>()?;
        if ring.first() != ring.last() {
            ring.push(ring[0].clone());
        }
        rings.push(Value::Array(ring));
    }
    if rings.len() == 1 {
        Ok(Map::from_iter([
            ("type".to_owned(), Value::String("Polygon".to_owned())),
            ("coordinates".to_owned(), Value::Array(rings)),
        ]))
    } else {
        Ok(Map::from_iter([
            ("type".to_owned(), Value::String("MultiPolygon".to_owned())),
            (
                "coordinates".to_owned(),
                Value::Array(
                    rings
                        .into_iter()
                        .map(|ring| Value::Array(vec![ring]))
                        .collect(),
                ),
            ),
        ]))
    }
}

fn archive_assets(umm: &Map<String, Value>) -> ProviderResult<(Vec<Value>, Value)> {
    let data_granule = object(umm, "DataGranule")?;
    let archives = array(
        data_granule,
        "ArchiveAndDistributionInformation",
        MAX_ARCHIVES,
    )?;
    if archives.is_empty() {
        return Err(envelope_error("archives"));
    }
    let related = array(umm, "RelatedUrls", MAX_RELATED_URLS)?;
    let mut records = Vec::with_capacity(archives.len());
    let mut archive_names = Vec::with_capacity(archives.len());
    for archive in archives {
        let archive = archive
            .as_object()
            .ok_or_else(|| envelope_error("archive"))?;
        let name = text(archive, "Name")?;
        if !text(archive, "Format")?.eq_ignore_ascii_case("ZIP") {
            return Err(envelope_error("archive_format"));
        }
        let size = archive
            .get("SizeInBytes")
            .and_then(Value::as_u64)
            .filter(|value| *value > 0)
            .ok_or_else(|| envelope_error("archive_size"))?;
        let checksum = object(archive, "Checksum")?;
        let checksum = json!({
            "algorithm": text(checksum, "Algorithm")?,
            "value": text(checksum, "Value")?
        });
        archive_names.push(name.clone());
        records.push(json!({
            "asset_id": name,
            "native_role": "archive",
            "file_name": archive_names.last().expect("name inserted"),
            "media_type": "application/zip",
            "size_bytes": size,
            "checksum": checksum,
            "routes": []
        }));
    }

    let mut browse_urls = Vec::new();
    for related_url in related {
        let related_url = related_url
            .as_object()
            .ok_or_else(|| envelope_error("related_url"))?;
        let locator = text(related_url, "URL")?;
        let relation = text(related_url, "Type")?;
        let _description = related_url
            .get("Description")
            .and_then(Value::as_str)
            .map(|value| bounded_text(Some(value), "related_url_description"))
            .transpose()?
            .unwrap_or_default();
        if matches!(relation.as_str(), "BROWSE" | "GET RELATED VISUALIZATION") {
            browse_urls.push(Value::String(locator));
            continue;
        }
        if !matches!(relation.as_str(), "GET DATA" | "GET DATA VIA DIRECT ACCESS") {
            continue;
        }
        let parsed = Url::parse(&locator).map_err(|_| envelope_error("related_url_locator"))?;
        let scheme = match parsed.scheme() {
            "https" => "https",
            "s3" => "s3",
            _ => return Err(envelope_error("related_url_scheme")),
        };
        let matching = archive_names
            .iter()
            .enumerate()
            .filter(|(_, name)| {
                parsed
                    .path_segments()
                    .and_then(Iterator::last)
                    .is_some_and(|basename| basename == name.as_str())
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let index = match matching.as_slice() {
            [index] => *index,
            _ => return Err(envelope_error("related_url_archive_identity")),
        };
        records[index]["routes"]
            .as_array_mut()
            .expect("routes are an array")
            .push(json!({"scheme": scheme, "locator": locator}));
    }
    for record in &mut records {
        let routes = record["routes"]
            .as_array_mut()
            .expect("routes are an array");
        routes.sort_by_key(|left| left.to_string());
        routes.dedup();
        if routes.is_empty() {
            return Err(envelope_error("archive_routes"));
        }
    }
    let browse = if browse_urls.is_empty() {
        Value::Null
    } else {
        json!({"urls": browse_urls})
    };
    Ok((records, browse))
}

fn object<'a>(
    map: &'a Map<String, Value>,
    key: &'static str,
) -> ProviderResult<&'a Map<String, Value>> {
    map.get(key)
        .and_then(Value::as_object)
        .ok_or_else(|| envelope_error(key))
}

fn array<'a>(
    map: &'a Map<String, Value>,
    key: &'static str,
    maximum: usize,
) -> ProviderResult<&'a [Value]> {
    map.get(key)
        .and_then(Value::as_array)
        .filter(|values| values.len() <= maximum)
        .map(Vec::as_slice)
        .ok_or_else(|| envelope_error(key))
}

fn text(map: &Map<String, Value>, key: &'static str) -> ProviderResult<String> {
    bounded_text(map.get(key).and_then(Value::as_str), key)
}

fn optional_text(map: &Map<String, Value>, key: &'static str) -> ProviderResult<Option<String>> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => bounded_text(Some(value), key).map(Some),
        Some(_) => Err(envelope_error(key)),
    }
}

fn bounded_text(value: Option<&str>, reason: &'static str) -> ProviderResult<String> {
    bounded_text_with_limit(value, reason, MAX_TEXT_BYTES)
}

fn bounded_text_with_limit(
    value: Option<&str>,
    reason: &'static str,
    maximum: usize,
) -> ProviderResult<String> {
    value
        .filter(|value| {
            !value.is_empty()
                && value == &value.trim()
                && value.len() <= maximum
                && !value.chars().any(char::is_control)
        })
        .map(str::to_owned)
        .ok_or_else(|| envelope_error(reason))
}

fn finite_number(value: Option<&Value>, reason: &'static str) -> ProviderResult<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .ok_or_else(|| envelope_error(reason))
}

fn collection_error(reason: &'static str) -> ContractError {
    ContractError::new(
        "ASF_COLLECTION_IDENTITY_MISMATCH",
        "ASF CMR Collection 身份不匹配",
        Map::from_iter([("reason".to_owned(), Value::String(reason.to_owned()))]),
    )
    .expect("bounded raw UMM collection error is safe")
}

fn asset_limit_error(reason: &'static str) -> ContractError {
    ContractError::new(
        "ASF_ASSET_GRAPH_LIMIT_EXCEEDED",
        "NISAR 资产图超过冻结边界",
        Map::from_iter([("reason".to_owned(), Value::String(reason.to_owned()))]),
    )
    .expect("bounded raw UMM asset limit error is safe")
}

fn envelope_error(reason: &'static str) -> ContractError {
    ContractError::new(
        "ASF_ASSET_CLASSIFICATION_INVALID",
        "ASF CMR/UMM 记录结构无效",
        Map::from_iter([("reason".to_owned(), Value::String(reason.to_owned()))]),
    )
    .expect("bounded envelope error is safe")
}
