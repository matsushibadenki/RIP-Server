# Marker supplies / 消耗品情報 / 耗材信息

🟢 [Done] `org.jrip.cups-marker-supplies-v1`: optional CUPS marker telemetry over Native IPP/IPPS.

| Interface | Location |
| --- | --- |
| `GET /api/v1/printers/{id}` / MCP `print_get_printer` | `model.capabilities.markerSupplies` |
| `GET /api/v1/print/capabilities?printerId={id}` / MCP `print_get_capabilities` | Printer stage `extensions["org.jrip.printer"].markerSupplies` |

## 日本語

jobs:readで取得する、機器が報告したライブ情報。marker-names／types／colors／levels／low-levels／high-levelsを明示要求する。CUPS拡張であり、すべてのIPP機器で取得できるとは限らない。既存snapshotの観測時点・30秒期限・revisionを適用する。markerのindexは今回の並び順（1始まり）で、機器間や次回照会でも同じ消耗品を示す永続IDではない。

statusはREPORTED、UNREPORTED、INVALID_RESPONSE。百分率はlevelPercentに保持し、0を不明値へ変換しない。rawLevel=-1／-2／-3は、それぞれUNAVAILABLE／UNKNOWN／NOT_AT_CAPACITYとして保存し、levelPercentはnull。属性のunsupported／unknown／no-valueもlevelsAttributeStateで区別する。名前・種別・色・上下限が未報告ならnull。numericLevelsReportedとallLevelsNumericで数値の取得範囲を表す。

最大64項目、文字列はUTF-8で1024バイトまで。型・範囲・長さ・文字を検証し、存在する配列の件数が違えば切り詰めたり別の消耗品へ対応付けたりせず、情報全体をINVALID_RESPONSEにしてmarkersを空にする。reasonCodeと3言語のメッセージを返す。任意の消耗品情報の不正だけでは、用紙・DPI・受付可否等の有効な設定能力を捨てない。ただし内容はcapabilityRevisionへ含まれるため、変化により再確認が必要になる場合がある。

百分率・上下限・種別は報告値のまま。インク等の消費量と廃棄容器の充填量では方向が異なるため、「残り印刷枚数」や独自の十分／不足判定へ変換しない。条件reasonの解釈は[PRINTER-CONDITIONS.md](PRINTER-CONDITIONS.md)を参照。conditions.supplyLevelsKnown=falseはreasonから数量を導出していない意味であり、markerSuppliesは別の機器報告として扱う。読取は印刷・取消・補給・再送を行わず、advisoryOnly=true。

## English

Live jobs:read queries explicitly request the six CUPS marker attributes listed above. This vendor-extension family is optional; it is not universal IPP supply support. Parent snapshots provide observation time, a 30-second expiry and content revision. Marker indices are one-based positions in this response, not durable cartridge identities.

status distinguishes REPORTED, UNREPORTED and INVALID_RESPONSE. Numeric percentages retain zero. rawLevel -1/-2/-3 maps to UNAVAILABLE/UNKNOWN/NOT_AT_CAPACITY with a null levelPercent; levelsAttributeState also preserves unsupported/unknown/no-value attribute states. Unreported labels, types, colors and thresholds remain null. numericLevelsReported and allLevelsNumeric describe numeric coverage.

At most 64 markers and 1024 UTF-8 bytes per string are accepted. Types, ranges, lengths and control characters are checked. Present arrays must agree in cardinality. Malformed telemetry is withheld as an empty inventory plus reasonCode and three-language messages; it is never truncated or incorrectly zipped. An optional telemetry error does not discard valid core printer capabilities, though inventory changes participate in capabilityRevision and may require renegotiation.

Percentages and thresholds are device reports, not remaining-page estimates or derived readiness. Waste containers may fill up while consumables deplete; high/low thresholds are preserved without assuming a common direction. Reason-derived conditions remain separate and do not fabricate numeric levels. Reads perform no printing, cancellation, replenishment or resend, and advisoryOnly is true.

## 简体中文

jobs:read实时查询显式请求六个CUPS marker属性。它们是可选扩展，并非所有IPP设备都支持。沿用snapshot的观察时间、30秒期限及内容revision。marker index是本次响应从1开始的位置，不是跨查询或设备稳定的耗材ID。

status区分REPORTED、UNREPORTED与INVALID_RESPONSE。保留数值0；rawLevel的-1／-2／-3分别表示UNAVAILABLE／UNKNOWN／NOT_AT_CAPACITY，levelPercent为null。levelsAttributeState另行保留unsupported／unknown／no-value状态。未报告名称、类型、颜色及阈值为null；numericLevelsReported和allLevelsNumeric表示数值覆盖范围。

上限64项，每个字符串最多1024个UTF-8字节。验证类型、范围、长度及控制字符；已提供的数组数量必须相同，不截断或错误关联。无效耗材信息以空markers、reasonCode及三语信息返回，不丢弃其他有效打印能力；但变化参与capabilityRevision，可能需要重新协商。

百分比和阈值是设备报告，不转换为剩余可打印页数或就绪保证。废料容器可能增加，耗材可能减少，因此保留原始上下限而不假定共同方向。reason条件不推导数值，markerSupplies作为独立设备报告。读取不打印、取消、补充或重发，advisoryOnly=true。

Validation / 検証 / 验证: unit and mock IPP tests cover zero/100, all negative sentinels, missing/out-of-band attributes, partial metadata, waste thresholds, UTF-8 names, cardinality, malformed tags/lengths/text/ranges and 64-item bounds. Authenticated REST/common-model equality and jobs:read-only access are tested without changing job revisions or sending. MCP preserves supplied metadata.

🟠 [Next] Periodic observation, subscriptions and global retention policy / 定期観測・購読・全体保持ポリシー / 定期观察、订阅及整体保留策略。

⭕️ [Pending] Actual device coverage, update cadence, precision and consumable replacement identity need real-printer acceptance / 機種別対応・更新頻度・精度・交換時の識別は実機受入が必要 / 设备支持范围、更新频率、精度及更换耗材识别需实机验收。

Reference: [OpenPrinting CUPS supply attributes](https://github.com/OpenPrinting/libcups/blob/master/doc/spec-ipp.html#marker-levels).

🟢 [Done] Independent PWG supply records / 独立したPWG消耗品情報 / 独立PWG耗材信息：[PWG-PRINTER-SUPPLIES.md](PWG-PRINTER-SUPPLIES.md)。

🟢 [Done] Console status/supply presentation / 操作画面の状態・消耗品表示 / 界面状态与耗材展示：[CONSOLE-PRINTER-STATUS.md](CONSOLE-PRINTER-STATUS.md)。

🟢 [Done] Query-driven durable device observations and cursor history / 照会駆動の永続機器観測とカーソル履歴 / 查询驱动的持久设备观察与cursor历史：[PRINTER-OBSERVATIONS.md](PRINTER-OBSERVATIONS.md)。
