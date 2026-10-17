# Provider recovery and KMS / Provider復旧とKMS / Provider恢复与KMS

## English

🟢 [Done] S3 version-aware UNKNOWN reconciliation and AWS KMS audit archive signing.

New S3 prune journals use `org.jrip.audit-remote-prune-v2` and retain the source
`x-amz-version-id`, digest and size before the single conditional DELETE. Reconciliation
uses signed GETs for the current key and the recorded source version. It never resends
DELETE or deletes a replacement object. Historical v1 journals remain readable.

| Observation | Resolution |
|---|---|
| Pinned source version has the recorded bytes | `CONFIRMED_RETAINED` |
| Pinned source version returns 404 without a delete marker | `CONFIRMED_ABSENT` |
| No pinned version and current GET returns 404 without a marker | `CONFIRMED_ABSENT` (current key only) |
| Denied, timeout, mismatched version/hash, or unpinned delete marker | `UNKNOWN` |

Missing and literal `null` version IDs are unpinned. A marker cannot establish that
older versions disappeared. Terminal journals are historical observations; use
`remote-prune-version-status DATABASE ARCHIVE_UUID` to obtain fresh evidence without
changing the journal, or `remote-prune-reconcile DATABASE ARCHIVE_UUID` to resolve
PREPARED/UNKNOWN. Both require the S3 environment described in
[S3-AUDIT-VAULT.md](S3-AUDIT-VAULT.md). Version reads require appropriate
`GetObjectVersion` permission. UNKNOWN protects the local snapshot. No result proves
physical purge, who deleted an object, Object Lock enforcement or future durability.
See AWS [delete markers](https://docs.aws.amazon.com/AmazonS3/latest/userguide/DeleteMarker.html)
and [version deletion](https://docs.aws.amazon.com/AmazonS3/latest/userguide/DeletingObjectVersions.html).

KMS signing is an explicit audit archive operation. Existing live job receipt signing
continues to use its existing configuration. Configure an HTTPS root endpoint, immutable
single-region UUID key ARN, region, and independently trusted raw 32-byte Ed25519 public key:

```sh
export JRIP_KMS_URL=https://kms.ap-northeast-1.amazonaws.com/
export JRIP_KMS_KEY_ID=arn:aws:kms:ap-northeast-1:123456789012:key/00000000-0000-0000-0000-000000000001
export JRIP_KMS_PUBLIC_KEY=/secure/kms-ed25519-public.raw
export AWS_REGION=ap-northeast-1
# Supply AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY and optional AWS_SESSION_TOKEN securely.
# Unset JRIP_RECEIPT_SIGNING_KEY when explicitly selecting KMS.
cargo run -p rip-storage --bin jrip-audit -- create-kms-signed data/jobs.sqlite3 0 1792198924 100 audit.json
cargo run -p rip-storage --bin jrip-audit -- verify-signature audit.json /secure/kms-ed25519-public.raw
cargo run -p rip-storage --bin jrip-audit -- verify-current data/jobs.sqlite3 audit.json /secure/kms-ed25519-public.raw
```

The key must support `ECC_NIST_EDWARDS25519`, `SIGN_VERIFY`, and
`ED25519_SHA_512`. IAM must permit `GetPublicKey` and `Sign`. Alias substitution,
region mismatch, public-key mismatch, redirects, malformed responses, signature errors
and service failures reject publication without local or unsigned fallback.
The GetPublicKey DER and Sign result are checked against the pinned key and ARN.
[GetPublicKey](https://docs.aws.amazon.com/kms/latest/APIReference/API_GetPublicKey.html)
and [Sign](https://docs.aws.amazon.com/kms/latest/APIReference/API_Sign.html)
define these AWS contracts and the 4096-byte RAW signing limit.

The distinct `org.jrip.audit-archive-kms-ed25519-v1` profile signs
`UTF8(profile) || NUL || SHA256(JCS(archive without signature.value))` with
`MessageType=RAW`, `SigningAlgorithm=ED25519_SHA_512`. This bounded message supports
archives up to the existing 8 MiB limit; it is not the KMS PH/DIGEST algorithm.
Metadata including the immutable KMS ARN is covered by the signature. Independent
public-key verification remains available offline. Current-trust verification also
checks the registered key's revocation. Network calls hold no SQLite writer lock;
publication rechecks revocation and stores the archive and audit atomically.
Private keys never enter the process. Credentials come explicitly from the environment;
SDK discovery and automatic refresh are not provided. HTTPS certificate checking,
response limits, timeouts and disabled automatic retries apply.

Validation uses authenticated loopback S3/KMS fixtures, independent botocore SigV4
vectors, and an independently generated Ed25519 archive fixture. Tests cover lost
DELETE acknowledgement, retained/replaced/missing versions, denial/timeouts, signing
failures, tampering, restart and revocation during signing. No real cloud credentials
or objects are used.

⭕️ [Pending] Real AWS/compatible-provider TLS, IAM, regional Ed25519 availability,
versioning and durability acceptance.
🔴 [Later] Explicit SSE-KMS object encryption configuration, permanent version purge,
and Object Lock policy management. Audit signing does not provide these features.

## 日本語

🟢 [Done] S3のversion指定によるUNKNOWN解決と、AWS KMSによる監査archive署名を実装しました。
削除前のversion ID・digest・サイズをv2 journalへ保存し、現在のkeyと元versionを署名付きGETで確認します。
元versionの内容一致は `CONFIRMED_RETAINED`、元versionの404は `CONFIRMED_ABSENT` です。
403・timeout・内容不一致・version不一致は `UNKNOWN` のままです。未固定／`null` versionのdelete markerは
旧版の消失を証明しません。DELETEは再送せず、置換後のobjectも削除しません。UNKNOWNはlocal snapshotを保護します。
`remote-prune-reconcile` はjournalを解決し、`remote-prune-version-status` は最新の観測だけを返します。
終端journalは過去の観測であり、物理削除や永続保持の証明ではありません。

上記環境変数とCLI例を使用します。KMSは監査archive用の明示操作で、通常のjob receipt署名とは別です。
HTTPS endpoint、region一致の不変UUID key ARN、独立に信頼したraw 32-byte公開鍵が必要です。
`GetPublicKey` と `Sign` 権限、および `ECC_NIST_EDWARDS25519`／`ED25519_SHA_512` 対応鍵が必要です。
新profileはdomainとJCSのSHA-256をRAW署名し、大きなarchiveでもKMSの4096-byte制限内に収めます。
公開鍵・ARN・署名を照合し、失敗時にローカル署名や未署名へ切り替えません。応答待ち中はDB writerを保持せず、
保存時に失効を再確認します。オフライン検証と現在の失効状態を含む検証の両方に対応します。

模擬S3/KMS、独立署名vector、独立archive fixtureで復旧・失敗・改ざん・再起動・署名中の失効を検証します。
⭕️ [Pending] 実AWSのTLS・IAM・regionでの鍵対応・versioning・永続性の受入試験。
🔴 [Later] SSE-KMS暗号化の明示設定、旧versionの恒久削除、Object Lock管理。今回の署名機能には含みません。

## 简体中文

🟢 [Done] 实现S3版本感知的UNKNOWN解析和AWS KMS审计归档签名。
v2 journal在删除前保存源version ID、digest和大小，通过签名GET检查当前key与源版本。
源版本内容一致为 `CONFIRMED_RETAINED`，源版本404为 `CONFIRMED_ABSENT`；拒绝、超时、版本或内容
不一致保持 `UNKNOWN`。未固定或 `null` 版本的delete marker不能证明旧版本已消失。
不重发DELETE，不删除替换对象；UNKNOWN保护本地快照。`remote-prune-reconcile` 更新解析记录，
`remote-prune-version-status` 只读取最新证据。终态记录是历史观察，不证明物理清除或未来持久性。

使用上面的环境变量和CLI。KMS仅用于显式审计归档签名，普通job receipt保持现有配置。
要求HTTPS、region匹配的不可变UUID key ARN、独立可信的raw 32-byte公钥，以及 `GetPublicKey`／`Sign`
权限和 `ECC_NIST_EDWARDS25519`／`ED25519_SHA_512` 密钥。新profile以RAW方式签名domain与JCS的
SHA-256，使大归档也符合4096-byte限制。核对公钥、ARN及签名；失败不回退到本地或无签名。
网络等待不持有DB writer，发布前重新检查撤销，支持离线验证及当前信任验证。

模拟S3/KMS、独立签名向量和归档fixture验证恢复、错误、篡改、重启及签名期间撤销。
⭕️ [Pending] 实际AWS的TLS、IAM、区域密钥支持、版本及持久性验收。
🔴 [Later] 显式SSE-KMS加密配置、永久版本清除与Object Lock管理；审计签名不提供这些能力。

## Diagnostic codes / 診断コード / 诊断代码

| Code | English | 日本語 | 简体中文 |
|---|---|---|---|
| `invalid_kms_configuration` | Check endpoint, region, immutable ARN, credentials and raw public key | endpoint・region・不変ARN・認証情報・raw公開鍵を確認 | 检查endpoint、region、不可变ARN、凭证及raw公钥 |
| `kms_request_failed` | Inspect connectivity; archive was not published | 接続を確認。archiveは未保存 | 检查连接；归档未发布 |
| `kms_response_rejected` | Inspect KMS availability and IAM; do not bypass checks | KMS状態とIAMを確認。検証を回避しない | 检查KMS状态与IAM；勿绕过验证 |
| `kms_public_key_or_signature_mismatch` | Independently confirm key ARN and public pin | ARNと公開鍵pinを独立に再確認 | 独立核对ARN与公钥pin |
| `SOURCE_VERSION_UNAVAILABLE` | Check version-read permission and provider status | version読取権限とprovider状態を確認 | 检查版本读取权限及provider状态 |
| `UNPINNED_RETAINED_VERSIONS` | Keep UNKNOWN; inspect provider version history | UNKNOWNを維持しproviderの履歴を確認 | 保持UNKNOWN并检查provider版本历史 |
