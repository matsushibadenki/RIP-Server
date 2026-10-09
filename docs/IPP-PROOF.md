# PDF IPP proof printer / PDF IPP仮想プリンター / PDF IPP虚拟打印机

## 日本語

このサービスはPDFをTIFFへ変換するファイル出力用仮想プリンターです。IPPの完了状態はTIFF出力完了を示します。紙への印刷やAirPrint / IPP Everywhere適合を意味しません。

Docker Desktopを起動し、`jrip-mupdf:local` をビルド済みにしてください。`JRIP_API_TOKEN` に24文字以上のトークンを設定し、`bash scripts/start-proof-printer.sh` でAPI、IPP受付、Dispatcherをまとめて起動します。Ctrl-Cで停止します。既定はローカルAPI :8080、IPP :8631、出力root `./data`。起動失敗は `JRIP_DATA_ROOT/logs` へ記録します。実機Delivery Workerは起動しません。

LAN検証では以下を設定してから起動します（IPはサーバーのLANアドレスへ置換）。

```sh
export JRIP_IPP_LISTEN=0.0.0.0:8631
export JRIP_PRINTER_URI=ipp://192.168.200.12:8631/ipp/print
export JRIP_ALLOW_INSECURE_LAN=1
export JRIP_MDNS=1
bash scripts/start-proof-printer.sh
```

ブラウザーで `http://SERVER:8631/` を開くと、英語・日本語・简体中文でファイル出力先と機能制限を確認できます。成果物は `JRIP_DATA_ROOT/jobs/<UUID>/raster/page-000001.tiff` 以降に保存します。認証付きPrint APIのジョブ一覧・成果物APIから取得できます。

Mac標準CUPSクライアントでの確認：

```sh
ipptool -tv -f tests/corpus/colors.pdf ipp://SERVER:8631/ipp/print tests/corpus/ipp-pdf-proof.test
```

PDFのみ、300/600 dpi、CMYKカラー／Grayモノクロ。原稿のページ寸法と順序を保持します。1部・片面・1面付け・仕上げなしだけを受理し、それ以外の値と用紙変更は拒否します。Print-Job / Validate-Job / Get-Printer-Attributes / Get-Jobs / Get-Job-Attributes / Cancel-Jobを実装。PDF最大32MiB、属性領域16KiB、128属性、属性名255byte、属性値4096byte、受付同時4要求、要求全体30秒。過負荷はHTTP503、時間切れは408です。暗号化・認証なしのLAN受付は明示的な許可が必要です。IPPS導入時は証明書とクライアント認証／ネットワークアクセス制御を組み合わせ、REST Bearerを一般プリンタードライバーへ要求する方式には依存しません。

Dispatcherは異常終了・期限切れリースをFAILEDへ確定し、その原稿パスとJ-RIP専用ラベルの両方が一致する残存コンテナーだけを回収します。原稿が失敗した場合は診断を確認して既存の認証付きretry APIで新attemptを作成します。自動再試行は現段階では有効にしません。破損PDFの無限再処理や設定変更を避けるため、上位の再試行ポリシーは別工程として残します。

検証コマンド：`cargo test --locked --workspace`。Docker・ipptoolがある環境では `cargo test --locked -p jrip-server --test ipp_e2e -- --ignored` で独立IPPクライアント→実HTTP→Dispatcher→MuPDF→TIFF、1/2ページ、再起動後のID対応、原稿単位のコンテナー回収を検証します。

Mac標準CUPSキューを専用PPDで追加し、システム設定の一覧・種類表示、標準lpからのPDF投入、原稿SHA-256一致、2ページTIFF出力、CUPS側の完了を確認済みです。別Mac・アプリごとの印刷ダイアログ・Windows標準ドライバーは別の受入試験です。手動追加先はサーバーIP、IPP、キュー `ipp/print`。ドライバーがPDF以外（PS、PWG、URFなど）を送る場合は本サービスでは拒否し、クライアント互換対応として記録します。現在の能力でAirPrintやIPP Everywhereドライバーの動作を保証しません。

### MacのPDFパススルー定義

現時点のPDF限定サービスには、`adapters/cups/jrip-pdf-proof.ppd` を使用します。Macのプリンター追加で「使用」から「その他」を選び、このPPDを指定してください。CUPS CLIで追加する場合：

```sh
lpadmin -p JRIP_PDF_Proof -E -v ipp://SERVER:8631/ipp/print \
  -P "$PWD/adapters/cups/jrip-pdf-proof.ppd" -D "J-RIP PDF Proof" \
  -o printer-is-shared=false
lp -d JRIP_PDF_Proof tests/corpus/colors.pdf
```

PPDのA4／LetterはアプリがPDFを生成するためのページ定義です。サーバーの用紙変更・拡縮機能ではありません。既存PDFはフィルターなしで送られ、バイト列の一致を確認済みです。1部・片面のみです。

Mac CUPS 2.3.4からのdocument-name-supplied、job-originating-host-name、job-uuidは情報として受理し、認証identityには使いません。PDFパススルーで観測したdocument-formatの2重送信は、同じPDF指定、またはPDF指定＋octet-streamヒントで実データがPDF署名を持つ場合だけPDFへ正規化します。異なる具体的形式・3重指定・単独octet-streamは拒否します。

```sh
python3 scripts/test-native-cups.py
```

上記テストはローカルの一時CUPSキューを登録し、完了確認後に削除します。Docker・ビルド済みバイナリ・ローカルCUPSへの管理権限が必要です。PPDはCUPSの旧方式であり、将来の廃止対象です。`-m everywhere` は現在の用紙能力不足で生成に失敗するため、driverless対応完了とは扱いません。参照：[CUPS lpadmin](https://openprinting.github.io/cups/doc/man-lpadmin.html)、[Apple CUPS PPD生成実装](https://github.com/apple/cups/blob/master/cups/ppd-cache.c)。

## English

A file-output virtual printer accepting PDF and exporting ordered TIFF pages, without paper output or AirPrint / IPP Everywhere claims. Set a 24+ character `JRIP_API_TOKEN`, start Docker with the MuPDF image available, and run `bash scripts/start-proof-printer.sh`. API, IPP and dispatcher start together; logs live under `JRIP_DATA_ROOT/logs`. The physical delivery worker stays separate. Visit port 8631's root page for output-location information. LAN access requires an explicit opt-in and a matching reachable printer URI. Generic DNS-SD advertises PDF only. Limits: 32 MiB PDF, 16 KiB attributes, 128 attributes, 255-byte names, 4096-byte values, four concurrent requests, 30-second request timeout. Supported settings are 300/600 dpi and CMYK/Gray. Only one copy, simplex, one-up and no finishing are accepted; other values and media changes are rejected. Abandoned containers are removed only when both J-RIP ownership and source labels match failed worker jobs. Retries remain explicit, authenticated, immutable attempts. The independent ipptool/Docker test covers one/two-page TIFF exports and durable IDs. Mac native CUPS registration using the supplied PDF passthrough PPD, visibility in System Settings, unchanged source SHA-256, two TIFF pages and CUPS completion were verified. Run `python3 scripts/test-native-cups.py` for a disposable local queue test. PPD page sizes define application-generated PDF pages, not server resizing. The bounded PDF/octet-stream pair emitted by macOS CUPS is normalized only with PDF magic; concrete format conflicts are rejected. Driverless setup, application-specific print dialogs and Windows driver acceptance remain unfinished; non-PDF formats are unsupported. IPPS certificate and client/network access policies remain a subsequent transport milestone.

## 简体中文

本服务仅将PDF转换为按顺序排列的TIFF，不输出纸张，不宣称AirPrint或IPP Everywhere兼容。设置至少24字符的 `JRIP_API_TOKEN`，启动Docker并准备MuPDF镜像，再运行 `bash scripts/start-proof-printer.sh`。API、IPP和调度器共同启动，日志位于 `JRIP_DATA_ROOT/logs`，实体发送Worker不会启动。8631端口首页提供三语输出位置说明。LAN接收需显式允许，IPP地址必须可达；通用DNS-SD仅发布PDF能力。限制：PDF 32MiB、属性区域16KiB、128项属性、名称255字节、值4096字节、同时4项请求、30秒超时。支持300/600 dpi及CMYK/Gray，仅接受一份、单面、每页一面及无后处理，拒绝其他值及纸张变更。只回收J-RIP所有权及原稿标签同时匹配失败任务的容器。重试由认证API显式创建新attempt。独立ipptool/Docker测试覆盖单页/双页TIFF及持久化ID。已验证使用专用PDF透传PPD注册Mac CUPS队列、系统设置中的显示、原稿SHA-256一致、双页TIFF及CUPS完成状态。运行 `python3 scripts/test-native-cups.py` 可创建并清理临时测试队列。PPD纸张选项只定义应用生成PDF的页面，不代表服务器缩放。仅对带PDF签名的PDF/octet-stream双属性提示作限定归一化，拒绝具体格式冲突。无驱动安装、各应用打印对话框及Windows驱动仍需独立验收；非PDF格式不受支持。IPPS证书及客户端／网络访问策略属于后续传输阶段。

References: [IPP semantics, RFC 8011](https://www.rfc-editor.org/rfc/rfc8011.html), [CUPS ipptool](https://openprinting.github.io/cups/doc/man-ipptool.html).

## Portable client acceptance / 端末からの受入確認 / 客户端验收

Windows・Mac・LinuxでPython 3がある場合、OSのプリンター設定を変更せず、次のクライアントでプロトコル経路を確認できます。既定は能力照会のみです。PDF投入時はサーバーのDispatcherを起動してください。出力はTIFFで、用紙出力ではありません。

With Python 3 on Windows, Mac or Linux, these commands check direct IPP without changing installed printers. The default only queries capabilities. PDF completion requires a running dispatcher and produces TIFF files. This does not validate an OS standard printer driver.

Windows、Mac或Linux安装Python 3后，可在不修改打印机配置的情况下检查直接IPP通信。默认仅查询能力。PDF完成测试需要运行调度器，输出为TIFF。本测试不代表系统标准驱动验收成功。

```sh
python scripts/check-ipp-client.py ipp://SERVER:8631/ipp/print
python scripts/check-ipp-client.py ipp://SERVER:8631/ipp/print --pdf tests/corpus/colors.pdf --report completion.json
python scripts/check-ipp-client.py ipp://SERVER:8631/ipp/print --pdf tests/corpus/colors.pdf --cancel --report cancellation.json
```

レポートは能力、IPP job-id、投入PDFのSHA-256、終端状態を記録し、`standard_driver_tested: false` を明示します。タイムアウト時はjob-idを表示し、再投入しません。取消テストで取消するのは今回投入したジョブのみです。処理が先に完了した場合は取消成功とは扱いません。URIは暗号化しない `ipp` のため、信頼できる検証ネットワークで使用してください。

Reports include capabilities, IPP job ID, submitted PDF SHA-256 and terminal state, with `standard_driver_tested: false`. Timeouts identify the job without resubmitting. Cancellation targets only the job created by this invocation; a job that completes first does not pass cancellation acceptance. Use the cleartext IPP endpoint on a trusted test network.

报告记录能力、IPP任务ID、提交PDF的SHA-256及最终状态，并标明 `standard_driver_tested: false`。超时不会自动重新提交。取消仅针对本次创建的任务；若任务已先完成，不视为取消成功。明文IPP仅用于可信测试网络。

### Application print-dialog acceptance / アプリの印刷ダイアログ / 应用打印对话框

```sh
python3 scripts/test-native-cups.py --manual-submit --submit-timeout 180 --report application-print.json
```

日本語：表示された `JRIP_Proof_Test_...` キューをアプリの印刷ダイアログで明示的に選び、表示されたテストPDFを全2ページ・1部で印刷してください。自動でlpを実行せず、今回の投入を待ちます。受信PDFの署名と保存hash、ジョブの完了、2ページTIFF、CUPS側の完了を確認し、一時キューを削除します。アプリがPDFを再生成すると原稿hashはfixtureと異なるため、一致を強制せずレポートに記録します。画像内容の視覚的な一致は別途確認してください。UI接続がタイムアウトしたため、このモードでの実アプリ受入はまだPendingです。

English: Explicitly select the displayed `JRIP_Proof_Test_...` queue in your application's print dialog and print all two fixture pages, one copy. The helper waits without running lp, verifies the received PDF signature/hash, RIP completion, two TIFF pages and CUPS completion, then removes the queue. Applications may regenerate PDF, so fixture equality is reported rather than required. Inspect visual content separately. Actual application acceptance remains Pending because UI access timed out.

简体中文：在应用打印对话框中明确选择显示的 `JRIP_Proof_Test_...` 队列，打印测试PDF的全部两页、一份。脚本不会自动调用lp，而是等待提交，验证收到的PDF签名及hash、RIP完成、两页TIFF和CUPS完成，最后删除临时队列。应用可能重新生成PDF，因此仅记录与测试文件是否一致，不强制hash相同。图像内容需另行目视确认。由于UI连接超时，实际应用验收仍为Pending。

### IPP job hold and release / IPPジョブ保留・再開 / IPP任务暂停与恢复

日本語：`Hold-Job`（0x000c）と `Release-Job`（0x000d）を追加しました。待機中のジョブを無期限に保留し、再開すると待機列へ戻します。保留状態は再起動後も残り、REST／MCPから見える状態と共通です。保留済みジョブへの再保留は成功します。処理開始後・終端状態の保留、保留されていないジョブの再開は `client-error-not-possible` です。時刻指定の保留と投入時の `job-hold-until` は未対応です。信頼できるローカル／検証ネットワークを前提とする現在のIPP受付では、要求のユーザー名による所有者認証は行いません。

English: `Hold-Job` (0x000c) and `Release-Job` (0x000d) now hold queued jobs indefinitely and return held jobs to the queue. State survives restart and is shared with REST/MCP. Repeated holding succeeds. Holding processing/terminal jobs or releasing a job that is not held returns `client-error-not-possible`. Timed holds and creation-time `job-hold-until` remain unsupported. The current trusted local/test-network endpoint does not authenticate ownership from a supplied username.

简体中文：新增 `Hold-Job`（0x000c）和 `Release-Job`（0x000d），可无限期暂停排队任务并将已暂停任务恢复到队列。状态在重启后保留，并与REST／MCP共享。重复暂停成功；暂停处理中或已结束任务、恢复未暂停任务返回 `client-error-not-possible`。暂不支持定时暂停和提交时的 `job-hold-until`。当前可信本地／测试网络接口不根据请求用户名验证任务所有者。

Operation semantics: [RFC 8011, Hold-Job and Release-Job](https://www.rfc-editor.org/rfc/rfc8011.html#section-4.3.5).
