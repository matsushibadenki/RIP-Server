# PDF page selection / PDF選択描画 / PDF选择渲染

🟢 [Done] PDFの指定ページだけをMuPDF／Ghostscriptで描画します。原稿全体の厳密な構造・ページ内容検査とICC宣言解析は継続します。ページ選択は検査の省略ではありません。

```json
{
  "name": "proof.pdf",
  "format": "application/pdf",
  "dpi": 300,
  "color_mode": "rgb",
  "page_ranges": [{"first": 2, "last": 3}, {"first": 5, "last": 5}]
}
```

原稿投入用Manifestの `page_ranges` は1始まり・両端を含む昇順の範囲です。上限100ページ、重複・逆転・範囲外は拒否し、隣接範囲は結合します。省略・空配列は従来どおり全ページです。PS／EPSのManifestでは非空の選択を拒否します。原稿投入前PrintIntentでは `pageRanges: {mode,value}` として指定し、PDFのartifact用・物理pipeline用の両方で描画へ反映します。automaticは全ページ、preferredはPDFで指定を採用し、未対応形式では明示的に全ページへ調整します。requiredは未対応形式で競合を返します。物理pipelineのPS／EPSでは従来の描画後選択を維持します。

成果物の `page` とファイル名は生成順の連番です。選択描画した成果物には `sourcePage` と `sourcePageCount` を保存し、色変換後も保持します。例えば原稿2・5ページは `page-000001.tiff`／`page-000002.tiff`、`sourcePage` は2／5になります。通常の全ページ出力では追加属性を省略し、旧JSON・ticketの互換性を維持します。

配送 `pageRanges` は原稿の物理ページ番号です。選択描画後に未描画ページを要求すると `page_not_rendered`、原稿総数を超えると `page_range_out_of_bounds` です。省略時は生成済みページ全てを送ります。契約に原稿総数・描画数・選択した原稿ページを含め、承認・再起動・hash検査を継続します。PDFのページラベル、面付け後の位置、部数展開後の位置ではありません。

選択時は `JRIP-PAGES-2` と原稿総数を必須とし、旧アダプターによる無視を拒否します。ホストで原稿総数、選択数、連番、TIFF形式、容量、終端を検証し、ワーカーの厳密検査結果とも照合します。描画進捗の総数は選択ページ数、原稿検証の `pageCount` は原稿全体です。両エンジンの実描画、範囲外拒否、離れたページの画素一致、実ワーカー、色変換、模擬IPP配送、永続化と再起動を回帰試験しました。

🟢 [Done] [仮想IPP受付のpage-ranges](IPP-PAGE-RANGES.md)。
🟢 [Done] [操作画面のページ範囲入力・原稿番号プレビュー](OPERATOR-CONSOLE.md)。
⭕️ [Pending] 実機の紙枚数・ページ順・取消の受入確認。

## English

PDF manifests accept `page_ranges` to rasterize only selected source pages in both engines. Strict inspection of every source page and passive ICC declaration analysis remain mandatory. Ranges are inclusive, ascending, non-overlapping and bounded to 1–100; adjacent ranges merge. Missing/empty selects all. Nonempty PS/EPS manifest ranges are rejected. PDF PrintIntent selection applies before rendering; physical PS/EPS pipelines retain delivery-only filtering.

Selected outputs use dense file/page numbering plus immutable `sourcePage`/`sourcePageCount`, preserved through color processing. Delivery ranges refer to original physical source positions: unrendered pages fail rather than clipping. All-page legacy outputs omit mapping fields. Selected streams require version 2 with an independently checked source count, exact selected frame count, byte budgets and valid termination. Rendering progress counts selected pages; source-validation pageCount counts the original document. Virtual IPP range controls are implemented with their standard clipping semantics; console input is implemented; physical-device acceptance is Pending.

## 简体中文

PDF Manifest通过 `page_ranges` 指定MuPDF／Ghostscript仅渲染所选原稿页。仍严格检查全部原稿页及ICC声明。范围包含两端、升序且不重叠，页码限1至100；相邻范围合并，省略或空数组表示全部。PS／EPS Manifest拒绝非空范围。PDF PrintIntent在渲染前应用选择，实体PS／EPS流程保留投递时筛选。

选中产物以连续页码与文件名保存，同时记录 `sourcePage`／`sourcePageCount`，色彩转换后保持。投递范围表示原稿物理页，未渲染页会报错，不会裁剪。旧全页产物省略映射字段。选择渲染要求带原稿总数的第2版流协议，并检查页数、容量及结束标记。渲染进度统计所选页，原稿验证页数统计完整文档。虚拟IPP范围已按标准实现，界面范围输入已实现，真实设备验收为Pending。
