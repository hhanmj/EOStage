use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

#[test]
#[ignore = "requires explicitly selected product runtime and fixture; run with --ignored"]
fn product_runtime_reopens_sarscape_with_bounded_siblings() {
    let root = std::env::var_os("INSAR_COMPLETION_GIS_ROOT").expect("explicit GIS root");
    let input = std::env::var_os("INSAR_COMPLETION_DEM_INPUT").expect("explicit DEM fixture");
    let engine = crate::LazyNativeGisEngine::from_product_root(PathBuf::from(root)).unwrap();
    let fixture = Fixture::new();
    let source = engine.inspect_dem(PathBuf::from(&input)).unwrap();
    let output = fixture.0.join("fixture_dem");
    engine
        .convert_dem(
            PathBuf::from(input),
            fixture.0.join("ellipsoid.tif"),
            Some(&output),
            "WGS84_ELLIPSOID",
        )
        .unwrap();
    let reopened = engine
        .inspect_dem(&output)
        .expect("product GDAL must read its own ENVI output");
    assert_eq!(reopened["metadata"]["driver"], "ENVI");
    for key in ["width", "height", "count", "geotransform", "nodata"] {
        assert_eq!(reopened["metadata"][key], source["metadata"][key], "{key}");
    }
    let names = raster_siblings(&output).unwrap();
    engine
        .convert_dem(
            &output,
            fixture.0.join("roundtrip.tif"),
            Some(&fixture.0.join("roundtrip_dem")),
            "WGS84_ELLIPSOID",
        )
        .unwrap();
    assert_eq!(
        fs::read(&output).unwrap(),
        fs::read(fixture.0.join("roundtrip_dem")).unwrap()
    );
    assert_eq!(names.len(), 2, "only raster and matching HDR are admitted");
    fs::rename(
        output.with_extension("hdr"),
        fixture.0.join("unrelated.hdr"),
    )
    .unwrap();
    assert!(
        engine.inspect_dem(&output).is_err(),
        "unrelated HDR must not satisfy the read"
    );
}
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "insar-sarscape-unit-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("isolated fixture directory");
        Self(path)
    }

    fn staged(&self) -> PathBuf {
        let part = self.0.join("fixture_dem.part.bin");
        let values = [1.25_f32, -2.5, 0.0, 100.5, -9999.0, 4.75];
        let bytes = values
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<_>>();
        fs::write(&part, bytes).unwrap();
        let transform = [110.0, 0.25, 0.0, 31.0, 0.0, -0.25];
        write_envi_header(
            &sarscape_part_header_path(&part).unwrap(),
            3,
            2,
            &transform,
            Some(-9999.0),
            "GEOGCS[\"GCS_WGS_1984\"]",
        )
        .unwrap();
        write_sarscape_sml(
            &part,
            3,
            2,
            &transform,
            Some(-9999.0),
            "GEOGCS[\"GCS_WGS_1984\"]",
        )
        .unwrap();
        part
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn sarscape_sml_preserves_v219_schema_version_and_metadata() {
    let fixture = Fixture::new();
    let part = fixture.staged();
    let sml = fs::read_to_string(sidecar_path(&part, ".sml").unwrap()).unwrap();
    for expected in [
        "xsi:schemaLocation=",
        "SARscapeHeaderSchema_version_2.0.2.xsd",
        "<SmlVersion>2.0.2</SmlVersion>",
        "<DataUnits>ELLIPSOIDAL DEM</DataUnits>",
        "<NrOfPixelsPerLine>3</NrOfPixelsPerLine>",
        "<NrOfLinesPerImage>2</NrOfLinesPerImage>",
        "<CellType>FLOAT</CellType>",
        "<Interleave>LAYOUT_BSQ</Interleave>",
        "<BytesOrder>LSBF</BytesOrder>",
        "SOFTWARE",
        "SML VERSION",
        "<NullCellValue>-9999</NullCellValue>",
    ] {
        assert!(sml.contains(expected), "missing {expected}");
    }
    assert!(
        !fixture.0.join("fixture_dem.sml").exists(),
        "metadata remains staged"
    );
    assert!(sml.contains("GCS_WGS_1984"));
    assert!(sml.contains("&quot;"));
}

#[test]
fn sarscape_staging_requires_both_nonempty_sidecars_and_raster() {
    let fixture = Fixture::new();
    for suffix in ["raster", "hdr", "sml"] {
        let part = fixture.staged();
        validate_sarscape_staged(&part).unwrap();
        let target = match suffix {
            "raster" => part.clone(),
            "hdr" => sarscape_part_header_path(&part).unwrap(),
            _ => sidecar_path(&part, ".sml").unwrap(),
        };
        fs::write(&target, []).unwrap();
        assert!(validate_sarscape_staged(&part).is_err());
        fs::remove_file(&target).unwrap();
        assert!(validate_sarscape_staged(&part).is_err());
    }
}

#[test]
fn sarscape_metadata_roundtrips_arcsecond_and_subpixel_precision() {
    let fixture = Fixture::new();
    for spacing in [1.0 / 3600.0, 3.0 / 3600.0, 0.0000000000123456789] {
        let transform = [
            99.99986111111112,
            spacing,
            0.0,
            30.01013888888889,
            0.0,
            -spacing,
        ];
        let part = fixture.0.join("precision_dem.part.bin");
        let header = sarscape_part_header_path(&part).unwrap();
        write_envi_header(&header, 36, 36, &transform, Some(-9999.0), "WGS84").unwrap();
        write_sarscape_sml(&part, 36, 36, &transform, Some(-9999.0), "WGS84").unwrap();
        let hdr = fs::read_to_string(header).unwrap();
        let info = hdr
            .lines()
            .find(|line| line.starts_with("map info = "))
            .unwrap();
        let fields: Vec<_> = info.split(',').collect();
        for (field, expected) in [
            (3, transform[0]),
            (4, transform[3]),
            (5, spacing),
            (6, spacing),
        ] {
            assert_eq!(fields[field].trim().parse::<f64>().unwrap(), expected);
        }
        let sml = fs::read_to_string(sidecar_path(&part, ".sml").unwrap()).unwrap();
        for (tag, expected) in [
            ("LonEasting", transform[0]),
            ("LatNorthing", transform[3]),
            ("PixelSpacingLonEast", spacing),
            ("PixelSpacingLatNorth", -spacing),
            ("EastingGridSize", spacing),
            ("NorthingGridSize", spacing),
        ] {
            let value = sml
                .split_once(&format!("<{tag}>"))
                .unwrap()
                .1
                .split_once('<')
                .unwrap()
                .0;
            assert_eq!(value.parse::<f64>().unwrap(), expected, "{tag}");
        }
    }
}

#[test]
fn sarscape_header_size_layout_and_duplicate_fields_must_match_data() {
    let fixture = Fixture::new();
    for (old, new) in [
        ("samples = 3", "samples = 4"),
        ("data type = 4", "data type = 5"),
        ("interleave = bsq", "interleave = bip"),
        ("byte order = 0", "byte order = 1"),
        ("bands = 1", "bands = 2"),
        ("header offset = 0", "header offset = 128"),
        ("samples = 3", "samples = 3\nsamples = 3"),
    ] {
        let part = fixture.staged();
        let hdr = sarscape_part_header_path(&part).unwrap();
        let text = fs::read_to_string(&hdr).unwrap();
        fs::write(&hdr, text.replace(old, new)).unwrap();
        assert!(validate_sarscape_staged(&part).is_err(), "accepted {new}");
    }
}
