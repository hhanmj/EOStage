use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UiSmokeReport {
    pub react_mounted: bool,
    pub workbench_rendered: bool,
    pub application_service_ready: bool,
    pub leaflet_initialized: bool,
    pub map_width: u32,
    pub map_height: u32,
    pub map_resized: bool,
    pub tile_error_handled: bool,
    pub aoi_overlay_rendered: bool,
    pub unhandled_js_errors: u32,
    pub csp_violations: u32,
    pub v3_application_unavailable_seen: bool,
    pub region_created_or_selected: bool,
    pub aoi_bind_command_completed: bool,
    pub aoi_persisted_in_snapshot: bool,
    pub aoi_restored_after_restart: bool,
    pub search_request_uses_persisted_aoi: bool,
    #[serde(default)]
    pub workspace_open_completed: bool,
    #[serde(default)]
    pub project_region_context_completed: bool,
    #[serde(default)]
    pub per_region_aoi_completed: bool,
    #[serde(default)]
    pub first_region_aoi_id: Option<String>,
    #[serde(default)]
    pub second_region_aoi_id: Option<String>,
    #[serde(default)]
    pub first_region_aoi_revision: Option<u64>,
    #[serde(default)]
    pub second_region_aoi_revision: Option<u64>,
    #[serde(default)]
    pub dem_plan_task_id: Option<String>,
    #[serde(default)]
    pub dem_task_control_completed: bool,
    #[serde(default)]
    pub dem_task_control_revision: Option<u64>,
    #[serde(default)]
    pub dem_task_completed_id: Option<String>,
    #[serde(default)]
    pub dem_conversion_task_id: Option<String>,
    #[serde(default)]
    pub dem_conversion_completed_id: Option<String>,
    #[serde(default)]
    pub orbit_task_id: Option<String>,
    #[serde(default)]
    pub orbit_completed_id: Option<String>,
    #[serde(default)]
    pub gacos_plan_offline: bool,
    #[serde(default)]
    pub gacos_dry_run_offline: bool,
}

impl UiSmokeReport {
    fn validate(&self) -> Result<(), &'static str> {
        if !self.react_mounted
            || !self.workbench_rendered
            || !self.application_service_ready
            || !self.leaflet_initialized
        {
            return Err("UI_SMOKE_INCOMPLETE");
        }
        if self.map_width == 0 || self.map_height == 0 {
            return Err("UI_SMOKE_INVALID_DIMENSIONS");
        }
        if !self.map_resized || !self.tile_error_handled || !self.aoi_overlay_rendered {
            return Err("UI_SMOKE_MAP_COMPONENT_INCOMPLETE");
        }
        if self.unhandled_js_errors != 0 || self.csp_violations != 0 {
            return Err("UI_SMOKE_RUNTIME_ERRORS");
        }
        if self.v3_application_unavailable_seen {
            return Err("V3_APPLICATION_UNAVAILABLE");
        }
        if !self.region_created_or_selected
            || !self.aoi_bind_command_completed
            || !self.aoi_persisted_in_snapshot
            || !self.aoi_restored_after_restart
            || !self.search_request_uses_persisted_aoi
        {
            return Err("UI_SMOKE_AOI_BUSINESS_INCOMPLETE");
        }
        let non_empty =
            |value: &Option<String>| value.as_ref().is_some_and(|item| !item.trim().is_empty());
        if !self.workspace_open_completed
            || !self.project_region_context_completed
            || !self.per_region_aoi_completed
            || !non_empty(&self.first_region_aoi_id)
            || !non_empty(&self.second_region_aoi_id)
            || self.first_region_aoi_id == self.second_region_aoi_id
            || self.first_region_aoi_revision.is_none()
            || self.second_region_aoi_revision.is_none()
            || !non_empty(&self.dem_plan_task_id)
            || !self.dem_task_control_completed
            || self.dem_task_control_revision.is_none()
            || !non_empty(&self.dem_task_completed_id)
            || !non_empty(&self.dem_conversion_task_id)
            || !non_empty(&self.dem_conversion_completed_id)
            || !non_empty(&self.orbit_task_id)
            || !non_empty(&self.orbit_completed_id)
            || !self.gacos_plan_offline
            || !self.gacos_dry_run_offline
        {
            return Err("UI_SMOKE_FORMAL_OPERATION_INCOMPLETE");
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UiSmokePhase {
    #[default]
    PhaseB,
    PhaseA,
}

impl UiSmokePhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PhaseA => "phase_a",
            Self::PhaseB => "phase_b",
        }
    }
}

#[derive(Clone, Default)]
pub struct UiSmokeState {
    enabled: bool,
    phase: UiSmokePhase,
    report: Arc<Mutex<Option<UiSmokeReport>>>,
    last_checkpoint: Arc<Mutex<Option<String>>>,
    phase_a_complete: Arc<AtomicBool>,
}

impl UiSmokeState {
    pub fn disabled() -> Self {
        Self::default()
    }

    pub fn enabled() -> Self {
        Self {
            enabled: true,
            phase: UiSmokePhase::PhaseB,
            report: Arc::new(Mutex::new(None)),
            last_checkpoint: Arc::new(Mutex::new(None)),
            phase_a_complete: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn phase_a() -> Self {
        Self {
            enabled: true,
            phase: UiSmokePhase::PhaseA,
            report: Arc::new(Mutex::new(None)),
            last_checkpoint: Arc::new(Mutex::new(None)),
            phase_a_complete: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn phase(&self) -> UiSmokePhase {
        self.phase
    }

    pub fn checkpoint(&self, step: String) {
        if !self.enabled {
            return;
        }
        if let Ok(mut checkpoint) = self.last_checkpoint.lock() {
            *checkpoint = Some(step);
        }
    }

    pub fn last_checkpoint(&self) -> Option<String> {
        self.last_checkpoint
            .lock()
            .ok()
            .and_then(|checkpoint| checkpoint.clone())
    }

    pub fn complete_phase_a(&self) {
        self.phase_a_complete.store(true, Ordering::Release);
    }

    pub fn phase_a_completed(&self) -> bool {
        self.phase_a_complete.load(Ordering::Acquire)
    }

    pub fn accept(&self, report: UiSmokeReport) -> Result<(), &'static str> {
        if !self.enabled {
            return Err("UI_SMOKE_NOT_ENABLED");
        }
        report.validate()?;
        let mut stored = self.report.lock().map_err(|_| "UI_SMOKE_STATE_POISONED")?;
        if stored.is_some() {
            return Err("UI_SMOKE_ALREADY_REPORTED");
        }
        *stored = Some(report);
        Ok(())
    }

    pub fn report(&self) -> Option<UiSmokeReport> {
        self.report.lock().ok().and_then(|stored| stored.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::{UiSmokeReport, UiSmokeState};

    fn valid_report() -> UiSmokeReport {
        UiSmokeReport {
            react_mounted: true,
            workbench_rendered: true,
            application_service_ready: true,
            leaflet_initialized: true,
            map_width: 100,
            map_height: 100,
            map_resized: true,
            tile_error_handled: true,
            aoi_overlay_rendered: true,
            unhandled_js_errors: 0,
            csp_violations: 0,
            v3_application_unavailable_seen: false,
            region_created_or_selected: true,
            aoi_bind_command_completed: true,
            aoi_persisted_in_snapshot: true,
            aoi_restored_after_restart: true,
            search_request_uses_persisted_aoi: true,
            workspace_open_completed: true,
            project_region_context_completed: true,
            per_region_aoi_completed: true,
            first_region_aoi_id: Some("region-a".to_owned()),
            second_region_aoi_id: Some("region-b".to_owned()),
            first_region_aoi_revision: Some(1),
            second_region_aoi_revision: Some(1),
            dem_plan_task_id: Some("dem".to_owned()),
            dem_task_control_completed: true,
            dem_task_control_revision: Some(2),
            dem_task_completed_id: Some("dem".to_owned()),
            dem_conversion_task_id: Some("conversion".to_owned()),
            dem_conversion_completed_id: Some("conversion".to_owned()),
            orbit_task_id: Some("orbit".to_owned()),
            orbit_completed_id: Some("orbit".to_owned()),
            gacos_plan_offline: true,
            gacos_dry_run_offline: true,
        }
    }

    #[test]
    fn only_a_complete_error_free_report_is_accepted() {
        let state = UiSmokeState::enabled();
        assert!(state.accept(valid_report()).is_ok());
        assert!(state.report().is_some());
    }

    #[test]
    fn marker_only_report_is_rejected_without_typed_loopback_evidence() {
        let mut report = valid_report();
        report.first_region_aoi_id = None;
        report.second_region_aoi_id = None;
        report.first_region_aoi_revision = None;
        report.second_region_aoi_revision = None;
        report.dem_plan_task_id = None;
        report.dem_task_control_revision = None;
        report.dem_task_completed_id = None;
        report.dem_conversion_task_id = None;
        report.dem_conversion_completed_id = None;
        report.orbit_task_id = None;
        report.orbit_completed_id = None;
        report.gacos_plan_offline = false;
        report.gacos_dry_run_offline = false;

        assert_eq!(
            UiSmokeState::enabled().accept(report),
            Err("UI_SMOKE_FORMAL_OPERATION_INCOMPLETE")
        );
    }

    #[test]
    fn normal_runtime_cannot_submit_an_internal_ui_report() {
        assert_eq!(
            UiSmokeState::disabled().accept(valid_report()),
            Err("UI_SMOKE_NOT_ENABLED")
        );
    }

    #[test]
    fn phase_a_completion_is_guarded_and_distinct_from_the_report() {
        let state = UiSmokeState::phase_a();
        assert_eq!(state.phase(), super::UiSmokePhase::PhaseA);
        assert!(!state.phase_a_completed());
        state.complete_phase_a();
        assert!(state.phase_a_completed());
        assert!(state.report().is_none());

        let phase_b = UiSmokeState::enabled();
        assert_eq!(phase_b.phase(), super::UiSmokePhase::PhaseB);
        assert!(!phase_b.phase_a_completed());
    }
}
