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

#[link(name = "hts")]
// The test helper links htslib directly, so static htslib dependencies are not
// inherited through the qbix crate's build script link directives.
#[link(name = "deflate")]
#[link(name = "z")]
extern "C" {
    fn hts_open(path: *const c_char, mode: *const c_char) -> *mut RawHtsFile;
    fn hts_close(fp: *mut RawHtsFile) -> c_int;
    fn sam_hdr_parse(l_text: usize, text: *const c_char) -> *mut RawSamHdr;
    fn sam_hdr_write(fp: *mut RawHtsFile, h: *const RawSamHdr) -> c_int;
    fn sam_hdr_destroy(h: *mut RawSamHdr);
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
    let bam = HtsFile::open(path, "wb");
    let header = Header::parse("@HD\tVN:1.6\tSO:unknown\n@SQ\tSN:chr1\tLN:1000\n");
    assert_eq!(unsafe { sam_hdr_write(bam.0, header.0) }, 0);

    let record = BamRecord::new();
    for readname in readnames {
        record.set_unmapped(readname);
        assert!(unsafe { sam_write1(bam.0, header.0, record.0) } >= 0);
    }
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
