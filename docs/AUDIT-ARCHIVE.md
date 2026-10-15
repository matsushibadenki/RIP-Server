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

Automatic scheduling, object/WORM storage, source-audit deletion, external-storage retention and KMS integration remain planned. Creation and export do not delete records.

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

storageMeaning is filesystem-sync and worm is false. Local filesystem publication/replay is tested; actual NFS durability, disconnected-volume recovery and Windows directory-sync behavior remain Pending. A successful filesystem sync does not certify remote replication or provider retention. Object storage, WORM guarantees, automated retention/pruning and KMS remain unimplemented. Source audit records are retained. Local database snapshots can be pruned explicitly as described below.

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

## Retention assessment / 保持期限の対象確認 / 保留期限评估

```sh
cargo run -p rip-storage --bin jrip-audit -- retention-preview data/jobs.sqlite3 /admin-configured/vault 30 - 100
```

English: `retention-preview DATABASE VAULT_DIRECTORY DAYS AFTER_UUID_OR_DASH LIMIT` assesses local database snapshots older than 1–3650 days. It returns at most 100 metadata entries per page and rereads the selected vault copy for each archive. A localSnapshotPruneCandidate requires an existing acknowledgement for this vault and current exact content equality. Reason codes are VERIFIED_VAULT_COPY, VAULT_ACKNOWLEDGEMENT_MISSING, VAULT_CONTENT_MISMATCH and EVIDENCE_UNVERIFIABLE. File/read/integrity errors never produce a positive candidate. This is a read-only assessment: dryRun:true, deletionImplemented:true. Original audit records, snapshots, acknowledgements and vault files are retained. Normal repository opening may still perform schema initialization/migration.

Candidate status is a prerequisite assessment, not permission to delete or a WORM/replication guarantee. It does not verify signatures or current key trust; use verify-current separately. It does not evaluate legal holds or operator retention policy. A copy can change after assessment. Local snapshot pruning with revalidation and durable evidence is described below; external-storage retention remains unfinished. For reproducible paging, use the returned before value with list; repeated retention-preview invocations recompute the age cutoff from the current time.

日本語：`retention-preview` は1〜3650日の保持期限と最大100件のページングを指定し、期限を過ぎたDB snapshotの保管先を再読込します。同じ保管先の確認記録と現在の完全一致がある場合だけ候補とします。確認なし・内容不一致・検証不能は理由コード付きで除外します。`dryRun:true`、`deletionImplemented:true` で、記録・snapshot・保管先を削除しません。DBを開く際の通常schema移行はあり得ます。

候補表示は削除許可や外部永続性保証ではありません。署名・現在の鍵状態、legal hold、運用上の保持ポリシーは別途確認が必要です。表示後にファイルが変わる可能性があるため、実削除時の再確認と永続削除証跡は後述のpruneを使います。各CLI実行でcutoffを再計算します。固定cutoffでの一覧には返されたbeforeをlistに指定してください。

简体中文：`retention-preview` 接受1–3650天保留期限，每页最多100条，并重新读取过期DB快照对应的归档文件。仅当所选目录存在历史确认且当前内容完全一致时列为候选；缺少确认、内容不匹配及无法验证分别返回原因代码。固定返回 `dryRun:true`、`deletionImplemented:true`，不删除记录、快照或文件。打开DB时可能执行正常schema迁移。

候选结果不是删除授权或远程持久性保证，不验证签名、当前密钥信任、legal hold或运维保留策略。评估后文件可能变化；实际删除时再验证及持久删除证据由下述prune提供。每次CLI重新计算cutoff；固定时间分页可将返回before用于list。

## Local snapshot pruning / DB内snapshotの削除 / 本地快照删除

```sh
cargo run -p rip-storage --bin jrip-audit -- prune data/jobs.sqlite3 ARCHIVE_UUID /admin-configured/vault 30 sha256:EXPECTED_ARCHIVE_DIGEST --apply
cargo run -p rip-storage --bin jrip-audit -- prune-record data/jobs.sqlite3 ARCHIVE_UUID
```

English: `prune` requires an explicit archive ID, selected vault, retention days (1–3650), expected payload digest from list/preview and --apply. It removes only that local audit_archives snapshot. Under a SQLite writer transaction it checks the expected digest, archive integrity and age, plus the vault acknowledgement's full-file digest/byte count. It rereads and syncs the matching vault file and directory before atomically saving an immutable org.jrip.audit-archive-prune-v1 record, deleting the snapshot and inserting audit-archive-prune. Any validation, filesystem, database or audit insertion failure aborts the transaction. Replay after restart returns the original record without a duplicate audit event; changed digest or vault conflicts.

Source audit records, public-key records, historical export acknowledgements and vault files remain. The prune record preserves archive ID, payload/full-file digests, byte count, record range/count, signature profile/key ID, target, cutoff and time. `vault-status` continues to check the external file against the recorded full-file hash after pruning. Replay reports a historical deletion, not current vault integrity. DB export/store for a pruned ID returns not found; recover the original JSON from the retained `{UUID}.json` vault file and use verify/verify-signature/verify-current as appropriate. No automatic import or resigning occurs. list reports only remaining local snapshots; prune-record retrieves evidence by the known ID.

The vault must stay administrator-controlled during and after pruning. Filesystem verification/sync is not a WORM or remote replication guarantee and cannot prevent an administrator changing/deleting the sole retained copy afterward. Apply your legal holds, key trust requirements and backup policy before explicitly invoking --apply. No production pruning was performed during development; tests use temporary databases and directories. SQLite freed pages are reusable; this does not run VACUUM or guarantee a smaller database file. Provider-specific S3 deletion, original-audit pruning and KMS remain planned.

日本語：`prune` はID、保管先、保持日数1〜3650、確認したpayload digest、`--apply` を必須とします。SQLite writerトランザクション内で期限・digest・保管ACKを検査し、保管ファイルの再読込と同期後に、削除証跡・DB snapshot削除・監査を原子的に保存します。検証や監査保存の失敗時は削除も取り消します。再送は元の証跡を返し、異なるdigest・保管先は拒否します。

元の監査、鍵、保管ACK、保管ファイルは保持します。削除証跡にID・hash・容量・記録範囲・署名metadata・期限・時刻を残し、削除後もvault-statusでファイルを検証できます。復旧には保管先のJSONを使います。DBからのexport/storeは削除済みIDで404相当となり、自動再取込・再署名はしません。prune-recordは指定IDの証跡を返します。

管理者専用保管先が前提で、後からの唯一のcopyの削除やWORM・遠隔replicationを保証しません。legal hold・鍵の信頼・backupの運用方針を確認してから適用してください。本番DBには適用せず、開発テストの一時データのみで検証しました。VACUUMは実行せず、DBファイル縮小は保証しません。

简体中文：`prune` 必须指定ID、归档库、1–3650天保留期、确认的payload摘要及 `--apply`。在SQLite writer事务中验证期限、摘要、存储ACK，重新读取并同步归档文件后，原子保存删除证据、删除DB快照及审计。验证或审计写入失败时回滚删除。重放返回首次证据，不重复审计；不同摘要或归档库被拒绝。

保留原审计、密钥、存储ACK和归档文件。删除证据记录ID、摘要、大小、记录范围、签名metadata、期限及时间；删除后vault-status仍检查完整文件hash。恢复需读取归档库JSON，DB export/store对已删除ID返回未找到，不自动导入或重新签名。prune-record按已知ID查询证据。

仅支持管理员控制的目录，不保证WORM、远程复制或后续管理员删除唯一copy时的安全。应用前须确认legal hold、密钥信任及backup策略。开发仅在临时DB与目录测试，未删除生产数据。不执行VACUUM或保证DB文件变小。

## HTTPS object vault / HTTPS object保管 / HTTPS对象归档库

Configure `JRIP_AUDIT_VAULT_URL` (an HTTPS directory URL ending in `/`) and `JRIP_AUDIT_VAULT_TOKEN` through your administrator secret environment, then run:

```sh
cargo run -p rip-storage --bin jrip-audit -- remote-store data/jobs.sqlite3 ARCHIVE_UUID
cargo run -p rip-storage --bin jrip-audit -- remote-status data/jobs.sqlite3 ARCHIVE_UUID
```

English: This adapter defines a conditional HTTP object-vault contract, not an S3 client. The endpoint must accept authenticated PUT/GET at `{base}/{UUID}.json`, atomically honor PUT `If-None-Match: *` and return 412 when already present. Creation may return 200/201/204. GET must return 200 with the exact uncompressed bytes or 404; it must support read-after-write consistency. The conditional request follows [RFC 9110](https://www.rfc-editor.org/rfc/rfc9110.html#name-if-none-match). Configure a compatible gateway/server; SigV4, presigned URLs, multipart upload and provider Object Lock are not implemented.

Production configuration accepts only HTTPS and rejects URL user information, query, fragment, missing trailing slash and invalid/empty bearer credentials. TLS uses platform/native trust roots; no certificate-verification bypass exists. Redirects and environment proxy discovery are disabled. Connect timeout is 5 seconds, request/body timeout 30 seconds; readback is bounded to 8 MiB even without Content-Length. Transport errors are reported without request URLs/tokens. Credentials are absent from acknowledgement/audit records; keep URL paths free of secrets as well.

After a conditional write (including 412 replay), remote-store reads the object back and requires exact equality to the stored DB archive before atomically saving a separate remote acknowledgement and audit event. A remote publication followed by readback/DB failure leaves no acknowledgement; retry reconciles it. Replays preserve the original acknowledgement time. remote-status rereads and compares the complete-file hash; after local snapshot pruning it can use retained prune evidence. DB snapshots and remote objects are never deleted by these commands. Filesystem prune still requires a filesystem vault; a remote acknowledgement alone does not authorize it. Catalogue acknowledgedVaultCount counts filesystem copies only.

storageMeaning is http-readback and worm:false. Tests use a private plaintext loopback fake server only; production configuration cannot enable HTTP. Actual HTTPS service/certificate-chain deployment, provider durability, S3 compatibility, replication, WORM and remote retention remain Pending or planned. A successful readback does not guarantee that a service will retain the object later. No live external service was contacted during tests.

日本語：環境変数のHTTPS URLとBearer tokenで保管先を設定します。専用gatewayはUUID.jsonへのPUT/GET、原子的な `If-None-Match: *`、既存時412、読み戻しの完全一致とread-after-writeを提供する必要があります。S3 clientではなく、SigV4・presigned URL・Object Lockには未対応です。

HTTPSのみ許可し、認証付きURL・query・fragment・redirect・環境proxyを拒否します。tokenは管理者のsecret環境で設定し、監査・ACKへ保存しません。接続5秒、リクエスト30秒、読み戻し8 MiB制限です。保存後の完全一致を確認してからremote ACKと監査を原子的に保存し、再送で復旧します。remote-statusは現在の内容を確認し、ローカルsnapshot削除後も削除証跡のhashを使えます。remote ACKだけで既存filesystem pruneは実行できません。

検証はテスト専用loopback HTTPサーバーで行いました。本番constructorにHTTP許可やTLS検証回避はありません。実HTTPSサービス・証明書、事業者の永続性、S3互換性、WORM、remote保持期限は未検証／未実装です。読み戻し成功は将来の保持を保証しません。

简体中文：管理员通过HTTPS目录URL及Bearer token环境变量配置归档库。兼容gateway须提供UUID.json PUT/GET、原子 `If-None-Match: *`、已存在时412及read-after-write一致性。这不是S3 client，尚不支持SigV4、presigned URL或Object Lock。

仅允许HTTPS，拒绝URL用户凭证、query、fragment、redirect及环境proxy。token通过secret环境配置，不记录到审计或ACK。连接超时5秒、请求30秒、读取上限8 MiB。写入后完整读回匹配，才原子保存remote ACK与审计；重放可恢复中断。remote-status检查当前内容，本地快照删除后可使用删除证据摘要。remote ACK不能单独触发filesystem prune。

测试使用仅测试可用的loopback HTTP模拟服务，生产constructor不允许HTTP或绕过TLS校验。实际HTTPS服务/证书、provider持久性、S3兼容、WORM和远程保留期限仍未验证或实现。成功读回不保证后续保留。

## Remote retention assessment / 外部copyの保持期限確認 / 远程copy保留期限评估

With the same JRIP_AUDIT_VAULT_URL/TOKEN configuration:

```sh
cargo run -p rip-storage --bin jrip-audit -- remote-retention-preview data/jobs.sqlite3 90 - 20
```

English: This read-only command assesses remote acknowledgements older than 1–3650 days for the configured endpoint, using the first successful remote acknowledgement time as the age basis. A replay does not reset age. Results use canonical UUID pagination (nextAfter/hasMore), with limit 1–20 to bound network work. Each read can take up to 30 seconds; a 20-entry page may therefore take several minutes. CLI invocations recompute the cutoff; the response records before. Concurrent changes can affect later pages.

A remoteCopyPruneCandidate requires the original local DB snapshot to remain present, valid and equal to the acknowledged full-file digest/size, plus a current matching remote GET. Missing local snapshots are excluded even if a historical local-prune record exists, preventing an assessment that treats the remaining remote copy as disposable. Local evidence problems, missing/changed remote bytes and transport/size failures return explicit reasons and never positive candidates. This policy does not search other filesystem vaults or remote endpoints for alternate copies.

The result always states dryRun:true and remoteDeletionImplemented:true. It does not send DELETE, change acknowledgement times, prune snapshots or add audit events. Conditional remote deletion is described below. Legal-hold evaluation, server lifecycle configuration, WORM and future-retention guarantees are not implemented. Recheck copy survival and object identity atomically at actual deletion time when implementing a provider-specific delete workflow. Source audit records remain retained.

日本語：`remote-retention-preview` は設定済みendpointの最初のremote ACK時刻を基準に、1〜3650日を過ぎたcopyを確認します。再送で経過日数はリセットしません。UUID cursorで1〜20件ずつ返し、各読み込みは最大30秒のため、1ページに数分かかる場合があります。CLIごとにcutoffを再計算し、結果のbeforeに記録します。

ローカルDB snapshotが現存し、ACKの全体hash・容量と一致し、現在のremote GETも一致する場合だけ候補にします。ローカル削除証跡があってもsnapshotがなければ除外し、残る外部copyを候補にしません。他のvaultやendpointのcopyは探索しません。不一致・不明・通信エラーは理由コード付きで除外します。

常にdry-runで、DELETE・ACK時刻更新・snapshot削除は行いません。条件付き削除は後述します。legal hold、事業者lifecycle、WORMは未実装です。実削除の実装時には、その時点でcopyとobject identityを再確認する必要があります。

简体中文：`remote-retention-preview` 以配置endpoint的首次remote ACK时间为基准，评估超过1–3650天的copy；重放不重置年龄。按UUID分页，每页1–20条，每次读取最多30秒，整页可能耗时数分钟。CLI重新计算cutoff并在before中返回。

仅当原本地DB快照仍存在且与ACK完整摘要/大小一致，并且当前remote GET也一致时列为候选。本地快照已删除时，即使保留删除证据也排除，防止将剩余远程copy列为可丢弃。不会搜索其他vault或endpoint。缺失、改变、通信及大小异常均返回原因且不产生正候选。

始终dry-run，不发送DELETE、不更新ACK时间、不删除快照。条件式删除见下文；legal hold、provider lifecycle及WORM仍未实现。未来实际删除必须重新核对copy及object identity。

## Conditional remote deletion / 外部copyの条件付き削除 / 条件式远程删除

Use the same HTTPS vault configuration. Obtain EXPECTED_FILE_DIGEST from remote-store or remote-retention-preview (this is the complete-file hash, not the payload digest).

```sh
cargo run -p rip-storage --bin jrip-audit -- remote-prune data/jobs.sqlite3 ARCHIVE_UUID 90 sha256:EXPECTED_FILE_DIGEST --apply
cargo run -p rip-storage --bin jrip-audit -- remote-prune-record data/jobs.sqlite3 ARCHIVE_UUID
cargo run -p rip-storage --bin jrip-audit -- remote-prune-reconcile data/jobs.sqlite3 ARCHIVE_UUID
```

English: The gateway must additionally return a strong ETag on GET, synchronously support conditional DELETE with If-Match, return 412 without deleting when it differs, and provide consistent GET/404 observations. [RFC 9110 If-Match](https://www.rfc-editor.org/rfc/rfc9110.html#name-if-match) defines the conditional semantics. Weak/missing ETags are rejected. UUIDs are immutable object identities; do not externally reuse a deleted UUID. S3/versioned delete markers and provider-specific semantics are not covered.

remote-prune requires --apply, a retention age of 1–3650 days, the current stored complete-file digest, a matching live remote copy and a surviving original local snapshot. A short writer transaction rechecks that snapshot and the aged remote acknowledgement, then commits PREPARED intent and an audit event before any DELETE. Per-archive OS locks serialize store/delete/reconciliation across local processes without holding a DB writer lock over network calls. Keep `.jrip-remote-*.lock` files in place while the repository is in use. These locks are not a tested distributed/NFS coordination guarantee.

Exactly one application DELETE attempt is made, with the observed strong ETag in If-Match. HTTP client retries and redirects are disabled. 412 yields REJECTED. Successful 200/204 still requires a subsequent GET/404 to record CONFIRMED_ABSENT. Transport errors, unexpected responses or inconclusive readback yield UNKNOWN. A process crash may leave PREPARED; replay returns that journal instead of resending DELETE. remote-prune-reconcile only performs GET: 404 can settle CONFIRMED_ABSENT, while present objects or unavailable responses stay UNKNOWN. Confirmation means observed absence, not proof of who removed the object.

PREPARED/UNKNOWN journals block local snapshot pruning. Store to an archive/endpoint with any deletion journal is rejected, preventing this application from recreating a deleted object. Terminal deletion records and state-change audit events persist; terminal replays do not duplicate them. Once resolved, local pruning still requires a separately verified filesystem vault copy. remote-retention-preview excludes archives already having a deletion journal.

This is a conservative one-attempt workflow. There is no retry, cancellation or journal-reset command for an unresolved/rejected delete; retain the protected local evidence and investigate the gateway. Indefinite UNKNOWN can require later provider-specific/manual resolution. Strong If-Match support must be enforced by the configured gateway; the client cannot make a noncompliant server safe. Legal holds and backup policy remain administrator responsibilities. Only temporary fake-server objects were deleted in development; actual HTTPS DELETE/provider guarantees remain Pending.

日本語：gatewayはGETでstrong ETagを返し、If-Match付きDELETEを同期的・原子的に処理し、不一致なら412で削除しない必要があります。weak／欠落ETagは拒否します。`remote-prune` は --apply、保持日数、ファイル全体のdigest、現存するローカルsnapshotとremote copyの一致を必須とします。削除前にPREPAREDと監査を永続化します。archive単位OSロックでstore／削除／照会を直列化し、ネットワーク待ちでDB writerを占有しません。稼働中に専用lockファイルを削除しないでください。

DELETEは1回のみで、HTTP clientの自動retryも無効です。412はREJECTED、200/204後のGET/404でCONFIRMED_ABSENT、それ以外はUNKNOWNです。中断時のPREPAREDやUNKNOWNを再送せず、reconcileはGETだけで確認します。存在する／照会不能ならUNKNOWNを保持します。不在確認は削除原因を証明するものではありません。

不明な間はローカルsnapshot削除を拒否します。削除journalがある同一endpoint／IDへのstoreも拒否し、object再作成を防ぎます。確定したjournalと監査は保持します。取消・再試行・journalリセットは未実装で、不明状態は保護した証跡を残して調査が必要です。実HTTPS事業者でのDELETEとNFS分散ロックは未検証です。本番objectは削除せず、開発用模擬objectのみで検証しました。

简体中文：gateway必须提供strong ETag及同步原子If-Match DELETE；不匹配返回412且不删除。拒绝weak或缺失ETag。remote-prune要求 --apply、保留期、完整文件摘要，以及一致的本地快照和远程copy。先原子保存PREPARED与审计，再发送DELETE。每archive OS锁串行化store/删除/核对，网络等待不占用DB writer。运行期间不要移除专用lock文件。

仅发送一次DELETE，并禁用HTTP自动retry。412为REJECTED；200/204后还须GET/404才记为CONFIRMED_ABSENT，其余为UNKNOWN。中断后不重新发送，reconcile仅GET；仍存在或不可查询则保留UNKNOWN。不在场确认不证明删除者身份。

PREPARED/UNKNOWN保护本地快照，任何删除journal都阻止同endpoint/ID再次store以避免重建对象。保留journal及状态审计；取消、retry和journal reset未实现，不明状态需保留证据调查。实际HTTPS provider DELETE及NFS分布式锁仍Pending。开发仅删除临时模拟对象，未删除生产object。
