# Printer conditions / 機器状態の構造化 / 设备状态结构化

🟢 [Done] Profile `org.jrip.printer-conditions-v1`.

| Interface | Location |
| --- | --- |
| `GET /api/v1/printers/{id}` / MCP `print_get_printer` | `model.conditions` |
| `GET /api/v1/print/capabilities?printerId={id}` / MCP `print_get_capabilities` | Printer stage `extensions["org.jrip.printer"].conditions` |

## 日本語

jobs:readで取得するライブ情報。Native IPPで検証したprinter-stateとprinter-state-reasonsを共通の純粋関数で解釈し、RESTとMCPへ同じ構造で返す。元のstate／reasonsは保持する。問い合わせは印刷・取消・再送を行わず、操作権限も与えない。

条件ごとにrawReason、suffixを除いたreason、known、category、severity、severitySourceと3言語の復旧案を返す。error／warning／report suffixを優先する。suffixがなければ、stopped状態はerror、idle／processingはwarningとして扱う。復旧actionは助言であり、自動実行しない。用紙不足、ジャム、トナー／marker supply、廃棄容器、カバー、トレイ、排紙部、一時停止、接続の主要reasonを分類し、未対応の標準・vendor reasonはunknownとして原文を残す。

reason欠落はUNREPORTED、noneのみはNO_REASONS_REPORTEDとし、正常・残量十分と推測しない。重複条件は除き、条件を安定した順に返す。noneと他reasonの混在、errorと非stopped状態、停止中のerror報告なしはinconsistenciesに明示する。機器の値を上書きしたり、印刷結果の失敗へ勝手に変換したりしない。件数・state・受付可否・条件の意味は独立である。

supplyLevelsKnown=false。これは消耗品の警告／状態であり、色別残量・百分率・型番・残り印刷枚数の取得ではない。automaticResendPermitted=false、authorizationGranted=false。親snapshotのrevision・観測時点・30秒期限・認証を適用し、現時点の助言として扱う。

## English

Live jobs:read queries interpret the validated Native IPP printer-state/reasons through one shared pure function. REST and MCP expose the same conditions alongside the original values. Reads never print, cancel, resend or grant control authority.

Each condition preserves rawReason and exposes its base reason, known/category, severity/source and advisory recovery action/messages in English, Japanese and Simplified Chinese. Explicit error/warning/report suffixes take precedence. Without a suffix, stopped defaults to error and idle/processing defaults to warning. Supported categories cover common media, jam, marker supply, waste, covers, trays, output, pause and connection reasons. Other standard/vendor reasons remain unknown with their raw value. Suggestions are not executed.

Missing reasons produce UNREPORTED; none alone produces NO_REASONS_REPORTED, not a health guarantee. Conditions are deduplicated and sorted. Mixed none/other reasons, errors outside stopped state and stopped state without a reported error are surfaced as inconsistencies without rewriting device data or job outcomes. Supply warning conditions do not provide numeric levels, cartridge identity, color-specific amounts or remaining impressions: supplyLevelsKnown=false. automaticResendPermitted=false and authorizationGranted=false. The enclosing authenticated snapshot supplies revision, observation time and its 30-second expiry.

## 简体中文

通过jobs:read查询实时信息，以共享纯函数解释Native IPP已验证的printer-state／reasons。REST与MCP返回相同结构并保留原值。读取不打印、取消、重发或授予操作权限。

每个条件包含rawReason、去除suffix的reason、known／category、severity／来源，以及英日中三语恢复建议。显式error／warning／report suffix优先；无suffix时，stopped解释为error，idle／processing为warning。分类覆盖常见纸张、卡纸、成像耗材、废料容器、盖板、纸盒、输出、暂停及连接reason。其他标准或厂商reason按unknown保留原文，不执行建议操作。

缺失reason为UNREPORTED，仅none为NO_REASONS_REPORTED，不推测健康状态。条件去重并稳定排序；混合none与其他reason、非stopped状态下的error、停止状态未报告error，均以inconsistencies公开，不改写设备数据或作业结果。supplyLevelsKnown=false：警告不是百分比、颜色余量、耗材型号或剩余印数。automaticResendPermitted=false、authorizationGranted=false。沿用认证snapshot的revision、观察时间与30秒期限。

Validation / 検証 / 验证: unit tests cover suffix/default severity, unknown values, missing/none, deduplication, inconsistent reports and three-language guidance. Mock IPP integration checks both REST models return identical conditions without sending or changing job revision. MCP forwarding preserves the structured data.

🟠 [Next] Broader reason coverage and periodic device-condition subscriptions / reason拡充・定期観測と購読 / 扩展reason及定期观察／订阅。

⭕️ [Pending] Vendor-specific reason accuracy and recovery procedures require real printer acceptance / 機種別reasonの正確性と復旧手順は実機受入が必要 / 厂商reason准确性及恢复步骤需实机验收。

Severity and base-reason reference: [RFC 8011 §5.4.12](https://www.rfc-editor.org/rfc/rfc8011.html#section-5.4.12).

🟢 [Done] Separate CUPS marker numeric telemetry / 独立したCUPS marker数値情報 / 独立CUPS marker数值信息：[PRINTER-SUPPLIES.md](PRINTER-SUPPLIES.md)。

🟢 [Done] Separate PWG supply telemetry / 独立したPWG数値情報 / 独立PWG数值信息：[PWG-PRINTER-SUPPLIES.md](PWG-PRINTER-SUPPLIES.md)。

🟢 [Done] Console status/supply presentation / 操作画面の状態・消耗品表示 / 界面状态与耗材展示：[CONSOLE-PRINTER-STATUS.md](CONSOLE-PRINTER-STATUS.md)。

🟢 [Done] Query-driven durable device observations and cursor history / 照会駆動の永続機器観測とカーソル履歴 / 查询驱动的持久设备观察与cursor历史：[PRINTER-OBSERVATIONS.md](PRINTER-OBSERVATIONS.md)。
