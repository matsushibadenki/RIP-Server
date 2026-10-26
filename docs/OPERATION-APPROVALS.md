# Deletion and persistent-setting approvals

🟢 [Done] `org.jrip.operation-approval-v1`

## 日本語

`JRIP_REQUIRE_DELETE_APPROVAL=1` で終端ジョブの記録削除、`JRIP_REQUIRE_SETTINGS_APPROVAL=1` で次の永続設定APIの承認を必須にできます。省略または `0` は従来のscope検査だけを行い、その他の値は起動エラーになります。

| 操作 | API | 実行scope | 確認する影響 |
| --- | --- | --- | --- |
| job-delete | DELETE `/api/v1/jobs/{id}` | admin | 現在のrevision・状態・原稿hashと記録削除。ファイルや履歴は残る |
| icc-register | POST `/api/v1/color/profiles` | settings:write | 登録前の状態とICCのSHA-256・バイト数 |
| color-policy-create | POST `/api/v1/color/policies` | settings:write | 登録前の状態と定義・理由・JCS digest |
| retention-extend | PUT `/api/v1/jobs/{id}/retention` | settings:write | 現在の保持設定・revisionと新しい期限・理由 |
| receipt-key-revoke | POST `/api/v1/print/receipt-keys/{keyId}/revoke` | admin | 現在の失効状態と新しい失効理由・署名への影響 |

1. `JRIP_AUTH_POLICY` に実行者と承認者を別々のidentityで登録します。承認者は `admin`、実行者は上記の実行scopeが必要です。承認は実行権限の代わりにはなりません。同じidentityの別トークンでも自己承認は拒否します。単一legacy tokenだけの環境では別identityの承認者がいないため、有効にすると対象操作を実行できません。
2. 操作要求を送ると、未承認の場合はHTTP 403、`error.code=approval_required` と `error.approvalRequest.binding` を返します。bindingはサーバーが作成し、主体、操作、HTTP method/path、要求digest、変更前後、3言語の影響を含みます。実行前の内容確認用で、ファイルを削除したり設定値を登録したりしません。認証・scope・現在revisionの不一致などは先に拒否されます。
3. 管理者が影響を確認し、`POST /api/v1/operation-approvals` に `{"binding": <返されたbinding>, "reason": "承認理由"}` を送ります。別identityのadminのみ発行できます。返された `id` は発行から300秒間、1回限り有効です。承認理由は空欄不可、最大1024バイト、制御文字不可です。
4. 実行者が元の要求に `x-jrip-approval-id: <id>` を付けて再実行します。JSONはJCSで正規化するため、空白やキー順の違いを許します。ICCは全バイトのSHA-256を固定します。主体・対象・理由・revision・設定・現在値が変わると承認は適用できません。削除では確認したrevisionを最終削除処理にも渡し、受付後の変更を拒否します。保持期限は既存のrevision検査も継続します。
5. 未使用承認はadminが `POST /api/v1/operation-approvals/{id}/revoke` で失効できます。期限切れ・失効済み・使用済み・不一致・存在しない承認では実行しません。変更後は最新の確認内容で新たに承認を取得してください。

承認はSQLiteの原子的な更新で**操作受付時**に消費します。同時要求では一つだけが受付可能で、失効との競合も同じDBで直列化します。消費が先に確定した操作を後から失効で取り消すことはできません。受付後の入力検証エラー・工程競合・I/O障害・プロセス停止でも承認は戻しません。処理失敗や通信断では状態を読み取り、必要なら新しい承認を取得してください。これは1回限りの受付であり、業務更新やICCファイル公開と同一トランザクションでの成功保証ではありません。

発行・消費・失効の証跡は各更新と同一トランザクションで保存し、監査失敗では更新をロールバックします。既存のAPI監査は拒否された要求も記録します。承認binding・発行者・理由・期限・消費時刻はDBに保持し、再起動しても承認を再利用できません。設定はAPIプロセス起動時に読み込みます。通常のAPI入力検証は引き続き最終ハンドラーで実行します。

`GET /api/v1/approval-policy` で必須化状態を照会できます。MCPは `jrip://approval-policy` を `resources/list` に公開し、`resources/read` で取得できます（jobs:readまたはsettings:write、adminを含む）。`rip_delete_job` と `print_revoke_receipt_key` は任意のUUID `approvalId` をAPIヘッダーへ渡し、未承認時の具体的な影響と回復案をそのまま返します。MCPに承認発行toolは追加しません。発行は別管理者のREST経路です。stdioとHTTP MCPは同じAPI middlewareを使用します。

この機構は別の管理者credentialを承認境界として扱います。人であることの生体認証や外部IdPの対話認証は行いません。CLI、直接のRepository呼出し、DB／ファイルの直接編集はAPIの承認境界に含めません。プリンター設定など起動時ファイル設定はこのAPIでは変更できません。実機印刷には既存の `JRIP_REQUIRE_PRINT_APPROVAL` と出力専用承認を引き続き使用します。

## English

Set `JRIP_REQUIRE_DELETE_APPROVAL=1` and/or `JRIP_REQUIRE_SETTINGS_APPROVAL=1` to guard the five API operations listed above; defaults are off, invalid values fail startup. Configure distinct executor and admin reviewer identities through `JRIP_AUTH_POLICY`. Approval never replaces execution scopes. Another token for the same identity cannot self-approve.

An unapproved operation returns 403 `approval_required` with a server-generated `error.approvalRequest.binding`: identity, operation, method/path, request digest, before/after values and localized impact. Review it, then POST `{binding, reason}` to `/api/v1/operation-approvals` using a separate admin identity. Retry the original request with `x-jrip-approval-id`. Grants last 300 seconds and admit one operation. JSON whitespace/key order may vary; ICC bytes, reason, target, current state and revision are bound exactly. Deletion retains files/history and rechecks the reviewed revision. Admin may revoke an unused grant at `/api/v1/operation-approvals/{id}/revoke`.

SQLite atomically consumes admission and serializes concurrent use/revocation. Consumption persists even if subsequent validation, mutation, I/O or process execution fails; inspect state and obtain a fresh grant if needed. A committed admission cannot be cancelled by later revocation. Admission is not an atomic transaction with the business mutation or filesystem publication. Issue/consume/revoke audits are transactional; audit failure rolls back the corresponding transition. Persistent approvals survive restart without enabling reuse. Existing API audits include rejected requests.

Read `/api/v1/approval-policy` or MCP `jrip://approval-policy` to discover enforcement. MCP deletion and key-revocation tools accept `approvalId` and preserve detailed challenge/recovery responses. Issuance is deliberately a separate administrator REST action. This is a separate credential boundary, not proof of human presence. Direct repository/CLI/DB/filesystem access and startup configuration files are outside this API gate. Physical-print approval remains independently configured.

## 简体中文

通过 `JRIP_REQUIRE_DELETE_APPROVAL=1` 和 `JRIP_REQUIRE_SETTINGS_APPROVAL=1` 启用上表五项API审批；默认关闭，其他值导致启动失败。使用 `JRIP_AUTH_POLICY` 配置不同的执行者和admin审批者identity。审批不能替代执行scope，同一identity的不同token也不能自我审批。

未审批操作返回403 `approval_required`，在 `error.approvalRequest.binding` 中提供服务器生成的主体、操作、method/path、请求digest、变更前后及三语影响说明。管理员核对后，以另一identity向 `/api/v1/operation-approvals` 提交 `{binding, reason}`。执行者携带 `x-jrip-approval-id` 重试原请求。审批有效期300秒，只允许一次受理。JSON空白及键顺序可不同；ICC字节、理由、目标、当前状态及revision须一致。删除只移除任务元数据，保留文件及历史，并再次验证所审批revision。管理员可通过 `/api/v1/operation-approvals/{id}/revoke` 撤销未使用审批。

SQLite原子消耗审批，并串行处理并发使用及撤销。消耗后即使验证、业务修改、I/O或进程失败，也不恢复审批；请查询实际状态并在必要时重新取得审批。后续撤销不能取消已受理的操作。审批受理与业务更新、文件发布不是同一事务。签发、消耗及撤销各自与审计同事务，审计失败则回滚。重启不会恢复已消耗审批。原有API审计仍记录被拒绝请求。

通过 `/api/v1/approval-policy` 或MCP `jrip://approval-policy` 查询启用状态。MCP删除和密钥撤销tool接受 `approvalId`，并保留具体影响和恢复建议；签发通过独立管理员REST操作。此机制验证独立管理凭据，不证明真人在场。直接Repository、CLI、DB、文件访问及启动配置文件不在API审批边界内。实体打印继续使用独立的现有审批设置。

## Verification

Storage tests cover expiry, actor/binding mismatch, restart, one-use races, revoke races and audit rollback. API tests cover all five mutations, scope denial, self-approval denial, exact request matching, canonical JSON, retained source files and persistent approval. MCP tests obtain review challenges and execute approved deletion and Receipt-key revocation through the shared API.
