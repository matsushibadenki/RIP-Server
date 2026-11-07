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

🟠 [Next] Stored-state fallback/subscriptions and a printer dashboard independent of a completed raster job / 保存済み状態の復元・購読・原稿出力欄に依存しない機器画面 / 保存状态fallback／订阅及独立于已完成光栅作业的设备界面。

⭕️ [Pending] Real-device labels, supply accuracy and vendor recovery procedures / 実機ラベル・残量精度・機種別復旧手順 / 实机标签、余量精度及厂商恢复步骤。

🟢 [Done] Query-driven durable device observations and cursor history / 照会駆動の永続機器観測とカーソル履歴 / 查询驱动的持久设备观察与cursor历史：[PRINTER-OBSERVATIONS.md](PRINTER-OBSERVATIONS.md)。
