# Bucketed QBI Index Builder

This document describes the bucketed build path used by `qbix index`.
It is an implementation detail of index construction shared by QBI file-format
v1 (`QBI1`) and experimental v2 (`QBI2`).

## Goal

The original build path accumulated every index row in one `Vec<Record>`, then
sorted and wrote the full vector at the end.  For very large BAM files this
costs roughly `record_count * 16` bytes of memory.  For example, 600 million
records require about 10 GB just for the record table.

The bucketed builder keeps each selected file format unchanged while reducing
peak record memory to approximately one bucket at a time during final sorting.

## Format Compatibility

The builder writes one of two on-disk versions:

- v1 (`QBI1`): the stable default with a 48-byte header and 16-byte records,
  documented in [qbi1-format.md](qbi1-format.md)
- v2 (`QBI2`): the experimental grouped radix layout documented in
  [qbi2-format.md](qbi2-format.md)

Both preserve the logical ordering by `(qhash, file_offset)`.

Existing load and query operations continue to work without format-specific
changes:

- `load`
- `get`
- `show`
- `check`
- `stats`

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
  initialize the selected QBI1 or QBI2 sorted-record sink

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

- QBI1 binary search over full `(qhash, virtual_offset)` rows
- QBI2 streaming generation of radix, suffix, group, rank, and offset sections
- sequential logical-record and hash-group iteration used by `show`, `check`,
  and `stats`

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
qbix index --index-format qbi2 --qbi2-radix-bits 16 reads.bam
```

For QBI2, omitting `--qbi2-radix-bits` (or specifying `auto`) selects P=8 when
the BAM has at most 522,240 records and P=16 otherwise. Explicit `8` and `16`
values override this conservative automatic choice.

`--memory` accepts integer values with optional `K`, `M`, or `G` suffixes.

`--sort-threads` is independent of `--bgzf-threads` / `-@`.  It only
parallelizes bucket sorting during final index assembly.

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
    pub index_format: Option<IndexFormat>,
    pub qbi2_radix_bits: Option<u8>,
}
```

`None` means the CLI-equivalent default:

- `memory_limit`: 512 MiB
- `bucket_bits`: 8
- `sort_threads`: 1
- `temp_dir`: output index directory
- `index_format`: QBI1
- `qbi2_radix_bits`: automatic (P=8 for at most 522,240 records, otherwise P=16)

`BuildOptions`, `LookupOptions`, and `CheckOptions` are `#[non_exhaustive]`.
External users should start from `Default` and then assign fields:

```rust
let mut options = qbix::BuildOptions::default();
options.bucket_bits = Some(12);
options.memory_limit = Some(1024 * 1024 * 1024);
options.sort_threads = Some(4);
options.index_format = Some(qbix::IndexFormat::Qbi2);
```

## C API

The existing C ABI is unchanged:

```c
qbix_build_index(bam_path, index_path, threads)
```

It uses the default bucketed build settings.  A future extended C API can add
explicit build options without breaking the existing function.
