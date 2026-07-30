# qbix 20分ミニベンチマーク計画

## 目的

一般的なLinux PCで短時間に完走し、qbix論文に最低限必要な性能データを得る。
本計画は最終的な大規模評価ではなく、**小規模だが再現可能な予備ベンチマーク**である。

**20分に含めるもの:** 環境記録、正しさ確認、正式測定、結果ファイルの確認  
**20分に含めないもの:** ツールの導入・コンパイル、BAMのダウンロード、大きな元BAMからの切り出し

---

## 1. 今回示すこと

1. qbixインデックスの構築時間、ピークメモリ、サイズ
2. 少数QNAME検索がBAM全走査より速いか
3. QNAME数が増えたときの `--query-order` と `--bam-order` の違い
4. qbixとSAMtoolsが同じレコード集合を返すこと

今回は、あらゆるデータや計算機に対する一般的な優位性までは主張しない。

---

## 2. 比較対象

| 方法 | 役割 |
|---|---|
| qbix `--query-order` | デフォルトの検索方式 |
| qbix `--bam-order` | BAM内オフセット順に読み出す方式 |
| `samtools view -N` | インデックスを使わない全走査の基準 |

Atlantool、`bri`、cold-cache、複数データセット、合成BAM、並列化評価は後日に回す。

---

## 3. 測定用BAM

座標ソート済みBAMを1本だけ使う。

- ローカルSSD上に置く
- 目安は **500 MB〜1.5 GB**
- 搭載RAMの25%以下
- 3万個以上の異なるQNAMEを含む
- 可能ならNanoporeまたはPacBio BAM

大きなBAMしかない場合は、測定前に小さな座標順BAMを作る。

### 時間超過を防ぐ事前判定

正式測定前に各コマンドを1回試す。

- qbixのインデックス構築が **60秒超**
- `samtools view -N` による全走査が **15秒超**

どちらかに該当したら、反復数を減らすのではなくBAMをさらに小さくする。
この試行結果は正式結果に含めない。

---

## 4. 共通条件

- qbixはrelease buildを使用
- 全ツールを原則1スレッドに固定
- BAM、インデックス、一時ファイルは同じローカルSSD上
- 検索出力はheaderなしSAMとして `/dev/null` へ送る
- OSページキャッシュは明示的に消去しない
- 正式測定前にBAMを1回読み、warm-cache条件にそろえる
- 各正式条件を3回測定し、中央値と範囲を報告
- 測定中は重いバックグラウンド処理を止める

```bash
cargo build --release --locked
cat benchmark.bam > /dev/null
```

論文には次のように記載する。

> The BAM file was read once before benchmarking, and the operating-system page cache was not explicitly cleared between runs.

---

## 5. 環境とBAM情報の記録

```bash
date -Is
uname -a
cat /etc/os-release
lscpu
free -h
lsblk -o NAME,MODEL,ROTA,SIZE,TYPE,FSTYPE,MOUNTPOINTS
findmnt -T benchmark.bam

qbix --version
samtools --version

samtools quickcheck -v benchmark.bam
samtools view -H benchmark.bam > benchmark.header.sam
samtools view -c benchmark.bam > alignment_count.txt
stat -c '%s' benchmark.bam > bam_bytes.txt
```

本文に記載する項目:

- CPU型番と論理CPU数
- RAM容量
- Linux distributionとkernel
- SSD種別とファイルシステム
- 各ツールのversionまたはcommit
- BAMサイズ、alignment record数、platform
- 1スレッド・warm-cache条件であること

---

## 6. 問い合わせQNAME

次の3段階を測定する。

```text
1, 100, 10,000 QNAMEs
```

固定seedで3組の独立したQNAME集合を作る。

```text
rep1: 最大10,000件
rep2: 最大10,000件
rep3: 最大10,000件
```

各replicateの先頭から1件、100件、10,000件を切り出す。

```text
queries/rep1_n00001.txt
queries/rep1_n00100.txt
queries/rep1_n10000.txt
...
queries/rep3_n10000.txt
```

同一replicate内では入れ子にする。
QNAME抽出スクリプト、seed、各ファイルのSHA-256を保存する。

異なるQNAMEが3万件に満たない場合は、最大条件を1,000件に下げる。

---

## 7. 正しさの確認

**実験Aの3回目のインデックス構築が完了した後**、検索速度の正式測定に入る前に、`rep1_n10000.txt`について1回だけ確認する。

```bash
qbix get --query-order \
  -i benchmark.qbi \
  -f queries/rep1_n10000.txt \
  benchmark.bam > check.qbix-query.sam

qbix get --bam-order \
  -i benchmark.qbi \
  -f queries/rep1_n10000.txt \
  benchmark.bam > check.qbix-bam.sam

samtools view -@ 1 \
  -N queries/rep1_n10000.txt \
  benchmark.bam > check.samtools.sam
```

出力順序が異なるため、ソート後に比較する。

```bash
for f in check.*.sam; do
  LC_ALL=C sort "$f" | sha256sum
done

wc -l check.*.sam
```

確認項目:

- 出力件数が一致
- ソート後SHA-256が一致
- primary、secondary、supplementary alignmentが欠落していない

SAM表現の違いで一致しない場合は、QNAME、FLAG、RNAME、POS、CIGARなどを正規化して比較する。
正しさを確認できない状態では性能結果を採用しない。

---

## 8. 実験A: インデックス構築

### 測定対象

qbixインデックスを3回構築する。

記録値:

- wall-clock time
- user CPU time
- system CPU time
- peak RSS
- exit status
- 完成したインデックスの総バイト数

### qbix

```bash
for rep in 1 2 3; do
  rm -f benchmark.qbi
  rm -rf tmp/qbix
  mkdir -p tmp/qbix

  /usr/bin/time \
    -f '%e\t%U\t%S\t%M\t%x' \
    -o "results/index_qbix_rep${rep}.time" \
    qbix index \
      --bgzf-threads 1 \
      --sort-threads 1 \
      --memory 512M \
      --temp-dir tmp/qbix \
      -i benchmark.qbi \
      benchmark.bam
done
```

3回目に作成したインデックスを検索測定に使う。

### インデックスサイズ

```bash
stat -c '%s' benchmark.qbi
```

次も計算する。

```text
index bytes per alignment record
  = index bytes / alignment record count
```

qbixについては、実測値が概ね `16 × alignment records + header` と一致することを確認する。

---

## 9. 実験B: QNAME検索

3問い合わせ数 × 3replicate × 3方法、合計27コマンドを実行する。

### コマンド形式

qbix query order:

```bash
qbix get --query-order \
  --bgzf-threads 1 \
  -i benchmark.qbi \
  -f names.txt \
  -o /dev/null \
  benchmark.bam
```

qbix BAM order:

```bash
qbix get --bam-order \
  --bgzf-threads 1 \
  -i benchmark.qbi \
  -f names.txt \
  -o /dev/null \
  benchmark.bam
```

SAMtools:

```bash
samtools view -@ 1 -N names.txt benchmark.bam > /dev/null
```

各コマンドを `/usr/bin/time` で囲み、次をTSVへ保存する。

```text
tool
mode
query_count
replicate
elapsed_s
user_s
sys_s
max_rss_kb
exit_status
query_sha256
```

実行順をreplicateごとに変える。

```text
rep1: qbix-query → samtools → qbix-bam-order
rep2: samtools → qbix-bam-order → qbix-query
rep3: qbix-bam-order → qbix-query → samtools
```

同じQNAMEファイルを全ツールへ渡す。

---

## 10. 集計と論文用出力

各条件について3測定の以下を報告する。

- 中央値
- 最小値
- 最大値

有意差検定は行わず、生の3測定をTSVで公開する。

### 表1: インデックス構築

| Tool | Median time (s) | Min–max (s) | Peak RSS (MiB) | Index bytes | Bytes/record |
|---|---:|---:|---:|---:|---:|
| qbix | | | | | |

### 図1または表2: QNAME検索時間

| QNAMEs | qbix query-order | qbix BAM-order | samtools |
|---:|---:|---:|---:|
| 1 | | | |
| 100 | | | |
| 10,000 | | | |

図にする場合:

- 横軸: QNAME数（対数）
- 縦軸: wall-clock time（対数）
- 3系列を表示
- 必要なら最小値〜最大値を細いエラーバーで表示

この**表1枚＋図1枚**を本文に載せ、生データとスクリプトをリポジトリに置く。

---

## 11. 20分の時間配分

| 作業 | 目安 |
|---|---:|
| 環境・BAM情報 | 1分 |
| QNAME集合作成 | 1〜2分 |
| BAMのwarm-up | 1分以内 |
| インデックス構築3回 | 3〜4分 |
| 正しさ確認 | 1〜2分 |
| 検索27回 | 3〜5分 |
| サイズ測定・結果確認 | 1分 |
| **合計** | **10〜15分** |

時間を超えそうな場合は、正式測定前にBAMを縮小する。
正式測定に入った後は、3回反復や比較対象を途中で減らさない。

---

## 12. 論文での主張範囲

結果が支持する場合、次のように限定して述べる。

- このPCとこのBAMでは、qbixインデックスを報告した時間・メモリ・サイズで構築できた。
- このデータのwarm-cache条件では、少数QNAMEの取得でqbixがBAM全走査を避けたことによる時間差が観察された。
- 問い合わせ数に応じた `--query-order` と `--bam-order` の実測差を示した。
- 検証した問い合わせでは、比較ツールが同じレコード集合を返した。

次は断言しない。

- qbixは未測定のQNAME索引ツールより常に速い、または小さい
- 検索時間はBAMサイズに依存しない
- すべてのBAM、ストレージ、並列度で同じ傾向になる
- 10万件以上でも同じ結果になる

英語では `on the tested dataset`、`in this benchmark`、`under warm-cache conditions` を明記する。

---

## 13. 保存するファイル

```text
paper/work/
├── README.md
├── pixi.toml
├── pixi.lock
├── run_quick_benchmark.sh
├── benchmark.py
└── output/
    └── RUN_ID/
        ├── datasets.tsv
        └── DATASET_ID/
            ├── manifest.json
            ├── commands.jsonl
            ├── environment.txt
            ├── versions.txt
            ├── queries/
            │   ├── checksums.tsv
            │   └── rep*_n*.txt
            ├── index_runs.tsv
            ├── query_runs.tsv
            ├── correctness.tsv
            ├── index_summary.tsv
            ├── query_summary.tsv
            └── query_time.pdf
```

非公開データを使う場合はQNAMEファイルを公開せず、抽出スクリプト、seed、公開可能な代替データでの再現手順を示す。

---

## 14. 実行前チェックリスト

- [ ] BAMは座標ソート済み
- [ ] BAMはローカルSSD上で500 MB〜1.5 GB程度
- [ ] 異なるQNAMEが3万件以上ある
- [ ] インデックス構築1回が各60秒以内
- [ ] SAMtools全走査1回が15秒以内
- [ ] qbixはrelease build
- [ ] 全ツールを1スレッドに固定
- [ ] 3組の問い合わせを固定seedで作成
- [ ] 正しさを確認してから速度測定
- [ ] 生の測定値をTSVへ保存
- [ ] 論文の主張を1データセット・warm-cache条件に限定

---

## 15. 後日の拡張順序

1. 10万QNAMEを追加
2. QNAME長を変えた合成BAMを追加
3. ロングリードとショートリードを各1本に増やす
4. cold-cache条件を追加
5. `bri`を追加
6. BAMサイズとスレッド数のスケーリングを追加

最初の結果を見てから、一つずつ追加する。
