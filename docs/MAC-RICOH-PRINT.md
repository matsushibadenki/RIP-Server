# Mac → J-RIP → RICOH

## 日本語

明示的に有効にするローカル自動配送です。MacのPDF → MuPDFのsRGB TIFF → PWG sRGB Raster → Native IPP → 実機完了確認、の順に処理します。実機ICCによる色校正ではありません。

1. `JRIP_PRINTER_CONFIG` に機器とA4を登録します。
2. 秘密情報を含まないローカルJSONを作成します。

```json
{"printerId":"ricoh","printerUri":"ipp://printer.local:631/ipp/print","media":"iso_a4_210x297mm","dpi":360}
```

3. `JRIP_IPP_FORWARD_CONFIG` にその絶対パスを指定し、`--with-printers`で起動します。無人印刷では管理者が明示的に`JRIP_REQUIRE_PRINT_APPROVAL=0`を設定します。承認必須のままなら配送は承認待ちで止まります。自動配送のIPP受付はloopback限定です。
4. Macのキューに `adapters/cups/jrip-ricoh-forward.ppd` を指定します。用紙A4、360dpi、カラー、1部、片面で投入してください。過去のプリセットの300／600dpiは変更が必要です。

配送先は投入時にURIまで保存します。過去のファイル出力ジョブは自動印刷しません。既存のpipeline／deliveryの永続化・重複防止・UNKNOWN停止を使い、再起動を理由に送信済みジョブを再送しません。Macには実機の完了確認まで処理中を返します。実機に用紙がない場合は補充してください。取消は送信前、または受付後の実機取消要求に対応し、送信結果不明の場合は自動再送しません。

初期経路はA4・360dpi・sRGBに限定します。RICOHがその解像度・色型・用紙・PWG形式を広告している必要があります。標準RGBのMuPDF出力をPWG sRGBとして包装し、CMYKをsRGBへ付け替えることはありません。埋め込みICC成果物はこの経路で付け替えません。画素64 MiB／ページ、配送入力合計64 MiB、PWGファイル64 MiBを上限とします。大きい原稿、多ページ写真、両面・部数・他の用紙はこの自動経路の対応外です。

## English

Opt-in, loopback-only physical forwarding: Mac PDF → MuPDF sRGB TIFF → PWG sRGB → Native IPP → device-confirmed completion. Configure an A4 printer and an `IppForward` JSON as above, set `JRIP_IPP_FORWARD_CONFIG`, and start with `--with-printers`. Unattended printing requires explicitly setting `JRIP_REQUIRE_PRINT_APPROVAL=0`; otherwise approval remains required. Install the supplied `jrip-ricoh-forward.ppd` and select A4, 360 dpi, color, one copy, simplex. Existing application presets may retain older resolutions.

Only new intake-time routes are forwarded. The target URI is pinned; existing pipelines/deliveries provide durable, deduplicated dispatch and conservative UNKNOWN handling. Mac reports completion only after the device confirms it. No physical ICC proof claim is made. The initial route supports A4/360 dpi/sRGB only, with 64 MiB decoded samples per page, 64 MiB total delivery input and 64 MiB PWG output limits. Embedded ICC artifacts are not relabelled. Media is verified, not resized. Paper-empty conditions require paper; unknown sends are never automatically retried.

## 简体中文

显式启用且仅接受loopback连接的实体投递：Mac PDF → MuPDF sRGB TIFF → PWG sRGB → Native IPP → 设备确认完成。配置A4打印机及上述JSON，设置`JRIP_IPP_FORWARD_CONFIG`，使用`--with-printers`启动。无人值守打印须显式设置`JRIP_REQUIRE_PRINT_APPROVAL=0`，否则保留审批。Mac队列使用附带的`jrip-ricoh-forward.ppd`，选择A4、360dpi、彩色、1份及单面；已有应用预设可能保留旧分辨率。

仅自动投递启用后新接收并绑定路由的作业，目标URI被固定。使用现有持久队列、防重发及UNKNOWN停止机制；Mac仅在设备确认后显示完成。不声称实体ICC色彩校样。初始路径仅支持A4／360dpi／sRGB：每页解码数据64 MiB、投递输入合计64 MiB、PWG文件64 MiB。不会重新标记嵌入ICC的产物，不缩放原稿。缺纸须补纸，发送结果未知时绝不自动重发。

## Local acceptance status / ローカル受入状況 / 本地验收状态

⭕️ [Pending] 用紙未準備のため実機送信前で停止。ローカル設定は印刷ごとの承認必須とし、自動IPP配送と実機配送workerを無効のまま維持。確認用ページは未送信。

Physical acceptance is paused before sending because paper is unavailable. Local configuration requires approval for every physical print; automatic IPP forwarding and physical delivery workers remain disabled. No test page has been sent.

因未备纸，实机验收停在发送前。本地配置要求每次实体打印均需审批；自动IPP投递与实体投递worker保持关闭，未发送测试页。
