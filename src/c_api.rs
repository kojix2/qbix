#![allow(non_camel_case_types)]

use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::ptr;

use crate::api::{IndexedBam, LookupOptions};
use crate::commands;

const QBIX_OK: c_int = 0;
const QBIX_ERR: c_int = -1;

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::new("").expect("empty string is valid C"));
}

#[repr(C)]
pub struct qbix_index_t {
    indexed: IndexedBam,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct qbix_hit_t {
    pub virtual_offset: i64,
}

pub type qbix_check_mode_t = c_int;
pub const QBIX_CHECK_QUICK: qbix_check_mode_t = 0;
pub const QBIX_CHECK_FULL: qbix_check_mode_t = 1;

#[no_mangle]
pub extern "C" fn qbix_build_index(
    bam_path: *const c_char,
    index_path: *const c_char,
    threads: usize,
) -> c_int {
    c_status(|| {
        let bam_path = cstr_arg(bam_path, "bam_path")?;
        let index_path = optional_cstr_arg(index_path, "index_path")?;
        validate_threads(threads)?;
        commands::build_index(bam_path, index_path, false, threads)
    })
}

#[no_mangle]
pub extern "C" fn qbix_check_index(
    bam_path: *const c_char,
    index_path: *const c_char,
    threads: usize,
    mode: qbix_check_mode_t,
) -> c_int {
    c_status(|| {
        let bam_path = cstr_arg(bam_path, "bam_path")?;
        let index_path = optional_cstr_arg(index_path, "index_path")?;
        validate_threads(threads)?;
        let mode = check_mode(mode)?;
        commands::check_index(bam_path, index_path, threads, false, mode)
    })
}

#[no_mangle]
pub extern "C" fn qbix_index_open(
    bam_path: *const c_char,
    index_path: *const c_char,
    threads: usize,
) -> *mut qbix_index_t {
    c_ptr(
        || open_index(bam_path, index_path, threads),
        ptr::null_mut(),
        |index| Box::into_raw(Box::new(index)),
    )
}

#[no_mangle]
/// # Safety
///
/// `index` must be a valid handle returned by `qbix_index_open`. `read_name`
/// must point to a valid NUL-terminated UTF-8 string. `hits_out` and
/// `hit_count_out` must be valid writable pointers. On success, a non-null
/// `*hits_out` must be released with `qbix_hits_free` and the returned count.
pub unsafe extern "C" fn qbix_index_lookup(
    index: *mut qbix_index_t,
    read_name: *const c_char,
    hits_out: *mut *mut qbix_hit_t,
    hit_count_out: *mut usize,
) -> c_int {
    c_status(|| {
        if index.is_null() {
            return Err("[qbix] index handle is null".to_string());
        }
        if hits_out.is_null() {
            return Err("[qbix] hits_out is null".to_string());
        }
        if hit_count_out.is_null() {
            return Err("[qbix] hit_count_out is null".to_string());
        }
        let read_name = cstr_arg(read_name, "read_name")?;
        let index = &*index;
        let hits = index
            .indexed
            .lookup_offsets(read_name)
            .map_err(|err| err.to_string())?;
        let hits = hits
            .into_iter()
            .map(|offset| qbix_hit_t {
                virtual_offset: offset.as_i64(),
            })
            .collect::<Vec<_>>();

        let hit_count = hits.len();
        if hit_count == 0 {
            *hits_out = ptr::null_mut();
            *hit_count_out = 0;
            return Ok(());
        }

        let boxed = hits.into_boxed_slice();
        *hits_out = Box::into_raw(boxed) as *mut qbix_hit_t;
        *hit_count_out = hit_count;
        Ok(())
    })
}

#[no_mangle]
/// # Safety
///
/// `hits` must be either null or a pointer returned by `qbix_index_lookup` with
/// the same `hit_count`. Each non-null allocation must be freed at most once.
pub unsafe extern "C" fn qbix_hits_free(hits: *mut qbix_hit_t, hit_count: usize) {
    c_void(|| {
        if hits.is_null() {
            return;
        }
        let slice = ptr::slice_from_raw_parts_mut(hits, hit_count);
        drop(Box::from_raw(slice));
    });
}

#[no_mangle]
/// # Safety
///
/// `index` must be either null or a handle returned by `qbix_index_open`. Each
/// non-null handle must be closed at most once.
pub unsafe extern "C" fn qbix_index_close(index: *mut qbix_index_t) {
    c_void(|| {
        if !index.is_null() {
            drop(Box::from_raw(index));
        }
    });
}

#[no_mangle]
pub extern "C" fn qbix_last_error() -> *const c_char {
    LAST_ERROR.with(|last_error| last_error.borrow().as_ptr())
}

#[no_mangle]
pub extern "C" fn qbix_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *const c_char
}

fn open_index(
    bam_path: *const c_char,
    index_path: *const c_char,
    threads: usize,
) -> Result<qbix_index_t, String> {
    let bam_path = cstr_arg(bam_path, "bam_path")?;
    let index_path = optional_cstr_arg(index_path, "index_path")?;
    validate_threads(threads)?;
    let indexed = IndexedBam::open(
        bam_path,
        LookupOptions {
            index_path: index_path.map(PathBuf::from),
            threads,
        },
    )
    .map_err(|err| err.to_string())?;

    Ok(qbix_index_t { indexed })
}

fn c_status<F>(f: F) -> c_int
where
    F: FnOnce() -> Result<(), String>,
{
    match c_result(f) {
        Ok(()) => {
            clear_last_error();
            QBIX_OK
        }
        Err(err) => {
            set_last_error(err);
            QBIX_ERR
        }
    }
}

fn c_ptr<F, T, R, M>(f: F, null_value: R, map_ok: M) -> R
where
    F: FnOnce() -> Result<T, String>,
    M: FnOnce(T) -> R,
{
    match c_result(f) {
        Ok(index) => {
            clear_last_error();
            map_ok(index)
        }
        Err(err) => {
            set_last_error(err);
            null_value
        }
    }
}

fn c_void<F>(f: F)
where
    F: FnOnce(),
{
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(()) => clear_last_error(),
        Err(payload) => set_last_error(panic_message(payload)),
    }
}

fn c_result<F, T>(f: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String>,
{
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(payload) => Err(panic_message(payload)),
    }
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        format!("[qbix] panic across C API boundary: {message}")
    } else if let Some(message) = payload.downcast_ref::<String>() {
        format!("[qbix] panic across C API boundary: {message}")
    } else {
        "[qbix] panic across C API boundary".to_string()
    }
}

fn cstr_arg<'a>(ptr: *const c_char, name: &str) -> Result<&'a str, String> {
    if ptr.is_null() {
        return Err(format!("[qbix] {name} is null"));
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map_err(|_| format!("[qbix] {name} is not valid UTF-8"))
}

fn optional_cstr_arg<'a>(ptr: *const c_char, name: &str) -> Result<Option<&'a str>, String> {
    if ptr.is_null() {
        Ok(None)
    } else {
        cstr_arg(ptr, name).map(Some)
    }
}

fn validate_threads(threads: usize) -> Result<(), String> {
    if threads == 0 {
        return Err("[qbix] threads must be a positive integer".to_string());
    }
    Ok(())
}

fn check_mode(mode: qbix_check_mode_t) -> Result<commands::CheckMode, String> {
    match mode {
        QBIX_CHECK_QUICK => Ok(commands::CheckMode::Quick),
        QBIX_CHECK_FULL => Ok(commands::CheckMode::Full),
        _ => Err(format!("[qbix] unsupported check mode: {mode}")),
    }
}

fn clear_last_error() {
    set_last_error(String::new());
}

fn set_last_error(err: String) {
    let err = err.replace('\0', " ");
    LAST_ERROR.with(|last_error| {
        *last_error.borrow_mut() = CString::new(err).expect("interior NULs were replaced");
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c_status_converts_panic_to_error() {
        let status = c_status(|| panic!("boom"));

        assert_eq!(status, QBIX_ERR);
        let error = unsafe { CStr::from_ptr(qbix_last_error()) }
            .to_string_lossy()
            .into_owned();
        assert!(error.contains("panic across C API boundary: boom"));
    }
}
