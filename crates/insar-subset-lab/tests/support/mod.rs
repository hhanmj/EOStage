#![allow(dead_code, non_local_definitions)]

pub mod http;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use hdf5::types::FixedAscii;
use hdf5::{File, FileBuilder, Group};

static NEXT_ROOT: AtomicUsize = AtomicUsize::new(0);

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, hdf5::H5Type)]
pub struct CFloat32 {
    pub r: f32,
    pub i: f32,
}

pub struct TempRoot {
    path: PathBuf,
}

impl TempRoot {
    pub fn new(label: &str) -> Self {
        let ordinal = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "insar-subset-lab-{label}-{}-{ordinal}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create isolated subset-lab directory");
        Self { path }
    }

    pub fn file(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub fn create_rslc_fixture(path: &Path, userblock: u64, shape: [usize; 2], chunk: [usize; 2]) {
    let mut builder = FileBuilder::new();
    if userblock > 0 {
        builder.with_fcpl(|properties| properties.userblock(userblock));
    }
    let file = builder.create(path).expect("create synthetic RSLC fixture");
    let identification = ensure_group(&file, "/science/LSAR/identification");
    write_ascii_scalar::<32>(&identification, "missionId", "NISAR");
    write_ascii_scalar::<32>(&identification, "productType", "RSLC");
    write_ascii_scalar::<128>(&identification, "granuleId", "NISAR_L1_PR_RSLC_SUBSET_LAB");
    write_ascii_scalar::<128>(
        &identification,
        "boundingPolygon",
        "POLYGON ((117 30, 120 30, 120 32, 117 32, 117 30))",
    );
    write_ascii_list::<4>(&identification, "listOfFrequencies", &["A", "B"]);

    let swaths = ensure_group(&file, "/science/LSAR/RSLC/swaths");
    swaths
        .new_dataset::<f64>()
        .shape((shape[1],))
        .create("slantRange")
        .unwrap()
        .write_raw(
            &(0..shape[1])
                .map(|column| 800_000.0 + column as f64 * 10.0)
                .collect::<Vec<_>>(),
        )
        .unwrap();
    swaths
        .new_dataset::<f64>()
        .shape((shape[0],))
        .create("zeroDopplerTime")
        .unwrap()
        .write_raw(
            &(0..shape[0])
                .map(|row| row as f64 * 0.002)
                .collect::<Vec<_>>(),
        )
        .unwrap();

    for (frequency, polarizations, seed) in [
        ("frequencyA", [Some("HH"), Some("HV")], 1_000.0_f32),
        ("frequencyB", [Some("VV"), None], 2_000.0_f32),
    ] {
        let group = ensure_group(&file, &format!("/science/LSAR/RSLC/swaths/{frequency}"));
        let values = polarizations.into_iter().flatten().collect::<Vec<_>>();
        write_ascii_list::<4>(&group, "listOfPolarizations", &values);
        for (polarization_index, polarization) in values.into_iter().enumerate() {
            let data = complex_values(shape, seed + polarization_index as f32 * 100.0);
            group
                .new_dataset::<CFloat32>()
                .shape(shape)
                .chunk(chunk)
                .shuffle()
                .deflate(6)
                .fletcher32()
                .create(polarization)
                .unwrap()
                .write_raw(&data)
                .unwrap();
        }
    }
    file.flush().unwrap();
}

pub fn complex_values(shape: [usize; 2], seed: f32) -> Vec<CFloat32> {
    (0..shape[0])
        .flat_map(|row| {
            (0..shape[1]).map(move |column| {
                let linear = row * shape[1] + column;
                match linear {
                    14 => CFloat32 { r: -0.0, i: 0.0 },
                    15 => CFloat32 {
                        r: f32::from_bits(1),
                        i: -f32::from_bits(1),
                    },
                    105 => CFloat32 {
                        r: 1.0e-30,
                        i: -1.0e-30,
                    },
                    _ => {
                        let value = seed + linear as f32;
                        CFloat32 {
                            r: value,
                            i: -value / 7.0,
                        }
                    }
                }
            })
        })
        .collect()
}

pub fn crop<T: Copy>(source: &[T], shape: [usize; 2], window: [usize; 4]) -> Vec<T> {
    let [row_start, row_end, column_start, column_end] = window;
    (row_start..row_end)
        .flat_map(|row| {
            (column_start..column_end).map(move |column| source[row * shape[1] + column])
        })
        .collect()
}

pub fn ensure_group(file: &File, path: &str) -> Group {
    let mut group = file.group("/").unwrap();
    for component in path.split('/').filter(|component| !component.is_empty()) {
        group = if group.link_exists(component) {
            group.group(component).unwrap()
        } else {
            group.create_group(component).unwrap()
        };
    }
    group
}

fn write_ascii_scalar<const N: usize>(group: &Group, name: &str, value: &str) {
    let value = FixedAscii::<N>::from_ascii(value).unwrap();
    group
        .new_dataset::<FixedAscii<N>>()
        .create(name)
        .unwrap()
        .write_scalar(&value)
        .unwrap();
}

fn write_ascii_list<const N: usize>(group: &Group, name: &str, values: &[&str]) {
    let values = values
        .iter()
        .map(|value| FixedAscii::<N>::from_ascii(value).unwrap())
        .collect::<Vec<_>>();
    group
        .new_dataset::<FixedAscii<N>>()
        .shape((values.len(),))
        .create(name)
        .unwrap()
        .write_raw(&values)
        .unwrap();
}
