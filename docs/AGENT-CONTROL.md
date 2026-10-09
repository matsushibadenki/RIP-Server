# Agent control / Agent制御 / Agent控制

## Durable audit / 永続監査 / 持久化审计

`GET /api/v1/audit?after=0&limit=50` requires `admin`; limits are 1–100. API requests record authenticated identity, matched route, allowed action name, target UUID, server-generated correlation ID, start/end time and HTTP status. `x-jrip-correlation-id` identifies the API record. Query strings, Bearer tokens, document contents, filenames and request bodies are excluded. Production stdio MCP records validated tool calls with identity, tool name, target UUID and outcome; it also uses the audited API. MCP outcome statuses are 200 for success, 403 for locally denied scopes and 400 for other tool failures, not downstream HTTP status codes. No audit admission means no operation; admission failure returns 503. An interrupted request remains an incomplete record. A completion-write failure is logged and does not replay a mutation. This is a local durable journal, not a signed or immutable external archive. Invalid MCP protocol/argument requests are rejected before tool admission.

日本語：監査一覧は管理者のみ取得できます。APIの主体、照合済みルート、操作、対象UUID、相関ID、開始・終了時刻、HTTP結果を永続化します。トークン、クエリー文字列、原稿、ファイル名、リクエスト本文を記録しません。実行前の監査保存に失敗した場合は503で拒否し、途中終了した記録は未完了のまま残ります。MCPの有効なtool呼出しも主体・tool名・対象・結果を保存します。署名済み外部アーカイブではありません。

简体中文：审计列表仅供管理员读取。持久化API身份、匹配路由、操作、目标UUID、关联ID、开始／结束时间和HTTP结果，不记录令牌、查询字符串、原稿、文件名及请求体。操作前无法记录审计时返回503；中断记录保持未完成状态。有效MCP工具调用同样记录身份、工具名、目标及结果。这是本地持久化日志，不是签名外部归档。

## Stage events / 工程イベント / 阶段事件

New job events carry `schemaVersion: 1`, `stage`, `revision`, `reasonCode`, `retryable`, `progressPercent` and `completionKind`. Metadata is written in the same transaction as the state change and remains available after job metadata deletion. Old events retain their original fields without invented historical metadata. `progressPercent` is null until TIFF export completion (100). Page-level progress is not currently measured. `completionKind: tiff-export` distinguishes RIP completion from physical delivery.

日本語：新しい履歴に工程・revision・reason codeを追加しました。状態と同じトランザクションで保存し、ジョブ削除後も残します。旧履歴の未記録情報は補完しません。進捗率はTIFF出力完了時のみ100、それ以外はnullです。ページ単位の進捗は未計測です。

简体中文：新事件包含阶段、revision和reason code，与状态更新在同一事务中保存，删除任务记录后仍保留。旧历史不补造元数据。只有TIFF导出完成时进度为100，其他情况为null；尚未测量页级进度。

| Reason | Meaning / 意味 / 含义 | Retry hint |
|---|---|---|
| NONE | Normal transition / 通常遷移 / 正常转换 | false |
| HELD_BY_USER | Held / 保留 / 已暂停 | false |
| CANCELED_BY_USER | Canceled / 取消 / 已取消 | false |
| TIFF_EXPORT_COMPLETED | File export finished / ファイル出力完了 / 文件导出完成 | false |
| LEASE_EXPIRED | Lost worker lease / リース失効 / 租约失效 | true |
| WORKER_EXIT | Worker exited / ワーカー異常終了 / Worker异常结束 | true |
| TIMEOUT | Processing timed out / 処理時間切れ / 处理超时 | true |
| UNSPECIFIED_FAILURE | No detailed diagnosis / 詳細診断なし / 无详细诊断 | false |

Interpreter error codes are preserved as reason codes. A retry hint never triggers resubmission; authenticated explicit retry and source integrity checks remain required.

## MCP operations / MCP操作 / MCP操作

- `print_resolve_intent`: negotiate settings through Print API (`jobs:submit`).
- `print_list_printers`: list configured IDs/media without device addresses or secrets (`jobs:read`); this is configuration, not live device status.
- `print_get_delivery`: read delivery state (`jobs:read`, `deliveryId`).
- `print_deliver_job`: prepare an explicit delivery (`printers:print`, `jobId`, `expectedRevision`, UUID `requestId`, configured `printerId`, `media`). The worker sends it later; this operation does not confirm paper output.
- `print_cancel_delivery`: persist cancellation (`printers:print`, `deliveryId`, `expectedRevision`).
- `rip_delete_job`: delete terminal job metadata (`admin`, `jobId`, `expectedRevision`). Artifacts, history and receipts are not physically deleted by this operation.

JSON resources: `jrip://jobs/{jobId}`, `/manifest`, `/events` require `jobs:read`; `/artifacts` requires `artifacts:read`. Only `/events` supports subscriptions. Large TIFF data stays outside MCP messages and uses the authenticated artifact API. Individual binary artifact resources, approval policy and remote MCP transport remain unfinished.

日本語：実機toolは既存のDelivery API・権限・revision・冪等性を共有します。Agentから任意のプリンターURLは指定できません。resourcesでジョブ、manifest、履歴、成果物一覧を取得し、バイナリは認証付き成果物APIを利用します。人の承認ポリシー、個別バイナリresource、リモートMCPは未完了です。

简体中文：实体投递工具复用Delivery API、权限、revision与幂等规则，Agent不能指定任意打印机URL。resources提供任务、manifest、历史及产物列表，二进制通过认证产物API取得。人工审批策略、单独二进制resource和远程MCP尚未完成。

## Physical delivery approval / 実機出力承認 / 实体投递审批

Enable `JRIP_REQUIRE_PRINT_APPROVAL=1` with an explicit `JRIP_AUTH_POLICY`. An administrator creates a five-minute approval with `POST /api/v1/approvals`, supplying `identity`, `jobId`, `expectedRevision`, UUID `requestId`, configured `printerId`, `media` and a nonempty `reason` (max 512 bytes). The completed source job, authorized identity and configured media must exist. The grant also binds the destination profile hash, including its URI and media geometry. Pass the returned ID as `x-jrip-approval-id`, or MCP `print_deliver_job.approvalId`. A missing, expired, revoked or mismatched grant produces `approval_required`. `POST /api/v1/approvals/{id}/revoke` requires admin. Validity and revocation are rechecked transactionally when enqueueing. The same delivery request ID converges to one attempt; replaying an already-enqueued attempt does not require a new grant and never adds an extra print.

Keep administrator credentials separate from AI credentials if a human approval boundary is required. This policy covers delivery admission only; revocation does not cancel a delivery already queued or accepted. Use delivery cancellation for that. Deletion and persistent settings approval are not implemented yet. Default is disabled for compatibility; legacy admin-only configurations cannot create identity-bound grants.

日本語：`JRIP_REQUIRE_PRINT_APPROVAL=1` と明示的な認証ポリシーで、実機出力の承認を必須にできます。管理者が主体・完了ジョブ・revision・attempt UUID・出力先・用紙・理由を指定して5分間の承認を作成します。出力先URIと寸法もhashで固定し、失効・取消・流用を拒否します。Agentは承認IDを指定して投入し、同じattemptの再要求は二重印刷になりません。人による承認を必要とする運用では管理者資格情報をAIから分離してください。承認取消は投入済みDeliveryを取り消さないため、取消APIを使用します。削除・設定変更の承認は未実装です。

简体中文：使用 `JRIP_REQUIRE_PRINT_APPROVAL=1` 和明确认证策略可要求实体投递审批。管理员指定身份、已完成任务、revision、attempt UUID、目标、纸张及原因，签发五分钟审批。目标URI和尺寸通过hash绑定，拒绝过期、撤销或挪用。Agent提供审批ID；同一attempt重放不会重复打印。需要人工审批时，应将管理员凭据与AI凭据分离。撤销审批不会取消已入队Delivery，需使用取消API。删除及设置变更审批尚未实现。

`GET /api/v1/printers/{id}`, MCP `print_get_printer` and `jrip://printers/{printerId}` query a configured device's live Native IPP capabilities with `jobs:read`. Replies include the model, a hash-based capability revision, observation time and a 30-second freshness window. Model revisions bind the configured URI without disclosing it. Arbitrary paths/URLs are rejected as printer IDs. Supply levels are not currently exposed.

日本語：設定済み実機の能力・受付可否・状態理由をAPI／MCP／resourceから取得できます。revisionと30秒の有効期限を付け、URIや認証情報を返しません。消耗品残量は未取得です。

简体中文：通过API／MCP／resource读取已配置设备的能力、是否接收及状态原因，附带revision和30秒有效期，不返回URI或凭据。尚未提供耗材余量。


Color control / 色処理制御 / 颜色控制：`color_get_capabilities` describes supported formats, policy constraints, limits, capability revision and expiry; worker availability is explicitly not probed. `color_apply` queues an immutable, source-revision-bound attempt; `color_get_attempt`, `color_get_events`, `color_cancel_attempt` and `color_get_profile` delegate the scoped API. The original RIP artifact remains intact, and physical delivery can select a completed `colorAttemptId`. See [Color processing](COLOR-MANAGEMENT.md). / 色処理能力・制約・上限・版・期限を公開し、Worker稼働状態は未確認と明示します。色attemptは原本revisionを固定し、完了後に実機送信へ選択できます。 / 发布颜色能力、约束、限额、revision及期限，明确Worker状态未探测。颜色尝试固定原稿revision，完成后可选用于实体投递。
