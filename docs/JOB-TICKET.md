# Common JobTicket and capabilities / 共通契約 / 通用契约

## 日本語

新規REST・IPP・MCP投入は共通の公開処理でJobTicket v0.1を保存する。ジョブ・原稿hash・サイズ・manifest・選択エンジン・DPI・色型・完了条件を同一トランザクションで記録する。`GET /api/v1/jobs/{id}/ticket` は `jobs:read` が必要で、再起動やジョブメタデータ削除後も記録を返す。導入前のジョブへ証跡を推測して補完しないため、記録がない場合は404。

`POST /api/v1/print/validate-ticket` は `jobs:submit` による設定照合のみ。取得・構造検証・投入は行わない。原稿hashの整合性確認と構造検証を区別し、呼び出し側による `structureVerified` / `documentVerified` の真値は拒否する。現在のticketは1部・片面・全ページ・TIFF出力用で、実機完了を意味しない。

`GET /api/v1/print/capabilities` はRIP・Color・Queueを同じ型で公開する。`printerId` を指定すると設定済みプリンターへ能力照会し、PWG形式・DPI・色型・設定済み用紙との共通部分を返す。revisionは能力内容に結び付き、観測時刻を含めない。有効期限は30秒。未照会工程は `not_probed`。能力取得は認可・原稿検証・実行の保証ではない。物理PrintIntentのpreflight・commitとpipeline workerは実装済み（PIPELINE.md参照）。

MCPでは `print_get_capabilities`（任意のprinterIdで実機も照会）、`print_get_job_ticket`、`print_validate_ticket` を提供する。実機のライブ照会は既存の `print_get_printer` を使用する。PrintIntentにエンジン・ページ範囲・numberUp・面付け・品質を追加し、色方針・優先度・期限にもrequirement指定を受け付ける。未対応必須条件は競合、推奨・自動条件は変更内容と代替案を返す。フィールドの表現対応は機能の実装を意味しない。

## English

New REST, IPP and MCP submissions persist a v0.1 JobTicket atomically with the job. It binds source integrity evidence, manifest, engine and raster completion meaning. Authenticated ticket reads survive restart and metadata deletion; historical jobs without records return 404. Pure ticket validation neither downloads nor structurally validates a source and rejects caller assertions of structural verification.

Common capability snapshots expose RIP, Color and Queue, plus live configured Printer capabilities when REST `printerId` is supplied. Revisions bind capability contents; snapshots expire after 30 seconds. Discovery grants no execution authorization. Physical intent preflight, commit and pipeline orchestration are implemented (see PIPELINE.md). MCP exposes common discovery, ticket reads and pure validation. Unsupported required intent settings produce conflicts; preferred or automatic settings produce explicit adjustments and alternatives.

## 简体中文

新REST、IPP及MCP作业通过共用提交流程，在同一事务中持久化v0.1 JobTicket，绑定原稿完整性、manifest、引擎和栅格导出完成含义。认证查询在重启或删除作业元数据后仍可读取；旧作业无记录时返回404。设置验证不下载或结构验证原稿，并拒绝调用者宣称结构已验证。

通用能力快照包含RIP、Color、Queue；REST指定 `printerId` 时加入已配置设备的实时能力。revision绑定能力内容，快照30秒过期。能力发现不授予执行权限。实体意图预检、提交及流程编排已实现（见PIPELINE.md）。MCP提供能力发现、契约查询及纯设置验证。不支持的必需设置返回冲突，推荐或自动设置明确返回调整和替代方案。

## Delivery intersection / 送信条件の交差照合 / 投递条件交叉匹配

`deliveryIntersection` は共通能力照会に含まれる。TIFF→PWG bridge、離散DPIとRIP範囲、直接送信可能な色型、ICC処理で出力可能な色型、8bit、設定用紙、最小の容量上限を照合する。不一致は `pipeline_*` reason code、プリンター利用不可は再試行可能な `printer_unavailable` を返す。プリンター未指定時は `printer_capability_required`。`settingsCompatible` は原稿・geometry・profile適合・実行権限を保証しない。`runtimeProbed=false` と残る検査項目を返し、自動連携はpipeline APIで明示的に投入する。

Common REST and MCP discovery now includes `deliveryIntersection`: the TIFF→PWG bridge, discrete DPI within RIP limits, direct color spaces, ICC output spaces, 8-bit layout, configured media and the minimum artifact budget. Conflicts carry stable reason codes. Settings compatibility does not attest document geometry, profile compatibility, runtime readiness or authorization; remaining checks are explicit. MCP accepts an optional configured printer ID for live discovery. Explicit pipeline submission provides automatic orchestration.

通用REST与MCP能力查询新增 `deliveryIntersection`，检查TIFF→PWG转换、RIP范围内的离散DPI、直接颜色空间、ICC输出空间、8bit、配置纸张和最小产物容量。冲突返回稳定原因代码。设置兼容不证明原稿尺寸、profile适配、运行就绪或授权；剩余检查明确列出。MCP可选传入已配置设备ID进行实时查询。显式提交流程可进行自动编排。

## Physical delivery ticket / 実機送信契約 / 实体投递契约

`POST /api/v1/jobs/{id}/delivery-preview` に `requestId`、`printerId`、`media`、`expectedRevision`、必要なら `colorAttemptId` を指定する。`jobs:read` と `artifacts:read` が必要。完成済み原稿の成果物を検証し、PWGへ変換してDPI・色型・用紙寸法とライブ能力を照合する。キュー投入や実機への送信は行わず、`contract.ticket` と `contract.ticketSha256` を返す。previewには一時スナップショット・変換処理のコストがある。

`POST /api/v1/jobs/{id}/deliveries` に同じ設定と `expectedTicketSha256` を渡すと、再検査後の契約が異なる場合は409 `delivery_ticket_conflict`。既存クライアントはhashを省略できる。承認必須構成では従来どおり、対象actor・原稿revision・requestId・プリンター・用紙・色attemptに結び付いた承認が必要。previewは承認を付与しない。契約にはジョブID、送信attempt ID、原稿hash/revision、色attempt、PWG hash/サイズ、プリンター設定hash、片面1部、device-confirmed完了条件を含む。`GET /api/v1/deliveries/{id}/ticket` は確定契約を照会する。契約と配信キューを同じSQLiteトランザクションで保存し、再起動後も保持する。プリンターURIは公開せず設定hashに含める。旧配信で契約が保存されていない場合は404。

MCPの `print_preview_delivery` と `print_get_delivery_ticket` を追加し、`print_deliver_job` に `expectedTicketSha256` を追加。実機への自動連携と投入前原稿の物理PrintIntent解決はpipeline APIで提供する。共通契約の照会は後述。

**English:** `delivery-preview` verifies a completed job's artifact integrity, PWG layout, media geometry, color target and live device capabilities without queueing or printing. It requires job and artifact read scopes and returns a versioned physical contract with SHA-256. Pass its hash as `expectedTicketSha256` to delivery submission to reject changed settings or artifacts. Submission rechecks authorization and approval. The committed contract and queue entry are atomic and survive restart; the authenticated delivery ticket endpoint returns this evidence. Legacy deliveries without contracts return 404. MCP exposes preview and contract reads. This is a delivery-stage contract; physical intent execution and automatic orchestration are provided by the pipeline API.

**简体中文：** `delivery-preview` 验证已完成作业的产物完整性、PWG布局、纸张尺寸、颜色目标及设备实时能力，不加入队列或打印。需要作业与产物读取权限，返回带版本的实体契约及SHA-256。提交时传入 `expectedTicketSha256`，设置或产物改变时拒绝；授权与审批重新检查。契约与队列在同一事务中保存，重启后保持，认证ticket端点可读取。旧投递无契约时返回404。MCP支持预览和契约读取。当前为投递阶段契约，pipeline API提供自动编排与实体PrintIntent执行。

## Unified pipeline ticket / 全工程の契約照会 / 全流程契约查询

`GET /api/v1/pipelines/{id}/ticket`（`jobs:read`）とMCP `print_get_pipeline_ticket` は、保存済みRIP JobTicket、原稿hash・プリンター設定hashを拘束した登録契約、色処理契約、確定済み実機送信契約を一つのSQLite読取snapshotから返す。profileは `org.jrip.print.unified-ticket-v1`。原稿ID・hash・出力先の不一致は409。プリンターURI・認証情報を追加しない。新たな実行権限や承認は付与しない。

`ticketSha256` は `ticket` のserde JSONを対象とし、工程の契約が追加された時点で変わる。状態とrevisionは別の `execution` に含むため、契約確定後の状態進行ではhashが変わらない。`contractFinalized` は実機送信契約の保存を表し、印刷完了ではない。`execution.completed` はpipelineとdeliveryがともにCOMPLETEDで、実機契約がある場合のみtrue。UNKNOWNはfalse。Device-confirmedはIPPデバイスの完了報告を意味し、紙面の目視検査ではない。

旧ジョブのRIP証跡は推測せず `raster=null`、`rasterEvidenceAvailable=false` とする。送信前は `physical=null`。再起動やジョブメタデータの削除後も保存済み契約を照会できる。ラスター検証APIは引き続きラスター契約のみを受け付け、この全工程照会形式を投入や承認として扱わない。

**English:** REST and MCP return an explicitly versioned aggregate of persisted raster, registration, color and physical contracts from one SQLite read snapshot. Source and destination mismatches fail with 409. Missing legacy raster evidence and uncommitted delivery contracts remain null. The ticket digest changes when a stage contract is added, but excludes mutable execution states and revisions. Finalized means the physical contract exists; completed additionally requires both workflow and delivery completion. UNKNOWN is never completion. Device confirmation is the IPP device report, not visual paper inspection. Reads survive restart and job metadata deletion, confer no authority, and do not change raster validation semantics.

**简体中文：** REST与MCP从同一SQLite读取快照返回有版本的栅格、注册、颜色及实体契约集合。原稿或目标不一致时返回409。旧作业缺失证据和未提交实体契约保留为null。阶段契约加入时摘要变化，运行状态及revision不进入摘要。Finalized仅表示实体契约存在；completed还要求流程与投递均完成。UNKNOWN不是完成。设备确认指IPP完成报告，不是纸面目视检验。重启或删除作业元数据后仍可读取，不授予权限，也不改变栅格验证规则。
