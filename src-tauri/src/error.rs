use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct SarSearchRangeDiagnostic {
    pub start: Option<String>,
    pub end: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct SarSearchDateRangeDiagnostic {
    pub start: Option<String>,
    pub end: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct SarSearchFilterDiagnostic {
    pub platforms: Vec<String>,
    pub instrument: Option<String>,
    pub processing_levels: Vec<String>,
    pub product_families: Vec<String>,
    pub beam_modes: Vec<String>,
    pub polarizations: Vec<String>,
    pub flight_direction: Option<String>,
    pub look_direction: Option<String>,
    pub relative_orbit: Option<i64>,
    pub frame: Option<i64>,
    pub asf_frame: Option<String>,
    pub path: Option<i64>,
    pub track_range: Option<SarSearchRangeDiagnostic>,
    pub frame_range: Option<SarSearchRangeDiagnostic>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct SarSearchQueryShapeDiagnostic {
    pub platform_count: usize,
    pub processing_level_count: usize,
    pub product_family_count: usize,
    pub beam_mode_count: usize,
    pub polarization_count: usize,
    pub has_instrument: bool,
    pub has_relative_orbit: bool,
    pub has_frame: bool,
    pub has_asf_frame: bool,
    pub has_path: bool,
    pub has_track_range: bool,
    pub has_frame_range: bool,
    pub has_aoi_geojson: bool,
    pub has_aoi_wkt: bool,
    pub has_bbox: bool,
    pub has_burst: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct SarSearchFailureDiagnostic {
    pub operation: String,
    pub route: String,
    pub sanitized_reason: String,
    pub dataset: String,
    pub query_shape: SarSearchQueryShapeDiagnostic,
    pub aoi_state: String,
    pub date_range: SarSearchDateRangeDiagnostic,
    pub filter_state: SarSearchFilterDiagnostic,
    pub max_results: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ShellError {
    pub code: String,
    pub message_key: String,
    pub context: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<Box<SarSearchFailureDiagnostic>>,
}

impl ShellError {
    pub fn new(code: &'static str, message_key: &'static str) -> Self {
        Self {
            code: code.to_owned(),
            message_key: message_key.to_owned(),
            context: None,
            diagnostic: None,
        }
    }

    pub fn with_code(code: impl Into<String>, message_key: &'static str) -> Self {
        Self {
            code: code.into(),
            message_key: message_key.to_owned(),
            context: None,
            diagnostic: None,
        }
    }

    pub fn invalid_origin() -> Self {
        Self::new("GUI000", "shell.origin_not_allowed")
    }

    pub fn window_url_unavailable() -> Self {
        Self::new("GUI003", "shell.window_url_unavailable")
    }

    pub fn window_not_allowed() -> Self {
        Self::new("GUI000", "shell.window_not_allowed")
    }

    pub fn application_service(code: &'static str) -> Self {
        Self::new(code, "application_service.unavailable")
    }

    pub fn application_service_with_context(code: &'static str, context: String) -> Self {
        Self {
            code: code.to_owned(),
            message_key: "application_service.invalid_request".to_owned(),
            context: Some(context),
            diagnostic: None,
        }
    }

    pub fn application_service_with_unavailable_context(
        code: &'static str,
        context: String,
    ) -> Self {
        Self {
            code: code.to_owned(),
            message_key: "application_service.unavailable".to_owned(),
            context: Some(context),
            diagnostic: None,
        }
    }

    pub fn application_sar_search(
        code: &'static str,
        context: Option<String>,
        diagnostic: SarSearchFailureDiagnostic,
    ) -> Self {
        Self {
            code: code.to_owned(),
            message_key: "application_service.unavailable".to_owned(),
            context,
            diagnostic: Some(Box::new(diagnostic)),
        }
    }
}

impl std::fmt::Display for ShellError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message_key)
    }
}

impl std::error::Error for ShellError {}

#[cfg(test)]
mod tests {
    use super::ShellError;

    #[test]
    fn error_projection_is_bounded_and_path_free() {
        let error = ShellError::invalid_origin();
        let serialized = serde_json::to_string(&error).expect("error serializes");
        assert!(serialized.contains("GUI000"));
        assert!(serialized.contains("shell.origin_not_allowed"));
        assert!(!serialized.contains("E:\\\\"));
        assert!(error.context.is_none());
        assert!(error.diagnostic.is_none());
    }

    #[test]
    fn invalid_request_context_uses_invalid_request_message_key() {
        let error = ShellError::application_service_with_context(
            "V3_APPLICATION_INVALID_REQUEST",
            "standalone DEM bbox is invalid".to_owned(),
        );
        assert_eq!(error.message_key, "application_service.invalid_request");
        assert_eq!(
            error.context.as_deref(),
            Some("standalone DEM bbox is invalid")
        );
    }
}
