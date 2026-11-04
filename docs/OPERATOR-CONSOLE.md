# Operator console / 操作画面 / 操作界面

🟢 [Done] Open `/console` on the J-RIP API server (default: `http://127.0.0.1:8080/console`). No frontend build or separate web server is required; Rust embeds the HTML, CSS and JavaScript.

日本語: J-RIP APIサーバーの `/console` を開きます（標準: `http://127.0.0.1:8080/console`）。RustがHTML・CSS・JavaScriptを配信するため、フロントエンドのビルドや別サーバーは不要です。

简体中文: 打开J-RIP API服务器的 `/console`（默认: `http://127.0.0.1:8080/console`）。Rust内嵌HTML、CSS和JavaScript，无需前端构建或单独服务器。

| Operation / 操作 | Behavior / 動作 / 行为 |
| --- | --- |
| Connect / 接続 / 连接 | Existing API token; memory only, cleared on disconnect or reload. / 既存トークンをメモリ内だけに保持、切断・再読み込みで消去。 / 现有令牌仅保存在内存中，断开或刷新后清除。 |
| Submit / 投入 / 提交 | PDF, PS, EPS; up to 32 MiB; format detected from file signature. / 署名で形式を判定、32 MiBまで。 / 根据文件签名检测格式，最大32 MiB。 |
| Settings / 設定 / 设置 | New-job DPI 72–2400, RGB/CMYK/gray, automatic/MuPDF/Ghostscript, priority 0–100. / 新規ジョブの解像度・色空間・エンジン・優先度。 / 新任务的分辨率、色彩空间、引擎及优先级。 |
| Queue / キュー / 队列 | 50 jobs per page, automatic refresh every 3 seconds while visible. / 1画面50件、表示中は3秒ごとに更新。 / 每页50项，可见时每3秒刷新。 |
| Control / 制御 / 控制 | Hold, release, cancel using expected revision; refresh on conflict. / revision付き保留・再開・取消、競合時は再取得。 / 通过预期revision暂停、恢复、取消，冲突时刷新。 |
| Preview / プレビュー / 预览 | Completed RIP page selection and authenticated BMP fetch; blob URLs revoked when replaced/disconnected. / 出力済みページを認証付き取得し、切替・切断時に表示データを解放。 / 认证获取已导出页，切换或断开时释放显示数据。 |

The UI uses the same API scopes as other clients: `jobs:read`, `jobs:submit`, `jobs:control`, `artifacts:read` (or `admin`). Server-side scope and credential expiry checks remain authoritative. Read-only tokens can view jobs; unauthorized mutations show an error. Static assets are public and contain no credentials or document data. The UI does not persist tokens, use cookies, or load third-party scripts/fonts. CSP permits only same-origin scripts/styles/API access and blob previews. Names and API errors are rendered as text.

日本語: 権限は既存APIと共通です。`jobs:read`・`jobs:submit`・`jobs:control`・`artifacts:read`（または `admin`）をサーバーで検査します。読み取り専用トークンでは参照でき、変更は拒否されます。静的画面に認証情報や原稿は含めず、トークンの永続化・Cookie・外部スクリプト／フォントは使用しません。原稿名やエラーは文字列として表示します。

简体中文: 权限与现有API一致，由服务器检查 `jobs:read`、`jobs:submit`、`jobs:control`、`artifacts:read`（或 `admin`）。只读令牌可查看任务，修改操作会被拒绝。静态资源不包含认证信息或原稿，不持久化令牌、不使用Cookie或第三方脚本／字体。原稿名和错误作为文本显示。

## Scope / 対応範囲 / 支持范围

Settings apply to new RIP submissions; they do not edit existing jobs or server configuration. The console controls RIP jobs, not physical printer deliveries. “Raster exported” means TIFF output completed. It does not mean paper was printed. Workers must run separately as before. Source-time page ranges, duplex/media, physical delivery controls and color-stage attempt selection are not exposed in this screen. Unsupported previews display an explicit message. Unknown rendering totals are not presented as fabricated percentages.

日本語: 設定は新規投入に適用し、既存ジョブやサーバー設定は変更しません。対象はRIPジョブで、「ラスター出力済み」はTIFF生成完了を表します。実機印刷の完了ではありません。ワーカーは従来どおり別途起動します。この画面では原稿描画時のページ範囲、両面・用紙、実機配信、色変換attemptの選択は扱いません。表示できないプレビューは明示し、未計測の進捗率は表示しません。

简体中文: 设置仅适用于新提交，不修改现有任务或服务器配置。界面控制RIP任务，“光栅已导出”表示TIFF生成完成，不表示实机打印完成。Worker仍需单独运行。此界面不提供原稿渲染页码范围、双面／介质、实体投递或色彩转换attempt选择。不支持的预览会明确提示，不虚构未测量的进度百分比。

A lost submission response is not automatically retried; refresh the queue before submitting again. UI languages are English, Japanese and Simplified Chinese. At narrow widths the panels stack and only the table scrolls horizontally; page content stays within 16px gutters.

日本語: 投入応答が失われた場合は自動再投入せず、キューを確認する案内を表示します。英語・日本語・简体中文に対応し、狭い画面ではパネルを縦に配置、表だけ横スクロールします。左右16pxの余白を確保します。

简体中文: 提交响应丢失时不会自动重试，会提示先检查队列。支持英语、日语及简体中文。窄屏时面板纵向排列，仅表格横向滚动，左右保留16px边距。

## Verification / 検証 / 验证

```sh
npm ci --prefix tests/ui --ignore-scripts
# Linux CI installs browser system dependencies with --with-deps.
tests/ui/node_modules/.bin/playwright install chromium
cargo test -p jrip-server --test console --test api --test color --locked
cargo test -p jrip-server --test console_browser --locked -- --ignored
```

The browser integration test starts an isolated real HTTP server and temporary repository, uses synthetic documents, and never contacts a printer. It verifies PDF/PS/EPS upload, exact manifest settings, measured state refresh, hold/release/cancel with revisions, conflict recovery, preview page switching, scope denial, disconnected/reloaded state, text-safe filenames, and EN/JA/zh at 375px and 430px. Set `JRIP_CHROMIUM_PATH` to use an existing Chromium binary; set `JRIP_UI_SCREENSHOTS` to a temporary directory for desktop/mobile evidence. CI installs the pinned npm dependencies and Chromium before running ignored integration tests.

⭕️ [Pending] Real printer acceptance remains separate. / 実機受入確認は別途保留。 / 实机验收另行待验证。

### Visual QA / 画面確認 / 视觉验证

The generated design concept and final rendered screenshots were inspected with `view_image`; IAB additionally verified the Japanese login and queue with no browser warnings/errors. Chromium exercised actual HTTP operations. Desktop was checked at 1440×900 and the concept's native 1487×1058 size; mobile at 375px and 430px in all three languages.

| Check | Evidence and intentional implementation choices |
| --- | --- |
| Layout | Preserves left upload panel, upper-right queue, lower-right preview/details; uses a compact 320px sidebar and stacked mobile panels. |
| Typography | System typeface, 13px controls, compact headings; UUIDs wrap inside details, not outside the viewport. |
| Palette | White panels, gray dividers, blue submission/selection, red cancellation and green completed status. |
| Icons | Consistent outline icons for connection, refresh, page navigation, info, hold and cancel. |
| Copy | Above-fold copy reviewed against the concept. Intentional corrections: PDF/PS/EPS only, 32 MiB limit, real engine names, “Raster exported” instead of paper-completion claims; adds supported priority control. No marketing text or invented capabilities. |
| Assets and responsive behavior | All controls are live HTML. Preview is a real authenticated raster, not the concept image. The native-language-dependent file button was replaced by the translated chooser/drop area; table overflow is confined to the table. |

日本語: 設計案と最終画面を画像で照合し、配置・書体・配色・アイコン・文言・狭幅時の改行を確認しました。概念図の未対応形式・容量・仮のエンジン名を実APIに合わせて修正し、対応済みの優先度設定を追加しています。

简体中文: 已对照设计概念和最终截图，检查布局、字体、配色、图标、文案及窄屏换行。将概念图中未支持的格式、容量及虚构引擎名修正为真实API能力，并添加已支持的优先级设置。
