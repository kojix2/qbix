# QBI File Format Version 2 (QBI2, Experimental)

QBI file-format version 2 is identified by the magic `QBI2`. It is an
experimental grouped radix index that stores each distinct XXH3-64 QNAME hash
once and keeps BAM virtual offsets in a separate array. QBI file-format version
1 (`QBI1`) remains the stable default; readers identify either version from its
magic bytes. See [QBI File Formats](qbi-format.md) for the version overview.
Early index-only measurements are recorded in
[qbi2-benchmark.md](qbi2-benchmark.md).

All integers are little-endian. The physical layout is:

```text
128-byte header
N u64 virtual offsets
W u64 group-start bit-vector words
L u64 rank entries
(2^P + 1) u64 radix entries
K packed hash suffixes
```

Here `N` is the BAM record count, `K` is the distinct hash count, `P` is 8 or
16, `W = ceil((N + 1) / 64)`, and `L = ceil(W / 8) + 1`. Suffix width is
`(64 - P) / 8`, giving seven bytes for P=8 and six bytes for P=16.

## Header

| Offset | Size | Type | Name | Required value or meaning |
| ---: | ---: | --- | --- | --- |
| 0 | 4 | bytes | magic | `QBI2` |
| 4 | 2 | u16 | header_size | `128` |
| 6 | 2 | u16 | flags | `0` |
| 8 | 1 | u8 | radix_bits | `8` or `16` |
| 9 | 1 | u8 | suffix_bytes | `7` or `6`, matching `radix_bits` |
| 10 | 1 | u8 | rank_block_words_log2 | `3` (8 words) |
| 11 | 1 | u8 | hash_algorithm | `1` (XXH3-64) |
| 12 | 4 | bytes | reserved | zero |
| 16 | 8 | u64 | record_count | `N` |
| 24 | 8 | u64 | unique_hash_count | `K` |
| 32 | 8 | u64 | bam_size | source BAM size |
| 40 | 8 | u64 | bam_mtime | nanoseconds since Unix epoch |
| 48 | 8 | u64 | bam_header_hash | FNV-1a 64-bit header hash |
| 56 | 8 | u64 | hash_seed | `0` |
| 64 | 8 | u64 | offsets_offset | offset-array start |
| 72 | 8 | u64 | group_bits_offset | bit-vector start |
| 80 | 8 | u64 | rank_offset | rank-directory start |
| 88 | 8 | u64 | radix_offset | radix-directory start |
| 96 | 8 | u64 | suffix_offset | suffix-array start |
| 104 | 8 | u64 | file_size | exact completed file size |
| 112 | 8 | u64 | radix_entry_count | `2^P + 1` |
| 120 | 8 | u64 | rank_entry_count | `L` |

Section offsets must equal the layout derived from the counts; arbitrary gaps,
overlaps, truncated sections, and trailing bytes are invalid. Arithmetic must
be checked for overflow. Unknown flags, algorithms, parameters, and nonzero
reserved data are unsupported.

Normal index opening performs constant-work structural checks: header and file
layout, radix/rank endpoints, the first and sentinel group bits, and final-word
padding. It does not scan the full bit vector, rank directory, radix directory,
or suffix array. `qbix check --full` performs those complete monotonicity,
population-count, and suffix-order checks before validating BAM records.

## Radix And Suffixes

For a hash `h`, `prefix = h >> (64 - P)`. `radix[p]` and `radix[p + 1]`
delimit the unique suffixes for that prefix. The directory starts at zero, is
monotonic, and ends at `K`. Packed suffixes are little-endian low bytes of the
hash and are strictly increasing within each radix range.

Lookup binary-searches only `suffixes[radix[p]..radix[p + 1]]`. A found suffix
has unique-hash index `j`.

## Groups And Select

Bit position `i` is one when offset `i` starts a hash group. Position `N` is an
additional one-bit sentinel. Bits are stored least-significant bit first in
each u64 word. Unused high bits in the final word are zero, and the total number
of one bits is `K + 1`.

Each rank entry stores the cumulative one count before an eight-word (512-bit)
block. A final entry stores `K + 1`. `select1(j)` locates the zero-based j-th
one by finding its rank block and scanning at most eight words. The candidate
offset range for unique hash `j` is:

```text
offsets[select1(j)..select1(j + 1)]
```

For an empty index, `N = K = 0`; offsets and suffixes are empty, while the bit
vector contains one word with bit zero set.

## Ordering And Verification

Offsets are ordered first by full hash and then by virtual offset, exactly like
logical QBI1 rows. `qbix show` reconstructs and prints those logical rows; it
does not expose physical sections.

QBI2 stores hashes rather than QNAMEs. Every candidate returned to a user is
therefore read from the BAM and compared with the exact requested QNAME. Hash
collisions can add candidates but cannot add false verified results. BAM size,
mtime, and header hash checks are identical to QBI1.

QBI2 readers and writers in this version accept P=8 and P=16. P=16 has 522,240
more directory bytes but saves one byte per distinct hash, so the exact size
crossover is `K = 522,240`. At build start `K` is not yet known. The automatic
builder therefore selects P=8 when `N <= 522,240`, where P=8 is guaranteed to
be smaller because `K <= N`, and uses the search-oriented P=16 otherwise.
`--qbi2-radix-bits auto|8|16` can retain or override that choice. `qbix stats`
reports the smallest layout using the completed index's exact K. QBI1 reading
and writing remain supported indefinitely.
