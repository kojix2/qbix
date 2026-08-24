# QBI File Formats

This document is the entry point for the on-disk `.qbi` index formats written
by `qbix`. The file-format version is identified by the first four magic bytes;
it is independent of the `qbix` software version.

| File-format version | Magic | Status | Writer selection | Specification |
| --- | --- | --- | --- | --- |
| v1 | `QBI1` | Stable | `--index-format qbi1` | [QBI v1 specification](qbi1-format.md) |
| v2 | `QBI2` | Stable and default | `--index-format qbi2` | [QBI v2 specification](qbi2-format.md) |

Current readers detect v1 or v2 from the magic and accept both. Writers use
v2 unless v1 is requested explicitly. A future `qbix` release may add another
file-format version without changing the meaning of existing versions.

All current QBI versions share these properties:

- the index filename normally appends `.qbi` to the BAM path;
- integers are stored in little-endian byte order;
- QNAMEs are represented by XXH3-64 hashes rather than stored directly;
- hash matches are candidates and must be verified against the BAM QNAME;
- BAM size, modification time, and header hash detect stale indexes;
- `qbix show` presents logical `(qhash, virtual_offset)` rows regardless of the
  physical layout.

Version-specific headers, sections, invariants, and size calculations belong
in the corresponding specification document. New formats should use a new
magic such as `QBI3` and receive their own `qbi3-format.md`; existing format
definitions remain unchanged.
