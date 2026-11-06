# PrintIntent page selection / 投入前のページ指定 / 原稿接收前页码选择

🟢 [Done] `POST /api/v1/print/pipeline-preflights` accepts physical `intent.pageRanges`. The existing MCP `print_create_pipeline_preflight` forwards the same intent.

```json
"pageRanges": {
  "mode": "required",
  "value": [{"first": 2, "last": 3}, {"first": 5, "last": 5}]
}
```

## 日本語

原稿を投入する前に印刷対象ページを指定できます。**今回は全ページをRIPし、Deliveryで指定ページだけを選択します。描画時間を削減する機能ではありません。** 元のラスターは全ページ保持し、ページ番号は原稿順に対応するRIP出力の位置です。ページラベルや部数で繰り返した後の位置ではありません。面付け・numberUp等によるページ再配置は引き続き未対応です。

- `required` と `preferred` は、有効な指定範囲をそのまま採用します。実際のページ数を超えた場合も範囲を切り詰めません。
- `automatic` は全ページを選びます。非空の範囲が提示されていた場合、`AUTOMATIC_SELECTION` と `resolved: []` を返します。
- 省略・空配列は全ページです。1始まり・終端を含み、各値は1〜100、最大100範囲、昇順・重複なしとします。隣接範囲は結合します。0・逆転・順序不正・重複・上限超過は全モードで `invalid_page_ranges` として拒否します。
- 原稿検査 `inspectionId` がある場合は、実ページ数と照合し、範囲外を422 `page_range_out_of_bounds` で拒否します。用紙寸法照合は選択ページに対して実施します。未選択ページについてもページ順・寸法値の有効性は検査します。
- 検査結果がない場合、プリフライトはページ数を保証しません。RIP完了後、Delivery準備時に実数と照合し、範囲外なら送信しません。

正規化した範囲を `workflowRequest.pageRanges` と `registrationTicket.request.pageRanges` に保存し、プリフライトdigestと原稿commitに拘束します。既存のpipeline・承認・物理契約・送信処理へそのまま渡すため、再起動後も保持します。複数部では、選択済み文書を1回のIPPジョブで指定部数だけ出力します。

ラスター用ticketは引き続き全ページの生成を表します。能力モデルは `deliveryIntersection.deliveryPageRanges` に選択段階と上限を公開し、`renderAllPages: true` を明示します。Printer工程のextensionにも機能を含め、capability revisionへ拘束します。artifact専用resolve／ラスター事前確認、PDF仮想IPP受付、操作画面へのページ範囲UI追加は今回の対象外です。

## English

Physical pipeline preflight accepts page selection before source commit. **All source pages are still rendered; selection is applied at delivery. This does not reduce rendering work.** Numbers are one-based RIP output positions in original order, not PDF page labels or copy-expanded indices. Original rasters remain intact; page reordering/imposition is not supported.

`required` and `preferred` retain valid requested ranges. `automatic` selects all and reports `AUTOMATIC_SELECTION`, `resolved: []` for a nonempty hint. Omitted/empty means all. Ranges are inclusive, sorted, non-overlapping, within 1–100 and limited to 100 entries. Adjacent ranges merge. Invalid ranges are rejected in every mode, never silently clipped or replaced.

With `inspectionId`, preflight verifies page-count bounds (422 `page_range_out_of_bounds`) and checks configured media geometry only for selected pages; all inspection entries still undergo structural/order/dimension-value validation. Without inspection, page-count checks occur after RIP, before delivery; out-of-bounds selection never sends a document.

Canonical ranges are bound in `workflowRequest`, the registration ticket, preflight digest, source commit, existing approval/delivery contracts and receipts. They survive restart. Copies apply to the selected document in a single IPP submission. The raster ticket still describes all-page generation. `deliveryIntersection.deliveryPageRanges` advertises stage and bounds with `renderAllPages: true`; printer-stage extensions bind this capability to the revision. Artifact-only resolution, virtual IPP intake and the console gain no new range controls here.

## 简体中文

实体pipeline预检现可在提交原稿前选择输出页。**仍然渲染全部原稿页，仅在Delivery阶段选择指定页，不减少渲染工作。** 页码为原稿顺序对应的RIP输出位置，从1开始，不是PDF页面标签或多份展开后的索引。保留完整原始光栅；不支持页面重排或拼版。

`required` 与 `preferred` 保留有效请求范围。`automatic` 选择全部页，非空提示通过 `AUTOMATIC_SELECTION`、`resolved: []` 明确说明。缺省或空数组表示全部页。范围包含终点，要求升序、无重叠、每个值1至100、最多100项，合并相邻范围。所有模式均拒绝无效范围，不静默裁剪或替换。

提供 `inspectionId` 时预检会验证实际页数边界（越界返回422 `page_range_out_of_bounds`），仅对所选页匹配介质尺寸；仍验证全部检查条目的页序及尺寸值有效性。无检查结果时，在RIP完成后的Delivery准备阶段验证实际页数，越界时不发送文档。

规范化范围绑定在 `workflowRequest`、登记ticket、预检digest、原稿commit及既有审批／Delivery契约／Receipt中，重启后保留。份数作用于所选文档，使用单个IPP任务发送。光栅ticket仍表示全页生成。`deliveryIntersection.deliveryPageRanges` 公布阶段及上限，并明确 `renderAllPages: true`；Printer阶段extension将能力绑定至revision。本次不向仅产物resolve、虚拟IPP入口或操作界面添加范围控件。

## Verification / 検証 / 验证

🟢 [Done] Real local IPP mock tests cover canonical ranges through preflight, source commit/replay, restart and exact selected PWG bytes; copies apply once to that document. Additional tests reject malformed ranges in every mode and check inspection bounds and selected-page media. Existing downstream approval/range-binding tests remain active. Raster fixtures are supplied at the worker boundary; these tests do not exercise the external PDF renderer.

🟠 [Next] Render-time page skipping and virtual IPP/UI integration. / 描画対象の削減と仮想IPP・画面への統合。 / 渲染时跳页及虚拟IPP／界面集成。

⭕️ [Pending] Real printer paper/order acceptance. / 実機の紙・順序確認。 / 实机纸张及顺序验收。
