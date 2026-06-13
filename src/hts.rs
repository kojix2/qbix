use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};

use crate::error::Result;

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

extern "C" {
    fn hts_open(path: *const c_char, mode: *const c_char) -> *mut RawHtsFile;
    fn hts_close(fp: *mut RawHtsFile) -> c_int;
    fn sam_hdr_read(fp: *mut RawHtsFile) -> *mut RawSamHdr;
    fn sam_hdr_destroy(h: *mut RawSamHdr);
    fn bam_init1() -> *mut RawBam1;
    fn bam_destroy1(b: *mut RawBam1);
    fn sam_read1(fp: *mut RawHtsFile, h: *mut RawSamHdr, b: *mut RawBam1) -> c_int;
    fn sam_write1(fp: *mut RawHtsFile, h: *const RawSamHdr, b: *const RawBam1) -> c_int;
    fn hts_set_threads(fp: *mut RawHtsFile, n: c_int) -> c_int;
    fn qbix_hts_shim_bam_qname(b: *mut RawBam1) -> *const c_char;
    fn qbix_hts_shim_sam_hdr_text(h: *const RawSamHdr) -> *const c_char;
    fn qbix_hts_shim_sam_hdr_text_len(h: *const RawSamHdr) -> usize;
    fn qbix_hts_shim_bgzf_tell(fp: *mut RawHtsFile) -> i64;
    fn qbix_hts_shim_bgzf_seek(fp: *mut RawHtsFile, offset: i64) -> i64;
    fn qbix_hts_shim_bgzf_set_cache_size(fp: *mut RawHtsFile, size: c_int) -> c_int;
}

pub(crate) struct HtsFile(*mut RawHtsFile);
pub(crate) struct Header(*mut RawSamHdr);
pub(crate) struct BamRecord(*mut RawBam1);

impl HtsFile {
    pub(crate) fn open(path: &str, mode: &str) -> std::result::Result<Self, ()> {
        let path = CString::new(path).map_err(|_| ())?;
        let mode = CString::new(mode).map_err(|_| ())?;
        let fp = unsafe { hts_open(path.as_ptr(), mode.as_ptr()) };
        if fp.is_null() {
            Err(())
        } else {
            Ok(Self(fp))
        }
    }

    pub(crate) fn read_header(&self) -> std::result::Result<Header, ()> {
        let header = unsafe { sam_hdr_read(self.0) };
        if header.is_null() {
            Err(())
        } else {
            Ok(Header(header))
        }
    }

    pub(crate) fn set_threads(&self, threads: usize) -> Result<()> {
        if threads <= 1 {
            return Ok(());
        }
        let threads = c_int::try_from(threads)
            .map_err(|_| "[qbix] thread count is too large for htslib".to_string())?;
        if unsafe { hts_set_threads(self.0, threads) } < 0 {
            return Err("[qbix] hts_set_threads failed".to_string());
        }
        Ok(())
    }

    pub(crate) fn set_bgzf_cache_size(&self, size: usize) -> Result<()> {
        let size = c_int::try_from(size)
            .map_err(|_| "[qbix] BGZF cache size is too large for htslib".to_string())?;
        if unsafe { qbix_hts_shim_bgzf_set_cache_size(self.0, size) } != 0 {
            return Err("[qbix] bgzf_set_cache_size failed".to_string());
        }
        Ok(())
    }

    pub(crate) fn read_next(&self, header: &Header, record: &BamRecord) -> i32 {
        unsafe { sam_read1(self.0, header.0, record.0) }
    }

    pub(crate) fn write_record(&self, header: &Header, record: &BamRecord) -> Result<()> {
        if unsafe { sam_write1(self.0, header.0, record.0) } < 0 {
            return Err("[qbix] sam_write1 failed".to_string());
        }
        Ok(())
    }

    pub(crate) fn read_record_at(
        &self,
        header: &Header,
        record: &BamRecord,
        offset: i64,
    ) -> Result<()> {
        if offset < 0 {
            return Err("[qbix] invalid negative BGZF offset".to_string());
        }
        if unsafe { qbix_hts_shim_bgzf_seek(self.0, offset) } != 0 {
            return Err("[qbix] bgzf_seek failed".to_string());
        }
        if self.read_next(header, record) < 0 {
            return Err("[qbix] sam_read1 failed".to_string());
        }
        Ok(())
    }

    pub(crate) fn tell(&self) -> Result<i64> {
        let offset = unsafe { qbix_hts_shim_bgzf_tell(self.0) };
        if offset < 0 {
            Err("[qbix] bgzf_tell failed".to_string())
        } else {
            Ok(offset)
        }
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

impl Drop for Header {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                sam_hdr_destroy(self.0);
            }
        }
    }
}

impl Header {
    pub(crate) fn text_hash(&self) -> Result<u64> {
        if self.0.is_null() {
            return Err("[qbix] invalid BAM header".to_string());
        }
        let text = unsafe { qbix_hts_shim_sam_hdr_text(self.0) };
        let len = unsafe { qbix_hts_shim_sam_hdr_text_len(self.0) };
        if text.is_null() && len > 0 {
            return Err("[qbix] invalid BAM header text".to_string());
        }
        let bytes = if len == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(text as *const u8, len) }
        };
        Ok(fnv1a64(bytes))
    }
}

impl BamRecord {
    pub(crate) fn new() -> Result<Self> {
        let record = unsafe { bam_init1() };
        if record.is_null() {
            Err("[qbix] could not allocate BAM record".to_string())
        } else {
            Ok(Self(record))
        }
    }

    pub(crate) fn qname(&self) -> Result<&str> {
        if self.0.is_null() {
            return Err("[qbix] invalid BAM record".to_string());
        }
        let qname = unsafe { qbix_hts_shim_bam_qname(self.0) };
        if qname.is_null() {
            return Err("[qbix] invalid BAM record".to_string());
        }
        let cstr = unsafe { CStr::from_ptr(qname) };
        cstr.to_str()
            .map_err(|_| "[qbix] BAM read name is not valid UTF-8".to_string())
    }
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
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
