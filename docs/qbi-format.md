# QBI File Format

This document describes the on-disk `.qbi` index format written by `qbix`.
The current format is identified by the magic bytes `QBI1`.

`.qbi` indexes are lookup tables for BAM records by read name. They store a
64-bit hash of each BAM record `QNAME` and the BGZF virtual offset of that
record in the source BAM. Read names themselves are not stored in the index.

## File Layout

All integer fields are little-endian. The file consists of one fixed-size
header followed by a sorted table of fixed-size records:

| Byte range | Contents                     |
| ---------- | ---------------------------- |
| `0..48`    | `QBI1` header                |
| `48..end`  | `record_count` index records |

The expected file size is:

```text
48 + record_count * 16
```

Readers should reject files whose length does not match the header.

## Header

The `QBI1` header is 48 bytes:

| Offset | Size | Type  | Name                 | Description                                                       |
| ------ | ---- | ----- | -------------------- | ----------------------------------------------------------------- |
| 0      | 4    | bytes | magic                | ASCII `QBI1`.                                                     |
| 4      | 2    | u16   | header_size          | Must be `48`.                                                     |
| 6      | 2    | u16   | record_size          | Must be `16`.                                                     |
| 8      | 8    | u64   | read_name_byte_count | Must be `0` for current indexes. Nonzero values are incompatible. |
| 16     | 8    | u64   | record_count         | Number of index records following the header.                     |
| 24     | 8    | u64   | bam_size             | Size in bytes of the BAM file when the index was built.           |
| 32     | 8    | u64   | bam_mtime            | BAM modification time in nanoseconds since the Unix epoch.        |
| 40     | 8    | u64   | bam_header_hash      | FNV-1a 64-bit hash of the BAM header text.                        |

`read_name_byte_count` is present as a compatibility guard for older or
experimental layouts that carried a read-name table. Current `qbix` indexes do
not include such a table, and readers should ask users to rebuild the index
when this field is nonzero.

## Records

Each record is 16 bytes:

| Offset within record | Size | Type | Name           | Description                                                           |
| -------------------- | ---- | ---- | -------------- | --------------------------------------------------------------------- |
| 0                    | 8    | u64  | qhash          | XXH3-64 hash of the BAM record `QNAME` bytes.                         |
| 8                    | 8    | u64  | virtual_offset | BGZF virtual offset returned by htslib before reading the BAM record. |

`virtual_offset` must fit in htslib's signed 64-bit offset type. Negative
offsets are never written by `qbix`.

## Record Ordering

Records are sorted by:

1. `qhash` ascending.
2. `virtual_offset` ascending for records with the same `qhash`.

This ordering allows readers to binary-search the record table for all
candidates matching a read-name hash.

## Build Algorithm

The on-disk format is independent of the builder implementation. Current
`qbix index` builds large indexes by partitioning records into temporary bucket
files using the high bits of `qhash`, then sorting buckets and appending the
sorted records to the final `QBI1` file in bucket-prefix order.

Default build settings are:

- `--memory 512M`: maximum size of one bucket loaded during final sorting
- `--bucket-bits 8`: 256 temporary buckets
- `--sort-threads 1`: number of bucket sort worker threads
- `--temp-dir`: unset, so bucket temporary files are placed next to the output
  index

The final temporary index is always written in the same directory as the output
index before being renamed into place. Bucket temporary files are written under
a unique work directory created below `--temp-dir`, or below the output index
directory when `--temp-dir` is unset. During a build, temporary disk use is
roughly one additional index-sized file for bucket records, plus the final
temporary index while it is being assembled.

## Hashes

Read-name hashes use XXH3-64 over the exact BAM `QNAME` byte sequence as
exposed by htslib. The hash is stored as an unsigned 64-bit integer.

Because `.qbi` stores hashes rather than full read names, hash matches are
candidates. A reader that returns BAM records to users must seek to each
candidate `virtual_offset`, read the BAM record, and compare the record `QNAME`
with the requested read name. This verification prevents false results from
64-bit hash collisions.

The BAM header hash stored in the header is FNV-1a 64-bit over the BAM header
text bytes as returned by htslib.

## BAM Compatibility Checks

A `.qbi` index is tied to the BAM file used to build it. When opening an index
for lookup or validation, `qbix` compares the following header fields against
the current BAM:

- `bam_size`
- `bam_mtime`
- `bam_header_hash`

If any value differs, the index is considered stale and should be rebuilt.

## Default Filename

When no explicit index path is supplied, the index path is formed by appending
`.qbi` to the BAM path:

```text
reads.bam -> reads.bam.qbi
```

## Display Format

The `qbix show` command prints the raw record table as tab-separated decimal
values:

```text
qhash<TAB>virtual_offset
```

The command does not print header fields.

## Compatibility

Readers for the current format should require:

- magic bytes equal to `QBI1`
- `header_size == 48`
- `record_size == 16`
- `read_name_byte_count == 0`
- file size equal to `48 + record_count * 16`

Unknown magic bytes, unsupported header or record sizes, nonzero
`read_name_byte_count`, and file-size mismatches should be treated as
unsupported or corrupt indexes.
