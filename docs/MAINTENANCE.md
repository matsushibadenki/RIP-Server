# Retention / 保持期限 / 保留期限

```sh
JRIP_DATA_ROOT=./data cargo run --locked -p jrip-server --bin jrip-maintenance -- --retention-days 30
JRIP_DATA_ROOT=./data cargo run --locked -p jrip-server --bin jrip-maintenance -- --retention-days 30 --apply
```

English: The default is a dry-run. `--apply` removes source/raster files and job/artifact metadata for terminal jobs older than the configured window (1–3650 days, default 30). Jobs with worker leases or QUEUED/SENDING/ACCEPTED/UNKNOWN deliveries are excluded. Known orphan directories from deleted terminal jobs qualify only when durable terminal history is old enough; arbitrary directories and unfinished uploads never qualify. Each database claim creates a durable tombstone before moving files to an internal trash directory. Interrupted purges resume on the next run. Terminal delivery PWG/registration files for the expired job are also removed; delivery history, job events, acceptance/result receipts and audit records remain. Uncommitted expired preflights and expired approvals are pruned; committed preflight evidence remains. Limits are 100 live-job candidates, 100 known orphans and 100 purge completions per pass; rerun until no pending work remains. The CLI prevents concurrent instances with an OS lock. Do not remove its lock files while running. Unknown upload debris requires a later writer-lock cleanup design; it is not removed by age alone.

日本語：既定は対象を表示するだけです。`--apply` で、設定日数を過ぎた終端ジョブの原稿・ラスターとジョブ／成果物記録を削除します。ワーカーリースや待機・送信・受理・UNKNOWNのDeliveryがある場合は除外します。メタデータ削除済みの孤立ディレクトリは、永続履歴で終端状態と期限を確認できるものだけ清掃します。未完了アップロードや任意ディレクトリは削除しません。清掃対象をDBへ先に記録し、途中終了後は次回再開します。終端DeliveryのPWG／登録ファイルも削除しますが、履歴・Receipt・監査は保持します。未commitの期限切れpreflightと承認も清掃します。1回の処理件数は制限されているため、残件がなくなるまで再実行してください。実行中のロックファイルは削除しないでください。

简体中文：默认仅显示候选项。`--apply` 删除超过保留期限的已结束任务原稿、栅格文件及任务／产物记录。存在Worker租约或排队、发送、受理、UNKNOWN投递时排除。已删除元数据的孤立目录仅在持久化历史能确认终端状态及期限时清理；不删除未完成上传或任意目录。先记录DB清理标记，中断后下次继续。结束投递的PWG／注册文件也删除，但保留历史、Receipt及审计。清理未commit的过期预检和审批。单次处理有数量限制，应重复运行至无待办；运行期间不要删除锁文件。

Artifact downloads reserve up to 1 GiB across processes sharing the canonical data root using 64 OS-locked 16 MiB slots, in addition to the local semaphore. Reservations release on download completion, disconnection or process exit. This is a single-host/shared-root budget, not a distributed storage quota. Budget lock files are persistent coordination files; do not delete them while a process is using the root.

日本語：成果物の取得は同一データrootのプロセス間で合計1GiBまで予約し、16MiB単位・64枠のOSロックで共有します。配信完了・切断・プロセス終了で解放します。複数ホストの分散容量管理ではありません。使用中の予算ロックファイルを削除しないでください。

简体中文：同一数据root的进程共享最多1GiB下载预算，通过64个16MiB的OS锁槽协调。下载完成、断开或进程结束时释放。这不是跨主机分布式配额；使用数据root期间不要删除预算锁文件。

Private download snapshots live under `artifact-snapshots`, hold an OS lock and normally disappear when the request finishes. Maintenance considers at most 128 entries per pass, removing only recognized `artifact-snapshot-*.part` files older than one hour whose locks can be acquired. Active downloads and unrelated files are skipped. Crash leftovers release their locks and become eligible on a later pass.

日本語：私有スナップショットは専用ディレクトリとOSロックで管理します。通常は取得終了時に消えます。清掃は1時間以上前の専用名のファイルで、ロックを取得できるものだけを対象とし、進行中の配信と無関係なファイルを除外します。

简体中文：私有快照位于专用目录并持有OS锁，通常在下载结束时删除。清理仅处理超过一小时、名称匹配且可取得锁的文件，跳过正在下载及无关文件。

Color attempts in QUEUED/RUNNING also protect the source. Color output inside the source job follows its retention policy; color attempt records/events remain. Abandoned color staging older than one hour is removed only when its writer lock is free. / 待機・実行中の色処理も原本を保護し、色成果物は原本と同じ保持期限で清掃、記録は保持します。孤立stagingは1時間超かつwriter lock解放済みの場合のみ削除します。 / 排队及运行中的颜色尝试保护原稿，颜色输出与原稿一同清理，记录和事件保留。孤立staging仅在超过一小时且writer lock空闲时删除。
