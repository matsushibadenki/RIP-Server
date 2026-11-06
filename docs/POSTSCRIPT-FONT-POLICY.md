# PostScript simple-font resolution / PostScript単純フォント解決 / PostScript简单字体解析

🟢 [Done] Ghostscript runs with fixed worker-controlled options:

```text
-dNOFONTPATH
-dNOPLATFONTS
-dNONATIVEFONTMAP
-sSUBSTFONT=JRIP-Missing-Font-Rejected
```

The packaged Fontmap remains available, as do fonts defined by the source program.
The substitution target deliberately does not exist in the supplied image. When a
simple `findfont` lookup cannot resolve a font, the pinned Ghostscript build raises
`invalidfont` instead of silently returning Courier. No PostScript findfont wrapper,
source-text rewrite or document-controlled command-line argument is introduced.
The options apply to every Ghostscript invocation, including explicit PDF rendering.
PDF input still passes the separate [PDF font policy](PDF-FONT-POLICY.md) first.

日本語: 固定オプションでパッケージ内Fontmapと原稿内定義の書体を使用し、不明な単純フォントをCourierへ黙って置換しません。代替先を存在しない名前に固定することで、現在のGhostscriptでは`invalidfont`になります。独自のfindfont命令置換や原稿文字列の書換えは加えていません。PDFの明示的Ghostscript描画にも適用し、PDF固有の事前検査も維持します。

简体中文: 固定选项保留软件包Fontmap和原稿定义的字体，不再将未知简单字体静默替换为Courier。将替代目标固定为不存在的名称，使当前Ghostscript产生`invalidfont`。不替换findfont指令，也不改写原稿文本。选项同样适用于显式Ghostscript PDF渲染，并保留PDF预检。

The native options and their font lookup semantics are documented in
[Ghostscript font parameters](https://ghostscript.readthedocs.io/en/gs10.03.0/Use.html#font-related-parameters)
and [Fontmap documentation](https://ghostscript.readthedocs.io/en/latest/Fonts.html).
Tests verify behavior on the actual supplied Debian image rather than assuming
that setting an unknown substitution name rejects fonts on every Ghostscript release.

## Publication / 公開 / 发布

A page streamed before a later font error is progress evidence, not a completed job.
The existing worker marks the render failed and publishes no final raster artifacts.
REST, IPP and MCP share this worker path. A retry requires an explicit new attempt;
there is no automatic different-font or different-engine retry.

日本語: 後続の書体解決で失敗した場合、先行ページの途中進捗があってもジョブは失敗し、最終成果物を公開しません。REST・IPP・MCPは共通ワーカーを使用し、別書体や別エンジンへの自動再試行はしません。

简体中文: 即使前面的页面已产生进度，后续字体解析失败仍使作业失败，不发布最终产物。REST、IPP及MCP使用共用工作器，不自动换用其他字体或引擎重试。

## Scope / 範囲 / 范围

This is the default simple-font resolution policy, not a font whitelist or a
complete PostScript resource inventory. Packaged Fontmap aliases are intentional
mappings; their outlines are not asserted to equal the original commercial font.
PostScript is executable: a program can explicitly define fonts, choose alternate
fonts, redefine its operators or handle an error. This setting does not prove the
program's typographic intent, prohibit explicit source-program substitution, or
replace the existing process/filesystem/network sandbox. It does not declare which
font was used for each glyph. PS/EPS validation reports do not gain a fabricated
PDF-style embedded-font inventory.

日本語: 単純フォントの既定解決方針です。Fontmapの明示的な別名対応は残します。実行可能なPostScriptが明示的に別書体を選ぶことやエラーを処理することまでは禁止せず、使用書体・使用字形の完全な一覧を保証しません。PS／EPSの検証結果へPDF用の埋め込み書体判定を付けません。

简体中文: 这是简单字体的默认解析策略，保留Fontmap中的显式别名映射。PostScript程序仍可主动选择替代字体或处理错误，不保证完整的实际字体、字形清单。不向PS／EPS报告附加PDF专用的内嵌字体判定。

## Verification / 検証 / 验证

```sh
cargo test -p rip-interpreter --test container postscript_missing_fonts_fail_instead_of_silent_courier_substitution --locked -- --ignored
cargo test -p jrip-server --test source_validation --locked -- --ignored --test-threads=1
cargo test -p rip-interpreter --locked -- --ignored --test-threads=1
```

The first test renders Helvetica, Courier and an owned inline Type3 font, rejects
an unknown font before output and after a prior showpage. Worker tests add missing
fonts in PS and EPS and assert failure, absence of source-validation success and
absence of published artifacts. Existing PDF RGB/CMYK/gray, owned embedded fonts,
CID CFF and vertical-punctuation regressions cover compatibility.

🟢 [Done] Fixed CIDFont substitution maps and missing PDF CID program rejection: [CID policy](CID-FONT-POLICY.md).
日本語: CIDFontの固定代替マップと欠落PDFプログラムの拒否に対応しました。
简体中文: 已实现固定CIDFont替代映射及缺失PDF程序拒绝。

🟠 [Next] Actual runtime font-use evidence and
practical multilingual document comparison remain unfinished.
日本語: 実際の使用書体の記録、実用多言語原稿の比較は未完了です。
简体中文: 实际运行字体使用记录及实际多语言原稿比较仍未完成。
