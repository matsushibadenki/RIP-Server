# Follow-on pipeline / RIP後の処理連携 / RIP后续流程

## 日本語

受付済みRIPジョブに対して、`POST /api/v1/jobs/{id}/pipelines` でICC処理から実機送信までの連携を予約する。`requestId`、RIPジョブの `expectedRevision`、設定済み `printerId`、`media`、必要なら `color`（`inputProfileSha256`、`outputProfileSha256`、明示的 `policy`）を指定する。jobs:read、jobs:submit、artifacts:read、printers:printが必要。同じrequestIdと同じ主体・条件は同じworkflowへ対応付け、条件が異なる再送は拒否する。

ワーカーを同じデータroot・プリンター設定・認可設定で起動する。

```sh
cargo run -p jrip-server --bin jrip-pipeline-worker
cargo run -p jrip-server --bin jrip-color-worker
cargo run -p jrip-server --bin jrip-delivery-worker
```

各コマンドは別プロセスで動かす。`JRIP_DATA_ROOT`、`JRIP_API_TOKEN`、`JRIP_PRINTER_CONFIG`、必要なら `JRIP_AUTH_POLICY`、`JRIP_REQUIRE_PRINT_APPROVAL=1`、`JRIP_PRINTER_SECURITY` を既存ワーカーと揃える。pipeline workerはDockerや実機送信を直接実行せず、既存の永続キューへ接続する。認可・プリンター設定は各プロセスの起動時構成であり、変更を反映する際は各サービス／ワーカーを再起動する。

`GET /api/v1/pipelines/{id}` で状態を確認する。主な遷移は QUEUED → RIP（未完了時）→ COLOR（ICC指定時）→ WAITING_APPROVAL（承認必須時）→ DELIVERY → COMPLETED。COLORとdeliveryのchild attempt IDはworkflowのrequestIdと同じUUIDで、それぞれ別のテーブルへ保存する。RIP job-idは `sourceJob` に保持する。終了状態は実機からのjob完了応答を意味し、RIP側のTIFF完了とは区別する。

承認必須構成では、色処理完了後に既存の `/api/v1/approvals` で対象主体、sourceJob/revision、workflow UUID、printer/media、ICC指定時はworkflow UUIDのcolorAttemptIdに結び付いた承認を作る。`POST /api/v1/pipelines/{id}/resume` にworkflowの `expectedRevision` と `approvalId` を渡す。resumeは作成主体のみ許可し、次のworker passで既存承認規則を再検査する。承認の期限切れ・撤回時は再び待機する。Bearer tokenはDBへ保存せず、workerは作成主体に対応する現在の構成済みcredentialと必要scopeを工程開始前に再確認する。権限喪失や設定変更はBLOCKED。BLOCKEDも主体が条件を解決してresumeできるが、固定したプリンター設定の変更を自動的に受け入れない。

送信結果不明はUNKNOWNで保持し、送信を自動再試行しない。既存delivery reconcileで結果が確定した後、workerが状態を追従する。既に存在するchildを照会するため、工程間でプロセスが終了しても二重投入しない。処理中・承認待ち・BLOCKED・UNKNOWNの原稿はmetadata削除とretentionから保護する。

`POST /api/v1/pipelines/{id}/cancel` は作成主体のjobs:controlとprinters:printが必要で、workflowのexpectedRevisionを指定する。送信attempt作成前に限り取消でき、待機中の色attemptは同時に取り消す。色処理実行中は競合、送信attempt作成後は既存delivery cancelを使う。既存childの状態はworkerが追従する。

MCPは `print_create_pipeline`、`print_get_pipeline`、`print_resume_pipeline`、`print_cancel_pipeline` を提供する。受付済みジョブのRIP完了待ちと工程共通イベントに対応する。原稿投入前の実機向けPrintIntent解決と一括受付にも対応する（後述）。分散ワーカー・公平性は残作業。

## English

Create a durable pipeline for an accepted RIP job with requestId, expectedRevision, configured printerId/media and optional explicit ICC parameters. It requires job read/submit, artifact read and printer print scopes. Run pipeline, color and delivery workers as separate processes sharing configuration and data root. The pipeline worker queues existing services; it neither runs Docker nor sends to a device directly.

The workflow UUID identifies color and delivery child attempts in separate tables. Immutable requests, pinned printer settings, durable CAS states and idempotent child APIs let orchestration resume after restart. Tokens are never persisted; current startup-configured actor credentials and scopes are rechecked. Approval is obtained after color completion and attached through owner-only resume. Missing approval waits; revoked authority or changed configuration blocks; uncertain sends stay UNKNOWN and are never automatically retried. Active workflows protect their source from deletion and retention. Cancellation before delivery requires control and print scopes; running color and existing delivery attempts use the documented conflict/child cancellation paths. MCP exposes create/read/resume/cancel. Bound physical PrintIntent preflight and atomic acceptance are also available (below); distributed scheduling remains unfinished.

## 简体中文

对已接收RIP作业创建持久化流程，指定requestId、expectedRevision、已配置printerId/media及可选显式ICC参数。需要作业读取/提交、产物读取及打印权限。pipeline、color及delivery worker作为独立进程使用相同配置和数据目录；编排worker只连接已有队列，不直接运行Docker或投递设备。

流程UUID作为颜色与投递子attempt ID，分别存储于不同表。不可变请求、固定设备设置、持久化CAS状态与幂等子API支持重启恢复。token不保存至DB；开始工程前重新检查启动配置中的主体权限。颜色完成后取得审批并通过仅限创建主体的resume附加；无审批则等待，权限撤销或配置改变则BLOCKED，不确定投递保持UNKNOWN且不会自动重发。活动流程的原稿受到删除与保留期限清理保护。投递前取消需要控制与打印权限；运行中颜色及既有投递按冲突或子attempt取消路径处理。MCP提供创建、查询、恢复及取消。也支持下述上传前实体意图预检及原子接收；分布式调度仍待实现。

## Stage history / 工程履歴 / 阶段历史

`GET /api/v1/pipelines/{id}/events?after=0&limit=50` はjobs:readで取得できる。`nextAfter` を次の `after` へ渡して差分を取得し、最大100件ずつ読む。永続sequenceはworkflow間で共有するため、番号の連続性を仮定しない。状態更新と履歴保存は同じトランザクションで、再送や失敗したrevision競合は履歴を増やさない。状態、revision、工程、reason、復旧ヒント、完了種別を記録する。`progress` はnullで、ページ進捗は既存color eventsを参照する。完了種別device-confirmedはプリンターのjob完了応答を意味する。導入前のworkflowの過去履歴を補完しない。MCPは `print_get_pipeline_events` を提供する。現在はworkflowの工程遷移であり、RIP・色処理・実機の全イベントを単一ストリームへ統合したものではない。

**English:** Read authenticated persistent pipeline history with `after` and `limit` (1–100), then continue from `nextAfter`. Sequence numbers are global to pipeline history and can have gaps. State changes and events commit atomically; replay and rejected CAS updates add no events. Records include stage, reason, revision, recovery hints and completion meaning. Historical transitions are not backfilled, and per-page color progress remains in color events. MCP exposes `print_get_pipeline_events`; this is orchestration history, not yet a merged RIP/color/device event stream.

**简体中文：** 使用认证接口和after/limit（1–100）读取持久化流程历史，以nextAfter继续增量查询。序号在所有流程历史间共享，允许有间隔。状态与事件原子提交；重复请求及失败CAS不会增加事件。记录工程、原因、revision、恢复提示及完成含义。旧流程历史不补写；颜色页级进度仍使用color events。MCP新增 `print_get_pipeline_events`。当前是编排历史，尚未合并全部RIP/颜色/设备事件。

## Unified timeline / 全工程timeline / 统一阶段timeline

`GET /api/v1/pipelines/{id}/timeline?after=0&limit=50` とMCP `print_get_pipeline_timeline` は、原稿RIP・対応する色attempt・対応するdelivery・workflow履歴を同じenvelopeで返す。jobs:readが必要。`nextAfter` と `hasMore` で最大100件ずつ差分取得する。各履歴のINSERTと同じSQLiteトランザクション内のtriggerで共通sequenceを付け、再起動後も値を保持する。順序はDBへの記録順で、時刻順への並び替えや未確認工程の推測はしない。

初回導入時の既存履歴は一度だけ取り込み、`historicalImport=true` とする。過去履歴の異なる工程間の発生順序は復元できないため、取り込み順を実際の発生順序と解釈しない。`origin.kind`、`subjectId`、元sequenceを保存し、既存の各履歴APIも維持する。詳細が保存されていない旧履歴のrevision・reason・進捗はnullで、現在値から補完しない。新しいdeliveryイベントはrevision・reason・deviceJobIdも同時保存する。

共通フィールドはstage、state、reasonCode、revision、progressPercent、completionMeaning、retryable。RIP完了はraster_export、色処理はcolor_managed_export、プリンターまたはworkflow完了はdevice-confirmed。workflow状態とchild状態は別イベントとして保持し、完了を二重実行と解釈しない。原稿・色attempt・deliveryの対応関係を確認してから対象timelineへ含める。MCP push通知と受付済みRIPの完了待機にも対応する（後述）。HTTP SSEと原稿投入前の解決は残作業。

**English:** The authenticated timeline endpoint and MCP tool expose RIP, matching color/delivery attempts and workflow transitions in one envelope, with a stable arrival-order sequence, origin references and resumable pagination. Triggers index source-history inserts atomically. Existing history is imported once and explicitly marked historical; cross-stage historical chronology cannot be reconstructed. Missing legacy metadata stays null. New delivery events retain revision/reason/device ID. Completion meanings distinguish raster, color export and device-confirmed completion. Existing stage APIs remain available; push notifications and pre-RIP orchestration are unfinished.

**简体中文：** 认证timeline接口及MCP工具统一返回RIP、匹配的颜色/投递attempt和流程事件，提供稳定的记录顺序序号、来源引用和增量分页。trigger在原历史INSERT事务中原子建立索引。旧历史仅导入一次并明确标记；无法恢复旧事件跨阶段发生顺序，缺失元数据保持null。新投递事件保存revision/原因/设备ID。完成含义区分栅格导出、颜色导出及设备确认完成。各阶段原接口保留；MCP push与已接收RIP作业的完成等待也已支持（见下文）；HTTP SSE与上传前解析仍待实现。

## MCP timeline notifications / MCP更新通知 / MCP更新通知

`jrip://pipelines/{pipelineId}/timeline` はjobs:readで参照・購読できるJSON resource。`resources/subscribe` にURIを渡すと、既存イベントまたは新しいイベントがあれば `notifications/resources/updated` を受け取る。解除は `resources/unsubscribe`。既存のjob eventsと合わせて1接続32購読まで。サーバーは内部で約1秒ごとに永続履歴を確認するため、クライアントから追加リクエストを送らなくても通知する。通知はURIのみの更新ヒントで、イベント本体や厳密な一度だけの配送を保証しない。

クライアントは `print_get_pipeline_timeline` の `nextAfter` を保存し、通知時に保存済みafterから差分取得する。再接続後も同じcursorで読み続ける。resourceのreadは最初のページを返すため、継続取得にはtoolのafter/limitを使う。subscribeのcursorは接続内のみで、永続的なクライアントcursorを代替しない。通知がない間は静かに待機する。HTTP SSEやremote MCP transportのpush対応は後続。

**English:** Subscribe to the JSON timeline resource through MCP. The server checks durable history every second and sends standard resource-updated URI hints without further client requests. Job-event and timeline subscriptions share a 32-per-session limit. Save the timeline tool's nextAfter cursor and fetch deltas after notifications or reconnect; resource reads return the first page. Hints may be coalesced or repeated and are not exactly-once event delivery. HTTP SSE and remote MCP push remain unfinished.

**简体中文：** 可通过MCP订阅JSON timeline resource。服务端每秒检查持久化历史，无需客户端继续请求即可发送标准resource-updated URI提示。作业与timeline共用每连接32个订阅上限。保存timeline工具的nextAfter，在通知或重连后增量读取；resource read返回第一页。通知可能合并或重复，不代表严格一次事件配送。HTTP SSE及remote MCP push尚待实现。

## RIP completion gate / RIP完了待ち / 等待RIP完成

**日本語：** QUEUED、HELD、RIPPINGなどの受付済みジョブにも登録でき、未完了時はworkflowがRIP状態で待機する。保留を自動解除せず、既存RIP workerが処理する。FAILED/CANCELLEDになれば後続attemptを作らずworkflowをFAILEDにする。登録時の原稿SHA256とmanifestを固定し、完了時に一致を検査する。登録のexpectedRevisionは受付時のsource revisionで、色処理・送信・承認には完了後のsource revisionを使用する。同じrequestId/主体/条件の再送はsource revisionが進んでも元のworkflowを返す。再起動後も待機を継続する。既存の別操作に加え、下記の一括受付を利用できる。

**English:** Pipelines may be registered against accepted queued, held or running RIP jobs. The RIP state waits for the existing worker without releasing holds. Failed or cancelled sources fail the workflow before child attempts are queued. Source SHA256 and manifest are pinned at registration and checked before proceeding. Registration expectedRevision identifies the original source revision; color, delivery and approval use the completed source revision. Identical requests replay the original workflow even after source revisions advance. Waiting survives restart. The atomic submission endpoint below is also available.

**简体中文：** 可对已接收、排队、暂停或运行中的RIP作业注册流程。RIP状态等待现有worker，不自动解除暂停。源作业失败或取消时，流程失败且不创建后续attempt。注册时固定源SHA256和manifest，继续前核对。注册expectedRevision表示当时源revision；颜色、投递与批准使用完成后的源revision。源revision推进后，相同请求仍返回原流程。等待支持重启恢复；也可使用下述原子提交接口。

## Atomic submission / 原稿とworkflowの一括受付 / 原子提交

`POST /api/v1/print/submissions` accepts multipart fields `manifest`, `pipeline` and `document`, in any order. `pipeline` uses the registration request above with `expectedRevision: 0`. Response: HTTP 202 with `jobId` and `pipeline`. All four registration scopes are required. Source limit: 32 MiB; each JSON field: 16 KiB. Unknown/duplicate fields are rejected. The existing `/api/v1/jobs` upload API remains available.

**日本語：** 原稿を検証・保存してから、job、JobTicket、workflow、両工程の受付イベントを同一DBトランザクションで登録する。workerは両方の登録完了後にジョブを取得する。requestIdは固定して再送する。同じ主体・原稿SHA256・manifest・処理条件・プリンター設定は元のjobIdと現在のworkflowを返す。異なる条件は409で拒否し、重複ジョブを残さない。失敗・再送時に新しく作った原稿ディレクトリは削除する。プロセスが突然終了した場合の未登録ディレクトリは既存の孤立データ清掃で回収する。ファイルシステムとDB自体が単一トランザクションになるという意味ではない。受付は形式の検出と原稿の整合性検査であり、PDF構造検証や実機完了を意味しない。承認・ICC処理・送信の既存検査を省略しない。

**English:** Seal the validated source before atomically publishing the job, JobTicket, workflow and acceptance events in SQLite. Workers see only committed entries. Replay a stable requestId with identical actor, source hash, manifest, options and printer configuration to recover the original jobId/current workflow; changed conditions return 409 without duplicate jobs. Failed/replayed staging directories are removed; abrupt-exit orphans use existing retention cleanup. Filesystem and DB are not one transaction. Acceptance verifies source integrity and format detection, not PDF structure or device completion. Existing ICC, approval and delivery checks remain enforced.

**简体中文：** 验证并保存源文档后，在同一SQLite事务中发布job、JobTicket、流程和接收事件。worker只能看到已提交作业。固定requestId，相同主体、源哈希、manifest、参数及设备配置重放返回原jobId与当前流程；不同条件返回409且不留下重复作业。失败或重放时删除新暂存目录；异常退出的孤立目录由现有清理机制回收。文件系统与DB并非同一事务。接收仅检测格式和源完整性，不代表PDF结构验证或设备完成；ICC、审批及投递检查继续生效。

MCP `print_submit_pipeline` accepts `{sourceName, manifest, pipeline}`. It is exposed only with a configured input inbox and all four scopes; sourceName is a single inbox filename, never an arbitrary path. / 原稿inbox設定と全scopeが必要で、任意パスは指定できない。 / 需要源inbox配置及全部权限，不接受任意路径。

## Bound physical PrintIntent / 実機向けPrintIntentの接続 / 实体PrintIntent绑定

1. `POST /api/v1/print/pipeline-preflights` (JSON limit 16 KiB) with `{requestId, intent, color?}`. Intent uses `output: {destination: "printer", printerId: "configured-id"}` and a SHA256-pinned HTTPS document reference. This reference describes the source; the server never downloads it. Optional intent.idempotencyKey must equal requestId. The explicit ICC request has the same structure as pipeline registration.
2. Inspect the returned resolved `ticket.manifest`, `workflowRequest`, `adjustments`, `capabilityRevision`, `preflightId` and `preflightDigest`.
3. `POST /api/v1/print/pipeline-preflights/{preflightId}/commit` with `x-jrip-preflight-digest` and multipart `document` only. The resolved manifest and workflow cannot be overridden at commit. Response contains `jobId`, `pipeline`, `replayed`. Initial acceptance is 201; replay is 200.

**日本語：** 事前確認は実機の現在の能力と構成済み用紙の交差から解像度・入力色・用紙を解決する。requiredが未対応なら422と代替案を返す。preferredの変更はadjustmentsへ明記し、automaticまたは未指定は対応する値を選ぶ。部数1・片面・全面出力など他の条件は共通resolverで検査する。ICC指定時は入力色をpolicyへ合わせ、出力色が実機に対応すること、登録profileとpolicyの組合せを確認する。原稿の寸法はこの時点で不明なので、TIFF/PWG生成時の正確な用紙寸法検査は継続する。

作成・読取・commitは全4scopeと作成主体への紐付けを必要とする。有効期限は能力snapshotに合わせ30秒。commit時にdigest、期限、主体、プリンター設定、最新能力、原稿SHA256を再検査し、アップロード後にも能力を確認する。原稿・job・JobTicket・workflow・preflight消費・受付イベント・receiptは既存の一括登録経路で対応付ける。同じpreflightの並行commitは1件だけを登録する。成功後の再送は期限経過後も元のjobIdを返す。receiptは受付済みを示し、実機完了を証明するものではない。既存のRIP専用commitでは実機向けpreflightを受け付けない。

**English:** Negotiate DPI, input color and configured media against live device capabilities. Unsupported required choices return 422 with alternatives; preferred adjustments are explicit. Automatic/omitted choices select supported defaults. The shared resolver checks remaining raster settings. Explicit ICC binds input/output colors and validates registered profiles/policy using a synthetic pixel. Exact document geometry is still checked after RIP. All four scopes and owner binding apply. Preflights expire with the 30-second capability snapshot. Commit rechecks digest, identity, expiry, configuration, live capabilities and source SHA256, including a capability check after upload. Source/job/ticket/workflow/preflight consumption/events/receipt publish through one DB transaction. Concurrent commits create one job; successful replay survives expiry. Acceptance receipts do not prove device completion. Raster-only commit rejects physical preflights.

**简体中文：** 按实时设备能力解析DPI、输入色与配置纸张。required不支持时返回422及替代方案；preferred调整明确记录。automatic或省略项选择支持的默认值，其他栅格条件由共用resolver检查。显式ICC绑定输入/输出色，并以合成像素验证注册profile和policy；RIP后仍检查文档准确尺寸。全部4项权限及创建主体绑定适用。预检随能力snapshot在30秒后过期。commit复核digest、主体、期限、配置、实时能力和源SHA256，上传后再次检查能力。源文档/job/ticket/流程/预检消费/事件/receipt通过同一DB事务发布。并发commit只生成一个作业，成功后的重放不受原期限影响。接收receipt不证明设备完成；仅RIP的commit拒绝实体预检。

MCP: `print_create_pipeline_preflight` takes `{requestId, intent, color?}`; `print_commit_pipeline` takes `{preflightId, preflightDigest, sourceName}` and reads only from the configured inbox. / commitはinbox内ファイル限定。 / commit仅限配置inbox内文件。

## PDF inspection evidence / PDF原稿の検査証跡 / PDF检查证据

Trusted local CLI (same data root and credential configuration as the server):

```sh
cargo run -p jrip-server --bin jrip-inspect-document -- /absolute/path/source.pdf
```

Configure `JRIP_DATA_ROOT` and `JRIP_API_TOKEN` before running. The credential needs jobs:submit (or admin). Rebuild the image after updating: `docker build -t jrip-mupdf:local adapters/mupdf`.

**日本語：** CLIは最大32 MiBの原稿を私有snapshotへコピーし、ハッシュを確定してから、ネットワークなし・512 MiBメモリ・1 CPU・PID制限・読み取り専用のMuPDFコンテナーで検査する。100ページ、出力64 KiB、全体120秒を上限とし、通常終了・失敗・timeoutでコンテナーを削除する。コピーは既存のホスト共有1 GiB snapshot予算を使用する。cleanup失敗時は検査結果を保存せず、snapshotを保持してCLIへ場所を表示する。検査はページ内容をdisplay listへ解釈し、画像を生成しない。実効ページ寸法はCropBox、継承した回転、UserUnitを含めMuPDFに解決させる。暗号化PDF、修復警告・内容解釈警告、上限超過は採用しない。特定のMuPDFビルドのICC機能未搭載警告だけを許容し、ICC処理は既存の色エンジンで別途検証する。

成功するとinspectionId、sourceSha256、sourceBytes、各ページのwidthPoints/heightPoints、contentParsedを保存して返す。`GET /api/v1/print/document-inspections/{id}` とMCP `print_get_document_inspection` はjobs:submitと同じ作成主体で読取可能。`print_create_pipeline_preflight` または対応APIに `inspectionId` を追加すると、主体・原稿SHA256・全ページの用紙寸法を確認し、その証跡をpreflightへ固定する。commitでも原稿との対応を再検査する。寸法は送信時と同じくDPI換算で1 pixelの許容差を用いる。

この検査はMuPDFでのparse・内容解釈と寸法検査の証跡であり、PDF/A・PDF/Xなどの規格適合認証ではない。既存JobTicketのstructureVerified/documentVerifiedを真へ書き換えない。inspectionIdは後方互換のため省略可能で、省略時はこの追加検査を実行しない。HTTP/MCPから検査原稿を非同期受付するqueue/workerは未実装。PostScript/EPSも検査CLIの対象外。プロセス強制終了やDocker cleanup失敗で残った検査snapshotの専用回収は残作業。

**English:** The trusted CLI copies at most 32 MiB to a private hash-pinned snapshot, using the shared 1 GiB host budget. Inspection runs only in a networkless, read-only MuPDF container with 512 MiB/1 CPU/PID limits, 100 pages, 64 KiB output and 120-second timeout. It parses page content into display lists without rasterizing, and obtains effective bounds including inherited crop/rotation and UserUnit. Encrypted PDFs, repair/content warnings and resource excesses fail. Only the pinned build's missing-ICC warning is allowed; explicit color processing remains separate. Containers are removed on ordinary success/failure/timeout. Cleanup failure publishes no evidence and retains the snapshot.

Stored evidence is owner-bound and available through the authenticated API/MCP reader. Supply optional inspectionId to physical pipeline preflight to pin source hash and check every page against configured media, with the same one-pixel tolerance as delivery. Commit verifies that binding again. Evidence means MuPDF parse/content/geometry checks, not PDF/A or PDF/X certification; shared ticket verification flags remain false. Omission preserves legacy behavior. Asynchronous HTTP/MCP inspection submission, PS/EPS support, and dedicated recovery of inspection snapshots after abrupt exit/cleanup failure remain unfinished.

**简体中文：** 信任的本地CLI将最多32 MiB源文档复制到私有哈希绑定snapshot，并使用共享1 GiB预算。在无网络、只读、512 MiB/1 CPU/PID受限的MuPDF容器中检查，限制100页、64 KiB输出及120秒。内容解析为display list，不生成栅格；有效页面尺寸包含继承的CropBox/旋转与UserUnit。加密PDF、修复/内容警告或资源超限拒绝，仅允许固定构建的ICC未搭载警告。成功、失败或timeout均清理容器；清理失败不发布证据并保留snapshot。

保存的证据绑定创建主体，可通过认证API/MCP读取。在实体preflight中指定可选inspectionId，会核对源哈希和每页纸张尺寸（与投递相同的一像素容差），并固定证据；commit再次验证。证据仅表示MuPDF解析/内容/尺寸检查，并非PDF/A或PDF/X认证，通用ticket验证标志保持false。省略时维持旧行为。异步HTTP/MCP检查接收、PS/EPS支持及异常退出后snapshot专用回收仍待实现。

MuPDF reference: [Page bounds and display lists in the 1.21 API](https://mupdf.readthedocs.io/_/downloads/en/1.22.0/pdf/).
