# Bucketed QBI Index Builder

This document describes the bucketed build path used by `qbix index`.
It is an implementation detail of index construction and does not change the
on-disk `.qbi` format.

## Goal

The original build path accumulated every index row in one `Vec<Record>`, then
sorted and wrote the full vector at the end.  For very large BAM files this
costs roughly `record_count * 16` bytes of memory.  For example, 600 million
records require about 10 GB just for the record table.

The bucketed builder keeps the `QBI1` format unchanged while reducing peak
record memory to approximately one bucket at a time during final sorting.

## Format Compatibility

The `.qbi` format remains:

- magic: `QBI1`
- header size: 48 bytes
- record size: 16 bytes
- records sorted by `(qhash, file_offset)`

Existing load and query operations continue to work without format-specific
changes:

- `load`
- `get`
- `show`
- `check`
- `stats`

The in-memory `Index::save()` path is kept as a small-data/reference path and
uses the same record ordering as the bucketed builder.

## Defaults

```text
--memory      512M
--bucket-bits 8
--sort-threads 1
--temp-dir    unset
```

`--bucket-bits` is constrained to `1..=12`.

`--memory` is the maximum size of a single bucket loaded during final sorting.
It is not a strict global memory cap.  Actual peak memory is approximately:

```text
one bucket Vec (<= --memory)
+ allocated staging buffers
+ htslib buffers
```

Staging buffers are allocated lazily per bucket.  With `--bucket-bits 12`, the
maximum staging-buffer footprint is `4096 * 64 KiB = 256 MiB`.

`--sort-threads` controls how many buckets may be loaded and sorted in
parallel during the final phase.  Because each worker may load one full bucket,
peak record memory can rise to approximately:

```text
--sort-threads * --memory
+ allocated staging buffers
+ htslib buffers
```

## Algorithm

```text
scan BAM once
  readname    = rec.qname()?
  qhash       = qname_hash64(readname)
  bucket      = qhash >> (64 - bucket_bits)
  append (qhash, file_offset) to that bucket's staging buffer
  total_records += 1
  bucket.records += 1
  bucket.bytes += 16

  if bucket.bytes > memory_limit:
      fail fast

finish
  flush all bucket staging buffers
  create final tmp next to output index
  write QBI1 header with total record count

  for chunks of up to sort_threads buckets in ascending prefix order:
    read and sort buckets in the chunk in parallel
    append sorted records to final tmp
    best-effort remove consumed bucket temp file

  flush and close final tmp
  rename final tmp to output index
  best-effort remove bucket work directory
```

## Bucket Temp Record Layout

Bucket temporary files store fixed-size little-endian rows:

```text
u64 qhash
i64 file_offset
```

Each row is 16 bytes.

## Correctness

Bucket assignment uses the high bits of `qhash`:

```text
bucket = qhash >> (64 - bucket_bits)
```

Processing buckets in ascending bucket order and sorting each bucket by
`(qhash, file_offset)` produces exactly the same global order as sorting all
records together by `(qhash, file_offset)`.  When `--sort-threads` is greater
than 1, buckets are still written to the final index in prefix order.

This preserves the invariant required by:

- `Index::range_indices()`, which binary-searches by `qhash`
- `stats`, which scans run lengths of equal `qhash`

`file_offset` is a BGZF virtual offset and is expected to be unique per BAM
record, so `(qhash, file_offset)` is a total ordering.  `sort_unstable_by` is
therefore deterministic for these records.

## File Descriptor Strategy

The builder does not keep one file descriptor open per bucket.  Each bucket has
a lazy staging buffer.  When a buffer fills:

```text
open bucket file with append
write buffer
close file
```

This keeps the number of simultaneously open file descriptors low, including on
systems with conservative limits such as macOS defaults.

## Temporary Files

The final temporary index is always created in the same directory as the output
index.  This keeps the final `rename()` atomic on normal local filesystems.

Bucket temporary files are created under a unique work directory:

- under `--temp-dir` when provided
- otherwise under the output index directory

The work directory is created with an exclusive `create_dir` retry loop to avoid
collisions with stale files or concurrent builds.

Bucket temporary files may live on a different filesystem from the final index
because they are only read back and rewritten into the final temporary index.

Temporary disk usage is roughly:

```text
bucket records: record_count * 16 bytes
final tmp:      record_count * 16 bytes + header
```

Peak temporary usage while the final file is being assembled is therefore close
to two index-sized files.

## Cleanup

`TempGuard` tracks:

- final temporary index file
- bucket work directory

On error or panic, the guard removes these paths on a best-effort basis.

During successful finish:

1. Consumed bucket files are removed best-effort.
2. The final temporary index is flushed and closed.
3. The final temporary index is renamed into place.
4. The bucket work directory is removed best-effort.
5. The guard is disarmed.

Cleanup failures after all data has been written do not turn a successful build
into a failure.  This avoids wasting large completed builds because of
housekeeping issues such as transient filesystem errors or lingering handles.

## Oversized Buckets

Version 1 fails fast when any bucket exceeds `--memory`:

```text
[qbix] bucket is too large; retry with larger --memory or higher --bucket-bits
```

Future fallback, if needed, should be external merge sort rather than recursive
bucket splitting.

## CLI

```sh
qbix index --memory 512M --bucket-bits 8 --temp-dir DIR reads.bam
```

`--memory` accepts integer values with optional `K`, `M`, or `G` suffixes.

`--sort-threads` is independent of htslib `--threads`.  It only parallelizes
bucket sorting during final index assembly.

## Rust API

`BuildOptions` includes:

```rust
pub struct BuildOptions {
    pub index_path: Option<PathBuf>,
    pub threads: usize,
    pub verbose: bool,
    pub memory_limit: Option<usize>,
    pub bucket_bits: Option<u8>,
    pub sort_threads: Option<usize>,
    pub temp_dir: Option<PathBuf>,
}
```

`None` means the CLI-equivalent default:

- `memory_limit`: 512 MiB
- `bucket_bits`: 8
- `sort_threads`: 1
- `temp_dir`: output index directory

`BuildOptions`, `LookupOptions`, and `CheckOptions` are `#[non_exhaustive]`.
External users should start from `Default` and then assign fields:

```rust
let mut options = qbix::BuildOptions::default();
options.bucket_bits = Some(12);
options.memory_limit = Some(1024 * 1024 * 1024);
options.sort_threads = Some(4);
```

## C API

The existing C ABI is unchanged:

```c
qbix_build_index(bam_path, index_path, threads)
```

It uses the default bucketed build settings.  A future extended C API can add
explicit build options without breaking the existing function.

## Tests

The test suite covers:

- byte-for-byte equivalence with the in-memory reference path
- byte-for-byte equivalence with parallel bucket sorting
- equivalence at `bucket_bits` bounds (`1` and `12`)
- oversized bucket fail-fast behavior
- cleanup after an oversized error with flushed bucket temp files
- existing CLI, Rust API, C API, and end-to-end workflows
