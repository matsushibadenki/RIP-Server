# CIDFont policy / CIDFont方針 / CIDFont策略

🟢 [Done] Both PDF rendering adapters require the declared-resource font policy
before producing a page frame. The trusted manifest format is passed through a fixed internal marker; the Ghostscript adapter does not infer PDF status from a filename or a short source prefix. Missing CIDFont programs are rejected, including
when Ghostscript is invoked directly through the interpreter interface. The common
worker still inspects the complete PDF before page selection. TrueType CIDToGIDMap,
CID CFF and explicit horizontal/vertical CMaps remain supported when embedded.
See [PDF font policy](PDF-FONT-POLICY.md) for the bounded traversal and exclusions.

日本語: PDFの両描画アダプターで、ページ進捗を出す前に宣言リソースの書体方針を適用します。CIDプログラムが欠落したPDFは、Ghostscriptを直接呼ぶ経路でも拒否します。共通ワーカーによる全原稿の事前検査も維持します。埋め込みCID TrueType／CFFと横／縦CMapの描画は継続して検証します。

简体中文: 两个PDF渲染适配器在输出页面进度前应用声明资源字体策略。即使直接调用Ghostscript解释器接口，也拒绝缺失CID字体程序的PDF。保留共用工作器的完整原稿预检，并持续验证内嵌CID TrueType／CFF及横竖排CMap。

## Ghostscript resources / Ghostscriptリソース / Ghostscript资源

Ghostscript uses fixed options in addition to the simple-font policy:

```text
-dPDFNOCIDFALLBACK
-dPDFSTOPONERROR
-I/usr/local/lib/jrip/strict-resources
```

The image supplies comment-only `cidfmap` and `FAPIcidfmap` at that search path.
The adapter checks that both maps exist and contain no executable mapping records.
A container test verifies that Ghostscript's `findlibfile` selects these exact files.
The configured default maps therefore add no aliases to other CIDFont resources.
Source-defined CIDFont resources remain possible. A missing PS CIDFont requested
via `findresource` fails rather than gaining a configured alias.

日本語: 固定の検索先へ空の`cidfmap`／`FAPIcidfmap`を配置し、存在と内容をアダプターで確認します。Ghostscript自身がその2ファイルを選択することも検査します。既定マップによるCIDFont別名対応を追加せず、原稿自身が定義するCIDFontは使用できます。

简体中文: 在固定搜索路径提供空的`cidfmap`及`FAPIcidfmap`，适配器检查文件存在且无可执行映射记录。测试也验证Ghostscript自身选择这两个文件。默认映射不再添加CID字体别名，但允许原稿定义CID字体。

The [official CID substitution documentation](https://ghostscript.readthedocs.io/en/gs10.03.0/Use.html#cid-font-substitution)
describes the native PDF fallback switches. On the actual Debian Ghostscript
10.0.0 image, a copy of the owned punctuation PDF with both FontFile links removed
still rendered successfully as NimbusSans under those switches alone. Consequently,
those switches are defense in depth; the independent PDF font gate is required.
Deleting DroidSansFallback alone is insufficient: Ghostscript documents a further
bullet-font fallback. No claim is made that the native switches alone enforce
embedding on all Ghostscript builds.

日本語: 実際のGhostscript 10.0.0では、CIDプログラムを外した自作PDFがネイティブの禁止オプションだけではNimbusSansへ代替されました。そのため、独立したPDF書体検査を必須にしています。フォールバック書体の削除だけで解決したとは扱いません。

简体中文: 实测Ghostscript 10.0.0在仅使用原生禁止选项时，仍将移除CID程序的自制PDF替代为NimbusSans。因此必须独立执行PDF字体检查，不能只依赖删除回退字体文件。

## Verification / 検証 / 验证

```sh
docker build -f adapters/ghostscript/Dockerfile -t jrip-ghostscript:local .
cargo test -p rip-interpreter --test container ghostscript_rejects_missing_cid_programs_before_any_page_frame --locked -- --ignored
cargo test -p rip-interpreter --locked -- --ignored --test-threads=1
cargo test -p jrip-server --test source_validation --locked -- --ignored --test-threads=1
```

The negative PDF preserves xref offsets by renaming FontFile2/3 keys to equal-length
unused keys. The regression checks zero page callbacks and no first TIFF. Positive
coverage uses embedded CID TrueType/CFF in RGB and gray on both engines, including
vertical metrics and punctuation. Rebuild the Ghostscript image before deploying
this worker version; no extra package dependency is introduced. Upgrade the worker and both images together: the Ghostscript adapter consumes the internal `--jrip-format=pdf|postscript|eps` marker, and rejects missing or duplicate markers. Mixing old workers/images with this version is not supported.
日本語: ワーカーと両イメージは一緒に更新してください。Ghostscriptアダプターは内部の原稿形式指定を必須とし、旧バージョンとの混在は対応しません。
简体中文: 工作器及两个镜像须一起升级。Ghostscript适配器要求内部原稿格式标记，不支持与旧版本混用。

🟠 [Next] Actual runtime font/glyph-use evidence, practical multilingual document
comparisons, OpenType shaping and arbitrary source-defined PostScript substitution
analysis remain unfinished. These controls do not prove the source program's
intended typography or replace the existing sandbox.
日本語: 実際の使用書体・使用字形の記録、実用多言語原稿、OpenType組版、PostScript原稿が定義する代替処理の解析は未完了です。
简体中文: 实际字体与字形使用记录、实际多语言原稿、OpenType排版及PostScript原稿自定义替代分析仍未完成。
