# Local runtime / 一括起動と基本運用 / 统一启动及基本运行

## 日本語

🟢 [Done] `jrip-runtime` はmacOS／Linuxの単一ホストでAPI・IPP・自動RIP・ICC・PDF検査をまとめて起動する。`scripts/build-rip-server.sh` が全workspaceの実行ファイルとMuPDF／Ghostscript／色変換のDockerイメージを構築し、`scripts/start-rip-server.sh` がforegroundで監視する。Rust/Dockerを揃えてREADMEの手順で起動する。

`--check` はtoken長、設定JSON、実行ファイル、待受ポート、Dockerと固定の3イメージを検査する。データrootとSQLiteは初期化するが、workerの起動・ジョブ処理・機器への接続は行わない。実際の原稿の互換性、実機の利用可否や色再現までは証明しない。アップグレード後はbuildスクリプトでイメージも再構築する。

| 設定 | 既定値・意味 |
|---|---|
| `JRIP_API_TOKEN` | 必須、24文字以上。認可policy設定時はその中のtokenでAPIへ接続 |
| `JRIP_DATA_ROOT` | `data`。同じrootと認可設定で再起動 |
| `JRIP_BIN_DIR` | runtimeと同じ実行ファイルディレクトリ。管理者CLI設定のみ |
| `JRIP_LISTEN` | `127.0.0.1:8080`。IPアドレスと0以外のポート |
| `JRIP_IPP_LISTEN` | `127.0.0.1:8631`。APIと異なるポート |
| `--without-ipp` | IPPプロセスを起動しない |
| `--with-printers` | pipeline／delivery／観測workerも起動。プリンター設定が必須 |
| `JRIP_REQUIRE_PRINT_APPROVAL` | 一括起動の子プロセスでは未設定時に`1`。明示`0`で変更可能 |
| `JRIP_AUTH_POLICY` | 承認必須の実機モードには設定済み主体tokenが必要 |

起動時に同じrootの`runtime.lock`をOSロックする。別の一括起動は拒否し、正常停止・異常終了後はOSがロックを解放する。既存の手動サービスとの併用や共有NFS・複数ホストの排他を保証するものではない。dispatcherにも既存の単一起動ロックがある。

子プロセスごとに`logs/jrip-*.log`へ追記する。ログディレクトリは0700、新規ログは0600。子プロセスは別process groupで実行し、端末のCtrl+Cで描画workerを直接中断しない。どれかが終了したら残りへSIGINTを送り、スタックを非0で終了する。子プロセスを勝手に再起動したり、不明な印刷結果を再送したりしない。

Ctrl+C／SIGTERMで停止する。dispatcherは現在のworkerを待ち、色変換・検査・pipeline・delivery workerは実行中passを終えて終了する。処理時間は原稿と未完了配送数に依存する。APIのSSE接続は5秒後にAPIプロセスだけを終了できるため、購読中クライアントで全体停止を無期限に待たせない。処理workerへ自動SIGKILLは送らない。SIGKILL・ホスト停止・Docker喪失後は既存のlease復旧と清掃規則を使い、UNKNOWNは明示照合する。

標準出力の`status:ready`はAPIの`/health`とIPPの`/`へのHTTP応答、監視対象の生存を起動時に確認した結果。継続的なDocker／実機readinessや色品質の保証ではない。起動失敗時はstderrと個別ログを確認する。設定・ポート競合・Docker停止・不足イメージを修正して再起動する。

MCPは既存の独立サービスを同じroot／認可設定で別途起動できる。保持期限の削除は自動実行せず、maintenanceをdry-runで確認してから明示applyする。

## English

🟢 [Done] The macOS/Linux single-host supervisor runs API, optional IPP, automatic RIP dispatch, ICC and inspection processes. The build script builds all workspace binaries and three fixed sandbox images. The start script runs the supervisor in the foreground. `--with-printers` additionally enables pipeline, delivery and observation; default mode never starts those physical-dispatch services.

`--check` initializes the root/database and checks configuration, token length, binaries, ports, Docker and images without starting services or contacting printers. It does not prove document/device/color compatibility; rebuild images after upgrades. Environment variables and defaults appear in the table above. Approved physical mode needs configured identity credentials, and managed children default to required print approval unless explicitly overridden with 0. Starting physical mode resumes already-admitted deliveries; policy changes do not retroactively cancel them.

An OS lock prevents a second managed stack on the same local root. This does not fence independently started services or establish NFS/multi-host guarantees. Logs append per process under a private directory; new files are private. Separate process groups keep terminal Ctrl+C from directly interrupting interpreters. Any child exit stops siblings and returns failure; children and UNKNOWN deliveries are never automatically retried.

SIGINT/SIGTERM drains active passes. The dispatcher waits for its worker; color, inspection, pipeline and delivery finish their current pass. Drain time depends on documents and outstanding deliveries. API-only forced termination after five seconds prevents SSE clients from blocking shutdown; active processing workers are not automatically killed. SIGKILL/host/Docker failures use existing lease recovery/cleanup and explicit UNKNOWN reconciliation.

Ready output checks listener HTTP responses and child liveness at startup, not continuous hardware/Docker readiness or output quality. Inspect supervisor stderr and per-process logs for failures. MCP can run separately using the same root/authorization. Retention deletion is explicit after a maintenance dry-run.

## 简体中文

🟢 [Done] macOS／Linux单机监督程序统一启动API、可选IPP、自动RIP调度、ICC及检查进程。build脚本构建全部workspace程序和三个固定隔离镜像，start脚本以前台运行。`--with-printers`另行启用流程、实体投递与观察，默认模式不启动实体投递服务。

`--check`初始化目录／数据库并检查配置、token长度、程序、端口、Docker及镜像，不启动服务或连接打印机。不保证原稿、设备或颜色兼容性，升级后须重建镜像。环境变量见上表。需要审批的实体模式必须配置主体凭据，统一启动默认要求打印审批，只有明确设置0才覆盖。启用实体模式会恢复已接收投递，策略修改不会追溯取消它们。

本地OS锁拒绝相同目录的第二个统一实例，不保证手动服务、NFS或多主机互斥。日志按进程追加，目录与新文件采用私有权限。独立process group避免终端Ctrl+C直接中断解释器。任何子进程退出时停止其他进程并返回失败，不自动重启进程或重发UNKNOWN。

SIGINT／SIGTERM等待当前处理结束：调度器等待worker，色彩、检查、流程及投递完成当前pass。时间取决于原稿和未完成投递。五秒后仅可强制结束API，防止SSE客户端阻塞整个停止；不自动SIGKILL处理worker。SIGKILL、主机或Docker故障仍通过既有lease恢复、清理及明确UNKNOWN核对处理。

Ready只验证启动时HTTP响应及进程存活，不保证持续Docker／设备readiness或输出质量。故障时检查监督程序stderr与各进程日志。MCP可使用相同目录／授权独立运行。保留期限删除须先dry-run再明确apply。

⭕️ [Pending] Native Windows supervisor, real devices/drivers, shared NFS and service-manager deployment acceptance / Windows一括起動・実機／driver・共有NFS・service manager配備の受入 / Windows统一启动、实体设备／驱动、共享NFS及service manager部署验收。

## Basic-operation acceptance / 基本運用の受入 / 基本运行验收

🟢 [Done] 実際のsupervisorと全workerを私有rootで起動し、PDF／PS／EPS・IPPの自動RIP、TIFF hash／プレビュー、不正原稿後の継続、ICC、承認待ち、模擬機器へのPWG送信1回、機器完了、停止／再起動後の状態保持と再送なしを検証。DB初期化はローカルOSロックと初期化用の即時write transactionで競合を避け、既存writer稼働中の同時openもテストする。 / Real managed-stack acceptance covers automatic formats/IPP, TIFF identity/preview, rejected-source recovery, ICC/approval, one fake PWG print, completion and restart/no-resend. Local schema locks and an immediate installation transaction handle concurrent initialization, tested with an active writer. / 已通过真实统一栈验证格式／IPP自动RIP、TIFF标识／预览、异常原稿后继续、ICC／审批、一次模拟PWG打印、完成及重启防重发。通过本地schema锁和初始化即时写事务处理竞争，并在活动writer下测试并发open。

```sh
cargo test -p jrip-server --test runtime managed_stack --locked -- --ignored --nocapture
```
