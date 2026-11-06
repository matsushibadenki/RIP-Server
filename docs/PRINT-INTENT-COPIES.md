# PrintIntent device copies / PrintIntentの実機部数 / PrintIntent设备份数

🟢 [Done] `POST /api/v1/print/pipeline-preflights` negotiates `intent.copies` against live printer capabilities before source acceptance. The existing MCP `print_create_pipeline_preflight` forwards the same intent.

日本語: 原稿投入前の実機pipelineプリフライトで、`intent.copies` をライブ機器能力と照合します。既存MCPの `print_create_pipeline_preflight` からも同じ要求を渡せます。

简体中文: 在接收原稿前，通过实体pipeline预检将 `intent.copies` 与实时设备能力匹配。现有MCP `print_create_pipeline_preflight` 也可转发相同意图。

```json
{
  "requestId": "a4f2318a-44cb-483d-8d78-802ce3243636",
  "intent": {
    "document": {
      "uri": "https://example.org/proof.pdf",
      "mediaType": "application/pdf",
      "sha256": "REPLACE_WITH_ACTUAL_SOURCE_SHA256"
    },
    "copies": {"mode": "required", "value": 3},
    "output": {"destination": "printer", "printerId": "test"}
  }
}
```

The document reference is descriptive; commit still supplies and verifies the actual source bytes. This example is not executable until the source digest and configured printer are replaced.

| Mode / モード / 模式 | Resolution / 解決 / 解析 |
| --- | --- |
| `required` | Exact count or HTTP 422 conflict with alternatives. / 指定部数、非対応なら代替案付き422。 / 严格匹配，否则返回422及替代值。 |
| `preferred` | Requested supported count, otherwise lowest supported count with `PREFERENCE_NOT_SUPPORTED`. / 対応時は指定値、非対応時は対応最小値と変更理由。 / 支持时采用请求值，否则采用最小支持值并说明原因。 |
| `automatic` | Lowest supported count; differing hints produce `AUTOMATIC_SELECTION`. / 対応最小値を選択し、要求値との差を明示。 / 选择最小支持值并明确记录与请求值的差异。 |

Values outside 1–999 are rejected in every mode. No clipping or silent increase occurs. Missing `copies-supported` retains the existing single-copy compatibility path; malformed capability responses are rejected by the printer adapter. Valid device ranges excluding one are supported for physical output. The application caps larger advertised maxima at 999.

日本語: どのモードでも1〜999以外は拒否します。黙った切り詰め・増加はしません。機器の `copies-supported` 省略時は従来互換の1部だけを扱い、不正な能力応答はアダプターで拒否します。実機が1部に対応しない場合も、有効な対応範囲で部数を選べます。機器の上限が999を超える場合はアプリ上限999を適用します。

简体中文: 所有模式均拒绝1至999以外的值，不静默裁剪或增加。设备未公布 `copies-supported` 时仅支持兼容的单份；适配器拒绝无效能力响应。即使设备不支持单份，也可从有效范围选择实体输出份数。设备最大值超过999时，应用上限为999。

## Stage boundaries / 工程の区別 / 阶段边界

- `deliveryIntersection.copies: [1,1]` remains the single raster-set constraint. / ラスター生成は1セット。 / 光栅仅生成一套。
- `deliveryIntersection.deliveryCopies: [min,max]` is the supported physical delivery range; null means incompatible. / 実機出力範囲、nullは非互換。 / 实体输出范围，null表示不兼容。
- The preflight's raster `ticket.copies` remains one. `workflowRequest.copies` and `registrationTicket.request.copies` bind physical copies; absent internal `copies` continues to mean one for legacy JSON/hash compatibility. / 原稿ticketは1、実機部数はworkflowと登録ticketに固定し、内部属性の省略は1部。 / 原稿ticket保持1，实体份数绑定workflow及登记ticket，内部字段缺省仍表示1。
- Commit atomically stores the source and bound workflow, verifies its digest, and rechecks capability revision. Existing approval, delivery contract, worker-side live checks and receipt binding apply unchanged. / commitで原稿とworkflowを原子的に保存し、digest・能力revisionを再検査。既存承認・契約・送信前検査・Receiptへ引き継ぐ。 / commit原子保存原稿与workflow，并重新检查digest及能力revision，沿用现有审批、契约、发送前检查及Receipt绑定。
- Raster pages are not duplicated. A single IPP Print-Job carries the selected copies. / 画素を複製せず1回のIPP送信に部数を指定。 / 不复制光栅数据，单个IPP任务携带份数。

The artifact-only `/api/v1/print/resolve` and raster preflight remain single-copy; this extension applies to physical pipeline preflight. The PDF-only virtual IPP printer and console do not gain device-copy controls in this change. Collation and real sheet counts remain device-dependent.

日本語: artifact専用のresolve／ラスター事前確認は1セットのままです。今回は実機pipeline入口を拡張し、PDF仮想IPP受付・操作画面の部数UIは追加しません。丁合と実際の紙枚数は実機検証が必要です。

简体中文: 仅产物的resolve及光栅预检仍保持一套。本次扩展实体pipeline入口，不新增PDF虚拟IPP接收或操作界面的设备份数控制。逐份整理及实际纸张数量需要实机验证。

🟢 [Done] Tests: mode negotiation, invalid ranges, raster/physical capability separation, atomic commit replay, restart persistence, exact IPP copies with one raster set, and capability changes before commit. Existing approval and send-time capability tests continue to apply.

⭕️ [Pending] Real printer copies/collation acceptance. / 実機部数・丁合受入。 / 实机份数及整理验收。
