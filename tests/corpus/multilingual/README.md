# Multilingual production proof / 多言語業務原稿 / 多语言业务原稿

🟢 [Done] Three A4-sized pages (595 × 842 pt): a Japanese manufacturing note,
a Simplified Chinese inspection sheet and a mixed-language packing list. All text
is fixture content, with no physical-print authorization. English, Japanese and
Simplified Chinese use renamed, embedded Noto CJK regional subsets. The JP CFF
has eight Font DICTs and the SC CFF has six; the corpus exercises realistic CID
CFF programs, proportional Latin advances, CJK punctuation and regional glyphs.

日本語: 製造指示書・简体中文の検査票・混在言語の出荷票を含む3ページです。地域別の実書体をサブセット化して埋め込み、複数FDを持つCID CFF、欧文の比例送り、CJK字形・約物を検証します。顧客原稿や実印刷の承認ではありません。

简体中文: 三页包括日文制造指示、简体中文检验单及混合语言装箱单。内嵌地区字体子集，验证多FD的CID CFF、西文比例字距及中日文字形、标点。不是客户原稿或实体打印授权。

## Fonts / 書体 / 字体

`jp.otf` and `sc.otf` derive from indices 0 and 2 of the existing Debian bookworm
`NotoSansCJK-Regular.ttc`, source SHA-256
`b76b0433203017ca80401b2ee0dd69350349871c4b19d504c34dbdd80541690a`.
Their primary names and CFF Font DICT names were changed to JRIPCorpusCJKJP/SC.
Original copyright metadata is retained. The subsets remain under SIL OFL 1.1;
see [LICENSE.txt](LICENSE.txt), [SOURCE-COPYRIGHT.txt](SOURCE-COPYRIGHT.txt) and
[PROVENANCE.json](PROVENANCE.json) for original notices, hashes and versions.
The full collection is not distributed in this repository.

日本語: 元コレクションのハッシュを固定し、主書体名とFD名を変更して配布します。元の著作権表示を保持し、サブセットのライセンスはSIL OFL 1.1です。全コレクションは同梱しません。

简体中文: 固定原集合哈希，修改主字体名称及FD名称后分发，保留原版权信息。子集仍按SIL OFL 1.1许可，不分发完整集合。

## Oracle / 検査点 / 检查点

`probes.json` is computed from fontTools outline containment, not from either RIP's
output. Probe locations must be at least 40 font units inside/outside the outline,
and stay within the glyph advance to avoid touching the next character. At 288dpi,
tests check expected ink (sample ≤75 within a one-pixel hinting window) and
whitespace (center sample ≥180). This permits antialiasing/hinting differences;
it does not assert pixel identity or colorimetric accuracy. Page dimensions,
RGB/gray layout and clean page margins are also checked. A deliberately wrong
C→D CID map must be detected while the program and ToUnicode remain intact.

日本語: 検査点はRIPの正解画像から作らず、fontToolsで輪郭の内外から求めます。隣文字へはみ出す点を除き、288dpiでインク・白抜き・寸法・余白を検査します。黒点のみ1ピクセルのヒンティング差を許容します。CIDを誤らせた負例の検出も確認します。画素完全一致や測色品質の証明ではありません。

简体中文: 检查点由fontTools轮廓内外计算，不来自RIP参考图。排除越过字距、触及相邻字形的点，在288dpi检查墨迹、空白、尺寸及页边距。仅墨迹点容许一像素提示差异，并验证错误CID负例。不证明像素完全一致或测色准确性。

```sh
# Uses pinned tests/corpus/fonts/requirements.txt; no network/font lookup needed.
python tests/corpus/multilingual/generate.py --check
cargo test -p rip-interpreter --test multilingual_quality --locked -- --ignored
# Optional, for authorized source collection with the exact pinned hash:
python tests/corpus/multilingual/generate.py --prepare /path/to/NotoSansCJK-Regular.ttc
```

Normal regeneration reads only the checked-in subsets and checks their hashes.
CI reproduces both PDF and probes. MuPDF text extraction independently verifies
the three section titles; pypdf 6.21.2's extraction does not correctly handle this
custom multibyte Encoding CMap, so cross-reader text extraction is not certified.

日本語: 通常の再生成は同梱サブセットだけを読み、ハッシュを検査します。CIでPDFと検査点を再現します。MuPDFで3言語の見出しを抽出できることを確認済みです。pypdf 6.21.2はこの独自マルチバイトCMapの文字抽出に制約があり、全リーダーの文字抽出互換性は保証しません。

简体中文: 常规再生成仅读取同梱子集并验证哈希，CI重现PDF及检查点。已确认MuPDF能提取三个语言标题。pypdf 6.21.2对此自定义多字节CMap的文本提取有限制，不保证所有阅读器的提取兼容性。

🟠 [Next] Broader real documents, small-type/other-DPI coverage, OpenType shaping,
vertical production layouts, additional font families and print matching.
⭕️ [Pending] Customer-font acceptance and physical proofs require suitable originals
and a real device. These synthetic business documents do not replace that acceptance.
