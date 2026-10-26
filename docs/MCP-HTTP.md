# Remote MCP / リモートMCP / 远程MCP

## 日本語

`jrip-mcp-http` はMCP 2025-11-25 Streamable HTTPを単一 `/mcp` エンドポイントで提供する。従来の `cargo run --locked -p jrip-mcp` はstdioのまま利用できる。

```sh
cargo run --locked -p jrip-mcp --bin jrip-mcp-http
```

Print APIと同じ `JRIP_DATA_ROOT`、24文字以上の `JRIP_API_TOKEN`、任意の `JRIP_AUTH_POLICY` を設定する。`JRIP_MCP_LISTEN` は既定 `127.0.0.1:8091`。外部接続にはHTTPSリバースプロキシを使用し、Bearer・MCPヘッダーを維持してSSEをバッファしない。`JRIP_MCP_ALLOWED_ORIGINS` は許可するOriginの完全一致リスト（カンマ区切り）。既定は空で、Originヘッダー付き要求は拒否する。ヘッダーのない認証済みネイティブクライアントは利用できる。自動CORS対応は提供しない。

1. `POST /mcp` にinitialize JSON-RPCを送る。Authorization: Bearer、Content-Type:application/json、Accept:application/json,text/event-streamが必要。応答JSONと `MCP-Session-Id` を保存する。
2. 同じBearer・session ID、`MCP-Protocol-Version:2025-11-25` でnotifications/initializedをPOSTし、202・空bodyを確認する。
3. tools/list、tools/call、resources/read/subscribe/unsubscribeをPOSTする。要求にはJSONで応答し、受理した通知には202で応答する。複数メッセージの配列やJSON-RPC response入力は受け付けない。後続のprotocolヘッダー省略時も保存済みの2025-11-25セッションを使用する。
4. 同じBearer・session ID・protocolヘッダーとAccept:text/event-streamで `GET /mcp` を開く。`message` イベントのdataに標準JSON-RPC更新通知が届く。最初の空data/retryはJSONとして処理しない。
5. `DELETE /mcp` でセッションを終了する（204）。これは投入済み印刷や検査を取り消さない。取消にはそれぞれのMCP操作を明示的に使う。

セッションは作成時のBearerトークンに固定され、別の認証済みトークンでも共有できない。作成から900秒で失効し、プロセス再起動後も404となる。404時は再初期化して購読を再登録する。セッションIDがない後続要求は400、認証失敗は401、scope不足や不正Originは403。作成時のscopeによりツールが絞られ、実行時にもPrint APIが権限を検査する。MCP操作監査はstdioと同じ永続監査を使う。認証設定の反映は再起動時。

接続セッションは同一プロセスで64件まで（超過503）、通知ストリームは1セッション1本（追加GETは409）、購読は32件まで、POST本文は64 KiBまで。通知は約1秒ごとの内部確認で、15秒ごとのkeep-aliveコメントも返す。切断は操作の取消にならず、同じセッションでGETを再開できる。GET再接続時に既存履歴の更新ヒントを再通知するため、クライアントは重複ヒントを許容する。

通知SSEにはevent IDを付けず、Last-Event-IDによる通知再配送は提供しない（指定時400）。イベント本体のsequence/nextAfter/nextCursorをクライアントが保存し、通知後や再接続時に履歴toolから差分取得する。これにより通知の欠落や重複を印刷の再実行として扱わない。通知配送は厳密な一度だけの保証ではない。

remoteセッションはサーバーのfilesystem inboxを持たず、`JRIP_INPUT_ROOT` も適用しない。原稿は認証済みPrint APIで投入してからjob IDを参照する。任意のサーバーパスやURLの取得権限は付与しない。原稿inboxを使うstdioツールとは公開範囲が異なる。OAuth discovery・動的クライアント登録・一般的なOAuthログイン・旧HTTP+SSE transportは未実装。現在は設定済みBearerトークンを渡せるクライアント向けのtransport。

## English

Run `cargo run --locked -p jrip-mcp --bin jrip-mcp-http` for a single `/mcp` Streamable HTTP endpoint. The default executable remains stdio. Use the Print API data root and authorization policy. The listener defaults to 127.0.0.1:8091; configure JRIP_MCP_LISTEN explicitly and use an HTTPS reverse proxy for external clients. Preserve Bearer/MCP headers and disable SSE buffering. Present Origin headers must exactly match the comma-separated JRIP_MCP_ALLOWED_ORIGINS allowlist, which is empty by default. Automatic CORS is not provided.

POST one JSON-RPC message with JSON content type and Accept listing both application/json and text/event-stream. Initialize first, retain MCP-Session-Id, then POST notifications/initialized. Subsequent requests use the same Bearer token, session ID and MCP-Protocol-Version:2025-11-25. Requests return JSON, accepted notifications return empty 202 responses. Open GET with Accept:text/event-stream for standard JSON-RPC notifications in message events; ignore initial empty data/retry. DELETE returns 204 and terminates transport state without cancelling accepted print work.

Sessions are bound to the creating token, expire after 900 seconds, and are lost on restart. Reinitialize and restore subscriptions after 404. Missing session IDs return 400. Unknown tokens return 401; invalid Origins and denied scopes are rejected. Tool access and durable audit reuse existing MCP/Print API policy. Sessions share a 64-process limit, each session has one GET stream (additional GET returns 409), subscriptions are capped at 32 and POST bodies at 64 KiB. Keep-alive comments occur every 15 seconds; notification polling is internal every second. Disconnect releases the stream but leaves the session available until expiry.

Reconnect GET reannounces existing history hints. Notification streams have no event IDs or Last-Event-ID replay (that header returns 400). Persist durable history cursors and fetch deltas through tools after reconnect and notifications; hints may repeat and are not exactly-once delivery. Remote sessions receive no filesystem inbox, even when JRIP_INPUT_ROOT is set. Submit documents through the authenticated Print API and reference job IDs. OAuth discovery/login, dynamic client registration, automatic CORS and the legacy HTTP+SSE transport are not implemented; clients must support configured Bearer tokens.

## 简体中文

运行 `cargo run --locked -p jrip-mcp --bin jrip-mcp-http`，在单一 `/mcp` 提供Streamable HTTP；默认可执行程序仍使用stdio。沿用Print API的数据目录和授权配置。默认监听127.0.0.1:8091，可通过JRIP_MCP_LISTEN配置；外部连接使用HTTPS反向代理，保留Bearer/MCP头并关闭SSE缓冲。Origin头必须完全匹配JRIP_MCP_ALLOWED_ORIGINS逗号分隔列表；默认空列表。不会自动提供CORS。

POST一次一个JSON-RPC消息，使用JSON内容类型，Accept同时列出application/json和text/event-stream。先initialize并保存MCP-Session-Id，再POST notifications/initialized。后续请求携带相同Bearer、session ID和MCP-Protocol-Version:2025-11-25。请求返回JSON，接受的通知返回202空body。GET使用Accept:text/event-stream接收message事件中的标准JSON-RPC通知；忽略初始空data/retry。DELETE返回204，结束传输会话但不取消已接受的打印任务。

会话绑定创建时的token，900秒后过期，重启后丢失；404时重新初始化并注册订阅。后续请求缺少session ID返回400，无效token返回401，不合法Origin或权限不足会拒绝。沿用MCP/Print API权限与持久审计。每进程最多64会话，每会话一个GET流（额外GET返回409），最多32订阅，POST上限64 KiB。每15秒keep-alive，每秒内部检查通知。断开会释放流但在到期前保留会话。

重新GET会再次提示已有历史更新；通知可能重复，不保证严格一次配送。不提供通知event ID或Last-Event-ID重放（该头返回400）。客户端保存持久历史cursor，在通知或重连后通过tool增量读取。远程会话没有服务器文件inbox，也不使用JRIP_INPUT_ROOT；通过认证Print API提交原稿后引用job ID。尚未实现OAuth发现/登录、动态客户端注册、自动CORS及旧HTTP+SSE传输；客户端需支持配置的Bearer token。

Reference / 参照 / 参考: [MCP 2025-11-25 Streamable HTTP transport](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports).

## Identity request rate / identity別要求頻度 / 按身份限制请求频率

🟢 [Done] `org.jrip.mcp-identity-rate-limit-v1`

| Setting / 設定 / 配置 | Default / 既定 / 默认 | Range / 範囲 / 范围 |
| --- | --- | --- |
| `JRIP_MCP_REQUESTS_PER_MINUTE` | 120 | 1–60000 |
| `JRIP_MCP_REQUEST_BURST` | 30 | 1–10000 |

日本語：`jrip-mcp-http` は、認証・Origin・基本ヘッダー検査の後、本文読取とMCP処理の前にidentity単位のtoken bucketを検査します。POST（初期化・通知・tool呼出しを含む）、GET、DELETEが同じbucketを使い、同一identityの別tokenや別sessionでも共有します。他のidentityは独立しています。既定では最初に30要求、以後は毎秒2要求相当を連続補充し、未使用枠は30まで蓄積します。固定の分単位windowではありません。認証済みの不正なMCP要求も枠を消費しますが、認証・Origin・基本ヘッダーで拒否した要求は消費しません。超過要求は追加消費せず、本文やtoolを処理しません。

超過時はHTTP 429、秒単位で切り上げた `Retry-After` とJSON `error` を返します。`code=rate_limited`、`retryable=true`、`retryAfterSeconds`、`requestsPerMinute`、`burst`、上記profileと3言語の `messages` を含みます。指定秒数だけ待ち、同じsessionで再試行してください。他の要求によって空き枠が使われる場合もあるため、待機後の受付保証ではありません。429はJSON-RPC処理前のHTTPエラーであり、tool失敗として再投入する必要はありません。

制限は同一routerを共有するプロセス内で有効で、再起動すると初期枠へ戻ります。状態は設定済み認証identityだけに割り当てます。トークン文字列・session ID・転送元IPによる別枠は作りません。IP偽装ヘッダーは使用しません。分散プロセス間の共有quota、未認証要求のIP制限、Print REST API／IPP／stdioのレート制限は対象外です。既存SSE接続上の通知は要求数に数えず、再接続GETは数えます。長時間SSEの接続枠制限は従来通り有効です。

CLIは環境変数を起動時に読み、非整数・0・範囲外を拒否します。埋め込みAPIの `http::router` は常に既定値を使い、`http::router_with_limits` は検証済みの `rate_limit::RequestLimits` を受け取ります。環境変数の読込は明示的な `RequestLimits::from_env` です。設定の無効化値はありません。TLS配備と短期Bearer tokenは別の未完了項目です。

English: The HTTP MCP gate applies a continuously refilled, process-local token bucket per authenticated identity after authentication/Origin/basic-header validation and before reading request bodies or dispatching MCP. POST (including initialization and notifications), GET and DELETE share the quota across all tokens and sessions for that identity. Different identities are independent. Defaults allow a burst of 30 requests and refill at 120/minute, capped at the burst size. Authenticated malformed MCP requests count; requests rejected by authentication/Origin/basic-header validation do not. Denied requests do not consume further tokens or dispatch tools.

HTTP 429 returns a rounded-up `Retry-After` plus localized JSON `error` with `code=rate_limited`, `retryable=true`, `retryAfterSeconds`, `requestsPerMinute`, `burst`, and the profile above. Wait and retry using the same session. Another concurrent caller may use the replenished capacity first. This is an HTTP admission error before JSON-RPC dispatch, not a failed print submission. Existing SSE notifications do not consume requests; reconnecting GET does. State is allocated only for configured identities and resets on process restart. There is no IP/forwarded-header bypass or per-token/session quota. Limits are not distributed across processes and do not apply to unauthenticated IP traffic, REST, IPP or stdio. Existing connection caps remain independent.

The executable loads the two environment settings at startup and rejects non-integers, zero and out-of-range values. Embedded `http::router` uses defaults; `router_with_limits` accepts validated `RequestLimits`, with explicit `RequestLimits::from_env` for environment loading. There is no disabling value. Short-lived Bearer authentication and TLS deployment remain separate unfinished work.

简体中文：HTTP MCP在认证、Origin及基本头验证后、读取正文或派发MCP前，按认证identity使用持续补充的进程内token bucket。POST（包括初始化及通知）、GET、DELETE共享配额；同一identity的不同token或session不能另建配额，不同identity互相独立。默认突发30次，以每分钟120次连续补充，上限30次。已认证的不合法MCP请求仍消耗配额，认证、Origin及基本头验证拒绝的请求不消耗配额。超限请求不会继续消耗token、读取正文或执行tool。

超限返回HTTP 429，携带向上取整的秒数 `Retry-After` 及三语JSON `error`，包含 `code=rate_limited`、`retryable=true`、`retryAfterSeconds`、`requestsPerMinute`、`burst` 和上述profile。等待后使用同一session重试；并发调用者可能先消耗补充容量，因此等待后不保证立即受理。此错误发生于JSON-RPC派发前，不代表打印投入失败。现有SSE流上的通知不计数，重连GET计数。

状态只分配给配置好的identity，重启后重置；不按token、session或转发IP头建立额外配额。该配额不跨进程共享，也不覆盖未认证流量的IP限制、REST、IPP及stdio。连接数限制继续独立生效。可执行程序启动时读取上表环境变量，拒绝非整数、0及越界值。嵌入式 `http::router` 使用默认值，`router_with_limits` 接受经过验证的 `RequestLimits`，通过 `RequestLimits::from_env` 显式加载环境配置。没有禁用值。短期Bearer认证及TLS部署仍是独立的未完成工作。
