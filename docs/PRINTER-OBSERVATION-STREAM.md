# Printer observation SSE / 機器観測SSE / 设备观察SSE

🟢 [Done] `GET /api/v1/printers/{printerId}/observations/stream`

## 日本語

jobs:readのBearer認証で、保存済みの最新機器状態をSSEで受け取る。登録済み機器だけを対象とし、既存の登録／セキュリティ設定に拘束した観測を読む。購読自体は機器に接続せず、印刷・取消・再送・承認を行わない。状態を記録する既存の明示照会、または[定期観測worker](PRINTER-OBSERVER.md)を別途実行する。

```sh
curl --no-buffer --header "Authorization: Bearer $JRIP_API_TOKEN" \
  http://127.0.0.1:8080/api/v1/printers/test/observations/stream
```

`printer-observation`イベントのJSONはprofile `org.jrip.printer-observation-stream-v1`、printerId、streamId、latest、reasonCode、deliveryMeaningを持つ。latestは既存の[観測履歴](PRINTER-OBSERVATIONS.md)と同じhash検証済みsnapshot・成功時刻・期限・照会失敗を含み、未観測ならnull。isLiveとauthorizationGrantedは常にfalseで、印刷条件の承認には使わない。失敗時は前回の成功snapshotを保ち、成功観測の期限を延長しない。

初回は最新snapshot（または未観測）を送り、その後約1秒間隔で保存履歴を確認する。内容が同じでも新しい完了照会があれば時刻／期限を通知する。複数の照会が確認間隔内に完了した場合は最新へ集約する。全変更の再生、exactly-once配信、観測していない中間状態の証明ではない。厳密な変更履歴は別のcursor履歴APIで取得する。

SSE idは `<streamIdのUUID>:<querySerial>`。event sequenceではない。再接続時はLast-Event-ID、またはURLのafterへこの文字列を渡す。ヘッダーを優先するが、不正なafterも拒否する。同じheadなら重複送信せず、新しいheadがあれば最新だけを送る。設定変更でstreamが異なればSTREAM_CHANGED、cursorがheadより進んでいればCURSOR_AHEADを付けて現在のheadへ戻す。履歴のイベント清掃でheadは失われない。未観測の登録ではNO_REPORTとlatest:null、空idで以前のcursorを消す。

接続数は既存のtimeline／inspection SSEと共有の64件上限。1接続は最新snapshotと最大1件のコンパクト履歴だけを保持し、クライアントの読取に応じて進む。15秒keep-alive、900秒でRECONNECT_REQUIRED、資格情報失効時はAUTHORIZATION_EXPIREDで終了する。認可は送信直前にも再検査する。保存状態を読めないときはOBSERVATION_STREAM_UNAVAILABLEで終了し、内部エラーや接続情報を送らない。切断で接続枠を解放する。private/no-storeとproxy buffering無効化ヘッダーを付ける。ブラウザーの標準EventSourceはBearerヘッダーを指定できないため、fetchストリーム等の認証付きクライアントを使う。tokenをURLへ入れない。

独立パネルへの[自動購読接続](CONSOLE-PRINTER-SUBSCRIPTIONS.md)は実装済み。実機出力欄は引き続き明示照会で更新する。

## English

The jobs:read Bearer endpoint streams stored evidence for a configured, security-bound printer. It never probes a device, prints, cancels, retries or approves. Run existing explicit queries or the periodic observer separately. Each printer-observation event has profile org.jrip.printer-observation-stream-v1, printerId, streamId, latest, reasonCode and deliveryMeaning. The verified latest snapshot includes observation time, expiry and query failure; it is null before observation. isLive and authorizationGranted are false. Failures retain the previous successful snapshot without extending its expiry.

It sends the initial latest head and checks storage about once per second. New completed queries notify even for identical content so freshness can update. Intermediate queries coalesce: deliveryMeaning is coalesced-latest-snapshot, not a lossless log, exactly-once delivery or evidence of unobserved states. Use the separate history API for retained change events.

The SSE id is canonical UUID:querySerial, not event sequence. Last-Event-ID takes precedence over the optional after query; malformed inputs are rejected. Equal heads are suppressed, stale cursors receive the latest head, different registration streams reset with STREAM_CHANGED, ahead cursors reset with CURSOR_AHEAD. NO_REPORT carries latest:null and an empty id. Event pruning does not remove the head.

The shared SSE limit is 64 connections, with one head and at most one compact history event per connection. Backpressure bounds reads; keep-alive is 15 seconds and lifetime is 900 seconds. RECONNECT_REQUIRED asks the client to reconnect. Authorization is checked before emission; expired credentials receive AUTHORIZATION_EXPIRED. Read failures end with OBSERVATION_STREAM_UNAVAILABLE without leaking internal details. Disconnect releases capacity. Responses use private/no-store and disable proxy buffering. Use authenticated fetch streaming or another header-capable client; never put tokens in URLs. [Console automatic subscriptions](CONSOLE-PRINTER-SUBSCRIPTIONS.md) are implemented for the independent panel.

## 简体中文

jobs:read Bearer接口只读取已注册、绑定安全配置的打印机保存证据，不查询设备、不打印、取消、重试或审批。须另行执行明确查询或定期观察worker。printer-observation事件包含profile org.jrip.printer-observation-stream-v1、printerId、streamId、latest、reasonCode与deliveryMeaning。latest含已验证snapshot、成功观察时间、期限与查询失败；未观察时为null。isLive及authorizationGranted始终为false。失败保留上次成功snapshot，不延长期限。

初次发送最新head，此后每秒左右检查存储。即使内容相同，完成新查询后也通知时点／期限。间隔内的多个查询合并为最新状态，不是完整事件重放、exactly-once或未观察中间状态的证明。保留的变化事件通过独立历史API读取。

SSE id是规范UUID:querySerial，不是event sequence。Last-Event-ID优先于可选after，不正确的输入被拒绝。相同head不重复，较旧cursor只返回最新head；注册stream变化时标记STREAM_CHANGED，cursor超前时标记CURSOR_AHEAD。未观察时以NO_REPORT、latest:null与空id清除旧cursor。事件清理不删除head。

与其他SSE共享64连接上限，每连接最多保持一个head与一个紧凑历史事件，通过背压限制读取。15秒keep-alive，900秒后RECONNECT_REQUIRED。发送前重新授权，凭据失效以AUTHORIZATION_EXPIRED结束；存储读取失败以OBSERVATION_STREAM_UNAVAILABLE结束，不泄露内部错误。断开释放容量。响应private/no-store并禁用代理缓冲。使用可设置Bearer头的fetch流客户端，不将token放入URL。独立面板已实现[自动订阅](CONSOLE-PRINTER-SUBSCRIPTIONS.md)。

Validation / 検証 / 验证: initial/empty heads, identical queries, coalescing, failures/recovery, restart, header precedence, registration changes, ahead cursors, scope/expiry, shared capacity, real HTTP delivery/disconnect and zero printer TCP connections.

🟢 [Done] [Console subscriptions and reconnect UI / 操作画面の購読・再接続表示 / 操作界面订阅及重连显示](CONSOLE-PRINTER-SUBSCRIPTIONS.md)。
⭕️ [Pending] Reverse-proxy and real-device deployment acceptance / 実reverse proxy・実機環境の受入 / 实际反向代理与设备环境验收。
