---
title: 'qbix: An index for searching coordinate-sorted BAM files by read name'
tags:
  - bioinformatics
  - genomics
  - BAM
  - htslib
  - Rust
  - command-line-utilities
authors:
  - name: kojix2
    affiliation: '1'
affiliations:
  - index: 1
    name: Independent researcher
date: 2 August 2026
bibliography: paper.bib
---

# Summary

BAM files are commonly sorted by genomic coordinate, which allows efficient access to records from a selected region [@li2009sam]. However, records belonging to one read may occur at different coordinates because of secondary or supplementary alignments. Examining a read as a whole therefore requires collecting its records by QNAME rather than by genomic position.

`qbix` adds QNAME lookup to an existing coordinate-sorted BAM file through a side index. It uses a fixed-width hash to locate candidate BAM records and then verifies their exact QNAMEs. The index can be built with a memory-efficient external sort. This allows alignments of a read identified in one genomic region to be collected from distant coordinates and used for detailed inspection, realignment, local assembly, and other read-centered analyses.

# Statement of need

Coordinate-based analysis often identifies a read at a genomic region of interest, but the next step is to examine the read as a whole. Other records with the same QNAME may describe a distant breakpoint, an alternative mapping, or another segment of a chimeric alignment. This is especially important for long reads that span complex rearrangements or sequences that are poorly represented in the reference genome.

QNAME lookup connects analysis of a genomic region with read-centered analysis. After a coordinate-based analysis identifies a read of interest, its QNAME can retrieve every BAM record for that read. These records support detailed read inspection, realignment to candidate sequences, local assembly, and construction of read--locus graphs. The same access is useful when reads are grouped by haplotype or base-modification pattern.

`samtools view -N` can extract records that match a list of QNAMEs [@danecek2021twelve]. However, it must scan the entire BAM file even when only a few reads are requested. This makes it slow for large BAM files. `qbix` is designed to retrieve all records belonging to specified QNAMEs from an existing coordinate-sorted BAM file.

# State of the field

BAI and CSI, the index formats commonly used with BAM files, index genomic coordinates but not QNAMEs. A queryname-sorted BAM can support QNAME lookup through a sparse index that uses the BAM file itself [@kojix2026bni]. However, many tools that process BAM files expect coordinate-sorted input, so a separate coordinate-sorted BAM usually remains necessary.

For coordinate-sorted BAM files, `bri` introduced an index that stores complete QNAME strings together with BGZF virtual offsets [@simpson2019bri]. Atlantool stores complete QNAMEs and offsets in a BGZF-compressed data file and places a sparse upper-level index above it [@rath2025atlantool].

`qbix` also adds a side index to a coordinate-sorted BAM, but it does not store complete QNAME strings. Each QNAME is represented by a fixed-width hash, and candidate records are checked against their actual QNAMEs in the BAM file. Each hash is stored once, while its associated offsets are kept in a separate array. The index size therefore does not depend on QNAME length, and the search key is not repeated for multiple records belonging to the same QNAME.

# Software design

## QBI index structure

A QBI file contains a 128-byte header and five data sections: a radix directory, a hash-suffix array, a group-start bit vector, a rank directory, and an offset array. The header records the position and size of each section, the BAM record count, the number of unique hashes, the radix parameters, and information about the source BAM file.

Each 64-bit QNAME hash `h` is divided into a radix prefix formed by the high `P` bits and a hash suffix formed by the remaining bits. The radix directory records the start and end positions in the hash-suffix array for each prefix. The hash-suffix array stores the suffix of each unique hash once, in the order of the original 64-bit hashes. QBI currently supports `P = 8` and `P = 16`.

The offset array stores one 64-bit BGZF virtual offset for each BAM record. The offsets are ordered by hash and then by virtual offset within each hash group. One hash may correspond to several offsets.

The group-start bit vector marks the start of each hash group in the offset array. One bit corresponds to each offset: the first offset in a group is marked with 1 and the others with 0. One additional sentinel bit marks the end of the final group. The rank directory accelerates `select1` on this bit vector. It stores the cumulative number of set bits before each 512-bit block; `select1` first locates the relevant block and then examines at most eight 64-bit words within it.

![QBI separates hash lookup from offset retrieval. The sections are shown in logical lookup order.](figures/qbi-index-structure.png){width=100%}

During lookup, the QNAME hash is divided into its prefix and suffix. The radix directory identifies the relevant range of the hash-suffix array, and binary search is limited to that range. When a hash is found, the group-start bit vector and rank directory give the start and end positions of its offset group.

`qbix` reads the candidate BAM records through htslib [@bonfield2021htslib] and compares their actual QNAMEs. This ensures that only the correct records are returned, even when hash collisions occur.

## Memory-efficient construction

`qbix` scans the BAM file once and distributes fixed-width `(hash, offset)` records into temporary files according to the high bits of the hash. Each temporary file is sorted independently and then processed in prefix order to build the QBI index. Peak memory use therefore depends mainly on the largest bucket sorted at one time, rather than on the total number of BAM records.

The sorted records are processed once. Each offset is written, while suffixes and group boundaries are recorded only when the hash changes. The rank and radix directories are built in the same pass, without retaining all hashes and offsets in memory.

## Lookup and interfaces

`qbix` provides query-order and BAM-order output modes. Query-order mode reads QNAMEs incrementally and reports results in the same order. BAM-order mode sorts the candidate offsets to reduce random seeks when many QNAMEs are requested.

QNAMEs can be supplied as arguments, from a file, or through standard input, and results are written as SAM or BAM. `qbix` can remove repeated queries, record missing QNAMEs, and is also available through Rust and C APIs.

# Evaluation

Current measurements use HG002 chromosome 21 subsets from Illumina, PacBio HiFi, and Oxford Nanopore data. Values are medians, and timings below the timer resolution are reported as `<0.01 s`. QBI was built with `P = 16`. The final evaluation will repeat the same measurements on public whole-genome BAM files and will report accession, BAM size, record count, unique QNAME count, hardware, software versions, commands, repetition count, and cache conditions.

## Index construction

Table 1 reports the qbix measurements available so far. Atlantool construction measurements and temporary-disk use remain as placeholders.

| Dataset | Tool | Build time | Peak RSS | Temporary disk | Index size |
|:--|:--|--:|--:|--:|--:|
| Illumina chr21 | qbix | 14.32 s | 21.9 MiB | `[TO BE MEASURED]` | 124.8 MB |
|  | Atlantool | `[TO BE MEASURED]` | `[TO BE MEASURED]` | `[TO BE MEASURED]` | `[TO BE MEASURED]` |
| PacBio HiFi chr21 | qbix | 4.40 s | 8.4 MiB | `[TO BE MEASURED]` | 2.4 MB |
|  | Atlantool | `[TO BE MEASURED]` | `[TO BE MEASURED]` | `[TO BE MEASURED]` | `[TO BE MEASURED]` |
| Oxford Nanopore chr21 | qbix | 19.04 s | 20.5 MiB | `[TO BE MEASURED]` | 7.1 MB |
|  | Atlantool | `[TO BE MEASURED]` | `[TO BE MEASURED]` | `[TO BE MEASURED]` | `[TO BE MEASURED]` |

A separate PacBio HiFi whole-genome run compared QBI representations. Build times were similar: 286.11 s for QBI1 and 288.23 s for QBI with `P = 16`. The corresponding index sizes were 151.0 MB and 131.5 MB.

## End-to-end lookup

Table 2 reports representative present-QNAME measurements. Timings include index lookup, BAM access, QNAME verification, and output. The complete benchmark also includes query sets of 10 and 1,000 QNAMEs and will be distributed with the raw results.

| Dataset | QNAMEs | qbix, query order | Atlantool | `samtools view -N` full scan |
|:--|--:|--:|--:|--:|
| Illumina chr21 | 1 | <0.01 s | 0.03 s | 5.40 s |
|  | 100 | 0.01 s | 0.15 s | 5.39 s |
|  | 10,000 | 1.06 s | 6.12 s | 5.36 s |
| PacBio HiFi chr21 | 1 | <0.01 s | <0.01 s | 4.09 s |
|  | 100 | 0.01 s | 0.08 s | 4.05 s |
|  | 10,000 | 0.74 s | 3.31 s | 4.16 s |
| Oxford Nanopore chr21 | 1 | 0.01 s | 0.24 s | 17.91 s |
|  | 100 | 0.03 s | 0.33 s | 18.03 s |
|  | 10,000 | 1.70 s | 6.03 s | 18.19 s |

For 10,000 present QNAMEs, BAM-order mode reduced qbix time to 0.98 s for Illumina, 0.72 s for PacBio HiFi, and 1.56 s for Oxford Nanopore. For 10,000 absent QNAMEs, query-order lookup took `<0.01 s`, `<0.01 s`, and 0.01 s, respectively. The corresponding Atlantool and `samtools view -N` absent-query measurements remain `[TO BE MEASURED]`.

Correctness will be checked across all tools against a complete `samtools view -N` scan after canonicalizing record order and SAM formatting. Secondary and supplementary records must match. Cross-tool correctness hashes are `[TO BE MEASURED]`. Benchmark scripts, query sets, commands, and raw results will be included in the repository.

# Research impact statement

`qbix` adds read-name lookup while keeping the coordinate-sorted BAM used for region-based access. It can pass all alignments of a read identified in a genomic region to downstream analysis without requiring a second queryname-sorted BAM.

This manuscript does not claim external scholarly adoption. Its current contributions are a documented one-to-many index, exact BAM-level QNAME verification, memory-efficient construction, and a reproducible evaluation.

# Availability

`qbix` is available from [GitHub](https://github.com/kojix2/qbix) and [crates.io](https://crates.io/crates/qbix) under the MIT license. Prebuilt binaries are distributed through GitHub Releases, and releases are archived on Zenodo [@kojix2026qbix]. The repository includes the QBI format specification, benchmark scripts, raw measurements, tests, and examples.

# AI usage disclosure

OpenAI ChatGPT (GPT-5.6 Thinking, accessed July--August 2026) was used for implementation suggestions, code review, test planning, documentation, manuscript restructuring, and English revision. It also assisted discussion of the index design and interpretation of results supplied by the author. The model did not execute the reported benchmarks. The author made the architectural decisions and reviewed and validated all AI-assisted code and text.

# Acknowledgements

The author thanks the developers of `bri` for the read-name-index concept and the htslib and SAMtools communities for the BAM and BGZF infrastructure used by `qbix`.

# References
