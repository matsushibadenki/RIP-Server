# Audit archive / 監査アーカイブ / 审计归档

## English

The local administrator CLI creates immutable, bounded snapshots of durable audit records. It requires filesystem access to an existing repository database; it does not expose a new remote scope or accept paths from an Agent. Repository opening can apply normal schema migrations.

```sh
cargo run -p rip-storage --bin jrip-audit -- create data/jobs.sqlite3 0 1791558000 100 audit-001.json
cargo run -p rip-storage --bin jrip-audit -- export data/jobs.sqlite3 ARCHIVE_UUID recovered.json
cargo run -p rip-storage --bin jrip-audit -- verify audit-001.json sha256:INDEPENDENTLY_PINNED_DIGEST
```

Use your actual database path. `create` arguments are DATABASE, exclusive AFTER sequence, exclusive BEFORE Unix timestamp in seconds, LIMIT (1–1000), and output path. It selects a contiguous completed prefix in sequence order. An unfinished or newer operation stops selection; later records are not silently skipped. Empty selection returns an error. The returned payload.through is the next batch's AFTER cursor. An abandoned unfinished admission can block further progress; it requires investigation, rather than automatic fabrication of a completion result.

Profile `org.jrip.audit-archive-v1` wraps payload `{profile,id,createdAt,after,through,before,records}` with `digestProfile:org.jrip.jcs-safe-v1` and the canonical SHA-256 digest of that payload. A batch is limited to 1000 records and 8 MiB of compact JSON. The archive and a local audit-archive-create record commit in one database transaction. A database error leaves neither. Original audit records are retained and remain available through the admin API; archive creation does not reduce database size.

The CLI prints the archive ID and digest before writing the export. Output uses create-new, owner-only permissions on Unix and file sync; existing paths are never overwritten. An interrupted write can leave a partial file, so recovery must use a fresh output path and the printed archive ID. Stored snapshots do not change when later audit records finish. Concurrent database conflicts fail rather than publishing an inconsistent snapshot; retry creation after resolving contention.

Offline verify caps input at 16 MiB, rejects duplicate JSON keys, validates profile, sequence order, completion statuses and the canonical digest, and requires a separately supplied pinned digest. The archive is not digitally signed: an attacker can replace both data and an untrusted digest. Keep the pinned digest through an independently trusted channel. Exported audit identities and operations are administrative data; protect the destination appropriately. Keys, bearer tokens and original request bodies are not added by this feature.

Automatic scheduling, external/WORM storage, audit deletion, archive retention and signing/KMS integration remain planned. No production records are deleted by this implementation.

## 日本語

ローカル管理者CLIで監査記録を不変のarchiveとして保存します。既存DBへのファイルアクセスが必要で、通常のschema移行が実行される場合があります。Agentから任意の保存先を受け付けるAPIは追加しません。

`create` はDB、開始sequence（その値を除く）、終了時刻（Unix秒、その値を除く）、件数1〜1000、出力先を指定します。順番に完了済み記録を採用し、未完了または期限前の記録で停止します。未完了記録を飛ばしません。次回は `payload.through` を開始sequenceにします。中断した監査操作が進行を止める場合は調査が必要です。

1バッチはcompact JSONで8 MiB以内です。JCS digest付きarchiveと作成の監査記録を同じDBトランザクションで保存し、元の監査記録は保持します。出力前にIDとdigestを表示し、ファイルは上書きせず同期保存します。中断時にはIDを使って別の出力先へ `export` できます。DB競合時は失敗し、再実行が必要です。

`verify` は16 MiB以内の入力を厳密に解析し、重複キー、profile、sequence、完了status、digestを検証します。信頼できる別経路で保持したdigestを指定してください。署名ではないため、内容とdigestが同時に置換された場合の真正性は保証しません。出力には監査主体・操作情報が含まれるので管理者用データとして保管してください。

外部保管、自動実行、元記録の削除、archive保持期限、署名・KMS連携は未実装です。

## 简体中文

本地管理员CLI创建不可变审计快照，需要访问现有数据库文件，打开时可能执行正常schema迁移。本功能不增加允许Agent指定任意主机路径的API。

`create` 参数为数据库、排除式起始sequence、排除式截止Unix秒、1–1000条记录及输出文件。按sequence选取连续的已完成前缀，遇到未完成或过新的记录即停止，不跳过记录。下一批使用 `payload.through` 作为起始游标；中断的未完成操作需调查处理。

每批compact JSON不超过8 MiB，采用JCS摘要。归档及创建审计在同一DB事务中保存，保留原记录。写出前显示ID和摘要；仅创建新文件并同步，不覆盖已有文件。中断后可按ID导出至新路径，DB竞争失败后需重试。

离线验证输入不超过16 MiB，拒绝重复键，检查profile、sequence、完成status及摘要，并要求通过独立可信渠道保存的摘要。归档没有数字签名；内容和不可信摘要同时被替换时不能证明真实性。导出包含管理员审计主体及操作信息，应保护文件。

外部存储、自动调度、原记录删除、归档保留期限、签名及KMS集成仍未实现。
