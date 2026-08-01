# QBI2 Lookup Microbenchmark

This is an early index-only measurement, not an end-to-end `qbix get`
benchmark. It compares QBI1 and the QBI2 radix widths using the same synthetic
sorted rows:

- `N = 1,000,000` records
- `K = 500,000` hashes
- two offsets per hash
- 100,000 pseudo-random present queries and 100,000 absent queries
- seven warm lookup rounds
- three complete logical-record and hash-group scan rounds
- candidate offset decoding included
- QNAME hashing, BAM seeks, and BGZF decompression excluded

The command is:

```sh
cargo test --release benchmark_qbi_lookup_smoke -- --ignored --nocapture --test-threads=1
```

The first table records the v0.0.9 flags-zero layout. It reports medians from
four runs on Linux 6.18, an AMD Ryzen 7 5700G, and Rust 1.97.0. mmap pages were
warm.

| Format | Bytes | Final encode | Open | Present | Absent | Record scan | Group scan |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| QBI1 | 16,000,048 | 9.76 ms | 11.91 us | 1,090 ns/query | 999 ns/query | 4.97 ms | 5.45 ms |
| QBI2 P=8 | 11,642,832 | 11.76 ms | 18.77 us | 297 ns/query | 157 ns/query | 10.46 ms | 7.55 ms |
| QBI2 P=16 | 11,665,072 | 11.85 ms | 24.10 us | 184 ns/query | 50 ns/query | 10.95 ms | 7.80 ms |

For this `N/K = 2` case, both QBI2 variants were about 27% smaller. P=8 was
about 3.7 times faster for present queries and 6.4 times faster for absent
queries. P=16 was about 5.9 times and 20 times faster, respectively.

The QBI2 open cost was 7-12 us higher, but remained constant-work after removal
of eager full-section validation. Final encoding was about 20% slower in this
isolated writer measurement. BAM scanning and
bucket sorting are shared by both formats, so this does not imply the same
percentage increase for complete index construction.

QBI2 group boundaries are traversed with a forward bit-vector cursor for
logical record and group scans. Before that cursor was introduced, one run of
the same benchmark took 41-48 ms for a QBI2 record scan and 38-45 ms for a
group scan because every group performed independent rank-directory searches.
The cursor reduced those scans to about 10-11 ms and 8 ms. QBI1 remains faster
for a complete record scan because its logical rows are already flat on disk.

Present lookup also locates adjacent group boundaries together when they fall
in the same rank block. This avoids the second rank-directory search in the
common case while retaining a bounded-search fallback for very large groups.

## Parallel Record Radix Experiment

Adding a second radix directory containing record boundaries lets `select1`
search only the matching prefix's rank blocks. Replacing the original radix
directory with record boundaries was rejected: it kept the file size unchanged
but made absent lookup 17-54% slower and full scans more than twice as slow.
Keeping both directories preserved the fast hash search and improved present
lookup. This intermediate prototype paired the record radix with the existing
rank directory. The reader remained compatible with v0.0.9 flags-zero files.

A representative completed-layout run on the same host was:

| Format | Bytes | Final encode | Open | Present | Absent | Record scan | Group scan |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| QBI1 | 16,000,048 | 11.97 ms | 12.15 us | 1,402 ns/query | 1,396 ns/query | 5.20 ms | 5.79 ms |
| QBI2 P=8 | 11,644,888 | 15.44 ms | 18.39 us | 277 ns/query | 158 ns/query | 10.57 ms | 7.46 ms |
| QBI2 P=12 | 11,706,328 | 13.35 ms | 20.62 us | 198 ns/query | 104 ns/query | 10.57 ms | 7.48 ms |
| QBI2 P=16 | 12,189,368 | 12.03 ms | 25.63 us | 152 ns/query | 51 ns/query | 11.10 ms | 7.85 ms |

For this workload P=12 costs only 61,440 bytes over P=8 while reducing present
lookup by about 29% and absent lookup by about 35%. P=16 remains fastest but
costs another 483,040 bytes over P=12. This supports exposing P=12 explicitly;
the automatic policy remains conservative pending real BAM measurements.

## Select Directory Replacement

The next experiment replaced the current-layout rank directory with direct
samples of every 512th group start. The offset radix or nearest select sample
provides the starting record position, after which lookup scans the bounded
group-start words. The reserved directory size remains approximately `N/64`
bytes, so index sizes are unchanged. Legacy flags-zero files continue using
their rank directory.

Four-run medians were:

| Format | Bytes | Present | Absent | Record scan | Group scan |
| --- | ---: | ---: | ---: | ---: | ---: |
| QBI1 | 16,000,048 | 1,338 ns/query | 1,352 ns/query | 4.81 ms | 5.96 ms |
| QBI2 P=8 | 11,644,888 | 297 ns/query | 164 ns/query | 10.62 ms | 7.51 ms |
| QBI2 P=12 | 11,706,328 | 205 ns/query | 105 ns/query | 10.64 ms | 7.54 ms |
| QBI2 P=16 | 12,189,368 | 148 ns/query | 55 ns/query | 11.24 ms | 7.89 ms |

Compared with the rank plus record-radix run, direct select was roughly even at
P=16 and several percent slower at P=8/P=12. Sampling every 256 groups slightly
improved P=8 but added about 15.6 KiB at this N; every 128 groups reached the
old P=8 speed but added about 46.9 KiB and did not help P=12/P=16. The
512-group interval was the clearest size-neutral candidate, but still did not
justify replacing rank.

Neither record-radix variant was retained. The final implementation returned
to the original six-section layout with one hash radix and the 512-bit rank
directory, while keeping P=12 as an independent radix-width option. Four-run
medians after that reversion were:

| Format | Bytes | Present | Absent | Record scan | Group scan |
| --- | ---: | ---: | ---: | ---: | ---: |
| QBI1 | 16,000,048 | 1,109 ns/query | 1,181 ns/query | 4.79 ms | 5.48 ms |
| QBI2 P=8 | 11,642,832 | 311 ns/query | 161 ns/query | 10.48 ms | 6.99 ms |
| QBI2 P=12 | 11,673,552 | 245 ns/query | 104 ns/query | 10.19 ms | 7.07 ms |
| QBI2 P=16 | 11,665,072 | 197 ns/query | 54 ns/query | 10.68 ms | 7.43 ms |

In the retained layout P=12 costs 30,720 bytes over P=8 and improves present
lookup by about 21% and absent lookup by about 35%. It is therefore retained
without adding another file-format structure.

An initial P=16 run exposed one writer issue: the radix directory was emitted
as 65,537 individual writes. Buffering that section reduced final encoding from
about 58 ms to about 10-12 ms.

These results support the radix lookup design, but do not establish end-to-end
performance. Real Illumina, PacBio, and ONT BAM measurements are still needed,
especially for verified present queries where BAM seek and decompression are
expected to dominate.
