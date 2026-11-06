# Device copies / 実機出力の部数 / 实体输出份数

🟢 [Done] Explicit Native IPP copy counts through REST, MCP and post-RIP pipelines.

## 日本語

一般的なRIPとしての使いやすさを優先し、TIFFを生成した後の実機送信に部数指定を追加しました。`copies` は1〜999の整数で、指定省略時は1です。さらにプリンターが広告した `copies-supported` の範囲内である必要があります。対応範囲が取得できない機器では複数部を受け付けず、従来互換の1部だけを扱います。不正な型・重複・逆転した能力範囲は正常な能力として採用しません。

対象APIは次の3つです。

- `POST /api/v1/jobs/{jobId}/delivery-preview`
- `POST /api/v1/jobs/{jobId}/deliveries`
- `POST /api/v1/jobs/{jobId}/pipelines`

```json
{
  "requestId": "<UUID>",
  "expectedRevision": 1,
  "printerId": "proof-printer",
  "media": "iso_a4_210x297mm",
  "copies": 3
}
```

原稿は受付済みジョブを参照します。直接DeliveryはRIP完了後、pipelineはRIP完了待ちを含めて予約できます。pipelineは部数を保存し、実際のDelivery準備時にプリンター能力を検査します。対応部数外の直接Deliveryは `unsupported_copies` で拒否します。1〜999以外は `invalid_copies` です。MCPの `print_preview_delivery`、`print_deliver_job`、`print_create_pipeline` にも同じ `copies` を追加しました。scope・revision・原稿検証・用紙寸法照合は引き続き必要です。

部数はDelivery Ticket、物理契約、そのhash、永続pipeline要求、結果Receiptへ渡します。再起動しても保持し、同じrequestIdで部数だけ変えた再投入は競合として拒否します。1部の旧Delivery Ticket／pipeline要求は `copies` 属性がなくても読め、1部の内部ticketでは属性を省略して従来のhash・保存JSONとの互換性を保ちます。物理契約の明示 `copies` は指定部数を表します。

実機承認が必要な場合、`POST /api/v1/approvals` にも同じ `copies` を指定してください。承認前後で部数を変更できません。旧承認の部数省略は1部として扱い、複数部には流用できません。APIでの検査に加え、キュー登録と同じDBトランザクションでも部数を再検査します。pipelineが既存のDeliveryを引き継ぐ際と統合ticket照会でも部数一致を確認します。

送信直前にプリンター能力を再取得します。対応範囲が変わっていた場合は、送信せず `REJECTED` / `unsupported_copies` とします。IPP Print-Jobに明示的な `copies` と既存の `ipp-attribute-fidelity=true` を含め、暗黙の部数変更を要求しません。プリンターが属性無視・置換の成功コードを返す場合は既存規則でUNKNOWNとし、自動再送しません。

TIFF／PWG Rasterのページデータは部数分複製しません。プリンターに1件のジョブを送って機器側で部数を処理します。結果Receiptが示すのは機器の完了報告であり、紙の枚数や印刷品質をセンサー等で保証するものではありません。丁合の指定は追加しておらず、機器の既定動作になります。

今回の対象は明示的な実機Delivery／後続pipelineです。PDF限定のIPP仮想プリンター受付とartifact専用PrintIntent／raster JobTicketは引き続き1部です。実機pipelineの原稿投入前PrintIntentは現在複数部に対応します（[PRINT-INTENT-COPIES.md](PRINT-INTENT-COPIES.md)）。原稿ページ数やRIP完了意味は変更しません。共通能力のPrinter工程には対応範囲を公開しますが、ラスター用 `deliveryIntersection.copies` は1セットを表し、実機用の `deliveryIntersection.deliveryCopies` に対応範囲を公開します。

⭕️ [Pending] 実機での複数部出力・丁合順・取消タイミング・紙枚数の受入確認。開発環境では模擬IPP要求、能力変更、承認、再送、再起動、pipeline伝播まで検証しています。

🟠 [Next] 両面、ページ範囲、用紙指定の拡充、PrintIntent経路への印刷条件統合、基本プレビュー／操作画面。一般RIPの実用度を優先して進めます。

## English

REST delivery preview, explicit delivery and post-RIP pipeline requests, plus MCP `print_preview_delivery`, `print_deliver_job` and `print_create_pipeline`, accept integer `copies` from 1–999 (default 1). Multiple copies require a valid printer-advertised `copies-supported` range. Missing capability permits only the compatibility single-copy path; malformed, duplicate or reversed ranges are rejected. Printer-stage capabilities expose the range, capped by the application limit.

Copies are pinned in persistent delivery/pipeline tickets, physical-contract hashes, approval checks and terminal receipt evidence. Changing copies for an existing request ID conflicts. Legacy absent fields deserialize as one, and single-copy internal serialization omits the new field to retain old JSON/hash compatibility. Physical-print approvals accept `copies`; omitted legacy approval fields authorize only one copy. Queue admission atomically rechecks approved copies. Pipeline child matching and unified-ticket reads reject copy mismatches.

The worker rechecks live capabilities immediately before sending. Unsupported changed settings reject without Print-Job. The single IPP job carries explicit `copies` and attribute fidelity; raster data is not duplicated. Substituted/ignored-attribute success remains UNKNOWN and is never automatically resent. Acceptance/completion evidence does not prove physical sheet counts or appearance. Collation is not newly controlled and follows the device default.

The PDF-only virtual printer and artifact-only PrintIntent/raster ticket remain single-copy. Physical pipeline preflight now supports copies before source acceptance; see [PRINT-INTENT-COPIES.md](PRINT-INTENT-COPIES.md). `deliveryIntersection.copies` remains one raster set; `deliveryCopies` advertises physical counts. Real-device copies/collation/cancellation acceptance is Pending. See ROADMAP for current unfinished work.

## 简体中文

REST投递预览、显式投递、RIP后续pipeline，以及MCP `print_preview_delivery`、`print_deliver_job`、`print_create_pipeline` 接受1至999的整数 `copies`，默认1。多份输出必须在设备公布的 `copies-supported` 范围内。未公布时只允许兼容的单份路径；错误类型、重复及倒置范围会拒绝。Printer工程能力公开受应用上限限制的范围。

份数写入持久Delivery／pipeline ticket、物理契约hash、审批及最终Receipt。相同requestId更改份数会冲突。旧数据缺省按1处理；单份内部序列化省略新增字段，保留旧JSON及hash兼容性。实体打印审批也接受 `copies`；旧审批缺省只授权一份。队列入库事务再次验证份数，pipeline子任务匹配及统一ticket读取也拒绝份数不一致。

发送前重新取得设备能力，范围变化时拒绝，不发送Print-Job。使用一个IPP任务，显式传递 `copies` 和属性fidelity，不复制光栅页数据。设备忽略或替换属性的成功响应仍归为UNKNOWN，不自动重发。设备完成报告不证明实际纸张数量或外观。未新增逐份整理控制，使用设备默认值。

PDF虚拟打印机及仅产物的PrintIntent／raster ticket仍限制为一套。实体pipeline预检现已支持接收原稿前协商份数，参见 [PRINT-INTENT-COPIES.md](PRINT-INTENT-COPIES.md)。`deliveryIntersection.copies` 表示一套光栅，`deliveryCopies` 公布实体输出范围。真实设备份数、整理顺序及取消验收为Pending，未完成工作参见ROADMAP。

🟢 [Done] Rendered-page selection is now available for explicit delivery/pipelines. RIP後のページ範囲指定／RIP后页码范围选择: [PRINT-PAGE-RANGES.md](PRINT-PAGE-RANGES.md).
