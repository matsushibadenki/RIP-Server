# PDF font policy / PDFフォント方針 / PDF字体策略

🟢 [Done] The trusted PDF inspection gate applies `org.jrip.pdf-font-policy-v1`
before either RIP engine runs. REST, IPP and MCP jobs share this worker gate.

| Declared font | Result |
| --- | --- |
| Type1/MMType1/TrueType or Type0 with a descendant and an embedded program | Accepted; MuPDF subsequently parses page content and the program. |
| Type3 with CharProcs | Accepted as self-contained glyph procedures; subsequent content parsing still applies. |
| Exact standard 14 name in a non-embedded Type1 font | Accepted, explicitly reported as `STANDARD14_SUBSTITUTION`. |
| Other non-embedded font, including subset-prefixed standard names | Rejected before rendering. No arbitrary installed-font search is authorized. |
| Missing/ambiguous/empty/external program, unsupported subtype or exceeded limit | Rejected. |

Standard 14 means the Times, Helvetica and Courier families (four styles each),
Symbol and ZapfDingbats. It does not include Arial or arbitrary CJK fonts. The
existing pinned container font environment supplies engine substitutions; this is
not a promise of original outlines, cross-engine pixel identity or print matching.
Embed the intended program for reproducible typography. Type3 resource references
are inspected too; Type3 does not exempt nested missing fonts.

日本語: PDFは両RIPに共通のワーカー検査で、埋め込みプログラム付き書体とType3を受け付けます。非埋め込みType1は標準14書体の完全一致名に限り、代替使用を明示します。それ以外の欠落書体は描画前に拒否します。標準14の代替は原書体との字形一致を保証しません。正確な書体が必要な原稿ではフォントを埋め込んでください。

简体中文: PDF通过两个RIP共用的工作器检查，接受内嵌字体程序及Type3。非内嵌Type1仅允许名称完全匹配的标准14字体，并明确记录替代。其他缺失字体在渲染前拒绝。标准14替代不保证原始轮廓一致，需要准确字体的原稿应嵌入字体程序。

The report's `fontPolicy` contains `profile`, `accepted`, `scope`, `fonts` and
`standard14Substitution`. It is available in document inspection reports, and is
persisted with source hash/run-bound worker evidence at
`GET /api/v1/jobs/{id}/source-validation` (existing `jobs:read` authorization).
The source-validation API preserves the report across restart; legacy reports
without this field do not acquire a fabricated font assessment.

日本語: `fontPolicy`は原稿検査レポートと、原稿ハッシュ・ワーカー実行に結び付いた永続検証結果で参照できます。旧レポートへ判定を後付けしません。

简体中文: `fontPolicy`可在原稿检查报告及与源哈希、工作器执行绑定的持久验证结果中查看。不为旧报告伪造字体判定。

The inventory follows declared page resources, nested Form/Pattern resources,
annotation appearances and AcroForm resources, including inherited page resources.
Cycles are deduplicated. Limits are 32 MiB input, 100 pages, 8,192 object visits,
48 levels, 128 fonts and 128 characters per BaseFont name. It reads no external
font files and does not decode font streams; the existing MuPDF parser and
sandbox enforce the subsequent content/program checks. A declaration may be
unused, so an unused missing font is conservatively rejected as well.
This is not executed-glyph coverage or an assertion that every glyph is present.

日本語: ページ・フォーム・パターン・注釈外観・AcroFormの宣言リソースを有界走査します。未使用の欠落書体も保守的に拒否します。実際に使用された全文字の収録、OpenType組版、文字抽出を保証する検査ではありません。

简体中文: 有界遍历页面、Form、Pattern、注释外观及AcroForm声明资源。未使用但缺失的字体也保守拒绝。不保证所有实际使用字符均存在，也不验证OpenType排版或文本提取。

## Verification / 検証 / 验证

```sh
python3 -m unittest discover -s adapters -p 'test_font_policy.py' -v
cargo test -p rip-storage source_validation --locked
cargo test -p rip-interpreter --test container inspection_reports_embedded_fonts_and_rejects_removed_programs --locked -- --ignored
cargo test -p jrip-server --test source_validation default_worker_validates_all_supported_formats_and_rejects_late_errors --locked -- --ignored
```

Python uses the existing pinned `adapters/source_profiles/requirements.txt`.
Docker tests require the rebuilt `adapters/mupdf/Dockerfile` image. CI runs the
Python checks and container tests. Removing FontFile2 links without changing PDF
xref offsets proves that missing programs fail inspection and worker publication.

🟢 [Done] Default PostScript simple-font substitution now rejects unknown names; see [PostScript policy](POSTSCRIPT-FONT-POLICY.md).
日本語: PostScriptの単純フォントの不明書体置換を拒否します。
简体中文: 拒绝PostScript简单字体的未知字体替代。

🟠 [Next] Runtime font-use evidence, practical multilingual
customer-font comparison, OpenType vertical shaping and complete glyph-use analysis.
日本語: 使用書体記録、実用多言語原稿の比較、OpenType縦組みと使用字形解析は未完了です。
简体中文: 字体使用记录、实际多语言原稿比较、OpenType竖排及已使用字形分析仍未完成。

⭕️ [Pending] Licensed customer fonts/documents and paper acceptance require supplied
rights-cleared originals and a real printer.

CID program rejection now also applies to direct interpreter calls: [CID policy](CID-FONT-POLICY.md).
日本語: 直接描画経路でも欠落CIDプログラムを拒否します。
简体中文: 直接渲染路径也拒绝缺失CID程序。

New inspection reports also include [MuPDF text-operation evidence](FONT-USAGE-EVIDENCE.md) under `fontPolicy.runtimeEvidence`.
日本語: 新しい検査結果ではMuPDFのテキスト操作実測も参照できます。
简体中文: 新检查报告也包含MuPDF文本操作实测记录。
