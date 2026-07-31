# QBI2 Lookup Microbenchmark

This is an early index-only measurement, not an end-to-end `qbix get`
benchmark. It compares QBI1, QBI2 P=8, and QBI2 P=16 using the same synthetic
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

The table reports medians from four runs on Linux 6.18, an AMD Ryzen 7 5700G,
and Rust 1.97.0. mmap pages were warm.

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

An initial P=16 run exposed one writer issue: the radix directory was emitted
as 65,537 individual writes. Buffering that section reduced final encoding from
about 58 ms to about 10-12 ms.

These results support the radix lookup design, but do not establish end-to-end
performance. Real Illumina, PacBio, and ONT BAM measurements are still needed,
especially for verified present queries where BAM seek and decompression are
expected to dominate.
