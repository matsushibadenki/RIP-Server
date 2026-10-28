# Explicit printer delivery / 実機送信 / 实体打印投递

## 日本語

管理者が `config/printers.example.json` を参考にローカル設定を作り、APIとWorker両方に同じ `JRIP_PRINTER_CONFIG` と `JRIP_DATA_ROOT` を設定する。URIは設定ファイルからのみ取得する。現在は認証なしの `ipp://`、片面・1部、8-bit PWG Rasterに限定する。IPPS/TLS、ICC変換、両面、部数、MCPの実機操作toolは未実装。

`POST /api/v1/jobs/{id}/deliveries` に `requestId`（UUID）、`printerId`、`media`、`expectedRevision` を渡す。`printers:print` と `artifacts:read` の両scopeが必要。完了済みTIFFをハッシュ検証し、PWG変換、ページ寸法照合、プリンター能力照合を行って202と永続Deliveryを返す。202は送信予約であり、実機受理や排紙完了を意味しない。同じrequestIdの再要求は同じDeliveryに収束する。入力TIFF合計32MiB、PWG出力32MiB、最大100ページ。

```sh
export JRIP_DATA_ROOT=./data
export JRIP_PRINTER_CONFIG=./config/printers.local.json
# JRIP_API_TOKENは既存の環境設定を使用
cargo run --locked -p jrip-server --bin jrip-delivery-worker
```

Workerの起動によってQUEUEDの送信を実行する。送信前に登録済みファイルのSHA-256・サイズ・用紙寸法・接続先の不変性と最新の能力を再確認する。`GET /api/v1/deliveries/{id}` は `jobs:read` で状態を取得する。RIPのCOMPLETEDとDeliveryのCOMPLETEDは別。通信後に結果が不明、または送信リースが失効した場合はUNKNOWNとなり自動再送しない。`POST /api/v1/deliveries/{id}/cancel` は `expectedRevision` 必須、`printers:print` scopeで取消を要求する。QUEUEDは送信claimと競合判定して即時CANCELLED（200）。ACCEPTEDは取消意図を永続化して202を返し、WorkerがCancel-Jobを1回試行する。受理だけでは取消完了とせず、状態照会でCANCELLED／COMPLETED／ABORTEDを確定する。SENDINGとUNKNOWNへの取消要求は409。取消送信中の切断・再起動では `CANCEL_DISPATCHING` を保持し、取消を自動再送せず照会を続ける。既に印刷されたページは取り消せない。UNKNOWNの解消と永続ファイルの保持期限は今後の作業。ACXの既存RIP能力は引き続きphysicalOutput=false。

## English

Configure administrator-owned printer IDs, IPP URIs and media dimensions using the example JSON. Set the same `JRIP_PRINTER_CONFIG` and `JRIP_DATA_ROOT` for API and delivery worker. POST a UUID `requestId`, `printerId`, `media` and `expectedRevision` to `/api/v1/jobs/{id}/deliveries`, requiring both `printers:print` and `artifacts:read`. The API verifies completed TIFF artifacts, converts to bounded 8-bit PWG, matches media geometry and live printer capabilities, and returns a durable delivery with HTTP 202. Starting `jrip-delivery-worker` dispatches queued deliveries; GET `/api/v1/deliveries/{id}` requires `jobs:read`. Acceptance and device completion remain separate. Ambiguous or expired sends become UNKNOWN and are never automatically resent. Current limits: 32 MiB TIFF input, 32 MiB PWG output, 100 pages, one copy, simplex, unauthenticated IPP. POST `/api/v1/deliveries/{id}/cancel` requires `expectedRevision` and `printers:print`. Queued cancellation atomically prevents dispatch (200). Accepted cancellation persists intent (202); the worker claims one Cancel-Job attempt and polls until a terminal device state. SENDING and UNKNOWN return 409. Interrupted cancellation retains CANCEL_DISPATCHING and is not automatically repeated; printed pages cannot be undone. TLS, ICC conversion, retention and MCP physical-print tools remain unfinished.

## 简体中文

参考JSON示例配置由管理员维护的打印机ID、IPP地址和纸张尺寸。API和发送Worker使用相同的 `JRIP_PRINTER_CONFIG` 与 `JRIP_DATA_ROOT`。向 `/api/v1/jobs/{id}/deliveries` POST UUID `requestId`、`printerId`、`media` 和 `expectedRevision`，需要 `printers:print` 与 `artifacts:read` 两项权限。API验证已完成的TIFF，转换为有界8位PWG，检查纸张尺寸及实时设备能力，返回HTTP 202和持久化投递记录。启动 `jrip-delivery-worker` 后发送排队任务。使用 `jobs:read` 查询 `/api/v1/deliveries/{id}`。设备受理与设备完成分别记录；不确定或租约过期的发送标记为UNKNOWN，不自动重发。目前仅支持无认证IPP、单面一份，TIFF输入及PWG输出各32MiB、最多100页。向 `/api/v1/deliveries/{id}/cancel` POST `expectedRevision`，需要 `printers:print`。排队取消原子阻止发送并返回200；设备已受理时持久化取消意图，返回202，由Worker仅尝试一次Cancel-Job并查询最终设备状态。SENDING和UNKNOWN返回409。取消通信中断后保留CANCEL_DISPATCHING，不自动再次取消，继续查询；已打印页面无法撤回。TLS、ICC转换、保留期限及MCP实体打印工具尚未实现。

## IPPS and printer credentials / IPPSとプリンター認証 / IPPS与打印机认证

Native IPP accepts `ipp://` and `ipps://` administrator-configured destinations. IPPS validates server names and certificate chains using system trust roots. No insecure verifier or redirect-following mode is provided. Optional `JRIP_PRINTER_SECURITY` points to a protected JSON file containing an array of `{ "printerId": "configured-id", "bearerToken": "device-token", "caPem": "PEM certificate contents" }`. Both optional fields may be omitted. Custom CA anchors replace system trust for that destination; `caPem` is bounded to 64 KiB and 32 certificates. Tokens are rejected for cleartext IPP. Unknown or duplicated destination IDs and invalid credentials fail startup. The printer listing excludes addresses and secrets. This implements Bearer-capable printer/proxy authentication; HTTP Digest, mTLS and inbound IPPS remain separate work.

日本語：設定済み出力先でIPPSを利用できます。システムの信頼ストアで証明書・接続先名を検証します。保護された `JRIP_PRINTER_SECURITY` JSONファイルで出力先IDごとのBearerトークンとPEM信頼証明書を指定できます。独自CA指定時はその出力先の信頼ストアを置き換えます。平文IPPへのトークン設定は拒否します。実装対象はBearer対応プリンター／プロキシで、Digest認証、相互TLS、受信側IPPSは未実装です。

简体中文：配置目标支持IPPS，并通过系统信任库验证证书链及服务器名。受保护的 `JRIP_PRINTER_SECURITY` JSON文件可按目标ID配置Bearer令牌和PEM信任证书；自定义CA会替换该目标的系统信任库。拒绝明文IPP令牌配置。当前支持Bearer打印机／代理，Digest、双向TLS及接收端IPPS尚未实现。

The local TLS test verifies trusted delivery, untrusted certificate rejection, hostname mismatch rejection, TLS-only token policy and invalid-header rejection. Physical IPPS printers still require acceptance testing.

## UNKNOWN reconciliation / 不明結果の照合 / 不确定结果核对

`POST /api/v1/deliveries/{id}/reconcile` requires admin and `{ "expectedRevision": 2, "deviceJobId": 42 }`. Only UNKNOWN attempts qualify. The configured and registered destination is queried with Get-Job-Attributes; the remote name must equal `J-RIP <delivery UUID>`. A different/missing name or unavailable remote job leaves UNKNOWN untouched. Matching terminal jobs resolve to their terminal state; matching pending/processing jobs become ACCEPTED for normal polling. No Print-Job is issued and UNKNOWN never becomes QUEUED. The revision check prevents conflicting administrator decisions. If the printer has discarded its job history, automatic proof is unavailable and UNKNOWN remains unresolved. A remote name match is operational correlation, not a cryptographic printer attestation.

日本語：管理者が実機job-idを指定し、登録済み出力先へ照会して `J-RIP <Delivery UUID>` のジョブ名が一致した場合だけUNKNOWNを復旧します。別ジョブ・履歴消失の場合は変更しません。再送は行わず、待機列にも戻しません。未完了ならACCEPTEDへ戻して照会を続け、終端なら同じ終端状態を記録します。ジョブ名による照合であり、暗号学的な実機証明ではありません。

简体中文：管理员指定实体job-id后，向已注册目标查询，只有任务名等于 `J-RIP <Delivery UUID>` 才恢复UNKNOWN。其他任务或历史已丢失时保持原状态。不重新发送，也不回到队列；未完成时恢复ACCEPTED继续查询，已结束时记录相应状态。名称匹配是操作关联，不是密码学设备证明。


### Explicit color-managed delivery / 色成果物の送信 / 颜色成果物投递

`POST /api/v1/jobs/{id}/deliveries` and MCP `print_deliver_job` accept optional `colorAttemptId`. It must be a completed attempt for the same source revision. Configure `colorTarget` on the printer with exactly one lowercase SHA-256: `outputProfileSha256` for normal ICC conversion, or `deviceLinkSha256` for a direct link. The registered result must match; geometry and PWG device capabilities still apply. A configured color target requires an explicit color attempt even if raw TIFF exists. The delivery ticket, approval grant and idempotency comparison bind the color attempt. Human approval requests must include the same `colorAttemptId`. The ordinary conversion path does not accept arbitrary ICC-tagged TIFF.

日本語：DeliveryとMCP送信に `colorAttemptId` を指定できます。同じ原本revisionの完了attemptが必要です。プリンターの `colorTarget` に通常ICCなら `outputProfileSha256`、DeviceLinkなら `deviceLinkSha256` のいずれか1つを設定してください。hashが一致し、寸法・PWG実機能力も対応する場合のみ受け付けます。colorTarget設定時は原本TIFFの直接送信を拒否します。ticket・承認・重複判定に色attemptを固定し、人の承認にも同じIDを指定します。

简体中文：Delivery及MCP可指定 `colorAttemptId`，须为同一原稿revision的已完成尝试。在打印机 `colorTarget` 中配置普通ICC的 `outputProfileSha256` 或DeviceLink的 `deviceLinkSha256`，必须且仅能设置一个。hash、尺寸及PWG设备能力匹配才接受；设置colorTarget后拒绝直接发送原始TIFF。ticket、审批及幂等比较固定颜色尝试，人工审批须包含相同ID。

```json
"colorTarget": {
  "outputProfileSha256": "<registered lowercase SHA-256>",
  "deviceLinkSha256": null
}
```

See [Color processing](COLOR-MANAGEMENT.md) for registration, worker and API details. / 登録・Worker・APIは色処理文書を参照。 / 注册、Worker及API详见颜色处理文档。

Device copy counts / 実機出力部数 / 实体输出份数: [PRINT-COPIES.md](PRINT-COPIES.md).
