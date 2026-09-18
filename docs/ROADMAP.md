# 実装ロードマップ / Roadmap / 开发路线图

- [Done] implemented in the current codebase / 実装済み / 已实现
- [Next] high-priority unfinished work / 優先して取り組む未完了の作業 / 优先待完成
- [Later] planned, but not the closest next step / 後続の作業 / 后续计划

最初の利用目標は、同じLANのMacからJ-RIPを選択してPDFジョブを投入し、RIP結果をファイルとして確認できること。その後、実機プリンターへ出力する。IPP受付（クライアントからJ-RIPへ）とIPP出力（J-RIPから実機へ）は別の機能として管理する。仕様書のMVP 1 / MVP 2はまだ完成していない。

## 現在の基盤

- [Done] Rust Workspace、Job Core / Storage / Interpreter / Serverの境界。
- [Done] PDF / PS / EPSのREST API受付、入力シグネチャ照合、SHA-256、32 MiB上限の逐次アップロード。
- [Done] SQLite WALによるジョブ永続化、状態変更イベント、revisionによる競合防止、保留・解除・取消API。
- [Done] 共通Bearer認証。アプリケーションエラーの英語・日本語・简体中文メッセージ。
- [Done] PDFはMuPDF、PS/EPSはGhostscriptを既定とし、PDFでのGhostscript明示指定も可能。選択したエンジンをジョブに保存。
- [Done] RGB / CMYK / GrayのTIFFファイル出力、コンテナー隔離、リソース上限、タイムアウト。ワーカーはジョブIDを指定して1件ずつ起動。
- [Done] 完成した出力ディレクトリを一括公開。現在の `COMPLETED` は**TIFFファイル出力完了**を意味し、紙への印刷完了ではない。
- [Done] 2ページPDFの色・ページ順序、実PS変換、上限・タイムアウトの結合テスト。

## [Next] 第1段階：PDFを受けるIPP仮想プリンター

この段階は印刷プロトコルの受付を検証する**ファイル出力用の仮想プリンター**とする。実機出力やIPP Everywhere / AirPrint適合を完了と表示しない。

1. [Next] **受付を共有する。** `crates/rip-ipp` にIPPメッセージ・属性・応答コード・ジョブ変換を置き、`services/jrip-ipp` に `POST /ipp/print` の受信を置く。RESTとIPPは共通のジョブ投入サービス・バリデーション・保管処理を使い、IPPから内部RESTへHTTPで折り返さない。RIP CoreにIPP固有の型を持ち込まない。
2. [Next] **PDF限定の最小プロトコルを動かす。** 最初に `Get-Printer-Attributes`、`Print-Job`、`Get-Job-Attributes`、`Cancel-Job` を実装する。IPPの版・operation-id・request-id・属性グループ・必須属性・文書データ境界を検証し、不正要求や未対応の形式には明確なIPPエラーを返す。受信したPDFは既存のMuPDF経路で処理する。`Create-Job`、`Send-Document`、`Get-Jobs`、`Validate-Job` を後続のクライアント互換テストで必要になる順に追加する。
3. [Next] **能力を正確に公開する。** 当面の `document-format-supported` は `application/pdf` のみ。解像度・カラー・用紙・部数・両面などは、受理して実際に反映できる値だけを `Get-Printer-Attributes` とDNS-SDのTXTに掲載する。属性をmanifestへ変換する際、未対応値を黙って捨てない。`copies`、`media`、`sides` などを提供する前にmanifest・出力経路・検証を揃える。
4. [Next] **ジョブ状態を一貫させる。** IPP job-idと内部UUIDの対応を永続化する。IPPの取消・照会を既存ジョブの状態に結び、投入・RIP・ファイル出力・失敗を返す。仮想プリンターのUIにはファイル出力先と状態の意味を明示する。実機がない段階で紙への印刷成功を返さない。
5. [Next] **LANで発見できるようにする。** 実際に待ち受ける `/ipp/print` とポートをDNS-SD / mDNSで `_ipp._tcp` として広告する。`rp`・`pdl` などのTXTは実装値と一致させる。TLSと証明書を用意した後に `_ipps._tcp` / IPPSを追加する。送信先プリンターの探索は別機能とする。
6. [Next] **受付の安全性を確認する。** 受信サイズ、属性数、入れ子・文字列長、タイムアウト、同時接続数を制限する。IPPS時の認証・アクセス範囲を設計し、既存RESTのBearerトークンをプリンタークライアントへ要求する前提にしない。

**第1段階の完了条件:** IPPプロトコルテストで属性照会・PDF投入・ジョブ照会・取消を検証する。Macの同一LANでサービス発見と手動追加を確認し、1ページ・複数ページPDFが正しい内部ジョブとTIFFへ到達する。Windowsの手動IPP追加・標準クラスドライバーからの印刷も実機で検証し、送られる文書形式がPDFでない場合はその形式の対応を別タスクとして記録する。接続できることとAirPrint / IPP Everywhereへの適合は別に判定する。

## [Next] 第2段階：実機への出力

- [Next] `PrinterBackend` と設定済み出力先を導入する。IPPクライアント受付と、J-RIPから実機へ送信するNative IPPバックエンドを分離する。
- [Next] TIFFからプリンターが実際に受理する形式への変換を実装する。PWG Rasterを優先し、ページ順序、用紙、解像度、カラーモードを能力照合してから送る。
- [Next] キュー取得・スケジューラー・リース・再試行・クラッシュ復旧を実装し、送信完了とプリンター側のジョブ結果を区別する。部数・両面などはバックエンドで反映可能になったものから順に公開する。
- [Next] Mac / Windowsの実機印刷、取消、障害、再起動を通し、クライアント表示と実際の印刷結果が一致することを確認する。
- [Next] IPP Everywhereを名乗る前に、必要なPWG Raster入力と、カラー機で必要なJPEG入力、属性・DNS-SD・セキュリティ要件を満たし、PWGの自己認証テストを実行する。PDFだけの受付では適合を主張しない。

## [Later] 印刷品質と製品機能

- [Later] `Create-Job` / `Send-Document` の複数文書とApple URF入力をクライアント互換性に応じて追加する。未実装の形式を広告しない。
- [Later] 日本語CID/CMap・縦書き・IVSとフォント置換レポート、EPS / 日本語のGolden Master。
- [Later] LittleCMSによるICC変換、黒版保持、DeviceLink、TAC、オーバープリント、分版プレビュー。
- [Later] CMYK16、タイルRIP、メモリ予算、ハーフトーン、DeviceN、特色、Trapping。
- [Later] React管理UI（英語・日本語・简体中文）にRIP・プリンター状態と専門設定を集約する。必要ならWindows Print Support Appを検討する。
- [Later] 実機プリンター探索・能力取得、CUPS連携、PostgreSQL Repository、ロール認可、監査、保持期限と容量管理。
- [Later] Linuxサービス・パッケージ、SBOM／ライセンス監査、必要時のみHTTPSで接続するクラウドControl Plane。

参考規格: [PWG IPP Everywhere](https://pwg.org/ipp/everywhere.html)、[PWG自己認証ツール](https://www.pwg.org/ippeveselfcert/)。Mac / Windowsの実際の対応状況は、対象OSでの結合テストを完了条件とする。

## English / 简体中文

**English:** Next, build a PDF-only IPP virtual printer with DNS-SD discovery and shared job ingestion. Validate real macOS and Windows clients. Add physical printer output and the formats required for IPP Everywhere before claiming conformance. `COMPLETED` currently means TIFF export only.

**简体中文：** 下一阶段实现仅接收PDF的IPP虚拟打印机，通过DNS-SD发现并复用现有作业处理。实际验证macOS和Windows客户端。完成实体打印机输出及IPP Everywhere要求的格式后再声明兼容。当前的 `COMPLETED` 仅表示TIFF文件导出完成。
