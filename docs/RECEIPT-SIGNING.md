# Receipt signing (experimental)

## English

`org.jrip.receipt-ed25519-v2` optionally signs new acceptance and raster-result receipts in the same SQLite transaction as the job change. Existing receipts remain unchanged. Without configuration, new receipts retain `signed:false`. This does not certify physical output or grant authorization.

Generate a PKCS#8 v2 private key and raw 32-byte public key:

```sh
cargo run -p rip-storage --bin jrip-receipt -- generate-key /secure/receipt.pk8 /secure/receipt-public.bin
export JRIP_RECEIPT_SIGNING_KEY=/secure/receipt.pk8
```

Set the same key path for the API, MCP and every worker process that opens the repository; restart them after rotation. Invalid/unreadable keys stop repository startup. Keep private keys outside the repository and restrict access. Unix generation uses mode 0600 and never overwrites existing files. Distribute public keys through an independently trusted channel and retain previous public keys for historical receipts. Key rotation can produce acceptance and result receipts signed by different keys.

Download the receipt through its existing authenticated API, then verify offline:

```sh
cargo run -p rip-storage --bin jrip-receipt -- verify /secure/receipt-public.bin receipt.json
```

Legacy v1 signatures remain supported. Their signature object contains `profile`, `algorithm:Ed25519`, `encoding:hex`, `digestProfile:org.jrip.serde-json-v1`, `publicKey` (lowercase hex), `keyId` (`sha256:` plus public-key SHA-256) and `value` (lowercase signature hex). The message is UTF-8 `org.jrip.receipt-ed25519-v1`, one zero byte, then the compact serde_json serialization of the entire receipt with only `signature.value` removed. Object keys are sorted lexicographically by the serde_json default map; arrays retain order. This is a Rust serde_json profile, not RFC 8785/JCS: other runtimes must reproduce its number/string serialization exactly. Metadata, proof and result are all covered. Verification requires a caller-supplied trusted public key; an embedded key alone provides no trust. Unknown profiles, altered content and wrong keys fail verification. The CLI accepts receipts up to 1 MiB.

Implementation uses [ring's Ed25519 API](https://docs.rs/ring/0.17.14/ring/signature/struct.Ed25519KeyPair.html). KMS integration, broader JCS input support remain planned. New v2 signatures and delivery-result receipts are described below.

## 日本語

新しい受付・ラスター結果Receiptを、ジョブ更新と同じトランザクションで任意のEd25519署名付きで保存します。既存Receiptは変更しません。設定なしでは無署名のままです。署名は実機出力の保証や操作の承認を意味しません。

上記コマンドで秘密鍵と公開鍵を生成し、API・MCP・全ワーカーに同じ `JRIP_RECEIPT_SIGNING_KEY` を設定してください。不正な鍵では起動を停止します。秘密鍵はリポジトリ外に保管し、アクセスを制限します。鍵交換後は各プロセスを再起動し、過去の検証用公開鍵を保持します。受付と結果の署名鍵が異なる場合があります。

検証は認証付きAPIから取得したReceiptと、別経路で信頼を確認した公開鍵を使って行います。新しい署名v2は下記のJCS規則、過去のv1はserde_json規則で検証します。どちらも署名対象はprofile名・ゼロバイト・signature.valueだけを除いたReceipt全体です。埋め込み公開鍵だけでは信頼できません。改ざん・鍵違い・未知profileは拒否します。KMSとJCSの全数値範囲対応は未実装です。

## 简体中文

可选Ed25519签名在与任务变更相同的事务中保存新的受理回执和栅格结果回执。已有回执保持不变；未配置时仍为无签名。签名不代表实体输出保证或操作授权。

使用上述命令生成密钥，将同一 `JRIP_RECEIPT_SIGNING_KEY` 配置到API、MCP和所有worker。无效密钥阻止启动。私钥保存在仓库之外并限制访问。轮换后重启各进程，并保留历史公钥；受理和结果回执可能由不同密钥签名。

通过已有认证API获取回执，使用独立可信渠道取得的公钥验证。新签名v2按下方JCS规则验证，历史v1仍使用serde_json规则。两者均覆盖profile名、零字节及仅删除signature.value后的整个回执。内嵌公钥本身不建立信任。篡改、错误密钥及未知profile验证失败。KMS及JCS全部数字范围仍未实现。

## Public-key discovery / 公開鍵の取得 / 公钥发现

Authenticated `GET /api/v1/print/receipt-keys?limit=50&after=sha256:…` requires `jobs:read`. Omit `after` for the first page; follow `nextAfter` while `hasMore` is true. Limit is 1–100. Each configured signing key is registered as public material in SQLite at startup and retained across rotation. `activeForThisProcess` and `signingEnabledForThisProcess` describe the responding process only: separate workers can have different configuration. Revocation status is included; see the revocation procedure below. Discovery does not establish trust (`trustEstablished:false`); compare `keyId` with independently pinned keys before verification. The registry starts recording keys with this feature; keys used only before this upgrade remain in their historical receipts and are not automatically imported.

MCP exposes `print_get_receipt_keys` (`after`, `limit` optional), `print_get_receipt` (`jobId`) and `print_get_result_receipt` (`jobId`). All require `jobs:read` and work in stdio and Streamable HTTP modes. Receipt reads preserve stored signatures even when current signing is disabled. Missing receipts return an error; they do not manufacture unsigned evidence.

日本語：公開鍵一覧APIと上記3つのMCPツールは `jobs:read` が必要です。カーソルでページを取得でき、起動時に登録した公開鍵は鍵交換後もSQLiteに保持します。有効な鍵の表示は応答したプロセスの設定のみを示します。取得した鍵は別経路で固定したkeyIdと照合してください。失効管理は下記の手順に対応しています。この機能導入前だけに使われた鍵は自動登録しません。保存済みReceiptは署名設定を無効にしても変更しません。

简体中文：公钥列表API及上述3个MCP工具均需要 `jobs:read`。支持游标分页，启动时注册的公钥在轮换后仍保存在SQLite。当前密钥状态仅描述响应进程。获取的keyId必须与独立固定的可信密钥比较；撤销管理见下方说明。仅在升级前使用的密钥不会自动导入。禁用签名后，已有回执及签名保持不变。

## Key revocation / 鍵の失効 / 密钥撤销

`POST /api/v1/print/receipt-keys/{keyId}/revoke` accepts `{"reason":"compromised"}`; allowed reasons are `compromised`, `retired`, `operator-request`. It requires `admin`. MCP `print_revoke_receipt_key` takes `keyId` and `reason`, requires the same authority and is annotated as a destructive operation. Revocation is permanent; rotate to a new key to resume signing. Repeating the same reason preserves the original actor/time and returns `replayed:true`; a different reason conflicts. Unknown keys return 404. The revocation and successful operator audit record are stored atomically.

Key catalogs retain the public key and add `revoked`, `revokedAt`, `revocationReason`, `revokedBy` and `configuredForThisProcess`. Revoked keys are never `activeForThisProcess`; `signingEnabledForThisProcess` becomes false and `signingBlockedReason` is `KEY_REVOKED`. Upgrade all writer processes before relying on this mechanism. Writers check SQLite under the same transaction as receipt publication, so already-running upgraded workers see revocation without restart. New acceptance with a configured revoked key rolls back and returns HTTP 503 `receipt_signing_key_revoked`. A terminal job still records its result atomically, without a signature and with `proof.signingBlocked.code:KEY_REVOKED` and the keyId. It never reports that unsigned result as signed. Existing receipts and signatures are unchanged.

`verify` checks cryptographic validity only and explicitly reports that revocation was not checked. For current trust against an existing local repository, use:

```sh
cargo run -p rip-storage --bin jrip-receipt -- verify-current /data/jobs.sqlite3 /secure/receipt-public.bin receipt.json
```

`verify-current` and `Repository::verify_receipt_current` require the caller's independently trusted public key, a registered non-revoked key and a valid signature. They reject all receipts from a revoked key, including earlier receipts: issuedAt is not a trusted timestamp, so there is no automatic historical-validity exception. Preserve earlier receipts for forensic examination. Remote clients must obtain current key status over their trusted authenticated connection; discovery is still not a trust anchor. Offline verification without fresh status cannot detect subsequent revocation. Revocation cannot be undone. Signed revocation catalogs, trusted timestamps and KMS integration remain planned.

日本語：管理者APIとMCPで鍵を恒久失効できます。reasonは上記3種類です。同じ理由で再送すると元の時刻・主体を保持し、違う理由は競合になります。失効と成功監査記録は同じトランザクションに保存します。公開鍵・過去のReceiptは削除しません。全writerを更新してから使用してください。更新済みの実行中workerも保存時にDBを確認し、失効鍵の新しい受付は503で拒否します。終端結果は処理の事実を失わないよう無署名で保存し、署名不能の理由と鍵を明示します。

通常の `verify` は暗号学的検証のみです。`verify-current` は独立に信頼した公開鍵とローカルDBの現在状態で検証し、失効鍵の過去のReceiptも拒否します。issuedAtには信頼できる時刻証明がないためです。遠隔AIは認証付き接続で最新の失効状態も取得してください。鍵の失効解除、失効一覧自体の署名、信頼時刻、KMSは未実装です。

简体中文：管理员API及MCP可永久撤销密钥。reason仅允许上述3种；同理由重试保留原时间和主体，不同理由返回冲突。撤销与成功审计记录原子保存。保留公钥及历史回执；启用前升级全部writer。已升级且正在运行的worker在保存时查询DB，拒绝使用已撤销密钥的新受理并返回503。终态结果仍保存，但不签名，并明确记录无法签名的原因和keyId。

普通 `verify` 仅检查密码学签名。`verify-current` 使用独立可信公钥及本地DB当前状态，拒绝已撤销密钥的全部历史回执，因为issuedAt没有可信时间证明。远程AI也须通过可信认证连接获取最新撤销状态。尚未实现解除撤销、撤销目录签名、可信时间及KMS。

## Delivery result receipts / 実機配送の結果Receipt / 实体投递结果回执

New terminal delivery transitions publish `org.jrip.delivery-result-receipt-v1` receipts atomically with their state and event. Authenticated `GET /api/v1/deliveries/{id}/result-receipt` and MCP `print_get_delivery_result_receipt {deliveryId}` require `jobs:read`. QUEUED, SENDING, ACCEPTED and UNKNOWN have no terminal receipt (404); they never imply successful physical completion. Historical terminal rows from before this feature are not backfilled.

The receipt preserves delivery ID/revision/state, source job, device job ID, reason, immutable delivery ticket, optional physical contract, and the hash of the stored raster-result receipt when available. requestHash covers `{deliveryId,ticket,physicalContract}`; resultHash covers `proof.result`, using `org.jrip.serde-json-v1`. Missing legacy contracts or raster receipts remain null. COMPLETED becomes status `succeeded` and completionMeaning `device-confirmed`; CANCELLED becomes `cancelled`, ABORTED/REJECTED become `failed`, with completionMeaning `delivery-terminal`. A completed result requires a positive remote job ID. `confirmationSource` distinguishes remote-job reconciliation, device-state observation and other delivery-state records.

Device-confirmed means the provider recorded a printer-reported terminal result through its existing IPP observation/reconciliation path. It is not a printer's cryptographic attestation and does not establish paper appearance: `printedAppearanceVerified:false` is explicit. The existing administrator reconciliation endpoint verifies the remote job before changing UNKNOWN; this receipt does not add any printing/retry operation. Signing, revocation checks and explicit unsigned `KEY_REVOKED` fallback follow the raster-result rules above. Reads never re-sign old evidence. Fake IPP acceptance tests exercise this behavior; actual printer quality remains pending.

日本語：新しい配送終端遷移と同じトランザクションで結果Receiptを保存します。取得API/MCPはjobs:read限定です。待機・送信中・受付済み・UNKNOWNには終端Receiptを発行しません。導入前の過去終端記録も後付けで作り直しません。

配送ID・revision・原稿ジョブ・実機job-id・理由・配送契約・任意の実機契約・既存ラスター結果Receiptのhashを結び付けます。COMPLETEDだけがdevice-confirmedであり、取消・中止・拒否は成功扱いしません。確認経路も記録します。実機の暗号学的な証明や紙面品質の保証ではなく、外観未検証を明示します。署名失効時も終端事実を無署名で保存します。偽IPPで検証し、実機品質はPendingのままです。

简体中文：新的投递终态与结果回执在同一事务中保存。API/MCP获取需要jobs:read。排队、发送中、已受理及UNKNOWN均不产生终态回执；不补造升级前的历史终态回执。

回执关联投递ID、revision、源任务、设备job-id、原因、投递契约、可选实体契约及已有栅格结果回执hash。仅COMPLETED声明device-confirmed，取消、中止及拒绝均不是成功。记录确认来源。这不是设备密码学证明或纸面质量保证，并明确外观未经验证。密钥撤销时终态事实仍以无签名形式保存。通过伪IPP验证，真实打印质量仍为Pending。


## JCS signature v2 / JCS署名v2 / JCS签名v2

New signatures now use org.jrip.receipt-ed25519-v2 and org.jrip.jcs-safe-v1. Legacy v1 signatures remain immutable and verifiable with their original algorithm. See [CANONICAL-DIGEST.md](CANONICAL-DIGEST.md) for exact canonicalization, bounds, independent Node checks and portable ticket hashes. Existing proof request/result hashes keep their original digest profile.

日本語：新署名はv2とJCSを使います。過去のv1署名は元の規則で検証でき、変更しません。詳細は上記共通digest仕様を参照してください。

简体中文：新签名使用v2及JCS；历史v1保持不变并按原规则验证。详细规则见公共digest文档。
