# RIP-Server

[開発仕様書](docs/RIP-Server開発仕様書.md)に基づく、ローカル／エッジ向けRIPサーバーの初期実装です。
現在は **PDF / PS / EPSの受付 → 永続ジョブ管理 → 隔離MuPDF / Ghostscript → TIFFファイル出力** を提供します。
仕様書全体のMVP 1はまだ完成していません。実機への印刷、PWG / IPP、独立したICC処理、Web UIは未実装です。

## 起動

Rust（rustup）、Docker（TIFF変換時）が必要です。API自体はDockerへのアクセスを必要としません。
`rust-toolchain.toml` でRust 1.98.0とClippy／rustfmtを指定し、ローカルとCIのチェック条件を揃えています。CIのOSはUbuntu 24.04です。

```sh
cargo build --workspace --locked
docker build -t jrip-ghostscript:local adapters/ghostscript
docker build -t jrip-mupdf:local adapters/mupdf
export JRIP_API_TOKEN='replace-with-a-random-secret-at-least-24-characters'
export JRIP_DATA_ROOT="$PWD/data"
export RUST_LOG=info
cargo run -p jrip-server --bin jrip-server
```

APIは既定で `127.0.0.1:8080` に待ち受けます。`JRIP_LISTEN` で変更できます。
LAN公開時はTLS終端を設けてください。現在の認証は共通Bearerトークンで、ロール別認可ではありません。
データディレクトリは専用ユーザーのみがアクセスできる場所に配置してください。

## ジョブ投入とファイル出力

```sh
curl --fail-with-body http://127.0.0.1:8080/api/v1/jobs \
  -H "Authorization: Bearer $JRIP_API_TOKEN" \
  -F 'manifest={"name":"basic.ps","format":"application/postscript","dpi":300,"color_mode":"cmyk","priority":50}' \
  -F 'document=@tests/corpus/basic.ps'

# 応答のidを指定。同一JRIP_DATA_ROOTを使う別プロセスで実行します。
cargo run -p jrip-server --bin jrip-worker -- JOB_UUID
```

出力は `data/jobs/JOB_UUID/raster/page-000001.tiff` 以降に保存します。
`COMPLETED` はこの版では **TIFFファイル出力完了** を意味します。プリンターへの送信は行いません。
ワーカーは指定ジョブを1件処理して終了します。自動キュー取得・常駐スケジューラーは未実装です。

対応するmanifestフィールドは `name`, `format`, `dpi`, `color_mode`, `priority`, `engine` です。
未知フィールドは無視せず拒否します。DPIは72〜2400、優先度は0〜100、色は `rgb` / `cmyk` / `gray`。
PDFは `application/pdf`、PSは `application/postscript`、EPSは `application/eps` を指定します。
ファイル先頭のシグネチャを照合しますが、構造全体のプリフライトはまだ行いません。

### ハイブリッドエンジン

| `engine` | PDF | PostScript / EPS |
| --- | --- | --- |
| `auto`（既定） | MuPDF | Ghostscript |
| `mupdf` | MuPDF | 投入時に拒否 |
| `ghostscript` | Ghostscript | Ghostscript |

PDFの互換性比較や既存Ghostscript設定を優先する場合は、manifestに `"engine":"ghostscript"` を指定してください。
既存のengine未指定manifestは `auto` として読まれるため、PDFの既定エンジンが変わります。従来のPDF出力を維持するジョブでは明示指定が必要です。
ジョブ詳細の `selected_engine` に処理開始時の選択を保存します（未開始・旧ジョブはnull）。
失敗・タイムアウト・ページ上限超過時の自動フォールバックはありません。失敗したジョブはFAILEDになり、異なるエンジンでの処理は新規投入で明示指定します。

MuPDFは128行のバンドで8bit PAMを生成し、そのサンプルを色変換せず通常TIFFに格納します。
TIFF格納処理は64 KiBずつコピーし、RGB / CMYK / Gray、解像度、ページ順序を維持します。
MuPDF出力は非圧縮TIFF、Ghostscript出力はLZW TIFFなので、ファイルハッシュや容量は一致しません。
PAMの一時ファイルも同じtmpfs予算を使用し、現状の1ファイル上限（既定約10 MiB）により高DPI・大判ページは失敗する場合があります。
`JRIP_GS_IMAGE` / `JRIP_MUPDF_IMAGE` で対応するコンテナーイメージを変更できます。

| API | 動作 |
| --- | --- |
| `GET /health` | 認証不要のヘルスチェック |
| `POST /api/v1/jobs` | multipartの `manifest` と `document` を受信 |
| `GET /api/v1/jobs?limit=50&offset=0` | 優先度降順・受信時刻昇順、最大100件 |
| `GET /api/v1/jobs/{id}` | ジョブ詳細 |
| `POST /api/v1/jobs/{id}/hold` | 待機ジョブを保留 |
| `POST /api/v1/jobs/{id}/release` | 保留解除 |
| `POST /api/v1/jobs/{id}/cancel` | 取消。実行中は処理終了後に結果を破棄 |
| `DELETE /api/v1/jobs/{id}` | 終端状態のメタデータを削除。原稿・出力・イベントは保持 |

ドメインエラーは `error.code` と `error.messages`（`en`, `ja`, `zh-CN`）を返します。
HTTPフレームワークの抽出エラー（不正UUIDなど）は標準レスポンスです。

## 構成と制限

- `rip-core`: ジョブ型、状態遷移、manifest検証、64bitオーバーフロー検査付きラスターサイズ計算。
- `rip-storage`: SQLx / SQLite WAL。状態変更とイベントを同一トランザクションで保存し、revisionによって二重処理を防止。
- `rip-interpreter`: `DocumentInterpreter` 境界、HybridInterpreter、MuPDF / Ghostscriptアダプター、共通の隔離実行基盤。Coreは外部エンジンに依存しません。
- `jrip-server`: Axum HTTP API。32 MiB上限の原稿を逐次保存し、SHA-256を記録。
- `jrip-worker`: 別プロセスでジョブを処理。検証を通過したページ集合だけをディレクトリ単位で公開。

両エンジンはネットワークなし、read-only root、追加capabilityなし、no-new-privileges、CPU 1、メモリ／swap上限512 MiB、PID上限64、120秒のタイムアウトで動きます。
入力だけをread-only bind mountし、出力は最大1 GiBのtmpfs、作業領域は64 MiBのtmpfsです。
出力はサイズ上限付きアーカイブ経由で回収し、通常TIFFファイルだけを新規作成で展開します。最大100ページで、101ページ目を検出した場合は失敗します。
tmpfsもコンテナーメモリに算入されるため、実効出力上限はRAM予算にも制約されます。巨大ページは失敗することがあり、独自タイルRIPは今後の実装です。

Docker CLIの終了だけではコンテナーが停止しないため、処理終了・タイムアウト後にコンテナーを明示的に削除します。
ワーカー自体が強制終了した場合のコンテナー回収／RIPPING状態の自動復旧は未実装です。監督プロセス・リース導入前の評価用実装として扱ってください。
取消は最大実行時間まで待つ場合があります。削除後のファイル保持、ディスク全体の容量管理、保持期限の自動清掃も未実装です。

Noto CJKをイメージに含みますが、日本語CID/CMap・縦書きの互換性、フォント置換の検出／報告はまだ検証していません。
現在のCMYKは選択したMuPDFまたはGhostscriptの標準処理です。独立LittleCMS層、ICC登録、黒版保持、CMYK16品質を提供するものではありません。
MuPDF / Ghostscriptの配布・製品化時は仕様書のライセンス方針に従って確認してください。このリポジトリには製品ライセンスを選定していません。
Artifex社のMuPDF / Ghostscriptに依存するため、このソフトウェアはAGPLv3としています。

## 検証

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
# Dockerイメージを構築した環境で実変換、ページ上限、タイムアウトを検証
cargo test --workspace --locked -- --ignored
```

## English

Initial local RIP foundation: authenticated job API, SQLite persistence, guarded transitions, and a separate Docker-isolated hybrid worker exporting TIFF. PDFs use MuPDF by default; PS/EPS use Ghostscript. Set manifest `engine` to `ghostscript` to explicitly render PDFs with Ghostscript. There is no automatic fallback; `selected_engine` records the chosen engine. Start the API with `JRIP_API_TOKEN` (24+ characters), upload a multipart `manifest` and `document`, then run `jrip-worker JOB_UUID`. `COMPLETED` means file export only. IPP/PWG, automatic scheduling, recovery, Japanese font validation, ICC management and the Web UI are pending. See [roadmap](docs/ROADMAP.md).

## 简体中文

这是本地RIP服务器的初始实现：提供需要身份验证的作业API、SQLite持久化、状态转换检查，以及通过独立Docker隔离进程输出TIFF的混合引擎。PDF默认使用MuPDF，PS/EPS使用Ghostscript。可在manifest中设置 `engine: "ghostscript"` 来指定PDF引擎；不会自动切换引擎，`selected_engine` 记录实际选择。设置至少24个字符的 `JRIP_API_TOKEN`，上传包含 `manifest` 和 `document` 的multipart请求，然后运行 `jrip-worker JOB_UUID`。`COMPLETED` 仅表示文件导出完成。IPP/PWG、自动调度、崩溃恢复、日文字体验证、ICC管理和Web界面尚未实现。参见[路线图](docs/ROADMAP.md)。

MuPDFのコマンド仕様: [公式 mutool draw ドキュメント](https://mupdf.readthedocs.io/en/1.27.0/tools/mutool-draw.html)。導入版の対応機能はコンテナー結合テストで確認しています。
