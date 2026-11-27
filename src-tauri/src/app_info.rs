use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub shell_kind: String,
    pub business_runtime: String,
}

pub fn desktop_app_info(name: impl Into<String>, version: impl Into<String>) -> AppInfo {
    AppInfo {
        name: name.into(),
        version: version.into(),
        shell_kind: "tauri_single_desktop_host".to_owned(),
        business_runtime: "application_service".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::desktop_app_info;

    #[test]
    fn desktop_info_identifies_the_single_business_host() {
        let info = desktop_app_info("EOStage", "2.2.0-beta.0");
        assert_eq!(info.shell_kind, "tauri_single_desktop_host");
        assert_eq!(info.business_runtime, "application_service");
    }
}
