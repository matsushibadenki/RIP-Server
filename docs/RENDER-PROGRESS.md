# Rendering progress / 描画進捗 / 渲染进度

## English

Both default adapters observe Linux `IN_CLOSE_WRITE` in the private container
output directory, installed before starting the renderer. MuPDF closes a PAM per
page; the wrapper checks its complete sample length and packages it as TIFF.
Ghostscript closes a separate TIFF per page. Engine stdout is discarded and
stderr is never parsed. The wrapper sends completed page bytes immediately,
while subsequent pages can still be rendering. The host does not poll file sizes
or interpret document-produced log lines as progress.

Adapter protocol `JRIP-PAGES-1\n` is followed by frames: big-endian u32 page
number, big-endian u64 byte length, then TIFF bytes. `(0,0)` terminates a successful
stream. Pages must start at 1, be contiguous and stay within configured page and
aggregate-byte limits. No arbitrary filenames/paths are accepted. The host reads
in bounded chunks, checks TIFF magic, creates fresh files, syncs each full page,
and awaits persistence of its observation before reading the next frame. It
requires the terminal frame, EOF and successful interpreter exit before accepting
the complete set. Malformed/truncated/oversized frames or callback failure abort
processing; the container is explicitly removed on success, failure and timeout.
The stream replaces the old whole-document tar transfer; rebuild **both** images
with the repository-root build commands in README when updating the worker.
Old images fail closed rather than silently falling back to a buffered protocol.

Job history, SSE, MCP and the unified pipeline timeline expose:

- `progressPhase: rendering`, `reasonCode: PAGE_RENDERED`, state `RIPPING`.
- `completedPages`: complete page frames received and checked so far.
- `totalPages`: the strict pre-render PDF inspection count; null for PS/EPS.
- `progressPercent`: integer percentage when total is known, otherwise null.
- `runId`, current control `revision`, `completionKind: null`.

Persistence requires the current unexpired worker lease and a contiguous page
sequence for that run and phase. It never advances the control revision. The
rendering and subsequent `artifact-verification` phases have independent counts.
Unknown PS/EPS totals are never guessed from comments or output logs. 100% is not
successful engine exit, source-validation evidence, artifact publication, color
conversion or physical printing. An observation is provisional raster-output
progress, not a document-conformance certificate. Later failure preserves already
observed page counts, but publishes no raster set or validation evidence.
Cancellation rejects subsequent observations, removes the container once the
next page callback or render timeout is reached, and discards staged output.
Cancellation does not promise immediate interruption of an in-flight page.
Events remain durable and cursor-resumable across API/MCP reconnections/restarts.

Tests cover streaming before producer completion, malformed/partial/oversized
frames, observer failure, real MuPDF and Ghostscript PDF colors and ordering,
PS/EPS validation, ignored document logs, late PostScript errors, live durable
progress during a deliberately slow second PS page, reconnection, cancellation,
page overflow, timeouts and source-scoped container cleanup.

## 日本語

両adapterは、描画開始前にコンテナー内の出力先へ `IN_CLOSE_WRITE` の監視を設置します。
MuPDFは閉じられたPAMのサンプル長を検査してTIFFへ包装し、Ghostscriptは閉じられた
ページ別TIFFを送ります。後続ページの描画中でも完成ページを即座に送信します。
原稿が出せるstdoutやstderrは進捗として扱いません。

ホストは版付きバイナリstreamのページ番号・連続性・容量上限・TIFFヘッダーを検査し、
ページ全体を保存・同期してから `PAGE_RENDERED` を永続化します。状態は `RIPPING`、
工程は `rendering` です。PDFの総数は厳密な事前検査結果、PS/EPSの総数・率はnullです。
現在の実行リースと操作revisionを確認しますが、進捗ではrevisionを変更しません。
成果物検証工程のページ数とは独立しています。

100%でもエンジン終了・原稿検証成功・成果物公開・色変換・実機完了を意味しません。
後続ページで失敗した場合も観測済みの進捗は残し、ラスター一式は公開しません。
取消後の進捗保存を拒否し、次のページ通知または描画timeout時にコンテナーを回収して
一時出力を破棄します。処理中ページの即時中断は保証しません。
履歴・SSE・MCP・共通timelineは再接続後もcursorから取得できます。

旧tar転送とは互換性がないため、更新時はREADMEのコマンドで両イメージを再構築してください。
実Dockerで両エンジンの色・順序・形式、低速PSの描画中進捗、取消、後続失敗、ページ上限、
timeoutとコンテナー回収を検証します。不完全・過大なframeを進捗に数えない単体テストも含みます。

## 简体中文

两种adapter在渲染器启动前监听容器输出目录的 `IN_CLOSE_WRITE`。
MuPDF关闭PAM后检查完整样本长度并封装成TIFF；Ghostscript直接发送已关闭的逐页TIFF。
后续页仍在渲染时即可传输完成页。原稿产生的stdout及stderr不作为进度依据。

主机检查版本化二进制流的页号、连续性、容量上限与TIFF头，完整保存并同步页面后，
持久化 `PAGE_RENDERED`。状态仍为 `RIPPING`，阶段为 `rendering`。
PDF总页数来自严格预检；PS/EPS总页数及百分比保持null。更新须有当前有效worker租约，
但不改变控制revision，与后续 `artifact-verification` 阶段独立。

100%不代表引擎成功退出、原稿验证成功、成果发布、色彩转换或实机打印完成。
后续失败保留已有观测，但不发布光栅成果。取消后拒绝新的进度更新，并在下一页通知或
渲染超时时回收容器和临时输出；不保证立即中断正在处理的页面。
历史、SSE、MCP及统一时间线支持重连后按cursor恢复。

本协议替代旧tar传输；更新worker时必须按README重新构建两个镜像。
测试包含真实双引擎输出、低速PS实时进度、重连、取消、后续错误、页数上限、超时、
容器回收，以及不完整或过大frame的拒绝。
