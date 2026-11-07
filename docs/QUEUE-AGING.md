# Queue aging / キューのaging / 队列老化

## 日本語

🟢 [Done] 単一ホストRIP dispatcherの`next_queued`は、次の順序でQUEUEDジョブを選択する。

1. 実効優先度 `min(100, 指定優先度 + floor(max(0, 現在時刻 - createdAt) / 60))` の降順。
2. 同点では受付時刻の古い順、さらに同時刻ならUUIDの昇順。

指定優先度0でも、受付から100分で実効優先度100になり、後から来た高優先度ジョブより先に選択される。これは処理開始まで100分という保証ではない。既に実行中の長いジョブは中断せず、先行する古いジョブも処理する。指定優先度・契約・revisionは変更しない。再起動後も保存済み受付時刻から同じ選択を計算する。

HELD、実行中、取消、失敗、完了は選択対象外。保留解除した古いジョブは元の受付時刻でagingする。時計が戻った場合は待ち時間を0に制限し、指定優先度を下げない。時計が進むとagingも進む。キュー一覧は従来の指定優先度順であり、dispatcherの実効順位を表示するものではない。

検証：0優先度の5999秒／6000秒境界、最高優先度の新着との競合、保留除外、終端除外、時計の逆行、再起動、同時刻UUID順、指定優先度とrevisionの不変性。

🟠 [Next] tenant別fairness、期限、複数機器への割当、回数制限付き自動再試行。Deliveryの再送や優先順位は本機能の対象ではない。

## English

🟢 [Done] The single-host RIP dispatcher selects QUEUED jobs by descending effective priority `min(100, priority + floor(max(0, now - createdAt) / 60))`, then oldest creation time and ascending UUID. A priority-zero job reaches 100 after 100 minutes and outranks newer arrivals. This is not a 100-minute start-time guarantee: running work is not preempted and older queued jobs still precede it.

Declared priority, immutable contracts and revisions remain unchanged. Selection survives restarts because it derives from stored creation times. Held, running and terminal jobs are excluded. Releasing held work retains its original arrival age. Clock rollback clamps age to zero; clock jumps advance age. The job listing retains declared-priority ordering rather than advertising a live scheduler rank.

Tests cover the aging boundary, newer highest-priority work, excluded states, clock rollback, reopening the database, deterministic ties and unchanged metadata.

🟠 [Next] Tenant fairness, deadlines, device assignment and bounded automatic retries remain unfinished. This scheduler neither resends nor reprioritizes device deliveries.

## 简体中文

🟢 [Done] 单机RIP dispatcher只选择QUEUED作业，按有效优先度`min(100, priority + floor(max(0, now - createdAt) / 60))`降序，再按接收时间和UUID升序。优先度0的作业经过100分钟达到100，排在后续到达作业之前。这不是100分钟内开始处理的保证：不抢占正在执行的作业，较早的等待作业仍优先。

指定优先度、不可变契约与revision不变；重启后使用持久化接收时间计算。保留、执行中及终结作业不参与选择。解除保留仍使用原接收时间。时钟回退将等待时间限制为0，前跳会加快老化。作业列表仍按指定优先度排序，不表示实时调度排名。

测试覆盖老化边界、新到达的最高优先度作业、状态排除、时钟回退、重开数据库、确定性排序和元数据不变性。

🟠 [Next] 租户公平、期限、设备分配及有限自动重试仍未完成。本调度不重发或调整设备投递顺序。

## Capability discovery and PrintIntent / 能力公開と投入 / 能力公开与投入

🟢 [Done] 共通Capability ModelのRIP stageは`extensions["org.jrip.scheduling"]`で`org.jrip.priority-aging-v1`を公開する。単一ホストRIP、QUEUED限定、指定優先度0〜100・既定50、60秒ごとに1点、上限100、createdAt／jobIdの同点順序、実行中を中断しないことを示す。上限・間隔はSQLite選択と共有する定数であり、公開値だけの別実装にしない。Delivery stageのfairnessや機器内job-priorityを有効と宣言しない。

PrintIntentの既存priority交渉を公開経路で検証した。required／preferredの有効値は保持し、unsupported preferredは50へ調整して明示、automaticは50を選ぶ。requiredの範囲外は競合、負数・小数・文字列・u32範囲外は型エラーとして投入しない。優先度はpreflight ticketとrequestHashに拘束され、commitで別値へ上書きできない。同一commitの再要求は同じjob-idへ戻る。RESTの原稿投入・再起動・実キュー選択、MCP事前確認・能力読取、実機PrintIntentのRIP側優先度保持を試験した。事前確認が承認や実行権限を授与する意味ではない。

🟢 [Done] The RIP stage advertises `extensions["org.jrip.scheduling"]`, profile `org.jrip.priority-aging-v1`: single-host RIP, QUEUED eligibility, declared priority range/default, aging interval/cap, FIFO/UUID ties and no preemption. SQLite selection and discovery share constants for interval and cap. This does not enable device-delivery priority, tenant fairness or printer-native job-priority.

Existing PrintIntent priority negotiation is now covered end to end. Valid required/preferred values survive; unsupported preferences explicitly adjust to 50; automatic chooses 50. Required out-of-range values conflict; invalid numeric/string types are rejected without jobs. Priority is bound into the preflight ticket hash and committed manifest. Replay returns the same job ID. REST tests verify five modes, hash binding, replay, reopening and actual queue choice; MCP tests verify forwarding/discovery; physical PrintIntent tests preserve the source RIP priority through restart. Preflight grants no execution authority.

🟢 [Done] RIP stage公开`extensions["org.jrip.scheduling"]`，版本`org.jrip.priority-aging-v1`，说明单机RIP、仅QUEUED、优先度范围与默认值、老化间隔与上限、FIFO／UUID同分顺序及不抢占。SQLite选择与公开上限、间隔共享常量，不宣称设备投递优先度、租户公平或打印机job-priority。

已有PrintIntent优先度协商增加端到端验证：有效required／preferred保持原值，不支持的preferred明确调整为50，automatic选择50；required超范围冲突，负数、小数、字符串及u32溢出拒绝且不创建作业。优先度绑定到预检ticket摘要与提交manifest，同一提交重放返回原job-id。REST测试覆盖五种协商、摘要、重放、重启和真实队列选择，MCP测试验证转发与能力读取，实体PrintIntent测试验证RIP优先度跨重启保持。预检不授予执行权限。
