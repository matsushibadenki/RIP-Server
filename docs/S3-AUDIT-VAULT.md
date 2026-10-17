# S3 audit vault / S3監査保管先 / S3审计归档库

## English

The existing `jrip-audit remote-*` commands can select an S3 SigV4 provider:

```sh
export JRIP_AUDIT_VAULT_PROVIDER=s3
export JRIP_AUDIT_VAULT_URL=https://bucket.s3.ap-northeast-1.amazonaws.com/jrip-audit/
export AWS_REGION=ap-northeast-1
# Supply AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY through your secret environment.
# Supply AWS_SESSION_TOKEN as well when using temporary credentials.
cargo run -p rip-storage --bin jrip-audit -- remote-store data/jobs.sqlite3 ARCHIVE_UUID
cargo run -p rip-storage --bin jrip-audit -- remote-status data/jobs.sqlite3 ARCHIVE_UUID
```

`JRIP_AUDIT_VAULT_PROVIDER=http` (or unset) preserves the Bearer provider with
`JRIP_AUDIT_VAULT_TOKEN`. Unknown provider names fail; no authentication fallback
occurs. S3 uses explicit environment credentials only, not SDK profiles, IMDS,
automatic region discovery or automatic credential refresh. Rotate credentials in
the administrator's environment before the next CLI invocation. Never put secrets
in the endpoint; URLs appear as non-secret storage identities in durable records.

This adapter supports regular regional S3 endpoints and compatible services that
implement the same signed conditional requests. Choose a complete bucket/prefix URL
ending in `/`; virtual-host and path-style addressing are both possible. The
prefix must consist of ASCII unreserved characters and `/`. Percent-escaped,
Unicode, backslash, dot-segment, query and fragment forms are rejected rather than
silently normalized. Keys are always `{prefix}/{archive-uuid}.json`. S3 Express,
MRAP/SigV4a, multipart and presigned URLs are not implemented. Internally generated
versionId GET queries are supported for recovery; configured endpoint queries remain rejected.

Every request signs its HTTP method, literal object path, host, UTC timestamp,
actual SHA-256 payload, session token when present, and request-specific headers.
`If-None-Match`, `If-Match`, Content-Type and GET cache-control are included when
sent. PUT uses `If-None-Match: *`, then requires an exact bounded GET readback before
storing the existing durable ACK/audit. The conditional PUT follows the
[AWS conditional-write contract](https://docs.aws.amazon.com/AmazonS3/latest/userguide/conditional-writes.html).
The authentication construction follows
[AWS SigV4](https://docs.aws.amazon.com/IAM/latest/UserGuide/reference_sigv-create-signed-request.html).

Existing remote retention preview, single conditional DELETE, PREPARED/UNKNOWN
journal, GET-only reconciliation and local-snapshot protection are reused. No
DELETE retry occurs. ETags remain opaque strong preconditions; SHA-256 readback,
not an ETag-as-MD5 assumption, validates bytes. Conditional deletion evaluates
only the current object version; see
[AWS conditional deletes](https://docs.aws.amazon.com/AmazonS3/latest/userguide/conditional-deletes.html).
Version-aware recovery distinguishes source-version retention from absence; see
[PROVIDER-RECOVERY-KMS.md](PROVIDER-RECOVERY-KMS.md). No result proves physical purge.
403/409/transport errors remain conservative failures or UNKNOWN; never treat them
as absence. A missing-key GET may require bucket-list permission to return 404.
Configure least-privilege GetObject/PutObject/DeleteObject and the appropriate
bucket/prefix access with your administrator.

The existing HTTPS-only transport keeps native certificate verification, disabled
redirect/proxy/automatic retries, five-second connect and thirty-second operation
limits, and an 8 MiB readback limit. Authorization and session-token headers are
sensitive; credentials are not written to receipts, ACKs or audit records.
`provider: s3-sigv4` identifies new responses/prune records; historical records are
not rewritten. `storageMeaning: http-readback` and `worm:false` remain accurate.
Version-aware reconciliation and KMS audit signing are described in
[PROVIDER-RECOVERY-KMS.md](PROVIDER-RECOVERY-KMS.md). Object Lock, retention policy
enforcement, explicit SSE-KMS and permanent version purge are not provided.

Validation: fixed GET/PUT/DELETE signatures generated independently using botocore
are committed in `tests/corpus/s3-sigv4-botocore.json`. Normal Rust tests compare
all signature/hash/date bytes without needing botocore installed. A private
loopback fake checks transmitted signatures, session tokens, body hashes and
signed conditional headers, and exercises the same store/readback/retention/delete
failure and replay cases as the Bearer provider. No AWS account, production object
or credentials were used. Real S3/compatible-provider TLS, IAM, versioning,
Object Lock and durability acceptance remains Pending.

## 日本語

既存の `jrip-audit remote-*` に、任意のS3 SigV4 providerを追加しました。
`JRIP_AUDIT_VAULT_PROVIDER=s3`、bucketとprefixを含むHTTPS URL、`AWS_REGION`、
`AWS_ACCESS_KEY_ID`、`AWS_SECRET_ACCESS_KEY` を設定します。一時認証では
`AWS_SESSION_TOKEN` も必要です。未指定または `http` は既存のBearer方式を維持し、
誤設定・認証失敗時に方式を切り替えません。SDK profile・IMDS・自動更新は使用しません。

パスはASCII unreserved文字と `/` に限定し、末尾 `/` が必要です。percent encoding・Unicode・
dot segment・query等は拒否します。ファイル名はarchive UUID固定です。各GET／PUT／DELETEで
本文hash・時刻・host・session token・条件ヘッダーを署名します。条件付き保存後の完全一致読み戻し、
永続ACK、削除journal、UNKNOWN非再送、local保護は既存処理を共有します。

version指定GETによるUNKNOWN解決とKMS監査署名は [追加仕様](PROVIDER-RECOVERY-KMS.md) を参照してください。
不在の観測は物理容量回収を証明しません。403・409・通信障害を不在と扱いません。
Object Lock管理・明示SSE-KMS設定・永久version削除は後続項目です。

botocoreで独立生成した署名vectorと、実際のHTTP署名を検査する模擬S3で保存・再送・保持期限・
削除失敗・復旧を検証します。本番設定はHTTPS必須で、証明書検証を回避しません。
実S3／互換providerのTLS・IAM・versioning・Object Lock・永続性はPendingです。

## 简体中文

现有 `jrip-audit remote-*` 新增可选S3 SigV4 provider。设置 `JRIP_AUDIT_VAULT_PROVIDER=s3`、
包含bucket与prefix且以 `/` 结尾的HTTPS URL、`AWS_REGION`、`AWS_ACCESS_KEY_ID` 与
`AWS_SECRET_ACCESS_KEY`；临时凭证还需 `AWS_SESSION_TOKEN`。未设置或选择 `http` 时保留原Bearer
方式，错误时不自动切换。未实现SDK profile、IMDS或凭证自动刷新。

路径仅接受ASCII unreserved字符与 `/`，拒绝percent编码、Unicode、dot segment及query。
每个GET/PUT/DELETE签名覆盖正文hash、时间、host、session token及条件头。复用条件写入后的完整
读回、持久ACK、删除journal、UNKNOWN不重发与local保护。403、409及通信错误不视为对象不存在。

version指定GET恢复和KMS审计签名见 [追加规格](PROVIDER-RECOVERY-KMS.md)。
观察到缺失不证明物理空间回收。Object Lock管理、显式SSE-KMS和永久版本清除仍为后续工作。

以独立botocore签名向量及检查真实HTTP签名的模拟S3验证保存、重放、保留期限、删除失败和恢复。
生产配置须使用HTTPS且不绕过证书验证；实际S3/provider的TLS、IAM、版本、Object Lock及持久性验收为Pending。
