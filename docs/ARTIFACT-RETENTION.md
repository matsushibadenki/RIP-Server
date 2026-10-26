# Common artifact retention / 共通artifact保持期限 / 共同artifact保留期限

## English

🟢 [Done] Local source documents, RIP rasters, color-managed outputs and terminal
delivery spool share one job-family retention unit. Maintenance considers the age
of the source **and** related color attempts, delivery events and pipeline events.
A recently finished conversion or delivery now protects its older source family;
source age alone is no longer sufficient to purge the files.

All of these must hold before a new purge claim:

- The source has a known terminal state and is old enough, with no worker lease.
- Every related color/delivery/pipeline stage is terminal and old enough; color cleanup is finished.
- Related timestamps are present, nonnegative integers; unknown states/ages remain protected.
- No explicit `retainUntil` is still in the future.

`QUEUED`, `RUNNING`, `SENDING`, `ACCEPTED`, `UNKNOWN`, nonterminal pipelines and
unknown future states remain protected regardless of age. The same guard applies
to dry-run selection, metadata-deleted orphans with terminal history, and the actual
SQLite purge claim. Preview is not an authorization token: the claim rechecks live
state under the database writer lock. New stage admission already requires an
existing source job, so it cannot begin after the claim removes that metadata.

The effective local deletion boundary is the later of the latest family stage
activity plus the operator's retention-days policy, and the explicit retainUntil.
This is a minimum retention boundary, not a scheduled deletion time. Active stages,
cleanup, missing timestamps or UNKNOWN can keep the family longer.

## Explicit minimum retention

`GET /api/v1/jobs/{id}/retention` requires `jobs:read`. An unset policy has
`revision:0`, `retainUntil:null`; it does not disable the normal age/dependency rules.
`PUT` on the same path requires `settings:write` and accepts:

```json
{
  "expectedRevision": 0,
  "retainUntil": 1900000000,
  "reason": "Keep source and color proof artifacts"
}
```

Replace retainUntil with a future Unix timestamp within 3650 days. Expected revision
is the retention-policy revision, not the job revision. A write increments it;
a stale revision, shorter deadline or already-claimed purge returns 409. Repeating
the same deadline with the current revision is a no-op preserving the prior record.
There is no shorten/remove endpoint. Invalid revision/time/reason returns 400 with
English/Japanese/Simplified Chinese recovery text. The normal credential policy
applies; this endpoint does not add human approval or tenant isolation.

The `org.jrip.artifact-retention-policy-v1` record contains the job ID, scope
(source/raster/color-managed/delivery-spool), deadline, revision, actor, reason,
previous deadline and correlation ID. Each change, its revision history and its
correlated audit record commit atomically. Audit failure rolls back the hold change.
An extension and a purge claim serialize: either the hold prevents claiming, or the
claim prevents a late hold. Policies survive restart and continue protecting files
if terminal job metadata was explicitly deleted. Such metadata deletion still does
not itself remove the files or restore their normal job APIs.

## Maintenance and evidence

The existing administrator CLI remains opt-in:

```sh
cargo run -p jrip-server --bin jrip-maintenance -- --retention-days 30
# Review the dry run, then explicitly add --apply when appropriate.
```

Preview/apply report `artifactRetentionProfile:org.jrip.artifact-family-retention-v1`
and `ageBasis:latest-related-stage-activity`. Apply uses the existing durable purge
journal and trash/cleanup flow: it removes the job tree (source/raster/color files)
and eligible terminal delivery `.pwg`/`.json` spool. Interrupted claims resume on the
next apply. A hold cannot be added after claim, even if physical removal is pending.
Already-claimed purges are not retrospectively canceled by this feature.

Audit/event records, receipts, source/color reports, retention revisions and policy
metadata remain available as historical evidence; that does not mean their artifact
bytes still exist. ICC registry entries and reusable Color Policy definitions are
shared configuration, so this family purge does not remove them. The policy is a
local application rule, not a filesystem/object-store WORM guarantee. Existing
abandoned-upload/snapshot/staging cleanup retains its separate lock-based rules.
Retention policies and their revision history count as registration evidence for
upload cleanup, even if legacy source history is missing; upload GC cannot bypass
a family hold or invent terminal history after its expiry.

Tests use temporary databases and directories. They cover newly completed related
stages, old stages becoming eligible, missing ages, metadata-deleted orphans,
cleanup and unknown states, preview/claim changes, hold/purge concurrency,
revision conflicts, no shortening, expiry, audit rollback, restart, scoped APIs,
file preservation and interrupted purge recovery.

⭕️ [Pending] Actual shared-NFS deployment and cross-host locking acceptance.
🔴 [Later] Independent per-artifact expiries, remote object-store expiry adapters,
and standardized retention references in the complete content-addressed artifact contract.

## 日本語

🟢 [Done] 原稿・RIP・色変換成果物・終端の送信spoolをjob-family単位で保持します。
元jobだけでなく、関連する色変換・送信・pipelineの最終活動時刻も保持日数の基準にします。
新しく完了した派生工程がある場合、元jobが古くても削除しません。処理中、UNKNOWN、cleanup待ち、
時刻の欠落／不正も保護します。preview、metadata削除済みの孤立job、削除claimで同じ条件を使い、
claim直前に再確認します。

上記GETは `jobs:read`、PUTは `settings:write` が必要です。保持policy専用のexpectedRevision、
3650日以内の将来のretainUntil（Unix秒）、理由を指定します。期限の短縮・削除はできません。
古いrevision・短縮・claim後の更新は409、不正な入力は3言語の回復案付き400です。
同じ期限を現在のrevisionで再送した場合は既存記録を維持します。人の承認機能を追加するものではありません。

変更前後の期限、主体、理由、revision履歴とcorrelation ID付き監査を同一DBトランザクションで保存します。
監査失敗時は変更を取り消し、保持延長と削除claimの競合では片方だけが成立します。
再起動やmetadata削除後もファイル保護を維持します。期限到来だけでは自動削除せず、関連工程も十分古くなり、
管理者が既存maintenanceを明示applyした場合だけ削除します。

削除単位はjobツリーと終端delivery spoolで、DB上の監査・Receipt・report・保持履歴は証跡として残します。
共用ICC registry／Color Policyは削除しません。claim済みの削除は再起動後に再開し、後からholdを追加できません。
旧記録で元履歴がない場合も、保持policy／履歴を登録証跡として扱い放置upload清掃から除外します。
filesystemのWORMや独立したファイルごとのTTLとは区別します。試験は一時DB・ディレクトリで実行します。

⭕️ [Pending] 実共有NFS／複数hostのロック受入試験。
🔴 [Later] 個別artifact期限、remote object store、content-addressed artifact契約への保持参照。

## 简体中文

🟢 [Done] 原稿、RIP、颜色转换artifact及终态发送spool采用统一job-family保留单位。
保留期限同时考虑相关颜色、发送及pipeline的最新活动；原job较旧但派生阶段刚完成时不会删除。
处理中、UNKNOWN、cleanup等待及时间缺失／无效均受保护。preview、metadata已删除的孤立job及实际claim使用相同条件，
claim时再次检查实时状态。

上述GET需要 `jobs:read`，PUT需要 `settings:write`，提供保留policy专用expectedRevision、
3650天以内的未来retainUntil（Unix秒）及原因。不允许缩短或移除期限；旧revision、缩短及claim后更新返回409，
无效输入返回带三语言恢复信息的400。同期限配当前revision重放保留原记录，不增加人工审批功能。

原子保存变更前后期限、身份、原因、revision历史及带correlation ID的审计。审计失败则回滚；
hold与claim并发时仅一方成立。重启或metadata删除后仍保护文件。
到期不自动删除，还须全部相关阶段足够旧并由管理员显式maintenance apply。

删除job目录及终态delivery spool，保留DB审计、Receipt、report及保留历史作为证据；
不删除共享ICC registry或Color Policy。已claim的删除可在重启后恢复，不能再追加hold。
旧记录缺少源历史时，保留policy／历史仍视为注册证据，避免放置upload清扫绕过。
这不是filesystem WORM或独立文件TTL。测试仅使用临时数据库和目录。

⭕️ [Pending] 实际共享NFS／多host锁验收。
🔴 [Later] 单独artifact期限、remote object store及完整content-addressed契约的保留引用。
