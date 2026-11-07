# PWG printer supplies / PWG消耗品情報 / PWG耗材信息

🟢 [Done] `org.jrip.pwg-printer-supplies-v1`: bounded read support for `printer-supply` and `printer-supply-description`.

| Interface | Location |
| --- | --- |
| `GET /api/v1/printers/{id}` / MCP `print_get_printer` | `model.capabilities.printerSupplies` |
| `GET /api/v1/print/capabilities?printerId={id}` / MCP `print_get_capabilities` | Printer stage `extensions["org.jrip.printer"].printerSupplies` |

## 日本語

Native IPP／IPPSで2属性を明示要求し、jobs:readで公開する。statusはREPORTED／UNREPORTED／INVALID_RESPONSE。unsupported・unknown・no-valueはattributeStateに保ち、0と区別する。CUPSのmarkerSuppliesとは独立した情報源で、項目の自動対応付け・値の上書き・平均化は行わない。

各項目はposition、raw、正規化したkeyのfields、typeに対応するkind、colorantName、supplyClass、unit、原値と解釈済みのlevel／maxCapacityを返す。positionは今回の配列順であり、永続的な消耗品IDではない。旧index等もfieldsに保持するが、対応付けには使わない。levelPercentはunit=percentと明示され、値が非負のときだけ返す。他単位からの比率計算や、単位欠落時の推測を行わない。

PWGのlevel=-1／-2／-3はOTHER／UNKNOWN／SOME_AVAILABLEとして原値を保持し、数値のlevelはnull。maxCapacityの-1／-2もOTHER／UNKNOWNとして保持する。CUPSの負値の意味とは別である。classによりlevelMeaningをCURRENT_SUPPLY／REMAINING_SPACE／UNSPECIFIEDで区別し、容器の充填量と残り空間を取り違えない。残り印刷枚数・出力可能性・補給操作は導出しない。

キーと値の構造、必須キー、重複、整数範囲、percentの範囲を検証する。区切りは`key=value;`で、コメント・改行・任意コードを解釈しない。keyは大文字小文字を区別せず照合し、rawは保持する。標準の色名とvendor形式の文字を許容するが、IANA登録済み値全体の認証ではない。上限は64項目、1record 4096バイト、32キー、1値1024バイト。既存のIPP応答全体64KiB制限も適用する。

descriptionは対応する位置にだけ結び付ける。textWithLanguageの長さとlanguage／UTF-8を検証し、言語を保持する。textWithoutLanguageも受け入れ、そのlanguageはnullとする。説明は最大1024 UTF-8バイト、明示言語は最大63バイト。descriptionの欠落・unknownはnull、件数不一致や不正descriptionは情報全体をINVALID_RESPONSEとし、切り詰めない。機器の説明を自動翻訳したり、その文章を命令として実行したりしない。

不正な任意の消耗品情報はreasonCode・3言語の案内・空suppliesとして分離し、他の有効な設定能力を破棄しない。親snapshotのrevision、観測時点、30秒期限を適用する。advisoryOnly=trueで、読取による印刷・取消・再送・外部URI取得はない。

## English

Native IPP/IPPS explicitly requests both attributes. jobs:read exposes REPORTED, UNREPORTED or INVALID_RESPONSE plus separate attribute availability, preserving unsupported/unknown/no-value without inventing zero. This source remains independent of CUPS markerSupplies; records are not automatically joined, averaged or overwritten.

Each supply carries its response position, raw record, parsed fields, kind/color/class/unit, original numbers and optional numeric quantities. Position and deprecated MIB indices are not treated as cartridge identities. Only an explicitly reported percent unit produces levelPercent. Other units and absent units are retained without ratio conversion. PWG negative level values retain their MIB semantics, separately from CUPS sentinels. levelMeaning distinguishes current supply, remaining receptacle space and unspecified meaning. No remaining-page prediction or readiness decision is inferred.

Parsing bounds are 64 records, 4096 bytes per record, 32 keys and 1024 bytes per value, within the existing 64KiB IPP reply cap. Key/value structure, required fields, duplicate keys, numeric ranges and percent ranges are checked; keys are case-insensitive and raw data is retained. Color-name extension characters are supported without claiming full IANA/MSN certification. Description arrays must align exactly when reported. Bounded textWithLanguage retains language and UTF-8 text; textWithoutLanguage retains a null language. Missing descriptions remain null. Malformed optional telemetry is withheld as an empty inventory with reasonCode and three-language messages, leaving valid core printer settings available. Parent snapshots provide revision, observation and 30-second expiry. Reads perform no print/control/resend actions or external URI fetches.

## 简体中文

Native IPP／IPPS显式请求两个属性，通过jobs:read公开REPORTED／UNREPORTED／INVALID_RESPONSE，并保留unsupported／unknown／no-value，不推测为0。与CUPS markerSupplies保持独立，不自动对应、平均或覆盖。

每项保留本次位置、原文、解析字段、类型／颜色／分类／单位及原始数值。位置与旧MIB index不是稳定耗材ID。仅显式unit=percent产生levelPercent，其他单位或单位缺失不换算。PWG负值按MIB语义保留，与CUPS特殊值区分。levelMeaning区分当前耗材量、容器剩余空间及未指定含义，不预测可打印页数或就绪状态。

限制64项、每record 4096字节、32个key、每value 1024字节，并遵守IPP整体64KiB上限。验证结构、必填项、重复key、整数及百分比范围；key不区分大小写，保留原文。支持扩展颜色名字符，但不声明完整IANA／MSN认证。description按位置严格对应，保留有界textWithLanguage的语言和UTF-8；无显式语言时language=null。缺失描述为null，数量不符或无效信息整体隔离为空supplies、reasonCode和三语说明，不丢弃其他有效设置能力。沿用snapshot的revision、观察时间及30秒期限。读取不打印、操作、重发或抓取外部URI。

Validation / 検証 / 验证: unit tests cover record grammar, ordering/extensions, sentinels, quantities, class meaning, language encoding, malformed payloads and budgets. Mock IPP verifies requested attributes and wire decoding. jobs:read-only REST tests compare both model paths with simultaneous CUPS reports, without sends or job mutation; MCP preserves the new fields.

🟠 [Next] Periodic supply observation/subscriptions, explicit cross-source reconciliation and supply-info link handling / 定期観測・購読・明示的な情報源照合・管理リンク / 定期观察、订阅、显式来源核对及管理链接。

⭕️ [Pending] Physical-device accuracy, update timing and PWG device conformance / 実機精度・更新時点・PWG機器適合性 / 实机精度、更新时间及PWG设备适合性。

References: [PWG 5100.13-2023 §6.6.11–12](https://ftp.pwg.org/pub/pwg/candidates/cs-ippnodriver20-20230301-5100.13.pdf), [RFC 3805 marker supply semantics](https://www.rfc-editor.org/rfc/rfc3805.html).

🟢 [Done] Console status/supply presentation / 操作画面の状態・消耗品表示 / 界面状态与耗材展示：[CONSOLE-PRINTER-STATUS.md](CONSOLE-PRINTER-STATUS.md)。

🟢 [Done] Query-driven durable device observations and cursor history / 照会駆動の永続機器観測とカーソル履歴 / 查询驱动的持久设备观察与cursor历史：[PRINTER-OBSERVATIONS.md](PRINTER-OBSERVATIONS.md)。
