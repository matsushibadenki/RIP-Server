# Embedded font identities / 埋め込み書体の識別 / 内嵌字体标识

🟢 [Done] PDF font inspection attaches a bounded binary identity to each declared
`EMBEDDED` font. `fontPolicy.programIdentityProfile` advertises
`org.jrip.embedded-font-program-v1`; `fonts[].programIdentity` carries the result.
The identity is computed from the embedded FontFile/FontFile2/FontFile3 stream,
independently of the declared BaseFont name. Matching decoded bytes have the same
SHA-256 even when their compression differs. Different bytes remain distinct even
when the declared font name is identical.

日本語: PDFの宣言リソースにある埋め込み書体を、BaseFont名ではなくFontFile系ストリームの内容で識別します。圧縮形式が異なっても復号後のバイト列が同じなら同一SHA-256となり、同名でも内容が違えば区別します。

简体中文: PDF声明资源中的内嵌字体按FontFile流内容识别，不依赖BaseFont名称。压缩方式不同但解码字节相同，SHA-256相同；名称相同但内容不同则分别识别。

| Field | Meaning |
| --- | --- |
| `profile` | `org.jrip.embedded-font-program-v1` |
| `field` | `FontFile`, `FontFile2` or `FontFile3` |
| `encodedSha256`, `encodedBytes` | Hash and size of the stored stream bytes, before filters. These alone are not a decoded-program identity. |
| `status: HASHED` | `sha256` and `bytes` identify the decoded program bytes. |
| `status: UNAVAILABLE` | A coded `reason` is present; decoded `sha256` and `bytes` are absent. |

The same source snapshot is already hash-verified by the worker. Its report,
including these identities, is persisted with source SHA-256/run ID and returned
by `GET /api/v1/jobs/{id}/source-validation` under existing authorization. Reopening
the database preserves it. Inspection APIs also expose the identities. Legacy
stored reports without the profile remain readable; they gain no invented hashes.
New workers require the profile from the current inspection image. Rebuild both
RIP Docker images when upgrading the worker.

日本語: 検証済みスナップショットから計測し、原稿SHA-256・実行IDと結び付けて保存します。既存の認証付き原稿検証APIおよび検査APIから参照できます。旧保存結果へハッシュを後付けしません。ワーカー更新時は両RIPイメージも再構築してください。

简体中文: 从已验证快照计算，与源SHA-256及执行ID绑定持久保存，通过现有认证原稿验证及检查API读取。不为旧报告补造哈希。升级工作器时须重建两个RIP镜像。

## Decoding and bounds / 復号と上限 / 解码及上限

Unfiltered streams and Flate, ASCIIHex, ASCII85, RunLength and default LZW filters
are supported, including chains of up to four filters. Flate predictors use pinned
pypdf 6.21.2 with explicit decompression/work/row limits. Non-default LZW EarlyChange
and LZW predictors are deliberately unavailable because the pinned library does
not implement those semantics. Image/crypt filters are not invoked for font identity.
No external font file, external decoder process or system-font lookup is used.

日本語: 無圧縮・Flate・ASCIIHex・ASCII85・RunLength・既定LZWを扱い、最大4段のフィルターを復号します。Flate predictorは固定版pypdfの制限付き処理を使用します。未対応のLZW条件や画像・暗号フィルターを別の方式として推測せず、外部フォント・外部復号プロセス・OS書体検索も使用しません。

简体中文: 支持无压缩、Flate、ASCIIHex、ASCII85、RunLength及默认LZW，最多四层过滤器。Flate predictor使用固定版pypdf的有界处理。不猜测不支持的LZW参数、图像或加密过滤器，也不读取外部字体、调用外部解码进程或查询系统字体。

Each decoded filter stage is limited to 16 MiB. The inspection allows 64 MiB
of expanded stage-output bytes across distinct referenced streams. Failed decodes
conservatively charge the remaining per-stage allowance; intermediate filter outputs
also consume the total. ASCII85 shorthand expansion is checked before allocation. A shared FontFile reference is
processed and charged once, including horizontal/vertical resources sharing a
program. Distinct objects containing duplicate bytes can be conservatively charged
separately. Metadata caches retain identities, not decoded program buffers. The
existing 32 MiB input, font/resource traversal limits, 64 KiB inspection output,
container memory/CPU/time limits remain in force.

`UNAVAILABLE` reasons include `PROGRAM_BYTE_LIMIT`, `TOTAL_BYTE_LIMIT`,
`DECODE_LIMIT`, `FILTER_CHAIN_LIMIT`, `UNSUPPORTED_FILTER`,
`UNSUPPORTED_PARAMETERS`, `EMPTY_PROGRAM` and `DECODE_FAILED`. Unavailability
alone does not change admission: existing font policy, parse/content and render
checks still apply. A failed identity is never published as a complete decoded hash.
Rust validates profile, statuses, canonical hashes, field names, size limits and
unique-program totals before inspection acceptance or persistence.

日本語: 復号段ごとに16MiB、全ストリームの展開出力の合計64MiBまでです。失敗時も段の残余予算を消費し、中間出力も合計へ含めます。共有参照は一度だけ処理し、復号バッファをキャッシュしません。識別不能だけで描画可否を決めず、従来のフォント・構文・描画検査を維持します。Rustでも状態・ハッシュ形式・容量・総量を検査し、不完全な識別結果を完全なハッシュとして扱いません。

简体中文: 每层解码输出上限16MiB，所有流的展开输出合计64MiB；失败时也扣除该层剩余额度，中间输出计入总量；共享引用只处理一次，不缓存解码缓冲区。无法识别本身不决定渲染准入，保留原有字体、语法及渲染检查。Rust还验证状态、哈希格式、容量及总量，不将不完整结果当作完整哈希。

## Scope / 範囲 / 范围

These are binary stream identities, not normalized-outline equivalence or proof
of the font actually used for rasterization. Font timestamps/name changes can alter
the hash. PDF Encoding, ToUnicode, CIDToGIDMap, widths and other outside-the-stream
objects are covered by the source-document hash, not this program hash. Type3
procedures and standard-14 substitutions have no embedded-file identity. The
[MuPDF text-use report](FONT-USAGE-EVIDENCE.md) groups engine-reported names; it is
not automatically joined to these hashes when multiple programs share a name.
A hash does not certify validity, licensing, character coverage or visual quality.

日本語: バイナリストリームの識別であり、輪郭の同等性や実ラスタライズに使用された書体の証明ではありません。ストリーム外のCMap・幅情報等は原稿ハッシュで保持します。Type3や標準14の代替書体に架空のFontFileハッシュは付けません。同名プログラムと使用記録を自動で一意に結び付けず、字形品質・収録範囲等も認証しません。

简体中文: 标识对象是二进制流，不证明轮廓等价或实际栅格化所用字体。流之外的CMap、宽度等由源文档哈希覆盖。不为Type3及标准14替代字体制造FontFile哈希，不自动将同名字体使用记录唯一绑定至程序，也不认证字形质量或字符覆盖。

## Verification / 検証 / 验证

```sh
python -m unittest discover -s adapters -p 'test_font_*.py' -v
cargo test -p rip-core font_programs --locked
cargo test -p rip-storage source_validation --locked
cargo test -p rip-interpreter --locked -- --ignored --test-threads=1
cargo test -p jrip-server --test source_validation --locked -- --ignored --test-threads=1
```

Tests cover equivalent plain/compressed/multi-filter bytes, same-name different
programs, Flate predictors, expansion limits, unavailable results, shared references,
invalid metadata and total limits. Real TrueType/CID CFF inspection and both engines'
worker/API/reopen tests verify identities persist with source/run provenance.

🟠 [Next] Raster-stage program/glyph association, Ghostscript/PS/EPS runtime use,
Type3 graph identities and additional decoder parameter coverage remain unfinished.
日本語: ラスター工程でのプログラム・字形対応、Ghostscript／PS／EPS実行時記録、Type3グラフ識別・復号条件拡充は未完了です。
简体中文: 栅格阶段程序与字形关联、Ghostscript／PS／EPS运行记录、Type3图标识及更多解码参数仍未完成。
