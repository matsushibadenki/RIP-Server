# Distributed snapshot budget / 分散snapshot予算 / 分布式快照预算

## English

Artifact downloads, artifact verification, color-managed artifact downloads and
existing internal callers of `snapshot_file` can share an optional central
reservation budget across independent data roots and hosts. Existing per-process
semaphores and per-root OS locks still apply. The coordinator runs in an ordinary
J-RIP API process; clients communicate over HTTP(S), never access its database.
Its SQLite database must live on the authority's local disk. Multiple authorities
with separate databases do **not** share a budget. HA, distributed database consensus,
physical disk quotas, and all-stage compute/staging budgets are outside this scope.

Authority configuration (persisted on first startup):

```sh
export JRIP_SNAPSHOT_BUDGET_BYTES=1073741824
```

Capacity defaults to 1 GiB; it must be a 16 MiB multiple between 16 MiB and 1 TiB.
Changing an existing database's capacity through environment variables is rejected
at startup, so a second process cannot silently reset an active pool. Reservations
round up to 16 MiB; a single request is 1 byte to 1 GiB. Counts and admission are
serialized in a SQLite write transaction. The UUID request ID and original size
are immutable; replay returns the original HELD or RELEASED record without charging
again. Released IDs never reacquire capacity. Catalog reads use a single snapshot.

Each client API/color worker using artifact snapshots configures:

```sh
export JRIP_SNAPSHOT_COORDINATOR_URL=https://budget.example.org/api/v1/budgets/snapshots/
export JRIP_SNAPSHOT_COORDINATOR_TOKEN=replace-with-worker-specific-secret
```

Both variables are required together. With neither, prior local behavior remains.
Use a separate credential identity per host and grant it `budgets:reserve` in the
authority's `JRIP_AUTH_POLICY`. Credential rotation may preserve that identity.
The application serving client downloads still checks its normal `artifacts:read`
permission; that permission alone does not grant central budget mutation.

Production URLs require HTTPS. Literal loopback HTTP addresses are allowed for local
testing; HTTP DNS hostnames, userinfo, query strings, fragments and a missing trailing
slash are rejected. Requests use native TLS trust, no proxy, redirect or automatic
retry, a 3-second connect/5-second total timeout and a 32 KiB response bound.
Tokens are sent as Bearer headers and are not included in records or logs. Configure
HTTPS on the authority's trusted reverse proxy; no external VPS needs DB access.

Central endpoints:

| Method / path under `/api/v1/budgets/snapshots` | Permission | Contract |
| --- | --- | --- |
| POST `/reservations` | `budgets:reserve` | `{requestId,bytes}`; owner comes from authentication |
| POST `/reservations/{id}/release` | `budgets:reserve`, same owner | `{expectedBytes}` using the rounded reservation size |
| GET `?after=UUID&limit=50` | `admin` | Active reservations, capacity, held total, UUID cursor; limit 1–100 |
| POST `/reservations/{id}/reconcile` | `admin` | `{expectedOwner,expectedBytes,cleanupConfirmed:true}` after actual cleanup |

The catalogue is not a fixed multi-request snapshot. All routes pass the existing
API audit middleware. Error messages include English/Japanese/简体中文. A full pool
returns 503 `artifact_budget_exhausted`; coordinator/authentication/protocol failures
at a client return 503 `snapshot_coordinator_unavailable`, with no local fallback.

The client reserves **before copying** a private artifact snapshot. Its local
filename includes the reservation UUID. After file deletion succeeds, it schedules
a bounded owner-authenticated release. Body completion, disconnect, integrity errors
and invalid-range responses all drop the guard. Failed deletion, process crash,
missing runtime, lost admission/release ACK or network failure conservatively retain
quota. A HELD record has **no TTL**: an unreachable process may still retain bytes,
so time alone must not permit another host to reuse its reservation. This is a
budget for admitted temporary snapshot allocations, not a hard filesystem quota
or a guarantee about the exact instant open file descriptors reclaim disk blocks.

Recovery: stop or fence the owning host/process first; locate its snapshot by UUID
in `artifact-snapshots/artifact-snapshot-{reservation-id}-*.part`; remove orphaned
files only after confirming no reader/writer remains. Admission ACK loss may leave
no file at all. Existing local maintenance can remove orphaned snapshots but does
not automatically release central quota. Then use the admin catalogue's exact ID,
owner and rounded bytes with `cleanupConfirmed:true`. Reconciliation is idempotent,
keeps a RELEASED tombstone, and cannot alter another reservation. Do not use it as
a capacity reset while a holder might still run. The server records the operator's
assertion; it cannot remotely prove filesystem cleanup.

Source uploads, source-validation snapshots, interpreter tmpfs/RIP staging, color
staging and persistent artifact storage are **not** covered by this new remote
pool; their existing local limits remain. Test coverage includes concurrent
admission, duplicate/conflicting IDs, ownership/scopes, restart, independent-root
HTTP clients, disconnect/hash-failure cleanup, lost ACKs, redirects, bounded
responses and authority outage. Real cross-VPS HTTPS/certificate and partition
acceptance remains Pending.

## 日本語

成果物の取得・整合性検査と、同じsnapshot経路を使う色成果物・内部処理に、任意の中央API予算を
追加しました。異なるroot・hostからHTTPSで予約し、中央SQLiteの単一トランザクションで
合計容量を確認します。既存のプロセス内・root内の制限も引き続き有効です。
通常VPSのworkerが中央DBへ直接接続する必要はありません。中央DBは権威サーバーのローカル
ディスクに置き、別DBを持つ複数サーバーを同一予算として運用しないでください。

中央の `JRIP_SNAPSHOT_BUDGET_BYTES` は既定1 GiB、16 MiB単位、最大1 TiBです。
初回値を永続化し、起動時に値を変えても既存予算は上書きしません。予約は16 MiBへ切り上げ、
同じUUID・元容量の再送は重複計上しません。解放済みIDでの再予約も行いません。

clientは `JRIP_SNAPSHOT_COORDINATOR_URL` と `JRIP_SNAPSHOT_COORDINATOR_TOKEN` を両方設定します。
URLは上記の `/api/v1/budgets/snapshots/` まで含めます。中央認証policyではhostごとに異なる
identityへ `budgets:reserve` を付与してください。成果物利用者の `artifacts:read` とは別です。
両環境変数が未設定なら従来のローカル動作を維持します。本番はHTTPS必須、試験用のIP literal
loopback HTTPのみ例外です。redirect・proxy・自動再試行は使わず、通信時間と応答容量を制限します。

コピー前に予約し、予約UUIDを含む一時ファイルを削除できてから解放します。取得完了・切断・
hash不一致でも同じ処理を行います。通信障害・応答喪失・削除失敗・プロセス停止では予約を保持し、
時刻だけで自動解放しません。中央に接続できない場合は503で拒否し、ローカルだけで続行しません。

復旧時は所有hostを停止または隔離し、該当UUIDのsnapshotと使用中handleが残っていないことを
確認します。その後、管理者が一覧のID・owner・丸め後bytesを指定し、`cleanupConfirmed:true` で
reconcile APIを呼びます。再送可能な解放記録を保持します。サーバーが確認するのは管理者の申告であり、
遠隔ファイルの削除を自動証明する機能ではありません。既存のローカル清掃は中央予約を解放しません。

原稿投入・原稿検査・RIP／色変換staging・永続成果物の全容量は対象外です。HAや物理disk quotaとも
区別します。独立rootとlocalhost HTTPで競合・再起動・解放・障害を検証し、実VPS間HTTPS・証明書・
回線分断の受入試験はPendingです。

## 简体中文

新增可选中央API预算，用于成果下载、完整性验证及使用同一快照路径的色彩成果和内部操作。
不同root与host通过HTTPS申请预留；中央SQLite写事务原子检查合计容量。原有进程内与root内限制
继续生效，worker无需直接访问中央DB。中央DB须位于权威节点本地磁盘；独立DB不能组成共享预算。

中央 `JRIP_SNAPSHOT_BUDGET_BYTES` 默认1 GiB，按16 MiB取整，最大1 TiB。首次配置持久化，后续
不允许通过环境变量静默重置。相同UUID及原始大小的重放不会重复计费，已释放ID不重新占用容量。
client须同时设置 `JRIP_SNAPSHOT_COORDINATOR_URL` 和 `JRIP_SNAPSHOT_COORDINATOR_TOKEN`，
并为每个host分配拥有 `budgets:reserve` 的独立identity；普通成果访问仍须 `artifacts:read`。
两项均未设置时保留原本地行为。生产使用HTTPS，仅literal loopback IP可用HTTP测试；禁止redirect、
proxy和自动重试，并限制超时及响应大小。

复制前预留，临时文件名包含预留UUID；删除成功后才请求释放。完成、断开、hash错误均清理。
删除失败、崩溃、网络错误或ACK丢失会保守保留预算，无自动TTL回收；中央不可达时返回503，
不回退到本地预算。恢复时先停止或隔离拥有者，确认对应文件及使用中的handle已清理，再由管理员以
准确ID、owner、取整后bytes及 `cleanupConfirmed:true` 调用reconcile。保留可重放的释放记录；
服务器记录管理员确认，无法自动证明远端文件已删除。本地维护也不会自动释放中央预留。

原稿上传、原稿检查、RIP/色彩暂存和持久成果容量不属于该远程池，也不提供HA或物理磁盘配额。
已以独立root及localhost HTTP验证并发、身份、重启、清理和故障；真实跨VPS HTTPS、证书和网络分区
验收标为Pending。
