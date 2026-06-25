# qbix bucket partition builder 実装プラン

## 目的

現在の `qbix index` は全 record を `Vec<Record>` に積み、最後に一括 sort/write するため、
巨大 BAM で record 数 × 16 byte（例: 6 億 record で ~10 GB）のメモリを消費する。

これを **`.qbi` format を変更せず、build 経路だけ** bucket partition 方式へ置き換え、
ピークメモリを「最大 1 bucket 分」に抑える。

> 現在のメモリ消費は [src/index.rs](src/index.rs) の `IndexStorage::Owned { records: Vec<Record> }` に
> 全件を push し、[Index::save()](src/index.rs) で一括 `sort_by` → write する構造に由来する。

---

## 基本方針

- `.qbi` format（`QBI1` / header 48 byte / record 16 byte）は**変更しない**
- `load`, `get`, `show`, `check`, `stats` は基本的にそのまま使える
- `Index::save()`（in-memory 経路）は小規模 API/テスト用に**残す**
  - ただし sort は `sort_by` → `sort_unstable_by` に変更
- `qbix index` の build 経路だけ新しい `BucketIndexBuilder` を使う
- default `bucket_bits = 8`（= 256 buckets）
- default `memory_limit = 512 MiB`
- `bucket_bits` の許容範囲は `1..=12`
- oversized bucket は v1 では明示エラー（fail-fast）
- 将来の究極 fallback は recursive split ではなく **external merge sort**（v2 で追加できる設計に留める）

---

## アルゴリズム

```text
scan BAM once
  readname   = rec.qname()?            # 現状と同じ。UTF-8 検証込み
  qhash      = qname_hash64(readname)  # xxh3_64
  bucket     = (qhash >> (64 - bucket_bits)) as usize
  append (qhash, file_offset) を bucket の staging buffer へ 16 byte LE で
  total_records      += 1
  buckets[bucket].records += 1
  buckets[bucket].bytes   += 16
  # fail-fast: bytes > memory_limit になった時点でエラー（全 scan を待たない）

finish
  flush all bucket buffers
  create final tmp = "<out>.tmp.<pid>"  (= output index と同じ directory)
  write QBI1 header (record_count = total_records)

  for bucket in 0..bucket_count:        # prefix 昇順
    read bucket temp file into Vec<Record>
    sort_unstable_by(|a, b| a.cmp_key(b))   # (qhash, file_offset)
    append sorted records to final tmp
    delete bucket temp

  flush/close final tmp
  rename(final tmp -> <out>.qbi)        # atomic
```

### レコードのディスク表現（bucket temp）

```text
u64 qhash         (little-endian)
i64 file_offset   (little-endian)   # 16 byte / record
```

---

## 正しさ

- `bucket = qhash >> (64 - bucket_bits)` は qhash の上位 prefix。
  bucket を prefix 昇順に処理し、各 bucket 内を `(qhash, file_offset)` で sort すれば、
  最終出力は既存 [Index::save()](src/index.rs) と**完全に同じ** `(qhash, file_offset)` 昇順になる
  （バイト一致）。
- `file_offset` は BGZF virtual offset で record ごとに一意 → `(qhash, file_offset)` は全順序。
  よって `sort_unstable_by` でも結果は決定的。
- この「qhash 昇順・同一 qhash が連続」という不変条件は、以下が依存している:
  - [Index::range_indices()](src/index.rs) の二分探索
  - [compute_qname_hash_stats()](src/commands.rs)（`stats` コマンド。run-length で集計）
  bucket builder はこの不変条件を保つため、これらは無改修で動く。

---

## fd 対策（staging buffer + append open/close）

256（〜4096）個の writer を**常時 open しない**。bucket への書き込みは:

```text
bucket ごとに staging buffer（lazy 確保）
buffer 満杯時だけ:
  OpenOptions::new().create(true).append(true).open(path)
  write
  close (drop)
```

これにより同時 open fd は実質 1 に保たれ、macOS の `ulimit -n 256` でも動く。
（10 GB 相当でも open/close は十数万回程度で、BAM 展開に対し無視できるコスト。）

---

## buffer / メモリ設計

`bucket_bits` が大きいと、全 bucket へ 64 KiB buffer を確保すると無視できない:

```text
256   buckets * 64 KiB =  16 MiB
4096  buckets * 64 KiB = 256 MiB
65536 buckets * 64 KiB =   4 GiB   # → v1 では許可しない
```

対策:

- `BucketState.buffer` は `Option<Vec<u8>>` とし、**最初に record が来た bucket だけ確保**する
- v1 は `bucket_bits <= 12`（最大 4096 buckets = 256 MiB buffer）に制限
- 将来 `bucket_bits` を上げるなら、総バッファ予算固定（`buffer_size = budget / bucket_count`）
  または active buffer 数の LRU flush を追加

`--memory` の意味は **global limit ではなく**、実装上は明確に
**「finish 時に 1 bucket を読み込む Vec の最大サイズ」** として扱う。
実ピーク ≒ `1 bucket Vec (<= memory_limit)` + `確保済み staging buffer 合計` + `htslib buffer`。
`bucket_bits=12` では後者が最大 256 MiB 乗ることを help / docs に明記する。

```rust
struct BucketState {
    path: PathBuf,
    buffer: Option<Vec<u8>>, // lazy allocate
    bytes: u64,
    records: u64,
}
```

oversized 判定（fail-fast、scan 中に）:

```text
if buckets[bucket].bytes > memory_limit:
    error "[qbix] bucket is too large; retry with larger --memory or higher --bucket-bits"
```

---

## 一時ファイル

- **final tmp は必ず output index と同じ directory** に作る（rename の atomicity は同一 FS 内のみ保証）
  - 例: `reads.bam.qbi.tmp.<pid>`
  - 成功後に `rename(final_tmp, <out>.qbi)`
- `--temp-dir` は **bucket temp 専用**
  - 指定時はそこへ、未指定なら output index と同じ directory へ置く
  - bucket temp は読み戻して再書き込みするだけなので cross-device でも問題ない
- temp 使用量は最終 index とほぼ同サイズ（= record_count × 16 byte）。ディスク約 2 倍。docs に明記。

---

## cleanup（RAII guard）

`TempGuard` を作り、以下を Drop / error / panic 時に確実に削除する:

- 各 bucket temp file
- final tmp file

正常系:

- bucket 処理後、その bucket temp を削除し guard の対象から外す
- final tmp の `rename` 成功後、guard を disarm（forget）して残骸を残さない

---

## CLI / API

### CLI（[src/cli.rs](src/cli.rs)）

```sh
qbix index --memory 512M --bucket-bits 8 --temp-dir DIR reads.bam
```

default:

```text
--memory      512M       # K/M/G suffix を parse
--bucket-bits 8          # 1..=12 を範囲チェック
--temp-dir    (unset)
```

- 既存スタイルに合わせ `ARG_MEMORY` / `ARG_BUCKET_BITS` / `ARG_TEMP_DIR` const と
  `*_arg()` ヘルパを追加、`index_command()` に `.arg(...)` 追加
- `Action::Index` に `memory_limit: usize` / `bucket_bits: u8` / `temp_dir: Option<String>` を追加
- `--bucket-bits` は advanced 扱いでよい

### Rust API（[src/api.rs](src/api.rs)）

`BuildOptions` を拡張:

```rust
pub struct BuildOptions {
    pub index_path: Option<PathBuf>,
    pub threads: usize,
    pub verbose: bool,
    pub memory_limit: Option<usize>,   // None = default 512 MiB
    pub bucket_bits: Option<u8>,       // None = default 8
    pub temp_dir: Option<PathBuf>,     // None = output index と同じ directory
}
```

- フィールド追加はリテラル構築に対し技術的に破壊的。この機会に
  `BuildOptions`（および `LookupOptions` / `CheckOptions`）へ `#[non_exhaustive]` を付与する。
- `Default` の値は従来挙動（512M / 8 / unset）。

### C API（[src/c_api.rs](src/c_api.rs)）

- ABI 維持のため既存 `qbix_build_index(bam_path, index_path, threads)` は **default 固定**で呼ぶ
- 拡張が必要になれば将来 `qbix_build_index_with_options` を追加

---

## `commands::build_index` の変更

現在のシグネチャ:

```rust
pub(crate) fn build_index(input_bam, output_index, verbose, threads) -> Result<()>
```

- 引数に `memory_limit: usize` / `bucket_bits: u8` / `temp_dir: Option<&str>` を追加
- 呼び出し元は **3 か所**: [cli.rs](src/cli.rs) / [api.rs](src/api.rs) / [c_api.rs](src/c_api.rs)
  （c_api は default 値を渡す）
- 内部を `Index::new()` + `index.add()` + `index.save()` から
  `BucketIndexBuilder::new(...)` + `builder.add(readname, file_offset)` + `builder.finish(...)` へ
- verbose 進捗は builder が公開する `total_records()` と直近 record（qhash, file_offset）で出す
  （現状の readname 表示は scan ループ側で保持している readname をそのまま使える）

---

## 実装対象ファイル

| ファイル              | 変更                                                                            |
| ----------------- | ----------------------------------------------------------------------------- |
| `src/index.rs`    | `Record::cmp_key()`、`BucketIndexBuilder`、`BucketState`、`TempGuard`、書き込み helper 共通化、`save()` を `sort_unstable_by` 化 |
| `src/commands.rs` | `build_index()` を builder 経由へ。verbose は `total_records` + 直近 record + readname  |
| `src/cli.rs`      | `--memory` / `--bucket-bits` / `--temp-dir` 追加、`Action::Index` 拡張             |
| `src/api.rs`      | `BuildOptions` 拡張（+ `#[non_exhaustive]`）                                       |
| `src/c_api.rs`    | 既存 ABI 維持（default 値を渡すだけ）                                                      |
| `docs/qbi-format.md` | format は不変。build algorithm の節を追記（任意）                                          |
| `tests/*`         | 一致性・cleanup・error・default behavior テストを追加                                      |

---

## テスト

1. 小さい BAM で従来 `Index::save()` と bucket builder の出力が**バイト一致**
2. `--bucket-bits 1` でも全体が正しく `(qhash, file_offset)` 昇順になる
3. `--memory` が小さすぎると oversized bucket error（fail-fast）
4. 正常終了後に bucket temp / final tmp が残らない
5. エラー時に bucket temp / final tmp が掃除される
6. final 書き込み失敗時に既存 `.qbi` を壊さない（rename 前に旧ファイルは無傷）
7. `show` の出力が `(qhash, offset)` 昇順
8. `get` が従来通り動く（query / bam order とも）
9. `stats` の出力が従来 `Index::save()` 経路と一致（不変条件の確認）
10. CLI の index default 挙動・Rust API / C API の既存テストが通る
11. `--bucket-bits 0` / `13` 以上は範囲エラー

---

## まとめ

v1 は **hash prefix bucket + lazy staging buffer append + final atomic rename + RAII cleanup** で実装する。
巨大 bucket（同一 qhash の極端な多重度など）への究極 fallback は recursive split ではなく
**external merge sort**。これは v2 で追加できるよう、bucket 単位処理のインターフェイスだけ残しておく。
