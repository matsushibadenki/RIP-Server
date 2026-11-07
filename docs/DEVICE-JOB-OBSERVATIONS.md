# Device job observations / 機器ジョブ状態 / 设备作业状态

## 日本語

🟢 [Done] Native IPPで受付済みのDeliveryは、`deviceObservation`（profile `org.jrip.device-job-observation-v1`）に機器のjob-state、job-state-reasons、変更を観測したUNIX秒を保存する。状態3=待機、4=保留、5=印刷中、6=停止の間はDelivery全体をACCEPTEDに保ち、7=取消、8=中止、9=完了で終端へ進む。ページ進捗や印刷品質は推測しない。

理由は最大64件、各255バイトまでの空でない制御文字なし文字列で、ソート・重複除去する。同じ状態・理由・進捗カウンターの再照会ではrevision、イベント、時刻を更新しない。時刻は最終変更の観測時刻であり、稼働確認のheartbeatではない。通信失敗では前回記録を保持し、オンラインとは断定しない。

機器job-idと期待revisionを照合し、状態変更とイベントを同一トランザクションで保存する。終端のresult receiptには最後の観測値を含めてhashを計算する。APIのDelivery詳細・履歴、およびそれを取得するMCPから参照でき、pipelineイベントのdetailsにも含まれる。過去の記録や手動UNKNOWN解決の結果には観測値がない場合がある。

操作画面は状態・理由・記録時刻を表示する。既知理由は翻訳と原コードを併記し、未知のベンダー理由も原コードを保持する。取消要求は状態照会で消去しない。UNKNOWNを送信可能に戻したり、停止を理由に再印刷したりしない。

検証：ストレージ試験で永続化、重複抑止、競合revision、異なるjob-id拒否、取消要求保持、receiptのhashを確認。模擬IPPのpipeline試験で印刷→停止→再開→完了と送信1回を確認。

⭕️ [Pending] 実機固有の理由・通信断・ジョブ保持期間・実際の紙面は実プリンターでの受入検証が必要。

## English

🟢 [Done] Accepted Native IPP deliveries persist `deviceObservation`, profile `org.jrip.device-job-observation-v1`: numeric IPP job state (3 pending, 4 held, 5 processing, 6 stopped, 7 cancelled, 8 aborted, 9 completed), reasons and a UNIX timestamp. States 3–6 retain ACCEPTED; 7–9 become terminal. No page progress or appearance is inferred.

Reasons are sorted and deduplicated, limited to 64 nonempty strings of at most 255 bytes without control characters. Samples with unchanged state, reasons and progress counters do not advance revisions, events or timestamps. The timestamp records an observed change, not a heartbeat; failed polling leaves the previous sample intact.

Job-id and revision checks protect updates. State, event and terminal receipt are transactional; the receipt hash includes the final observation. Delivery detail/history and their MCP readers expose the metadata; pipeline event details retain it. Legacy records and manual UNKNOWN reconciliation may omit it. Cancellation requests survive polling. No automatic resend or UNKNOWN recovery is introduced.

The console shows state, translated known reasons with raw codes, unknown raw reasons, and observation time. Storage tests cover durability, deduplication, concurrent updates, invalid identity, cancellation and receipt hashing. The fake IPP pipeline tests printing, stopped, resumed and completed states with one submission.

⭕️ [Pending] Real-device reasons, disconnections, retention and printed appearance require hardware acceptance tests.

## 简体中文

🟢 [Done] Native IPP已接收的投递持久化`deviceObservation`，版本为`org.jrip.device-job-observation-v1`，包含设备状态、原因及观察到变化的UNIX时间。3等待、4保留、5打印、6停止时维持ACCEPTED；7取消、8中止、9完成时终结。不推测页面进度或纸面质量。

原因最多64条，每条非空、不含控制字符且不超过255字节，排序并去重。相同状态、原因和进度计数不增加revision、事件或时间。时间表示变化的观察时刻，并非心跳；通信失败时保留上次记录。

更新检查设备job-id和revision，状态、事件与终结receipt以事务保存，receipt的hash包含最后观察值。Delivery详情、历史及相应MCP读取可获取记录，pipeline事件details也保留记录。旧数据或手动UNKNOWN协调可能没有此字段。轮询保留取消请求，不自动重发或恢复UNKNOWN。

界面显示状态、已知原因的翻译和原代码、未知原因原代码以及记录时间。存储测试覆盖持久化、去重、并发冲突、身份拒绝、取消及receipt hash；模拟IPP pipeline覆盖打印、停止、恢复和完成，并确认仅提交一次。

⭕️ [Pending] 实际设备原因、断线、保留期限及纸面质量需要实体打印机验收。

## Bounded polling / 照会範囲 / 有界查询

🟢 [Done] Get-Job-Attributesは`requested-attributes`に`job-id`、`job-name`、`job-state`、`job-state-reasons`と進捗2属性（[DEVICE-PROGRESS.md](DEVICE-PROGRESS.md)）を明示する。全属性の既定応答を避けるが、応答64KiB上限は維持する。機器／ジョブ理由は受信時にも最大64件・各255バイト・非空・制御文字なしを検査し、不正応答は前回の観測を上書きしない。

🟢 [Done] Get-Job-Attributes explicitly requests only job-id, job-name, job-state and job-state-reasons and two optional progress counters instead of the default all-attributes response. The 64KiB reply limit remains enforced. Printer/job reason sets are checked at the backend boundary for 64 entries, 255 bytes each, nonempty and no control characters. Invalid responses leave previous observations intact. Fake IPP tests inspect the wire request and cover empty, oversized, control-character, excessive-count and wrong-type reasons, while preserving unknown vendor reasons.

🟢 [Done] Get-Job-Attributes显式请求job-id、job-name、job-state和job-state-reasons，以及两个可选进度计数器，避免默认返回全部属性，同时保留64KiB响应上限。设备与作业原因在接收边界检查：最多64条、每条255字节、非空且不含控制字符。不合法响应不覆盖前次记录。模拟IPP验证请求和无效原因，并保留未知厂商原因。

Protocol reference: [RFC 8011 §4.3.4 Get-Job-Attributes](https://www.rfc-editor.org/rfc/rfc8011.html#section-4.3.4). These are local resource limits, not a claim of full IPP Everywhere conformance.

## Unified execution snapshot / 共通実行状態 / 统一执行快照

🟢 [Done] `GET /api/v1/pipelines/{id}/ticket`とMCP `print_get_pipeline_ticket`は、`execution.deviceObservation`で同じ観測値を返す。未観測・旧記録ではnull。契約本体の`ticket`には含めず、状態変化で承認対象の`ticketSha256`やportableDigestを変更しない。SQLiteの同一読取snapshotでDeliveryとpipelineを読むが、pipeline workerの反映前には`pipelineState`とDeliveryの状態に時間差がある。`completed`は既存通り両方の完了と確定契約を要求する。

🟢 [Done] The unified ticket REST endpoint and `print_get_pipeline_ticket` expose the sample in `execution.deviceObservation`, null before observation or for legacy records. It is execution evidence outside the immutable contract: changes do not alter ticketSha256 or portableDigest and never grant authorization. The read uses one SQLite snapshot, but pipeline orchestration may lag the delivery worker; completed still requires both completed states and a finalized physical contract. Integration tests cover printing/stopped/resumed/completed samples, null before observation, and invariant contract digests.

🟢 [Done] 统一ticket REST与MCP在`execution.deviceObservation`中返回观察记录，未观察或旧记录为null。该字段位于不可变契约之外，状态变化不改变ticketSha256或portableDigest，也不授予权限。SQLite同一读取快照包含pipeline与Delivery，但协调worker可能滞后；completed仍要求两个状态均完成且实体契约确定。集成测试覆盖打印、停止、恢复、完成、未观察的null及契约摘要不变性。

The observation timestamp and deduplication now include optional device counters; see [DEVICE-PROGRESS.md](DEVICE-PROGRESS.md). / 観測時刻と重複判定には機器カウンターの変化も含む。 / 观察时间与去重也包括设备计数变化。

Query failure/recovery is recorded separately without discarding the last observation: [DEVICE-QUERY-STATUS.md](DEVICE-QUERY-STATUS.md). / 照会失敗と復旧は前回観測と分離して記録する。 / 查询失败与恢复独立记录，保留前次观察。
