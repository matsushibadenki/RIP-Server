# Output context / 出力操作のコンテキスト / 输出操作上下文

🟢 [Done] 実機配信画面で認証主体・承認条件・色変換結果を参照するための読取APIとMCP一覧を実装しました。
🟢 [Done] [操作画面へ接続](PHYSICAL-OUTPUT-CONSOLE.md)。用紙・部数・両面・色成果物の選択、照合、明示投入、状態・取消に対応します。

## 認証コンテキスト / Credential context / 认证上下文

`GET /api/v1/session` はBearerに対応する現在の `identity`、`scopes`、`expiresAt`、`credentialSource`、`observedAt`、`printApprovalRequired` を返します。profileは `org.jrip.auth-context-v1`。設定済みcredentialは `configured`、既存ローカルトークンは `legacy-local` と区別します。期限はUTC epoch秒、期限なしはnullです。未認証・期限切れ・有効期間前のcredentialは401。設定済みポリシーがあればローカルトークンへの迂回はできません。

読取専用のsnapshotです。トークンや他のcredentialの情報は返しません。`Cache-Control: no-store`、`Vary: Authorization` を付け、操作権限は各APIで毎回検査します。この取得に成功しても、`jobs:read` など別のscopeは付与されません。

This authenticated snapshot returns the caller's identity, effective scopes, expiry, credential source and print-approval policy. It excludes tokens and other credentials, is non-cacheable and rechecks validity on each call. Obtaining it grants no action scope; operation endpoints remain authoritative.

认证快照返回调用者身份、有效scope、到期时间、认证来源和打印审批策略，不包含令牌或其他认证记录。响应不可缓存，每次检查有效期。获取上下文不会增加操作权限，实际API仍重新检查。

## 色変換結果一覧 / Color summaries / 色彩转换摘要

```text
GET /api/v1/jobs/{jobId}/color-attempts?limit=50&state=COMPLETED
GET /api/v1/jobs/{jobId}/color-attempts?limit=50&state=COMPLETED&before={nextBefore}
```

`jobs:read` が必要です。省略時は全状態、`limit` は既定50、1〜100。`state` は `QUEUED`／`RUNNING`／`COMPLETED`／`FAILED`／`CANCELLED`。`before` は直前の応答の `nextBefore` をそのまま渡します。未知のquery属性・型・範囲・状態・cursorは400 `invalid_color_query`、存在しない原稿は404です。

応答には `attempts`、`hasMore`、`nextBefore` を含めます。新しい順（作成時刻・UUIDの降順）のカーソル取得なので、新規作成により既存結果のページ位置がずれません。状態変更や新規作成を含む一覧全体の固定snapshotではありません。各要素はID・原稿ID・状態・revision・作成／更新時刻・完了／原稿ページ数・reason code・DPI・入力／出力色・ICC digestの概要です。`ticket`、原稿TIFFの一覧、ICCバイナリー、ページ別レポートや変換成果物は含めません。詳細は既存の `GET /api/v1/color/attempts/{id}` で取得します。空結果の `attempts` は空配列です。

応答は `no-store`、`Vary: Authorization`。一覧の選択後も配送preview、profile照合、成果物hash、承認、投入直前の機器能力検査を行います。原稿別・状態別の作成順indexを使い、一覧取得を最大100件に制限します。カーソルと状態フィルター、一覧は再起動後も利用できます。

MCP tool: `color_list_attempts`。引数は必須の `jobId` と任意の `limit`、`before`、`state`。読取専用で `jobs:read` が必要です。不正なカーソルやquery注入となる文字列はAPI呼出し前に拒否します。

The source-job collection provides at most 100 lightweight summaries per call, newest first, with a creation-order cursor and optional state filter. New insertions do not shift previously consumed rows. It is a live collection, not a frozen snapshot across state changes. Large artifacts, tickets and per-page reports stay in existing detail resources. Invalid queries have a stable localized error; unknown parents return 404. REST and the read-only `color_list_attempts` MCP tool require jobs:read. Selecting a summary does not bypass delivery verification or approval.

原稿集合每次最多返回100条轻量摘要，按创建时间与UUID降序排列，支持游标和状态筛选。新增记录不会移动已读取的记录位置；状态变化时不保证整个集合为固定快照。大产物、ticket及逐页报告留在详情资源。错误查询返回稳定的本地化错误，原稿不存在返回404。REST及只读MCP工具 `color_list_attempts` 均要求jobs:read，选择摘要不会绕过投递验证或审批。

## 検証 / Verification / 验证

```sh
cargo test -p jrip-server --test api --test color --locked
cargo test -p jrip-mcp --test protocol --locked
```

認証期限・未認証・他のscopeを付与しないこと・情報非漏洩、原稿分離、同時追加後のページ送り、状態別取得、空結果、件数上限、不正query、再起動、MCPの入力検査と権限制御を検証します。

⭕️ [Pending] 実機の用紙・排紙・取消受入確認は、機器を使える環境で別途実施します。
