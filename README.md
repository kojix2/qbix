# qbix

[![Cargo Build & Test](https://github.com/kojix2/qbix/actions/workflows/ci.yml/badge.svg)](https://github.com/kojix2/qbix/actions/workflows/ci.yml)
[![Crates.io Version](https://img.shields.io/crates/v/qbix)](https://crates.io/crates/qbix)
[![Lines of Code](https://img.shields.io/endpoint?url=https%3A%2F%2Ftokei.kojix2.net%2Fbadge%2Fgithub%2Fkojix2%2Fqbix%2Flines)](https://tokei.kojix2.net/github/kojix2/qbix)
[![DOI](https://zenodo.org/badge/1268179852.svg)](https://doi.org/10.5281/zenodo.20686763)

`qbix` indexes and retrieves BAM records by QNAME (read name) using a `.qbi`
index.

Use it when you need records for one or more QNAMEs from a BAM file without
scanning the file.

The index stores:

- XXH3-64 hashes of QNAMEs
- BGZF virtual offsets
- BAM size, mtime, and header hash for stale-index detection

The `.qbi` file-format versions are documented in
[docs/qbi-format.md](docs/qbi-format.md): v1 uses magic `QBI1`, while the
experimental v2 uses magic `QBI2`.

`qbix` was inspired by [jts/bri](https://github.com/jts/bri). `.qbi` is not
compatible with `.bri`. `.qbi` stores QNAME hashes and BGZF virtual offsets
instead of names. Lookup candidates are checked against BAM `QNAME` before output.

## Install

Download a prebuilt binary from [GitHub Releases](https://github.com/kojix2/qbix/releases).

## Build From Source

Requirements:

- Rust and Cargo
- htslib
- `pkg-config` recommended

Install htslib and `pkg-config` with your system package manager:

```sh
# macOS
brew install htslib pkg-config

# Ubuntu/Debian
sudo apt-get install libhts-dev pkg-config
```

```sh
cargo build --release
```

The binary is:

```sh
target/release/qbix
```

If htslib is installed under a custom prefix:

```sh
HTSDIR=/path/to/htslib cargo build --release
```

For static htslib linking:

```sh
HTSLIB_STATIC=1 cargo build --release
```

Optional colored SAM output uses
[libbiosyntax](https://github.com/kojix2/libbiosyntax), which is GPL-3.0-only.
It is disabled by default. Enable it explicitly if that license is acceptable:

```sh
cargo build --release --features biosyntax
```

Prebuilt binaries from GitHub Releases are built without this feature and do
not include libbiosyntax.

With this feature enabled, the build downloads libbiosyntax v0.1.0 into Cargo's
build directory. To use an existing checkout instead:

```sh
LIBBIOSYNTAX_DIR=/path/to/libbiosyntax cargo build --release --features biosyntax
```

You can also build and install from crates.io:

```sh
cargo install qbix
```

The crates.io install builds from source and expects htslib headers and
libraries to be available on the system.

## Basic Use

Create an index:

```sh
qbix index reads.bam
```

QBI file-format v1 (`QBI1`) remains the default. The experimental grouped radix
format v2 (`QBI2`) can be built explicitly; `get`, `show`, `check`, and `stats`
detect either version:

```sh
qbix index --index-format qbi2 reads.bam
```

QBI v2 is intended for evaluation and is not yet the default or a frozen format.
Use `--qbi2-radix-bits 8` or `16` to compare its two experimental layouts;
when omitted, the builder selects P=8 for at most 522,240 records and P=16
otherwise. This keeps the search-oriented P=16 default for normal large data
while avoiding its 512 KiB directory on indexes that are guaranteed to be
smaller with P=8. An explicit value always takes precedence.

This writes:

```sh
reads.bam.qbi
```

Fetch records by QNAME. Output is SAM:

```sh
qbix get reads.bam read_a read_b
```

SAM output contains matching alignment records without the SAM header by
default. Use `--with-header` to include the source BAM header. BAM output
always includes it.

```sh
qbix get --with-header reads.bam read_a read_b
```

Fetch records from a newline-delimited read-name file:

```sh
qbix get reads.bam -f names.txt
```

Use `-f -` to read names from stdin:

```sh
cat names.txt | qbix get reads.bam -f -
```

Process each input read name only once while preserving the order of its first
occurrence:

```sh
qbix get --unique reads.bam read_a read_a read_b
qbix get --unique reads.bam -f names.txt
```

This only removes duplicate query names. If the BAM contains multiple records
for a read name, all of those records are still written.

Write query names that have no matching BAM record to a newline-delimited file:

```sh
qbix get --missing missing.txt reads.bam -f names.txt
```

Missing names are written in query order. Duplicate queries are reported once
per occurrence unless `--unique` is also used. The report file is empty when
all query names are found.

Write matching records as BAM:

```sh
qbix get reads.bam -f names.txt -b -o hits.bam
qbix get reads.bam -f names.txt -Ob -o hits.bam
```

When built with `--features biosyntax`, SAM output to a terminal is colored by
default. Pipes and files are left plain. Use `--color always` or
`--color never` to override:

```sh
qbix get --color always reads.bam read_a
qbix get --color never reads.bam read_a
```

Use more BGZF/htslib I/O threads:

```sh
qbix index --bgzf-threads 4 reads.bam
qbix get -@ 4 reads.bam read_a
```

Use an explicit index path:

```sh
qbix index -i reads.qbi reads.bam
qbix get -i reads.qbi reads.bam read_a
```

## Output Order

Default output is query order:

```sh
qbix get reads.bam read_a read_b
qbix get --query-order reads.bam read_a read_b
```

Query-order output streams lookups without collecting all hits in memory. When
read names come from `-f -`, each name is looked up as it is read from stdin;
qbix does not wait for EOF or collect all input names first. `--unique` retains
only the set of names already seen.

For multiple read names, `--bam-order` reads records in BAM file-offset order.
This can reduce random seeking:

```sh
qbix get --bam-order reads.bam read_a read_b
```

`--bam-order` must collect the input names and sort matching hits before output.

If name-sorted output is needed, sort downstream:

```sh
qbix get --bam-order reads.bam read_a read_b | samtools sort -N -O SAM -
```

## Other Commands

Quickly check that an index matches its BAM size, mtime, and header hash:

```sh
qbix check reads.bam
qbix check --quick reads.bam
```

`check` defaults to `--quick`. Use `--full` to validate complete QBI2 directory
and suffix ordering, seek to every indexed record, and verify its read-name hash:

```sh
qbix check --full reads.bam
```

Print records-per-hash statistics and QBI1/QBI2 size estimates from the index.
`stat` is accepted as an alias for `stats`:

```sh
qbix stats reads.bam
qbix stat reads.bam
qbix stats -i reads.qbi reads.bam
```

Use JSON output for scripts:

```sh
qbix stats --json reads.bam
```

Show raw index rows:

```sh
qbix show reads.bam.qbi
```

`show` prints:

```text
qhash<TAB>voff
```

Print the version:

```sh
qbix --version
```

## Notes

- `.qbi` files are tied to the BAM size, mtime, and header hash.
- Rebuild the index after replacing or rewriting the BAM.
- Read names are not stored in the index. Hash hits are verified against the BAM record QNAME before output.

## Rust Library

`qbix` also exposes a small Rust API:

```rust
let index_path = qbix::build_index("reads.bam", qbix::BuildOptions::default())?;
qbix::check_index("reads.bam", qbix::CheckOptions::default())?;
qbix::check_index(
    "reads.bam",
    qbix::CheckOptions {
        mode: qbix::CheckMode::Full,
        ..qbix::CheckOptions::default()
    },
)?;
let mut bam = qbix::IndexedBam::open("reads.bam", qbix::LookupOptions::default())?;
let hits = bam.lookup("read_a")?;
```

## C Library

`qbix` can be built as a C library from source:

```sh
cargo build --release
```

Libraries are written under `target/release`, for example `libqbix.so`,
`libqbix.a`, or `libqbix.dylib`. The header is `include/qbix.h`.

For HTSlib-based applications, build `qbix` against the same HTSlib installation
as the host application. C API errors are stored per calling thread.
`qbix_index_t` handles are not thread-safe.

Example:

```c
#include "qbix.h"

int main(void) {
    qbix_hit_t *hits = 0;
    size_t n_hits = 0;

    if (qbix_build_index("reads.bam", 0, 1) != 0) return 1;
    if (qbix_check_index("reads.bam", 0, 1, QBIX_CHECK_QUICK) != 0) return 1;

    qbix_index_t *idx = qbix_index_open("reads.bam", 0, 1);
    if (!idx) return 1;

    if (qbix_index_lookup(idx, "read_a", &hits, &n_hits) == 0) {
        qbix_hits_free(hits, n_hits);
    }

    qbix_index_close(idx);
    return 0;
}
```
