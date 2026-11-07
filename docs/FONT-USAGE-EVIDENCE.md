# Font-use evidence / 書体使用の実測記録 / 字体使用实测记录

🟢 [Done] PDF inspection now records text operations observed when MuPDF replays
its parsed display lists, including annotation/widget content. This differs from
the declared-resource inventory: an unused declared font contributes no glyph
callbacks. Evidence is stored at `fontPolicy.runtimeEvidence` in inspection reports
and in the hash/run-bound source-validation report. The existing authenticated
`GET /api/v1/jobs/{id}/source-validation` API exposes it after a successful worker
run. Existing report persistence and receipt binding also preserve this field.

日本語: PDF事前検査で、MuPDFの解析済みDisplayListを再生した際のテキスト操作を記録します。未使用の宣言フォントは字形件数に含まれません。原稿検査および原稿ハッシュ・ワーカー実行に結び付いた検証結果の`fontPolicy.runtimeEvidence`へ保存します。既存の認証付き検証結果APIから参照でき、再起動や結果証明への結び付けも維持します。

简体中文: PDF预检记录MuPDF重放已解析DisplayList时的文本操作。未使用的声明字体不计入字形回调。记录保存于原稿检查及与源哈希、工作器执行绑定的验证结果的`fontPolicy.runtimeEvidence`，通过现有认证验证结果API读取，并保留重启持久性及结果证明绑定。

The profile is `org.jrip.mupdf-font-usage-v1`, with `engine: "mupdf"` and
`phase: "pdf-inspection"`. These values remain explicit when Ghostscript is the
selected raster engine: inspection observations do not certify Ghostscript's
rendered font choice or the later TIFF. The evidence contains no extracted source text, Unicode
codepoint sequences or text positions. Glyph identifiers are opaque engine values,
not a portable character encoding; a sample can still reveal which glyphs were
used when combined with the original font. The API retains its existing access
policy for source-validation metadata.

日本語: Ghostscriptで最終描画する場合も、記録元はMuPDFの事前検査と明示します。Ghostscriptの実描画や後続TIFFで同じ書体が使用された証明にはしません。Unicode本文・コードポイント列・文字位置は保存しません。字形IDはエンジン固有で、元の書体と組み合わせれば使用字形を特定できる場合があります。

简体中文: 即使最终渲染选择Ghostscript，也明确标记记录来自MuPDF预检，不证明Ghostscript实际渲染或后续TIFF使用相同字体。不保存Unicode正文、码点序列或文字位置。字形ID属于引擎内部标识，与原字体结合时仍可能识别所用字形。

| Field | Meaning |
| --- | --- |
| `pageCount` | Number of inspected source pages, before selection. |
| `textOperations` | Text device callbacks: fill, stroke, clip, clipStroke, ignore. |
| `glyphCallbacks` | Total Text walker glyph callbacks, including negative mapping entries. |
| `fonts[].name` | Engine-reported name, grouped by exact name; not a unique font program identity/hash. |
| `glyphInstances` / `negativeGlyphIds` | Nonnegative glyph entries / extra or virtual mapping entries. |
| `zeroGlyphIds` | Number of glyph ID 0 entries; a diagnostic count, not a missing-character verdict. |
| `horizontalGlyphs` / `verticalGlyphs` | Counts of callbacks by writing mode. |
| `firstPage` / `lastPage` / `pageCount` | First, last and distinct inspected pages using that reported name. |
| `operations` | Glyph callbacks attributed to each text operation kind. |
| `glyphIdSample` / `sampleTruncated` | Up to 16 unique, sorted, nonnegative engine IDs; additional IDs are not retained. |

Names shared by distinct programs are intentionally grouped; no font-program
identity is inferred. Fill-and-stroke can count the same logical character twice.
Clipped, ignored, transparent or off-page text can appear in the record. This is
not visible-ink coverage, text extraction, character completeness, shaping quality
or a count of sheets/characters printed. Outlined text represented as paths has
no text callback and contributes no font record.

日本語: 同名の異なるプログラムは同じグループになります。塗りと線の両方で同じ文字が二重に数えられる場合があります。クリップ・非表示・透明・ページ外の文字も含み得ます。可視インク、全文字の収録、組版品質や印刷文字数を保証せず、パスへ変換された文字は書体記録に現れません。

简体中文: 不同字体程序若报告同名则合并分组。填充及描边可能重复计数同一逻辑字符。被裁剪、忽略、透明或页外文本也可能计入。不保证可见墨迹、字符完整性、排版质量或实际打印字符数；转为路径的文字不产生字体记录。

## Bounds and validation / 上限と検査 / 上限及验证

Limits: 100 source pages, 100,000 text operations, 1,000,000 glyph callbacks,
64 reported font names, 128 UTF-16 name units, 16 sampled IDs per name and
32 KiB evidence JSON. The full inspection retains its existing 64 KiB output cap.
An exceeded bound rejects inspection; a partial or truncated glyph-count report
is never labelled complete. Only the ID sample is allowed to truncate explicitly.
Rust validates schema, engine/phase, original page count, unique names, sorted
samples, counter sums, operation totals and checked arithmetic before accepting
or persisting evidence. Legacy stored reports without this field are unchanged.

日本語: 上限超過時は検査を拒否し、部分的な件数を完全な結果として保存しません。字形ID標本だけは明示的に打ち切れます。Rustで工程・エンジン・元ページ数・件数整合性・演算オーバーフロー等を検査します。旧レポートへ未計測の値を補いません。

简体中文: 超限时拒绝检查，不将部分计数保存为完整结果。仅字形ID样本可明确截断。Rust验证阶段、引擎、原页数、计数一致性及运算溢出等。不向旧报告补入未测量值。

## Verification / 検証 / 验证

```sh
cargo test -p rip-core font_usage --locked
cargo test -p rip-storage source_validation --locked
cargo test -p rip-interpreter --test container inspection_records_executed_font_names_glyphs_and_vertical_modes --locked -- --ignored
cargo test -p jrip-server --test source_validation --locked -- --ignored --test-threads=1
```

The 12-page owned punctuation PDF produces two reported font names, six glyph
callbacks per font, three horizontal and three vertical, with six distinct source
pages per name. Removing only its text-show operators preserves the declaration
inventory but produces no runtime font records. Storage tests reject malformed
engine/counter evidence and verify persistence after reopening the database. Real
MuPDF and Ghostscript jobs also verify nonempty evidence through the authenticated
API and a reopened database, retaining MuPDF inspection provenance for both engines.

The new worker requires this evidence from its inspection image. Rebuild `adapters/mupdf/Dockerfile` with the worker upgrade; old stored reports remain readable without fabricated new fields.
日本語: ワーカー更新時はMuPDF検査イメージも再構築してください。旧保存レポートは新しい値を補わず参照できます。
简体中文: 升级工作器时须重建MuPDF检查镜像。旧报告仍可读取，不伪造新字段。

The implementation uses [MuPDF Text walking](https://mupdf.readthedocs.io/en/1.28.1/reference/javascript/types/Text.html)
with the pinned bookworm mutool API: its devices accept callback objects rather
than requiring the newer `Device` constructor. The actual image is tested.

🟠 [Next] Font-use evidence from actual rasterization, Ghostscript/PS/EPS usage,
program-to-rendered-glyph association, practical multilingual comparisons and OpenType shaping.
日本語: 実ラスタライズ時・Ghostscript／PS／EPSの使用記録、実行字形とプログラムの対応、実用多言語比較・OpenType組版は未完了です。
简体中文: 实际栅格化、Ghostscript／PS／EPS字体使用记录、执行字形与程序关联、实际多语言比较及OpenType排版仍未完成。

Declared embedded programs now have separate [binary identities](EMBEDDED-FONT-IDENTITY.md); these are not inferred from text-use names.
日本語: 宣言プログラムのバイナリ識別も別途参照できます。使用名からハッシュを推測しません。
简体中文: 声明程序另有二进制标识，不根据使用名称猜测哈希。
