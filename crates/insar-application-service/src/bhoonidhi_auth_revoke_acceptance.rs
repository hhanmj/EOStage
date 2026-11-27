//! Opt-in desktop acceptance only. No UI-supplied profile, token, expiry or endpoint.
use super::*;
use insar_core::TaskLifecycleStatus;

fn rejected() -> ApplicationServiceError {
    ApplicationServiceError::NisarSubsetV2RequestInvalid
}

impl ApplicationServiceConfig {
    pub fn with_bhoonidhi_auth_revoke_acceptance_profile(
        mut self,
        profile: PathBuf,
    ) -> Result<Self, ApplicationServiceError> {
        let profile = profile.canonicalize().map_err(|_| rejected())?;
        let store = self
            .store_path()
            .ok_or_else(rejected)?
            .canonicalize()
            .map_err(|_| rejected())?;
        let expected = profile
            .join("localappdata/EOStage/v3/tasks.sqlite3")
            .canonicalize()
            .map_err(|_| rejected())?;
        // Only a named canonical hand-test profile is eligible, never the Owner's normal Store.
        if !self.is_business()
            || store != expected
            || profile
                .parent()
                .and_then(Path::file_name)
                .and_then(|v| v.to_str())
                != Some("profiles")
            || profile
                .parent()
                .and_then(Path::parent)
                .and_then(Path::file_name)
                .and_then(|v| v.to_str())
                != Some("desktop-handtest")
        {
            return Err(rejected());
        }
        self.auth_revoke_acceptance_profile = Some(profile);
        Ok(self)
    }
}

impl BhoonidhiSessionRuntime {
    pub(super) fn bind_acceptance_profile(
        mut self,
        profile: &Path,
    ) -> Result<Self, ApplicationServiceError> {
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            let lease = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .share_mode(0)
                .open(profile.join("bhoonidhi-auth-revoke-acceptance.lock"))
                .map_err(|_| rejected())?;
            let provider = BhoonidhiSessionTokenProvider::new(Arc::clone(&self.session))
                .with_desktop_login()
                .with_auth_revoke_acceptance_profile(profile.to_owned())
                .map_err(|_| rejected())?;
            self.provider = Arc::new(provider);
            self.acceptance_lease = Some(Arc::new(lease));
            Ok(self)
        }
        #[cfg(not(windows))]
        {
            let _ = profile;
            Err(rejected())
        }
    }
}

impl ApplicationService {
    fn auth_revoke_acceptance_session(
        &self,
        task_id: &str,
    ) -> Result<(BhoonidhiSessionRuntime, PathBuf), ApplicationServiceError> {
        self.ensure_running()?;
        let profile = self
            .config
            .auth_revoke_acceptance_profile
            .clone()
            .ok_or_else(rejected)?;
        self.with_business_runtime(|runtime| {
            let store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            let tasks = store
                .load_scheduler_tasks()
                .map_err(ApplicationServiceError::Store)?;
            let target = tasks
                .iter()
                .find(|task| task.snapshot.task_id == task_id)
                .ok_or_else(rejected)?;
            if target.runtime.status != TaskLifecycleStatus::Paused
                || !target.runtime.task_paused
                || target.snapshot.candidates.len() != 1
                || target.snapshot.candidates[0].provider_id != BHOONIDHI_PROVIDER_ID
                || tasks.iter().any(|task| {
                    !task.runtime.active_item_ids.is_empty()
                        || !task.runtime.queued_item_ids.is_empty()
                })
            {
                return Err(rejected());
            }
            Ok(())
        })?;
        let session = self
            .bhoonidhi_session
            .lock()
            .map_err(|_| ApplicationServiceError::StatePoisoned)?
            .as_ref()
            .filter(|session| session.acceptance_lease.is_some())
            .cloned()
            .ok_or_else(rejected)?;
        Ok((session, profile))
    }

    pub fn bhoonidhi_auth_revoke_acceptance_generation(
        &self,
        task_id: &str,
    ) -> Result<u64, ApplicationServiceError> {
        let (session, profile) = self.auth_revoke_acceptance_session(task_id)?;
        session
            .provider
            .acceptance_current_generation(&profile)
            .map_err(auth_error)
    }

    pub fn bhoonidhi_auth_revoke_acceptance_once(
        &self,
        task_id: &str,
        generation: u64,
    ) -> Result<(), ApplicationServiceError> {
        let (session, profile) = self.auth_revoke_acceptance_session(task_id)?;
        session
            .provider
            .acceptance_revoke_current_generation(&profile, generation)
            .map_err(auth_error)
    }
}

fn auth_error(error: insar_download::BhoonidhiAuthError) -> ApplicationServiceError {
    ApplicationServiceError::Provider(
        ContractError::new(error.code(), error.to_string(), Map::new()).expect("fixed auth error"),
    )
}
