# Retention / 保持期限 / 保留期限

```sh
JRIP_DATA_ROOT=./data cargo run --locked -p jrip-server --bin jrip-maintenance -- --retention-days 30
JRIP_DATA_ROOT=./data cargo run --locked -p jrip-server --bin jrip-maintenance -- --retention-days 30 --apply
```

English: The default is a dry-run. `--apply` removes source/raster files and job/artifact metadata for terminal jobs older than the configured window (1–3650 days, default 30). Jobs with worker leases or QUEUED/SENDING/ACCEPTED/UNKNOWN deliveries are excluded. Known orphan directories from deleted terminal jobs qualify only when durable terminal history is old enough; arbitrary directories and active uploads never qualify. Each database claim creates a durable tombstone before moving files to an internal trash directory. Interrupted purges resume on the next run. Terminal delivery PWG/registration files for the expired job are also removed; delivery history, job events, acceptance/result receipts and audit records remain. Uncommitted expired preflights and expired approvals are pruned; committed preflight evidence remains. Limits are 100 live-job candidates, 100 known orphans and 100 purge completions per pass; rerun until no pending work remains. The CLI prevents concurrent instances with an OS lock. Do not remove its lock files while running. Marked abandoned uploads follow the writer-lock procedure below; age alone never authorizes deletion.

日本語：既定は対象を表示するだけです。`--apply` で、設定日数を過ぎた終端ジョブの原稿・ラスターとジョブ／成果物記録を削除します。ワーカーリースや待機・送信・受理・UNKNOWNのDeliveryがある場合は除外します。メタデータ削除済みの孤立ディレクトリは、永続履歴で終端状態と期限を確認できるものだけ清掃します。受信中のアップロードや任意ディレクトリは削除しません。ロック付き中断原稿の回収手順は後述します。清掃対象をDBへ先に記録し、途中終了後は次回再開します。終端DeliveryのPWG／登録ファイルも削除しますが、履歴・Receipt・監査は保持します。未commitの期限切れpreflightと承認も清掃します。1回の処理件数は制限されているため、残件がなくなるまで再実行してください。実行中のロックファイルは削除しないでください。

简体中文：默认仅显示候选项。`--apply` 删除超过保留期限的已结束任务原稿、栅格文件及任务／产物记录。存在Worker租约或排队、发送、受理、UNKNOWN投递时排除。已删除元数据的孤立目录仅在持久化历史能确认终端状态及期限时清理；不删除正在上传的原稿或任意目录。先记录DB清理标记，中断后下次继续。结束投递的PWG／注册文件也删除，但保留历史、Receipt及审计。清理未commit的过期预检和审批。单次处理有数量限制，应重复运行至无待办；运行期间不要删除锁文件。

Artifact downloads reserve up to 1 GiB across processes sharing the canonical data root using 64 OS-locked 16 MiB slots, in addition to the local semaphore. Reservations release on download completion, disconnection or process exit. This remains the single-host/shared-root limit. An optional [central HTTP budget](DISTRIBUTED-BUDGET.md) additionally coordinates artifact snapshots across independent roots; it is admission control, not a filesystem storage quota. Budget lock files are persistent coordination files; do not delete them while a process is using the root.

日本語：成果物の取得は同一データrootのプロセス間で合計1GiBまで予約し、16MiB単位・64枠のOSロックで共有します。配信完了・切断・プロセス終了で解放します。複数ホストの分散容量管理ではありません。使用中の予算ロックファイルを削除しないでください。

简体中文：同一数据root的进程共享最多1GiB下载预算，通过64个16MiB的OS锁槽协调。下载完成、断开或进程结束时释放。这不是跨主机分布式配额；使用数据root期间不要删除预算锁文件。

Private download snapshots live under `artifact-snapshots`, hold an OS lock and normally disappear when the request finishes. Maintenance considers at most 128 entries per pass, removing only recognized `artifact-snapshot-*.part` files older than one hour whose locks can be acquired. Active downloads and unrelated files are skipped. Crash leftovers release their locks and become eligible on a later pass.

日本語：私有スナップショットは専用ディレクトリとOSロックで管理します。通常は取得終了時に消えます。清掃は1時間以上前の専用名のファイルで、ロックを取得できるものだけを対象とし、進行中の配信と無関係なファイルを除外します。

简体中文：私有快照位于专用目录并持有OS锁，通常在下载结束时删除。清理仅处理超过一小时、名称匹配且可取得锁的文件，跳过正在下载及无关文件。

Color attempts in QUEUED/RUNNING also protect the source. Color output inside the source job follows its retention policy; color attempt records/events remain. Abandoned color staging older than one hour is removed only when its writer lock is free. / 待機・実行中の色処理も原本を保護し、色成果物は原本と同じ保持期限で清掃、記録は保持します。孤立stagingは1時間超かつwriter lock解放済みの場合のみ削除します。 / 排队及运行中的颜色尝试保护原稿，颜色输出与原稿一同清理，记录和事件保留。孤立staging仅在超过一小时且writer lock空闲时删除。

## Abandoned upload reclamation / 中断アップロードの回収 / 中断上传回收

REST・IPP・MCPの新規RIP投入、preflight commit、pipeline commit、明示的retryは、原稿の書き込みからDB公開・失敗時清掃まで `jobs/{uuid}/upload-writer.lock` を保持する。メンテナンスはUUID名の実ディレクトリ、保持期限cutoffより古いmtime、既存の通常ファイルのmarker、取得可能なwriter lockを要求する。jobs・job_events・JobTicket・worker lease・retention markerの登録証跡がある場合は孤立アップロードとして削除しない。ロック取得後にDBとmtimeを再確認する。

dry-runの `abandonedUploads` は対象UUID一覧で、`--apply` の同じフィールドは今回清掃journalへ登録したUUID一覧。実際の削除完了は `purgedJobs` を確認する。適用は既存のretention監査に含まれる。ロック解放後の次回実行で中断原稿を回収できる。ファイルを削除する前に登録証跡を再確認する単一SQLで永続retention journalへ登録し、既存のtrash移動・削除を使う。削除途中でmarkerが消えても、再起動後にjournalから再開する。1回に最大4096ディレクトリエントリを走査し、128アップロードまで回収する。大量の登録済みディレクトリが走査枠を占有する場合は、通常の保持期限清掃で先に縮小する必要がある。

ロックmarker導入前の未登録原稿、markerのないディレクトリ、ディレクトリやmarkerのシンボリックリンク、保持期限前の原稿、活動中のwriterは対象外。登録済み終端ジョブは従来のretention手順で清掃する。検査queueには専用workerの清掃を使う。データroot使用中にwriter markerを手動で削除しない。

**English:** New REST/IPP/MCP RIP submissions, preflight/pipeline commits and retries hold an upload-writer.lock through source writes, DB publication and failure cleanup. Reclamation requires an old UUID directory, a regular marker, an available OS lock, and no job/history/ticket/lease/retention registration. Recheck DB and directory time after acquiring the lock. Dry-run lists eligible UUIDs; apply lists newly journaled UUIDs within existing retention audit; purgedJobs reports completed deletion. An atomic guarded DB claim precedes filesystem deletion through the existing trash/purge journal, so restart resumes even if the marker was already removed. Each pass scans at most 4096 entries and reclaims at most 128 uploads; normal job retention must first reduce large registered-directory populations when they fill the scan window. Active, recent, unmarked legacy and symlink entries remain untouched. Registered terminal jobs and inspection queues retain their existing cleanup paths. Do not remove markers while the root is in use.

**简体中文：** 新REST/IPP/MCP投入、预检/流程commit和retry在原稿写入、DB发布及失败清理期间保持upload-writer.lock。回收要求UUID实目录、mtime超过保留期限、普通marker文件、可获取OS锁，并且不存在作业/历史/ticket/lease/retention注册证据。取得锁后重新检查DB和目录时间。dry-run列出候选UUID，apply报告新登记清理journal的UUID并沿用retention审计，purgedJobs表示删除完成。文件删除前通过原子DB登记再次验证注册证据，沿用trash/purge journal，即使marker已被删除，也能在重启后继续。每次最多扫描4096项、回收128上传；大量已注册目录占满扫描窗口时，应先通过普通保留期限清理缩小目录数量。活动、近期、旧无marker及符号链接项目不删除。已注册终端作业及检查队列继续使用各自清理机制。使用数据root期间不要删除marker。

監査記録の不変snapshotと復旧可能なローカル出力は [AUDIT-ARCHIVE.md](AUDIT-ARCHIVE.md) を参照してください。元記録の削除は行いません。 / See AUDIT-ARCHIVE for immutable snapshots and recoverable local export; source audit records remain. / 不可变审计快照与可恢复导出详见AUDIT-ARCHIVE，保留原审计记录。
