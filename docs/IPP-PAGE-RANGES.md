# IPP page ranges / IPPページ範囲 / IPP页码范围

🟢 [Done] PDF仮想プリンターで `page-ranges` を受け付け、標準ワーカーのMuPDF／Ghostscript選択描画へ接続します。

## 日本語

`Get-Printer-Attributes` は `page-ranges-supported` をbooleanのtrueで返します。`page-ranges-default` はありません。`Print-Job`／`Validate-Job` はjob属性グループの `page-ranges`（`1setOf rangeOfInteger`）を受理します。1始まり・両端を含み、昇順・重複なし、各値1〜2147483647、最大100範囲です。既存の128属性・16KiB上限も適用します。不正な型・長さ・0以下・逆転・順序不正・重複は `client-error-bad-request` で拒否し、ジョブを作りません。隣接範囲は結合します。

[IPP/1.1 RFC 8011 §5.2.7](https://www.rfc-editor.org/rfc/rfc8011.html#section-5.2.7)に従い、存在しない原稿ページは出力しません。例えば2ページPDFへの `2-2147483647` は2ページ目だけを描画し、`3-2147483647` は成果物0ページで正常完了します。原稿全体の厳密な検査・ICC宣言解析は継続し、空選択を使って破損PDFの検証を回避することはできません。原稿そのものの最大100ページ、32MiB、コンテナーのCPU・メモリ・時間・出力容量制限は維持します。

範囲は永続Manifestの `page_ranges: {"ipp": [{"first":2,"last":2147483647}]}` として保存し、REST用の厳密な範囲配列とは区別します。RESTの通常配列は従来どおり上限100・原稿範囲外拒否です。IPP属性の返却には要求範囲を保持し、実際に生成したTIFFには `sourcePage`／`sourcePageCount` を保存します。検証記録には `pageSelectionSemantics`、`requestedPageRanges`、`renderedPageCount` を含めます。空出力の公開には原稿hashに一致する厳密PDF検証記録と、対象ページが本当に0であることの保存層での再確認が必要です。

`Get-Job-Attributes`／`Get-Jobs` は正規化した範囲を返し、`requested-attributes=page-ranges`／`job-template` に対応します。省略時は全ページで、範囲属性を付け足しません。ジョブと範囲は再起動後も保持します。完了はTIFFファイル出力の完了で、紙への印刷ではありません。

独立CUPS `ipptool` → 実HTTP → IPP → 実ワーカー → Dockerの試験で、単一ページ、上限値の大きい範囲、全て原稿外の範囲、複数範囲、原稿色、再起動、破損原稿拒否を確認します。

```sh
cargo test -p jrip-ipp --test protocol --locked
cargo test -p jrip-server --test ipp_e2e --locked -- --ignored
cargo test -p rip-interpreter --test container selected_pdf_pages_are_skipped_and_source_count_is_bound --locked -- --ignored
```

🟢 [Done] [操作画面からのページ範囲指定](OPERATOR-CONSOLE.md)。
⭕️ [Pending] Mac／Windowsの印刷ダイアログと実機の紙枚数・順序の受入確認。IPP Everywhere適合は別の未完了項目です。

## English

The PDF virtual printer advertises boolean `page-ranges-supported=true`, without a default attribute. Print-Job and Validate-Job accept job-group `1setOf rangeOfInteger`, inclusive, ascending and non-overlapping, with positive signed 32-bit endpoints and at most 100 ranges. Malformed types, lengths, order and overlaps fail before spooling. Adjacent ranges merge. Existing wire/resource budgets remain.

RFC 8011 §5.2.7 excludes nonexistent source positions: on a two-page PDF, `2-2147483647` renders page 2, while `3-2147483647` completes with zero artifacts. Full PDF validation and ICC declaration analysis still run, including for empty selections. Source limits remain 100 pages and 32MiB. Persistent manifests use an explicit `{ipp:[...]}` wrapper; ordinary REST range arrays retain strict bounds and reject out-of-source requests. Empty publication requires hash-bound strict PDF evidence and a checked empty intersection at the storage boundary.

Requested ranges survive restart and are returned through Get-Job-Attributes/Get-Jobs with requested-attributes filtering; actual TIFFs retain original source-page mapping. Independent CUPS-to-worker-to-Docker regressions cover selection, clipping, empty success, persistence and malformed-source rejection. Completion means TIFF export. Console controls are implemented; actual print-dialog/device acceptance is Pending, and IPP Everywhere conformance remains separate unfinished work.

## 简体中文

PDF虚拟打印机公布boolean类型 `page-ranges-supported=true`，不提供默认范围属性。Print-Job／Validate-Job接受job组的 `1setOf rangeOfInteger`，范围包含两端、升序、不重叠，端点为正32位有符号整数，最多100项。错误类型、长度、逆序及重叠在落盘前拒绝，相邻范围合并，原有协议与资源预算保持。

按照RFC 8011 §5.2.7，忽略不存在的原稿页：两页PDF的 `2-2147483647` 仅渲染第2页，`3-2147483647` 正常完成且产物为0页。即使空选择仍严格检查全部PDF与ICC声明，破损原稿不能绕过验证。原稿仍限100页、32MiB。持久Manifest以 `{ipp:[...]}` 明确记录协议语义，普通REST范围数组保持严格上限并拒绝原稿外范围。空产物发布要求与hash绑定的严格PDF验证记录，以及存储层确认交集为空。

范围在重启后保留，并通过Get-Job-Attributes／Get-Jobs按requested-attributes返回。实际TIFF保留原稿页映射。独立CUPS至实际worker与Docker的回归试验覆盖选择、范围截取、空结果、持久化及破损原稿拒绝。完成仅表示TIFF导出。界面范围输入已实现，真实打印对话框与设备验收为Pending，IPP Everywhere适合性仍为独立未完成项。
