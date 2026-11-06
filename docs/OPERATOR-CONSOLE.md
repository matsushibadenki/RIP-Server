# Operator console / 操作画面 / 操作界面

🟢 [Done] Open `/console` on the J-RIP API server (default: `http://127.0.0.1:8080/console`). No frontend build or separate web server is required; Rust embeds the HTML, CSS and JavaScript.

日本語: J-RIP APIサーバーの `/console` を開きます（標準: `http://127.0.0.1:8080/console`）。RustがHTML・CSS・JavaScriptを配信するため、フロントエンドのビルドや別サーバーは不要です。

简体中文: 打开J-RIP API服务器的 `/console`（默认: `http://127.0.0.1:8080/console`）。Rust内嵌HTML、CSS和JavaScript，无需前端构建或单独服务器。

| Operation / 操作 | Behavior / 動作 / 行为 |
| --- | --- |
| Connect / 接続 / 连接 | Existing API token; memory only, cleared on disconnect or reload. / 既存トークンをメモリ内だけに保持、切断・再読み込みで消去。 / 现有令牌仅保存在内存中，断开或刷新后清除。 |
| Submit / 投入 / 提交 | PDF, PS, EPS; up to 32 MiB; format detected from file signature. / 署名で形式を判定、32 MiBまで。 / 根据文件签名检测格式，最大32 MiB。 |
| Pages / ページ範囲 / 页码范围 | PDF source ranges such as `2-3,5`; blank means all. / PDFの原稿ページを指定、空欄は全ページ。 / 指定PDF原稿页，留空表示全部。 |
| Settings / 設定 / 设置 | New-job DPI 72–2400, RGB/CMYK/gray, automatic/MuPDF/Ghostscript, priority 0–100. / 新規ジョブの解像度・色空間・エンジン・優先度。 / 新任务的分辨率、色彩空间、引擎及优先级。 |
| Queue / キュー / 队列 | 50 jobs per page, automatic refresh every 3 seconds while visible. / 1画面50件、表示中は3秒ごとに更新。 / 每页50项，可见时每3秒刷新。 |
| Control / 制御 / 控制 | Hold, release, cancel using expected revision; refresh on conflict. / revision付き保留・再開・取消、競合時は再取得。 / 通过预期revision暂停、恢复、取消，冲突时刷新。 |
| Preview / プレビュー / 预览 | Completed RIP page selection and authenticated BMP fetch; blob URLs revoked when replaced/disconnected. / 出力済みページを認証付き取得し、切替・切断時に表示データを解放。 / 认证获取已导出页，切换或断开时释放显示数据。 |

The UI uses the same API scopes as other clients: `jobs:read`, `jobs:submit`, `jobs:control`, `artifacts:read` (or `admin`). Server-side scope and credential expiry checks remain authoritative. Read-only tokens can view jobs; unauthorized mutations show an error. Static assets are public and contain no credentials or document data. The UI does not persist tokens, use cookies, or load third-party scripts/fonts. CSP permits only same-origin scripts/styles/API access and blob previews. Names and API errors are rendered as text.

日本語: 権限は既存APIと共通です。`jobs:read`・`jobs:submit`・`jobs:control`・`artifacts:read`（または `admin`）をサーバーで検査します。読み取り専用トークンでは参照でき、変更は拒否されます。静的画面に認証情報や原稿は含めず、トークンの永続化・Cookie・外部スクリプト／フォントは使用しません。原稿名やエラーは文字列として表示します。

简体中文: 权限与现有API一致，由服务器检查 `jobs:read`、`jobs:submit`、`jobs:control`、`artifacts:read`（或 `admin`）。只读令牌可查看任务，修改操作会被拒绝。静态资源不包含认证信息或原稿，不持久化令牌、不使用Cookie或第三方脚本／字体。原稿名和错误作为文本显示。

## Scope / 対応範囲 / 支持范围

New-job settings apply to RIP submission; physical-output settings apply to explicit deliveries. Existing job data and server configuration are retained. The console controls RIP jobs and explicit [physical deliveries](PHYSICAL-OUTPUT-CONSOLE.md). “Raster exported” means TIFF output completed. It does not mean paper was printed. Workers must run separately as before. PDF source-time page ranges are supported. Completed-job physical output provides media, duplex, copies, color-result selection, review/approval, submission, history and cancellation. The source preview shows the original RIP raster. Unsupported previews display an explicit message. Unknown rendering totals are not presented as fabricated percentages.

日本語: 新規ジョブの設定はRIP投入に、実機出力の設定は明示配信に適用します。既存ジョブのデータとサーバー設定は保持します。対象はRIPジョブで、「ラスター出力済み」はTIFF生成完了を表します。実機印刷の完了ではありません。ワーカーは従来どおり別途起動します。PDFの描画時ページ範囲に対応します。完了ジョブの実機出力では、用紙・両面・部数・色結果の選択、確認・承認・投入・履歴・取消を扱います。原本プレビューはRIP成果物です。表示できないプレビューは明示し、未計測の進捗率は表示しません。

简体中文: 新任务设置用于RIP提交，实体输出设置用于明确投递，保持既有任务数据和服务器配置。界面控制RIP任务，“光栅已导出”表示TIFF生成完成，不表示实机打印完成。Worker仍需单独运行。此界面支持PDF原稿渲染页码范围，完成任务的实体输出可选择介质、单双面、份数及色彩结果，并进行确认、审批、提交、历史查询及取消。原稿预览显示原始RIP产物。不支持的预览会明确提示，不虚构未测量的进度百分比。

A lost submission response is not automatically retried; refresh the queue before submitting again. UI languages are English, Japanese and Simplified Chinese. At narrow widths the panels stack and only the table scrolls horizontally; page content stays within 16px gutters.

日本語: 投入応答が失われた場合は自動再投入せず、キューを確認する案内を表示します。英語・日本語・简体中文に対応し、狭い画面ではパネルを縦に配置、表だけ横スクロールします。左右16pxの余白を確保します。

简体中文: 提交响应丢失时不会自动重试，会提示先检查队列。支持英语、日语及简体中文。窄屏时面板纵向排列，仅表格横向滚动，左右保留16px边距。

## PDF ranges / PDFページ範囲 / PDF页码范围

🟢 [Done] `2-3,5` selects original PDF positions 2, 3 and 5 before rasterization. Blank selects all and omits `page_ranges` from the new-job manifest. Endpoints must be 1–100, ascending and non-overlapping; adjacent ranges merge. Full-width digits, Japanese/Chinese commas and common range dashes normalize without collapsing spaces between separate numbers. Invalid values and nonempty PS/EPS ranges fail locally without an upload. Server validation remains authoritative; source bounds are checked by the worker. The info icon exposes syntax and limitations without adding permanent explanatory copy.

日本語: `2-3,5` はPDF原稿の2・3・5ページを描画します。空欄は全ページで、Manifestの追加属性を省略します。1〜100の昇順・重複なし、隣接範囲は結合します。全角数字や読点も正規化しますが、`1 2` を12として扱いません。不正な範囲やPS／EPSへの非空指定は投入前に拒否します。原稿の実ページ数はワーカーで検査します。投入・制御処理中は入力を固定し、ファイルや設定の混在を防ぎます。

简体中文: `2-3,5` 表示渲染PDF原稿的第2、3、5页。留空表示全部，不增加Manifest属性。范围限1至100、升序、不重叠，相邻范围合并。支持全角数字和中日文逗号正规化，但不会把 `1 2` 当作12。不合法范围及PS／EPS的非空选择在上传前拒绝，实际原稿页数由worker验证。处理期间锁定输入，避免文件或设置混用。

Mapped previews show original source numbers, e.g. 2 and 5, while fetching dense artifact names. Changing language preserves the selected preview. Details show requested ranges, generated page count and known original total. A completed IPP empty intersection shows zero pages and an explicit localized message, without a stale image. [Render selection](RENDER-PAGE-SELECTION.md), [IPP semantics](IPP-PAGE-RANGES.md).

日本語: プレビューは成果物の連番ではなく原稿ページ2・5を表示し、言語切替でも選択を維持します。詳細には指定範囲・生成ページ数・判明している原稿総数を表示します。IPPの0ページ正常完了では、ページ数0と専用メッセージを表示し、以前の画像を残しません。

简体中文: 预览显示原稿页2、5而非产物连续编号，切换语言保持选择。详情显示请求范围、生成页数和已知原稿总数。IPP空交集正常完成时显示0页和专用消息，不保留旧图像。

## Verification / 検証 / 验证

```sh
npm ci --prefix tests/ui --ignore-scripts
# Linux CI installs browser system dependencies with --with-deps.
tests/ui/node_modules/.bin/playwright install chromium
cargo test -p jrip-server --test console --test api --test color --locked
cargo test -p jrip-server --test console_browser operator_console_real_browser_flow --locked -- --ignored
# With Docker and both rebuilt RIP images, also run the real-render browser flow:
cargo test -p jrip-server --test console_browser operator_console_selected_render_real_worker --locked -- --ignored
```

Browser tests start isolated real HTTP servers and temporary repositories, and never contact a printer. The base regression uses synthetic worker-boundary fixtures and verifies PDF/PS/EPS upload, exact manifest settings and canonical ranges, invalid-range prevention, PS/EPS rejection/recovery, source page 2/5 preview choices, empty IPP results, state refresh, revision controls, scopes, disconnect/reload, text safety and EN/JA/zh at 320px, 375px and 430px. The separate real-render test uploads the owned two-page PDF from the console, invokes both real bounded MuPDF/Ghostscript worker paths, and verifies a green original-page-2 preview, one TIFF, source count 2 and rendering total 1. Set `JRIP_CHROMIUM_PATH` to use an existing Chromium binary; set `JRIP_UI_SCREENSHOTS` to a temporary directory for desktop/mobile evidence. CI installs the pinned npm dependencies and Chromium before running ignored integration tests.

⭕️ [Pending] Real printer acceptance remains separate. / 実機受入確認は別途保留。 / 实机验收另行待验证。

### Visual QA / 画面確認 / 视觉验证

The generated design concept and final rendered screenshots were inspected with `view_image`; IAB additionally verified the Japanese login and queue with no browser warnings/errors. Chromium exercised actual HTTP operations. Desktop was checked at 1440×900 and the concept's native 1487×1058 size; mobile at 320px, 375px and 430px in all three languages. Browser plugin skill was absent for the range-control update, so the existing Playwright workflow used installed Google Chrome through `JRIP_CHROMIUM_PATH`; no browser dependency version changed.

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

🟢 [Done] [用紙・実機配信の操作画面](PHYSICAL-OUTPUT-CONSOLE.md)。 / Media and physical-delivery controls / 介质与实体投递界面。
