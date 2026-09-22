---
title: 'qbix: Read-name lookup for coordinate-sorted BAM files'
tags:
  - bioinformatics
  - genomics
  - BAM
  - htslib
  - Rust
  - command-line-utilities
authors:
  - name: kojix2
date: 2 August 2026
bibliography: paper.bib
documentclass: article
fontsize: 10pt
papersize: a4
header-includes:
  - \usepackage[a4paper,margin=24mm]{geometry}
---

# Summary

BAM files are commonly sorted by genomic coordinate, which allows efficient access to records from a selected region [@li2009sam]. However, records belonging to one read may occur at different coordinates because of secondary or supplementary alignments. Determining where all parts of a read align therefore requires collecting its records by QNAME rather than by genomic coordinate.

`qbix` adds QNAME lookup to an existing coordinate-sorted BAM file through a side index. It uses a fixed-width hash to locate candidate BAM records and then verifies their exact QNAMEs. The index can be built with a memory-efficient external sort. This allows alignments of a read identified in one genomic region to be collected from distant coordinates and used for detailed inspection, realignment, local assembly, and other read-centered analyses.

# Statement of need

Analyses of complex structural variation often require examining individual reads to determine precisely where each part maps to the reference genome. This is especially important for long reads. For example, collecting all alignment records with the same QNAME reveals the genomic location of each segment of a chimeric read.

QNAME lookup connects analysis of a genomic region with read-centered analysis. After a coordinate-based analysis identifies a read of interest, its QNAME can retrieve every BAM record for that read. These records support realignment to candidate sequences, local assembly, and analysis of relationships among genomic regions supported by the same read. The same retrieval method is useful for collecting alignments of reads that have been grouped by haplotype or base-modification pattern.

`samtools view -N` can extract records that match a list of QNAMEs [@danecek2021twelve]. However, it must scan the entire BAM file even when only a few reads are requested. This makes it slow for large BAM files. SA tags can provide the locations of supplementary alignments, but their availability depends on the aligner and its settings. They do not provide a general mechanism for enumerating every record with the same QNAME. `qbix` is designed to locate candidate records in an existing coordinate-sorted BAM file and return them after checking their actual QNAMEs.

# State of the field

BAI and CSI, the index formats used with coordinate-sorted BAM files, do not support QNAME lookup. A queryname-sorted BAM can support QNAME lookup through a sparse index that uses the BAM file itself [@kojix2026bni]. However, many tools also require a coordinate-sorted BAM, so retaining both orders means storing the BAM data twice.

For coordinate-sorted BAM files, `bri` introduced an index that stores complete QNAME strings together with BGZF virtual offsets [@simpson2019bri]. Atlantool stores complete QNAMEs and offsets in a BGZF-compressed data file and places a sparse upper-level index above it [@rath2025atlantool]. Both retain complete QNAME strings as search keys.

In contrast, `qbix` stores a fixed-width hash instead of the QNAME string, so the index size does not depend on QNAME length. During lookup, it reads BAM records at the BGZF virtual offsets associated with the hash and returns only records whose QNAME matches the query.

# Software design

`qbix` primarily targets QNAME lookup in coordinate-sorted long-read BAM files. Its design has three goals: fast lookup, a small index, and fast construction. The QBI index is a one-to-many index from fixed-width hashes to BAM record locations. It is built by an external bucket sort over fixed-width records.

## Hash-based lookup

A QNAME can occur in more than one BAM record, for example when a read has secondary or supplementary alignments. The index therefore maps the XXH3-64 hash of each QNAME to one or more BGZF virtual offsets. A BGZF virtual offset combines the location of a compressed block with an uncompressed position within that block.

The search keys are hashes rather than QNAME strings. Their size does not depend on QNAME length. The hash array contains each distinct hash once, in ascending order. The corresponding offsets are stored as contiguous groups in a separate array.

Each hash is divided into a radix prefix formed by its high `P` bits and a suffix formed by the remaining bits. The radix directory records the suffix-array range for each prefix. Binary search is limited to this range. This requires fewer comparisons than searching the complete array.

Offsets associated with the same hash occupy a contiguous range in the offset array. A hash's position in the hash array also identifies its offset group. A separate bit vector marks the start of each group. The rank directory records the cumulative number of group boundaries before each 512-bit block. These counts identify the block containing a requested boundary. Two consecutive group boundaries delimit the offset range for one hash.

![QBI separates hash lookup from offset retrieval. The sections are shown in logical lookup order.](figures/qbi-index-structure.png){width=100%}

`qbix` reads the BAM records at the resulting virtual offsets through htslib [@bonfield2021htslib]. It returns only records whose actual QNAME matches the query. The BAM record at each retrieved offset must be read before any result can be emitted. Exact verification adds only a QNAME comparison to this read. A hash collision causes extra BAM records to be read. It cannot add a record with the wrong QNAME to the result.

Published datasets and a typical 30-fold WGS configuration put the number of distinct QNAMEs in a human WGS BAM at approximately 2--10 million for long reads and 300--400 million for Illumina reads [@shumate2020ashkenazi; @illuminaDnaPrep]. Assuming independent, uniformly distributed 64-bit hashes, the probability that at least one pair of distinct QNAMEs shares a hash is on the order of 10⁻⁷--10⁻⁶ in the former range and approximately 0.3--0.4% in the latter.

The QBI header records the size, modification time, and header hash of the source BAM. Before lookup, `qbix` compares these values with metadata from the supplied BAM.

## Index construction

During index construction, `qbix` scans the BAM once and distributes fixed-width `(hash, offset)` records into temporary buckets selected by the high bits of the hash. Each bucket can be sorted independently. Several buckets can therefore be sorted in parallel. Processing the sorted buckets in numeric order yields the global `(hash, offset)` order, allowing the records to be streamed directly into the QBI file. This design avoids retaining all records in memory at the cost of temporary disk space and additional I/O.

# Evaluation

Measurements used chromosome 21 subsets extracted with `samtools view -bh` from three public HG002 alignment BAM files: GIAB Illumina HiSeq 2x250 Novoalign, GIAB PacBio HiFi Revio 48x (BioProject PRJNA1028149), and Oxford Nanopore R10.4.1 SUP from ONT Open Data. The Illumina source header declared `SO:unsorted`; its chromosome subset was therefore sorted again with SAMtools before measurement. The resulting BAM sizes and record counts were 1.52 GiB and 11,145,955 records for Illumina, 962.3 MiB and 133,605 records for PacBio HiFi, and 4.90 GiB and 545,839 records for Oxford Nanopore.

Benchmarks ran on Ubuntu Linux 26.04 with an AMD Ryzen 7 5700X (8 cores, 16 threads), 60 GiB RAM, and a Samsung 870 QVO SATA SSD using qbix 0.0.9, SAMtools/HTSlib 1.24, and Atlantool release `release-983975f`. The QBI index was built with `P = 16`, one BGZF thread, one sorting thread, a 512 MiB sorting-memory setting, and eight bucket bits. Index values are medians of three builds; lookup values are medians of five independently sampled query sets generated with seed 20260730. The BAM was read once before timing, and the filesystem cache was not explicitly cleared. Timings below the 0.01 s timer resolution are reported as `<0.01 s`. Commands, manifests, query checksums, and per-replicate results are recorded by the benchmark workflow in `paper/work`.

## Index construction

Table 1 reports index construction measurements for the chromosome 21 subsets. Peak temporary-disk use was sampled every 0.25 s while each tool ran; a zero value means that no transient file was observed at that sampling interval.

| Dataset | Tool | Build (s) | RSS (MiB) | Temp (MiB) | Index (MiB) |
|:--|:--|--:|--:|--:|--:|
| Illumina | qbix | 13.58 | 22.5 | 170.1 | 119.0 |
|  | Atlantool | 29.99 | 717.4 | 105.9 | 103.9 |
| PacBio HiFi | qbix | 4.11 | 8.9 | 1.9 | 2.3 |
|  | Atlantool | 8.71 | 291.7 | 0.0 | 1.4 |
| ONT | qbix | 17.98 | 21.3 | 8.0 | 6.8 |
|  | Atlantool | 33.88 | 542.2 | 10.4 | 11.2 |

On the PacBio HiFi chromosome 21 subset, a separate three-build comparison in the same environment gave median build times of 4.11 s for the legacy QBI1 format and 4.13 s for the default QBI2 format (`P = 16`). The corresponding index sizes were 2.0 MiB and 2.3 MiB. At this subset size, the fixed radix directory makes QBI2 slightly larger than QBI1.

## End-to-end lookup

`qbix` can emit results in query order or BAM order. Query order preserves the input order, whereas BAM order sorts candidate offsets to reduce random seeks.

The scaling curves show the different cost profiles of indexed lookup and a full BAM scan. qbix was fastest or tied for fastest throughout the measured range. Its time increased with the number of present QNAMEs, whereas `samtools view -N` remained nearly constant because it scanned the complete input for every query set. Atlantool also avoided a full scan, but its time increased more rapidly than qbix on these datasets.

![End-to-end lookup time for present QNAMEs. Points are medians of five query sets; lines connect measured query counts. Values recorded as 0.00 s are shown at 0.005 s solely for logarithmic plotting.](figures/query-scaling.png){width=100%}

Table 2 reports representative times from the same present-QNAME measurements. The qbix column uses query order and the SAMtools column uses a full scan. Timings include index lookup, BAM access, QNAME verification, and SAM formatting to `/dev/null`. The complete benchmark also includes query sets of 10 and 1,000 QNAMEs.

| Dataset | QNAMEs | qbix (s) | Atlantool (s) | SAMtools (s) |
|:--|--:|--:|--:|--:|
| Illumina | 1 | <0.01 | 0.02 | 4.85 |
|  | 100 | 0.01 | 0.14 | 4.86 |
|  | 10,000 | 0.96 | 5.66 | 4.85 |
| PacBio HiFi | 1 | <0.01 | <0.01 | 3.87 |
|  | 100 | 0.01 | 0.07 | 3.84 |
|  | 10,000 | 0.70 | 3.06 | 3.90 |
| ONT | 1 | 0.01 | 0.23 | 16.93 |
|  | 100 | 0.03 | 0.32 | 16.97 |
|  | 10,000 | 1.58 | 5.54 | 17.16 |

For 10,000 present QNAMEs, BAM-order mode reduced qbix time to 0.89 s for Illumina, 0.67 s for PacBio HiFi, and 1.47 s for Oxford Nanopore. For 10,000 absent QNAMEs, query-order lookup took `<0.01 s`, `<0.01 s`, and 0.01 s, respectively. The corresponding Atlantool times were 2.78 s, 2.09 s, and 2.35 s, while `samtools view -N` took 4.86 s, 3.83 s, and 16.94 s.

Correctness was checked against a complete `samtools view -N` scan after normalizing record order and SAM optional-tag order. For the 10,000-QNAME sets, qbix query-order, qbix BAM-order, Atlantool, and SAMtools produced identical record counts and SHA-256 hashes: 19,971 records and `8ed699da...27d8e8` for Illumina, 10,167 records and `e414c34d...bd4b34` for PacBio HiFi, and 15,594 records and `336cbbab...885d1` for Oxford Nanopore. All four methods returned zero records for the absent-QNAME sets.

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
