# Print page ranges / 印刷ページ範囲 / 打印页码范围

🟢 [Done] Selection of already-rendered pages for explicit Delivery and follow-on pipelines.

## 日本語

RIP済みのページから必要な範囲だけを選択して印刷できます。対象は明示的な実機Delivery、Deliveryプレビュー、後続pipelineです。原稿投入前にMuPDF／Ghostscriptの描画対象を絞る機能ではありません。

```json
{
  "requestId": "<UUID>",
  "expectedRevision": 1,
  "printerId": "proof-printer",
  "media": "iso_a4_210x297mm",
  "copies": 2,
  "pageRanges": [{"first": 2, "last": 3}, {"first": 5, "last": 5}]
}
```

次のAPIに `pageRanges` を追加しました。

- `POST /api/v1/jobs/{jobId}/delivery-preview`
- `POST /api/v1/jobs/{jobId}/deliveries`
- `POST /api/v1/jobs/{jobId}/pipelines`

MCPでは `print_preview_delivery`、`print_deliver_job`、`print_create_pipeline` で同じ引数を使います。MCPのschemaと入力検査も対応しています。

番号は1始まりのRIP出力ページ番号で、PDFの印刷ラベル（ローマ数字など）ではありません。`first`／`last` は1〜100の整数で両端を含み、昇順・重複なしで指定します。範囲配列は最大100要素です。隣接する範囲は同じ選択として結合します（1〜1と2〜3は1〜3）。逆順、重複、0、101以上、誤った型、未知の範囲属性は拒否し、並べ替え・切り詰め・重複印刷で補正しません。指定省略または空配列は全ページで、nullは不正です。実際のRIPページ数を超えた場合は `page_range_out_of_bounds` を返します。

直接DeliveryではRIP完了後に存在するページ数と照合します。後続pipelineはRIP完了前でも範囲を保存できますが、実際のページ数の検査はDelivery準備時です。存在しないページが含まれる場合は実機へ送信しません。

変換前にRIP成果物のページメタデータの並びを確認し、選択ページだけをSHA-256検証付きsnapshotとして読み込みます。32MiBのDelivery入力上限は選択したTIFFの合計に適用します。選択しなかったTIFFのデータはこの段階では読み直しません。プレビューの `verificationScope=selected-raster-pages` はこの検証範囲を示し、原稿全体の再検査や未選択ページの正常性を保証しません。

選択ページを原稿順に1つのPWG Rasterへ変換します。範囲が2〜3なら2ページの送信データとなり、IPPのpage-rangesを追加して二重に絞る処理は行いません。`copies` は選択後の文書全体に適用されます。元のRIP／Color成果物は保持し、ページ番号を変更しません。

物理契約には `pageSelection`（例 `2-3,5`）、明示範囲の場合は `sourcePageCount`、`selectedSourcePages`、`selectedPageCount` を保存します。Delivery Ticket・pipeline要求・契約hash・結果Receiptにも範囲を引き継ぎます。全ページの旧ticketは新属性なしで読み込め、全ページの内部JSONでは属性を省略して互換性を保ちます。同じrequestIdで範囲を変更した再投入は競合として拒否します。隣接範囲の分割表記だけが違う要求は、正規化後の同じ範囲として再送できます。

実機承認が必要な場合、`POST /api/v1/approvals` にも同じ `pageRanges` を指定します。全ページ承認を特定ページ承認へ、またはその逆へ流用できません。APIに加え、キュー登録時のDBトランザクションで範囲一致を再確認します。pipelineの既存child照合と統合ticketの読取でも範囲一致を検査します。実機送信前にPWGのページ数が明示範囲の選択数に一致することを確認し、既存のhash・用紙・色・解像度・機器能力の検査も継続します。

⭕️ [Pending] 実機での選択ページ・紙枚数・丁合・取消の受入確認。開発環境ではページごとに異なる画素を持つTIFFと模擬IPPを使い、送信データの順序と内容、範囲変更拒否、承認、再起動、再送を確認しています。

🟠 [Next] 原稿投入時のページ指定による描画量削減、PrintIntent／IPP仮想プリンター受付への統合、両面・用紙指定、基本プレビューUI。現在のPDF限定IPP受付と原稿投入前PrintIntentは全ページのままです。

## English

Explicit delivery preview, delivery and follow-on pipeline requests, plus their MCP tools, accept `pageRanges: [{first,last}]`. Numbers are one-based rendered page positions, not PDF page labels. Endpoints are inclusive, sorted and non-overlapping, bounded to 1–100 with at most 100 ranges. Adjacent ranges merge canonically. Omitted/empty arrays select all; null, invalid types, reversed/overlapping ranges and unknown fields fail. Ranges beyond the actual raster page count fail without clipping or sending.

Only selected TIFFs are read as hash-verified snapshots and converted in original order into one PWG Raster document; the 32MiB input budget applies to selected TIFFs. Unselected file contents are not reverified. Preview `verificationScope=selected-raster-pages` states this limit. Source/color artifacts remain unchanged. Device copies apply to the selected document. No IPP page-ranges attribute is sent, avoiding double selection.

Persistent tickets, normalized pipeline requests, physical-contract hashes, receipts and approvals bind the selection. Physical contracts include `pageSelection` and, for explicit selections, `sourcePageCount`, `selectedSourcePages`, `selectedPageCount`. Queue admission atomically rechecks approval ranges; existing child matching and unified ticket reads check consistency. Identical adjacent-range representations converge; a changed selection under an existing request ID conflicts. Legacy absent fields remain all-pages and are omitted during default serialization. Before sending, the spool page count must match an explicit selection, alongside existing hash/media/color/DPI/capability checks.

Pipeline requests may be stored before RIP completion; actual page bounds are checked when preparing delivery. This feature filters after rendering, not MuPDF/Ghostscript input processing. Source PrintIntent and the PDF-only IPP virtual printer still process all pages. Render-time selection, duplex/media improvements and basic preview UI remain Next; real printer sheet/order/collation/cancellation acceptance is Pending.

## 简体中文

显式投递预览、投递、后续pipeline及对应MCP工具接受 `pageRanges: [{first,last}]`。页码从1开始，表示RIP输出页的位置，不是PDF页标签。范围包含两端，须升序且不重叠，页码限1至100，最多100个范围。相邻范围合并；省略或空数组表示全部。null、错误类型、倒置、重叠及未知字段会拒绝，超出实际页数时不会裁剪或发送。

仅对选中的TIFF建立hash验证snapshot，按原稿顺序转换为一个PWG Raster文档。32MiB输入预算只计算所选TIFF；未选择文件内容不再次验证。预览的 `verificationScope=selected-raster-pages` 明确此范围。原RIP／Color产物保持不变，设备份数应用于选择后的文档。不再发送IPP page-ranges属性，避免重复筛选。

持久ticket、正规化pipeline请求、物理契约hash、Receipt及审批均绑定范围。物理契约包含 `pageSelection`；显式选择另含 `sourcePageCount`、`selectedSourcePages`、`selectedPageCount`。队列事务再次核对审批范围，pipeline子任务及统一ticket读取也验证一致性。同一requestId改变选择会冲突；仅相邻范围分组不同的等价表示可复用。旧数据缺省仍为全部，默认序列化省略新增属性。发送前核对光栅页数与显式选择数，并保留hash、介质、颜色、DPI及设备能力验证。

pipeline可在RIP完成前保存请求，实际页数在投递准备时检查。此功能在渲染之后筛选，不减少MuPDF／Ghostscript的原稿渲染量。原稿PrintIntent及PDF虚拟打印机仍处理全部页面。渲染时选择、双面／介质扩展及基础预览UI为Next；真实设备页码、纸张数量、整理顺序及取消验收为Pending。

🟢 [Done] Pre-source physical PrintIntent selection / 原稿投入前の実機PrintIntent指定 / 原稿接收前的实体PrintIntent选择: [PRINT-INTENT-PAGES.md](PRINT-INTENT-PAGES.md). Rendering remains all-page / 描画は全ページのまま / 仍渲染全部页。
