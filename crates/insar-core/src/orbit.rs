use serde_json::{Map, Value};

use crate::{
    ContractError, OrbitFileMetadata, OrbitIssueSeverity, OrbitMatchIssue, OrbitMatchReport,
    OrbitMatchRequest, OrbitMatchResult, OrbitMatchSummary, OrbitParseError, OrbitParseRequest,
    OrbitParseResponse, OrbitPlatform, OrbitProductType, OrbitSceneInput,
};

const INVALID_FILENAME: &str = "invalid_filename";
const INVALID_TIMESTAMP: &str = "invalid_timestamp";
const INVALID_VALIDITY_WINDOW: &str = "invalid_validity_window";

pub fn parse_orbit_filenames(request: &OrbitParseRequest) -> OrbitParseResponse {
    let mut orbit_files = Vec::new();
    let mut errors = Vec::new();

    for file_name in &request.file_names {
        match parse_orbit_filename(file_name) {
            Ok(orbit) => orbit_files.push(orbit),
            Err(reason) => errors.push(parse_error(file_name, reason)),
        }
    }

    OrbitParseResponse {
        orbit_files,
        errors,
    }
}

pub fn match_orbits(request: &OrbitMatchRequest) -> OrbitMatchReport {
    let results: Vec<_> = request
        .scenes
        .iter()
        .map(|scene| match_scene(scene, &request.orbit_files))
        .collect();
    let matched = results.iter().filter(|result| result.is_matched).count() as u64;
    let issues = results
        .iter()
        .flat_map(|result| result.issues.iter().cloned())
        .collect();
    let mut orbit_types = Vec::new();
    for orbit in &request.orbit_files {
        if !orbit_types.contains(&orbit.orbit_type) {
            orbit_types.push(orbit.orbit_type.clone());
        }
    }
    orbit_types.sort_by_key(orbit_type_label);
    let total = request.scenes.len() as u64;

    OrbitMatchReport {
        total,
        matched,
        unmatched: total - matched,
        results,
        issues,
        summary: OrbitMatchSummary {
            orbit_files: request.orbit_files.len() as u64,
            scenes: total,
            matched,
            unmatched: total - matched,
            orbit_types,
        },
    }
}

fn match_scene(scene: &OrbitSceneInput, orbit_files: &[OrbitFileMetadata]) -> OrbitMatchResult {
    let Some(acquisition) = scene.acquisition_unix_seconds else {
        return unmatched(scene, "ORBIT_MISSING", "scene has no acquisition_datetime");
    };
    let platform_matches: Vec<_> = orbit_files
        .iter()
        .filter(|orbit| orbit.platform == scene.platform)
        .collect();
    let candidates: Vec<_> = platform_matches
        .iter()
        .filter(|orbit| {
            orbit.validity_start_unix_seconds <= acquisition
                && acquisition <= orbit.validity_stop_unix_seconds
        })
        .map(|orbit| (*orbit).clone())
        .collect();
    if candidates.is_empty() {
        return if orbit_files.is_empty() {
            unmatched(scene, "ORBIT_MISSING", "no orbit files")
        } else if platform_matches.is_empty() {
            unmatched(
                scene,
                "ORBIT_PLATFORM_MISMATCH",
                "no orbit matches the scene platform",
            )
        } else {
            unmatched(
                scene,
                "ORBIT_TIME_NOT_COVERED",
                "no orbit validity period covers the scene time",
            )
        };
    }

    let mut best = &candidates[0];
    for candidate in &candidates[1..] {
        let candidate_key = (
            product_priority(&candidate.orbit_type),
            candidate.creation_unix_seconds,
        );
        let best_key = (
            product_priority(&best.orbit_type),
            best.creation_unix_seconds,
        );
        if candidate_key > best_key {
            best = candidate;
        }
    }

    let mut issues = Vec::new();
    if candidates.len() > 1 {
        issues.push(OrbitMatchIssue {
            code: "ORBIT_MULTIPLE_CANDIDATES".to_string(),
            severity: OrbitIssueSeverity::Info,
            message: format!("{} candidate orbits cover the scene", candidates.len()),
            scene_id: Some(scene.scene_id.clone()),
            orbit_file: None,
            details: Map::from_iter([("count".to_string(), Value::from(candidates.len()))]),
        });
    }
    if let Some(code) = selected_code(&best.orbit_type) {
        issues.push(OrbitMatchIssue {
            code: code.to_string(),
            severity: OrbitIssueSeverity::Info,
            message: format!("selected {} orbit", orbit_type_label(&best.orbit_type)),
            scene_id: Some(scene.scene_id.clone()),
            orbit_file: Some(best.file_name.clone()),
            details: Map::new(),
        });
    }
    OrbitMatchResult {
        scene_id: scene.scene_id.clone(),
        matched_orbit: Some(best.clone()),
        candidate_orbits: candidates,
        issues,
        is_matched: true,
    }
}

fn unmatched(scene: &OrbitSceneInput, code: &str, message: &str) -> OrbitMatchResult {
    OrbitMatchResult {
        scene_id: scene.scene_id.clone(),
        matched_orbit: None,
        candidate_orbits: Vec::new(),
        issues: vec![OrbitMatchIssue {
            code: code.to_string(),
            severity: OrbitIssueSeverity::Warning,
            message: message.to_string(),
            scene_id: Some(scene.scene_id.clone()),
            orbit_file: None,
            details: Map::new(),
        }],
        is_matched: false,
    }
}

fn product_priority(orbit_type: &OrbitProductType) -> u8 {
    match orbit_type {
        OrbitProductType::POEORB => 3,
        OrbitProductType::MOEORB => 2,
        OrbitProductType::RESORB => 1,
        OrbitProductType::UNKNOWN => 0,
    }
}

fn selected_code(orbit_type: &OrbitProductType) -> Option<&'static str> {
    match orbit_type {
        OrbitProductType::POEORB => Some("ORBIT_SELECTED_POEORB"),
        OrbitProductType::MOEORB => Some("ORBIT_SELECTED_MOEORB"),
        OrbitProductType::RESORB => Some("ORBIT_SELECTED_RESORB"),
        OrbitProductType::UNKNOWN => None,
    }
}

fn orbit_type_label(orbit_type: &OrbitProductType) -> &'static str {
    match orbit_type {
        OrbitProductType::POEORB => "POEORB",
        OrbitProductType::MOEORB => "MOEORB",
        OrbitProductType::RESORB => "RESORB",
        OrbitProductType::UNKNOWN => "UNKNOWN",
    }
}

fn parse_orbit_filename(file_name: &str) -> Result<OrbitFileMetadata, &'static str> {
    let body = file_name
        .get(..file_name.len().checked_sub(4).ok_or(INVALID_FILENAME)?)
        .filter(|_| {
            file_name
                .get(file_name.len() - 4..)
                .is_some_and(|extension| extension.eq_ignore_ascii_case(".EOF"))
        })
        .ok_or(INVALID_FILENAME)?;
    const TIMESTAMP_SUFFIX_LENGTH: usize = 49;
    let prefix_end = body
        .len()
        .checked_sub(TIMESTAMP_SUFFIX_LENGTH)
        .ok_or(INVALID_FILENAME)?;
    let prefix = body.get(..prefix_end).ok_or(INVALID_FILENAME)?;
    let suffix = body.get(prefix_end..).ok_or(INVALID_FILENAME)?;
    if suffix.get(..1) != Some("_")
        || suffix.get(16..18) != Some("_V")
        || suffix.get(33..34) != Some("_")
    {
        return Err(INVALID_FILENAME);
    }

    let (platform, product_and_producer) = parse_platform(prefix)?;
    let (orbit_type, producer) = parse_product(product_and_producer)?;
    if producer.is_empty()
        || !producer
            .chars()
            .all(|character| character == '_' || character.is_alphanumeric())
    {
        return Err(INVALID_FILENAME);
    }

    let creation_unix_seconds = parse_utc_timestamp(suffix.get(1..16).ok_or(INVALID_FILENAME)?)?;
    let validity_start_unix_seconds =
        parse_utc_timestamp(suffix.get(18..33).ok_or(INVALID_FILENAME)?)?;
    let validity_stop_unix_seconds =
        parse_utc_timestamp(suffix.get(34..49).ok_or(INVALID_FILENAME)?)?;
    if validity_stop_unix_seconds <= validity_start_unix_seconds {
        return Err(INVALID_VALIDITY_WINDOW);
    }

    Ok(OrbitFileMetadata {
        file_name: file_name.to_string(),
        platform,
        orbit_type,
        creation_unix_seconds,
        validity_start_unix_seconds,
        validity_stop_unix_seconds,
    })
}

fn parse_platform(value: &str) -> Result<(OrbitPlatform, &str), &'static str> {
    for (prefix, platform) in [
        ("S1A_OPER_AUX_", OrbitPlatform::S1A),
        ("S1B_OPER_AUX_", OrbitPlatform::S1B),
        ("S1C_OPER_AUX_", OrbitPlatform::S1C),
        ("S1D_OPER_AUX_", OrbitPlatform::S1D),
    ] {
        if let Some(remainder) = value.strip_prefix(prefix) {
            return Ok((platform, remainder));
        }
    }
    Err(INVALID_FILENAME)
}

fn parse_product(value: &str) -> Result<(OrbitProductType, &str), &'static str> {
    for (prefix, orbit_type) in [
        ("POEORB_", OrbitProductType::POEORB),
        ("MOEORB_", OrbitProductType::MOEORB),
        ("RESORB_", OrbitProductType::RESORB),
    ] {
        if let Some(producer) = value.strip_prefix(prefix) {
            return Ok((orbit_type, producer));
        }
    }
    Err(INVALID_FILENAME)
}

fn parse_utc_timestamp(value: &str) -> Result<u64, &'static str> {
    if value.len() != 15
        || value.as_bytes().get(8) != Some(&b'T')
        || !value
            .bytes()
            .enumerate()
            .all(|(index, byte)| index == 8 || byte.is_ascii_digit())
    {
        return Err(INVALID_TIMESTAMP);
    }
    let year = parse_component(value, 0, 4)?;
    let month = parse_component(value, 4, 6)?;
    let day = parse_component(value, 6, 8)?;
    let hour = parse_component(value, 9, 11)?;
    let minute = parse_component(value, 11, 13)?;
    let second = parse_component(value, 13, 15)?;
    if year < 1970
        || !(1..=12).contains(&month)
        || day == 0
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return Err(INVALID_TIMESTAMP);
    }

    let days = days_from_civil(year as i64, month as i64, day as i64);
    u64::try_from(days * 86_400 + hour as i64 * 3_600 + minute as i64 * 60 + second as i64)
        .map_err(|_| INVALID_TIMESTAMP)
}

fn parse_component(value: &str, start: usize, stop: usize) -> Result<u32, &'static str> {
    value
        .get(start..stop)
        .ok_or(INVALID_TIMESTAMP)?
        .parse()
        .map_err(|_| INVALID_TIMESTAMP)
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: u32) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let adjusted_year = year - i64::from(month <= 2);
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let adjusted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn parse_error(file_name: &str, reason: &'static str) -> OrbitParseError {
    OrbitParseError {
        file_name: file_name.to_string(),
        error: ContractError {
            code: "ORB001".to_string(),
            user_message: "轨道文件名无效".to_string(),
            debug_context: Map::from_iter([(
                "reason".to_string(),
                Value::String(reason.to_string()),
            )]),
        },
    }
}
