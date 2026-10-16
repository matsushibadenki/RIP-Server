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

## Owner-bound preflight and authorization evidence / 主体拘束と認可証跡 / 身份绑定与授权证据

English: New settings preflights persist actor and authorizationEvidence (mechanism, requiredScopes, scopeCheck and purpose) inside their digest-bound payload. Create/get/commit require jobs:submit, and get/commit—including committed replay—require the same authenticated identity. Admin scope does not bypass owner binding. Two current credentials for the same identity can rotate without invalidating the preflight; scope checks still apply to each request. Physical preflights retain their existing owner check and now carry evidence for the required four workflow scopes. Retry requires submit/control and binds its durable requestId to the actor before returning an idempotent result.

New acceptance receipts identify scoped-bearer/shared-bearer or historical actor-bound-bearer correctly and copy actor plus the stored scope-check evidence into proof. These fields describe the preflight scope check, not an execution token, human approval or complete document-structure validation. Existing commit/runtime/approval checks remain necessary. Tokens and the caller's full scope grant list are not persisted. Stored historical receipts are not rewritten.

Historical settings/retry records without actor remain available only in shared-token mode. With an explicit credential policy, they fail with preflight_actor_required and callers must create a new bound request; malformed or different actors fail with preflight_actor_mismatch. Records are not assigned an invented historical owner. Existing JCS v2 and explicitly negotiated legacy v1 hashing rules are retained. Three-language messages explain the recovery action. This implements the owner/scope evidence milestone, not complete ACX lifecycle conformance or tenant isolation of all jobs.

日本語：新規設定preflightのactorと認可証跡をdigest対象に含め、取得・commit・再送時に同じidentityと現在のjobs:submitを確認します。管理者も別主体のpreflightを流用できません。同一identityのトークン更新は可能ですが、操作ごとのscopeを再検査します。物理preflightの4つのworkflow scope、retryのsubmit/controlも証跡へ記録し、retryのrequestIdを主体に拘束します。

受付Receiptには方式・主体・保存済みscope検査証跡を記録します。tokenや全付与scopeは保存せず、この証跡を実行token・人の承認・原稿構造検証とは扱いません。旧Receiptを書き換えません。actorのない旧設定／retryは共有tokenモードのみで利用でき、明示的policyでは新しい要求の作成が必要です。JCS v2と旧v1のhash規則は維持します。ACX lifecycle全体や全jobのtenant分離の完了とは扱いません。

简体中文：新设置预检的actor及scope检查证据纳入摘要，读取、commit及重放均检查相同identity及当前jobs:submit。管理员scope不能绕过归属。同identity可轮换token，但每次重新检查权限。实体预检记录4个workflow scope，retry记录submit/control并将requestId绑定身份。

接收回执记录机制、主体及保存的scope检查证据，不保存token或完整授权列表，也不将证据视为执行token、人工审批或原稿结构验证。历史回执不重写。无actor的旧设置／retry仅在共享token模式可用；显式policy下须创建新请求。保持JCS v2及旧v1摘要规则，不宣称完整ACX lifecycle或全部job的tenant隔离已完成。

### Measured page progress / 実測ページ進捗 / 实测页级进度

The standard worker emits `PAGE_ARTIFACT_VERIFIED` after hashing each complete TIFF.
`progressPhase: artifact-verification`, `completedPages`, `totalPages`, and
`progressPercent` describe that phase only. Rendering has already finished; 100%
does not mean publication, color conversion, or physical printing has completed.
The event keeps state `RIPPING`, never advances the control revision, and requires
the current unexpired worker lease. Events are durable and resumable through the
existing job history, SSE, MCP, and unified pipeline timeline. Color conversion emits its own measured phase as described below; unmeasured
device page counts remain null. Earlier progress remains evidence of work
performed even if later publication fails or cancellation wins.

標準workerは各TIFF全体のハッシュ計算後に `PAGE_ARTIFACT_VERIFIED` を記録します。
ページ数・進捗率は成果物検証工程のみを表し、描画中の進捗ではありません。
100%でも公開・色変換・紙への印刷完了を意味しません。状態は `RIPPING` のまま、
操作revisionも変更しません。有効な実行リースを検査し、履歴・SSE・MCP・共通timelineに
永続化します。色管理は下記の別工程として計測し、未計測の実機ページ数はnullです。後の公開失敗や取消でも、
それまでの作業記録は残ります。

标准worker在完整读取并计算每页TIFF哈希后记录 `PAGE_ARTIFACT_VERIFIED`。
页数与百分比仅表示成果验证阶段，而非渲染中的进度；100%不代表发布、色彩转换或实机打印完成。
状态保持 `RIPPING`，不改变控制revision，并检查当前有效worker租约。
历史、SSE、MCP与统一时间线均可恢复读取。色彩转换采用下述独立阶段，未测量的设备页数保留null；
后续发布失败或取消不会删除已经完成的工作记录。


Color events now include `progressPhase: color-conversion` and `totalPages`
(`pageCount` remains for compatibility). A page is counted after the worker has
validated its conversion report, geometry, profile and staged raster. This is
before final hashing and publication: a RUNNING event may reach 100%, while
`completionMeaning` remains null. Only COMPLETED confirms color-managed export.
Known counts cannot exceed the ticket, skip pages or regress; RUNNING/COMPLETED
updates require an unexpired lease and the expected revision. Terminal failures
preserve the last measured count. Old events are not rewritten.

色管理イベントは `progressPhase: color-conversion` と `totalPages` を返します。
互換性のため `pageCount` も保持します。変換report・寸法・profile・保存済みラスターを
workerが検査したページだけを数えます。最終ハッシュ計算と公開は後続のため、
RUNNINGの100%は公開完了ではありません。総数超過・飛び越し・逆戻りを拒否し、
RUNNING／COMPLETEDの更新には有効なリースと期待revisionが必要です。
失敗時は最後の実測数を保持し、過去のイベントは書き換えません。

色彩事件新增 `progressPhase: color-conversion` 与 `totalPages`，并保留 `pageCount`。
worker检查转换报告、尺寸、profile及暂存光栅后才累计页数。最终哈希与发布尚未完成时，
RUNNING也可达到100%，但不表示成果已发布。拒绝超限、跳页和倒退；
RUNNING及COMPLETED更新须具备有效租约与预期revision。失败保留最后实测页数，历史事件不变。


Live rendering progress is now available in `progressPhase: rendering`, before
artifact verification. See [rendering progress](RENDER-PROGRESS.md) for the adapter
protocol, unknown PS/EPS totals and failure/cancellation semantics.

描画中のページ進捗は `progressPhase: rendering` で取得できます。
成果物検証前の計測であり、[描画進捗仕様](RENDER-PROGRESS.md)に未知の総数・失敗・取消の意味を定義します。

渲染过程的页级进度使用 `progressPhase: rendering`，发生于成果验证之前。
未知总数、失败及取消的含义见[渲染进度规范](RENDER-PROGRESS.md)。
