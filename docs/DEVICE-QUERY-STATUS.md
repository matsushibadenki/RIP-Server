# Device query status / 機器照会状態 / 设备查询状态

## 日本語

🟢 [Done] 受付済みDeliveryの`deviceQuery`は、機器ジョブ照会の結果を`deviceObservation`と分けて保存する。`failure`は成功時null、失敗時は以下の分類を返す。

- `UNAVAILABLE`: 通信・認証・その他の照会エラー。機器の電源断やオフラインと断定しない。
- `INVALID_RESPONSE`: 不正なIPP応答。以前の状態・進捗を上書きしない。
- `JOB_NOT_FOUND`: 機器がIPPのjob不在を返した。履歴削除などもあり、印刷失敗・完了とは断定しない。
- `REGISTRATION_UNAVAILABLE`: 保存済み出力登録と現在の設定を確認できず、照会できない。変更先へ自動接続しない。

`changedAt`は照会結果の分類が変わったUNIX秒で、最終照会時刻やheartbeatではない。同じ失敗の反復はrevision・イベントを増やさない。照会が復旧すると、機器状態が前回と同じでも観測とquery成功を同一トランザクションで保存する。照会失敗中は前回の観測・取消要求・ACCEPTEDを保持し、結果receiptを作ったり再送可能に戻したりしない。再照会は行うが再印刷ではない。終端後は更新しない。

REST詳細・履歴、共通ticketの`execution.deviceQuery`、既存MCP読取、pipelineイベントdetailsで確認できる。終端receiptのhashにも最終query情報を含める。旧記録は未観測としてフィールド省略またはnullのまま扱う。契約digestは変更しない。

操作画面は失敗分類・記録時刻と「前回の機器状態」を表示し、復旧後は失敗表示を消す。照会成功の記録も現在のオンライン保証ではない。worker停止の検知や周期的heartbeatは本機能に含まない。

検証：ストレージの原子的保存・取消要求保持・重複抑止・再起動・終端保護、模擬IPPの不正応答・job不在・登録変更・TCP待受の停止／再開・復旧・送信1回、実APIブラウザーの3言語表示・復旧・モバイル・取消を確認。

⭕️ [Pending] 実機の履歴保持・認証更新・ネットワーク断・電源断の受入には実機環境が必要。

## English

🟢 [Done] Accepted deliveries persist `deviceQuery` independently from the last device observation. Its failure is null for a successful query, or UNAVAILABLE (transport/authentication/other failure), INVALID_RESPONSE, JOB_NOT_FOUND (IPP job-not-found), or REGISTRATION_UNAVAILABLE (stored registration cannot be matched to current configuration). These are query outcomes, not inferred print outcomes or proof of device power/online status.

changedAt records a changed query classification, not every poll or a heartbeat. Identical failures do not churn revisions/events. Recovery atomically records a fresh successful observation even if state and counters are unchanged. Failure preserves previous observations, cancellation requests and ACCEPTED state; it cannot issue a terminal receipt, requeue or resend. Terminal records are immutable.

REST detail/history, unified execution.deviceQuery, existing MCP readers and printer event details expose the metadata. Final receipt hashing includes it; immutable contract digests remain unchanged. Legacy records omit it or read as null. The console shows query failure and timestamp alongside the last reported device state, then clears the failure after recovery. Worker liveness monitoring is separate work.

Tests cover durable/no-op/stale/terminal updates, cancellation preservation, fake IPP invalid responses/job absence, changed registration, actual loopback TCP listener shutdown/restart, recovery, exactly one submission and trilingual responsive browser interactions.

⭕️ [Pending] Hardware acceptance must cover real job retention, credential updates, power and network interruptions.

## 简体中文

🟢 [Done] 已接收投递的`deviceQuery`与上次设备观察分别保存。成功时failure为null；失败分类为UNAVAILABLE（通信、认证等）、INVALID_RESPONSE、JOB_NOT_FOUND（IPP作业不存在）、REGISTRATION_UNAVAILABLE（无法核对保存注册与当前配置）。这些只表示查询结果，不推测打印结果或设备电源、在线状态。

changedAt表示查询分类变化的UNIX时间，不是每次查询时间或心跳。相同失败不增加revision与事件。恢复时即使状态与计数相同，也以事务记录新观察和查询成功。失败保留前次观察、取消请求及ACCEPTED，不生成终结receipt、不重新排队或发送。终结记录不可修改。

REST详情与历史、统一execution.deviceQuery、现有MCP读取、事件details公开记录；最终receipt摘要包含查询信息，契约摘要不变。旧记录省略或为null。界面显示失败分类、时间及上次设备状态，恢复后消除失败提示。worker存活监测是独立待办。

测试覆盖持久化、去重、过期revision、终结保护、保留取消、模拟IPP异常及作业不在、注册变化、实际本地TCP停止与重启、恢复、仅发送一次和三语言响应式浏览器交互。

⭕️ [Pending] 实际作业保留、凭证更新、断电及断网需要实体设备验收。
