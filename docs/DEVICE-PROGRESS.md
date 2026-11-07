# Device progress counters / 機器進捗カウンター / 设备进度计数器

## 日本語

🟢 [Done] Native IPPのGet-Job-Attributesで`job-impressions-completed`と`job-media-sheets-completed`を明示要求し、受付済みDeliveryの`deviceObservation.progress`に`impressionsCompleted`と`mediaSheetsCompleted`を保存する。RESTのDelivery詳細・履歴、共通ticketの`execution.deviceObservation`、既存MCP読取、pipelineイベントdetails、終端receiptに反映する。カウンター変化は状態が同じでもrevisionとイベントを更新し、同一観測は重複保存しない。

impressionは用紙の片面に配置された内容、media sheetは用紙1枚を表す。原稿ページ数と一致するとは限らず、両面・面付け・部数から完了率や残り枚数を推測しない。用紙カウンターは機器がマーキングと排紙を完了した枚数の報告であり、本システムによる紙面品質確認ではない。

各値は0〜2147483647の単一IPP整数。欠落・空unknownは未観測として省略し、0は報告があった場合だけ保存する。片方のみ報告される場合も扱う。負数・型違い・重複・長さ不正は応答全体を拒否し、前回記録を保持する。値の後退も機器報告をそのまま記録し、最大値への補正や成功判定には使わない。完了状態は従来通りjob-stateで確定する。

観測時刻は状態・理由・カウンターの変化を観測した時刻。未対応カウンターに架空の0を付けない。過去のprogressなし記録はそのまま読め、空progressは省略する。カウンターを契約本体へ含めず、承認対象のdigestは不変。

模擬IPP試験で0・片方のみ・未知・上限・不正属性、ストレージ試験で件数のみの更新・重複抑止・再起動・終端receipt、サーバー試験で共通ticketへの実応答の伝播を確認。

⭕️ [Pending] 実機のimpression／用紙カウント、途中取消・jam・再開時の挙動、実際の排紙との一致は実プリンターが必要。

## English

🟢 [Done] Native IPP explicitly requests job-impressions-completed and job-media-sheets-completed. Accepted deliveries persist them in `deviceObservation.progress` as impressionsCompleted and mediaSheetsCompleted. Delivery REST/history, unified execution snapshots, existing MCP readers, printer event details and terminal receipts carry the sample. Counter-only changes advance revision/events; identical samples do not.

An impression is content imposed on one sheet side; a media sheet is one physical sheet. Neither is assumed to equal source pages. No percentages, remaining pages or copy/duplex-based estimates are inferred. Sheet counts are the device's marked-and-stacked report, not verified printed appearance. Job-state still determines terminal outcome; raw counter regressions are retained rather than clamped or treated as success.

Counters accept one integer from 0 through 2147483647. Missing or empty unknown values are omitted, never synthesized as zero; either counter can be independently available. Negative, malformed, wrong-type or duplicate values invalidate the response, leaving the previous snapshot intact. Progress changes participate in the observation timestamp. Empty progress preserves legacy encoding; immutable contract digests remain unchanged.

Tests cover wire requests, zero/unknown/one-sided availability, limits and malformed replies, counter-only updates, no-op polling, reopening the DB, terminal receipt binding and unified ticket propagation.

⭕️ [Pending] Hardware acceptance must compare reported counters with actual output during cancellation, jams and resumption.

## 简体中文

🟢 [Done] Native IPP显式请求job-impressions-completed及job-media-sheets-completed，持久化到`deviceObservation.progress`的impressionsCompleted与mediaSheetsCompleted。Delivery REST及历史、统一执行快照、现有MCP读取、事件details和终结receipt保留记录。仅计数变化也更新revision与事件，相同观察不重复保存。

impression为纸张一面的内容，media sheet为一张纸，均不等同于原稿页数。不根据份数、双面或拼版推测百分比或剩余页数。纸张计数是设备报告的标记和堆叠完成数，不代表系统验证纸面质量。终结仍由job-state决定；计数回退保留原报告，不补正为最大值或据此判定成功。

接受0至2147483647的单个整数，缺失或空unknown省略，不生成虚假0；两个计数可独立存在。负数、类型错误、重复或长度错误拒绝整个响应，保留前次观察。计数变化更新观察时间，旧记录兼容，空progress省略，契约摘要不变。

模拟IPP与存储、服务器测试覆盖请求、0与未知、单项可用、边界及无效响应、仅计数更新、去重、重开数据库、receipt与统一ticket。

⭕️ [Pending] 实际设备取消、卡纸、恢复时的计数与真实排纸一致性需要实体设备。

Reference: [RFC 8011 §5.3.18](https://www.rfc-editor.org/rfc/rfc8011.html#section-5.3.18).

## Operator console / 操作画面 / 操作界面

🟢 [Done] `/console`の出力履歴に完了印刷面数・完了用紙枚数を表示する。機器が報告した0は表示し、未対応の値は行ごと省略する。単位は既存の情報アイコンのtooltipで説明する。原稿ページ進捗や百分率へ変換しない。

🟢 [Done] The console output history shows completed impressions and completed sheets independently. Reported zero is visible; unsupported counters are omitted. The existing information icon explains units without adding persistent explanatory text. No source-page percentage is shown. Real API + fake IPP browser tests cover zero/unknown/one counter, EN/JA/zh labels and tooltip, desktop and 320/375/430px layouts, state changes and cancellation without resend.

🟢 [Done] 操作界面输出历史分别显示已完成印面数及纸张数，显示设备报告的0，省略未支持项。现有信息图标的tooltip解释单位，不生成原稿页百分比。真实API与模拟IPP浏览器测试覆盖未知、0、单项计数、三语言标签与tooltip、桌面及320/375/430px、状态变化和不重发的取消。
