# Expiring credentials / 期限付き認証 / 有效期凭据

🟢 [Done] Configured Bearer credentials with a bounded validity window.

## 日本語

`JRIP_AUTH_POLICY` の各credentialに任意の `validity` を指定できます。トークン文字列は既存と同じ24文字以上のランダムなsecretを外部で生成し、identityとscopeを明示してください。

```json
[
  {
    "token": "<random-secret-at-least-24-characters>",
    "identity": "print-agent",
    "scopes": ["jobs:read", "jobs:submit"],
    "validity": {"notBefore": 1700000000, "expiresAt": 1700000900}
  }
]
```

上記時刻は形式の例で、既に期限切れです。実際には現在時刻に合わせたUTCのUNIX秒を設定します。`notBefore` と `expiresAt` は両方必須で、非負の開始時刻から1〜900秒の期間だけを許します。時刻が片方だけ、文字列・小数・真偽値、終了が開始以前、900秒超、未知の属性は起動時に拒否します。有効期間内の条件は `notBefore <= now < expiresAt` で、期限と同時に認証できなくなります。設定済みの期限切れ・未来のcredentialは読み込めますが、有効時刻外では使用できません。ホストのUTC時刻を使います。

既存設定との互換性のため、`validity` の省略またはnullは無期限です。設定ファイルを読み込んだ場合、期限切れ・未開始のtokenを `JRIP_API_TOKEN` のlegacy adminへフォールバックさせません。scopeは有効期間と独立して検査します。

`jrip-mcp-http` では `JRIP_MCP_REQUIRE_EXPIRING_TOKENS=1` を指定すると、有効な `validity` を持つcredentialだけを受け付けます。無期限tokenやlegacy tokenはHTTP 401 `expiring_token_required`、期限切れ・未開始・不明tokenは401 `unauthorized` になります。省略または0は従来の無期限tokenも許し、その他の値は起動エラーです。この設定はリモートMCPだけに適用し、REST管理APIの無期限credentialは従来通り使えます。期限が指定されたcredential自体の有効期間はREST／MCP HTTP共通で常に検査します。

認証はPOST・GET・DELETEを含む各MCP HTTP要求で再確認し、sessionの待機ロック取得後にも再確認します。通知SSEは配信ループで再確認し、期限後は保留中の通知を送らず接続を閉じます。RESTのInspection／PipelineイベントSSEも各イベントの送出前に再確認し、期限切れでは `stream-error` / `AUTHORIZATION_EXPIRED` を返して終了します。イベント配信はポーリングされる際に検査され、待機中は通常約1秒以内に終了します。

同じidentityに異なるtokenの連続した有効期間を設定してローテーションできます。MCP sessionは元のtokenそのものに固定されるため、新しいtokenでは再initializeし、保存済みのjob-id・履歴cursorから再開してください。同じidentityのscopeと原稿所有権・承認条件は既存ルールで引き続き検査します。レート制限はtokenが変わってもidentityで共有します。

期限切れは投入済みジョブやInspectionを取り消しません。有効時刻に認可済みの処理・ファイルダウンロードを途中で取り消す保証もありません。新しい要求やイベント配信の認証を拒否します。認可設定はプロセス起動時のsnapshotで、ファイルを書き換えただけでは反映されません。

埋め込み利用では `Credential.validity` と `CredentialValidity`、`http::HttpOptions.require_expiring_tokens` を使います。`http::router`／`router_with_limits` は従来通り無期限tokenも許し、`router_with_options` で必須化できます。CLIは `HttpOptions::from_env` を使います。直接Repository／DB／ファイル操作はこのAPI認証の対象外です。

🔴 [Later] 自動発行・更新・即時失効の仕組みと外部IdP／OAuth連携。今回の実装は設定済みopaque Bearerの時間制限です。TLS配備・証明書の検証も別項目です。

## English

An optional `validity` object on each `JRIP_AUTH_POLICY` credential requires integer UTC epoch seconds `notBefore` and `expiresAt`. The interval must start at a nonnegative timestamp and last 1–900 seconds. Partial, invalid, reversed, overlong or unknown-field windows fail startup. A credential is active exactly when `notBefore <= now < expiresAt`; future and expired entries may remain configured. The sample timestamps above are expired examples. Secrets are generated externally under the existing token rules.

Omitted/null `validity` preserves unbounded legacy credentials. Configured expired/future tokens never fall back to the legacy admin token. Scope checks remain independent. Set `JRIP_MCP_REQUIRE_EXPIRING_TOKENS=1` to require bounded credentials on HTTP MCP: unbounded credentials return 401 `expiring_token_required`, while expired/future/unknown credentials return 401 `unauthorized`. Defaults allow unbounded tokens; values other than 0/1 fail startup. Expiring credentials are checked on every authenticated REST/MCP HTTP request even without the flag.

MCP rechecks authentication after session-lock waits and in notification loops. Expiration closes notification SSE before buffered updates can be emitted. Inspection/Pipeline REST SSE rechecks before each event and ends with `stream-error` / `AUTHORIZATION_EXPIRED`. Checks run as streams are polled, normally within one second while waiting. Expiration does not cancel accepted jobs or guarantee interruption of already authorized operations/downloads.

Configure distinct tokens under the same identity for rotation. New tokens must initialize new MCP sessions; use durable job IDs and history cursors to reconnect. Ownership, scopes and approvals retain their existing semantics, and request quotas remain identity-based. Policy files load at process startup, not dynamically. This is bounded configured opaque Bearer authentication, not an issuance, refresh, immediate-revocation or OAuth service. These remain Later while everyday RIP usability takes priority, alongside TLS deployment verification. Embedded routers support `HttpOptions` and `router_with_options`; existing router helpers keep compatibility defaults. Direct repository/CLI/database/filesystem access is outside the API authentication boundary.

## 简体中文

`JRIP_AUTH_POLICY` 中每个credential可选配置 `validity`，必须同时提供整数UTC UNIX秒 `notBefore` 与 `expiresAt`。开始时间非负，时长为1至900秒。缺少字段、错误类型、倒置、超长或未知字段均导致启动失败。仅在 `notBefore <= now < expiresAt` 时有效；未来或过期条目可保留在配置中。上方示例时间已经过期，请替换为实际时间。secret按原规则在外部生成。

省略或null `validity` 保留无期限兼容行为。已配置但过期或尚未生效的token不会回退为legacy admin。scope继续独立验证。设置 `JRIP_MCP_REQUIRE_EXPIRING_TOKENS=1` 后，HTTP MCP只接受有有效期的credential；无期限返回401 `expiring_token_required`，过期、未生效或未知token返回401 `unauthorized`。默认允许无期限，0/1以外配置导致启动失败。已指定的有效期在REST和MCP HTTP每次请求时始终验证。

MCP在等待session锁后及通知循环中再次认证。过期后关闭通知SSE，不发送缓冲中的更新。Inspection／Pipeline REST SSE在每个事件前验证，过期发送 `stream-error` / `AUTHORIZATION_EXPIRED` 并结束。验证在流被读取时执行，等待期间通常约1秒内结束。过期不取消已接受的任务，也不保证中断已授权操作或文件下载。

可为同一identity配置不同token及连续有效期进行轮换。新token须重新初始化MCP session，再使用持久job-id及历史cursor继续跟踪。所有权、scope及审批规则保持有效，频率配额继续按identity共享。配置只在进程启动时加载，不自动热更新。此实现为配置好的opaque Bearer添加时限；自动签发、更新、即时撤销及OAuth／外部IdP集成按一般RIP可用性优先的方针暂列Later，TLS部署验证也是独立工作。嵌入式使用 `HttpOptions`／`router_with_options`；原router辅助函数保留兼容默认值。直接Repository、CLI、DB及文件访问不属于此API认证边界。
