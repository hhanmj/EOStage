#![allow(non_local_definitions)]

use std::ffi::CString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use hdf5::plist::dataset_create::DatasetCreate;
use hdf5::types::FixedAscii;
use hdf5::{Dataspace, Datatype, File, Group};
use hdf5_sys::h5d::{H5Dclose, H5Dcreate2};
use hdf5_sys::h5g::{H5Gclose, H5Gcreate2};
use hdf5_sys::h5p::{H5Pset_chunk, H5Pset_filter, H5P_DEFAULT};
use hdf5_sys::h5z::{H5Z_FILTER_DEFLATE, H5Z_FILTER_FLETCHER32, H5Z_FLAG_OPTIONAL};

pub const GRANULE_ID: &str = "NISAR_L1_PR_RSLC_TEST_GRANULE";
pub const LEGACY_PRODUCT_SPECIFICATION_VERSION: &str = "1.2.1";
pub const P05023_PRODUCT_SPECIFICATION_VERSION: &str = "1.5.0";

const IDENTIFICATION_PATH: &str = "/science/LSAR/identification";
const SWATHS_PATH: &str = "/science/LSAR/RSLC/swaths";
const GEOLOCATION_PATH: &str = "/science/LSAR/RSLC/metadata/geolocationGrid";

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug)]
pub enum FrequencyScope {
    AOnly,
    BOnly,
    Dual,
}

impl FrequencyScope {
    fn frequency_names(self) -> &'static [&'static str] {
        match self {
            Self::AOnly => &["A"],
            Self::BOnly => &["B"],
            Self::Dual => &["A", "B"],
        }
    }

    fn frequency_a_polarizations(self) -> Option<&'static [&'static str]> {
        match self {
            Self::AOnly => Some(&["HH"]),
            Self::BOnly => None,
            Self::Dual => Some(&["HH", "HV"]),
        }
    }

    fn frequency_b_polarizations(self) -> Option<&'static [&'static str]> {
        match self {
            Self::AOnly => None,
            Self::BOnly | Self::Dual => Some(&["VV"]),
        }
    }
}

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
            "insar-nisar-hdf5-{label}-{}-{ordinal}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create isolated NISAR HDF5 fixture directory");
        Self {
            path,
            #[cfg(test)]
            _hdf5_test_guard: hdf5_test_guard,
        }
    }

    pub fn file(&self, label: &str) -> PathBuf {
        self.path.join(format!("{label}.h5"))
    }
}

impl Drop for FixtureDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[derive(Clone, Copy, Debug)]
pub enum FixtureMutation {
    MissingSwaths,
    WrongMissionId,
    WrongProductType,
    WrongProductLevel,
    WrongRadarBand,
    WrongProductSpecificationVersion,
    MissingCompositeReleaseId,
    WrongCompositeReleaseId,
    WrongGranuleId,
    LegacySlc,
    Ssar,
    WrongFrequencyList,
    WrongPolarizationList,
    WrongImageType,
    WrongImageShape,
    EmptyBoundingPolygon,
    WrongGeolocationShape,
    SoftLink,
    ExternalLink,
    HardLinkAlias,
    UnknownFilter,
    FilterCountLimit,
    FilterClientDataLimit,
    DeflateLevel,
    FletcherClientData,
    ExternalStorage,
    VirtualDataset,
    DirectMemberLimit,
    LinkNameLimit,
    InvalidUtf8LinkName,
    AttributeLimit,
    DepthLimit,
    RankLimit,
    DimensionLimit,
    MetadataArrayLimit,
    MetadataStringBudget,
    MetadataUnlimitedMaxExtent,
    MetadataLargerFiniteMaxExtent,
    MetadataOversizedCompressedChunk,
    MetadataFilteredChunk,
}

pub fn create_valid_fixture(path: &Path, scope: FrequencyScope) {
    create_valid_fixture_for_granule(path, scope, GRANULE_ID);
}

pub fn create_valid_fixture_for_granule(path: &Path, scope: FrequencyScope, granule_id: &str) {
    create_valid_fixture_for_profile(
        path,
        scope,
        granule_id,
        LEGACY_PRODUCT_SPECIFICATION_VERSION,
    );
}

pub fn create_valid_fixture_for_profile(
    path: &Path,
    scope: FrequencyScope,
    granule_id: &str,
    product_specification_version: &str,
) {
    create_fixture(
        path,
        scope,
        granule_id,
        product_specification_version,
        (2, 3),
        false,
    );
}

pub fn create_huge_logical_fixture(path: &Path) {
    create_fixture(
        path,
        FrequencyScope::AOnly,
        GRANULE_ID,
        LEGACY_PRODUCT_SPECIFICATION_VERSION,
        (1_000_000, 1_000_000),
        true,
    );
}

pub fn create_allowed_filter_fixture(path: &Path) {
    create_valid_fixture(path, FrequencyScope::AOnly);
    let file = File::open_rw(path).expect("open allowed-filter NISAR fixture");
    file.new_dataset::<u8>()
        .shape((16,))
        .chunk((16,))
        .shuffle()
        .deflate(6)
        .fletcher32()
        .create("allowedFilters")
        .expect("create reviewed built-in filter pipeline");
}

fn create_raw_filter_fixture(file: &File, name: &str, filters: &[(i32, &[u32])]) {
    let dcpl = DatasetCreate::try_new().expect("create raw-filter property list");
    let chunk = [1_u64];
    let chunk_status =
        hdf5::sync::sync(|| unsafe { H5Pset_chunk(dcpl.id(), chunk.len() as i32, chunk.as_ptr()) });
    assert_eq!(chunk_status, 0, "set raw-filter chunk shape");
    for (filter_id, client_values) in filters {
        let filter_status = hdf5::sync::sync(|| unsafe {
            H5Pset_filter(
                dcpl.id(),
                *filter_id,
                H5Z_FLAG_OPTIONAL,
                client_values.len(),
                client_values.as_ptr(),
            )
        });
        assert_eq!(filter_status, 0, "set raw optional filter {filter_id}");
    }

    let datatype = Datatype::from_type::<u8>().expect("create raw-filter datatype");
    let dataspace = Dataspace::try_new(1).expect("create raw-filter dataspace");
    let name = CString::new(name).expect("bounded raw-filter dataset name");
    let dataset_id = hdf5::sync::sync(|| unsafe {
        H5Dcreate2(
            file.id(),
            name.as_ptr(),
            datatype.id(),
            dataspace.id(),
            H5P_DEFAULT,
            dcpl.id(),
            H5P_DEFAULT,
        )
    });
    assert!(dataset_id >= 0, "create raw-filter dataset");
    let close_status = hdf5::sync::sync(|| unsafe { H5Dclose(dataset_id) });
    assert_eq!(close_status, 0, "close raw-filter dataset");
}

pub fn create_mutated_fixture(path: &Path, mutation: FixtureMutation) {
    create_valid_fixture(path, FrequencyScope::AOnly);
    apply_mutation_to_fixture(path, mutation);
}

pub fn apply_mutation_to_fixture(path: &Path, mutation: FixtureMutation) {
    let file = File::open_rw(path).expect("open mutable NISAR fixture");

    match mutation {
        FixtureMutation::MissingSwaths => {
            file.unlink(SWATHS_PATH)
                .expect("remove required swaths group");
        }
        FixtureMutation::WrongMissionId => {
            replace_ascii_scalar::<16>(&file, IDENTIFICATION_PATH, "missionId", "NOT-NISAR");
        }
        FixtureMutation::WrongProductType => {
            replace_ascii_scalar::<16>(&file, IDENTIFICATION_PATH, "productType", "GSLC");
        }
        FixtureMutation::WrongProductLevel => {
            replace_ascii_scalar::<16>(&file, IDENTIFICATION_PATH, "productLevel", "L2");
        }
        FixtureMutation::WrongRadarBand => {
            replace_ascii_scalar::<16>(&file, IDENTIFICATION_PATH, "radarBand", "S");
        }
        FixtureMutation::WrongProductSpecificationVersion => {
            replace_ascii_scalar::<16>(
                &file,
                IDENTIFICATION_PATH,
                "productSpecificationVersion",
                "1.2.0",
            );
        }
        FixtureMutation::MissingCompositeReleaseId => {
            file.group(IDENTIFICATION_PATH)
                .expect("identification group")
                .unlink("compositeReleaseId")
                .expect("remove composite release identifier");
        }
        FixtureMutation::WrongCompositeReleaseId => {
            replace_ascii_scalar::<16>(&file, IDENTIFICATION_PATH, "compositeReleaseId", "P99999");
        }
        FixtureMutation::WrongGranuleId => {
            replace_ascii_scalar::<128>(
                &file,
                IDENTIFICATION_PATH,
                "granuleId",
                "NISAR_L1_PR_RSLC_DIFFERENT_GRANULE",
            );
        }
        FixtureMutation::LegacySlc => {
            file.create_group("/science/LSAR/SLC")
                .expect("create forbidden legacy SLC group");
        }
        FixtureMutation::Ssar => {
            file.create_group("/science/SSAR")
                .expect("create forbidden S-band group");
        }
        FixtureMutation::WrongFrequencyList => {
            replace_ascii_list::<2>(&file, IDENTIFICATION_PATH, "listOfFrequencies", &["B"]);
        }
        FixtureMutation::WrongPolarizationList => {
            replace_ascii_list::<4>(
                &file,
                "/science/LSAR/RSLC/swaths/frequencyA",
                "listOfPolarizations",
                &["HV"],
            );
        }
        FixtureMutation::WrongImageType => {
            let frequency = file
                .group("/science/LSAR/RSLC/swaths/frequencyA")
                .expect("frequency A group");
            frequency.unlink("HH").expect("remove CFloat32 image");
            frequency
                .new_dataset::<f32>()
                .shape((2, 3))
                .create("HH")
                .expect("create wrong image datatype");
        }
        FixtureMutation::WrongImageShape => {
            let frequency = file
                .group("/science/LSAR/RSLC/swaths/frequencyA")
                .expect("frequency A group");
            frequency.unlink("HH").expect("remove rank-two image");
            frequency
                .new_dataset::<CFloat32>()
                .shape((0, 3))
                .create("HH")
                .expect("create empty image shape");
        }
        FixtureMutation::EmptyBoundingPolygon => {
            replace_ascii_scalar::<128>(&file, IDENTIFICATION_PATH, "boundingPolygon", "");
        }
        FixtureMutation::WrongGeolocationShape => {
            let geolocation = file.group(GEOLOCATION_PATH).expect("geolocation group");
            geolocation
                .unlink("coordinateY")
                .expect("remove coordinateY");
            geolocation
                .new_dataset::<f64>()
                .shape((1, 2, 4))
                .create("coordinateY")
                .expect("create unequal coordinateY shape");
        }
        FixtureMutation::SoftLink => {
            file.link_soft("/science", "/softScience")
                .expect("create soft link");
        }
        FixtureMutation::ExternalLink => {
            file.link_external("untrusted.h5", "/", "/externalScience")
                .expect("create external link");
        }
        FixtureMutation::HardLinkAlias => {
            file.link_hard("/science/LSAR/RSLC", "/rslcAlias")
                .expect("create hard-link alias");
        }
        FixtureMutation::UnknownFilter => {
            file.new_dataset::<u8>()
                .shape((1,))
                .chunk((1,))
                .nbit()
                .create("unreviewedFilter")
                .expect("create dataset with an unreviewed built-in filter");
        }
        FixtureMutation::FilterCountLimit => {
            create_raw_filter_fixture(
                &file,
                "tooManyFilters",
                &[(32_000, &[]), (32_001, &[]), (32_002, &[]), (32_003, &[])],
            );
        }
        FixtureMutation::FilterClientDataLimit => {
            create_raw_filter_fixture(
                &file,
                "oversizedFilterClientData",
                &[(H5Z_FILTER_DEFLATE, &[7; 33])],
            );
        }
        FixtureMutation::DeflateLevel => {
            create_raw_filter_fixture(&file, "invalidDeflateLevel", &[(H5Z_FILTER_DEFLATE, &[10])]);
        }
        FixtureMutation::FletcherClientData => {
            create_raw_filter_fixture(
                &file,
                "invalidFletcherClientData",
                &[(H5Z_FILTER_FLETCHER32, &[1])],
            );
        }
        FixtureMutation::ExternalStorage => {
            file.new_dataset::<u8>()
                .shape((16,))
                .external("external-storage.raw", 0, 16)
                .create("externalStorage")
                .expect("create external-storage dataset");
        }
        FixtureMutation::VirtualDataset => {
            file.new_dataset::<u8>()
                .shape((2,))
                .create("virtualSource")
                .expect("create VDS source");
            file.new_dataset::<u8>()
                .shape((2,))
                .virtual_map(".", "/virtualSource", (2,), .., (2,), ..)
                .create("virtualDataset")
                .expect("create virtual dataset");
        }
        FixtureMutation::DirectMemberLimit => {
            for index in 0..512 {
                let name = format!("member{index:03}");
                file.new_dataset::<u8>()
                    .create(name.as_str())
                    .expect("create direct-member limit fixture");
            }
        }
        FixtureMutation::LinkNameLimit => {
            file.create_group(&"n".repeat(513))
                .expect("create overlong link name");
        }
        FixtureMutation::InvalidUtf8LinkName => {
            let name = [b'i', b'n', b'v', b'a', b'l', b'i', b'd', b'-', 0xff, 0];
            let group_id = hdf5::sync::sync(|| unsafe {
                H5Gcreate2(
                    file.id(),
                    name.as_ptr().cast(),
                    H5P_DEFAULT,
                    H5P_DEFAULT,
                    H5P_DEFAULT,
                )
            });
            assert!(group_id >= 0, "create invalid UTF-8 link name fixture");
            let close_status = hdf5::sync::sync(|| unsafe { H5Gclose(group_id) });
            assert_eq!(close_status, 0, "close invalid UTF-8 fixture group");
        }
        FixtureMutation::AttributeLimit => {
            for index in 0..65 {
                let name = format!("attribute{index:02}");
                file.new_attr::<u8>()
                    .create(name.as_str())
                    .expect("create attribute limit fixture");
            }
        }
        FixtureMutation::DepthLimit => {
            let path = (0..13)
                .map(|index| format!("depth{index:02}"))
                .collect::<Vec<_>>()
                .join("/");
            file.create_group(&path)
                .expect("create over-depth group tree");
        }
        FixtureMutation::RankLimit => {
            file.new_dataset::<u8>()
                .shape(vec![1_usize; 9])
                .create("rankNine")
                .expect("create rank-nine dataset");
        }
        FixtureMutation::DimensionLimit => {
            file.new_dataset::<u8>()
                .shape((u32::MAX as usize + 1,))
                .chunk((1,))
                .create("oversizedDimension")
                .expect("create oversized logical dimension");
        }
        FixtureMutation::MetadataArrayLimit => {
            replace_ascii_list::<2>(
                &file,
                IDENTIFICATION_PATH,
                "listOfFrequencies",
                &[
                    "A", "A", "A", "A", "A", "A", "A", "A", "A", "A", "A", "A", "A", "A", "A", "A",
                    "A",
                ],
            );
        }
        FixtureMutation::MetadataStringBudget => {
            let identification = file
                .group(IDENTIFICATION_PATH)
                .expect("identification group");
            identification
                .unlink("boundingPolygon")
                .expect("remove bounded polygon");
            identification
                .new_dataset::<FixedAscii<65_536>>()
                .create("boundingPolygon")
                .expect("create over-budget fixed string");
        }
        FixtureMutation::MetadataUnlimitedMaxExtent => {
            let identification = file
                .group(IDENTIFICATION_PATH)
                .expect("identification group");
            identification
                .unlink("listOfFrequencies")
                .expect("remove fixed frequency list");
            let value = FixedAscii::<2>::from_ascii("A").expect("frequency fits fixed ASCII type");
            identification
                .new_dataset::<FixedAscii<2>>()
                .shape((1_usize..,))
                .chunk((1,))
                .create("listOfFrequencies")
                .expect("create unlimited frequency list")
                .write(&[value])
                .expect("write unlimited frequency list");
        }
        FixtureMutation::MetadataLargerFiniteMaxExtent => {
            let identification = file
                .group(IDENTIFICATION_PATH)
                .expect("identification group");
            identification
                .unlink("listOfFrequencies")
                .expect("remove fixed frequency list");
            let value = FixedAscii::<2>::from_ascii("A").expect("frequency fits fixed ASCII type");
            identification
                .new_dataset::<FixedAscii<2>>()
                .shape((1_usize..=2_usize,))
                .chunk((1,))
                .create("listOfFrequencies")
                .expect("create finite-resizable frequency list")
                .write(&[value])
                .expect("write finite-resizable frequency list");
        }
        FixtureMutation::MetadataOversizedCompressedChunk => {
            let identification = file
                .group(IDENTIFICATION_PATH)
                .expect("identification group");
            identification
                .unlink("listOfFrequencies")
                .expect("remove fixed frequency list");
            let values = [
                FixedAscii::<32_769>::from_ascii("A")
                    .expect("frequency fits oversized fixed ASCII type"),
                FixedAscii::<32_769>::from_ascii("A")
                    .expect("frequency fits oversized fixed ASCII type"),
            ];
            identification
                .new_dataset::<FixedAscii<32_769>>()
                .shape((2,))
                .chunk((2,))
                .deflate(6)
                .create("listOfFrequencies")
                .expect("create oversized compressed metadata chunk")
                .write(&values)
                .expect("write oversized compressed metadata chunk");
        }
        FixtureMutation::MetadataFilteredChunk => {
            let identification = file
                .group(IDENTIFICATION_PATH)
                .expect("identification group");
            identification
                .unlink("listOfFrequencies")
                .expect("remove fixed frequency list");
            let value = FixedAscii::<2>::from_ascii("A").expect("frequency fits fixed ASCII type");
            identification
                .new_dataset::<FixedAscii<2>>()
                .shape((1,))
                .chunk((1,))
                .deflate(6)
                .create("listOfFrequencies")
                .expect("create filtered metadata chunk")
                .write(&[value])
                .expect("write filtered metadata chunk");
        }
    }
}

fn create_fixture(
    path: &Path,
    scope: FrequencyScope,
    granule_id: &str,
    product_specification_version: &str,
    image_shape: (usize, usize),
    chunk_image: bool,
) {
    let file = File::create(path).expect("create synthetic NISAR HDF5 fixture");
    let identification = file
        .create_group(IDENTIFICATION_PATH)
        .expect("create identification group");
    let swaths = file
        .create_group(SWATHS_PATH)
        .expect("create RSLC swaths group");
    let geolocation = file
        .create_group(GEOLOCATION_PATH)
        .expect("create geolocationGrid group");

    write_ascii_scalar::<16>(&identification, "missionId", "NISAR");
    write_ascii_scalar::<16>(&identification, "productType", "RSLC");
    write_ascii_scalar::<16>(&identification, "productLevel", "L1");
    write_ascii_scalar::<16>(&identification, "radarBand", "L");
    write_ascii_scalar::<16>(
        &identification,
        "productSpecificationVersion",
        product_specification_version,
    );
    if product_specification_version == P05023_PRODUCT_SPECIFICATION_VERSION {
        write_ascii_scalar::<16>(&identification, "compositeReleaseId", "P05023");
    }
    write_ascii_scalar::<128>(&identification, "granuleId", granule_id);
    write_ascii_list::<2>(
        &identification,
        "listOfFrequencies",
        scope.frequency_names(),
    );
    write_ascii_scalar::<128>(
        &identification,
        "boundingPolygon",
        "POLYGON ((117 30, 120 30, 120 32, 117 32, 117 30))",
    );

    if let Some(polarizations) = scope.frequency_a_polarizations() {
        create_frequency(
            &swaths,
            "frequencyA",
            polarizations,
            image_shape,
            chunk_image,
        );
    }
    if let Some(polarizations) = scope.frequency_b_polarizations() {
        create_frequency(
            &swaths,
            "frequencyB",
            polarizations,
            image_shape,
            chunk_image,
        );
    }

    let epsg = geolocation
        .new_dataset::<i32>()
        .create("epsg")
        .expect("create EPSG scalar");
    epsg.write_scalar(&4326_i32).expect("write EPSG scalar");
    geolocation
        .new_dataset::<f64>()
        .shape((1, 2, 3))
        .create("coordinateX")
        .expect("create coordinateX");
    geolocation
        .new_dataset::<f64>()
        .shape((1, 2, 3))
        .create("coordinateY")
        .expect("create coordinateY");
    for name in ["slantRange", "zeroDopplerTime", "heightAboveEllipsoid"] {
        geolocation
            .new_dataset::<f64>()
            .shape((2,))
            .create(name)
            .expect("create geolocation vector");
    }
}

fn create_frequency(
    swaths: &Group,
    name: &str,
    polarizations: &[&str],
    image_shape: (usize, usize),
    chunk_image: bool,
) {
    let frequency = swaths.create_group(name).expect("create frequency group");
    write_ascii_list::<4>(&frequency, "listOfPolarizations", polarizations);
    for polarization in polarizations {
        let builder = frequency.new_dataset::<CFloat32>().shape(image_shape);
        if chunk_image {
            builder
                .chunk((1, 1))
                .create(*polarization)
                .expect("create sparse logical CFloat32 image");
        } else {
            builder
                .create(*polarization)
                .expect("create CFloat32 image");
        }
    }
}

fn write_ascii_scalar<const N: usize>(group: &Group, name: &str, value: &str) {
    let value = FixedAscii::<N>::from_ascii(value).expect("fixture scalar fits fixed ASCII type");
    group
        .new_dataset::<FixedAscii<N>>()
        .create(name)
        .expect("create fixed ASCII scalar")
        .write_scalar(&value)
        .expect("write fixed ASCII scalar");
}

fn write_ascii_list<const N: usize>(group: &Group, name: &str, values: &[&str]) {
    let values = values
        .iter()
        .map(|value| {
            FixedAscii::<N>::from_ascii(value).expect("fixture list value fits fixed ASCII type")
        })
        .collect::<Vec<_>>();
    group
        .new_dataset::<FixedAscii<N>>()
        .shape((values.len(),))
        .create(name)
        .expect("create fixed ASCII list")
        .write(&values)
        .expect("write fixed ASCII list");
}

fn replace_ascii_scalar<const N: usize>(file: &File, group: &str, name: &str, value: &str) {
    let group = file.group(group).expect("open scalar owner");
    group.unlink(name).expect("remove scalar fixture dataset");
    write_ascii_scalar::<N>(&group, name, value);
}

fn replace_ascii_list<const N: usize>(file: &File, group: &str, name: &str, values: &[&str]) {
    let group = file.group(group).expect("open list owner");
    group.unlink(name).expect("remove list fixture dataset");
    write_ascii_list::<N>(&group, name, values);
}
