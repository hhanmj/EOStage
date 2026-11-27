use insar_core::NisarSubsetSourceProductIdentityV2;
use insar_provider::catalog::{
    BHOONIDHI_NATIVE_COLLECTION_ID, BHOONIDHI_PROFILE_ID, BHOONIDHI_PROVIDER_ID,
    NISAR_NATIVE_COLLECTION_ID, NISAR_PROFILE_ID, NISAR_PROVIDER_ID,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NisarScienceRoot {
    Lsar,
    Ssar,
}

impl NisarScienceRoot {
    pub(crate) fn from_source_identity(
        identity: &NisarSubsetSourceProductIdentityV2,
    ) -> Option<Self> {
        match (
            identity.provider_id.as_str(),
            identity.collection_id.as_str(),
            identity.collection_profile_id.as_str(),
        ) {
            (NISAR_PROVIDER_ID, NISAR_NATIVE_COLLECTION_ID, NISAR_PROFILE_ID) => Some(Self::Lsar),
            (BHOONIDHI_PROVIDER_ID, BHOONIDHI_NATIVE_COLLECTION_ID, BHOONIDHI_PROFILE_ID) => {
                Some(Self::Ssar)
            }
            _ => None,
        }
    }

    pub(crate) const fn science_root(self) -> &'static str {
        match self {
            Self::Lsar => "/science/LSAR",
            Self::Ssar => "/science/SSAR",
        }
    }

    pub(crate) const fn swath(self) -> &'static str {
        match self {
            Self::Lsar => "LSAR_RSLC",
            Self::Ssar => "SSAR_RSLC",
        }
    }

    pub(crate) const fn radar_band(self) -> &'static str {
        match self {
            Self::Lsar => "L",
            Self::Ssar => "S",
        }
    }

    pub(crate) fn identification_path(self) -> String {
        format!("{}/identification", self.science_root())
    }

    pub(crate) fn swaths_path(self) -> String {
        format!("{}/RSLC/swaths", self.science_root())
    }

    pub(crate) fn geolocation_path(self) -> String {
        format!("{}/RSLC/metadata/geolocationGrid", self.science_root())
    }

    pub(crate) fn metadata_path(self) -> String {
        format!("{}/RSLC/metadata", self.science_root())
    }

    pub(crate) fn orbit_path(self) -> String {
        format!("{}/orbit", self.metadata_path())
    }

    pub(crate) fn attitude_path(self) -> String {
        format!("{}/attitude", self.metadata_path())
    }

    pub(crate) fn processing_path(self) -> String {
        format!("{}/processingInformation", self.metadata_path())
    }

    pub(crate) fn calibration_path(self) -> String {
        format!("{}/calibrationInformation", self.metadata_path())
    }

    pub(crate) fn provenance_path(self) -> String {
        format!("{}/remoteSubsetProvenance", self.metadata_path())
    }

    pub(crate) fn derived_root(self) -> String {
        format!("{}/RSLC/derivedSubsets", self.science_root())
    }

    pub(crate) fn selected_science_path(self, frequency: &str, polarization: &str) -> String {
        format!("{}/frequency{frequency}/{polarization}", self.swaths_path())
    }

    pub(crate) fn zero_doppler_time_path(self) -> String {
        format!("{}/zeroDopplerTime", self.swaths_path())
    }

    pub(crate) fn slant_range_path(self, frequency: &str) -> String {
        format!("{}/frequency{frequency}/slantRange", self.swaths_path())
    }

    pub(crate) fn mask_path(self, frequency: &str) -> String {
        format!(
            "{}/frequency{frequency}/inputDataExceptionMask",
            self.swaths_path()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::NisarScienceRoot;
    use insar_core::NisarSubsetQualityStatusV2;
    use insar_core::NisarSubsetSourceProductIdentityV2;
    use insar_provider::catalog::{
        BHOONIDHI_NATIVE_COLLECTION_ID, BHOONIDHI_PROFILE_ID, BHOONIDHI_PROVIDER_ID,
        NISAR_NATIVE_COLLECTION_ID, NISAR_PROFILE_ID, NISAR_PROVIDER_ID,
    };

    fn identity(
        provider_id: &str,
        collection_id: &str,
        collection_profile_id: &str,
    ) -> NisarSubsetSourceProductIdentityV2 {
        NisarSubsetSourceProductIdentityV2 {
            provider_id: provider_id.to_owned(),
            collection_id: collection_id.to_owned(),
            collection_profile_id: collection_profile_id.to_owned(),
            provider_product_id: "fixture-product".to_owned(),
            provider_product_key: "fixture-key".to_owned(),
            product_specification_version: "fixture-version".to_owned(),
            crid: "fixture-crid".to_owned(),
            quality_status: NisarSubsetQualityStatusV2::Provisional,
        }
    }

    #[test]
    fn exact_source_identity_owns_the_science_root() {
        let lband = identity(
            NISAR_PROVIDER_ID,
            NISAR_NATIVE_COLLECTION_ID,
            NISAR_PROFILE_ID,
        );
        let sband = identity(
            BHOONIDHI_PROVIDER_ID,
            BHOONIDHI_NATIVE_COLLECTION_ID,
            BHOONIDHI_PROFILE_ID,
        );
        assert_eq!(
            NisarScienceRoot::from_source_identity(&lband),
            Some(NisarScienceRoot::Lsar)
        );
        assert_eq!(
            NisarScienceRoot::from_source_identity(&sband),
            Some(NisarScienceRoot::Ssar)
        );
        assert_eq!(NisarScienceRoot::Lsar.science_root(), "/science/LSAR");
        assert_eq!(NisarScienceRoot::Ssar.science_root(), "/science/SSAR");
        assert_eq!(NisarScienceRoot::Lsar.swath(), "LSAR_RSLC");
        assert_eq!(NisarScienceRoot::Ssar.swath(), "SSAR_RSLC");
    }

    #[test]
    fn mixed_or_unknown_source_identity_is_rejected() {
        let mixed = identity(
            BHOONIDHI_PROVIDER_ID,
            BHOONIDHI_NATIVE_COLLECTION_ID,
            NISAR_PROFILE_ID,
        );
        let unknown = identity("unknown.provider", "unknown.collection", "unknown.profile");
        assert_eq!(NisarScienceRoot::from_source_identity(&mixed), None);
        assert_eq!(NisarScienceRoot::from_source_identity(&unknown), None);
    }
}
