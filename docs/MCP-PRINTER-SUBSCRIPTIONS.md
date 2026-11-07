# MCP printer subscriptions / MCP機器観測の購読 / MCP设备观察订阅

🟢 [Done] `resources/subscribe` / `resources/unsubscribe`: `jrip://printers/{printerId}/observations`.

## 日本語

jobs:readを持つAgentは、保存済みの機器観測resourceを購読できる。stdioおよびMCP HTTP/SSEの既存transportが、追加のクライアント要求なしでnotifications/resources/updatedを送る。通知にはURIだけを含め、データ本文や権限を付与しない。受信後はresourceまたはprint_get_printer_observationsを読み、詳細を取得する。個別の保持イベントURIは購読対象ではない。

購読によって機器をpollしたり、印刷・取消・再送したりしない。既存のlive能力照会によって保存された変化を、約1秒ごとの内部履歴照会で確認する。[定期観測worker](PRINTER-OBSERVER.md)を別途起動すると、定期照会の保存結果も通知対象になる。照会の間の変化をすべて検出する保証はない。

要求済みcursorに加えてstreamIdと保持境界を追跡する。再登録でstreamが変わればcursorを0へ戻す。以前のstreamを復元してsequenceが小さくなっても読み落とさない。保持履歴のprunedThrough変更、登録解除、再利用可能化もresourceの再読込を促す。同一報告の観測時刻だけの更新では通知しない。権限不足や不正な応答ではcursorを進めず、状態を漏らさない。購読解除後は通知しない。

通知は更新の合図であり、イベントのexactly-once配信や全履歴の保管保証ではない。HTTP/SSE再接続時は保持履歴から通知を再生するため重複し得る。プロセス再起動後はMCPを再初期化して購読し直す。アプリケーションはstreamIdと読取APIのnextAfterを保存し、historyTruncatedを確認する。登録ごとの1000件保持は変更しない。既存の32購読上限、認証・セッション期限・SSE接続制限を維持する。

## English

Agents with jobs:read can subscribe to the stored observation resource above. Existing stdio and MCP HTTP/SSE transports push notifications/resources/updated without another client request. Notifications carry only the URI; read the resource or print_get_printer_observations for data. Individual retained event URIs are not subscribable.

Subscription checks stored history about once per second; it does not poll a printer or perform print/control/resend operations. Only changes recorded by live capability queries are observed. The separately started [periodic observer](PRINTER-OBSERVER.md) also records changes for notifications; changes between queries may be missed. Stream identity and pruning boundaries are tracked alongside the cursor. Registration changes reset the cursor, including restoration of an older stream with smaller sequences. Pruning, configuration removal and availability recovery invalidate the resource. Identical-report timestamps alone do not notify. Forbidden or malformed responses do not advance cursors.

Notifications are advisory, may coalesce or replay, and are not exactly-once event delivery. HTTP/SSE reconnection replays retained invalidation; process restart requires initialization/subscription again. Persist streamId and the read API's nextAfter, and inspect historyTruncated. The existing 1000-event retention, 32-subscription cap, authentication, session expiry and SSE connection limits remain in force.

## 简体中文

拥有jobs:read的Agent可订阅上述已保存观察resource。现有stdio与MCP HTTP／SSE可在无额外客户端请求时推送notifications/resources/updated。通知仅含URI，不授予权限；通过resource或print_get_printer_observations读取数据。单个保留事件URI不支持订阅。

每秒左右检查保存历史，不轮询打印机、不打印、取消或重发。只有实时能力查询已保存的变化才可通知；单独启动的[定期观察worker](PRINTER-OBSERVER.md)也会保存可通知的变化，但可能遗漏查询之间的变化。除cursor外还跟踪streamId和保留边界。重新注册时重置cursor，恢复较旧stream且sequence较小时也不漏读。历史清理、移除配置与恢复可用也触发重新读取。仅相同报告的时点更新不通知；权限不足或响应无效时不推进cursor。

通知是更新提示，可合并或重放，不提供exactly-once保证。HTTP／SSE重连重放保留提示，进程重启后需重新初始化／订阅。客户端保存streamId与读API的nextAfter，并检查historyTruncated。继续遵守1000事件保留、32订阅上限、认证、会话期限及SSE连接限制。

Validation / 検証 / 验证: session tests cover stream replacement/restoration, pruning, removal/recovery, malformed responses, permissions, URI bounds and unsubscribe. Stdio tests prove unsolicited push and unsubscribe without device calls. An actual MCP HTTP/SSE integration uses a real local mock IPP producer and persistent observation API to verify push, unchanged-report suppression, reconnect replay and unsubscribe. Existing MCP transport/authorization tests remain enabled.

🟠 [Next] Direct REST observation SSE / REST直接SSE / REST直接SSE。

⭕️ [Pending] Real device change cadence and deployed-network disconnect behavior / 実機の変化周期と運用ネットワークの切断挙動 / 实机变化周期及部署网络断连行为。
