# Console printer status / 操作画面の機器状態 / 操作界面设备状态

## 日本語

🟢 [Done] `/console`の実機出力欄でプリンターを選ぶと、共通Capability APIの機器状態・受付可否・非終端ジョブ件数・警告・消耗品を表示する。機器を更新するアイコンと説明用tooltipを追加し、固定ラベル・警告名・復旧案を英語／日本語／简体中文に対応した。機器固有の名前・description・未対応reasonは原文を保持する。

PWGとCUPSを別の情報源として表示する。PWGの容器の残り空間とCUPSの報告値を混同しない。0％は表示し、不明・未対応・未報告・特殊値を0へ置き換えない。percent以外の数量は報告単位を付け、百分率meterを作らない。meterは数値の視覚化で、印刷進捗や十分性の判定ではない。descriptionがない主要消耗品はローカライズした種別・色で表示する。

照会は選択・再読込・専用更新時に行い、機器を常時pollしない。snapshotの時刻と期限を表示し、開いている間の既存画面pollで期限切れ表示を更新する。照会失敗では同じプリンターの前回報告とその時刻を明示する。出力確認を無効化し、正常な再照会まで新しい確認を行えない。部数・色・ページ範囲等の有効な入力は維持する。更新は既存の出力確認と承認を解除し、ユーザーが確認し直す。取消や再送は行わない。プリンター解除・job変更・切断では以前の機器表示を消す。

取得値はtextContent／DOM属性へ設定し、HTMLとして解釈しない。外部画像・リンク・機器の文章を実行しない。既存CSPとscope制御を維持し、情報欄が操作権限を追加することはない。

## English

🟢 [Done] Selecting a printer in the console's physical-output panel displays common capability state, acceptance, device job count, structured notices and separate PWG/CUPS supply reports. Refresh/help icons, fixed labels, known fallback names and recovery guidance support English, Japanese and Simplified Chinese. Device labels and unknown reasons retain their original text.

Zero is visible; unknown, unsupported and absent values remain distinct. Only percentages produce meters; quantities in other units stay quantities. PWG remaining receptacle space is labeled separately from CUPS reported levels. Values do not represent page progress or sufficient supplies. Device queries occur on selection, reload or explicit refresh. Existing UI polling updates expiry labels without repeatedly probing the printer. A failed query clearly retains the same device's previous report and timestamp; new output review is blocked until a successful query. Refresh clears previous review/approval while preserving valid form inputs. Clearing a device or session clears the view. Device strings are text, never HTML or executable instructions. Existing CSP, API scopes and explicit print authority remain in force.

## 简体中文

🟢 [Done] 在实体输出栏选择打印机后，显示通用能力中的状态、接收能力、设备作业数、结构化通知及独立PWG／CUPS耗材报告。刷新／说明图标、固定标签、已知备用名称及恢复建议支持英日中三语，设备名称与未知reason保留原文。

保留0，不将未知、不支持或未报告值变成0。仅百分比显示meter，其他单位保留数量；PWG容器剩余空间与CUPS报告值明确区分，不表示页面进度或耗材充足保证。选择、重新加载或明确刷新时才查询设备；已有界面poll仅更新过期提示。查询失败显示同一设备的上次报告与时点，成功刷新前禁止新的输出确认；刷新清除旧确认／批准并保留有效输入。清除设备或会话时清除旧显示。设备字符串使用纯文本，不作为HTML或指令执行，沿用CSP、scope和明确打印授权。

Validation / 検証 / 验证: real local Chrome + Playwright, isolated Rust HTTP server, mock IPP and actual Docker color conversion. EN/JA/zh-CN at 320/375/430/1440px; zero/unknown/units, localized recovery tooltips, malformed reports, expiry, failed-query recovery, long/HTML-like labels, input preservation, approval invalidation and read-only credentials are exercised alongside the existing print/cancel/reload/lost-response flow. Screenshots are QA artifacts outside the repository. Browser skill was unavailable; installed Chrome was used without installing browser dependencies.

🟠 [Next] Push subscriptions / push購読 / push订阅。

⭕️ [Pending] Real-device labels, supply accuracy and vendor recovery procedures / 実機ラベル・残量精度・機種別復旧手順 / 实机标签、余量精度及厂商恢复步骤。

🟢 [Done] Query-driven durable device observations and cursor history / 照会駆動の永続機器観測とカーソル履歴 / 查询驱动的持久设备观察与cursor历史：[PRINTER-OBSERVATIONS.md](PRINTER-OBSERVATIONS.md)。

## Saved state after restart / 再起動後の保存状態 / 重启后保存状态

🟢 [Done] プリンター選択・更新時に、認証付き観測履歴を先に取得して前回の状態・警告・消耗品・成功観測時刻を復元する。その後に最新能力を照会し、成功すれば最新表示へ置き換える。保存表示は「保存済みの報告」と明示し、期限切れと照会失敗を独立して表示する。履歴の読取では機器照会・期限延長・印刷を行わない。保存値から用紙・部数・両面の能力、出力確認、承認を復元しない。登録に保存状態がなければ空表示とし、選択解除・セッション変更で以前の表示を消す。履歴API障害時も最新照会は試行する。画面は引き続き完了済みジョブの実機出力欄にある。自動的なプリンター選択や機器状態の常時購読は含まない。

🟢 [Done] Selection/refresh first reads authenticated stored observations, restoring status, notices, supplies and the last successful observation time before requesting live capabilities. Successful live queries replace the display. Saved reports are labelled explicitly; expiry and failed queries remain visible separately. Reading history never queries a device, renews expiry or prints. Cached data never populates media/copies/duplex capabilities, reviews or approvals. Missing evidence clears the display; deselection/session changes remove prior data. Cache API failure still allows a live query. This remains in the completed-job physical-output panel, without automatic destination selection or continuous subscriptions.

🟢 [Done] 选择或刷新打印机时，先读取已认证的观察历史，恢复状态、通知、耗材及上次成功观察时间，再查询实时能力。成功后替换为实时显示。明确标记已保存报告，并分别显示过期和查询失败。读取历史不会查询设备、延长有效期或打印。缓存不会恢复纸张、份数、双面能力、输出确认或审批。没有保存证据时清空显示；取消选择或变更会话时移除旧数据。缓存API故障不阻止实时查询。仍位于已完成作业的实体输出面板，不包含自动选择设备或持续订阅。

Tests reopen the Rust application/repository after persisting a fake-device report, reload the browser with failed live queries, and check EN/JA/zh-CN at 320/375/430/1440px. Expired/missing cache, cache failure with live recovery, disabled stale review and unchanged device-query counters are covered. / 模擬機器の状態保存後にAppとRepositoryを再生成し、ブラウザー再読込・3言語／各画面幅・期限切れ・履歴なし・履歴障害時の復旧・能力確認禁止・機器照会件数不変を検証する。 / 保存模拟设备报告后重建App与Repository，验证浏览器重载、三语言／各宽度、过期、无历史、缓存故障恢复、禁止缓存能力确认及设备查询计数不变。

## Independent printer panel / 独立プリンターパネル / 独立打印机面板

🟢 [Done] 接続後の「プリンター」パネルはジョブ0件・原稿未選択でも利用できる。パネルを開くと登録一覧を取得し、機器を選ぶと保存済み観測だけを読む。更新アイコンで選択機器の最新能力を明示照会し、失敗時は保存報告・時刻を残す。画面内タイマーは期限表示を更新するだけで、機器へ接続しない。一覧更新で登録変更を反映し、空登録・保存履歴なしは不明／未報告として扱う。プリンター切替・パネル閉鎖・切断をversionで区別し、遅い応答で別の機器やセッションを上書きしない。

このパネルはjobs:readで利用でき、投入・取消・再送・承認・出力条件変更を行わない。実機出力欄のreview／approvalは所有しない。最新表示も印刷承認や消耗品充足の証明にはならない。既存の実機出力欄は独自の能力照合と承認を引き続き行う。表示rendererのみ共有し、機器選択と状態は独立する。登録一覧を横断して全機器へ自動照会することもない。

🟢 [Done] After connecting, the Printers panel works with zero jobs and no selected document. Opening reads registrations; selection only reads stored observations. Its refresh icon explicitly queries the selected device and retains stored evidence/time on failure. The local timer only updates expiry labels. Reloading registrations reflects configuration changes; absent registrations/history remain unknown or unreported. Version guards fence late responses across device switches, panel closure and disconnects. The jobs:read view never submits, cancels, retries, approves or modifies delivery reviews. It shares the renderer, not delivery state. Current reports are not approval or proof of sufficient supplies; physical output retains its own capability checks and approval. No automatic fanout to devices is performed.

🟢 [Done] 连接后可在零作业、未选择原稿时使用打印机面板。打开时读取注册列表，选择设备只读取保存观察；刷新图标明确查询所选设备，失败时保留保存报告及时间。界面定时器仅更新过期提示。刷新注册列表反映配置变化，无注册或历史时显示未知／未报告。版本检查防止切换设备、关闭面板及断开会话后被延迟响应覆盖。jobs:read界面不提交、取消、重试、审批或修改投递确认，只共享renderer，不共享输出状态。实时报告不代表审批或耗材充足；实体输出仍独立检查能力及审批。不自动查询所有设备。

Validation: no-job Rust HTTP fixture with a fake IPP backend and a reopened repository; read-only credential; explicit query counters; failed response, device-switch race, disconnect/reconnect; EN/JA/zh-CN at 320/375/430/1440px. Real-printer accuracy remains Pending. / ジョブなし・Repository再生成・模擬IPP・読取専用資格情報で照会件数、失敗、切替競合、切断／再接続と3言語／各画面幅を検証。実機精度はPending。 / 无作业、重建Repository、模拟IPP与只读凭据下验证查询计数、失败、切换竞争、断开／重连及三语言／各宽度；实体设备精度仍为Pending。
