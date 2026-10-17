# 実装ロードマップ / Roadmap / 开发路线图

- 🟢 [Done] implemented in the current codebase / 実装済み / 已实现
- 🟠 [Next] high-priority unfinished work / 優先して取り組む未完了の作業 / 优先待完成
- 🔴 [Later] planned, but not the closest next step / 後続の作業 / 后续计划
- ⭕️ [Pending] cannot be tested in the dev environment; on hold / 開発環境で検証できないため保留 / 开发环境无法验证，暂缓

- 🟢 [Done] Agent向けジョブ状態履歴API：`GET /api/v1/jobs/{id}/events?after=0&limit=50`。Bearer認証、永続sequenceによる差分取得、最大100件、再起動・メタデータ削除後の履歴参照に対応。状態変更と実測したページ成果物検証進捗を記録する。 / Authenticated, persistent, resumable state history / 认证、持久化、可恢复的状态历史。
- 🟢 [Done] 新規履歴に工程・reason code・revision・再試行ヒント・TIFF完了種別を同一トランザクションで保存。未計測の工程の途中進捗率はnull。旧履歴は元の情報を保持。 / Durable stage and reason metadata / 持久化阶段与原因元数据。
- 🟢 [Done] TIFFのページ単位の検証進捗を永続化し、REST履歴・SSE・MCPと既存のRIP／色管理／実機共通timelineへ接続。進捗はartifact-verification工程だけを示す。 / Durable measured TIFF verification progress across event transports and the unified timeline / 持久化TIFF逐页验证进度，并接入事件接口及统一时间线。
- 🟢 [Done] 色管理のページ変換・出力検査進捗を `color-conversion` として共通timelineに統合。総数・順序・リースを検査し、変換100%と成果物公開完了を区別。 / Measured color-conversion progress with bounded counts, ordered updates and lease checks; publication remains separate / 色彩转换进度接入统一时间线，检查总数、顺序与租约，并区分转换完成和成果发布。
- 🟢 [Done] MuPDF／Ghostscriptの描画中ページ計測。閉じられたページ出力を容量制限付きstreamで受信・検査し、`rendering` 進捗を永続化。原稿ログは使用せず、PDF総数は事前検査、PS/EPSの未確定総数・率はnull。取消・後続失敗・timeoutでは公開せず、観測済み進捗だけを残す。 / Live bounded page streaming from both engines, durable observations without trusting document logs / 两引擎实时页输出流与持久化进度，不信任原稿日志。 [詳細 / Details / 详情](RENDER-PROGRESS.md)
- ⭕️ [Pending] 実機のページ進捗はbackendが確認できる場合だけ公開する。実機受入試験は未実施。 / Device page progress requires backend evidence and real-printer acceptance / 实机页级进度须有设备依据，尚待实机验收。

最初の利用目標は、同じLANのMacからJ-RIPを選択してPDFジョブを投入し、RIP結果をファイルとして確認できること。その後、実機プリンターへ出力する。並行して、**Agentが印刷機を操作できるPrint Infrastructure**として、Agent → Print API → RIP → Color Management → Queue → Printerを標準化する。AIクライアントはMCP経由で各層の能力を発見し、印刷意図の指定、検証、投入、監視、成果物確認、取消、再実行まで一貫して操作できる。IPP受付、Print API、MCP制御、RIP、色管理、キュー、プリンターバックエンドは交換可能な境界として分離する。仕様書のMVP 1 / MVP 2はまだ完成していない。

## 現在の基盤

- 🟢 [Done] Rust Workspace、Job Core / Storage / Interpreter / Serverの境界。
- 🟢 [Done] PDF / PS / EPSのREST API受付、入力シグネチャ照合、SHA-256、32 MiB上限の逐次アップロード。
- 🟢 [Done] SQLite WALによるジョブ永続化、状態変更イベント、revisionによる競合防止、保留・解除・取消API。
- 🟢 [Done] 共通Bearer認証。アプリケーションエラーの英語・日本語・简体中文メッセージ。
- 🟢 [Done] PDFはMuPDF、PS/EPSはGhostscriptを既定とし、PDFでのGhostscript明示指定も可能。選択したエンジンをジョブに保存。
- 🟢 [Done] RGB / CMYK / GrayのTIFFファイル出力、コンテナー隔離、リソース上限、タイムアウト。ワーカーはジョブIDを指定して1件ずつ起動。
- 🟢 [Done] 完成した出力ディレクトリを一括公開。現在の `COMPLETED` は**TIFFファイル出力完了**を意味し、紙への印刷完了ではない。
- 🟢 [Done] ローカルDockerでMuPDF / Ghostscript両イメージを構築し、2ページPDFの色・ページ順序、実PS変換、ページ上限、破損PDF、タイムアウト、処理後のコンテナー削除を結合テストで確認。
- 🟢 [Done] RESTとIPPで共用するジョブ投入処理、PDF限定のIPPメッセージ解析・受付、6つの操作、IPP job-idと内部UUIDの永続対応。
- 🟢 [Done] LAN公開を明示的に有効化した場合の汎用 `_ipp._tcp` DNS-SD広告。広告するポート・パス・PDF形式とIPP受付の整合性を検証。
- 🟢 [Done] IPP属性の型・重複・順序を検証し、`requested-attributes` 付きのプリンター／ジョブ照会で要求された属性だけを返す。
- 🟢 [Done] IPPの300/600 dpiとカラー／モノクロ指定をmanifestへ反映し、能力照会とジョブ照会にも同じ値を返す。未対応値は拒否。
- 🟢 [Done] `Get-Jobs` でIPPジョブを未完了／完了済みに分けて照会し、件数・要求属性を絞れる。返した `job-uri` から照会・取消できる。
- 🟢 [Done] `Validate-Job` でPDF形式とRIP設定を `Print-Job` と共通の規則で検証し、ジョブを作成せず結果を返す。
- 🟢 [Done] 実験段階の `jrip-dispatcher` がREST・IPPの待機ジョブを優先度順に選び、既存ワーカーを1件ずつ起動する。競合時はワーカーのrevisionチェックで二重処理を防ぐ。
- 🟢 [Done] ディスパッチャーの二重起動をファイルロックで拒否し、実行IDが一致する監督中ワーカーの異常終了だけを `FAILED` に確定する。
- 🟢 [Done] ワーカーリースをSQLiteへ永続化し、10秒ごとのハートビートと30秒の期限を管理する。ディスパッチャー再起動後、期限切れの処理中ジョブを `FAILED/LEASE_EXPIRED` へ復旧する。
- 🟢 [Done] `rip-acx` と `/.well-known/acx.json` を追加し、現在のTIFF出力能力だけをACX 0.1 Manifestとして公開。ACX側にnamespaced extensionsと英語・日本語・简体中文の実験的PrintIntent profile、schema、検証ベクターを追加し、両実装間のschema検証を完了。

## 第1段階：PDFを受けるIPP仮想プリンター（Mac専用PPDで検証済み、Windows・driverless受入試験待ち）

この段階は印刷プロトコルの受付を検証する**ファイル出力用の仮想プリンター**とする。実機出力やIPP Everywhere / AirPrint適合を完了と表示しない。

1. 🟢 [Done] **受付を共有する。** `crates/rip-ipp` にIPPメッセージ・属性・応答コード・ジョブ変換を置き、`services/jrip-ipp` に `POST /ipp/print` の受信を置く。RESTとIPPは共通のジョブ投入サービス・バリデーション・保管処理を使い、IPPから内部RESTへHTTPで折り返さない。RIP CoreにIPP固有の型を持ち込まない。
2. 🟢 [Done] **PDF限定の最小プロトコルを動かす。** `Get-Printer-Attributes`、`Validate-Job`、`Print-Job`、`Get-Jobs`、`Get-Job-Attributes`、`Cancel-Job` を実装した。IPPの版・operation-id・request-id・属性グループ・必須属性・文書データ境界を検証し、不正要求や未対応の形式には明確なIPPエラーを返す。受信したPDFは既存のMuPDF経路で処理できる。`Create-Job`、`Send-Document` は後続のクライアント互換テストで必要になれば追加する。
3. 🟢 [Done] **能力を正確に公開する。** PDFのみ、300/600 dpi、CMYK color／Gray monochromeを公開しmanifestへ反映。属性の型・重複・順序・requested-attributesを検査。printer-state、job-state、operations-supportedはenum型。1部・片面・1面付け・仕上げなしの固定値のみ受理・広告し、用紙変更と他の値は拒否する。
4. 🟢 [Done] **ジョブ状態を一貫させる。** 永続IPP job-id／UUID、順次Dispatcher、競合防止、取消、異常終了・期限切れリースのFAILED復旧を実装。J-RIP専用ラベルと原稿パスで残存コンテナーを限定回収する。Web説明ページ・printer-more-info・job-state-messageでTIFFファイル出力と保存先を明示。1/2ページPDFのIPP→実HTTP→Dispatcher→Docker MuPDF→TIFF、状態照会、再起動後のID対応をMac標準ipptoolで検証。
5. 🟢 [Done] **LANで発見できるようにする。** 汎用 `_ipp._tcp`、`rp=ipp/print`、`pdl=application/pdf` を広告し、Mac標準 `dns-sd` によるLAN上の発見とLANアドレスへのipptool接続・PDF投入を確認。IPPS/TLSは後続transport工程。
6. 🟢 [Done] **受付の安全性を確認する。** PDF32MiB、属性領域16KiB、128属性、名前255byte、値4096byte、同時4要求、要求全体30秒。過負荷503・時間切れ408・不正URI拒否を検証。LANは明示的に許可した場合だけ公開し、REST Bearerを通常プリンタークライアントへ要求しない。IPPS時の証明書・認証・アクセス制御の方針は[IPP-PROOF.md](IPP-PROOF.md)に記録。

- 🟢 [Done] `scripts/start-proof-printer.sh` でAPI／IPP／Dispatcherを一括起動。実機Delivery Workerは別起動。
- 🟢 [Done] Mac専用PDFパススルーPPD、標準CUPSキュー登録、システム設定での一覧・種類表示、lp→原稿hash一致→2ページTIFF→CUPS完了を実測。再現用 `scripts/test-native-cups.py` とCUPS既定属性・限定MIMEヒントの回帰テストを追加。 / Native Mac CUPS passthrough verified / 已验证Mac原生CUPS透传。
- 🟢 [Done] Windows / Mac / Linux向けの標準ライブラリのみの直接IPP受入クライアントを追加。能力照会、PDF検証・投入、今回のジョブのみの取消、終端状態追跡、SHA-256付きJSONレポートを実装。ローカル実HTTP・MuPDFで確認済み。OS標準ドライバーの受入とは区別する。 / Portable direct IPP acceptance client and JSON evidence / 跨平台直接IPP验收客户端与JSON证据。
- 🟢 [Done] CUPS受入テストにアプリ投入待機モードとJSON証跡を追加。再生成PDFの受信hash、2ページTIFF、CUPS完了、重複投入を検査。自動lp経路の回帰確認済み。実アプリUIは接続タイムアウトのためPending。 / Application acceptance harness; actual UI acceptance pending / 应用验收辅助模式，实际UI验收待完成。
- 🟢 [Done] IPP Hold-Job／Release-Jobを追加し、REST／MCPと共通の保留状態を永続化。保留中の処理開始防止、古いrevisionでのworker取得拒否、再起動後の保留維持、処理開始後の保留拒否を検証。時刻指定・投入時保留は後続。 / Durable IPP hold/release / 持久化IPP暂停与恢复。
- ⭕️ [Pending] driverless追加、アプリごとの印刷ダイアログ、別Macからの接続、Windowsの手動IPP追加・標準ドライバー印刷。対象端末・ドライバーでPDFが送られることを確認し、非PDFなら追加形式の互換対応を記録する。Mac上のBonjourとipptoolの成功をOS印刷ダイアログの受入成功へ読み替えない。
- 🔴 [Later] 自動再試行ポリシー、IPPS/TLS、OSドライバーに必要となったCreate-Job／Send-Document・追加文書形式。現在の再試行は認証付きAPIで明示的に新attemptを作り、エンジンや設定を暗黙に変更しない。

**第1段階の完了条件（OSクライアント受入試験はPending）:** プロトコルの照会・投入・取消、Mac標準ipptoolからの1/2ページPDF→TIFF、Mac BonjourによるLAN発見、再起動後のID復元、受付制限、限定コンテナー回収は検証済み。Mac専用PPDでのキュー登録・設定画面表示・標準lpからの完了は確認済み。driverless追加・アプリ印刷ダイアログ・Windows標準ドライバーの受入確認が残るため、段階全体をDoneにはしない。実機出力やAirPrint / IPP Everywhere適合はこの段階の対象外。

## Agent Print Infrastructureの標準契約

- 🟢 [Done] 共通JobTicket v0.1を新規REST・IPP・MCP投入と同じトランザクションで保存。原稿hash・エンジン・設定・TIFF完了条件を記録し、設定検証APIと削除後も保持する照会APIを追加。 / Durable shared raster ticket and pure validation / 持久化通用栅格契约及纯设置验证。詳細：[JOB-TICKET.md](JOB-TICKET.md)。
- 🟢 [Done] RIP・Color・Queue・設定済みPrinterの共通Capability Model、内容revisionと30秒有効期限、MCP能力照会・ticket取得・検証を追加。未照会と利用可能を区別する。 / Common stage capability snapshots and MCP bindings / 通用阶段能力快照及MCP绑定。
- 🟢 [Done] PrintIntentにengine・ページ範囲・numberUp・面付け・品質を追加し、色方針・優先度・期限のrequirement指定と明示的な代替案を実装。 / Extended intent negotiation / 扩展意图协商。
- 🟢 [Done] 共通能力照会へ送信条件の交差照合を追加。TIFF→PWG接続、DPI・色型・8bit・用紙・容量と不一致理由を返し、MCPから実機ID指定で照会できる。 / Delivery settings intersection with live MCP discovery / 投递设置交叉匹配及MCP实时查询。
- 🟢 [Done] 完成済み成果物から実機送信previewを作成し、PWG・寸法・ICC・ライブ能力を検証。hash拘束付き投入と配信契約の原子的保存・取得、MCP操作を実装。 / Verified physical delivery preview and atomic contract / 已验证实体投递预览及原子契约。
- 🟢 [Done] 完成済みRIPからICC処理・承認待ち・実機送信へ進む永続pipeline worker、状態照会・再開・投入前取消とMCP操作を追加。再起動時のchild再利用、scope再検査、UNKNOWN非再送、活動中原稿の削除・retention保護に対応。 / Durable post-RIP orchestration / 持久化RIP后续编排。詳細：[PIPELINE.md](PIPELINE.md)。
- 🟢 [Done] workflow状態と同時に工程履歴を永続化。sequence差分取得・最大100件・再起動保持・MCP照会・再送とrevision競合の履歴重複防止。 / Atomic resumable orchestration history / 原子可恢复编排历史。
- 🟢 [Done] RIP・色・delivery・workflowの統合timeline、原子trigger索引、永続sequence差分取得、既存履歴の一度だけの明示的取り込み、MCP照会を実装。 / Unified persistent stage timeline / 统一持久化阶段timeline。
- 🟢 [Done] 統合timelineのMCP resource・購読・標準更新通知・解除。1秒の内部確認、32購読上限、jobs:read制御、保存cursorによる再接続後の差分取得に対応。 / MCP timeline update notifications / MCP timeline更新通知。
- 🟢 [Done] 実機向けPrintIntentのlive能力照合、digest・原稿hash・主体・期限を拘束したpreflight、一括commit、RIP待機から実機送信までの接続、MCPを実装。Docker MuPDFと偽IPPデバイスで完了を検証。 / Bound physical preflight and RIP-to-device workflow verified with Docker/fake IPP / 绑定实体预检与RIP到设备流程，已通过Docker与模拟IPP验证。
- 🟢 [Done] 全工程JobTicket照会をREST/MCPに追加。RIP・登録・色処理・実機契約を単一読取snapshotで統合し、原稿・出力先の一致、契約hashと実行状態の分離、未確定・旧証跡欠落・UNKNOWNを明示。 / Unified pipeline contract reads with bound evidence and separate execution state / 统一流程契约查询，绑定证据并分离运行状态。
- 🟢 [Done] 検査状態と原子的なイベント保存、所有者限定REST/MCP差分取得、再起動保持、MCP resource購読・更新通知・解除。 / Transactional inspection history and owner-bound MCP notifications / 事务性检查历史与所有者限定MCP通知。
- 🟢 [Done] 全工程timeline・所有者限定検査履歴のHTTP SSE、Last-Event-ID再開、ページ単位の背圧、接続制限・切断解放・keep-alive。 / Bounded authenticated REST SSE with durable replay / 有界认证REST SSE及持久重放。
- 🟢 [Done] remote MCP Streamable HTTPのPOST操作・GET通知・DELETE終了、token拘束セッション、Origin検証・期限・容量制限、再接続時の更新ヒント。 / Authenticated stateful remote MCP and notification streams / 认证有状态远程MCP与通知流。詳細：[MCP-HTTP.md](MCP-HTTP.md)。
- 🔴 [Later] MCP OAuth discovery・認証フローと外部クライアントの相互運用試験。 / MCP OAuth discovery and external client interoperability / MCP OAuth发现与外部客户端互操作测试。
- 🔴 [Later] 部数・両面・面付け等の実行対応。表現対応とは区別する。 / Execute copies, duplex and imposition settings / 执行份数、双面与拼版设置。

- 🟢 [Done] 認証付きTIFFダウンロードとページ単位の公開後整合性検査を追加。サイズ・SHA-256不一致は409、未登録ページは404。検証済み非公開スナップショットを逐次配信し、検証後の元ファイル変更でも配信内容を固定する。
- 🟢 [Done] 単一HTTP RangeとETagによるIf-Rangeを実装。一時スナップショットはプロセスごとに合計1 GiBを予約し、配信終了・切断で解放。予算不足は503。
- 🟢 [Done] 同じデータrootのプロセス間でOSロックによる1GiB予算を共有。終了時に予約解放。 / Shared single-host artifact budget / 单机共享产物预算。

- 🟢 [Done] ワーカーがTIFFページを逐次hash計算し、ページ順・名前・サイズ・SHA-256を完了状態と同じトランザクションで保存。結果Receiptにも結び付け、認証付き成果物一覧APIを追加。hash計算中もleaseを更新する。
- 🟢 [Done] dry-run既定の保持期限CLI、終端ジョブと既知の孤立ファイル清掃、処理中／UNKNOWN除外、途中終了後の再開、履歴・Receipt保持。詳細は[MAINTENANCE.md](MAINTENANCE.md)。 / Resumable opt-in retention / 可恢复的可选保留期限清理。
- 🟢 [Done] 未登録アップロード残骸のwriter-lock連携清掃。REST・IPP・MCP投入/commit/retryを保護し、登録証跡・期限を再確認し、永続journalで中断復旧、dry-runと監査付き適用を追加。 / Writer-locked orphan upload reclamation with durable recovery / writer lock保护的孤立上传回收与持久恢复。
- 🟢 [Done] 成果物snapshotの分散予算。認証付き中央API、原子的な容量判定、主体拘束・冪等予約／解放、別rootからの取得・検証への接続、清掃後解放、未確認予約の恒久保持と管理者復旧を実装。既存の単一host制限も併用。[DISTRIBUTED-BUDGET.md](DISTRIBUTED-BUDGET.md)。 / Central HTTP admission for distributed artifact snapshots with conservative crash recovery / 成果快照的中央HTTP分布式预算与保守崩溃恢复。
- ⭕️ [Pending] 実VPS間HTTPS・証明書・回線分断の受入試験。開発環境では独立rootとlocalhost HTTPで検証。 / Actual cross-VPS HTTPS and network-partition acceptance / 实际跨VPS HTTPS及网络分区验收。
- 🔴 [Later] 原稿検査・RIP staging・色変換一時領域を含む全工程の分散資源予算。 / Distributed resource budgets for source inspection and RIP/color staging / 原稿检查与RIP、色彩暂存的全阶段分布式资源预算。

- 🟢 [Done] commit経由ジョブの終端状態と同一トランザクションでACX結果Receiptを保存。完了・失敗・取消・lease失効を記録し、結果JSONのSHA-256と受付Receiptを関連付ける。認証付き `GET /api/v1/jobs/{id}/result-receipt` で取得でき、ジョブ削除後も保持する。
- 🟢 [Done] 新ワーカーの結果JSONに成果物のページ数・サイズ・SHA-256一覧を含めた。resultHashはこの結果JSONを対象とする。

- 🟢 [Done] commitと同一トランザクションでACX 0.1受付Receiptを永続化し、認証付き取得APIとreceiptUrlを追加。ticket hash、原稿hash、preflight digest、job-idを関連付け、再起動・ジョブ削除後も同じ受付証跡を保持する。
- 🟢 [Done] 任意のEd25519署名付き受付・ラスター結果Receipt、トランザクション保存、公開鍵を指定したオフライン検証、鍵交換と再起動後の証跡保持。[RECEIPT-SIGNING.md](RECEIPT-SIGNING.md)。 / Optional signed receipts / 可选签名回执。
- 🟢 [Done] jobs:read限定のReceipt公開鍵一覧APIとMCP鍵・受付・結果取得。起動登録した公開鍵履歴を鍵交換後も保持し、cursor取得と独立key pinningを明示。 / Public-key discovery and MCP receipt reads / 公钥发现与MCP回执读取。
- 🟢 [Done] 管理者API/MCPで署名鍵を恒久失効、成功監査と原子保存、同理由の再送、実行中writerの保存時確認。失効鍵の受付は拒否し、終端結果は署名不能を明示して保存。現在状態を確認する検証CLIも追加。 / Durable key revocation / 持久密钥撤销。
- 🟢 [Done] 配送の完了・取消・中止・拒否を状態／イベントと同じトランザクションでReceipt保存。実機job-id、配送／実機契約、ラスター結果hashと署名を結び付け、jobs:read限定のAPI/MCPで取得。UNKNOWN／受付済みは完了と扱わない。 / Atomic delivery result receipts / 原子投递结果回执。
- 🟢 [Done] 制限付きRFC 8785共通digest crate／厳密parser／CLI、JCS署名v2と旧v1検証、共通ticketの追加portableDigest。既存hashを変更せずRust／Nodeで相互検証。[CANONICAL-DIGEST.md](CANONICAL-DIGEST.md)。 / Portable bounded JCS digest / 可移植有界JCS摘要。
- 🟢 [Done] 事前承認digestのversion移行：新規settings／pipeline preflightはJCS v2、明示指定で旧v1、保存済みprofileによるcommit検証。再起動後の互換性と旧物理workflow受付を検証。 / Versioned preflight digest negotiation and persisted-profile verification / 版本化预检摘要协商与持久化profile验证。
- 🟢 [Done] 監査archiveの不変DB snapshot・JCS digest・ローカルCLI出力／復旧／独立digest検証。未完了記録を飛ばさず元記録を保持。 [AUDIT-ARCHIVE.md](AUDIT-ARCHIVE.md)。 / Immutable bounded audit archives and local export verification / 不可变有界审计归档及本地导出验证。
- 🟢 [Done] 監査archiveの専用Ed25519／JCS署名、鍵必須CLI、失効時の保存拒否、公開鍵での署名検証と現在の失効状態による検証。保存済みarchiveは再署名しない。 / Domain-separated audit signatures and current key trust / 独立domain审计签名与当前密钥信任验证。
- 🟢 [Done] 監査archiveの管理者指定filesystem vault、非上書き公開、内容確認後の永続ACK・監査、再送復旧と現在の保管状況照会。 / Verified filesystem vault and replay recovery / 验证文件系统归档库与重放恢复。
- ⭕️ [Pending] 実NFSの永続性・切断復旧、Windowsのdirectory sync。 / Actual NFS durability and Windows directory sync / 实际NFS持久性及Windows目录同步。
- 🟢 [Done] 監査archiveのmetadata一覧、UUID cursor、作成時刻cutoff、過去の保管確認数、既存archiveからの復旧ID検索。 / Bounded archive metadata catalogue / 有界归档元数据目录。
- 🟢 [Done] 監査archive保持期限のdry-run対象確認、指定vaultのACKと現在の内容再検証、理由コード・件数制限付き一覧。 / Read-only retention assessment with live vault checks / 实时归档库检查及只读保留期限评估。
- 🟢 [Done] 指定ID・digest・保持期限・--applyによる監査DB snapshot削除、保管先再検証・同期、削除証跡と監査の原子保存、再送復旧と削除後のvault-status。元監査と保管copyは保持。 / Revalidated local snapshot pruning and durable evidence / 重新验证的本地快照删除及持久证据。
- 🟢 [Done] HTTPS conditional object-vault adapter、管理者環境設定、保存後のbounded読み戻し、永続remote ACK・監査、再送／現在状態照会。 / Conditional HTTPS vault adapter / 条件式HTTPS归档库adapter。
- ⭕️ [Pending] 実HTTPSサービス・証明書と事業者の永続性、S3互換・WORM検証。 / Live HTTPS/provider durability and S3/WORM verification / 实际HTTPS及provider持久性、S3/WORM验证。
- 🟢 [Done] remote copy保持期限のdry-run確認、初回ACK基準、現存local snapshotと現在のGET照合、最後のcopyの候補除外、bounded paging。 / Conservative remote-retention assessment / 保守远程保留期限评估。
- 🟢 [Done] HTTPS copyのstrong ETag条件付き単一DELETE、PREPARED journal・監査、UNKNOWN照会復旧、local snapshot保護、store競合防止。 / Conditional remote deletion with durable uncertainty handling / 条件式远程删除与持久不确定状态处理。
- ⭕️ [Pending] 実HTTPS事業者のIf-Match DELETE・read-after-delete保証と共有NFSロック。 / Live provider conditional deletion and shared NFS locks / 实际provider条件删除及共享NFS锁。
- 🟢 [Done] 監査archiveのS3 SigV4 adapter。GET／条件付きPUT／条件付きDELETE、session token、CLI provider設定、既存の読戻しACK・UNKNOWN非再送・local保護を統合。独立botocore署名vectorと署名検証付き模擬S3で検証。[S3-AUDIT-VAULT.md](S3-AUDIT-VAULT.md)。 / S3 SigV4 audit vault adapter with independent signature vectors / S3 SigV4审计归档adapter及独立签名向量验证。
- ⭕️ [Pending] 実S3／互換providerのTLS・IAM・versioning／Object Lock・耐久性の受入試験。 / Live S3/provider TLS, IAM, versioning, Object Lock and durability acceptance / 实际S3/provider的TLS、IAM、版本及Object Lock验收。
- 🟢 [Done] S3 version指定GETによるUNKNOWN解決（保持／不在／未確定、DELETE非再送）とAWS KMS Ed25519監査archive署名・公開鍵pin・失効再確認・CLI・模擬試験。 / S3 version-aware recovery and KMS audit signing / S3版本恢复及KMS审计签名。[仕様・検証範囲](PROVIDER-RECOVERY-KMS.md)
- ⭕️ [Pending] 実AWSのTLS・IAM・region鍵対応・versioning・永続性の受入試験。 / Real AWS acceptance / 实际AWS验收。
- 🔴 [Later] 永久version削除、Object Lock管理、明示SSE-KMS暗号化設定。 / Permanent version purge, Object Lock and explicit SSE-KMS / 永久版本清除、Object Lock及显式SSE-KMS。

- 🟢 [Done] preflight digest、原稿SHA-256、形式、有効期限を検証するmultipart commit APIを実装。保存済みticketからジョブを登録し、SQLiteトランザクションで同一preflightの同時再送を重複排除する。共通Bearerトークンによる認可を使用。
- 🟢 [Done] 設定／物理preflightとretryの主体拘束、操作ごとのscope検査、受付Receiptへの方式・主体・検査証跡。Receipt、期限切れpreflightと孤立ファイルの清掃は既存実装で検証済み。 / Owner-bound requests and scoped evidence / 身份绑定请求与scope证据。
- 🟢 [Done] 全投入経路が合流する標準workerで必須の原稿SHA再確認・私有snapshot・PDF厳密全ページ検査・PDF/PS/EPS全体のbounded解釈を実装。公開前の証跡保存、結果Receipt・API/MCP resource、IPP／模擬実機完了と異常原稿拒否を検証。規格認証や添付全object検査とは区別。[SOURCE-VALIDATION.md](SOURCE-VALIDATION.md)。 / Mandatory bounded source validation in the standard worker / 标准worker强制有界原稿验证。

- 🟢 [Done] 設定preflightの永続化、5分の期限、確定ticketのrequestHash、preflightDigest、再起動後の取得、期限切れ拒否。新規v2は共有JCS bounded profile、明示選択したv1は旧encoding。保存済み方式を維持。 / Versioned durable preflight / 版本化持久预检。
- 🟢 [Done] 主体別の認可証跡とJRIP/ACX実験的print profileの共有JCS digestに対応。設定preflightの作成だけでは実行を許可しない。 [AGENT-CONTROL.md](AGENT-CONTROL.md)。 / Owner evidence and shared experimental JCS profile / 身份证据及共享实验性JCS profile。

- 🟢 [Done] `POST /api/v1/print/resolve` でACX PrintIntentをTIFF出力用JobTicketへ能力照合する。必須条件の競合、推奨値の調整、実機出力・ICC・用紙・両面・期限など未対応設定を明示する。原稿取得・検証、認可、コミット、ジョブ投入は後続工程。
- 🟠 [Next] ACX lifecycle全体の適合を検証する。resolverは設定照合のみで、原稿構造検証や主体別認可を含むACX lifecycleの完了とは扱わない。

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

1. 🟠 [Next] **Print APIを正規の入口にする。** ACX側の実験的PrintIntent schemaとnamespaced capability extensionsは実装済み。次にREST、IPP、MCPから受けた要求を、共通の `PrintIntent` と `JobTicket` へ正規化する。原稿参照、部数、ページ範囲、用紙、面付け、片面／両面、解像度、色意図、品質、優先度、期限、出力先、承認条件を表現し、各フィールドを `required`、`preferred`、`automatic` のいずれかとして指定できるようにする。能力不足時は黙って変更せず、拒否または具体的な代替案を返す。
2. 🟠 [Next] **共通Capability Modelを定義する。** RIP、Color Management、Queue、Printerが能力と制約を同じ形式で公開する。入力・出力MIME、最大寸法、DPI、色空間、ICC profile、レンダリングインテント、用紙、両面、部数、仕上げ、現在の利用可否を表し、`capability_revision` と有効期限を付ける。Print APIは全工程の交差部分を計算し、実行可能なticketと不一致理由をAgentへ返す。
3. 🟠 [Next] **工程間artifact契約を固定する。** `SourceDocument`、`RasterArtifact`、`ColorManagedArtifact`、`DeliveryArtifact` をcontent-addressed artifactとして扱い、SHA-256、MIME、ページ数、寸法、色空間、profile、生成元、保持期限を必須メタデータにする。大容量データはサービス間メッセージへ埋め込まず、許可されたobject storeまたはローカルartifact storeの参照で渡す。
4. 🟠 [Next] **状態とイベントを工程単位にする。** ジョブ全体の状態に加えて `VALIDATING`、`RIPPING`、`COLOR_MANAGING`、`QUEUED_FOR_DEVICE`、`SENDING`、`PRINTING`、`COMPLETED`、`PARTIALLY_COMPLETED`、`CANCELED`、`FAILED` を標準化し、工程、進捗、ページ、reason code、再試行可否をイベントとして記録する。`COMPLETED` は実機から確認できた完了条件をticketへ記録し、現在のTIFF出力完了とは別に扱う。
5. 🟠 [Next] **Color Managementを独立サービス境界にする。** ICC profile登録、入力profile判定、出力profile選択、レンダリングインテント、黒点補正、黒版保持、DeviceLink、TACを `ColorPolicy` として版管理する。変換結果には使用profileのhashと `ColorReport` を付け、Agentが再現性、警告、gamut、インキ制限を確認できるようにする。初期実装はLittleCMSを候補とし、RIPエンジン固有の暗黙変換を明示的に記録する。
6. 🟠 [Next] **Queueを実機制御の責任境界にする。** 優先度、期限、依存関係、保留、取消、回数制限付き再試行、device affinity、fairnessを扱う。RIP再実行とプリンター再送を別attemptとして記録し、同じページの重複印刷を防ぐdelivery idempotency keyを導入する。用紙切れ、インク／トナー、カバー開放、オフライン、jamなどを構造化reason codeとして上位へ伝える。
7. 🟠 [Next] **Printer Adapterを標準化する。** `PrinterBackend` traitで能力取得、検証、送信、ジョブ照会、取消、device/supply状態を統一し、最初の実装をNative IPPとする。PWG Raster、PDF、JPEGなどは実機能力に応じて選択し、ベンダー固有機能はnamespaced extensionとして隔離する。CUPSやメーカーSDKも同じ契約へ追加できるようにする。
8. 🟠 [Next] **ポリシーと観測性を全工程へ通す。** identity、tenant、scope、承認、費用上限、ページ上限、カラー印刷可否、データ所在地、保持期限を `ExecutionPolicy` として評価する。すべての判断、ticket変更、artifact、attempt、device応答を監査し、OpenTelemetry traceと同じcorrelation idで結ぶ。Agentには機密情報を除いた構造化診断と回復可能な次の操作を返す。
9. 🔴 [Later] **公開仕様と適合テストを整備する。** Print APIのOpenAPI、MCP schema、JobTicket JSON Schema、PrinterBackend conformance suite、reason code registry、サンプルAgent SDKを英語・日本語・简体中文で公開する。IPP / PWGの既存標準へ対応付け、独自項目には安定したnamespaceを使う。

**Infrastructure段階の完了条件（未達）:** Agentが自然言語の印刷意図から能力照合済みJobTicketを作り、RIP、明示的な色変換、永続キュー、Native IPP実機出力を通して排紙結果まで追跡できる。途中の再起動、Agentの切断、プリンター障害、用紙切れ、取消、再試行でも同じjob-idから工程とattemptを復元でき、重複印刷を起こさない。各境界の契約テストと、模擬プリンターを使うend-to-endテストを必須とする。

## AIから直接利用するMCP制御面

`services/jrip-mcp` を独立プロセスとして追加し、MCP固有の型や接続処理をRIP Coreへ持ち込まない。最初はローカルAIエージェント向けのstdio transportを提供し、認証・TLS・接続制限を整えた後にリモート向けStreamable HTTPを追加する。MCPサーバーはSQLiteやDockerを直接操作せず、既存のJob Service、Repository、Dispatcher、将来のPrinter Backendを型付きアプリケーション境界から呼び出す。

1. 🟠 [Next] **自己記述可能な能力を公開する。** MCPの `tools/list` とresourcesから、Print APIの共通Capability Model、入力形式、MuPDF / Ghostscript、Color Policy、解像度、色空間、ページ・容量上限、キュー、プリンター能力、device/supply状態、現在の負荷、機能フラグを機械可読JSON Schemaで返す。未実装の能力は公開せず、スキーマへ版番号を付ける。英語・日本語・简体中文の表示名、説明、エラー回復案を提供する。
2. 🟠 [Next] **RIP操作を細粒度のtoolsにする。** `rip_validate`、`rip_submit`、`rip_get_job`、`rip_list_jobs`、`rip_watch_job`、`rip_hold_job`、`rip_release_job`、`rip_cancel_job`、`rip_retry_job`、`rip_delete_job` を提供する。投入時は原稿、エンジン、DPI、色空間、ページ範囲、優先度、出力先を明示できるようにし、検証だけを行うdry-runとidempotency keyを用意する。更新系toolは期待revisionを受け取り、AIの並行操作による上書きを防ぐ。
3. 🟠 [Next] **成果物と診断情報をresourcesにする。** `jrip://jobs/{id}`、`jrip://jobs/{id}/events`、`jrip://jobs/{id}/manifest`、`jrip://jobs/{id}/artifacts`、`jrip://jobs/{id}/artifacts/{name}`、`jrip://printers/{id}` を定義する。TIFF、サムネイル、プリフライト結果、エンジンログを列挙し、大容量成果物はMCPメッセージへ埋め込まず、期限付きURLまたは範囲取得可能なresourceとして渡す。ジョブイベント購読と進捗通知に対応し、ポーリング間隔をAI側へ強制しない。
4. 🟠 [Next] **AIが安全に印刷仕様を組み立てられるようにする。** `print_resolve_intent`、`print_validate_ticket`、`rip_get_presets`、`rip_compare_engines`、`rip_estimate`、`rip_create_preview` を提供し、実行前に能力照合済みJobTicket、代替案、予想ページ数、ラスター寸法、色変換、メモリ・ディスク使用量、印刷機の利用可否を返す。永続preset、Color Policy、キュー、実機プリンター設定の変更は別scopeに分け、変更前後の値と監査理由を必須にする。任意のシェルコマンド、Docker引数、ホストパスは受け付けない。
5. 🟠 [Next] **権限と人の承認境界を実装する。** `jobs:read`、`jobs:submit`、`jobs:control`、`artifacts:read`、`printers:print`、`settings:write`、`admin` のscopeを設ける。ローカルstdioも呼出元identityを監査し、リモートtransportは短期トークン、TLS、レート制限、入力容量制限を必須にする。ファイル出力は許可済みroot配下に限定する。実機印刷、ジョブ削除、永続設定変更はポリシーで人の承認を要求でき、MCP応答は `approval_required` と具体的な影響を返す。
6. 🟠 [Next] **長時間処理と再接続を標準化する。** tool呼出しは短時間で永続job-idを返し、RIP処理をMCPセッションの寿命へ結び付けない。AIが切断・再接続してもjob-idとidempotency keyから処理を追跡できるようにする。構造化エラーには安定したcode、再試行可否、推奨待機時間、現在revisionを含め、失敗時に別エンジンへ自動変更せず、AIが差分を確認して明示的に再実行する。
7. 🔴 [Later] **複数RIPノードの統合制御へ拡張する。** MCP Gatewayから複数のJ-RIP Workerを能力・負荷・データ所在で選択できるようにする。Cloud側と独立WorkerはHTTPS APIまたはジョブキューで疎結合にし、WorkerからManaged PostgreSQLへの直接接続に依存しない。認証情報や原稿をモデルの会話履歴へ露出させず、データ所在地と保持期限をポリシーで制御する。

**MCP段階の完了条件（未達）:** MCP Inspectorと2種類以上のAIクライアントから、能力発見、dry-run、PDF投入、進捗通知、成果物取得、保留・解除・取消、失敗診断、revision競合、idempotency、再接続をend-to-endで検証する。権限不足と承認待ちを構造化応答として確認し、MCP経由でホストの任意ファイル、任意コマンド、Docker APIへ到達できないことをテストする。すべての更新操作を主体、tool名、引数の要約、結果、job-id、時刻とともに監査できる状態を完了とする。

## 第2段階：実機への出力

- 🟢 [Done] `rip-printer` に `PrinterBackend` とNative IPP送信アダプターを実装。能力取得、受理ジョブID、状態照会、取消を模擬プリンターとのHTTP通信で検証。IPPクライアント受付とは別の境界。
- 🟢 [Done] 管理者設定の出力先、永続Delivery Queue、検証付きPWG登録、Print API、Delivery Workerによる送信・状態照会を接続。RIP完了と実機完了は別状態。詳細は[DELIVERY.md](DELIVERY.md)。 / Configured destinations, verified PWG registration, API and worker / 配置目标、验证PWG注册、API与Worker。
- 🟢 [Done] Native IPPのIPPS/TLS、システム／独自CAによる証明書・接続先名検証、TLS限定Bearer認証。模擬TLSプリンターで検証。 / Verified IPPS and TLS-only Bearer / 经验证IPPS与TLS专用Bearer。
- ⭕️ [Pending] 実機IPPSの証明書・認証互換性。 / Physical IPPS acceptance / 实体IPPS验收。
- 🟢 [Done] 8-bit TIFFからPWG Rasterへの変換、ページ順序・用紙寸法・解像度・色型の照合、明示的なDelivery要求での送信。
- 🟠 [Next] 実装済みのリースと失敗復旧を基盤に、残存コンテナー回収と回数制限付き再試行を加え、送信完了とプリンター側のジョブ結果を区別する。部数・両面などはバックエンドで反映可能になったものから順に公開する。
- ⭕️ [Pending] Mac / Windowsの実機印刷、取消、障害、再起動を通し、クライアント表示と実際の印刷結果が一致することを確認する。対象クライアントと実機を使用できる検証環境が必要。
- 🟠 [Next] IPP Everywhereを名乗る前に、必要なPWG Raster入力と、カラー機で必要なJPEG入力、属性・DNS-SD・セキュリティ要件を満たし、PWGの自己認証テストを実行する。PDFだけの受付では適合を主張しない。

## 印刷品質と製品機能

- 🔴 [Later] `Create-Job` / `Send-Document` の複数文書とApple URF入力をクライアント互換性に応じて追加する。未実装の形式を広告しない。
- 🔴 [Later] 日本語CID/CMap・縦書き・IVSとフォント置換レポート、EPS / 日本語のGolden Master。
- 🟢 [Done] `rip-color` 独立ライブラリ／単ページTIFF CLI：明示ICC・4 intents・BPC・K-only／K-plane設定・DeviceLink・TAC・hash付きColorReport。sRGB一致、ICC／寸法保持、DeviceLink TACと純K保持を検証。詳細は[COLOR-MANAGEMENT.md](COLOR-MANAGEMENT.md)。
- 🟢 [Done] SHA-256で固定するICC profile registryと登録・照会CLI。重複登録は同じhashを返し、破損・シンボリックリンクを拒否する。 / Immutable ICC registry and CLI / 不可变ICC注册库及CLI。
- 🟢 [Done] ICC登録API（settings:write）、永続Color attempt、隔離Docker Worker、ページ進捗、hash付きカラー成果物・レポート、profileを照合するDelivery接続、MCP色処理tool。 / Isolated color pipeline and scoped control / 隔离颜色流程及权限控制。
- 🟠 [Next] 永続Color Policy/preset、入力profile判定、gamut警告、共通artifact保持期限・ACX digestへの統合。
- ⭕️ [Pending] 実機ICCによるK-only／K-planeと色再現品質の受入確認。
- 🔴 [Later] オーバープリント、分版プレビュー、gamut解析。
- 🔴 [Later] CMYK16、タイルRIP、メモリ予算、ハーフトーン、DeviceN、特色、Trapping。
- 🔴 [Later] React管理UI（英語・日本語・简体中文）にRIP・プリンター状態と専門設定を集約する。必要ならWindows Print Support Appを検討する。
- 🔴 [Later] 実機プリンター探索・能力取得、CUPS連携、PostgreSQL Repository、ロール認可、監査、保持期限と容量管理。
- 🔴 [Later] Linuxサービス・パッケージ、SBOM／ライセンス監査、必要時のみHTTPSで接続するクラウドControl Plane。

参考規格: [PWG IPP Everywhere](https://pwg.org/ipp/everywhere.html)、[PWG自己認証ツール](https://www.pwg.org/ippeveselfcert/)。Mac / Windowsの実際の対応状況は、対象OSでの結合テストを完了条件とする。

## English / 简体中文

**English:** The target is an Agent-operated Print Infrastructure with standardized boundaries from Agent → Print API → RIP → Color Management → Queue → Printer. A versioned PrintIntent, JobTicket, Capability Model, artifact contract, event model, ColorPolicy, queue contract, and PrinterBackend let an Agent negotiate executable settings and follow one job through physical output. MCP is the AI control plane, while REST and IPP remain protocol adapters over the same application services. The design includes idempotency, optimistic concurrency, resumable jobs, delivery deduplication, audit records, scoped authorization, and approval policies. `COMPLETED` currently means TIFF export only until physical-printer completion is implemented separately.

**简体中文：** 目标是建立由Agent操作的Print Infrastructure，标准化Agent → Print API → RIP → Color Management → Queue → Printer的全部边界。通过带版本的PrintIntent、JobTicket、Capability Model、成果物契约、事件模型、ColorPolicy、队列契约和PrinterBackend，Agent可以协商可执行的打印设置，并使用同一个job-id追踪到实体输出。MCP作为AI控制面，REST和IPP则作为共用应用服务之上的协议适配器。设计包含幂等、乐观并发控制、可恢复作业、防止重复打印、审计、分级授权及审批策略。在单独实现实体打印完成状态之前，当前的 `COMPLETED` 仍仅表示TIFF文件导出完成。

- 🟢 [Done] ローカルstdio MCPの初期実装：MCP 2025-11-25、initialize、tools/list、能力・ジョブ一覧・詳細・履歴・成果物一覧の5つの読み取りtool。既存API処理を共有し、入力64 KiB・応答4 MiBを上限にする。 / Local stdio MCP with five read tools / 本地stdio MCP与五个读取工具。
- 🟢 [Done] MCPの事前確認・投入・再試行、identity別scope、状態履歴resourceの読取・購読通知。 / Preflight, submit/retry, identity scopes and event subscriptions / 预检、提交与重试、身份权限与事件订阅。

- 🟢 [Done] MCP保留・再開・取消tool：jobIdとexpectedRevision必須。RESTのrevisionヘッダー検証と既存SQLiteの楽観的更新を共有し、古い状態への操作を409として拒否する。 / Hold, release and cancel with revision checks / 带版本检查的暂停、恢复和取消。
- 🟢 [Done] API／本番stdio MCPの永続監査、管理者監査API、ジョブ・manifest・成果物一覧resources。詳細は[AGENT-CONTROL.md](AGENT-CONTROL.md)。 / Durable audit and JSON resources / 持久化审计与JSON资源。
- 🟢 [Done] 実機出力承認ポリシー：主体・revision・attempt・出力先profile・用紙に固定した5分間の承認、取消、enqueue時の原子的再検査。 / Bound physical-delivery approvals / 绑定实体投递审批。
- 🟠 [Next] 削除／永続設定の承認、個別バイナリresource。remote transportは実装済み。 / Deletion/settings approval and binary resources; remote transport implemented / 删除及设置审批、二进制资源；远程传输已实现。

- 🟢 [Done] 再試行はFAILED/CANCELLED限定、expectedRevisionとUUID requestId必須。永続preflight/commitに同一操作を結び、同時要求・再起動後の再送を同じ新規ジョブへ収束させる。原稿SHA-256を再検証し、元ジョブ・設定・Receiptは保持する。
- 🟢 [Done] JRIP_AUTH_POLICYのidentityとscopeをAPI/MCP共通で検査。jobs:read、jobs:submit、jobs:control、artifacts:read、admin。再試行はsubmitとcontrolの両方が必要。設定済みポリシー外のトークンは拒否する。
- 🟢 [Done] jrip://jobs/{jobId}/eventsの標準MCP購読通知。内部1秒間隔、最大32購読、カーソルで再接続後の差分を回収。 / Standard event-resource notifications with resumable cursor / 带可恢复游标的标准事件资源通知。

- 🟢 [Done] `rip-raster`：8-bit DeviceRGB/DeviceCMYK/BlackのTIFF→PWG Raster変換とCLI。ページ順、DPI、色の一致、入力制限、失敗時の一時ファイル清掃、上書き禁止を実装。macOS CUPSによる独立読取を確認。 / Bounded TIFF-to-PWG conversion, independently decoded by CUPS / 有界TIFF转PWG，经CUPS独立解码验证。
- 🟢 [Done] PWG解像度・色型・用紙能力の照合、変換成果物の永続登録と送信API接続。ICC処理のDelivery接続と両面は未実装。 / PWG matching, registration and delivery API / PWG匹配、注册与投递API。

- 🟢 [Done] Native IPPのPWG能力取得と完全一致チェック：解像度、色型、用紙、MIME、受付可否。 / Exact PWG capability matching / 精确PWG能力匹配。
- 🟢 [Done] 永続Delivery Queueの状態管理：不変Ticket・試行UUIDによる冪等登録、単一claim、run-id・revision・60秒リース、受理と実機完了の分離、期限切れ送信のUNKNOWN化。RIPジョブの完了意味を変更しない。 / Durable delivery state and conservative recovery / 持久化投递状态与保守恢复。
- 🟢 [Done] 検証付きPWG登録、ページ寸法照合、送信直前の能力再取得、Print APIとDelivery Workerの送信・照会を接続。改変拒否と不明結果の再送禁止を模擬プリンターで検証。 / Verified delivery API and worker / 验证投递API与Worker。
- 🟢 [Done] Delivery取消API：revisionとprint scopeを検査、QUEUEDの原子的取消、ACCEPTEDの永続取消要求、単一Cancel-Job試行と実機状態照会。 / Durable cancellation API / 持久化取消API。
- 🟢 [Done] 実機能力・受付可否・状態理由のAPI／MCP／resource、hash版と30秒有効期限。URI・認証情報は非公開。 / Live printer capabilities / 实时打印机能力。
- 🟢 [Done] MCPの設定済み出力先一覧・Delivery照会／投入／取消、印刷意図照合、revision付き終端ジョブ記録削除。 / MCP delivery controls and terminal metadata deletion / MCP投递控制与结束任务记录删除。
- 🟢 [Done] UNKNOWNを実機job-id・Delivery UUID付きジョブ名・revisionで照合して復旧。再送せず、別ジョブは拒否。 / UNKNOWN remote-job reconciliation / UNKNOWN远程任务核对。

- 🟢 [Done] 受付済みRIPジョブへのworkflow登録、RIP完了待機、失敗・取消時の後続停止、完了revisionへの接続、再起動とrequestId再送に対応。 / Register workflows before RIP completion, await completion and stop on failure/cancellation, with durable replay. / RIP完成前注册流程、等待完成、失败或取消时停止，支持重启与幂等重放。
- 🟢 [Done] 実機向けPrintIntentをlive能力・ICC条件と照合し、主体・原稿SHA256・能力revision・期限を固定したpreflightから原子的にworkflowを受付。MCPの作成・commit工具も接続。 / Bound physical PrintIntent preflight and atomic workflow commit, including MCP. / 绑定实体PrintIntent预检与原子流程接收，含MCP。

- 🟢 [Done] multipart一括受付APIとMCP print_submit_pipeline。原稿検証後にjob/JobTicket/workflow/受付イベントを同じDBトランザクションで保存し、並行再送は同じjobIdへ対応付ける。競合・不正原稿・不正出力条件は新規ジョブを残さない。 / Atomic source/workflow API and inbox-bound MCP tool, transactional acceptance and idempotent concurrent replay. / 源文档与流程原子接收API及inbox限定MCP工具，事务接收与并发幂等重放。

- 🟢 [Done] 信頼されたPDF検査CLI、Docker MuPDFの内容解釈・実効寸法検査、原稿hash・主体に拘束した保存結果、API/MCP読取、inspectionId指定時の用紙照合とcommit固定を実装。継承CropBox/回転/UserUnitと修復PDFの拒否、検査から偽IPP完了までを検証。 / Trusted PDF inspection CLI and opt-in bound evidence, verified geometry and end-to-end fake device output / 信任PDF检查CLI与可选绑定证据，已验证尺寸及模拟设备完整输出。

- 🟢 [Done] HTTP/MCPのPDF検査非同期受付、主体・原稿hash固定の並行再送、状態照会・revision付き待機中取消、32件/256 MiB制限、trusted worker、成功状態と証跡の原子的保存、中断時FAILED、永続cleanupと孤立原稿・新CLI snapshot回収。Dockerで処理・中断復旧・lock保護・清掃を検証。 / Async bounded PDF inspection queue, MCP controls, trusted worker and durable recovery/cleanup / 有界异步PDF检查队列、MCP控制、可信worker与持久恢复/清理。
