//! Deterministic search guardrails, volatile-field fingerprinting, and
//! zero-result circuit breaker persisted across run checkpoints.
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Volatile identifiers and timestamps excluded from `Executor::record`
/// `summary_sha256` so repeated polls with changing request IDs or elapsed
/// milliseconds still produce a stable semantic fingerprint.
const VOLATILE_FINGERPRINT_KEYS: &[&str] = &[
    "request_id",
    "batch_id",
    "search_batch_id",
    "elapsed_ms",
    "duration_ms",
    "started_at",
    "updated_at",
    "created_at",
    "observed_at",
];

pub fn strip_volatile_fields(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(strip_volatile_fields).collect()),
        Value::Object(fields) => {
            let mut clean = Map::new();
            for (key, item) in fields {
                if VOLATILE_FINGERPRINT_KEYS.contains(&key.as_str()) {
                    continue;
                }
                clean.insert(key.clone(), strip_volatile_fields(item));
            }
            Value::Object(clean)
        }
        _ => value.clone(),
    }
}

pub fn is_sentinel1_resource(resource_id: &str) -> bool {
    let lower = resource_id.trim().to_ascii_lowercase();
    lower.starts_with("sentinel-1") || lower.starts_with("s1_") || lower.contains("sentinel-1")
}

/// Returns true when `[west, south, east, north]` lies inside a major
/// non-polar continental landmass (`[-55, 60]` latitude band) where Sentinel-1
/// acquires IW DV (`VV` / `VV+VH`) rather than polar/maritime `HH` / `HH+HV`.
pub fn is_continental_non_polar_land(bbox: [f64; 4]) -> bool {
    let [west, south, east, north] = bbox;
    if !west.is_finite()
        || !south.is_finite()
        || !east.is_finite()
        || !north.is_finite()
        || west > east
        || south > north
    {
        return false;
    }
    if south < -55.0 || north > 60.0 {
        return false;
    }
    let lon = 0.5 * (west + east);
    let lat = 0.5 * (south + north);
    // Mainland Asia (including all continental China provinces such as Sichuan)
    let mainland_asia = (60.0..=135.0).contains(&lon) && (18.0..=55.0).contains(&lat);
    // Continental Europe & Middle East
    let europe_me = (-9.5..=60.0).contains(&lon) && (30.0..=58.0).contains(&lat);
    // Continental Africa
    let africa = (-17.0..=51.0).contains(&lon) && (-35.0..=37.0).contains(&lat);
    // Continental North America (non-Arctic)
    let north_america = (-125.0..=-70.0).contains(&lon) && (25.0..=55.0).contains(&lat);
    // Continental South America (non-sub-Antarctic)
    let south_america = (-81.0..=-35.0).contains(&lon) && (-50.0..=12.0).contains(&lat);
    // Continental Australia
    let australia = (113.0..=153.5).contains(&lon) && (-39.5..=-11.0).contains(&lat);

    mainland_asia || europe_me || africa || north_america || south_america || australia
}

/// Detects whether the user explicitly asked for `HH` or `HH+HV` polarization
/// (with ASCII alphabetic word-boundary checks so words like `through` do not match).
pub fn user_explicitly_requested_hh(user_text: &str) -> bool {
    let lower = user_text.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    for (idx, _) in lower.match_indices("hh") {
        let prev_alpha = idx > 0 && bytes[idx - 1].is_ascii_alphabetic();
        let end = idx + 2;
        let next_alpha = end < bytes.len() && bytes[end].is_ascii_alphabetic();
        if !prev_alpha && !next_alpha {
            return true;
        }
    }
    false
}

pub fn extract_polarizations(input: &Value) -> Vec<String> {
    let candidates = [
        input.pointer("/filters/polarizations"),
        input.pointer("/filters/sar/polarizations"),
        input.pointer("/sar/polarizations"),
        input.get("polarizations"),
    ];
    let mut out: Vec<String> = candidates
        .into_iter()
        .flatten()
        .find_map(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(|pol| pol.trim().to_ascii_uppercase())
                .filter(|pol| !pol.is_empty())
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out.dedup();
    out
}

pub fn is_hh_only_polarizations(polarizations: &[String]) -> bool {
    !polarizations.is_empty()
        && polarizations
            .iter()
            .all(|pol| matches!(pol.as_str(), "HH" | "HH+HV" | "HV"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum S1PolarizationDecision {
    Allow,
    /// User explicitly asked for HH over continental non-polar land: allow one
    /// live verification search, then halt retries immediately if 0 scenes return.
    AllowOnceMarkDeterministic,
    /// Model selected HH/HH+HV on continental land without user asking for HH.
    RefuseUnpromptedLandHh,
}

pub fn evaluate_s1_land_polarization(
    resource_id: &str,
    aoi_bbox: Option<[f64; 4]>,
    input: &Value,
    user_text: &str,
) -> S1PolarizationDecision {
    if !is_sentinel1_resource(resource_id) {
        return S1PolarizationDecision::Allow;
    }
    let Some(bbox) = aoi_bbox else {
        return S1PolarizationDecision::Allow;
    };
    if !is_continental_non_polar_land(bbox) {
        return S1PolarizationDecision::Allow;
    }
    let pols = extract_polarizations(input);
    if !is_hh_only_polarizations(&pols) {
        return S1PolarizationDecision::Allow;
    }
    if user_explicitly_requested_hh(user_text) {
        S1PolarizationDecision::AllowOnceMarkDeterministic
    } else {
        S1PolarizationDecision::RefuseUnpromptedLandHh
    }
}

/// Semantic key for a `search_start` step, deliberately ignoring `time_range`
/// and `cloud_cover_max` tweaks so repeated date-window expansions hit the
/// same zero-result budget.
pub fn semantic_search_bucket_key(input: &Value, resolved_bbox: Option<[f64; 4]>) -> String {
    let resource_id = input
        .get("resource_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let bbox_part = resolved_bbox
        .or_else(|| bbox_from_input_aoi(input))
        .map(|[w, s, e, n]| format!("{w:.2},{s:.2},{e:.2},{n:.2}"))
        .unwrap_or_else(|| "aoi".to_owned());
    let pols = extract_polarizations(input).join("+");
    let beam_modes = [
        input.pointer("/filters/beam_modes"),
        input.pointer("/filters/sar/beam_modes"),
        input.pointer("/sar/beam_modes"),
    ]
    .into_iter()
    .flatten()
    .find_map(Value::as_array)
    .map(|arr| {
        let mut items: Vec<String> = arr
            .iter()
            .filter_map(Value::as_str)
            .map(|s| s.trim().to_ascii_uppercase())
            .collect();
        items.sort();
        items.dedup();
        items.join("+")
    })
    .unwrap_or_default();
    let direction = [
        input.pointer("/filters/flight_direction"),
        input.pointer("/filters/sar/flight_direction"),
        input.pointer("/sar/flight_direction"),
    ]
    .into_iter()
    .flatten()
    .find_map(Value::as_str)
    .unwrap_or("")
    .trim()
    .to_ascii_lowercase();
    format!("{resource_id}|{bbox_part}|{pols}|{beam_modes}|{direction}")
}

fn bbox_from_input_aoi(input: &Value) -> Option<[f64; 4]> {
    let aoi = input.get("aoi")?;
    if let Some(arr) = aoi.get("bbox").and_then(Value::as_array) {
        if arr.len() == 4 {
            return Some([
                arr[0].as_f64()?,
                arr[1].as_f64()?,
                arr[2].as_f64()?,
                arr[3].as_f64()?,
            ]);
        }
    }
    None
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BucketZeroState {
    pub zero_count: u32,
    pub halted: bool,
    #[serde(default)]
    pub deterministic_code: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingBatchMeta {
    pub bucket_key: String,
    #[serde(default)]
    pub deterministic_s1_land_hh: bool,
}

/// Persisted in `RunState` so pause/resume checkpoints retain search retry budgets.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchCircuitBreaker {
    #[serde(default)]
    pub buckets: BTreeMap<String, BucketZeroState>,
    #[serde(default)]
    pub batch_meta: BTreeMap<String, PendingBatchMeta>,
    #[serde(default)]
    pub observed_zero_batches: BTreeSet<String>,
    #[serde(default)]
    pub total_zero_searches: u32,
    #[serde(default)]
    pub search_halted: bool,
}

impl SearchCircuitBreaker {
    /// Records a newly started search batch so its later `search_progress`
    /// result can be attributed to the right semantic bucket.
    pub fn register_batch(
        &mut self,
        batch_id: &str,
        bucket_key: String,
        deterministic_s1_land_hh: bool,
    ) {
        if batch_id.is_empty() {
            return;
        }
        self.batch_meta.insert(
            batch_id.to_owned(),
            PendingBatchMeta {
                bucket_key,
                deterministic_s1_land_hh,
            },
        );
    }

    /// Checks whether a proposed `search_start` in `bucket_key` is already
    /// halted by the zero-result circuit breaker. Returns a structured JSON
    /// payload if tripped.
    pub fn check_before_search(&self, bucket_key: &str) -> Option<Value> {
        if let Some(bucket) = self.buckets.get(bucket_key) {
            if bucket.halted {
                let code = bucket
                    .deterministic_code
                    .as_deref()
                    .unwrap_or("ZERO_RESULT_RETRY_EXHAUSTED");
                return Some(json!({
                    "status": "circuit_breaker_tripped",
                    "code": code,
                    "search_halted": true,
                    "retry_budget_remaining": 0,
                    "zero_result_searches": bucket.zero_count,
                    "guidance": Self::halt_guidance(Some(code)),
                }));
            }
        }
        if self.search_halted {
            return Some(json!({
                "status": "circuit_breaker_tripped",
                "code": "ZERO_RESULT_RETRY_EXHAUSTED",
                "search_halted": true,
                "retry_budget_remaining": 0,
                "zero_result_searches": self.total_zero_searches,
                "guidance": Self::halt_guidance(None),
            }));
        }
        None
    }

    /// Inspects a `search_progress` summary and, when a terminal search
    /// finished with `0` items, updates retry budgets and injects
    /// `zero_result_diagnostics` into `summary`.
    pub fn observe_search_progress(&mut self, batch_id: &str, summary: &mut Value) {
        if !is_completed_zero_result_progress(summary) {
            return;
        }
        let key_id = if !batch_id.is_empty() {
            batch_id.to_owned()
        } else {
            summary
                .get("request_id")
                .or_else(|| summary.get("batch_id"))
                .or_else(|| summary.get("search_batch_id"))
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned()
        };
        let meta = self
            .batch_meta
            .get(&key_id)
            .cloned()
            .unwrap_or_else(|| PendingBatchMeta {
                bucket_key: "default".to_owned(),
                deterministic_s1_land_hh: false,
            });
        let first_observation = self.observed_zero_batches.insert(key_id);
        let bucket = self.buckets.entry(meta.bucket_key).or_default();
        if first_observation {
            bucket.zero_count = bucket.zero_count.saturating_add(1);
            self.total_zero_searches = self.total_zero_searches.saturating_add(1);
            if meta.deterministic_s1_land_hh {
                bucket.halted = true;
                bucket.deterministic_code = Some("S1_LAND_HH_UNAVAILABLE".to_owned());
                self.search_halted = true;
            } else if bucket.zero_count >= 2 || self.total_zero_searches >= 2 {
                bucket.halted = true;
                self.search_halted = true;
            }
        }
        let halted = bucket.halted || self.search_halted;
        let retry_remaining = if halted {
            0
        } else {
            2u32.saturating_sub(bucket.zero_count)
        };
        let code = bucket.deterministic_code.as_deref();
        let diagnostics = if code == Some("S1_LAND_HH_UNAVAILABLE") {
            json!({
                "code": "S1_LAND_HH_UNAVAILABLE",
                "deterministic_cause": true,
                "zero_result_searches": bucket.zero_count,
                "retry_budget_remaining": 0,
                "search_halted": true,
                "recommended_polarizations": ["VV", "VV+VH"],
                "guidance": Self::halt_guidance(Some("S1_LAND_HH_UNAVAILABLE")),
            })
        } else if halted {
            json!({
                "code": "ZERO_RESULT_RETRY_EXHAUSTED",
                "deterministic_cause": false,
                "zero_result_searches": bucket.zero_count,
                "retry_budget_remaining": 0,
                "search_halted": true,
                "guidance": Self::halt_guidance(None),
            })
        } else {
            json!({
                "code": "ZERO_RESULTS_FIRST_ATTEMPT",
                "deterministic_cause": false,
                "zero_result_searches": bucket.zero_count,
                "retry_budget_remaining": retry_remaining,
                "search_halted": false,
                "guidance": "0 scenes matched. At most 1 broader retry is permitted if a constraint was overly narrow (for example Sentinel-1 single VV excludes dual-pol VV+VH on ASF, so omit polarizations or include VV+VH); otherwise conclude now and explain why no scenes matched.",
            })
        };
        if let Value::Object(map) = summary {
            map.insert("zero_result_diagnostics".to_owned(), diagnostics);
        }
    }

    fn halt_guidance(code: Option<&str>) -> &'static str {
        match code {
            Some("S1_LAND_HH_UNAVAILABLE") => {
                "Sentinel-1 IW over continental non-polar land acquires VV or VV+VH only; HH and HH+HV are polar and maritime modes. Widening time_range will still return 0 scenes. Stop searching now and explain this clearly to the user or ask whether to search VV / VV+VH instead."
            }
            _ => {
                "Zero-result retry budget is exhausted. Do not propose another search_start; summarize what was checked and offer concrete next options to the user."
            }
        }
    }
}

fn is_completed_zero_result_progress(summary: &Value) -> bool {
    let stage = summary.get("stage").and_then(Value::as_str).unwrap_or("");
    let status = summary
        .get("status")
        .or_else(|| summary.get("state"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let completed =
        stage == "done" || matches!(status, "completed" | "complete" | "succeeded" | "done");
    if !completed {
        return false;
    }
    if let Some(total) = summary
        .get("items_kept")
        .or_else(|| summary.get("total_candidates"))
        .or_else(|| summary.get("candidate_count"))
        .or_else(|| summary.get("total"))
        .and_then(Value::as_u64)
    {
        return total == 0;
    }
    if let Some(items) = summary
        .get("items")
        .or_else(|| summary.get("candidates"))
        .and_then(Value::as_array)
    {
        return items.is_empty();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_volatile_fields_ignores_ids_and_timestamps() {
        let a = json!({
            "request_id": "req-1",
            "batch_id": "batch-1",
            "elapsed_ms": 120,
            "observed_at": "2026-11-11T00:00:00Z",
            "status": "running",
            "nested": { "search_batch_id": "batch-1", "count": 0 }
        });
        let b = json!({
            "request_id": "req-2",
            "batch_id": "batch-2",
            "elapsed_ms": 980,
            "observed_at": "2026-11-11T00:00:05Z",
            "status": "running",
            "nested": { "search_batch_id": "batch-2", "count": 0 }
        });
        assert_eq!(strip_volatile_fields(&a), strip_volatile_fields(&b));
    }

    #[test]
    fn s1_continental_land_hh_guardrail_distinguishes_prompted_and_unprompted() {
        let sichuan = [97.35, 26.05, 108.55, 34.32];
        let greenland = [-55.0, 62.0, -20.0, 82.0];
        let hh_input = json!({
            "resource_id": "sentinel-1-slc",
            "sar": { "polarizations": ["HH"] }
        });
        let vv_input = json!({
            "resource_id": "sentinel-1-slc",
            "sar": { "polarizations": ["VV", "VV+VH"] }
        });

        assert_eq!(
            evaluate_s1_land_polarization(
                "sentinel-1-slc",
                Some(sichuan),
                &hh_input,
                "帮我查一下四川省的 Sentinel-1 数据"
            ),
            S1PolarizationDecision::RefuseUnpromptedLandHh
        );
        assert_eq!(
            evaluate_s1_land_polarization(
                "sentinel-1-slc",
                Some(sichuan),
                &hh_input,
                "帮我查一下四川省 HH 极化的 Sentinel-1 数据"
            ),
            S1PolarizationDecision::AllowOnceMarkDeterministic
        );
        assert_eq!(
            evaluate_s1_land_polarization(
                "sentinel-1-slc",
                Some(greenland),
                &hh_input,
                "Search Greenland Sentinel-1"
            ),
            S1PolarizationDecision::Allow
        );
        assert_eq!(
            evaluate_s1_land_polarization(
                "sentinel-1-slc",
                Some(sichuan),
                &vv_input,
                "帮我查一下四川省的 Sentinel-1 数据"
            ),
            S1PolarizationDecision::Allow
        );
        assert!(!user_explicitly_requested_hh(
            "search through Sichuan scenes"
        ));
    }

    #[test]
    fn circuit_breaker_ignores_time_range_and_halts_after_one_retry() {
        let sichuan = [97.35, 26.05, 108.55, 34.32];
        let k1 = semantic_search_bucket_key(
            &json!({
                "resource_id": "landsat-c2-l2",
                "time_range": { "start": "2024-01-01T00:00:00Z", "end": "2024-02-01T00:00:00Z" }
            }),
            Some(sichuan),
        );
        let k2 = semantic_search_bucket_key(
            &json!({
                "resource_id": "landsat-c2-l2",
                "time_range": { "start": "2023-01-01T00:00:00Z", "end": "2025-01-01T00:00:00Z" }
            }),
            Some(sichuan),
        );
        assert_eq!(k1, k2);

        let mut breaker = SearchCircuitBreaker::default();
        breaker.register_batch("b1", k1.clone(), false);
        let mut p1 = json!({ "batch_id": "b1", "status": "completed", "total_candidates": 0 });
        breaker.observe_search_progress("b1", &mut p1);
        assert_eq!(p1["zero_result_diagnostics"]["retry_budget_remaining"], 1);
        assert!(breaker.check_before_search(&k2).is_none());

        breaker.register_batch("b2", k2.clone(), false);
        let mut p2 = json!({ "batch_id": "b2", "status": "completed", "total_candidates": 0 });
        breaker.observe_search_progress("b2", &mut p2);
        assert_eq!(p2["zero_result_diagnostics"]["retry_budget_remaining"], 0);
        assert_eq!(p2["zero_result_diagnostics"]["search_halted"], true);
        let tripped = breaker
            .check_before_search(&k2)
            .expect("tripped on 3rd search");
        assert_eq!(tripped["status"], "circuit_breaker_tripped");
    }
}
