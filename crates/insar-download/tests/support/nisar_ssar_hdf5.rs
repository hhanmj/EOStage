#![allow(non_local_definitions)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use hdf5::types::FixedAscii;
use hdf5::{File, Group};

pub const BASE_ITEM: &str =
    "NISAR_S1_PR_RSLC_029_063_D_079_2500_CRNA_A_20260829T135915_20260829T135945_P00500_N_P_I_001";

const IDENTIFICATION_PATH: &str = "/science/SSAR/identification";
const FREQUENCY_A_PATH: &str = "/science/SSAR/RSLC/swaths/frequencyA";
const GEOLOCATION_PATH: &str = "/science/SSAR/RSLC/metadata/geolocationGrid";
static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, hdf5::H5Type)]
pub struct CFloat32 {
    pub r: f32,
    pub i: f32,
}

pub struct FixtureDirectory {
    path: PathBuf,
    #[cfg(test)]
    _hdf5_test_guard: insar_download::Hdf5TestLockGuard,
}

impl FixtureDirectory {
    pub fn new(label: &str) -> Self {
        #[cfg(test)]
        let hdf5_test_guard = insar_download::HDF5_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let ordinal = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "insar-nisar-ssar-{label}-{}-{ordinal}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create isolated SSAR fixture directory");
        Self {
            path,
            #[cfg(test)]
            _hdf5_test_guard: hdf5_test_guard,
        }
    }

    pub fn item_path(&self, item: &str) -> PathBuf {
        self.path.join(format!("{item}.h5"))
    }
}

impl Drop for FixtureDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Replace the four-character POLE token in the reviewed live item.
pub fn item_for_pole(pole: &str) -> String {
    let mut parts = BASE_ITEM.split('_').map(str::to_owned).collect::<Vec<_>>();
    assert_eq!(parts.len(), 18, "fixture item grammar");
    parts[9] = pole.to_owned();
    parts.join("_")
}

pub fn item_for_coverage(coverage: &str) -> String {
    let mut parts = BASE_ITEM.split('_').map(str::to_owned).collect::<Vec<_>>();
    assert_eq!(parts.len(), 18, "fixture item grammar");
    parts[15] = coverage.to_owned();
    parts.join("_")
}

#[derive(Clone, Copy, Debug)]
pub enum Mutation {
    WrongMission,
    WrongRadarBand,
    WrongProductType,
    WrongProductLevel,
    WrongSpecification,
    WrongRelease,
    WrongProductVersion,
    WrongGranule,
    WrongFrequencies,
    FrequencyB,
    WrongPolarizations,
    MissingDeclaredChannel,
    WrongImageType,
    WrongImageRank,
    LsarRoot,
    SoftLink,
    ExternalLink,
    HardLinkAlias,
    WrongEpsgType,
    WrongEpsgRank,
    WrongCoordinateXType,
    WrongCoordinateYShape,
    WrongSlantRangeRank,
    WrongSlantRangeLength,
    WrongZeroDopplerRank,
    WrongZeroDopplerLength,
    WrongHeightRank,
    WrongHeightLength,
    ExtraOpaqueField,
}

pub fn create_valid(path: &Path, item: &str) {
    create_valid_with_geolocation_shape(path, item, (4, 5, 6));
}

pub fn create_valid_with_geolocation_shape(
    path: &Path,
    item: &str,
    geolocation_shape: (usize, usize, usize),
) {
    let parts = item.split('_').collect::<Vec<_>>();
    assert_eq!(parts.len(), 18, "fixture item grammar");
    let polarizations = polarizations_for_pole(parts[9]).expect("supported fixture POLE");
    let file = File::create(path).expect("create SSAR HDF5 fixture");
    let identification = file
        .create_group(IDENTIFICATION_PATH)
        .expect("create SSAR identification");
    let frequency = file
        .create_group(FREQUENCY_A_PATH)
        .expect("create SSAR frequency A");
    let geolocation = file
        .create_group(GEOLOCATION_PATH)
        .expect("create SSAR geolocation grid");

    for (name, value) in [
        ("missionId", "NISAR"),
        ("radarBand", "S"),
        ("productType", "RSLC"),
        ("productLevel", "L1"),
        ("granuleId", item),
        ("productSpecificationVersion", "1.2.1"),
        ("compositeReleaseId", "P00500"),
        ("productVersion", "1.2.1"),
    ] {
        write_ascii_scalar::<160>(&identification, name, value);
    }
    write_ascii_list::<4>(&identification, "listOfFrequencies", &["A"]);
    write_ascii_list::<4>(&frequency, "listOfPolarizations", &polarizations);
    for polarization in &polarizations {
        frequency
            .new_dataset::<CFloat32>()
            .shape((2, 3))
            .create(*polarization)
            .expect("create SSAR complex image");
    }

    let epsg = geolocation
        .new_dataset::<i32>()
        .create("epsg")
        .expect("create SSAR EPSG scalar");
    epsg.write_scalar(&4326_i32)
        .expect("write SSAR EPSG scalar");
    for name in ["coordinateX", "coordinateY"] {
        geolocation
            .new_dataset::<f64>()
            .shape(geolocation_shape)
            .create(name)
            .expect("create SSAR geolocation cube");
    }
    geolocation
        .new_dataset::<f64>()
        .shape((geolocation_shape.2,))
        .create("slantRange")
        .expect("create slant-range dimension");
    geolocation
        .new_dataset::<f64>()
        .shape((geolocation_shape.1,))
        .create("zeroDopplerTime")
        .expect("create zero-Doppler dimension");
    geolocation
        .new_dataset::<f64>()
        .shape((geolocation_shape.0,))
        .create("heightAboveEllipsoid")
        .expect("create height dimension");
    file.flush().expect("flush SSAR fixture");
}

pub fn mutate(path: &Path, item: &str, mutation: Mutation) {
    let file = File::open_rw(path).expect("open SSAR fixture for mutation");
    let identification = file.group(IDENTIFICATION_PATH).expect("identification");
    let frequency = file.group(FREQUENCY_A_PATH).expect("frequency A");
    let geolocation = file.group(GEOLOCATION_PATH).expect("geolocation");
    let first_polarization = item
        .split('_')
        .nth(9)
        .and_then(|pole| polarizations_for_pole(pole))
        .and_then(|polarizations| polarizations.first().copied())
        .expect("supported fixture POLE");
    match mutation {
        Mutation::WrongMission => replace_ascii::<160>(&identification, "missionId", "OTHER"),
        Mutation::WrongRadarBand => replace_ascii::<160>(&identification, "radarBand", "L"),
        Mutation::WrongProductType => replace_ascii::<160>(&identification, "productType", "GSLC"),
        Mutation::WrongProductLevel => replace_ascii::<160>(&identification, "productLevel", "L2"),
        Mutation::WrongSpecification => {
            replace_ascii::<160>(&identification, "productSpecificationVersion", "1.5.0")
        }
        Mutation::WrongRelease => {
            replace_ascii::<160>(&identification, "compositeReleaseId", "P05023")
        }
        Mutation::WrongProductVersion => {
            replace_ascii::<160>(&identification, "productVersion", "1.4.1")
        }
        Mutation::WrongGranule => replace_ascii::<160>(&identification, "granuleId", "UNKNOWN"),
        Mutation::WrongFrequencies => {
            replace_ascii_list::<4>(&identification, "listOfFrequencies", &["A", "B"])
        }
        Mutation::FrequencyB => {
            file.create_group("/science/SSAR/RSLC/swaths/frequencyB")
                .expect("create forbidden frequency B");
        }
        Mutation::WrongPolarizations => {
            replace_ascii_list::<4>(&frequency, "listOfPolarizations", &["HH", "HV"])
        }
        Mutation::MissingDeclaredChannel => {
            frequency
                .unlink(first_polarization)
                .expect("remove declared image");
        }
        Mutation::WrongImageType => {
            frequency
                .unlink(first_polarization)
                .expect("remove complex image");
            frequency
                .new_dataset::<f32>()
                .shape((2, 3))
                .create(first_polarization)
                .expect("create wrong image type");
        }
        Mutation::WrongImageRank => {
            frequency
                .unlink(first_polarization)
                .expect("remove complex image");
            frequency
                .new_dataset::<CFloat32>()
                .shape((2, 3, 1))
                .create(first_polarization)
                .expect("create wrong image rank");
        }
        Mutation::LsarRoot => {
            file.create_group("/science/LSAR")
                .expect("create forbidden LSAR root");
        }
        Mutation::SoftLink => file
            .link_soft("/science", "/softScience")
            .expect("create soft link"),
        Mutation::ExternalLink => file
            .link_external("untrusted.h5", "/", "/externalScience")
            .expect("create external link"),
        Mutation::HardLinkAlias => file
            .link_hard("/science/SSAR/RSLC", "/rslcAlias")
            .expect("create hard-link alias"),
        Mutation::WrongEpsgType => {
            geolocation.unlink("epsg").expect("remove EPSG");
            geolocation
                .new_dataset::<i64>()
                .create("epsg")
                .expect("create wrong EPSG type");
        }
        Mutation::WrongEpsgRank => {
            geolocation.unlink("epsg").expect("remove EPSG");
            geolocation
                .new_dataset::<i32>()
                .shape((1,))
                .create("epsg")
                .expect("create rank-one EPSG");
        }
        Mutation::WrongCoordinateXType => {
            geolocation
                .unlink("coordinateX")
                .expect("remove coordinateX");
            geolocation
                .new_dataset::<f32>()
                .shape((4, 5, 6))
                .create("coordinateX")
                .expect("create wrong coordinateX type");
        }
        Mutation::WrongCoordinateYShape => {
            geolocation
                .unlink("coordinateY")
                .expect("remove coordinateY");
            geolocation
                .new_dataset::<f64>()
                .shape((4, 5, 7))
                .create("coordinateY")
                .expect("create wrong coordinateY shape");
        }
        Mutation::WrongSlantRangeRank => replace_vector(&geolocation, "slantRange", (6, 1)),
        Mutation::WrongSlantRangeLength => replace_vector(&geolocation, "slantRange", (5,)),
        Mutation::WrongZeroDopplerRank => replace_vector(&geolocation, "zeroDopplerTime", (5, 1)),
        Mutation::WrongZeroDopplerLength => replace_vector(&geolocation, "zeroDopplerTime", (6,)),
        Mutation::WrongHeightRank => replace_vector(&geolocation, "heightAboveEllipsoid", (4, 1)),
        Mutation::WrongHeightLength => replace_vector(&geolocation, "heightAboveEllipsoid", (5,)),
        Mutation::ExtraOpaqueField => {
            write_ascii_scalar::<32>(&identification, "opaqueField", "CP")
        }
    }
    file.flush().expect("flush mutated SSAR fixture");
}

fn polarizations_for_pole(pole: &str) -> Option<Vec<&'static str>> {
    match pole {
        "SHNA" => Some(vec!["HH"]),
        "SVNA" => Some(vec!["VV"]),
        "DHNA" => Some(vec!["HH", "HV"]),
        "DVNA" => Some(vec!["VV", "VH"]),
        "CLNA" => Some(vec!["LH", "LV"]),
        "CRNA" => Some(vec!["RH", "RV"]),
        _ => None,
    }
}

fn write_ascii_scalar<const N: usize>(group: &Group, name: &str, value: &str) {
    let value = FixedAscii::<N>::from_ascii(value).expect("fixture scalar fits");
    group
        .new_dataset::<FixedAscii<N>>()
        .create(name)
        .expect("create ASCII scalar")
        .write_scalar(&value)
        .expect("write ASCII scalar");
}

fn write_ascii_list<const N: usize>(group: &Group, name: &str, values: &[&str]) {
    let values = values
        .iter()
        .map(|value| FixedAscii::<N>::from_ascii(value).expect("fixture list value fits"))
        .collect::<Vec<_>>();
    group
        .new_dataset::<FixedAscii<N>>()
        .shape((values.len(),))
        .create(name)
        .expect("create ASCII list")
        .write(&values)
        .expect("write ASCII list");
}

fn replace_ascii<const N: usize>(group: &Group, name: &str, value: &str) {
    group.unlink(name).expect("unlink ASCII scalar");
    write_ascii_scalar::<N>(group, name, value);
}

fn replace_ascii_list<const N: usize>(group: &Group, name: &str, values: &[&str]) {
    group.unlink(name).expect("unlink ASCII list");
    write_ascii_list::<N>(group, name, values);
}

fn replace_vector(group: &Group, name: &str, shape: impl Into<hdf5::Extents>) {
    group.unlink(name).expect("unlink geolocation vector");
    group
        .new_dataset::<f64>()
        .shape(shape)
        .create(name)
        .expect("create mutated geolocation vector");
}
