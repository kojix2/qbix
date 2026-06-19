use crate::error::Result;
use std::os::raw::c_char;

const BIOSYN_FORMAT_SAM: u32 = 3;
const INITIAL_SPAN_CAP: usize = 128;

#[repr(C)]
#[derive(Clone, Copy)]
struct BiosynSpan {
    start: u64,
    length: u64,
    class_id: u32,
    reserved: u32,
}

extern "C" {
    fn biosyn_highlight_line(
        format: u32,
        line: *const c_char,
        len: u64,
        zero_based_line_no: u64,
        out: *mut BiosynSpan,
        out_cap: u64,
    ) -> u64;
    fn biosyn_render_ansi_line(
        line: *const c_char,
        len: u64,
        spans: *const BiosynSpan,
        span_count: u64,
        out: *mut c_char,
        out_cap: u64,
    ) -> u64;
}

pub(crate) fn render_sam_ansi(line: &[u8], zero_based_line_no: u64) -> Result<Vec<u8>> {
    let len = u64::try_from(line.len())
        .map_err(|_| "[qbix] SAM line is too long for libbiosyntax".to_string())?;
    let spans = highlight_sam(line, len, zero_based_line_no)?;
    render_ansi(line, len, &spans)
}

fn highlight_sam(line: &[u8], len: u64, zero_based_line_no: u64) -> Result<Vec<BiosynSpan>> {
    let mut spans = vec![empty_span(); INITIAL_SPAN_CAP];
    loop {
        let count = unsafe {
            biosyn_highlight_line(
                BIOSYN_FORMAT_SAM,
                line.as_ptr().cast(),
                len,
                zero_based_line_no,
                spans.as_mut_ptr(),
                spans.len() as u64,
            )
        };
        let count = usize::try_from(count)
            .map_err(|_| "[qbix] libbiosyntax span count is too large".to_string())?;
        if count <= spans.len() {
            spans.truncate(count);
            return Ok(spans);
        }
        spans.resize(count, empty_span());
    }
}

fn render_ansi(line: &[u8], len: u64, spans: &[BiosynSpan]) -> Result<Vec<u8>> {
    let span_count = u64::try_from(spans.len())
        .map_err(|_| "[qbix] libbiosyntax span count is too large".to_string())?;
    let mut out = vec![0u8; line.len().saturating_mul(8).saturating_add(64)];
    loop {
        let required = unsafe {
            biosyn_render_ansi_line(
                line.as_ptr().cast(),
                len,
                spans.as_ptr(),
                span_count,
                out.as_mut_ptr().cast(),
                out.len() as u64,
            )
        };
        let required = usize::try_from(required)
            .map_err(|_| "[qbix] libbiosyntax output is too large".to_string())?;
        if required <= out.len() {
            out.truncate(required);
            return Ok(out);
        }
        out.resize(required, 0);
    }
}

fn empty_span() -> BiosynSpan {
    BiosynSpan {
        start: 0,
        length: 0,
        class_id: 0,
        reserved: 0,
    }
}
