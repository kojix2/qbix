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

The `.qbi` file format is documented in [docs/qbi-format.md](docs/qbi-format.md).

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

This writes:

```sh
reads.bam.qbi
```

Fetch records by QNAME. Output is SAM:

```sh
qbix get reads.bam read_a read_b
```

Fetch records from a newline-delimited read-name file:

```sh
qbix get reads.bam -f names.txt
```

Use `-f -` to read names from stdin:

```sh
cat names.txt | qbix get reads.bam -f -
```

Write matching records as BAM:

```sh
qbix get reads.bam -f names.txt -b -o hits.bam
qbix get reads.bam -f names.txt -Ob -o hits.bam
```

Use more htslib threads:

```sh
qbix index -@ 4 reads.bam
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

For multiple read names, `--bam-order` reads records in BAM file-offset order.
This can reduce random seeking:

```sh
qbix get --bam-order reads.bam read_a read_b
```

If name-sorted output is needed, sort downstream:

```sh
qbix get --bam-order reads.bam read_a read_b | samtools sort -N -O SAM -
```

## Other Commands

Check an index against its BAM:

```sh
qbix check reads.bam
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
let bam = qbix::IndexedBam::open("reads.bam", qbix::LookupOptions::default())?;
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

    qbix_index_t *idx = qbix_index_open("reads.bam", 0, 1);
    if (!idx) return 1;

    if (qbix_index_lookup(idx, "read_a", &hits, &n_hits) == 0) {
        qbix_hits_free(hits, n_hits);
    }

    qbix_index_close(idx);
    return 0;
}
```
