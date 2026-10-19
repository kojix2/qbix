use std::ffi::CString;
use std::os::raw::{c_char, c_int};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[repr(C)]
struct RawHtsFile {
    _private: [u8; 0],
}

#[repr(C)]
struct RawSamHdr {
    _private: [u8; 0],
}

#[repr(C)]
struct RawBam1 {
    _private: [u8; 0],
}

#[repr(C)]
struct RawIndex {
    _private: [u8; 0],
}

#[repr(C)]
struct RawIterator {
    _private: [u8; 0],
}

#[repr(C)]
struct RawBgzf {
    _private: [u8; 0],
}

#[link(name = "hts")]
// The test helper links htslib directly, so static htslib dependencies are not
// inherited through the qbix crate's build script link directives.
#[link(name = "deflate")]
#[link(name = "z")]
extern "C" {
    fn hts_open(path: *const c_char, mode: *const c_char) -> *mut RawHtsFile;
    fn hts_close(fp: *mut RawHtsFile) -> c_int;
    fn sam_hdr_parse(l_text: usize, text: *const c_char) -> *mut RawSamHdr;
    #[allow(dead_code)]
    fn sam_hdr_read(fp: *mut RawHtsFile) -> *mut RawSamHdr;
    #[allow(dead_code)]
    fn sam_hdr_str(h: *mut RawSamHdr) -> *const c_char;
    fn sam_hdr_write(fp: *mut RawHtsFile, h: *const RawSamHdr) -> c_int;
    fn sam_hdr_destroy(h: *mut RawSamHdr);
    #[allow(dead_code)]
    fn sam_index_load(fp: *mut RawHtsFile, path: *const c_char) -> *mut RawIndex;
    #[allow(dead_code)]
    fn hts_idx_destroy(index: *mut RawIndex);
    #[allow(dead_code)]
    fn sam_itr_queryi(index: *const RawIndex, tid: c_int, begin: i64, end: i64)
        -> *mut RawIterator;
    #[allow(dead_code)]
    fn hts_itr_next(
        fp: *mut RawBgzf,
        iter: *mut RawIterator,
        record: *mut RawBam1,
        data: *mut RawHtsFile,
    ) -> c_int;
    #[allow(dead_code)]
    fn hts_itr_destroy(iter: *mut RawIterator);
    #[allow(dead_code)]
    fn hts_get_bgzfp(fp: *mut RawHtsFile) -> *mut RawBgzf;
    fn bam_init1() -> *mut RawBam1;
    fn bam_destroy1(b: *mut RawBam1);
    fn bam_set1(
        bam: *mut RawBam1,
        l_qname: usize,
        qname: *const c_char,
        flag: u16,
        tid: i32,
        pos: i64,
        mapq: u8,
        n_cigar: usize,
        cigar: *const u32,
        mtid: i32,
        mpos: i64,
        isize: i64,
        l_seq: usize,
        seq: *const c_char,
        qual: *const c_char,
        l_aux: usize,
    ) -> c_int;
    fn sam_write1(fp: *mut RawHtsFile, h: *const RawSamHdr, b: *const RawBam1) -> c_int;
}

pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new(name: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("qbix-{name}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

pub fn write_unmapped_bam(path: &str, readnames: &[&str]) {
    write_unmapped_bam_with_header(
        path,
        readnames,
        "@HD\tVN:1.6\tSO:unknown\n@SQ\tSN:chr1\tLN:1000\n",
    );
}

pub fn write_unmapped_bam_with_header(path: &str, readnames: &[&str], header_text: &str) {
    let bam = HtsFile::open(path, "wb");
    let header = Header::parse(header_text);
    assert_eq!(unsafe { sam_hdr_write(bam.0, header.0) }, 0);

    let record = BamRecord::new();
    for readname in readnames {
        record.set_unmapped(readname);
        assert!(unsafe { sam_write1(bam.0, header.0, record.0) } >= 0);
    }
}

#[allow(dead_code)]
pub fn write_coordinate_bam(path: &str, records: &[(&str, i64)]) {
    let bam = HtsFile::open(path, "wb");
    let header = Header::parse("@HD\tVN:1.6\tSO:coordinate\n@SQ\tSN:chr1\tLN:1000\n");
    assert_eq!(unsafe { sam_hdr_write(bam.0, header.0) }, 0);
    let record = BamRecord::new();
    for &(name, pos) in records {
        record.set_mapped(name, pos);
        assert!(unsafe { sam_write1(bam.0, header.0, record.0) } >= 0);
    }
}

#[allow(dead_code)]
pub fn read_bam_header(path: &str) -> String {
    let bam = HtsFile::open(path, "rb");
    let header = Header(unsafe { sam_hdr_read(bam.0) });
    assert!(!header.0.is_null());
    let text = unsafe { sam_hdr_str(header.0) };
    assert!(!text.is_null());
    unsafe { std::ffi::CStr::from_ptr(text) }
        .to_str()
        .unwrap()
        .to_string()
}

#[allow(dead_code)]
pub fn bam_index_loads(path: &str) -> bool {
    let bam = HtsFile::open(path, "rb");
    let path = CString::new(path).unwrap();
    let index = unsafe { sam_index_load(bam.0, path.as_ptr()) };
    if index.is_null() {
        false
    } else {
        unsafe { hts_idx_destroy(index) };
        true
    }
}

#[allow(dead_code)]
pub fn count_region(path: &str, begin: i64, end: i64) -> usize {
    let bam = HtsFile::open(path, "rb");
    let header = Header(unsafe { sam_hdr_read(bam.0) });
    assert!(!header.0.is_null());
    let path = CString::new(path).unwrap();
    let index = unsafe { sam_index_load(bam.0, path.as_ptr()) };
    assert!(!index.is_null());
    let iter = unsafe { sam_itr_queryi(index, 0, begin, end) };
    assert!(!iter.is_null());
    let record = BamRecord::new();
    let mut count = 0;
    loop {
        let result = unsafe { hts_itr_next(hts_get_bgzfp(bam.0), iter, record.0, bam.0) };
        if result == -1 {
            break;
        }
        assert!(result >= 0);
        count += 1;
    }
    unsafe {
        hts_itr_destroy(iter);
        hts_idx_destroy(index);
    }
    count
}

struct HtsFile(*mut RawHtsFile);
struct Header(*mut RawSamHdr);
struct BamRecord(*mut RawBam1);

impl HtsFile {
    fn open(path: &str, mode: &str) -> Self {
        let path = CString::new(path).unwrap();
        let mode = CString::new(mode).unwrap();
        let fp = unsafe { hts_open(path.as_ptr(), mode.as_ptr()) };
        assert!(!fp.is_null());
        Self(fp)
    }
}

impl Drop for HtsFile {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                hts_close(self.0);
            }
        }
    }
}

impl Header {
    fn parse(text: &str) -> Self {
        let text = CString::new(text).unwrap();
        let header = unsafe { sam_hdr_parse(text.as_bytes().len(), text.as_ptr()) };
        assert!(!header.is_null());
        Self(header)
    }
}

impl Drop for Header {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                sam_hdr_destroy(self.0);
            }
        }
    }
}

impl BamRecord {
    fn new() -> Self {
        let record = unsafe { bam_init1() };
        assert!(!record.is_null());
        Self(record)
    }

    fn set_unmapped(&self, readname: &str) {
        let readname = CString::new(readname).unwrap();
        let ret = unsafe {
            bam_set1(
                self.0,
                readname.as_bytes().len(),
                readname.as_ptr(),
                4,
                -1,
                -1,
                0,
                0,
                std::ptr::null(),
                -1,
                -1,
                0,
                0,
                std::ptr::null(),
                std::ptr::null(),
                0,
            )
        };
        assert!(ret >= 0);
    }

    fn set_mapped(&self, readname: &str, pos: i64) {
        let readname = CString::new(readname).unwrap();
        let cigar = [16_u32]; // 1M
        let seq = *b"A";
        let qual = [30_u8];
        let ret = unsafe {
            bam_set1(
                self.0,
                readname.as_bytes().len(),
                readname.as_ptr(),
                0,
                0,
                pos,
                60,
                1,
                cigar.as_ptr(),
                -1,
                -1,
                0,
                seq.len(),
                seq.as_ptr().cast(),
                qual.as_ptr().cast(),
                0,
            )
        };
        assert!(ret >= 0);
    }
}

impl Drop for BamRecord {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                bam_destroy1(self.0);
            }
        }
    }
}
