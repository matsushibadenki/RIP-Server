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
| 3 | Name-keyed CFF subset, FontFile3 / Type1C | Encoding Differences to uni65E5/uni672C. This is not CID-keyed CFF. |

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

🟢 [Done] RGB/gray rendering with distinct ink, counters and sentinel probes on both engines.

🟠 [Next] CID-keyed CFF, complex real-world fonts and explicit fallback behavior.

⭕️ [Pending] Customer font/document acceptance and paper output.
