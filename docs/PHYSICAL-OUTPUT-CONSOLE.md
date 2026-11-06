# Physical output console / 実機出力画面 / 实体输出界面

🟢 [Done] `/console` で、完了済みRIP成果物の用紙・部数・両面・色成果物・原稿ページ範囲を選び、確認・承認・投入・履歴・取消を操作できます。

## 日本語

成果物のある完了ジョブを選び、「実機出力」を開きます。設定済みプリンターを選ぶと、現在の機器能力と設定の交差から用紙、部数上限、実行可能な片面／両面方式を表示します。原稿サイズと解像度は維持し、自動の拡大縮小は行いません。対応しない寸法・DPI・色・機器状態はサーバーの確認で拒否します。用紙寸法はmmで先に表示し、用紙keywordを併記します。

色変換は、原稿に紐づく既存の完了結果から選びます。一覧は50件ずつ追加取得でき、設定された出力ICC／DeviceLinkに対応する候補の出力色・時刻・profile digestの先頭を表示します。プリンターに色ターゲットがある場合は色結果の選択が必須です。profileの一致は確認APIと投入APIで検査します。画面から新しいICC処理を自動実行することはありません。上部の「原本プレビュー」はRIP成果物です。

配信ページ範囲は原稿の物理ページ位置です。空欄は**生成済みページ全て**。例 `2-3,5`。選択描画した原稿で未生成ページを指定した場合は、サーバーが拒否します。部数は原稿の再RIP回数ではなく、選択済み文書の機器出力部数です。

「出力を確認」はPWG変換・成果物hash・選択範囲・用紙寸法・機器条件を検証し、プリンターへ投入しません。確認結果にプリンター・用紙・部数・とじ方・範囲・色結果・DPIを表示します。入力を変更すると確認結果と承認IDを消去し、再確認を必要とします。言語変更では選択条件を保持します。

承認ポリシーが有効な場合、設定済みcredentialのadminは理由を入力し、自分の主体に対する既存の出力承認APIを実行できます。他の承認者が発行した同じ条件の承認IDも入力できます。承認の対象は原稿revision、配信ID、出力先、用紙、部数、両面、範囲、色成果物へ固定されます。承認だけで投入されることはありません。最後に「印刷」を明示的に押すと、確認したticket digestを添えて配信キューへ登録します。サーバーで権限・承認・成果物・機器能力を再検査します。

出力履歴は原稿別に保存された配信を取得します。キュー受付、送信中、機器受付、機器側完了、取消、拒否、結果未確定を、RIPの「ラスター出力済み」と区別します。開いている画面を約3秒ごとに更新し、非表示中はポーリングを休止します。キュー待機中／機器受付後の取消は現在revisionで要求し、機器側の結果は後続の確認で表示します。再読み込み後は再接続・原稿選択から履歴を取り戻せます。ワーカーは別途起動が必要です。

通信断、HTTP 408、5xxなどで投入結果が不明になった場合、同じ配信IDをGETで確認し、自動でPOSTを再送しません。現在の接続中は原稿を選び直しても未確定IDを保持し、状態確認を優先します。トークン・確認・未確定IDはメモリ内のみで、切断／再読み込み後は永続履歴を参照します。404だけで「未投入」と断定しません。

`jobs:read` で一覧、`artifacts:read` で出力確認、`printers:print` で投入と取消、`admin` で承認を行います。UIの無効化に加え、APIでも毎回検査します。実機操作は明示的に実行する場合だけです。成果物0ページのジョブには出力パネルを表示しません。

## English

The console exposes physical output for completed RIP jobs with artifacts. Live capability intersection supplies configured media, supported copies and executable duplex modes. Exact raster geometry/DPI are retained; no automatic scaling is performed. Existing completed color results can be paged and selected; required targets are checked by the server. Delivery ranges use original physical source positions; blank selects all generated pages. Copies apply at the device, without repeating RIP.

Check output validates without queueing. The visible review binds settings to a ticket digest; changing settings clears the review and grant. When required, a configured admin can issue a reasoned grant for the current identity, or an operator can enter a matching externally issued approval ID. Only explicit Print queues output, with server-side authorization, approval and capability rechecks.

History distinguishes raster export from device acceptance and completion, supports current-revision cancellation and reload recovery, and polls while visible. Ambiguous network/408/5xx outcomes trigger GET recovery of the same delivery ID, never automatic POST retries. Pending IDs survive job reselection within the connection. Tokens and local review state stay in memory; after reconnecting, use persistent source-job history. A 404 alone is not proof of non-submission. Workers run separately.

## 简体中文

完成且有产物的RIP任务可打开实体输出。根据实时能力交集选择纸张、份数及可执行的单双面方式，保持原始尺寸和DPI，不自动缩放。可分页选择既有已完成的色彩转换结果，目标profile由服务器验证。投递页码表示原稿物理页，留空为全部已生成页，份数由设备执行，不重复RIP。

检查输出不会排队。确认结果通过ticket digest绑定条件，修改条件会清除确认及审批ID。按需由配置的admin输入理由，为当前身份发放审批，或输入其他审批者发放的匹配ID。仅明确点击打印才排队，服务器再次验证权限、审批、产物和设备能力。

历史区分光栅导出、设备接受和设备完成，可按当前revision取消并在刷新后恢复。仅在可见时轮询。网络／408／5xx结果不确定时，GET查询同一投递ID，绝不自动重新POST；连接期间切换任务仍保留未确定ID。令牌及本地确认仅保存在内存中，重连后查持久历史。单独404不足以证明未提交。Worker仍需单独运行。

## History API / 履歴API / 历史API

```text
GET /api/v1/jobs/{jobId}/deliveries?limit=50
GET /api/v1/jobs/{jobId}/deliveries?limit=50&before={nextBefore}
```

`jobs:read`; default 50, maximum 100. Newest-first creation-order cursor; response `deliveries`, `hasMore`, `nextBefore`. Parent must exist. Errors are 400 `invalid_delivery_query`, 401/403 or 404 as applicable. Responses are `no-store` with `Vary: Authorization`. Payloads contain delivery metadata/tickets, not raster frames or printer credentials. MCP: `print_list_deliveries(jobId, limit?, before?)` is read-only and validates the cursor before dispatch.

## Verification / 検証 / 验证

```sh
cargo test -p jrip-server --test delivery --test console --locked
cargo test -p jrip-mcp --test protocol --locked
# Requires Playwright Chromium and jrip-color:local; uses only an isolated mock IPP printer.
cargo test -p jrip-server --test delivery operator_console_physical_delivery --locked -- --ignored
```

実ICCワーカー、実HTTP、Chromium、模擬Native IPPで、用紙・複数部・両面・ページ範囲・色結果の拘束、未投入preview、承認なし拒否、設定変更時の再確認、機器受付と取消、再読み込み復旧、応答消失／502後の照会復旧とPOST回数、read-only権限、3言語・320／375／430pxを検証します。通常の原稿投入と両RIPエンジンの画面検証も回帰対象です。

⭕️ [Pending] 実機の用紙・排紙・両面方向・取消・障害復旧の受入検証。開発環境の模擬IPP試験は実際の紙出力を証明しません。
