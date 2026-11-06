# Owned font corpus / 自作フォント原稿 / 自制字体原稿

`font-quality.pdf` is generated solely from the outline coordinates in `generate.py`.
No system font, third-party font file, customer document, or network asset is used.
The two glyph designs resemble 日 and 本; 日 includes a deliberately nonstandard ink
island inside a counter, so a standard fallback font cannot silently pass the probes.
These are test shapes, not production typefaces or typography references.

日本語: 外部フォントを使わず、座標から自作した日・本の字形です。日のカウンター内の固有形状で、標準代替フォントへの置換を検出します。商用組版品質や実際の日本語書体の互換性を証明する原稿ではありません。

简体中文: 不使用外部字体，直接根据坐标构建日、本字形。日的字腔中包含特有墨块，用于检测标准回退字体替换。测试图形不用于生产排版，也不证明真实日文字体兼容性。

| Page | Embedded program | Encoding exercised |
| --- | --- | --- |
| 1 | TrueType subset, FontFile2 | Symbol cmap 3/0, F041/F042 → owned glyphs; single-byte PDF codes 41/42. |
| 2 | Same TrueType subset in CIDFontType2 | Identity-H two-byte codes 0007/0009, explicit non-identity CIDToGIDMap; CID values differ from Unicode and glyph IDs. |
| 3 | Name-keyed CFF subset, FontFile3 / Type1C | Encoding Differences to uni65E5/uni672C. |
| 4 | TrueType subset in CIDFontType2, Identity-V | Explicit W2: -1200 advance, (500,1000) origin; DW2 deliberately differs (-1000). |
| 5 | CID-keyed CFF subset, FontFile3 / CIDFontType0C | ROS Adobe/Identity, sparse charset CID 9/7, one Font DICT and FDSelect, Identity-H. |
| 6 | Same CID-keyed CFF, Identity-V | DW2 supplies -1200 advance and 1000 origin height; default horizontal origin is half the glyph width. |

The master font has .notdef, 日, 本 and an unused 語 test shape. fontTools actually
subsets it to .notdef plus 日/本 and asserts that the unused glyph was removed.
Both Unicode and Symbol mappings needed by the fixture are retained. ToUnicode
maps are supplied, but the current regression validates rendering, not text extraction.
Font timestamps are fixed for reproducibility; PDF offsets/lengths are generated exactly.

日本語: 元の4グリフからfontToolsで実際にサブセット化し、未使用グリフの除去を検査します。CIDはUnicodeやGIDと異なる番号にし、対応表の処理を検証します。ToUnicodeも含めますが、現在の試験対象は描画で、文字抽出ではありません。

简体中文: fontTools将原有四个字形实际裁剪为三个，并检查未使用字形已删除。CID与Unicode、GID使用不同编号来验证映射。包含ToUnicode，但当前测试仅验证渲染，不验证文本提取。

```sh
python3 -m venv /tmp/jrip-fonts
/tmp/jrip-fonts/bin/pip install -r tests/corpus/fonts/requirements.txt
/tmp/jrip-fonts/bin/python tests/corpus/fonts/generate.py
/tmp/jrip-fonts/bin/python tests/corpus/fonts/generate.py --check
cargo test -p rip-interpreter --test render_quality --locked -- --ignored
```

Regeneration requires pinned fontTools; rendering tests require only the checked-in PDF
and the existing local MuPDF/Ghostscript Docker images. CI checks reproducibility
before running the ignored Docker tests. `--check` does not overwrite the fixture.

🟢 [Done] RGB/gray rendering with distinct ink, counters and sentinel probes on both engines. Six pages × two colors × two engines produce 24 font-page rasters. Vertical probes check both glyph origins, custom advance, and whitespace between glyphs; using -1000 instead of -1200 fails.

日本語: 6ページ×2色空間×2エンジンの24ページを検証します。CIDベースCFFはFDArray／FDSelectと疎なCID charsetを持ちます。縦書きはTrueTypeの明示W2とCFFの既定DW2の両経路で原点・送り・文字間の空白を検査します。

简体中文: 验证6页×2色彩空间×2引擎共24页。CID型CFF包含FDArray／FDSelect及稀疏CID charset。分别通过TrueType显式W2及CFF默认DW2检查竖排原点、字距及字形间空白。

🟠 [Next] Real-world fonts, OpenType vertical shaping and explicit fallback behavior.

The vertical case tests PDF vertical metrics, not OpenType vert/vrt2 shaping, automatic punctuation
rotation, ruby, multiple Font DICT selection, variable fonts or all CJK vertical typography.
CID CFF is converted from the already-subset owned name-keyed program; no external
CID font is embedded and no claim is made about fontTools subsetting arbitrary CID fonts.

日本語: 縦送りの検査であり、OpenTypeの縦字形置換・句読点回転・ルビ・複数FD選択・任意の日本語組版は対象外です。CFFは自作のサブセットからCID形式へ変換しています。

简体中文: 仅检查PDF竖排度量，不覆盖OpenType竖排字形替换、标点旋转、注音、多FD选择或所有中日文排版。CFF由已裁剪的自制字体转换为CID形式。

⭕️ [Pending] Customer font/document acceptance and paper output.

## Explicit punctuation mapping / 明示的な句読点対応 / 显式标点映射

🟢 [Done] `punctuation-quality.pdf` is a separate, deterministic 12-page fixture
from `punctuation.py`. Each embedded TrueType and CID CFF subset supplies comma,
period and opening bracket in horizontal and vertical forms. Embedded Encoding
CMaps map the same Unicode-valued PDF codes to distinct sparse horizontal/vertical
CIDs; the TrueType CIDToGIDMap and CFF charset resolve those to owned glyphs.
Vertical WMode and W2 specify the origin. Probes distinguish relocated punctuation,
period counters and the rotated bracket with an asymmetric sentinel.
Both engines render RGB and gray: 48 page rasters. The existing six-page fixture
is unchanged. The generator and `--check` cover both PDFs.

日本語: 独立した12ページの自作原稿で、読点・句点・開き括弧の横／縦字形を検証します。同じ文字コードを埋め込みCMapで異なるCIDへ対応させ、TrueTypeとCID CFFの両経路を確認します。位置・白抜き・括弧の回転と固有印を48ページの実描画で検査します。

简体中文: 独立的12页自制原稿验证顿号、句号及左括号的横竖排字形。内嵌CMap将相同字符代码映射到不同CID，分别验证TrueType及CID CFF。48页实际渲染检查位置、字腔及括号旋转和特有标记。

This exercises glyph selection already encoded by the PDF producer. It does not
exercise OpenType vert/vrt2 shaping, automatic line layout, ruby or font fallback.
日本語: PDF内で指定済みの字形選択の検査です。OpenTypeの自動縦組み・ルビ・代替フォントは対象外です。
简体中文: 检查PDF中已指定的字形选择，不覆盖OpenType自动竖排、注音或字体回退。

The worker now enforces the [PDF font policy](../../../docs/PDF-FONT-POLICY.md).
日本語: ワーカーはPDFの欠落書体方針を強制します。
简体中文: 工作器强制执行PDF缺失字体策略。
