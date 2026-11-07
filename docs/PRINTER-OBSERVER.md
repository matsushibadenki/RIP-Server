# Periodic printer observer / 定期プリンター観測 / 定期打印机观察

🟢 [Done] Read-only worker, durable scheduling and bounded concurrency / 読取専用worker・永続スケジュール・並列数制限 / 只读worker、持久调度与并发限制。

## 日本語

API／MCPと同じ `JRIP_DATA_ROOT`、`JRIP_API_TOKEN`、`JRIP_PRINTER_CONFIG`、必要なら `JRIP_PRINTER_SECURITY` を設定して起動する。

```sh
cargo run -p jrip-server --bin jrip-printer-observer --locked
# 期限が来た登録を一巡して終了する場合
cargo run -p jrip-server --bin jrip-printer-observer --locked -- --once
```

設定済み機器へGet-Printer-Attributesだけを送る。結果は[永続観測履歴](PRINTER-OBSERVATIONS.md)へ保存し、既存の[MCP購読](MCP-PRINTER-SUBSCRIPTIONS.md)から変化を通知できる。印刷・取消・ジョブ再送は行わない。起動時の登録設定を使うため、設定変更後はworkerを再起動する。

| 環境変数 | 既定値 | 範囲 |
|---|---:|---|
| `JRIP_PRINTER_OBSERVER_INTERVAL_SECONDS` | 15 | 5–3600秒 |
| `JRIP_PRINTER_OBSERVER_CONCURRENCY` | 4 | 1–16 |
| `JRIP_PRINTER_OBSERVER_MAX_BACKOFF_SECONDS` | intervalと300の大きい方 | interval以上、86400秒以下 |

成功からinterval秒後に次回照会する。連続失敗時はintervalの2倍、4倍と最大待機まで増加し、成功でリセットする。登録内容ごとの60秒leaseと次回時刻をSQLiteへ保存する。異常終了・停止時の処理中leaseは期限後に回復できる。Native照会のタイムアウトは20秒。並列上限はプロセス単位であり、全workerを合わせた上限ではない。巡回は遅い機器の終了を待って一括再開する方式ではない。

`--once` は `attempted`、`successful`、`failed`、`skipped`、`storageErrors`、`leaseLost` をJSONで標準出力する。照会失敗・保存失敗・lease喪失があれば終了コードは非0。すべて待機中でskipした場合は成功。ログは標準エラーへ出力し、URI・認証情報を含めない。保存失敗時はlease期限まで待つ。

照会失敗は機器の電源断を断定しない。任意の消耗品属性の不正は、その属性のINVALID_RESPONSEとして保存され、基本能力照会自体は成功する場合がある。UNIX時刻を使うため時計の変更は待機時間へ影響する。検証はローカルSQLite・模擬IPPで実施。共有NFS・複数ホストのlease保証や実機品質は未検証。

## English

Run the commands above alongside API/MCP with the same data root, token, printer configuration and optional security configuration. Registrations are loaded at startup; restart after configuration changes. The worker only sends Get-Printer-Attributes, persists observations and enables MCP change notifications. It never prints, cancels or retries delivery.

The table defines defaults and bounds. Successful queries wait one interval; failures wait twice, four times, etc., capped by maximum backoff. Success resets failures. SQLite stores per-registration cooldowns and 60-second leases across restarts. Interrupted leases recover after expiry; native queries time out after 20 seconds. Concurrency is per process, not global. Round-robin scheduling avoids a whole-pass barrier behind slow devices.

`--once` emits JSON counters shown above and exits nonzero on query/storage failure or lost lease. An entirely skipped pass succeeds. Logs go to stderr without URIs or credentials. Storage failure leaves the lease to expire. Query failure does not prove a powered-off device. Invalid optional supply attributes may coexist with successful core capability queries. Wall-clock changes affect scheduling. Local SQLite and fake IPP tests do not establish shared-NFS, multi-host or physical-printer guarantees.

## 简体中文

使用上面的命令，与API／MCP共享数据目录、token、打印机配置及可选安全配置。启动时加载注册信息，修改配置后须重启worker。只发送Get-Printer-Attributes，保存观察结果并支持MCP变化通知，不打印、取消或重试投递。

环境变量默认值及范围见表。成功后等待一个间隔；连续失败依次等待两倍、四倍间隔，直到最大退避，成功后重置。SQLite保存每个注册的下次执行时间与60秒lease，重启后保留。中断的lease到期后可恢复，Native查询超时20秒。并发上限按进程计算，并非全局限制。轮转调度不会等待所有慢设备完成后才开始下一轮。

`--once` 输出上述JSON计数，查询、保存失败或丢失lease时返回非0；全部跳过仍为成功。日志写入stderr，不含URI与凭据。保存失败时等待lease过期。查询失败不代表设备断电；可选耗材属性无效时，核心能力查询仍可能成功。系统时钟变化会影响调度。已在本地SQLite与模拟IPP环境验证，尚不保证共享NFS、多主机或实体设备行为。

⭕️ [Pending] Physical printer and shared-NFS acceptance / 実機・共有NFS受入検証 / 实体设备及共享NFS验收。
