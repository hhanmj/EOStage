mod support;

use std::ffi::CString;
use std::path::Path;

use hdf5::plist::dataset_create::DatasetCreate;
use hdf5::{Dataspace, Datatype, File};
use hdf5_sys::h5d::{H5Dclose, H5Dcreate2};
use hdf5_sys::h5p::{H5Pset_chunk, H5P_DEFAULT};
use hdf5_sys::h5t::{H5T_order_t, H5Tset_order};
use insar_subset_lab::{
    discover_chunk_layout, discover_chunk_layout_with_prefix, read_raw_chunk_payloads,
    write_raw_chunk_payloads, FillValueDefinition, SubsetLabError, SupportedFilter, Window2D,
};

use support::{create_rslc_fixture, crop, ensure_group, CFloat32, TempRoot};

const DATASET: &str = "/science/LSAR/RSLC/swaths/frequencyA/HH";

#[test]
fn aligned_filtered_chunks_are_discovered_read_by_file_range_and_relocated_raw() {
    let root = TempRoot::new("aligned-raw");
    let source_path = root.file("source.h5");
    create_rslc_fixture(&source_path, 0, [8, 8], [2, 2]);
    let window = Window2D::new(2, 6, 2, 6, [8, 8]).unwrap();
    let layout = discover_chunk_layout(&source_path, DATASET, window).unwrap();

    assert_eq!(layout.dataset_shape, [8, 8]);
    assert_eq!(layout.chunk_shape, [2, 2]);
    assert_eq!(layout.chunks.len(), 4);
    assert_eq!(
        layout.filters,
        vec![
            SupportedFilter::Shuffle,
            SupportedFilter::Deflate(6),
            SupportedFilter::Fletcher32,
        ]
    );
    assert!(layout
        .chunks
        .windows(2)
        .all(|pair| pair[0].logical_offset < pair[1].logical_offset));
    let payloads = read_raw_chunk_payloads(&layout).unwrap();
    assert!(payloads.iter().all(|payload| !payload.bytes.is_empty()));

    let destination_path = root.file("aligned-subset.h5");
    let destination_file = File::create(&destination_path).unwrap();
    let destination_group = ensure_group(&destination_file, "/science/LSAR/RSLC/swaths/frequencyA");
    let destination = destination_group
        .new_dataset::<CFloat32>()
        .shape((4, 4))
        .chunk((2, 2))
        .shuffle()
        .deflate(6)
        .fletcher32()
        .create("HH")
        .unwrap();
    write_raw_chunk_payloads(&destination, &layout, &payloads, [2, 2]).unwrap();

    let source = File::open(&source_path)
        .unwrap()
        .dataset(DATASET)
        .unwrap()
        .read_raw::<CFloat32>()
        .unwrap();
    let expected = crop(&source, [8, 8], [2, 6, 2, 6]);
    assert_eq!(destination.read_raw::<CFloat32>().unwrap(), expected);
}

#[test]
fn unaligned_windows_plan_covering_chunks_but_cannot_claim_direct_relocation() {
    let root = TempRoot::new("unaligned");
    let source_path = root.file("source.h5");
    create_rslc_fixture(&source_path, 0, [8, 8], [2, 2]);
    let window = Window2D::new(1, 6, 3, 7, [8, 8]).unwrap();
    let layout = discover_chunk_layout(&source_path, DATASET, window).unwrap();
    assert_eq!(layout.chunks.len(), 9);
    let payloads = read_raw_chunk_payloads(&layout).unwrap();

    let destination_file = File::create(root.file("invalid-direct.h5")).unwrap();
    let destination = destination_file
        .new_dataset::<CFloat32>()
        .shape((8, 8))
        .chunk((2, 2))
        .shuffle()
        .deflate(6)
        .fletcher32()
        .create("image")
        .unwrap();
    assert_eq!(
        write_raw_chunk_payloads(&destination, &layout, &payloads, [1, 3]),
        Err(SubsetLabError::UnalignedDirectChunkOrigin)
    );
}

#[test]
fn raw_chunk_relocation_rejects_equal_width_datatype_reinterpretation() {
    let root = TempRoot::new("wrong-datatype");
    let source_path = root.file("source.h5");
    create_rslc_fixture(&source_path, 0, [8, 8], [2, 2]);
    let window = Window2D::new(2, 6, 2, 6, [8, 8]).unwrap();
    let layout = discover_chunk_layout(&source_path, DATASET, window).unwrap();
    let payloads = read_raw_chunk_payloads(&layout).unwrap();

    let destination_file = File::create(root.file("wrong-datatype.h5")).unwrap();
    let destination = destination_file
        .new_dataset::<u64>()
        .shape((4, 4))
        .chunk((2, 2))
        .shuffle()
        .deflate(6)
        .fletcher32()
        .create("image")
        .unwrap();
    assert_eq!(
        write_raw_chunk_payloads(&destination, &layout, &payloads, [2, 2]),
        Err(SubsetLabError::DatasetLayoutUnsupported)
    );
    assert_eq!(destination.num_chunks(), Some(0));
}

#[test]
fn raw_chunk_relocation_rejects_equal_width_opposite_byte_order() {
    let root = TempRoot::new("wrong-byte-order");
    let source_path = root.file("source-le.h5");
    let destination_path = root.file("destination-be.h5");
    create_ordered_u32_fixture(&source_path, H5T_order_t::H5T_ORDER_LE);
    create_ordered_u32_fixture(&destination_path, H5T_order_t::H5T_ORDER_BE);

    let window = Window2D::new(0, 4, 0, 4, [4, 4]).unwrap();
    let layout = discover_chunk_layout(&source_path, "/image", window).unwrap();
    let destination_layout = discover_chunk_layout(&destination_path, "/image", window).unwrap();
    assert_eq!(layout.datatype, destination_layout.datatype);
    assert_ne!(layout.datatype_sha256, destination_layout.datatype_sha256);
    assert_eq!(layout.datatype_sha256.len(), 64);
    let mut tampered_layout = layout.clone();
    tampered_layout.datatype_sha256 = destination_layout.datatype_sha256;
    assert_eq!(
        read_raw_chunk_payloads(&tampered_layout),
        Err(SubsetLabError::ChunkMetadataInvalid)
    );
    let payloads = read_raw_chunk_payloads(&layout).unwrap();
    assert!(payloads.is_empty());

    let destination_file = File::open_rw(&destination_path).unwrap();
    let destination = destination_file.dataset("/image").unwrap();
    assert_eq!(
        write_raw_chunk_payloads(&destination, &layout, &payloads, [0, 0]),
        Err(SubsetLabError::DatasetLayoutUnsupported)
    );
    assert_eq!(destination.num_chunks(), Some(0));
}

#[test]
fn raw_fill_identity_accepts_an_identical_nan_payload() {
    let root = TempRoot::new("same-nan-fill");
    let source_path = root.file("source.h5");
    let destination_path = root.file("destination.h5");
    let nan_bits = 0x7fc0_1234;
    create_f32_fill_fixture(&source_path, nan_bits);
    create_f32_fill_fixture(&destination_path, nan_bits);

    let window = Window2D::new(0, 4, 0, 4, [4, 4]).unwrap();
    let layout = discover_chunk_layout(&source_path, "/image", window).unwrap();
    assert_eq!(
        layout.fill_value_definition,
        FillValueDefinition::UserDefined
    );
    let payloads = read_raw_chunk_payloads(&layout).unwrap();
    assert!(payloads.is_empty());

    let destination_file = File::open_rw(&destination_path).unwrap();
    let destination = destination_file.dataset("/image").unwrap();
    write_raw_chunk_payloads(&destination, &layout, &payloads, [0, 0]).unwrap();
    assert_eq!(destination.num_chunks(), Some(0));
    assert!(destination
        .read_raw::<f32>()
        .unwrap()
        .iter()
        .all(|value| value.to_bits() == nan_bits));
}

#[test]
fn raw_fill_identity_rejects_negative_zero_as_positive_zero() {
    let root = TempRoot::new("signed-zero-fill");
    let source_path = root.file("source.h5");
    let destination_path = root.file("destination.h5");
    create_f32_fill_fixture(&source_path, (-0.0_f32).to_bits());
    create_f32_fill_fixture(&destination_path, 0.0_f32.to_bits());

    let window = Window2D::new(0, 4, 0, 4, [4, 4]).unwrap();
    let layout = discover_chunk_layout(&source_path, "/image", window).unwrap();
    let payloads = read_raw_chunk_payloads(&layout).unwrap();
    let destination_file = File::open_rw(&destination_path).unwrap();
    let destination = destination_file.dataset("/image").unwrap();
    assert_eq!(
        write_raw_chunk_payloads(&destination, &layout, &payloads, [0, 0]),
        Err(SubsetLabError::DatasetLayoutUnsupported)
    );
    assert_eq!(destination.num_chunks(), Some(0));
}

#[test]
fn raw_fill_identity_rejects_a_different_nan_payload() {
    let root = TempRoot::new("different-nan-fill");
    let source_path = root.file("source.h5");
    let destination_path = root.file("destination.h5");
    create_f32_fill_fixture(&source_path, 0x7fc0_1234);
    create_f32_fill_fixture(&destination_path, 0x7fc0_5678);

    let window = Window2D::new(0, 4, 0, 4, [4, 4]).unwrap();
    let layout = discover_chunk_layout(&source_path, "/image", window).unwrap();
    let payloads = read_raw_chunk_payloads(&layout).unwrap();
    let destination_file = File::open_rw(&destination_path).unwrap();
    let destination = destination_file.dataset("/image").unwrap();
    assert_eq!(
        write_raw_chunk_payloads(&destination, &layout, &payloads, [0, 0]),
        Err(SubsetLabError::DatasetLayoutUnsupported)
    );
    assert_eq!(destination.num_chunks(), Some(0));
}

#[test]
fn raw_fill_identity_rejects_default_and_user_defined_zero() {
    let root = TempRoot::new("fill-definition");
    let source_path = root.file("source.h5");
    let destination_path = root.file("destination.h5");
    let source_file = File::create(&source_path).unwrap();
    source_file
        .new_dataset::<f32>()
        .shape((4, 4))
        .chunk((2, 2))
        .create("image")
        .unwrap();
    source_file.flush().unwrap();
    drop(source_file);
    create_f32_fill_fixture(&destination_path, 0.0_f32.to_bits());

    let window = Window2D::new(0, 4, 0, 4, [4, 4]).unwrap();
    let layout = discover_chunk_layout(&source_path, "/image", window).unwrap();
    let destination_layout = discover_chunk_layout(&destination_path, "/image", window).unwrap();
    assert_eq!(layout.fill_value_definition, FillValueDefinition::Default);
    assert_eq!(
        destination_layout.fill_value_definition,
        FillValueDefinition::UserDefined
    );
    assert_eq!(layout.fill_value, destination_layout.fill_value);

    let payloads = read_raw_chunk_payloads(&layout).unwrap();
    let destination_file = File::open_rw(&destination_path).unwrap();
    let destination = destination_file.dataset("/image").unwrap();
    assert_eq!(
        write_raw_chunk_payloads(&destination, &layout, &payloads, [0, 0]),
        Err(SubsetLabError::DatasetLayoutUnsupported)
    );
    assert_eq!(destination.num_chunks(), Some(0));
}

#[test]
fn malformed_payload_batch_is_rejected_before_any_raw_chunk_is_written() {
    let root = TempRoot::new("payload-prevalidation");
    let source_path = root.file("source.h5");
    create_rslc_fixture(&source_path, 0, [8, 8], [2, 2]);
    let window = Window2D::new(2, 6, 2, 6, [8, 8]).unwrap();
    let layout = discover_chunk_layout(&source_path, DATASET, window).unwrap();
    let mut payloads = read_raw_chunk_payloads(&layout).unwrap();
    payloads.last_mut().unwrap().bytes[0] ^= 0xff;

    let destination_file = File::create(root.file("destination.h5")).unwrap();
    let destination = destination_file
        .new_dataset::<CFloat32>()
        .shape((4, 4))
        .chunk((2, 2))
        .shuffle()
        .deflate(6)
        .fletcher32()
        .create("image")
        .unwrap();
    assert_eq!(
        write_raw_chunk_payloads(&destination, &layout, &payloads, [2, 2]),
        Err(SubsetLabError::ChunkMetadataInvalid)
    );
    assert_eq!(destination.num_chunks(), Some(0));
}

#[test]
fn tampered_layout_cannot_remove_or_duplicate_a_required_chunk() {
    let root = TempRoot::new("layout-completeness");
    let source_path = root.file("source.h5");
    create_rslc_fixture(&source_path, 0, [8, 8], [2, 2]);
    let window = Window2D::new(2, 6, 2, 6, [8, 8]).unwrap();
    let layout = discover_chunk_layout(&source_path, DATASET, window).unwrap();
    let payloads = read_raw_chunk_payloads(&layout).unwrap();

    let mut missing = layout.clone();
    missing.chunks.pop();
    assert_eq!(
        read_raw_chunk_payloads(&missing),
        Err(SubsetLabError::ChunkMetadataInvalid)
    );
    let destination_file = File::create(root.file("missing-destination.h5")).unwrap();
    let destination = destination_file
        .new_dataset::<CFloat32>()
        .shape((4, 4))
        .chunk((2, 2))
        .shuffle()
        .deflate(6)
        .fletcher32()
        .create("image")
        .unwrap();
    assert_eq!(
        write_raw_chunk_payloads(&destination, &missing, &payloads, [2, 2]),
        Err(SubsetLabError::ChunkMetadataInvalid)
    );
    assert_eq!(destination.num_chunks(), Some(0));

    let mut duplicate = layout;
    duplicate.chunks[1] = duplicate.chunks[0].clone();
    assert_eq!(
        read_raw_chunk_payloads(&duplicate),
        Err(SubsetLabError::ChunkMetadataInvalid)
    );
}

#[test]
fn unallocated_chunks_preserve_the_source_fill_without_remote_payloads() {
    let root = TempRoot::new("fill-chunks");
    let source_path = root.file("source.h5");
    let source_file = File::create(&source_path).unwrap();
    source_file
        .new_dataset::<u8>()
        .shape((4, 4))
        .chunk((2, 2))
        .fill_value(7_u8)
        .create("image")
        .unwrap();
    source_file.flush().unwrap();
    drop(source_file);

    let window = Window2D::new(0, 4, 0, 4, [4, 4]).unwrap();
    let layout = discover_chunk_layout(&source_path, "/image", window).unwrap();
    assert_eq!(layout.chunks.len(), 4);
    assert!(layout.chunks.iter().all(|chunk| chunk.file_range.is_none()));
    let payloads = read_raw_chunk_payloads(&layout).unwrap();
    assert!(payloads.is_empty());

    let invalid_origin_file = File::create(root.file("invalid-origin.h5")).unwrap();
    let invalid_origin = invalid_origin_file
        .new_dataset::<u8>()
        .shape((4, 4))
        .chunk((2, 2))
        .fill_value(7_u8)
        .create("image")
        .unwrap();
    assert_eq!(
        write_raw_chunk_payloads(&invalid_origin, &layout, &payloads, [4, 4]),
        Err(SubsetLabError::UnalignedDirectChunkOrigin)
    );
    assert_eq!(invalid_origin.num_chunks(), Some(0));

    let nonempty_file = File::create(root.file("nonempty.h5")).unwrap();
    let nonempty = nonempty_file
        .new_dataset::<u8>()
        .shape((4, 4))
        .chunk((2, 2))
        .fill_value(7_u8)
        .create("image")
        .unwrap();
    nonempty.write_raw(&[9_u8; 16]).unwrap();
    assert_eq!(
        write_raw_chunk_payloads(&nonempty, &layout, &payloads, [0, 0]),
        Err(SubsetLabError::DestinationNotEmpty)
    );

    let destination_file = File::create(root.file("destination.h5")).unwrap();
    let destination = destination_file
        .new_dataset::<u8>()
        .shape((4, 4))
        .chunk((2, 2))
        .fill_value(7_u8)
        .create("image")
        .unwrap();
    write_raw_chunk_payloads(&destination, &layout, &payloads, [0, 0]).unwrap();
    assert_eq!(destination.num_chunks(), Some(0));
    assert_eq!(destination.read_raw::<u8>().unwrap(), vec![7_u8; 16]);
}

#[test]
fn requested_chunk_count_is_bounded_before_chunk_metadata_queries() {
    let root = TempRoot::new("chunk-budget");
    let source_path = root.file("source.h5");
    let source_file = File::create(&source_path).unwrap();
    source_file
        .new_dataset::<u8>()
        .shape((130, 130))
        .chunk((2, 2))
        .create("image")
        .unwrap();
    source_file.flush().unwrap();
    drop(source_file);

    assert_eq!(
        discover_chunk_layout(
            &source_path,
            "/image",
            Window2D::new(0, 130, 0, 130, [130, 130]).unwrap(),
        ),
        Err(SubsetLabError::ChunkBudgetExceeded)
    );
    let source_len = std::fs::metadata(&source_path).unwrap().len();
    assert_eq!(
        discover_chunk_layout_with_prefix(
            &source_path,
            &root.file("prefix-scratch.h5"),
            "/image",
            Window2D::new(0, 130, 0, 130, [130, 130]).unwrap(),
            source_len,
            source_len,
        ),
        Err(SubsetLabError::ChunkBudgetExceeded)
    );
}

#[test]
fn virtual_external_resizable_and_unreviewed_filters_fail_closed() {
    let root = TempRoot::new("unsupported-layouts");
    let path = root.file("unsupported.h5");
    let file = File::create(&path).unwrap();
    file.new_dataset::<u8>()
        .shape((4, 4))
        .chunk((2, 2))
        .nbit()
        .create("nbit")
        .unwrap()
        .write_raw(&[1_u8; 16])
        .unwrap();
    let external_link_path = root.file("external-link-target.h5");
    File::create(&external_link_path)
        .unwrap()
        .new_dataset::<u8>()
        .shape((4, 4))
        .chunk((2, 2))
        .create("image")
        .unwrap()
        .write_raw(&[1_u8; 16])
        .unwrap();
    file.link_external(
        external_link_path.to_str().unwrap(),
        "/image",
        "external_link",
    )
    .unwrap();
    file.link_external(path.to_str().unwrap(), "/nbit", "self_external")
        .unwrap();
    file.new_dataset::<u8>()
        .shape((4, 4))
        .external("external.raw", 0, 16)
        .create("external")
        .unwrap();
    file.new_dataset::<u8>()
        .shape((4, 4))
        .create("source")
        .unwrap();
    file.new_dataset::<u8>()
        .shape((4, 4))
        .virtual_map(".", "/source", (4, 4), .., (4, 4), ..)
        .create("virtual")
        .unwrap();
    file.new_dataset::<u8>()
        .shape((4_usize..=8, 4_usize))
        .chunk((2, 2))
        .create("resizable")
        .unwrap()
        .write_raw(&[1_u8; 16])
        .unwrap();
    file.flush().unwrap();
    drop(file);
    let window = Window2D::new(0, 4, 0, 4, [4, 4]).unwrap();

    assert_eq!(
        discover_chunk_layout(&path, "/nbit", window),
        Err(SubsetLabError::FilterUnsupported)
    );
    for dataset in ["/external", "/virtual", "/external_link", "/self_external"] {
        assert_eq!(
            discover_chunk_layout(&path, dataset, window),
            Err(SubsetLabError::ExternalOrVirtualStorage),
            "{dataset}"
        );
    }
    assert_eq!(
        discover_chunk_layout(&path, "/resizable", window),
        Err(SubsetLabError::DatasetLayoutUnsupported)
    );
    assert_eq!(
        discover_chunk_layout_with_prefix(
            &path,
            &root.file("unsupported-prefix.h5"),
            "/nbit",
            window,
            512,
            std::fs::metadata(&path).unwrap().len(),
        ),
        Err(SubsetLabError::FilterUnsupported)
    );
}

fn create_f32_fill_fixture(path: &Path, fill_bits: u32) {
    let file = File::create(path).unwrap();
    file.new_dataset::<f32>()
        .shape((4, 4))
        .chunk((2, 2))
        .fill_value(f32::from_bits(fill_bits))
        .create("image")
        .unwrap();
    file.flush().unwrap();
}

fn create_ordered_u32_fixture(path: &Path, order: H5T_order_t) {
    let file = File::create(path).unwrap();
    let datatype = Datatype::from_type::<u32>().unwrap();
    let order_status = hdf5::sync::sync(|| unsafe { H5Tset_order(datatype.id(), order) });
    assert_eq!(order_status, 0, "set explicit file datatype byte order");

    let dataspace = Dataspace::try_new((4, 4)).unwrap();
    let dcpl = DatasetCreate::try_new().unwrap();
    let chunk = [2_u64, 2_u64];
    let chunk_status =
        hdf5::sync::sync(|| unsafe { H5Pset_chunk(dcpl.id(), chunk.len() as i32, chunk.as_ptr()) });
    assert_eq!(chunk_status, 0, "set ordered fixture chunk shape");

    let name = CString::new("image").unwrap();
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
    assert!(dataset_id >= 0, "create ordered fixture dataset");
    let close_status = hdf5::sync::sync(|| unsafe { H5Dclose(dataset_id) });
    assert_eq!(close_status, 0, "close ordered fixture dataset");
    file.flush().unwrap();
}
