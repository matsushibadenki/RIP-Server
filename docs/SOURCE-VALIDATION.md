# Source validation / 原稿検証 / 原稿验证

## English

All current submission paths—REST job upload, settings/physical preflight commit, pipeline submission, retry, IPP PDF and MCP tools forwarding to these APIs—converge on the standard `jrip-worker`. That worker now requires a hash-bound private source snapshot and successful bounded full-page interpretation before publishing raster artifacts. HTTP, IPP and MCP frontends do not run Docker. Queue admission still checks format, size and SHA-256; acceptance is not a structural-validation success.

The worker reserves up to 32 MiB from the shared single-host snapshot budget, copies the source into `jobs/{id}/source-validation`, rejects symlink sources, and checks the copy against the admitted source hash. The exact sealed copy is used for both inspection and rendering, so changes to the original after the copy cannot change interpreted bytes. Each job/run still owns its normal worker lease and heartbeat.

For every PDF, MuPDF inspection parses all page contents and effective geometry, rejecting encrypted PDFs, repair/parse warnings (except the existing explicit ICC-build advisory allowlist), invalid geometry and more than 100 pages. This gate also applies when Ghostscript was explicitly selected for PDF rendering: both configured images must be available. The existing trusted inspection policy and its bounded interpreter are reused. PDF rendered page count must match inspection. PDF, PostScript and EPS must then finish a complete sandboxed interpreter run with 1–100 valid named TIFF outputs; late interpreter errors reject the entire result, including earlier pages. There is no silent engine fallback.

This profile means bounded page-content interpretation, not PDF/PS/EPS standards certification, antivirus scanning, static analysis of arbitrary PostScript programs or inspection of every unreferenced object/attachment. Time, memory, output, page and source-size limits can conservatively reject otherwise printable files. No pixel-perfect or paper-quality guarantee is added.

After success, the trusted worker stores `org.jrip.source-validation-v1` evidence while its current run owns an unexpired RIPPING lease. It binds job/run IDs, source SHA-256 and byte count, page count, format, selected engine and strict PDF inspection. Wrong owners, stale/expired leases, wrong hashes, unsupported evidence and duplicate publication fail. Evidence is immutable and retained after ordinary job metadata deletion. Evidence is stored before output publication; a later spool failure can therefore retain successful source interpretation evidence without claiming a completed export.

Read with jobs:read:

- REST: `GET /api/v1/jobs/{id}/source-validation` (404 until evidence exists).
- MCP resource: `jrip://jobs/{jobId}/source-validation`.
- New result receipts: `proof.result.sourceValidation`, covered by resultHash and any configured receipt signature.

There is no client write endpoint or caller-supplied verified flag. Admission JobTickets/preflights and historical receipts stay unchanged; their structureVerified/documentVerified flags describe admission, not a later worker pass. Old or custom workers without this evidence must not be inferred as validated. Deploy the standard worker binary to enforce the gate; custom JRIP_WORKER_BIN implementations must follow this contract themselves.

Snapshots are removed after successful cleanup, including normal failures. A failed container removal retains the snapshot conservatively. Dispatcher recovery handles LEASE_EXPIRED, WORKER_EXIT and container_cleanup_failed, removes only containers labelled for the original source or its fixed validation snapshot, then reclaims that snapshot. Other sources' containers are preserved. A job interrupted during validation fails through existing lease recovery and is not automatically rerun.

Validation performed: normal PDF/PS/EPS; explicit Ghostscript PDF; malformed and repair-requiring PDFs; PS/EPS errors after an earlier page; changed admitted bytes; REST preflight through result receipt; MCP submission through the worker and resource read; independent ipptool submission/restart; default RIP through fake device-confirmed output; current-run/hash/scope checks and orphan snapshot/container recovery. Real printer quality and arbitrary third-party workers remain outside this milestone.

## 日本語

REST投入、設定／物理preflight commit、pipeline、retry、IPP PDF、MCP経由の投入は標準 `jrip-worker` に合流します。workerは原稿hashに拘束した私有snapshotと全ページのbounded解釈を必須とし、成功するまでラスターを公開しません。HTTP・IPP・MCPへDocker権限は追加しません。キュー受付と構造検査成功は別です。

共有予算から32 MiBまでを予約して原稿をcopyし、受付時SHA-256と照合します。検査とRIPは同じ封印copyを使います。PDFは全ページのMuPDF content／実効寸法検査を必須とし、暗号化・修復・解析警告（既存のICC build通知の明示除外を除く）・不正寸法・100ページ超を拒否します。PDFのエンジンをGhostscriptにした場合も同じ検査が必要で、両イメージが必要です。検査と出力のページ数も照合します。PS/EPSを含め、隔離環境で最後まで解釈できなければ、先行ページが出ていても全体を失敗にします。

成功時は有効なworker run／leaseと原稿hashに拘束した不変証跡を保存し、jobs:readのREST／MCP resourceで取得できます。新しい結果Receiptの `proof.result.sourceValidation` にも含め、resultHashと設定済み署名の対象にします。受付ticketや旧Receiptは書き換えません。古いworker・独自workerの証跡なし履歴を検査済みとは扱わないでください。

これは印刷ページ内容の制限付き解釈であり、PDF等の規格認証、添付／未参照object全体の検査、任意PSの静的解析、排紙品質の保証ではありません。時間・容量上限による保守的拒否があります。キュー受付前の同期検査ではなく、trusted workerによる成果物公開前の必須gateです。

正常終了と通常の失敗ではsnapshotを消去し、コンテナー回収失敗時は保持します。dispatcher再起動時に失効lease・worker中断・cleanup失敗を再確認し、該当原稿とsnapshotのlabelを持つコンテナーだけを回収します。中断ジョブを自動再実行しません。Docker正常／異常系、独立IPP、REST Receipt、模擬プリンター完了、証跡の権限・hash・run拘束を検証しました。

## 简体中文

REST、设置／实体预检commit、pipeline、retry、IPP PDF及MCP提交均由标准 `jrip-worker` 处理。worker必须使用绑定原稿hash的私有snapshot，并在全部页面有界解释成功后才发布栅格。HTTP、IPP和MCP不运行Docker。队列接收不等于结构验证成功。

从共享预算预留最多32 MiB，复制原稿并核对接收时SHA-256，检查和RIP使用同一封存copy。所有PDF必须通过MuPDF全页内容及有效尺寸检查，拒绝加密、修复／解析警告（现有ICC build通知白名单除外）、非法尺寸及超过100页。即使选择Ghostscript渲染PDF也需要该检查和两个镜像，并核对页数。PS/EPS也须完整结束隔离解释器运行；后续错误会拒绝包括前页在内的全部结果，不自动换引擎。

成功后保存绑定当前worker run／有效lease、hash、字节数、页数及引擎的不可变证据。jobs:read可通过REST或MCP resource读取。新结果回执的proof.result.sourceValidation纳入resultHash及配置的签名。接收ticket和历史回执不重写；无证据的旧／自定义worker历史不应视为已验证。

该profile是有界页面内容解释，不是格式标准认证、全部附件／未引用object检查、任意PS静态分析或纸面质量保证。资源限制可能保守拒绝可打印文件。检查发生在trusted worker发布产物前，不是队列接收前同步检查。正常及一般失败时删除snapshot，容器回收失败时保留；dispatcher恢复只清理该原稿及snapshot标签对应的容器，不自动重跑中断任务。已验证Docker正常／异常输入、独立IPP、REST回执、模拟设备完成，以及权限／hash／run边界。
