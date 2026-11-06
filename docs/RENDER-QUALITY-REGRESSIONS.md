# Rendering quality regressions / 描画品質の回帰試験 / 渲染质量回归

🟢 [Done] Self-authored four-page PDF corpus and semantic pixel checks through the actual sandboxed MuPDF/Ghostscript adapters.

日本語: 自作の4ページPDFと、実際のMuPDF／Ghostscriptサンドボックス経由の画素検証を追加しました。エンジン出力から正解画像を作る方式ではなく、原稿の演算・図形に基づく期待値で判定します。

简体中文: 新增自制四页PDF，并通过实际MuPDF／Ghostscript沙箱验证像素。期望值来自原稿的运算及图形，而非引擎生成的黄金图像。

| Page / ページ / 页 | Content / 内容 | Checked output / 検証 / 验证 |
| --- | --- | --- |
| 1 | 50% red over blue / 青の上に赤50% / 蓝底上50%红 | RGB blue, purple overlap and red-over-white interior probes, tolerance ≤4/255. |
| 2 | Multiply / 乗算 / 正片叠底 | RGB cyan, black overlap, red; tolerance ≤4/255. |
| 3 | Process CMYK overprint vs knockout / プロセスCMYKのオーバープリントと抜き / CMYK叠印与挖空 | Cyan retained beneath yellow with OPM=1; cyan cleared in knockout; tolerance ≤2/255. |
| 4 | Embedded Type3 日／本 glyphs / 自作埋め込みグリフ / 自制嵌入字形 | Distinct ink and hole/background probes in each glyph; tolerance ≤3/255. |

Both engines render all four pages in RGB and CMYK at 72 dpi. Each output must preserve page count, 72×72 geometry, exact 8-bit color layout and complete sample lengths. Interior probes avoid antialias boundaries. Output order matters because each page has different expectations. Type3 glyph paths, PDF objects and ToUnicode mappings are authored in the repository; no external font binaries or redistributed customer documents are included.

日本語: 両エンジンでRGB／CMYKの4ページを72dpiで描画し、ページ数・72×72寸法・8ビット色配置・画素数・順序を検査します。検査点はアンチエイリアス境界を避けています。Type3の字形パス・PDF構造・ToUnicodeをリポジトリ内で自作し、外部フォントや顧客原稿は同梱しません。

简体中文: 两个引擎分别以RGB／CMYK和72dpi渲染四页，检查页数、72×72尺寸、8位色彩布局、样本长度及顺序。采样点避开抗锯齿边缘。Type3字形路径、PDF结构及ToUnicode均在仓库中自制，不包含外部字体或客户原稿。

## Run / 実行 / 运行

```sh
# Deterministically regenerate the checked-in synthetic PDF.
python3 tests/corpus/generate_render_quality.py
# Existing adapters; no changes to production rendering flags.
docker build -f adapters/mupdf/Dockerfile -t jrip-mupdf:local .
docker build -f adapters/ghostscript/Dockerfile -t jrip-ghostscript:local .
cargo test -p rip-interpreter --test render_quality --locked -- --ignored
cargo test -p rip-interpreter --test container --locked -- --ignored
```

The normal workspace test run ignores the quality test when Docker is unavailable. Existing CI builds both images and runs `cargo test --workspace --locked -- --ignored`, which includes this test. Test sources and output directories are private temporary files, cleaned on exit. No printer is contacted and no source/color settings are changed.

日本語: 通常テストではDocker必須試験として除外し、既存CIの両イメージ構築後の `--ignored` 実行で検証します。入力・出力は終了時に削除される専用一時ディレクトリを使います。印刷機への送信や描画設定の変更は行いません。

简体中文: 普通测试因需要Docker而跳过；现有CI在构建两个镜像后的 `--ignored` 阶段运行此测试。输入输出使用结束后清理的私有临时目录，不联系打印机或修改渲染设置。

## Limits / 検証範囲 / 验证范围

These are targeted regression checks, not whole-page perceptual diffs, PDF conformance certification, ICC color proofing or customer-document acceptance. The Type3 case covers embedded Japanese outline glyph execution; it does **not** demonstrate CID/TrueType/CFF font compatibility, font fallback/subsetting, complex CJK typography, spot separations or every transparency interaction. RGB/CMYK appearance is not asserted to be mutually identical. The overprint expectation is checked on CMYK output, not inferred from an RGB screen preview.

日本語: 特定機能の回帰検査であり、全画面の知覚比較・PDF適合認証・ICC色校正・顧客原稿受入ではありません。日本語ケースは埋め込みType3字形の実行を対象とし、CID／TrueType／CFF、代替フォント・サブセット、複雑な日本語組版、特色分版の互換性を証明しません。RGB／CMYKの見た目の同一性は要求せず、オーバープリントはCMYK出力で検査します。

简体中文: 本测试针对特定功能回归，不等同于整页感知比较、PDF合规认证、ICC校样或客户原稿验收。日文字形案例仅覆盖嵌入Type3执行，不证明CID／TrueType／CFF、字体回退／子集、复杂日文排版或专色分版兼容性。不要求RGB／CMYK外观相同；叠印直接检查CMYK输出。

🟠 [Next] CID/TrueType/CFF and subset/fallback coverage; broader real-document comparisons. / CID等・サブセット・代替フォントと実用原稿の比較拡充。 / CID等字体、子集／回退及实际原稿比较扩展。

⭕️ [Pending] Real printer output and customer-document acceptance. / 実機出力・顧客原稿の受入。 / 实机输出及客户原稿验收。

## Embedded font programs / 埋め込みフォントプログラム / 嵌入字体程序

🟢 [Done] Added a separate [owned font corpus](../tests/corpus/fonts/README.md): simple Symbol-cmap TrueType, CIDFontType2 with an explicit non-identity CIDToGIDMap, and name-keyed CFF/Type1C. The TrueType and CFF source programs are actually subset from four glyphs to three using pinned fontTools; no external font files are read. The CID values intentionally differ from both Unicode and glyph IDs. The six-page corpus is rendered by both engines in RGB and gray (24 font-page rasters); positive ink, two counters, background and a nonstandard sentinel detect blank output, swapped glyphs, missing glyph boxes and fallback substitution. Normal CI verifies that generator output exactly matches the checked-in PDF.

日本語: 自作原稿で、Symbol cmap付きTrueType、非恒等CIDToGIDMap付きCIDFontType2、名前ベースCFF／Type1Cを検証し、下記のCID CFF・縦送りへ拡張しています。fontToolsで実際に4→3グリフへサブセット化し、外部フォントは読みません。両エンジンでRGB／グレーを描画し、白黒の検査点と固有形状で欠落・誤対応・代替フォント置換を検出します。CIで生成PDFの一致も検査します。

简体中文: 自制原稿验证带Symbol cmap的TrueType、非恒等CIDToGIDMap的CIDFontType2及名称型CFF／Type1C，并扩展至下述CID CFF及竖排度量。fontTools实际将四个字形裁剪为三个，不读取外部字体。两个引擎分别渲染RGB／灰度，通过墨块、字腔、背景及特有形状检测缺失、错误映射与回退替换。CI同时验证生成PDF一致性。

This extends the earlier Type3-only coverage. OpenType shaping, arbitrary customer
fonts and deliberate missing-font fallback policies remain outside the corpus.
The sentinel verifies use of these embedded programs; it does not certify a
fallback font's suitability. Text extraction is not asserted.

日本語: OpenType組版・任意の顧客書体・欠落フォントの代替方針・文字抽出は対象外です。

简体中文: 不覆盖OpenType排版、任意客户字体、缺失字体回退策略及文本提取。


## CID CFF and vertical metrics / CID CFFと縦送り / CID CFF及竖排度量

🟢 [Done] The owned font corpus now has six pages. Added CID-keyed CFF with ROS,
sparse CID charset (7/9 rather than GIDs 2/1), FDArray/FDSelect and FontFile3
CIDFontType0C embedding. Added Identity-V for TrueType and CID CFF. The TrueType
page uses explicit W2 with -1200 advance while DW2 differs; the CFF page uses
DW2 with -1200 advance. Pixel probes test glyph origin offsets, advance and
inter-glyph whitespace, as well as the existing owned sentinel. RGB and gray on
both engines cover 24 font-page rasters. Regeneration remains deterministic and
is checked by the existing CI fixture step.

日本語: 自作フォント原稿を6ページへ拡張しました。CIDベースCFFの疎なCID対応・FDArray／FDSelectと、TrueType／CID CFFのIdentity-Vを実描画で検査します。W2明示とDW2既定の送り量を別々に扱い、原点・字間の空白・固有字形をRGB／グレーの両エンジンで確認します。

简体中文: 自制字体原稿扩展为六页，实际检查CID型CFF的稀疏CID映射、FDArray／FDSelect及TrueType／CID CFF的Identity-V。分别验证显式W2及默认DW2的字距，并在两个引擎的RGB／灰度输出中检查原点、字形间空白及特有形状。

This closes the synthetic CID-CFF/vertical-metrics coverage gap, not the broader
vertical typography or customer-font acceptance work. vert/vrt2 substitution,
automatic punctuation shaping, multiple Font DICTs and deliberate missing-font fallback
remain outside this corpus.

## Explicit punctuation selection / 句読点の明示的な字形選択 / 标点显式字形选择

🟢 [Done] The separate 12-page `punctuation-quality.pdf` exercises embedded
horizontal and vertical CMaps, sparse CID mappings, TrueType/CID CFF programs,
vertical origins, comma/period placement and bracket rotation. Semantic probes
check ink, counters, wrong-form whitespace and an asymmetric bracket sentinel
in RGB/gray on MuPDF/Ghostscript (48 page rasters). No external fonts are used.
See the [corpus documentation](../tests/corpus/fonts/README.md) for regeneration.

日本語: 横／縦CMapによる句読点・括弧の明示的な字形選択を48ページの実描画で確認します。既存6ページ原稿は維持し、独立した12ページ原稿を追加しました。

简体中文: 通过48页实际渲染验证横竖排CMap指定的标点及括号字形选择。保留原六页原稿，另增独立十二页原稿。

🟠 [Next] OpenType vert/vrt2 shaping, deliberate missing-font policy and practical document comparison remain separate unfinished work.
日本語: OpenType自動縦組み、欠落フォント方針、実用原稿の比較は未完了です。
简体中文: OpenType自动竖排、缺失字体策略及实际原稿比较仍未完成。

## Multilingual production-style corpus / 多言語業務原稿 / 多语言业务原稿

🟢 [Done] Added a [three-page regional Noto CJK corpus](../tests/corpus/multilingual/README.md), with OFL notices, renamed JP/SC subsets and hash-pinned provenance. Font-outline containment generates independent ink/whitespace probes. Both engines render RGB/gray at 288dpi (12 positive page rasters); a wrong CID map must fail the same oracle. Actual page images are reviewed. This adds realistic multi-FD CID CFF coverage, not arbitrary-font or paper certification.

日本語: 実書体による3言語原稿を12ページ分描画し、独立した輪郭検査点で照合します。多FD CFFと誤CIDを検証しますが、全書体・実機品質の保証ではありません。

简体中文: 实际字体三语言原稿共渲染12页，以独立轮廓检查点比对，覆盖多FD CFF及错误CID。不保证任意字体或实体打印质量。

The corpus exposed an ordinary-page limit bug: dividing the per-file RLIMIT_FSIZE
by the maximum page count restricted the default 100-page worker to about 10 MiB
per file, blocking the uncompressed RGB PAM for A4 near 300dpi. The per-file limit
is now `min(job spool budget, 65 MiB)`, independent of page allowance. The existing
512 MiB memory cap, output tmpfs size, page count and streamed aggregate byte budget
remain in force. Downstream 64 MiB decoded sample limits are unchanged.

日本語: 既定の100ページ設定で1ファイルが約10MiBに制限され、A4のRGB出力が失敗する問題を修正しました。ファイル上限は65MiBとジョブspool上限の小さい方とし、総量・メモリー・ページ数の上限を維持します。

简体中文: 修复默认100页设置把单文件限制在约10MiB、导致A4 RGB输出失败的问题。单文件上限改为65MiB与作业spool上限中的较小值，并保留总量、内存及页数限制。
