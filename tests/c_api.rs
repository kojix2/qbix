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
fn c_api_reports_last_error() {
    let ret = qbix::c_api::qbix_build_index(ptr::null(), ptr::null(), 1);
    assert_eq!(ret, -1);
    assert!(last_error().contains("bam_path is null"));
}

fn last_error() -> String {
    let ptr = qbix::c_api::qbix_last_error();
    assert!(!ptr.is_null());
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}
