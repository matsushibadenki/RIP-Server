# 実装ロードマップ / Roadmap / 开发路线图

状態: [Done] implemented in the current codebase / 実装済み / 已实现
[Next] high-priority unfinished work / 次の優先作業 / 下一步优先工作
[Later] planned, but not the closest next step / 将来作業 / 后续计划

仕様書 §109 の優先順位に沿って、空のリポジトリから初期基盤を構築しています。
以下のDoneは個別機能の実装状態であり、仕様書のMVP 1完成を意味しません。

## 初期基盤

- [Done] Rust Workspace、Core / Storage / Interpreter / Serverの境界。
- [Done] Job manifest、入力シグネチャ照合、SHA-256、32 MiBの逐次アップロード。
- [Done] SQLite WALによるジョブ永続化、状態変更イベント、revisionによる競合防止。
- [Done] ジョブ投入・一覧・詳細・保留・解除・取消・終端メタデータ削除API。
- [Done] 共通Bearer認証、アプリケーションエラーの英語・日本語・简体中文メッセージ。
- [Done] MuPDF / Ghostscriptハイブリッド。PDFはMuPDF、PS/EPSはGhostscript。PDFの明示Ghostscript指定と選択エンジンの永続化。
- [Done] RGB/CMYK/Gray TIFF出力。MuPDF PAMのサンプルを維持したTIFF格納と、共通のコンテナー隔離実行基盤。
- [Done] 明示起動の単一ジョブワーカー。
- [Done] コンテナーのネットワーク／リソース制限、タイムアウト後のコンテナー削除、上限付き出力回収。
- [Done] 完成出力ディレクトリのatomic rename。COMPLETEDはファイル出力完了を表す。
- [Done] Canonical Rasterのサイズ／予算／オーバーフロー検証。ラスター処理エンジン自体は未実装。
- [Done] 状態・永続性・API・実PostScript変換・ページ数上限・タイムアウトのテストとCI定義。

## 最優先の未完成機能

- [Next] 実ラスターの読み書き、CMYK16、タイル単位の処理と共通メモリ予算。
- [Next] PWG RasterとPrinterBackend、Native IPP送信、実機の能力照合。
- [Next] 日本語CID/CMap、横書き・縦書き・IVSの検証コーパス、必須フォントの不足／置換レポート。
- [Next] LittleCMSによる独立したICC変換、プロファイルとエンジンバージョンの再現性記録。
- [Next] 自動キュー取得、リース、即時取消、ワーカークラッシュからの安全な復旧。
- [Next] Re-RIP / Re-Spool、原稿・ラスターの保持期限、全体ディスク上限と孤立ファイルの回収。
- [Done] 2ページPDFのMuPDF RGB/CMYK/Gray変換、独立TIFFデコーダーでの検証、RGBピクセル／ページ順序確認、PDFの明示Ghostscript変換。
- [Next] EPS / 日本語の実変換回帰、複雑なPDFのエンジン間比較、フォント／色のGolden Master。
- [Next] エンジンバージョン・イメージdigestの保存、ICC／オーバープリント差異の評価、PAM一時保存容量の削減。

## 後続の製品機能

- [Later] PostgreSQL Repository、監査ログの拡充、ロール認可、OIDC/LDAP。
- [Later] IPP/IPPSサーバー、プリンター探索、CUPS連携。
- [Later] React管理UI（英語・日本語・简体中文）、プリフライト、プレビュー。
- [Later] 黒版保持、TAC、DeviceLink、オーバープリントと分版プレビュー。
- [Later] ハーフトーン、DeviceN、スポットカラー、Trapping、メーカー別Backend。
- [Later] ネイティブLinux隔離、systemd、SBOM／ライセンス監査、配布パッケージ。
- [Later] 必要時のみHTTPSで疎結合するクラウドControl Plane。印刷処理はLAN内で完結。
