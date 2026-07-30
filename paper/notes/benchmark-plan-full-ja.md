# qbix ベンチマーク測定計画書

**対象:** qbix 論文用性能評価  
**想定環境:** 一般的な Linux デスクトップまたはワークステーション、ローカル SSD  
**目的:** qbix の設計上の利点を、再現可能かつ公平な測定によって示す

## 1. このベンチマークで答える問い

本評価では、単に「qbix が速い」と示すのではなく、次の問いに個別に答える。

1. **インデックス構築コストは現実的か。**  
   構築時間、ピークメモリ、完成したインデックスのサイズを測定する。

2. **QNAME 検索は BAM 全体の逐次走査より速いか。**  
   取得する QNAME 数を変え、qbix、Atlantool、`samtools view -N` を比較する。

3. **多数の QNAME を取得するとき、qbix の `--bam-order` は有効か。**  
   `--query-order` と `--bam-order` の交差点を測定する。

4. **固定長ハッシュ方式により、インデックスサイズが QNAME 長に依存しないか。**  
   QNAME 長だけを変えた合成 BAM を使い、1レコード当たりのインデックスサイズを比較する。

5. **すべてのツールが同じレコード集合を返すか。**  
   性能測定の前に、出力の正しさを機械的に確認する。

この5点が確認できれば、JOSSだけでなく一般的なソフトウェア論文でも、qbixを新たに作る理由を説明しやすい。

---

## 2. 比較対象

### 必須

| ツール | 評価上の役割 |
|---|---|
| qbix | 提案手法 |
| Atlantool | 現在の直接的な比較対象 |
| `samtools view -N` | インデックスを作らず BAM 全体を走査する標準的な基準 |

### 可能なら追加

| ツール | 評価範囲 |
|---|---|
| bri | インデックス構築、インデックスサイズ、単一 QNAME 検索 |

`bri` のCLIは基本的に単一QNAME検索を想定しているため、多数のQNAMEについてシェルループで繰り返す測定は主比較に含めない。これはプロセス起動回数の比較になり、qbixやAtlantoolの一括検索と公平でないためである。

`grep`を使った方法は主比較から外す。逐次走査の基準には、QNAMEリストを直接受け取れる `samtools view -N names.txt input.bam` を使用する。

---

## 3. 評価の優先順位

### 論文に最低限必要な主実験

1. 実データでのインデックス構築時間・ピークRSS・インデックスサイズ
2. QNAME数を変えた検索時間
3. qbixの `--query-order` と `--bam-order` の比較
4. 合成データによる「QNAME長とインデックスサイズ」の関係
5. 出力一致の検証

### 余力があれば行う補助実験

- cold-cache検索
- スレッド数によるインデックス構築時間の変化
- qbixのメモリ上限を変えた構築試験
- BAMサイズを変えたスケーリング試験
- 存在しないQNAMEだけを検索する試験

主実験を先に完了し、補助実験は結果を見て必要なものだけ追加する。

---

## 4. 使用するデータセット

## 4.1 実データ

### データセットL: ロングリード BAM（必須）

- NanoporeまたはPacBio
- 座標ソート済み BAM
- ローカルSSDに置く
- 数GB以上、できれば数千万レコードまでの範囲
- supplementary alignmentを含む通常のアラインメント結果
- 公開データならアクセッション番号を記録する

qbixの用途を最も自然に示せるため、論文の主データセットとする。

### データセットS: ショートリード BAM（推奨）

- Illumina paired-end BAM
- WES、染色体単位のWGS、または扱いやすい大きさの公開BAM
- 数GB以上

ショートリードではレコード数が多いため、固定長インデックスの構築コストとサイズを別の条件で確認できる。PCの空き容量や測定負荷が問題になる場合、ロングリード1本だけでも主実験は成立する。ただし、その場合は論文でショートリード全般への性能上の一般化を避ける。

### データの再現性

論文で公開する結果には、次のいずれかを必ず付ける。

- 公開データのアクセッション番号と取得手順
- 公開できないBAMを使う場合は、同じ測定を再現できる公開代替データ
- 合成BAM生成スクリプト

患者由来データを使う場合、QNAMEそのものをリポジトリや論文補足資料に公開しない。問い合わせ用QNAMEファイルは、公開データから再生成できるスクリプトと乱数seedを公開する。

## 4.2 合成データ

固定長インデックスの性質を検証するため、次の4種類のBAMを作成する。

| 条件 | QNAME長 | BAMレコード数 |
|---|---:|---:|
| Q16 | 16文字 | 1,000,000 |
| Q36 | 36文字 | 1,000,000 |
| Q64 | 64文字 | 1,000,000 |
| Q128 | 128文字 | 1,000,000 |

条件は次のように統一する。

- 1 QNAMEにつき1レコード
- 配列、CIGAR、座標、タグ構成は同じ
- QNAMEだけを変える
- QNAMEは一意で、決めたseedから決定論的に生成する
- 長い共通接頭辞を避け、ハッシュ値や疑似乱数由来の高エントロピー文字列を使う

AtlantoolのインデックスはBGZF圧縮されるため、`AAAAAAAA...0001`のような圧縮されやすいQNAMEを使うと、QNAME長の影響が過小評価される。したがって、QNAMEは16進数またはBase32形式のランダムに近い文字列とする。

この実験では、BAMに対するインデックスの割合ではなく、次を報告する。

- インデックス総バイト数
- BAMレコード1件当たりのインデックスバイト数

BAM自体のサイズもQNAME長によって変わるため、「BAMサイズに対する割合」だけでは設計上の差を正しく表せない。

---

## 5. ソフトウェアとバージョンの固定

測定開始前に、使用するqbixのコミットを固定する。測定後にコードを変更した場合、性能に関係しそうな変更であれば全測定をやり直す。

qbixは次の条件でビルドする。

```bash
cargo build --release --locked
```

- debug buildを使用しない
- `biosyntax` featureを有効にしない
- 実際に測定したバイナリのSHA-256を保存する
- `git rev-parse HEAD`を保存する

```bash
git rev-parse HEAD
sha256sum target/release/qbix
target/release/qbix --version
```

AtlantoolはLinux用native executableを使用し、release名またはコミットを記録する。`bri`を使う場合は、可能ならqbixと同じHTSlibに対してビルドする。

保存する情報:

```bash
samtools --version
java -version                    # JAR版を使う場合のみ
atlantool-linux --help
bri 2>&1 | head                  # version表示がなければcommit hashを保存
```

---

## 6. ハードウェア・OS情報

普通のLinux PCで問題ない。ただし、絶対性能ではなく同一環境内の相対比較として扱う。

以下を `benchmarks/env/system.txt` に保存する。

```bash
date -Is
uname -a
cat /etc/os-release
lscpu
free -h
lsblk -o NAME,MODEL,ROTA,SIZE,TYPE,FSTYPE,MOUNTPOINTS
```

各BAMと一時ディレクトリについて、実際のファイルシステムも記録する。

```bash
findmnt -T /path/to/input.bam
findmnt -T /path/to/benchmark_tmp
```

論文には少なくとも次を記載する。

- CPU型番
- 物理コア数・論理CPU数
- RAM容量
- Linux distributionとkernel
- SSDの種類（NVMe、SATA、HDD）
- ファイルシステム
- 使用スレッド数
- キャッシュ条件

測定中は、バックアップ、ウイルススキャン、動画処理などの重い処理を停止する。CPU governorやTurbo Boostを変更する必要はないが、測定途中で条件を変えない。

ネットワークファイルシステムは使用しない。BAM、インデックス、一時ファイルはローカルSSD上に置く。空き容量は、少なくともBAMサイズと同程度を確保する。

---

## 7. データセットの基本情報

各BAMについて次を記録する。

```bash
samtools quickcheck -v input.bam
samtools view -H input.bam > input.header.sam
samtools view -c input.bam
stat -c '%n\t%s' input.bam
sha256sum input.bam              # 巨大ファイルで負担なら取得元checksumでもよい
```

`datasets.tsv`には次の列を持たせる。

```text
dataset_id
source_or_accession
bam_path
bam_bytes
alignment_records
sort_order
sequencing_platform
storage_type
notes
```

`@HD`の `SO:coordinate` を確認する。座標ソートされていないBAMは主実験に使用しない。

---

## 8. 問い合わせQNAME集合の作成

## 8.1 基本方針

存在するQNAMEをBAM全体から決定論的に抽出し、次のサイズの問い合わせファイルを作る。

```text
1, 10, 100, 1,000, 10,000
```

10,000件まで問題なく終わる場合は、100,000件も追加する。

各サイズについて5個の独立なreplicateを作る。各replicate内では問い合わせ集合を入れ子にする。

例:

```text
rep1/q000001.txt  = rep1の先頭1件
rep1/q000010.txt  = rep1の先頭10件
rep1/q000100.txt  = rep1の先頭100件
...
```

これにより、QNAME数を増やしたときにデータ集合が完全に別物になることを避ける。

## 8.2 候補QNAMEの取得

`samtools view`のtemplate単位subsamplingを固定seedで使い、十分な数のQNAME候補を得る。

概念的には次の処理を行う。

```bash
samtools view \
  --subsample-seed 20260730 \
  --subsample FRACTION \
  input.bam |
  cut -f1 |
  LC_ALL=C sort -u > candidates.txt
```

`FRACTION`は、最大問い合わせ数×replicate数の少なくとも2〜4倍の候補が得られるように設定する。候補が不足した場合はfractionを増やして再生成する。

次に、Pythonなどで固定seedを用いて `candidates.txt` をshuffleし、replicateごとに分割する。問い合わせファイルには重複QNAMEを含めない。

## 8.3 存在しないQNAME

補助実験として、次のような明らかに合成された名前を作る。

```text
__QBIX_ABSENT__000000001_<hash>
__QBIX_ABSENT__000000002_<hash>
```

qbixで検索し、出力件数が0であることを事前に確認する。存在しないQNAME試験は、まず1件、1,000件、10,000件だけでよい。

---

## 9. 正しさの検証

性能測定の前に、全ツールが同じSAMレコード集合を返すことを確認する。

例:

```bash
qbix get --query-order -f names.txt input.bam > qbix.query.sam
qbix get --bam-order   -f names.txt input.bam > qbix.bamorder.sam
atlantool-linux view input.bam -f names.txt > atlantool.sam
samtools view -N names.txt input.bam > samtools.sam
```

出力順序はツールごとに異なるため、ヘッダを除いたSAM行をバイト順にソートして比較する。

```bash
LC_ALL=C sort qbix.query.sam    | sha256sum
LC_ALL=C sort qbix.bamorder.sam | sha256sum
LC_ALL=C sort atlantool.sam     | sha256sum
LC_ALL=C sort samtools.sam      | sha256sum

wc -l qbix.query.sam qbix.bamorder.sam atlantool.sam samtools.sam
```

確認事項:

- 4つの出力行数が一致する
- ソート後SHA-256が一致する
- 存在しないQNAMEでは全ツールの出力が0件
- 複数レコードを持つQNAMEで、primary、secondary、supplementaryが欠落しない

タグ順序やSAM表現にツール差がある場合は、SAMを一度BAMへ変換してから、QNAME、FLAG、RNAME、POS、CIGAR、SEQなどの必須フィールドを正規化して比較する。差異が出た場合、性能測定を続ける前に原因を解決する。

---

## 10. 実験A: インデックス構築

## 10.1 測定条件

主比較では、各ツールに明示的に1スレッド相当を指定する。

qbix:

```bash
/usr/bin/time -v -o qbix.time.txt \
  qbix index \
    --bgzf-threads 1 \
    --sort-threads 1 \
    --memory 512M \
    --bucket-bits 8 \
    --temp-dir /path/to/tmp/qbix \
    -i /path/to/index/input.qbi \
    input.bam
```

Atlantool:

```bash
/usr/bin/time -v -o atlantool.time.txt \
  atlantool-linux index input.bam --thread-count=1
```

bri:

```bash
/usr/bin/time -v -o bri.time.txt \
  bri index input.bam
```

各ツール3回実行する。各runの前に、そのツールが前回作成したインデックスと一時ファイルを削除する。削除時間は測定に含めない。

## 10.2 キャッシュ

専用PCでsudoを使用できる場合、各runの直前にページキャッシュを落とす。

```bash
sync
sudo sh -c 'echo 3 > /proc/sys/vm/drop_caches'
```

これはシステム全体のキャッシュを消去するため、他の作業をしているマシンでは実行しない。

キャッシュを落とせない場合は、次のいずれかを採用し、論文に明記する。

- BAMがRAMより十分大きい状態で3回測定する
- 1回目をcoldに近い測定として別に示し、2〜3回目をwarm-cacheとして別集計する

coldとwarmを混ぜて単一の中央値にしない。

## 10.3 記録する値

- wall-clock time
- user CPU time
- system CPU time
- maximum resident set size
- exit status
- 完成したインデックスの総バイト数
- 可能なら一時ディスク使用量の最大値

インデックスサイズは `du -h` ではなく、バイト単位で測定する。

qbix:

```bash
stat -c '%s' input.qbi
```

Atlantool:

```bash
find input.bam.atlantool-index -type f -printf '%s\n' |
  awk '{s += $1} END {print s}'
```

briは生成されたインデックスファイルの `stat -c '%s'` を記録する。

## 10.4 集計

3回の中央値を本文に示し、最小値と最大値または個々の値を補足資料に残す。ピークRSSはGNU timeの `Maximum resident set size` を使用する。

---

## 11. 実験B: QNAME検索数に対するスケーリング

## 11.1 測定するコマンド

qbix query order:

```bash
qbix get \
  --query-order \
  --bgzf-threads 1 \
  -f names.txt \
  -o /dev/null \
  input.bam
```

qbix BAM order:

```bash
qbix get \
  --bam-order \
  --bgzf-threads 1 \
  -f names.txt \
  -o /dev/null \
  input.bam
```

Atlantool:

```bash
atlantool-linux view input.bam -f names.txt > /dev/null
```

SAMtools:

```bash
samtools view -@ 1 -N names.txt input.bam > /dev/null
```

briは単一QNAME条件だけ測る。

```bash
bri get input.bam "$(head -n 1 names.txt)" > /dev/null
```

全ツールでheaderなしSAMを `/dev/null` に出力する。これにより、ファイル書き込み速度を主測定から除きつつ、BAMレコードの読み出しとSAMへの変換コストは残る。

## 11.2 測定回数

各データセット、各QNAME数について、5個のreplicate問い合わせファイルをそれぞれ1回測る。ツールの実行順序はreplicateごとに変更する。

例:

```text
rep1: qbix-query → Atlantool → samtools → qbix-bam-order
rep2: samtools → qbix-bam-order → qbix-query → Atlantool
...
```

これにより、特定ツールだけが常に他ツールの全走査後に実行され、OSキャッシュの恩恵を受けることを避ける。

## 11.3 通常キャッシュ条件

主結果では、測定系列の開始前に各バイナリを1回だけ実行して、実行ファイルと共有ライブラリを読み込ませる。その後はOSキャッシュを意図的に消去せず、5つの異なる問い合わせreplicateを測定する。

この条件は「filesystem cache was not explicitly cleared」と記載する。まったく同じQNAME集合を何度も繰り返して最良値だけを取らない。

## 11.4 cold-cache補助測定

可能なら次の2条件だけ、ページキャッシュを毎回消去して3回測る。

- QNAME 1件
- QNAME 10,000件

cold-cacheと通常キャッシュは別表または別パネルにする。

## 11.5 測定値

`/usr/bin/time`または同等のラッパーで次を記録する。

- elapsed seconds
- user seconds
- system seconds
- maximum RSS
- exit status
- query count
- 期待される出力レコード数
- query fileのSHA-256

`query_runs.tsv`の推奨列:

```text
dataset
tool
mode
query_type
query_count
replicate
cache_condition
elapsed_s
user_s
sys_s
max_rss_kb
expected_output_records
query_sha256
exit_status
```

`mode`は `query-order`、`bam-order`、`default` などとする。`query_type`は `present`、`absent`、必要なら `mixed` とする。

## 11.6 集計

各条件について5replicateの中央値と四分位範囲を報告する。統計的有意差検定は不要である。個々の生データをTSVで公開する。

主図は両対数軸とする。

- 横軸: 問い合わせQNAME数
- 縦軸: wall-clock time
- 線: qbix query-order、qbix bam-order、Atlantool、samtools

この図から、逐次走査に対してインデックスが有利になる範囲と、qbix内で `--bam-order` が有利になる範囲を読み取る。

---

## 12. 実験C: 存在するQNAMEと存在しないQNAME

この実験はインデックス検索そのものと、候補BAMレコードへのシークを分けて理解するために行う。

条件:

| query type | QNAME数 |
|---|---:|
| present | 1, 1,000, 10,000 |
| absent | 1, 1,000, 10,000 |
| mixed 50% | 任意、余力があれば1,000と10,000 |

presentではインデックス検索に加えてBAMレコードの読み出しが必要になる。absentでは候補がなければインデックス検索だけで終了する。両者を分けて示すことで、単なる出力件数の違いを検索アルゴリズムの差と誤解することを防ぐ。

---

## 13. 実験D: QNAME長とインデックスサイズ

合成BAM Q16、Q36、Q64、Q128について、qbix、Atlantool、可能ならbriのインデックスを構築する。

各条件で記録する値:

- BAMレコード数
- 平均QNAME長
- BAMサイズ
- インデックス総サイズ
- index bytes / BAM record
- インデックス構築時間（補助値）

主図:

- 横軸: QNAME長
- 縦軸: index bytes per BAM record
- 線: qbix、Atlantool、bri

この実験の主目的は速度比較ではなく、保存形式の違いを直接可視化することである。インデックスサイズは決定論的なので、サイズだけなら各条件1回でよい。構築時間も本文に載せる場合は3回測る。

---

## 14. 補助実験

## 14.1 インデックス構築の並列化

CPUに余裕がある場合、1、2、4、8の名目スレッド数で測定する。

qbixでは、BGZFスレッドとソートスレッドを明示する。

```bash
qbix index \
  --bgzf-threads 4 \
  --sort-threads 4 \
  --memory 512M \
  input.bam
```

qbixの `--memory` はソートworkerごとの上限として効くため、`--sort-threads 4 --memory 512M` では最大で約4倍のbucketメモリを使用し得る。この点を結果表に明記する。

並列化は主比較ではなく、利用者向けの補足結果とする。

## 14.2 メモリ上限

大きめのBAMについて、次を試す。

```text
128M, 512M, 2G
```

低いメモリ上限でbucketが大きすぎる場合、`--bucket-bits`を増やす。報告時にはmemoryとbucket-bitsを必ず併記する。

この実験から「指定値を厳密に超えない」と断言するのではなく、「bucketソート用メモリを制御でき、観測されたピークRSSはこの範囲だった」と記述する。プロセス全体にはHTSlib、mmap、bufferなどの追加メモリがあるためである。

## 14.3 BAMサイズ依存性

論文に「検索時間はBAMサイズにほぼ依存しない」と残す場合は、BAMサイズを変えた測定を追加する。

同じ元BAMから固定seedでsubsampleし、レコード数の異なる4つの座標順BAMを作る。

```text
約1%, 5%, 25%, 100%
```

各BAMで100件または1,000件のQNAMEを検索し、検索時間を比較する。同時にインデックス構築時間をレコード数に対してプロットする。

この実験を行わない場合、本文の主張は「BAM全体の逐次走査を避ける」に留め、「BAMサイズから独立」といった定量的表現は避ける。

---

## 15. 公平性を保つための規則

1. 同じBAMと同じQNAMEファイルを全ツールに使う。
2. 出力形式はheaderなしSAMに統一する。
3. 性能測定では出力先を `/dev/null` に統一する。
4. 正しさの検証は別工程で行い、その時間を検索時間に含めない。
5. インデックス構築時間には、BAM走査、ソート、最終インデックス書き出しを含める。
6. 古いインデックスの削除、checksum計算、問い合わせ集合生成は測定時間に含めない。
7. release buildを使用する。
8. 失敗したrunを黙って除外しない。exit statusとエラーを保存する。
9. 外れ値を除外する場合は、同時にOS updateや別ジョブが動いたなど、客観的理由を記録する。
10. 各ツールの設定を結果TSVに完全に残す。

---

## 16. 推奨ディレクトリ構成

```text
benchmarks/
├── README.md
├── env/
│   ├── system.txt
│   ├── software_versions.txt
│   └── commands.txt
├── data/
│   ├── datasets.tsv
│   └── queries/
│       ├── longread/
│       │   ├── present/
│       │   └── absent/
│       └── shortread/
├── scripts/
│   ├── capture_environment.sh
│   ├── make_query_sets.py
│   ├── generate_synthetic_bam.py
│   ├── benchmark_index.sh
│   ├── benchmark_query.sh
│   ├── verify_outputs.sh
│   └── summarize.py
├── raw/
│   ├── index_runs.tsv
│   ├── query_runs.tsv
│   └── time/
├── derived/
│   ├── index_summary.tsv
│   ├── query_summary.tsv
│   └── break_even.tsv
└── figures/
    ├── query_scaling.pdf
    ├── index_building.pdf
    └── qname_length_index_size.pdf
```

すべてのコマンドを手入力するより、shell scriptまたはPython scriptから実行し、実際に実行した完全なコマンドラインを `commands.txt` に追記する。

---

## 17. 論文に載せる結果

### 表1: データセットと測定環境

| Dataset | Platform | BAM size | Records | Mean QNAME length | Storage |
|---|---|---:|---:|---:|---|

### 表2: インデックス構築

| Dataset | Tool | Threads | Time | Peak RSS | Index size | Bytes/record |
|---|---|---:|---:|---:|---:|---:|

### 図1: 問い合わせQNAME数と検索時間

- 横軸: QNAME数、対数
- 縦軸: wall time、対数
- qbix query-order
- qbix bam-order
- Atlantool
- samtools

### 図2: QNAME長とインデックスサイズ

- 横軸: QNAME長
- 縦軸: index bytes per BAM record
- qbix
- Atlantool
- bri（測定できた場合）

### 補助表または補助図

- cold-cache結果
- present/absent比較
- スレッド数による構築時間
- メモリ上限試験

本文には主要な表2点、図2点程度を載せ、全runのTSVと追加図をリポジトリに置く。

---

## 18. 派生評価: インデックス作成コストの回収点

各query countについて、インデックスを作る価値が出る問い合わせ回数を概算できる。

```text
break-even batches
  = index_build_time
    / (samtools_query_time - indexed_query_time)
```

分母が正の場合のみ計算する。これは追加測定を必要とせず、既存の中央値から算出できる。

ただし、index buildとqueryでキャッシュ条件が異なる場合は概算値として扱う。論文では「この環境とデータセットにおける回収点」と限定し、一般的な閾値として主張しない。

---

## 19. 実行順序

1. qbixの測定対象コミットを固定する。
2. software version、hardware、OS、storage情報を保存する。
3. BAMを検証し、データセット情報を記録する。
4. qbix、Atlantool、briのインデックスを一度作り、正しく検索できることを確認する。
5. 問い合わせQNAME集合を固定seedで作る。
6. 全ツールの出力一致を確認する。
7. インデックス構築を3回測定する。
8. 通常キャッシュ条件で検索スケーリングを測る。
9. 必要な2条件だけcold-cache測定を追加する。
10. 合成BAMでQNAME長とインデックスサイズを測る。
11. raw TSVからsummary TSVと図を生成する。
12. 論文本文の性能に関する表現を、実測結果の範囲に合わせて修正する。

---

## 20. 最小実行セット

時間やディスクに制約がある場合、まず次だけを行う。

- 公開または再現可能なロングリードBAM 1本
- qbix、Atlantool、samtools
- インデックス構築3回
- present QNAME: 1、10、100、1,000、10,000件
- 5 replicate
- qbix query-orderとbam-order
- 通常キャッシュ条件
- 合成BAM: QNAME長16、36、64、128、各100万レコード
- 出力一致確認

これで、構築コスト、検索速度、問い合わせ数に対する挙動、qbix内の出力順序の効果、固定長インデックスの意義を一通り示せる。

short-read BAM、cold-cache、並列化、メモリ上限、BAMサイズ依存性は、その結果を見て追加する。

---

## 21. 結果の解釈上の注意

- 1台のPCで得た絶対時間を、すべての環境で再現される値として扱わない。
- 同一PC上での相対比較とスケーリングを中心に議論する。
- SSD性能とOS page cacheが検索時間に大きく影響するため、storageとcache条件を必ず明記する。
- QNAME数が増えると、インデックス方式でも出力レコード数とBAMシーク数が増える。検索時間が完全に一定になるとは主張しない。
- `--bam-order`は候補を収集・ソートするため、少数問い合わせでは `--query-order` より遅い可能性がある。交差点自体が有用な結果である。
- qbixの固定長インデックスはQNAME長に依存しないが、レコード数には比例する。
- Atlantool、bri、qbixは保存形式と検索アルゴリズムが異なるため、単一の結果だけで全面的な優劣を断定しない。
