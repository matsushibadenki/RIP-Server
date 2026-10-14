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

Offline verify caps input at 16 MiB, rejects duplicate JSON keys, validates profile, sequence order, completion statuses and the canonical digest, and requires a separately supplied pinned digest. Unsigned archives are not authenticated: an attacker can replace both data and an untrusted digest. Keep the pinned digest through an independently trusted channel. Exported audit identities and operations are administrative data; protect the destination appropriately. Keys, bearer tokens and original request bodies are not added by this feature.

Automatic scheduling, object/WORM storage, audit deletion, archive retention and KMS integration remain planned. No production records are deleted by this implementation.

## 日本語

ローカル管理者CLIで監査記録を不変のarchiveとして保存します。既存DBへのファイルアクセスが必要で、通常のschema移行が実行される場合があります。Agentから任意の保存先を受け付けるAPIは追加しません。

`create` はDB、開始sequence（その値を除く）、終了時刻（Unix秒、その値を除く）、件数1〜1000、出力先を指定します。順番に完了済み記録を採用し、未完了または期限前の記録で停止します。未完了記録を飛ばしません。次回は `payload.through` を開始sequenceにします。中断した監査操作が進行を止める場合は調査が必要です。

1バッチはcompact JSONで8 MiB以内です。JCS digest付きarchiveと作成の監査記録を同じDBトランザクションで保存し、元の監査記録は保持します。出力前にIDとdigestを表示し、ファイルは上書きせず同期保存します。中断時にはIDを使って別の出力先へ `export` できます。DB競合時は失敗し、再実行が必要です。

`verify` は16 MiB以内の入力を厳密に解析し、重複キー、profile、sequence、完了status、digestを検証します。信頼できる別経路で保持したdigestを指定してください。未署名archiveでは、内容とdigestが同時に置換された場合の真正性は保証しません。出力には監査主体・操作情報が含まれるので管理者用データとして保管してください。

object storage、自動実行、元記録の削除、archive保持期限、KMS連携は未実装です。

## 简体中文

本地管理员CLI创建不可变审计快照，需要访问现有数据库文件，打开时可能执行正常schema迁移。本功能不增加允许Agent指定任意主机路径的API。

`create` 参数为数据库、排除式起始sequence、排除式截止Unix秒、1–1000条记录及输出文件。按sequence选取连续的已完成前缀，遇到未完成或过新的记录即停止，不跳过记录。下一批使用 `payload.through` 作为起始游标；中断的未完成操作需调查处理。

每批compact JSON不超过8 MiB，采用JCS摘要。归档及创建审计在同一DB事务中保存，保留原记录。写出前显示ID和摘要；仅创建新文件并同步，不覆盖已有文件。中断后可按ID导出至新路径，DB竞争失败后需重试。

离线验证输入不超过16 MiB，拒绝重复键，检查profile、sequence、完成status及摘要，并要求通过独立可信渠道保存的摘要。未签名归档的内容和不可信摘要同时被替换时不能证明真实性。导出包含管理员审计主体及操作信息，应保护文件。

对象存储、自动调度、原记录删除、归档保留期限、KMS集成仍未实现。

## Signed archives / 署名付きarchive / 签名归档

```sh
JRIP_RECEIPT_SIGNING_KEY=/secure/provider.pk8 cargo run -p rip-storage --bin jrip-audit -- create-signed data/jobs.sqlite3 0 1791558000 100 signed.json
cargo run -p rip-storage --bin jrip-audit -- verify-signature signed.json TRUSTED_PUBLIC.bin
cargo run -p rip-storage --bin jrip-audit -- verify-current data/jobs.sqlite3 signed.json TRUSTED_PUBLIC.bin
```

English: `create-signed` requires the configured PKCS#8 Ed25519 key; missing, invalid or revoked keys fail without publishing an archive. Ordinary `create` remains unsigned even if the environment contains a key. The archive signature uses the separate domain `org.jrip.audit-archive-ed25519-v1` and `signature.digestProfile:org.jrip.jcs-safe-v1`. Its message is UTF-8 domain name, zero byte, and the complete canonical archive after removing only signature.value. The payload digest retains its original meaning. Signed and unsigned archives share the archive-v1 payload contract. Signature metadata and proof are persisted atomically with the snapshot and creation audit, within the same size limit. Export never signs or modifies stored evidence.

`verify-signature` validates both the archive contract and Ed25519 signature using a separately trusted raw 32-byte public key. It does not check revocation. `verify-current` additionally requires a registered, currently non-revoked key in the specified existing repository. As with Receipt keys, revocation conservatively rejects all historical signatures from that key for current-trust verification. It preserves the historical bytes and offline cryptographic validity. The embedded publicKey is informational, not a trust anchor. The existing public-key registry and admin revocation API apply to this signing key; KMS and object/WORM storage are not implemented.

日本語：`create-signed` は設定済みPKCS#8 Ed25519鍵を必須とし、未設定・不正・失効済みならarchiveを保存しません。通常の `create` は環境変数に鍵があっても未署名です。専用domain `org.jrip.audit-archive-ed25519-v1`、ゼロbyte、signature.valueだけを除いたarchive全体のJCSを署名します。元のpayload digestは維持し、署名と作成監査を同じDBトランザクションで保存します。export時に再署名しません。

`verify-signature` は別経路で信頼した32-byte公開鍵で署名を検証し、失効は確認しません。`verify-current` は指定DBで登録済みかつ現在失効していないことも確認します。失効後は過去の署名も現在の信頼判定では拒否しますが、保存内容は変更しません。埋め込み公開鍵だけを信頼しないでください。既存の鍵一覧・管理者失効APIを共用し、KMSとobject storageは未実装です。

简体中文：`create-signed` 必须配置PKCS#8 Ed25519密钥；缺失、无效或已撤销时不发布归档。普通 `create` 始终不签名。独立domain为 `org.jrip.audit-archive-ed25519-v1`，签名消息包含domain、零字节及仅移除signature.value后的完整JCS归档。payload摘要保持原含义；签名、快照和创建审计原子保存，导出时不重新签名。

`verify-signature` 使用独立可信的32-byte公钥验证，不检查撤销。`verify-current` 还检查指定DB中的密钥已注册且当前未撤销。撤销后保守拒绝该密钥的全部历史签名用于当前信任判断，但保持历史字节及离线密码学有效性。嵌入公钥不构成信任锚；复用现有公钥列表和管理员撤销API，KMS及对象存储仍未实现。

## Filesystem vault / ファイルシステム保管先 / 文件系统归档库

```sh
cargo run -p rip-storage --bin jrip-audit -- store data/jobs.sqlite3 ARCHIVE_UUID /admin-configured/vault
cargo run -p rip-storage --bin jrip-audit -- vault-status data/jobs.sqlite3 ARCHIVE_UUID /admin-configured/vault
```

English: `store` copies the exact compact JSON representation of the stored archive to `{UUID}.json` in an existing administrator-controlled directory, which may be a separately mounted volume. A synced staging file is published by a non-overwriting hard link, read back and compared with the DB snapshot, then the file and directory are synced. Only after success does a DB transaction record the destination, SHA-256 of the complete file, byte count, verification time and audit-vault-store event. The full-file digest differs from the archive payload digest. Replay verifies contents again and preserves the original acknowledgement/time. A crash between filesystem publication and DB acknowledgement is recovered by replay. Conflicting files or final-path symlinks are rejected. Publication may leave an unacknowledged file on failure; original DB evidence is retained. Process termination may leave `.jrip-audit-*.tmp` staging files; these are not acknowledged archives and currently require administrator cleanup.

`vault-status` rechecks current contents rather than assuming the stored acknowledgement means the file still exists. acknowledged:true with contentMatches:false indicates missing or changed evidence. It does not delete, repair or resign files. `store` can restore a missing file; it never overwrites a conflicting file. The directory must be controlled by administrators; this is not a sandbox for a hostile shared writable directory. No Agent-supplied paths or remote tools are added.

storageMeaning is filesystem-sync and worm is false. Local filesystem publication/replay is tested; actual NFS durability, disconnected-volume recovery and Windows directory-sync behavior remain Pending. A successful filesystem sync does not certify remote replication or provider retention. Object storage, WORM guarantees, automated retention/pruning and KMS remain unimplemented. Source audit records and database snapshots are never deleted.

日本語：`store` は既存の管理者専用ディレクトリへ `{UUID}.json` を出力します。別volumeも指定できます。同期した一時ファイルを上書きしないhard linkで公開し、DB内容との一致とファイル・ディレクトリの同期後に保管確認と監査を原子的に記録します。ファイル全体のSHA-256はpayload digestとは別です。再送は内容を確認し、元の確認時刻を保持します。公開後・DB記録前の中断は再送で復旧します。不一致ファイルや出力先symlinkは拒否します。強制終了後に残った一時ファイルの清掃は管理者が行います。

`vault-status` は現在の内容を再確認し、過去の確認があっても欠落・改変を検出します。欠落はstoreで復旧でき、不一致は上書きしません。管理者が管理する保存先専用で、敵対者が書き換える共有ディレクトリへの安全性を保証するものではありません。ローカル検証済みですが、実NFSの永続性、切断復旧、Windowsのディレクトリ同期はPendingです。WORM・外部replication・保持期限を保証せず、元記録を削除しません。

简体中文：`store` 将DB快照输出为管理员控制目录中的 `{UUID}.json`，可使用独立挂载卷。同步临时文件后通过不覆盖的hard link发布，读回比对并同步文件及目录，再原子记录完整文件SHA-256、字节数、确认时刻及审计。文件摘要与payload摘要不同。重放重新检查内容并保留首次确认，发布后DB记录前中断可重放恢复；冲突文件和最终路径symlink被拒绝。强制退出遗留的临时文件需管理员清理。

`vault-status` 重新检查当前文件，历史确认不等于文件仍然存在。缺失可由store恢复，冲突不覆盖。仅支持管理员控制的目录，不保证敌对共享可写目录的安全性。本地流程已验证；实际NFS持久性、断连恢复及Windows目录同步仍Pending。不保证WORM、远程复制或保留期限，不删除原记录。

## Archive catalogue / archive一覧 / 归档目录

```sh
cargo run -p rip-storage --bin jrip-audit -- list data/jobs.sqlite3 - 1791558000 50
cargo run -p rip-storage --bin jrip-audit -- list data/jobs.sqlite3 NEXT_AFTER_UUID 1791558000 50
```

English: `list DATABASE AFTER_UUID_OR_DASH BEFORE_UNIX_SECONDS LIMIT` returns metadata for archives created strictly before the cutoff, ordered by canonical UUID. Use `-` for the first page, then nextAfter; limit is 1–100 and hasMore indicates another page. It works with existing snapshots without rewriting them. Results contain archive ID, createdAt, audit after/through sequences, record count, compact byte size, payload digest, signature profile and acknowledgedVaultCount; audit bodies and filesystem paths are omitted. UUID order is not chronological. Pagination is not a fixed snapshot: concurrent inserts sorting before the cursor appear only when scanning again from the beginning. Keep the cutoff unchanged across pages. A signature profile is metadata, not a verification result. Vault acknowledgements are historical; use vault-status for a current content check. This read does not delete or apply retention. A known archive ID remains recoverable with export even if a create command's output was lost.

日本語：`list` は作成時刻がcutoffより前のarchiveを、正規UUID順・最大100件で返します。初回は `-`、次回は `nextAfter` を指定し、同じcutoffを使います。本文・保存先pathを出さず、ID、記録範囲、件数、容量、digest、署名profile、過去の保管確認数だけを返します。既存archiveを書き換えません。UUID順は時刻順ではなく、ページングも固定snapshotではありません。同時に追加されたIDがcursorより前なら先頭からの再走査が必要です。署名profileは検証結果ではなく、保管確認数も現在状態ではありません。保持期限による削除は行いません。

简体中文：`list` 返回创建时间严格早于cutoff的归档，按规范UUID排序，每页最多100条。首次使用 `-`，后续使用 `nextAfter` 并保持相同cutoff。仅返回ID、记录范围、数量、大小、摘要、签名profile及历史存储确认数，不返回审计正文或路径，不重写既有归档。UUID顺序不是时间顺序，分页不是固定快照；并发新增ID若位于游标之前，需从头重新扫描。签名profile不是验证结果，历史确认不是当前文件状态。本操作不删除记录或执行保留策略。
