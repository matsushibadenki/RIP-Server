# 実装ロードマップ / Roadmap / 开发路线图

- [Done] implemented in the current codebase / 実装済み / 已实现
- [Next] high-priority unfinished work / 優先して取り組む未完了の作業 / 优先待完成
- [Later] planned, but not the closest next step / 後続の作業 / 后续计划

最初の利用目標は、同じLANのMacからJ-RIPを選択してPDFジョブを投入し、RIP結果をファイルとして確認できること。その後、実機プリンターへ出力する。並行して、**Agentが印刷機を操作できるPrint Infrastructure**として、Agent → Print API → RIP → Color Management → Queue → Printerを標準化する。AIクライアントはMCP経由で各層の能力を発見し、印刷意図の指定、検証、投入、監視、成果物確認、取消、再実行まで一貫して操作できる。IPP受付、Print API、MCP制御、RIP、色管理、キュー、プリンターバックエンドは交換可能な境界として分離する。仕様書のMVP 1 / MVP 2はまだ完成していない。

## 現在の基盤

- [Done] Rust Workspace、Job Core / Storage / Interpreter / Serverの境界。
- [Done] PDF / PS / EPSのREST API受付、入力シグネチャ照合、SHA-256、32 MiB上限の逐次アップロード。
- [Done] SQLite WALによるジョブ永続化、状態変更イベント、revisionによる競合防止、保留・解除・取消API。
- [Done] 共通Bearer認証。アプリケーションエラーの英語・日本語・简体中文メッセージ。
- [Done] PDFはMuPDF、PS/EPSはGhostscriptを既定とし、PDFでのGhostscript明示指定も可能。選択したエンジンをジョブに保存。
- [Done] RGB / CMYK / GrayのTIFFファイル出力、コンテナー隔離、リソース上限、タイムアウト。ワーカーはジョブIDを指定して1件ずつ起動。
- [Done] 完成した出力ディレクトリを一括公開。現在の `COMPLETED` は**TIFFファイル出力完了**を意味し、紙への印刷完了ではない。
- [Done] ローカルDockerでMuPDF / Ghostscript両イメージを構築し、2ページPDFの色・ページ順序、実PS変換、ページ上限、破損PDF、タイムアウト、処理後のコンテナー削除を結合テストで確認。
- [Done] RESTとIPPで共用するジョブ投入処理、PDF限定のIPPメッセージ解析・受付、6つの操作、IPP job-idと内部UUIDの永続対応。
- [Done] LAN公開を明示的に有効化した場合の汎用 `_ipp._tcp` DNS-SD広告。広告するポート・パス・PDF形式とIPP受付の整合性を検証。
- [Done] IPP属性の型・重複・順序を検証し、`requested-attributes` 付きのプリンター／ジョブ照会で要求された属性だけを返す。
- [Done] IPPの300/600 dpiとカラー／モノクロ指定をmanifestへ反映し、能力照会とジョブ照会にも同じ値を返す。未対応値は拒否。
- [Done] `Get-Jobs` でIPPジョブを未完了／完了済みに分けて照会し、件数・要求属性を絞れる。返した `job-uri` から照会・取消できる。
- [Done] `Validate-Job` でPDF形式とRIP設定を `Print-Job` と共通の規則で検証し、ジョブを作成せず結果を返す。
- [Done] 実験段階の `jrip-dispatcher` がREST・IPPの待機ジョブを優先度順に選び、既存ワーカーを1件ずつ起動する。競合時はワーカーのrevisionチェックで二重処理を防ぐ。
- [Done] ディスパッチャーの二重起動をファイルロックで拒否し、実行IDが一致する監督中ワーカーの異常終了だけを `FAILED` に確定する。
- [Done] ワーカーリースをSQLiteへ永続化し、10秒ごとのハートビートと30秒の期限を管理する。ディスパッチャー再起動後、期限切れの処理中ジョブを `FAILED/LEASE_EXPIRED` へ復旧する。

## [Next] 第1段階：PDFを受けるIPP仮想プリンター

この段階は印刷プロトコルの受付を検証する**ファイル出力用の仮想プリンター**とする。実機出力やIPP Everywhere / AirPrint適合を完了と表示しない。

1. [Done] **受付を共有する。** `crates/rip-ipp` にIPPメッセージ・属性・応答コード・ジョブ変換を置き、`services/jrip-ipp` に `POST /ipp/print` の受信を置く。RESTとIPPは共通のジョブ投入サービス・バリデーション・保管処理を使い、IPPから内部RESTへHTTPで折り返さない。RIP CoreにIPP固有の型を持ち込まない。
2. [Done] **PDF限定の最小プロトコルを動かす。** `Get-Printer-Attributes`、`Validate-Job`、`Print-Job`、`Get-Jobs`、`Get-Job-Attributes`、`Cancel-Job` を実装した。IPPの版・operation-id・request-id・属性グループ・必須属性・文書データ境界を検証し、不正要求や未対応の形式には明確なIPPエラーを返す。受信したPDFは既存のMuPDF経路で処理できる。`Create-Job`、`Send-Document` は後続のクライアント互換テストで必要になれば追加する。
3. [Next] **能力を正確に公開する。** 現在の `document-format-supported` は `application/pdf` のみ。属性の型・重複・順序と、`requested-attributes` に応じた照会は実装済み。解像度は300/600 dpi、カラーは `color`（CMYK）/ `monochrome`（Gray）を実際のmanifestに反映する。用紙・部数・両面などは、受理して実際に反映できる値だけを `Get-Printer-Attributes` とDNS-SDのTXTに掲載する。未対応値を黙って捨てず、`copies`、`media`、`sides` などを提供する前にmanifest・出力経路・検証を揃える。
4. [Next] **ジョブ状態を一貫させる。** IPP job-idと内部UUIDの永続対応、単一ホストの順次ディスパッチ、監督中ワーカーの異常終了検知、期限切れリースの失敗復旧は実装済み。IPPの取消・照会を既存ジョブの状態に結び、投入・RIP・ファイル出力・失敗を返す。残存コンテナー回収、自動再試行と、仮想プリンターUIでのファイル出力先の明示が残る。実機がない段階で紙への印刷成功を返さない。
5. [Next] **LANで発見できるようにする。** 汎用 `_ipp._tcp` 広告と `rp=ipp/print`・`pdl=application/pdf` は実装済み。実際のMacでの発見・手動追加とWindowsからの接続を検証する。TLSと証明書を用意した後に `_ipps._tcp` / IPPSを追加する。送信先プリンターの探索は別機能とする。
6. [Next] **受付の安全性を確認する。** 受信サイズ、属性数、入れ子・文字列長、タイムアウト、同時接続数を制限する。IPPS時の認証・アクセス範囲を設計し、既存RESTのBearerトークンをプリンタークライアントへ要求する前提にしない。

**第1段階の完了条件（未達）:** IPPプロトコルテストで属性照会・PDF投入・ジョブ照会・取消を検証する。Macの同一LANでサービス発見と手動追加を確認し、1ページ・複数ページPDFが正しい内部ジョブとTIFFへ到達する。Windowsの手動IPP追加・標準クラスドライバーからの印刷も実機で検証し、送られる文書形式がPDFでない場合はその形式の対応を別タスクとして記録する。接続できることとAirPrint / IPP Everywhereへの適合は別に判定する。

## [Next] Agent Print Infrastructureの標準契約

```text
Agent
  ↓ MCP / Agent SDK
Print API
  ↓ PrintIntent + JobTicket
RIP
  ↓ RasterArtifact + RenderingReport
Color Management
  ↓ ColorManagedArtifact + ColorReport
Queue
  ↓ DeliveryTicket
Printer
  ↑ DeviceState + SupplyState + PrintResult
```

すべての層を単一プロセスへ固定せず、versioned schemaと安定した識別子で接続する。1つの `job_id`、`correlation_id`、`attempt_id` を全工程で引き継ぎ、Agentが「受付済み」「RIP完了」「色変換完了」「送信済み」「印刷中」「排紙完了」「失敗」を区別できるようにする。

1. [Next] **Print APIを正規の入口にする。** REST、IPP、MCPから受けた要求を、共通の `PrintIntent` と `JobTicket` へ正規化する。原稿参照、部数、ページ範囲、用紙、面付け、片面／両面、解像度、色意図、品質、優先度、期限、出力先、承認条件を表現し、各フィールドを `required`、`preferred`、`automatic` のいずれかとして指定できるようにする。能力不足時は黙って変更せず、拒否または具体的な代替案を返す。
2. [Next] **共通Capability Modelを定義する。** RIP、Color Management、Queue、Printerが能力と制約を同じ形式で公開する。入力・出力MIME、最大寸法、DPI、色空間、ICC profile、レンダリングインテント、用紙、両面、部数、仕上げ、現在の利用可否を表し、`capability_revision` と有効期限を付ける。Print APIは全工程の交差部分を計算し、実行可能なticketと不一致理由をAgentへ返す。
3. [Next] **工程間artifact契約を固定する。** `SourceDocument`、`RasterArtifact`、`ColorManagedArtifact`、`DeliveryArtifact` をcontent-addressed artifactとして扱い、SHA-256、MIME、ページ数、寸法、色空間、profile、生成元、保持期限を必須メタデータにする。大容量データはサービス間メッセージへ埋め込まず、許可されたobject storeまたはローカルartifact storeの参照で渡す。
4. [Next] **状態とイベントを工程単位にする。** ジョブ全体の状態に加えて `VALIDATING`、`RIPPING`、`COLOR_MANAGING`、`QUEUED_FOR_DEVICE`、`SENDING`、`PRINTING`、`COMPLETED`、`PARTIALLY_COMPLETED`、`CANCELED`、`FAILED` を標準化し、工程、進捗、ページ、reason code、再試行可否をイベントとして記録する。`COMPLETED` は実機から確認できた完了条件をticketへ記録し、現在のTIFF出力完了とは別に扱う。
5. [Next] **Color Managementを独立サービス境界にする。** ICC profile登録、入力profile判定、出力profile選択、レンダリングインテント、黒点補正、黒版保持、DeviceLink、TACを `ColorPolicy` として版管理する。変換結果には使用profileのhashと `ColorReport` を付け、Agentが再現性、警告、gamut、インキ制限を確認できるようにする。初期実装はLittleCMSを候補とし、RIPエンジン固有の暗黙変換を明示的に記録する。
6. [Next] **Queueを実機制御の責任境界にする。** 優先度、期限、依存関係、保留、取消、回数制限付き再試行、device affinity、fairnessを扱う。RIP再実行とプリンター再送を別attemptとして記録し、同じページの重複印刷を防ぐdelivery idempotency keyを導入する。用紙切れ、インク／トナー、カバー開放、オフライン、jamなどを構造化reason codeとして上位へ伝える。
7. [Next] **Printer Adapterを標準化する。** `PrinterBackend` traitで能力取得、検証、送信、ジョブ照会、取消、device/supply状態を統一し、最初の実装をNative IPPとする。PWG Raster、PDF、JPEGなどは実機能力に応じて選択し、ベンダー固有機能はnamespaced extensionとして隔離する。CUPSやメーカーSDKも同じ契約へ追加できるようにする。
8. [Next] **ポリシーと観測性を全工程へ通す。** identity、tenant、scope、承認、費用上限、ページ上限、カラー印刷可否、データ所在地、保持期限を `ExecutionPolicy` として評価する。すべての判断、ticket変更、artifact、attempt、device応答を監査し、OpenTelemetry traceと同じcorrelation idで結ぶ。Agentには機密情報を除いた構造化診断と回復可能な次の操作を返す。
9. [Later] **公開仕様と適合テストを整備する。** Print APIのOpenAPI、MCP schema、JobTicket JSON Schema、PrinterBackend conformance suite、reason code registry、サンプルAgent SDKを英語・日本語・简体中文で公開する。IPP / PWGの既存標準へ対応付け、独自項目には安定したnamespaceを使う。

**Infrastructure段階の完了条件（未達）:** Agentが自然言語の印刷意図から能力照合済みJobTicketを作り、RIP、明示的な色変換、永続キュー、Native IPP実機出力を通して排紙結果まで追跡できる。途中の再起動、Agentの切断、プリンター障害、用紙切れ、取消、再試行でも同じjob-idから工程とattemptを復元でき、重複印刷を起こさない。各境界の契約テストと、模擬プリンターを使うend-to-endテストを必須とする。

## [Next] AIから直接利用するMCP制御面

`services/jrip-mcp` を独立プロセスとして追加し、MCP固有の型や接続処理をRIP Coreへ持ち込まない。最初はローカルAIエージェント向けのstdio transportを提供し、認証・TLS・接続制限を整えた後にリモート向けStreamable HTTPを追加する。MCPサーバーはSQLiteやDockerを直接操作せず、既存のJob Service、Repository、Dispatcher、将来のPrinter Backendを型付きアプリケーション境界から呼び出す。

1. [Next] **自己記述可能な能力を公開する。** MCPの `tools/list` とresourcesから、Print APIの共通Capability Model、入力形式、MuPDF / Ghostscript、Color Policy、解像度、色空間、ページ・容量上限、キュー、プリンター能力、device/supply状態、現在の負荷、機能フラグを機械可読JSON Schemaで返す。未実装の能力は公開せず、スキーマへ版番号を付ける。英語・日本語・简体中文の表示名、説明、エラー回復案を提供する。
2. [Next] **RIP操作を細粒度のtoolsにする。** `rip_validate`、`rip_submit`、`rip_get_job`、`rip_list_jobs`、`rip_watch_job`、`rip_hold_job`、`rip_release_job`、`rip_cancel_job`、`rip_retry_job`、`rip_delete_job` を提供する。投入時は原稿、エンジン、DPI、色空間、ページ範囲、優先度、出力先を明示できるようにし、検証だけを行うdry-runとidempotency keyを用意する。更新系toolは期待revisionを受け取り、AIの並行操作による上書きを防ぐ。
3. [Next] **成果物と診断情報をresourcesにする。** `jrip://jobs/{id}`、`jrip://jobs/{id}/events`、`jrip://jobs/{id}/manifest`、`jrip://jobs/{id}/artifacts`、`jrip://jobs/{id}/artifacts/{name}`、`jrip://printers/{id}` を定義する。TIFF、サムネイル、プリフライト結果、エンジンログを列挙し、大容量成果物はMCPメッセージへ埋め込まず、期限付きURLまたは範囲取得可能なresourceとして渡す。ジョブイベント購読と進捗通知に対応し、ポーリング間隔をAI側へ強制しない。
4. [Next] **AIが安全に印刷仕様を組み立てられるようにする。** `print_resolve_intent`、`print_validate_ticket`、`rip_get_presets`、`rip_compare_engines`、`rip_estimate`、`rip_create_preview` を提供し、実行前に能力照合済みJobTicket、代替案、予想ページ数、ラスター寸法、色変換、メモリ・ディスク使用量、印刷機の利用可否を返す。永続preset、Color Policy、キュー、実機プリンター設定の変更は別scopeに分け、変更前後の値と監査理由を必須にする。任意のシェルコマンド、Docker引数、ホストパスは受け付けない。
5. [Next] **権限と人の承認境界を実装する。** `jobs:read`、`jobs:submit`、`jobs:control`、`artifacts:read`、`printers:print`、`settings:write`、`admin` のscopeを設ける。ローカルstdioも呼出元identityを監査し、リモートtransportは短期トークン、TLS、レート制限、入力容量制限を必須にする。ファイル出力は許可済みroot配下に限定する。実機印刷、ジョブ削除、永続設定変更はポリシーで人の承認を要求でき、MCP応答は `approval_required` と具体的な影響を返す。
6. [Next] **長時間処理と再接続を標準化する。** tool呼出しは短時間で永続job-idを返し、RIP処理をMCPセッションの寿命へ結び付けない。AIが切断・再接続してもjob-idとidempotency keyから処理を追跡できるようにする。構造化エラーには安定したcode、再試行可否、推奨待機時間、現在revisionを含め、失敗時に別エンジンへ自動変更せず、AIが差分を確認して明示的に再実行する。
7. [Later] **複数RIPノードの統合制御へ拡張する。** MCP Gatewayから複数のJ-RIP Workerを能力・負荷・データ所在で選択できるようにする。Cloud側と独立WorkerはHTTPS APIまたはジョブキューで疎結合にし、WorkerからManaged PostgreSQLへの直接接続に依存しない。認証情報や原稿をモデルの会話履歴へ露出させず、データ所在地と保持期限をポリシーで制御する。

**MCP段階の完了条件（未達）:** MCP Inspectorと2種類以上のAIクライアントから、能力発見、dry-run、PDF投入、進捗通知、成果物取得、保留・解除・取消、失敗診断、revision競合、idempotency、再接続をend-to-endで検証する。権限不足と承認待ちを構造化応答として確認し、MCP経由でホストの任意ファイル、任意コマンド、Docker APIへ到達できないことをテストする。すべての更新操作を主体、tool名、引数の要約、結果、job-id、時刻とともに監査できる状態を完了とする。

## [Next] 第2段階：実機への出力

- [Next] `PrinterBackend` と設定済み出力先を導入する。IPPクライアント受付と、J-RIPから実機へ送信するNative IPPバックエンドを分離する。
- [Next] TIFFからプリンターが実際に受理する形式への変換を実装する。PWG Rasterを優先し、ページ順序、用紙、解像度、カラーモードを能力照合してから送る。
- [Next] 実装済みのリースと失敗復旧を基盤に、残存コンテナー回収と回数制限付き再試行を加え、送信完了とプリンター側のジョブ結果を区別する。部数・両面などはバックエンドで反映可能になったものから順に公開する。
- [Next] Mac / Windowsの実機印刷、取消、障害、再起動を通し、クライアント表示と実際の印刷結果が一致することを確認する。
- [Next] IPP Everywhereを名乗る前に、必要なPWG Raster入力と、カラー機で必要なJPEG入力、属性・DNS-SD・セキュリティ要件を満たし、PWGの自己認証テストを実行する。PDFだけの受付では適合を主張しない。

## [Later] 印刷品質と製品機能

- [Later] `Create-Job` / `Send-Document` の複数文書とApple URF入力をクライアント互換性に応じて追加する。未実装の形式を広告しない。
- [Later] 日本語CID/CMap・縦書き・IVSとフォント置換レポート、EPS / 日本語のGolden Master。
- [Later] LittleCMSによるICC変換、黒版保持、DeviceLink、TAC、オーバープリント、分版プレビュー。
- [Later] CMYK16、タイルRIP、メモリ予算、ハーフトーン、DeviceN、特色、Trapping。
- [Later] React管理UI（英語・日本語・简体中文）にRIP・プリンター状態と専門設定を集約する。必要ならWindows Print Support Appを検討する。
- [Later] 実機プリンター探索・能力取得、CUPS連携、PostgreSQL Repository、ロール認可、監査、保持期限と容量管理。
- [Later] Linuxサービス・パッケージ、SBOM／ライセンス監査、必要時のみHTTPSで接続するクラウドControl Plane。

参考規格: [PWG IPP Everywhere](https://pwg.org/ipp/everywhere.html)、[PWG自己認証ツール](https://www.pwg.org/ippeveselfcert/)。Mac / Windowsの実際の対応状況は、対象OSでの結合テストを完了条件とする。

## English / 简体中文

**English:** The target is an Agent-operated Print Infrastructure with standardized boundaries from Agent → Print API → RIP → Color Management → Queue → Printer. A versioned PrintIntent, JobTicket, Capability Model, artifact contract, event model, ColorPolicy, queue contract, and PrinterBackend let an Agent negotiate executable settings and follow one job through physical output. MCP is the AI control plane, while REST and IPP remain protocol adapters over the same application services. The design includes idempotency, optimistic concurrency, resumable jobs, delivery deduplication, audit records, scoped authorization, and approval policies. `COMPLETED` currently means TIFF export only until physical-printer completion is implemented separately.

**简体中文：** 目标是建立由Agent操作的Print Infrastructure，标准化Agent → Print API → RIP → Color Management → Queue → Printer的全部边界。通过带版本的PrintIntent、JobTicket、Capability Model、成果物契约、事件模型、ColorPolicy、队列契约和PrinterBackend，Agent可以协商可执行的打印设置，并使用同一个job-id追踪到实体输出。MCP作为AI控制面，REST和IPP则作为共用应用服务之上的协议适配器。设计包含幂等、乐观并发控制、可恢复作业、防止重复打印、审计、分级授权及审批策略。在单独实现实体打印完成状态之前，当前的 `COMPLETED` 仍仅表示TIFF文件导出完成。
