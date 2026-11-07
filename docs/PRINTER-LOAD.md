# Printer load / 機器負荷 / 设备负载

## 日本語

🟢 [Done] `GET /api/v1/printers/{id}`の`model.capabilities.queuedJobCount`とMCP `print_get_printer`で機器報告の非終端ジョブ件数を取得する。IPP `queued-job-count`は待機・保留・印刷中・停止中を含み、RIP側の待機件数や残りページ数ではない。

Get-Printer-Attributesで属性を明示要求する。0以上の単一integerを受け付ける。属性なし、または空のIPP unknown値はnull。0は機器が0件と報告した場合だけ返す。負数、型違い、長さ不正、重複は不正応答とし、負荷を推測しない。既存のobservedAt・expiresAtがsnapshotの時点・期限を示す。件数は状態や受付可否と独立で、0件でも出力可能とは保証しない。

模擬IPPで0・7・最大整数・欠落・unknown・不正値を検証。RESTで公開を確認し、MCP転送試験で0・値あり・nullを保持することを確認。機器負荷の変化も現在のcapabilityRevisionのhash対象なので、再照合が必要になる場合がある。自動的な機器割当や待ち時間予測は行わない。

⭕️ [Pending] 実機の件数更新タイミング、外部クライアントの投入との競合、認証による属性制限は実機受入が必要。

## English

🟢 [Done] The live printer REST model exposes `model.capabilities.queuedJobCount`; MCP `print_get_printer` preserves it. This is the printer-reported count of pending, held, processing and stopped jobs, not the local RIP queue, remaining pages or an ETA.

The backend explicitly requests queued-job-count. A single nonnegative IPP integer is accepted; absence or an empty IPP unknown value yields null. Zero is returned only when reported. Negative, wrong-type, malformed-length and duplicate values invalidate the response. Existing observedAt/expiresAt bound the snapshot. Queue size, accepting-jobs and device state are independent; zero does not promise readiness. Load is currently part of capabilityRevision hashing, so changes may require renegotiation. No automatic routing is introduced.

Mock IPP tests cover zero, seven, maximum integer, missing/unknown and malformed replies. REST tests check exposure; an MCP forwarding test checks zero/value/null preservation.

⭕️ [Pending] Real-device update timing, concurrent external submissions and attribute visibility require hardware acceptance.

## 简体中文

🟢 [Done] 实时设备REST的`model.capabilities.queuedJobCount`及MCP `print_get_printer`提供设备报告的非终结作业数，包括等待、保留、打印和停止；并非本地RIP队列、剩余页数或预计时间。

后端显式请求queued-job-count，仅接受单个非负IPP整数。缺失或空unknown值返回null，不将未知推测为0。负数、类型错误、长度错误及重复属性使响应无效。observedAt及expiresAt表示快照时点和期限。数量与接收能力、设备状态独立，0不保证可用。负载目前参与capabilityRevision摘要，变化可能要求重新协商，不自动分配设备。

模拟IPP测试覆盖0、7、最大整数、缺失、unknown及无效响应；REST验证公开，MCP转发测试验证保留0、数值及null。

⭕️ [Pending] 实际设备更新时间、外部客户端并发提交及权限限制需要实体设备验收。

Reference: [RFC 8011 §5.4.24](https://www.rfc-editor.org/rfc/rfc8011.html#section-5.4.24).

## Common capability snapshot / 共通能力snapshot / 通用能力快照

🟢 [Done] `GET /api/v1/print/capabilities?printerId={id}` and MCP `print_get_capabilities({printerId})` include `acceptingJobs`, `queuedJobCount` and `queuedJobCountMeaning=device-reported-nonterminal-jobs` in the Printer stage's `extensions["org.jrip.printer"]`. Existing state/reasons remain alongside them. jobs:read is sufficient; no print/control authority is granted. Unknown load remains null and reported zero remains zero, independently of acceptance and supported settings. An available stage describes acceptance and the implemented compatible settings, not an idle device, immediate printing, or a supply guarantee. Live changes participate in stage and overall capabilityRevision hashes; observedAt/expiresAt retain the 30-second observation window. Repeated identical observations retain the same content revision. Invalid device counters fail the query rather than returning a cached or guessed value. Reads do not send a print job. No automatic routing, queue-time prediction, consumable levels or tenant fairness is introduced.

🟢 [Done] 共通Print APIとMCPのPrinter工程にも受付可否・非終端ジョブ件数・意味を公開した。0とnull、受付可否、設定の互換性を独立に保つ。availableは受付と対応設定を表し、アイドル・即時出力・消耗品の十分性は保証しない。機器負荷も工程別／全体revisionへ反映し、30秒の観測期限を維持する。同じ内容ではrevisionを維持し、不正応答時はqueryを失敗させて推測・キャッシュ値を返さない。jobs:readで照会でき、印刷や操作権限の付与は発生しない。自動振分・待ち時間予測・消耗品残量・tenant fairnessは未実装。

🟢 [Done] 通用Print API和MCP的Printer阶段公开接收状态、非终结作业数及其含义。区分0与null、接收能力及设置兼容性；available并不保证空闲、立即打印或耗材充足。负载参与阶段及整体revision，保留30秒观察期限，相同内容保持同一revision。无效响应使查询失败，不返回猜测或缓存值。jobs:read即可查询，不发送打印作业或授予操作权限。自动分配、等待时间预测、耗材余量与tenant fairness仍未实现。

Validation / 検証 / 验证: mock IPP → authenticated common REST covers positive/zero/absent/invalid counts, acceptance=false with zero, content revision transitions, expiry and no sends; MCP forwarding preserves zero/value/null with jobs:read only.

🟢 [Done] Structured reason evidence / reasonの構造化 / reason结构化：[PRINTER-CONDITIONS.md](PRINTER-CONDITIONS.md)。
