mod support;

use std::fs;
use std::path::Path;

use hdf5::File;
use insar_subset_lab::{
    copy_allowlisted_datasets, discover_chunk_layout, read_raw_chunk_payloads,
    write_raw_chunk_payloads, SubsetLabError, Window2D, SYNTHETIC_RSLC_METADATA_ALLOWLIST,
};
use sha2::{Digest, Sha256};

use support::{create_rslc_fixture, crop, ensure_group, CFloat32, TempRoot};

const SHAPE: [usize; 2] = [12, 12];
const CHUNK: [usize; 2] = [4, 4];
const SLANT_RANGE: &str = "/science/LSAR/RSLC/swaths/slantRange";
const ZERO_DOPPLER_TIME: &str = "/science/LSAR/RSLC/swaths/zeroDopplerTime";
#[test]
fn unaligned_disjoint_windows_decode_through_sparse_chunks_and_preserve_declared_science() {
    let root = TempRoot::new("metadata-preservation");
    let source_path = root.file("complete-source.h5");
    let output_path = root.file("client-derived-subset.h5");
    create_rslc_fixture(&source_path, 0, SHAPE, CHUNK);

    let windows = [
        Window2D::new(1, 6, 2, 7, SHAPE).unwrap(),
        Window2D::new(8, 11, 9, 12, SHAPE).unwrap(),
    ];
    let datasets = [
        ("frequencyA", "HH"),
        ("frequencyA", "HV"),
        ("frequencyB", "VV"),
    ];

    let source = File::open(&source_path).unwrap();
    let output = File::create(&output_path).unwrap();
    let copied =
        copy_allowlisted_datasets(&source, &output, &SYNTHETIC_RSLC_METADATA_ALLOWLIST).unwrap();
    assert_eq!(
        copied
            .iter()
            .map(|record| record.path.as_str())
            .collect::<Vec<_>>(),
        SYNTHETIC_RSLC_METADATA_ALLOWLIST
    );
    assert_eq!(
        copy_allowlisted_datasets(&source, &output, &["relative/path"]),
        Err(SubsetLabError::MetadataPathInvalid)
    );
    assert_eq!(
        copy_allowlisted_datasets(
            &source,
            &output,
            &["/science/LSAR/RSLC/swaths/frequencyA/HH"]
        ),
        Err(SubsetLabError::MetadataPathInvalid)
    );

    let source_slant_range = source
        .dataset(SLANT_RANGE)
        .unwrap()
        .read_raw::<f64>()
        .unwrap();
    let source_zero_doppler = source
        .dataset(ZERO_DOPPLER_TIME)
        .unwrap()
        .read_raw::<f64>()
        .unwrap();
    let mut derived_science_elements = 0_usize;
    let mut saw_negative_zero = false;
    let mut saw_subnormal = false;
    let mut saw_low_amplitude = false;

    for (component_index, window) in windows.into_iter().enumerate() {
        let component_path = format!("/derived/components/component-{component_index}");
        let component = ensure_group(&output, &component_path);
        component
            .new_dataset::<u64>()
            .shape((4,))
            .create("sourceWindow")
            .unwrap()
            .write_raw(&[
                window.row_start as u64,
                window.row_end as u64,
                window.column_start as u64,
                window.column_end as u64,
            ])
            .unwrap();

        let coordinates = ensure_group(&output, &format!("{component_path}/coordinates"));
        let expected_zero_doppler = source_zero_doppler[window.row_start..window.row_end].to_vec();
        let expected_slant_range =
            source_slant_range[window.column_start..window.column_end].to_vec();
        coordinates
            .new_dataset::<f64>()
            .shape((expected_zero_doppler.len(),))
            .create("zeroDopplerTime")
            .unwrap()
            .write_raw(&expected_zero_doppler)
            .unwrap();
        coordinates
            .new_dataset::<f64>()
            .shape((expected_slant_range.len(),))
            .create("slantRange")
            .unwrap()
            .write_raw(&expected_slant_range)
            .unwrap();

        for (frequency, polarization) in datasets {
            let source_dataset_path =
                format!("/science/LSAR/RSLC/swaths/{frequency}/{polarization}");
            let layout = discover_chunk_layout(&source_path, &source_dataset_path, window).unwrap();
            let payloads = read_raw_chunk_payloads(&layout).unwrap();

            let staging_path = root.file(&format!(
                "staging-{component_index}-{frequency}-{polarization}.h5"
            ));
            let staging_file = File::create(&staging_path).unwrap();
            let staging = staging_file
                .new_dataset::<CFloat32>()
                .shape(SHAPE)
                .chunk(CHUNK)
                .shuffle()
                .deflate(6)
                .fletcher32()
                .create("image")
                .unwrap();
            write_raw_chunk_payloads(&staging, &layout, &payloads, [0, 0]).unwrap();
            assert_eq!(staging.num_chunks(), Some(layout.chunks.len()));

            let decoded = staging
                .read_slice_2d::<CFloat32, _>((
                    window.row_start..window.row_end,
                    window.column_start..window.column_end,
                ))
                .unwrap()
                .iter()
                .copied()
                .collect::<Vec<_>>();
            let source_values = source
                .dataset(&source_dataset_path)
                .unwrap()
                .read_raw::<CFloat32>()
                .unwrap();
            let expected = crop(
                &source_values,
                SHAPE,
                [
                    window.row_start,
                    window.row_end,
                    window.column_start,
                    window.column_end,
                ],
            );
            assert_complex_science_equal(&decoded, &expected);
            saw_negative_zero |= decoded
                .iter()
                .any(|value| value.r.to_bits() == (-0.0_f32).to_bits());
            saw_subnormal |= decoded.iter().any(|value| {
                value.r.to_bits() == 1 && value.i.to_bits() == (-f32::from_bits(1)).to_bits()
            });
            saw_low_amplitude |= decoded
                .iter()
                .any(|value| value.r.to_bits() == 1.0e-30_f32.to_bits());

            let output_group = ensure_group(
                &output,
                &format!("{component_path}/science/LSAR/RSLC/swaths/{frequency}"),
            );
            let output_dataset = output_group
                .new_dataset::<CFloat32>()
                .shape(window.shape())
                .chunk((
                    CHUNK[0].min(window.shape()[0]),
                    CHUNK[1].min(window.shape()[1]),
                ))
                .shuffle()
                .deflate(6)
                .fletcher32()
                .create(polarization)
                .unwrap();
            output_dataset.write_raw(&decoded).unwrap();
            let round_trip = output_dataset.read_raw::<CFloat32>().unwrap();
            assert_complex_science_equal(&round_trip, &expected);
            derived_science_elements += output_dataset.size();
        }

        assert_eq!(
            coordinates
                .dataset("zeroDopplerTime")
                .unwrap()
                .read_raw::<f64>()
                .unwrap(),
            expected_zero_doppler
        );
        assert_eq!(
            coordinates
                .dataset("slantRange")
                .unwrap()
                .read_raw::<f64>()
                .unwrap(),
            expected_slant_range
        );
    }

    let separate_component_elements = windows
        .iter()
        .map(|window| window.shape()[0] * window.shape()[1])
        .sum::<usize>()
        * datasets.len();
    let bridged_rectangle_elements = (windows[1].row_end - windows[0].row_start)
        * (windows[1].column_end - windows[0].column_start)
        * datasets.len();
    assert_eq!(derived_science_elements, separate_component_elements);
    assert!(derived_science_elements < bridged_rectangle_elements);
    assert!(saw_negative_zero);
    assert!(saw_subnormal);
    assert!(saw_low_amplitude);

    output.flush().unwrap();
    drop(output);
    drop(source);
    let checksum_before_reopen = file_sha256(&output_path);
    assert_eq!(checksum_before_reopen.len(), 64);

    let reopened = File::open(&output_path).unwrap();
    assert!(reopened.group("/derived/components/component-0").is_ok());
    assert!(reopened.group("/derived/components/component-1").is_ok());
    assert!(reopened.group("/derived/components/component-2").is_err());
    for metadata_path in SYNTHETIC_RSLC_METADATA_ALLOWLIST {
        assert!(reopened.dataset(metadata_path).is_ok(), "{metadata_path}");
    }
    drop(reopened);
    assert_eq!(file_sha256(&output_path), checksum_before_reopen);
}

#[test]
fn metadata_allowlist_rejects_oversized_objects_before_copy() {
    let root = TempRoot::new("metadata-budget");
    let source = File::create(root.file("oversized-source.h5")).unwrap();
    let identification = ensure_group(&source, "/science/LSAR/identification");
    identification
        .new_dataset::<u8>()
        .shape((1024 * 1024 + 1,))
        .create("missionId")
        .unwrap();
    let output = File::create(root.file("empty-output.h5")).unwrap();

    assert_eq!(
        copy_allowlisted_datasets(
            &source,
            &output,
            &["/science/LSAR/identification/missionId"]
        ),
        Err(SubsetLabError::MetadataBudgetExceeded)
    );
    assert!(output
        .dataset("/science/LSAR/identification/missionId")
        .is_err());
}

#[test]
fn metadata_batch_rejects_attributes_before_copying_earlier_valid_objects() {
    let root = TempRoot::new("metadata-attributes");
    let source_path = root.file("source.h5");
    create_rslc_fixture(&source_path, 0, SHAPE, CHUNK);
    let source = File::open_rw(&source_path).unwrap();
    source
        .dataset("/science/LSAR/identification/productType")
        .unwrap()
        .new_attr::<u8>()
        .create("unreviewed")
        .unwrap()
        .write_scalar(&1_u8)
        .unwrap();
    let output = File::create(root.file("output.h5")).unwrap();

    assert_eq!(
        copy_allowlisted_datasets(
            &source,
            &output,
            &[
                "/science/LSAR/identification/missionId",
                "/science/LSAR/identification/productType",
            ],
        ),
        Err(SubsetLabError::MetadataAttributeUnsupported)
    );
    assert!(output
        .dataset("/science/LSAR/identification/missionId")
        .is_err());
}

fn assert_complex_science_equal(actual: &[CFloat32], expected: &[CFloat32]) {
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        assert_eq!(actual.r.to_bits(), expected.r.to_bits());
        assert_eq!(actual.i.to_bits(), expected.i.to_bits());
        let actual_amplitude = actual.r.hypot(actual.i);
        let expected_amplitude = expected.r.hypot(expected.i);
        assert_eq!(actual_amplitude.to_bits(), expected_amplitude.to_bits());
        let actual_phase = actual.i.atan2(actual.r);
        let expected_phase = expected.i.atan2(expected.r);
        assert_eq!(actual_phase.to_bits(), expected_phase.to_bits());
    }
}

fn file_sha256(path: &Path) -> String {
    format!("{:x}", Sha256::digest(fs::read(path).unwrap()))
}
