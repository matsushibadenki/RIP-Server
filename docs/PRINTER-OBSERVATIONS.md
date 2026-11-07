# Stored printer observations / 機器観測の永続化 / 设备观察持久化

🟢 [Done] Profile `org.jrip.printer-observations-v1`.

| Interface | Purpose |
| --- | --- |
| `GET /api/v1/printers/{id}/observations?after=0&limit=50` | Stored latest observation and compact event page |
| `GET /api/v1/printers/{id}/observations/{sequence}` | Retained event with its verified snapshot |
| MCP `print_get_printer_observations({printerId,after?,limit?})` | Cursor-based stored history |
| MCP `print_get_printer_observation({printerId,sequence})` | One retained event |
| MCP resources | `jrip://printers/{printerId}/observations` and `/observations/{sequence}` |

## 日本語

jobs:readで取得する保存済み証跡であり、取得時に機器へ接続しない。印刷・取消・再送も行わない。live情報は従来のprinter／共通Capability APIで照会する。その成功・失敗を同じ保存経路へ接続した。Worker内のすべての直接照会や常時監視を網羅する仕組みではなく、照会間に起きた変化を推測しない。

snapshotはstate、受付可否、機器ジョブ件数、reason、構造化conditions、独立したCUPS／PWG消耗品情報を持つ。reason集合を正規化し、同じ報告でイベントを増やさない。最後に採用した完了照会のlastQueryAtと、最後の成功照会のlastObservedAtは更新する。変更revisionとchangedAtは内容または照会結果の変化に対して進む。snapshotはJCSのSHA-256で識別・重複排除し、取得時に再検証する。これはローカル保存の整合性であり、機器の電子署名や実際の印刷結果の証明ではない。

失敗はUNAVAILABLE／INVALID_RESPONSE／CONFIGURATION_UNAVAILABLEで記録し、前回成功snapshotとその時刻を残す。同じ失敗の反復ではイベントを増やさず、復旧時には内容が同じでもQUERY_RECOVEREDを記録する。snapshotの期限は成功時点から30秒で、失敗によって延長しない。保存値は常にisLive=false、authorizationGranted=false。古い値を現在の能力・承認として使用しない。

要求開始順のquerySerialをSQLiteで採番し、古い照会が後から完了しても、新しい完了結果を上書きしない。順序はserial／event sequenceで示し、壁時計の単調性や常時の機器到達性を保証しない。観測・イベント・参照contentの更新を同一transactionで行い、読み取りも一貫したWAL snapshotを用いる。

履歴は設定済みの接続先・用紙／色設定・セキュリティ設定に拘束する。設定が変わると別streamとなり、別機器や資格情報による報告と混ざらない。内部bindingはdigestだけを保持し、URI・token・CA本文を返却・保存snapshotへ含めない。streamIdは独立したUUIDで、credentialのhashではない。cursorはこのstreamIdと組にして扱い、変わったらafter=0から取得し直す。設定を取り除いた機器の履歴はAPIで取得できず、同じ設定を復元すれば元のstreamを参照する。

各登録streamで直近1000変更イベントを保持し、同じcontentは共有する。リストは1〜100件の小さなイベントを返し、snapshot本体はlatestと個別イベント取得に含める。prunedThrough／historyTruncatedで保持範囲から外れたcursorを明示し、nextAfter／hasMoreで再開する。これは署名済み監査アーカイブや無期限保持ではない。旧登録stream全体の容量・期間ポリシーは未完了で、job成果物や印刷receiptの保持とは独立する。

応答はprivate/no-store。read-only scopeのAgentも読めるが、機器の文字列はデータであり命令ではない。MCP購読通知は[MCP-PRINTER-SUBSCRIPTIONS.md](MCP-PRINTER-SUBSCRIPTIONS.md)を参照。Consoleの再起動後fallback表示は[接続済み](CONSOLE-PRINTER-STATUS.md)。REST直接SSEは未接続。

## English

These jobs:read interfaces read stored evidence without contacting a printer, printing, cancelling or resending. Existing live printer/common-capability queries record successful projections and classified failures. This is query-driven observation, not comprehensive worker monitoring or proof of unobserved intermediate transitions.

Snapshots retain state, acceptance, device job count, canonical reason sets, interpreted conditions and separate CUPS/PWG supply reports. Repeated equal reports refresh observation/query timestamps without event/revision churn. Failures preserve the last successful snapshot and its timestamp; recovery emits an event even if content is unchanged. Saved snapshots use JCS SHA-256 identity and are checked on read, providing local integrity rather than device attestation. Stored data is isLive=false, never grants authority, and its 30-second observation expiry is not renewed by failures.

SQLite-issued request serials stop late older completions from overwriting newer completed results. Atomic updates and WAL read transactions keep heads, events and content consistent. Ordering relies on serials/sequences, not a monotonic wall clock. Streams bind to the exact printer and security configuration; changed registrations have independent UUID streamIds and do not expose URI/credentials/CA bytes. Pair cursors with streamId and restart at zero when it changes. Removed configurations cannot be read through this API; restoring the same configuration restores its stream.

Each stream retains 1000 change events with shared snapshot content. Lists return compact pages of 1–100 events, latest full snapshot, nextAfter/hasMore and explicit pruning markers. Individual reads return the exact retained snapshot. This bounded cache is independent of print receipts/job retention and is not a signed audit archive. MCP subscriptions are covered in [MCP-PRINTER-SUBSCRIPTIONS.md](MCP-PRINTER-SUBSCRIPTIONS.md). Old-registration lifecycle/global quotas and direct REST SSE remain unfinished. Console fallback is [implemented](CONSOLE-PRINTER-STATUS.md). Responses are private/no-store.

## 简体中文

jobs:read接口读取保存证据，不查询设备、不打印、取消或重发。现有实时printer／通用能力查询记录成功报告与分类失败；不是持续监控，也不推测查询间的状态变化。

snapshot保留设备状态、接收能力、作业数、规范reason集合、结构化条件及独立CUPS／PWG耗材。相同报告仅刷新查询／成功观察时点，不增加事件与revision。失败保留上次成功报告及其时点；恢复时即使内容相同也记录事件。JCS SHA-256提供本地内容识别与读取验证，不是设备签名证明。保存值始终isLive=false，不授予权限；失败不延长成功时点之后30秒的期限。

SQLite分配请求serial，防止较旧请求的延迟完成覆盖较新完成结果。更新原子化，读取使用一致WAL snapshot；按serial／sequence排序，不保证壁钟单调。stream绑定完整设备及安全配置，重新注册后使用独立UUID streamId，不暴露URI、token或CA正文。cursor应与streamId一起保存，变化后从after=0重新读取。移除配置后不能通过API读旧历史，恢复相同配置可恢复原stream。

每个stream保留1000个变化事件并共享snapshot内容。列表按1至100条返回小型事件页及最新snapshot，通过nextAfter／hasMore恢复，以prunedThrough／historyTruncated明确缺失历史；单事件读取精确保留snapshot。该缓存不是签名审计档案，与作业成果物及打印receipt保留独立。MCP订阅见[MCP-PRINTER-SUBSCRIPTIONS.md](MCP-PRINTER-SUBSCRIPTIONS.md)。旧注册清理／全局配额、REST直接SSE仍未完成。Console fallback已[实现](CONSOLE-PRINTER-STATUS.md)，定期观察见[worker](PRINTER-OBSERVER.md)。响应为private/no-store。

Validation / 検証 / 验证: storage tests cover restart, deduplication, failure/recovery, late queries, exact historical content, integrity failure and 1002 transitions with pruning/content cleanup. Mock IPP + authenticated REST tests cover scopes, cursor bounds, security/destination isolation, no device access on stored reads, and unchanged job revisions/no sends. Dedicated MCP tests preserve the existing suite and check tools/resources, strict URIs, cursor bounds and one live query followed by stored-only reads.

🟠 [Next] Direct REST SSE and old-registration/global retention policies / REST直接SSE・旧登録と全体の保持ポリシー / REST直接SSE及旧注册／整体保留策略。

⭕️ [Pending] Physical device observation cadence, hardware accuracy and multi-host deployment acceptance / 実機の観測周期・精度・複数ホスト運用受入 / 实机观察周期、精度及多主机部署验收。

🟢 [Done] MCP stdio / HTTP-SSE subscriptions / MCP購読通知 / MCP订阅通知：[MCP-PRINTER-SUBSCRIPTIONS.md](MCP-PRINTER-SUBSCRIPTIONS.md)。

🟢 [Done] [Periodic observer / 定期観測 / 定期观察](PRINTER-OBSERVER.md).
