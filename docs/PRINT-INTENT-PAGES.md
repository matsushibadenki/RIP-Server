# PrintIntent page selection / 投入前のページ指定 / 原稿接收前页码选择

🟢 [Done] `POST /api/v1/print/pipeline-preflights` accepts physical `intent.pageRanges`. The existing MCP `print_create_pipeline_preflight` forwards the same intent.

```json
"pageRanges": {
  "mode": "required",
  "value": [{"first": 2, "last": 3}, {"first": 5, "last": 5}]
}
```


## 日本語

原稿を投入する前に印刷対象ページを指定できます。PDFでは指定ページだけを描画し、PS／EPSでは全ページ描画後に選択します。ページ番号は原稿の物理ページ位置です。ページラベルや部数で繰り返した後の位置ではありません。面付け・numberUp等によるページ再配置は引き続き未対応です。

- `required` と `preferred` は、有効な指定範囲をそのまま採用します。実際のページ数を超えた場合も範囲を切り詰めません。
- `automatic` は全ページを選びます。非空の範囲が提示されていた場合、`AUTOMATIC_SELECTION` と `resolved: []` を返します。
- 省略・空配列は全ページです。1始まり・終端を含み、各値は1〜100、最大100範囲、昇順・重複なしとします。隣接範囲は結合します。0・逆転・順序不正・重複・上限超過は全モードで `invalid_page_ranges` として拒否します。
- 原稿検査 `inspectionId` がある場合は、実ページ数と照合し、範囲外を422 `page_range_out_of_bounds` で拒否します。用紙寸法照合は選択ページに対して実施します。未選択ページについてもページ順・寸法値の有効性は検査します。
- 検査結果がない場合、プリフライトはページ数を保証しません。PDFではワーカーの描画前検査、PS／EPSではDelivery準備時に実数と照合し、範囲外なら送信しません。

正規化した範囲を `workflowRequest.pageRanges` と `registrationTicket.request.pageRanges` に保存し、プリフライトdigestと原稿commitに拘束します。既存のpipeline・承認・物理契約・送信処理へそのまま渡すため、再起動後も保持します。複数部では、選択済み文書を1回のIPPジョブで指定部数だけ出力します。

PDFのラスター用ticketも選択範囲を表し、生成成果物に原稿ページ対応を保存します。能力モデルは対応形式と範囲上限を公開します。artifact専用resolve／ラスター事前確認にも対応します。仮想IPPは[対応済み](IPP-PAGE-RANGES.md)、操作画面への入力も[対応済み](OPERATOR-CONSOLE.md)です。[選択描画の仕様](RENDER-PAGE-SELECTION.md)。

## English

Physical pipeline preflight accepts page selection before source commit. PDF renders only selected pages; PS/EPS retains all-page rendering followed by delivery selection. Numbers are one-based physical source positions, not PDF page labels or copy-expanded indices; page reordering/imposition is not supported.

`required` and `preferred` retain valid requested ranges. `automatic` selects all and reports `AUTOMATIC_SELECTION`, `resolved: []` for a nonempty hint. Omitted/empty means all. Ranges are inclusive, sorted, non-overlapping, within 1–100 and limited to 100 entries. Adjacent ranges merge. Invalid ranges are rejected in every mode, never silently clipped or replaced.

With `inspectionId`, preflight verifies page-count bounds (422 `page_range_out_of_bounds`) and checks configured media geometry only for selected pages; all inspection entries still undergo structural/order/dimension-value validation. Without preflight inspection, page-count checks occur in the PDF worker before rendering, or before delivery for PS/EPS; out-of-bounds selection never sends a document.

Canonical ranges are bound in `workflowRequest`, the registration ticket, preflight digest, source commit, existing approval/delivery contracts and receipts. They survive restart. Copies apply to the selected document in a single IPP submission. PDF raster tickets bind render selection and source-page mapping. Artifact-only resolution also supports PDF ranges. Capability metadata advertises supported formats and bounds. Virtual IPP is [implemented](IPP-PAGE-RANGES.md); console input is [implemented](OPERATOR-CONSOLE.md). See [render selection](RENDER-PAGE-SELECTION.md).

## 简体中文

实体pipeline预检现可在提交原稿前选择输出页。PDF仅渲染所选页，PS／EPS保留全页渲染后筛选。页码从1开始，表示原稿物理页，不是PDF标签或多份展开后的索引；不支持页面重排或拼版。

`required` 与 `preferred` 保留有效请求范围。`automatic` 选择全部页，非空提示通过 `AUTOMATIC_SELECTION`、`resolved: []` 明确说明。缺省或空数组表示全部页。范围包含终点，要求升序、无重叠、每个值1至100、最多100项，合并相邻范围。所有模式均拒绝无效范围，不静默裁剪或替换。

提供 `inspectionId` 时预检会验证实际页数边界（越界返回422 `page_range_out_of_bounds`），仅对所选页匹配介质尺寸；仍验证全部检查条目的页序及尺寸值有效性。无检查结果时，在PDF worker渲染前验证页数，PS／EPS在投递准备阶段验证，越界时不发送文档。

规范化范围绑定在 `workflowRequest`、登记ticket、预检digest、原稿commit及既有审批／Delivery契约／Receipt中，重启后保留。份数作用于所选文档，使用单个IPP任务发送。PDF光栅ticket绑定选择范围与原稿页映射，产物专用resolve同样支持。能力元数据公布格式及上限。虚拟IPP已[实现](IPP-PAGE-RANGES.md)，界面输入已[实现](OPERATOR-CONSOLE.md)。参见[选择渲染](RENDER-PAGE-SELECTION.md)。

## Verification / 検証 / 验证

🟢 [Done] Real local IPP mock tests cover canonical ranges through preflight, source commit/replay, restart and exact selected PWG bytes; copies apply once to that document. Additional tests reject malformed ranges in every mode and check inspection bounds and selected-page media. Existing downstream approval/range-binding tests remain active. Raster fixtures are supplied at the worker boundary; these tests do not exercise the external PDF renderer.

🟢 [Done] [Console range input / 操作画面の範囲入力 / 界面范围输入](OPERATOR-CONSOLE.md)。

⭕️ [Pending] Real printer paper/order acceptance. / 実機の紙・順序確認。 / 实机纸张及顺序验收。
