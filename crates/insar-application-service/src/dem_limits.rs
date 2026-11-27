//! Native DEM range limits shared by task creation and the desktop executor,
//! so a range refused at execution is already refused before a task exists.

pub const MAX_NATIVE_DEM_DOWNLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const MAX_NATIVE_DEM_TILES: usize = 512;
pub const MAX_NATIVE_DEM_MOSAIC_WIDTH: u64 = 32 * 1024;
pub const MAX_NATIVE_DEM_MOSAIC_HEIGHT: u64 = 32 * 1024;
pub const MAX_NATIVE_DEM_MOSAIC_PIXELS: u64 = 512 * 1024 * 1024;

/// Source-aligned output size and Copernicus tile count of one request window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DemRangeEstimate {
    pub width: u64,
    pub height: u64,
    pub pixels: u64,
    pub tiles: usize,
}

impl DemRangeEstimate {
    pub fn exceeds_mosaic(&self) -> bool {
        self.width == 0
            || self.height == 0
            || self.width > MAX_NATIVE_DEM_MOSAIC_WIDTH
            || self.height > MAX_NATIVE_DEM_MOSAIC_HEIGHT
            || self.pixels > MAX_NATIVE_DEM_MOSAIC_PIXELS
    }

    pub fn exceeds_tiles(&self) -> bool {
        self.tiles == 0 || self.tiles > MAX_NATIVE_DEM_TILES
    }
}

/// The limits apply to the native tiled-COG mosaic routes (Copernicus AWS,
/// FABDEM V1-2 public CloudFront COG tiles, Google GEM-15 v1 public GCS UTM COG
/// tiles, and Earthdata NISAR DEM ASF); OpenTopography has its own area limit.
pub fn limits_apply(provider: &str) -> bool {
    provider.trim().eq_ignore_ascii_case("COPERNICUS_AWS")
        || provider.trim().eq_ignore_ascii_case("NISAR_DEM_ASF")
        || provider.trim().eq_ignore_ascii_case("FABDEM_PUBLIC")
        || provider.trim().eq_ignore_ascii_case("GEM15_PUBLIC")
}

pub fn samples_per_degree(dataset: &str) -> f64 {
    match dataset.trim().to_ascii_uppercase().as_str() {
        "GEM15_DTM" | "GEM15_DSM" => 7_200.0,
        "COP90" | "SRTM_GL3" => 1_200.0,
        _ => 3_600.0,
    }
}

/// `bbox` is `[west, south, east, north]` in degrees.
pub fn estimate(dataset: &str, bbox: [f64; 4]) -> DemRangeEstimate {
    let samples = samples_per_degree(dataset);
    let width = ((bbox[2] - bbox[0]) * samples).ceil() as u64;
    let height = ((bbox[3] - bbox[1]) * samples).ceil() as u64;
    let rows = usize::try_from((bbox[3].ceil() as i32).saturating_sub(bbox[1].floor() as i32))
        .unwrap_or(0);
    let columns = usize::try_from((bbox[2].ceil() as i32).saturating_sub(bbox[0].floor() as i32))
        .unwrap_or(0);
    DemRangeEstimate {
        width,
        height,
        pixels: width.saturating_mul(height),
        tiles: rows.saturating_mul(columns),
    }
}

/// A coarser dataset is worth suggesting only for the 30 m Copernicus DEM.
pub fn coarser_dataset_available(dataset: &str) -> bool {
    dataset.trim().eq_ignore_ascii_case("COP30")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_range_estimate_matches_the_native_mosaic_formula() {
        let large = estimate("COP30", [100.0, 20.0, 111.111_1, 30.555_5]);
        assert_eq!((large.width, large.height), (40_000, 38_000));
        assert_eq!(large.pixels, 1_520_000_000);
        assert_eq!(large.tiles, 132);
        assert!(large.exceeds_mosaic());

        let at_limit = estimate("COP30", [0.0, 0.0, 9.102_2, 4.551_1]);
        assert_eq!((at_limit.width, at_limit.height), (32_768, 16_384));
        assert_eq!(at_limit.pixels, MAX_NATIVE_DEM_MOSAIC_PIXELS);
        assert!(!at_limit.exceeds_mosaic());

        let just_over = estimate("COP30", [0.0, 0.0, 9.102_2, 4.551_3]);
        assert_eq!((just_over.width, just_over.height), (32_768, 16_385));
        assert!(just_over.exceeds_mosaic());

        let hubei = estimate("COP30", [108.0, 29.0, 116.0, 34.0]);
        assert_eq!((hubei.width, hubei.height), (28_800, 18_000));
        assert!(!hubei.exceeds_mosaic());
        assert!(!hubei.exceeds_tiles());
    }

    #[test]
    fn ninety_metre_datasets_use_a_third_of_the_samples() {
        let window = [166.0, -47.0, 180.0, -34.0];
        let cop30 = estimate("COP30", window);
        let cop90 = estimate("cop90", window);
        assert_eq!((cop30.width, cop30.height), (50_400, 46_800));
        assert_eq!((cop90.width, cop90.height), (16_800, 15_600));
        assert!(cop30.exceeds_mosaic());
        assert!(!cop90.exceeds_mosaic());
        assert_eq!(cop90.tiles, 182);
        assert!(coarser_dataset_available("COP30"));
        assert!(!coarser_dataset_available("COP90"));
    }

    #[test]
    fn only_the_native_tiled_cog_mosaics_are_limited() {
        assert!(limits_apply("COPERNICUS_AWS"));
        assert!(limits_apply(" copernicus_aws "));
        assert!(limits_apply("NISAR_DEM_ASF"));
        assert!(limits_apply(" nisar_dem_asf "));
        assert!(limits_apply("FABDEM_PUBLIC"));
        assert!(limits_apply(" fabdem_public "));
        assert!(limits_apply("GEM15_PUBLIC"));
        assert!(limits_apply(" gem15_public "));
        assert!(!limits_apply("OPENTOPOGRAPHY"));

        let window = [0.0, 0.0, 1.0, 1.0];
        let cop30 = estimate("COP30", window);
        let gem15_dtm = estimate("GEM15_DTM", window);
        let gem15_dsm = estimate("gem15_dsm", window);
        assert_eq!((cop30.width, cop30.height), (3_600, 3_600));
        assert_eq!((gem15_dtm.width, gem15_dtm.height), (7_200, 7_200));
        assert_eq!((gem15_dsm.width, gem15_dsm.height), (7_200, 7_200));
    }
}
