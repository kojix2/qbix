# qbix ベンチマーク用公開データ候補

## 目的

qbix論文の実データベンチマークに使用できる、公開・再取得可能な
long-read BAM候補を記録する。主な候補はGenome in a Bottle（GIAB）の
HG002で、PacBio HiFiとOxford Nanopore Technologies（ONT）の両方を利用できる。

全ゲノムBAMは数十GB以上あるため、quick benchmarkでは座標ソート済みBAMから
chr21など一つの染色体を抽出し、500 MB〜1.5 GB程度のsubsetを作ることを想定する。

## 第一候補: HG002 PacBio HiFi Revio

### データ

- Sample: GIAB HG002 / NA24385
- Platform: PacBio Revio HiFi
- Reference: GRCh38-GIABv3
- Coverage: 48×
- BioProject: PRJNA1028149
- BAM size: 約70 GB
- BAM index: 公開済み
- checksum: 公開済み

公式ディレクトリ:

<https://ftp.ncbi.nlm.nih.gov/ReferenceSamples/giab/data/AshkenazimTrio/HG002_NA24385_son/PacBio_HiFi-Revio_20231031/>

対象BAM:

```text
HG002_PacBio-HiFi-Revio_20231031_48x_GRCh38-GIABv3.bam
HG002_PacBio-HiFi-Revio_20231031_48x_GRCh38-GIABv3.bam.bai
```

BioProject:

<https://www.ncbi.nlm.nih.gov/bioproject/PRJNA1028149>

### 利点

- NIST/GIABの標準試料で、出典を説明しやすい。
- BAM、BAI、checksumが同じ公式FTPに揃っている。
- 既にGRCh38へアラインされ、座標ソートされている。
- HiFi long-readであり、qbixの想定用途に合う。
- HTTP range requestが利用できれば、全BAMを取得せず染色体subsetを作れる。

### subset作成例

```bash
BAM_URL=https://ftp.ncbi.nlm.nih.gov/ReferenceSamples/giab/data/AshkenazimTrio/HG002_NA24385_son/PacBio_HiFi-Revio_20231031/HG002_PacBio-HiFi-Revio_20231031_48x_GRCh38-GIABv3.bam

samtools view \
  -bh \
  -o HG002.PacBio-HiFi.chr21.bam \
  "$BAM_URL" \
  chr21

samtools index HG002.PacBio-HiFi.chr21.bam
```

実行前に、使用するHTSlib/SAMtoolsがリモートBAMと隣接するBAIを正しく参照できるか
確認する。リモートrange accessが利用できない場合は、全BAMの取得が必要になる。

## 第二候補: HG002 Oxford Nanopore Kit14

### データ

- Sample: GIAB HG002 / GM24385
- Platform: Oxford Nanopore PromethION
- Chemistry: R10.4.1系Kit14
- Basecalling: HACまたはSUP
- Storage: ONT Open Data on AWS

ONTのGIAB 2025.01公式案内:

<https://epi2me.nanoporetech.com/giab-2025.01/>

公開S3:

```text
s3://ont-open-data/giab_2025.01/
```

2025.01リリースはPOD5を含む一次データの公式公開先として有用だが、qbixの
ベンチマークには、既にアラインされた2023.05解析データの方が扱いやすい。

2023.05解析データ:

```text
s3://ont-open-data/giab_2023.05/analysis/hg002/sup/
s3://ont-open-data/giab_2023.05/analysis/variant_calling/hg002_sup_60x/
```

アライン・haplotag済みBAM候補:

```text
s3://ont-open-data/giab_2023.05/analysis/variant_calling/hg002_sup_60x/hg002.haplotagged.bam
```

公開オブジェクトのブラウザ:

<https://42basepairs.com/browse/s3/ont-open-data/giab_2023.05/analysis/variant_calling/hg002_sup_60x?file=hg002.haplotagged.bam>

### 利点

- long-read QNAME検索というqbixの用途に合う。
- supplementary alignmentを含む現実的なデータである。
- `MM`、`ML`、haplotagなど、大きなoptional tagを含む可能性が高い。
- PacBioより大きく複雑なBAMレコードに対する実用的な確認になる。

### 注意点

- S3アクセス方法を固定する必要がある。
- HAC/SUP、coverage、解析段階の異なる複数BAMが存在する。
- 使用したflow cell、basecalling条件、リリース、BAMオブジェクトを明記する。
- subset作成前に、対象BAMに対応するBAIの存在とパスを確認する。
- BAMがmodified-base tagを保持しているか、`samtools view`で確認する。
- 2025.01のPOD5から再basecall・再alignする方法は再現可能だが、qbix評価には
  前処理が過大なので、主ベンチマークでは既存alignmentを優先する。

## 推奨する論文構成

### Quick benchmark

まず次の1データセットで測定系を確立する。

```text
HG002 PacBio HiFi Revio GRCh38, chr21 subset
```

選定理由は、出典、BAM、BAI、checksumが一か所に揃い、第三者が再取得しやすい
ためである。

### Full benchmark

可能なら次の2データセットを使用する。

1. HG002 PacBio HiFi Revio chr21
2. HG002 ONT SUP chr21

同じsampleと同程度のゲノム領域を使うことで、プラットフォーム差を示しながら、
試料由来の差を小さくできる。

### 合成データ

合成BAMは実データの代替ではなく、次の設計特性を分離して確認する補助実験に使う。

- QNAME長とインデックスサイズ
- 1 QNAMEあたりのレコード数
- optional tag量
- present/absent QNAME

小さな合成fixtureはベンチマーク基盤の動作確認専用とし、性能結果には採用しない。

## subset使用時の解釈

染色体subsetでは、同じQNAMEを持つ別染色体上のレコードが除外される可能性がある。
ただしqbixは「入力BAM内に存在する同名レコード」を取得するツールなので、subset
内部でqbixとSAMtoolsの出力集合が一致すれば、正しさ比較として成立する。

論文では、全ゲノムBAMではなく染色体subsetを使用したこと、抽出コマンド、元BAMの
URL、元BAMとsubsetのサイズ・レコード数を明記する。

## 測定前に確定する項目

- [ ] 使用するplatform（PacBio、ONT、または両方）
- [ ] 元BAMの完全なURL/S3 URI
- [ ] 元BAMのrelease、checksum、BAI
- [ ] subset対象染色体
- [ ] subsetのBAMサイズとalignment record数
- [ ] `@HD SO:coordinate`の確認
- [ ] QNAME数とprimary/secondary/supplementaryの構成
- [ ] ONTの場合はbasecaller、model、modified-base tagの有無
- [ ] subset生成コマンドと生成物のSHA-256
