mod common;

use std::ffi::{CStr, CString};
use std::ptr;

use common::{write_unmapped_bam, TempDir};

#[test]
fn c_api_builds_opens_looks_up_and_closes() {
    let temp = TempDir::new("c-api");
    let bam = temp.path().join("reads.bam");
    let bam_str = bam.to_str().unwrap();
    write_unmapped_bam(bam_str, &["read_b", "read_a", "read_a"]);

    let bam = CString::new(bam_str).unwrap();
    assert_eq!(
        qbix::c_api::qbix_build_index(bam.as_ptr(), ptr::null(), 1),
        0
    );
    assert_eq!(
        &std::fs::read(format!("{bam_str}.qbi")).unwrap()[..4],
        b"QBI2"
    );
    assert_eq!(
        qbix::c_api::qbix_check_index(bam.as_ptr(), ptr::null(), 1, qbix::c_api::QBIX_CHECK_QUICK,),
        0,
        "{}",
        last_error()
    );
    assert_eq!(
        qbix::c_api::qbix_check_index(bam.as_ptr(), ptr::null(), 1, qbix::c_api::QBIX_CHECK_FULL,),
        0,
        "{}",
        last_error()
    );

    let index = qbix::c_api::qbix_index_open(bam.as_ptr(), ptr::null(), 1);
    assert!(!index.is_null(), "{}", last_error());

    let read_name = CString::new("read_a").unwrap();
    let mut hits = ptr::null_mut();
    let mut hit_count = 0usize;
    let ret = unsafe {
        qbix::c_api::qbix_index_lookup(index, read_name.as_ptr(), &mut hits, &mut hit_count)
    };
    assert_eq!(ret, 0, "{}", last_error());
    assert_eq!(hit_count, 2);
    assert!(!hits.is_null());

    let hits_slice = unsafe { std::slice::from_raw_parts(hits, hit_count) };
    assert!(hits_slice.iter().all(|hit| hit.virtual_offset >= 0));

    unsafe {
        qbix::c_api::qbix_hits_free(hits, hit_count);
        qbix::c_api::qbix_index_close(index);
    }
}

#[test]
fn c_api_reader_opens_qbi2() {
    let temp = TempDir::new("c-api-qbi2");
    let bam_path = temp.path().join("reads.bam");
    let index_path = temp.path().join("reads.qbi");
    write_unmapped_bam(bam_path.to_str().unwrap(), &["read_a", "read_a", "read_b"]);
    let mut options = qbix::BuildOptions::default();
    options.index_path = Some(index_path.clone());
    options.index_format = Some(qbix::IndexFormat::Qbi2);
    options.qbi2_radix_bits = Some(8);
    qbix::build_index(&bam_path, options).unwrap();
    assert_eq!(std::fs::metadata(&index_path).unwrap().len(), 2_246);

    let bam = CString::new(bam_path.to_str().unwrap()).unwrap();
    let index = CString::new(index_path.to_str().unwrap()).unwrap();
    let handle = qbix::c_api::qbix_index_open(bam.as_ptr(), index.as_ptr(), 1);
    assert!(!handle.is_null(), "{}", last_error());
    let read_name = CString::new("read_a").unwrap();
    let mut hits = ptr::null_mut();
    let mut hit_count = 0usize;
    let ret = unsafe {
        qbix::c_api::qbix_index_lookup(handle, read_name.as_ptr(), &mut hits, &mut hit_count)
    };
    assert_eq!(ret, 0, "{}", last_error());
    assert_eq!(hit_count, 2);
    unsafe {
        qbix::c_api::qbix_hits_free(hits, hit_count);
        qbix::c_api::qbix_index_close(handle);
    }
}

#[test]
fn c_api_reports_last_error() {
    let ret = qbix::c_api::qbix_build_index(ptr::null(), ptr::null(), 1);
    assert_eq!(ret, -1);
    assert!(last_error().contains("bam_path is null"));
}

#[test]
fn c_api_lookup_clears_outputs_before_failure() {
    let mut hits = std::ptr::NonNull::<qbix::c_api::qbix_hit_t>::dangling().as_ptr();
    let mut hit_count = 42usize;

    let ret = unsafe {
        qbix::c_api::qbix_index_lookup(ptr::null_mut(), ptr::null(), &mut hits, &mut hit_count)
    };

    assert_eq!(ret, -1);
    assert!(hits.is_null());
    assert_eq!(hit_count, 0);
    assert!(last_error().contains("index handle is null"));
}

#[test]
fn c_api_rejects_unknown_check_mode() {
    let temp = TempDir::new("c-api-check-mode");
    let bam = temp.path().join("reads.bam");
    let bam_str = bam.to_str().unwrap();
    write_unmapped_bam(bam_str, &["read_a"]);

    let bam = CString::new(bam_str).unwrap();
    assert_eq!(
        qbix::c_api::qbix_build_index(bam.as_ptr(), ptr::null(), 1),
        0
    );

    let ret = qbix::c_api::qbix_check_index(bam.as_ptr(), ptr::null(), 1, 99);
    assert_eq!(ret, -1);
    assert!(last_error().contains("unsupported check mode"));
}

fn last_error() -> String {
    let ptr = qbix::c_api::qbix_last_error();
    assert!(!ptr.is_null());
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}
