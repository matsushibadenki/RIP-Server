# Console printer subscriptions / 操作画面の機器購読 / 操作界面设备订阅

🟢 [Done] Independent printer panel / 独立プリンターパネル / 独立打印机面板。

## 日本語

独立プリンターパネルを開いて機器を選ぶと、保存状態の読取後に[REST SSE](PRINTER-OBSERVATION-STREAM.md)へ接続する。定期観測workerや別クライアントの明示照会が保存した状態・警告・消耗品・観測時刻・照会失敗を、自動反映する。購読は機器照会、印刷、取消、再送、承認を行わず、実機出力欄の能力・review・approvalを変更しない。未観測の登録は未報告として扱う。

「保存報告を購読中」「通知停止・再接続中」等の状態を3言語で表示する。通知経路が切れても前回の報告と時刻を残し、機器照会の失敗とは区別する。期限切れの表示は既存のローカルタイマーで続ける。再接続は1秒から指数的に最大30秒まで待ち、同じ購読内では最後に受理したSSE idをLast-Event-IDへ渡す。無効なデータではcursorを進めない。未観測通知はcursorを消去する。900秒の接続終了にも再接続する。権限拒否では再試行を止め、認証期限切れではセッションを切断して表示を消す。

パネル閉鎖、機器切替、一覧更新、手動の最新照会、切断では旧接続をAbortし、遅い応答をversionで拒否する。機器未選択・閉じたパネルでは購読しない。再度開くと保存状態から読み直す。cursorとBearerはメモリ内だけで、URLやlocalStorageへ保存しない。Bearer付きfetchを使い、セッションAbortと購読Abortを合成する。対象は現行ブラウザーのAbortSignal.any対応環境。SSEのUTF-8分割・CRLF・keep-aliveを処理し、frame／残余bufferは512 Ki UTF-16 code unitsで制限する。

検証：ジョブ0件・読取専用資格情報・模擬IPP・実HTTP／Chrome。外部照会で保存された件数7→9の自動反映、機器への追加照会なし、通知終了→503→cursor付き再接続、パネル閉鎖／切断時のサーバー接続枠解放、切替競合、英語・日本語・简体中文の320／375／430／1440pxを確認。NodeのプロトコルテストでUTF-8分割、CRLF、過大frame、別機器data、403、cursor消去、認証失効、Abortを検証する。実機出力欄は引き続き明示照会で更新する。

## English

Selecting a device in the open independent panel reads stored state, then subscribes to REST SSE. Changes saved by the observer or another explicit-query client update status, notices, supplies, observation time and query failure automatically. The subscription never probes a device or changes print/cancel/retry/approval/review state. Empty registrations remain unreported.

Connection and reconnect status is localized separately from device-query failure. A broken transport retains the previous report/time; local expiry labels continue. Reconnect uses exponential 1–30 second backoff and the last accepted SSE id in Last-Event-ID. Invalid data does not advance it; empty-head notices clear it. The 900-second server lifetime reconnects normally. Forbidden stops retries; authorization expiry disconnects and clears the session display.

Closing the panel, switching devices, reloading registrations, manual refresh and disconnect abort the old stream. Version guards reject late responses. Closed/unselected panels do not subscribe. Reopening starts from stored state. Tokens and cursors stay in memory and out of URLs/storage. Authenticated fetch composes session/subscription Abort signals, requiring current AbortSignal.any-capable browsers. The UTF-8/CRLF/keep-alive parser bounds frames and remaining text to 512 Ki UTF-16 code units.

Tests use a no-job, read-only fixture, fake IPP and real HTTP/Chrome. External queries push queue counts 7→9 without extra UI device probes; reconnect after a stream-end and 503 preserves the cursor. Closing/disconnecting releases server slots. Device races and EN/JA/zh-CN at 320/375/430/1440px are covered. Node tests exercise fragmented UTF-8, CRLF, oversized/wrong-device frames, 403, empty cursors, expiry and Abort. The physical-output panel retains explicit refresh.

## 简体中文

打开独立面板并选择设备后，先读取保存状态，再订阅REST SSE。观察worker或其他明确查询保存的状态、通知、耗材、观察时间与查询失败会自动更新。订阅不查询设备，也不打印、取消、重试或改变输出确认／审批。未观察设备显示未报告。

三语言分别显示连接与重连状态，通知传输失败不等同于设备查询失败。断开时保留上次报告与时间，继续本地过期提示。重连按1至30秒指数退避，并通过Last-Event-ID传递最后接受的cursor；无效data不推进，空head清除cursor。900秒连接到期后正常重连。403停止重试，认证失效断开会话并清除显示。

关闭面板、切换设备、刷新注册、手动最新查询与断开时Abort旧连接，版本检查拒绝延迟响应。关闭或未选择设备时不订阅，再打开从保存状态开始。token及cursor只在内存，不放入URL或本地存储。Bearer fetch合并会话与订阅Abort信号，要求支持AbortSignal.any的当前浏览器。解析UTF-8分片、CRLF与keep-alive，将frame及剩余文本限制为512 Ki UTF-16 code units。

通过无作业、只读凭据、模拟IPP与真实HTTP／Chrome验证外部保存件数7→9自动更新、不额外查询设备、stream-end及503后的cursor重连、关闭／断开释放服务器槽位、切换竞争及三语言320／375／430／1440px。Node测试涵盖UTF-8分片、CRLF、过大或其他设备frame、403、空cursor、认证失效与Abort。实体输出栏仍采用明确刷新。

⭕️ [Pending] Real devices and reverse-proxy deployment acceptance / 実機・reverse proxy配備の受入 / 实体设备及反向代理部署验收。
