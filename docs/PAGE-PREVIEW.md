# Page preview / ページプレビュー / 页面预览

🟢 [Done] `GET /api/v1/jobs/{jobId}/artifacts/{name}/preview`

- EN: Requires `artifacts:read`. Verifies a private snapshot against the registered SHA-256 before decoding. Returns `image/bmp`, at most 1024 pixels per axis, preserving aspect ratio to integer rounding without upscaling. RGB8, Gray8 and CMYK8 single-image TIFF with top-left orientation. Embedded ICC is validated and converted to sRGB using LittleCMS, relative colorimetric intent and black point compensation. CMYK requires an embedded matching ICC; unprofiled RGB/gray retains the previous assumed-screen interpretation. DeviceLink, invalid ICC and color-space mismatches are rejected. This is a nearest-neighbor layout preview, not a color proof. Original artifacts are unchanged.
- 日本語: `artifacts:read` が必要です。登録済みSHA-256と一致する専用スナップショットを検証後、縦横最大1024pxのBMPを返します。整数丸めの範囲で縦横比を維持し、拡大しません。RGB8／Gray8／CMYK8の単一画像・左上原点TIFFに対応します。埋め込みICCを検証し、LittleCMSで相対的色域を維持する意図と黒点補正を使いsRGBへ変換します。CMYKには色空間の一致する埋め込みICCが必須です。ICCなしRGB／グレーは従来の画面色として扱います。DeviceLink・不正ICC・色空間不一致は拒否します。最近傍縮小による配置確認用で、色校正ではありません。原本は変更しません。
- 简体中文: 需要 `artifacts:read`。验证私有快照与登记的SHA-256一致后返回最大1024px的BMP，按整数舍入保持比例且不放大。支持RGB8／Gray8／CMYK8单图像、左上原点TIFF。验证嵌入ICC后，通过LittleCMS以相对色度和黑点补偿转换为sRGB。CMYK必须嵌入匹配的ICC。无ICC的RGB／灰度保持原有屏幕颜色解释。拒绝DeviceLink、无效ICC及色彩空间不匹配。最近邻缩小仅供版面检查，不用于色彩校样。原稿不变。

`Cache-Control: private, no-store`; `X-Content-Type-Options: nosniff`;
`X-JRIP-Preview-Color-Space: srgb`; `X-JRIP-Source-SHA256` identifies the verified source bytes.

Unsupported or malformed TIFF: HTTP 422 `preview_unsupported` (EN/JA/zh message).
Integrity mismatch: HTTP 409 `artifact_integrity_mismatch`.
At most two decoders per server process; saturation returns HTTP 503
`artifact_budget_exhausted`. Each decode is bounded to 64 MiB pixel data and
8 MiB intermediate buffers; existing shared snapshot disk quotas also apply.
Preview decoding runs off the asynchronous executor. Output is under 4 MiB.

🟢 [Done] Embedded ICC-aware RGB/CMYK/gray screen conversion. / 埋め込みICC対応RGB／CMYK／グレー画面変換。 / 嵌入ICC感知的RGB／CMYK／灰度屏幕转换。

🟠 [Next] UI integration. / 操作画面への統合。 / 界面集成。

⭕️ [Pending] Physical color proof acceptance. / 実機の色校正確認。 / 实机色彩校样验收。

Conversion runs only on the downsampled pixels (at most 1024×1024); ICC is limited to 4 MiB. The original raster is neither converted nor overwritten. The synthetic CMYK fixture and generator are under `crates/rip-raster/tests/fixtures`; they test transform behavior, not a real printing condition.

## Color-stage artifacts / 色変換工程の成果物 / 色彩管理阶段产物

🟢 [Done] `GET /api/v1/color/attempts/{attemptId}/artifacts/{name}/preview`

- EN: Requires `artifacts:read` and a `COMPLETED` color attempt. Verifies the registered artifact SHA-256 using a private snapshot, then requires the embedded ICC hash and raster color space to match the attempt ticket's `outputProfileSha256` and `policy.outputColor`. Converts to sRGB with the same bounded preview engine. It never substitutes the source RIP raster or changes the color-stage result. Missing output ICC (including DeviceLink-only output), missing/invalid/mismatched embedded ICC, or wrong color space returns HTTP 422 `preview_unsupported`. Non-completed attempts return 409; unknown pages return 404; changed artifact bytes return 409.
- 日本語: `artifacts:read` と `COMPLETED` の色変換attemptが必要です。専用スナップショットのSHA-256を検証後、埋め込みICCのhashと画素の色空間がticketの `outputProfileSha256`、`policy.outputColor` に一致することを確認します。同じ上限付きプレビュー処理でsRGBへ変換し、原本ラスターや色変換成果物は変更しません。出力ICCなし（DeviceLinkのみの出力を含む）、埋め込みICCの欠落・不正・不一致、色空間不一致は422 `preview_unsupported`、未完了は409、未登録ページは404、成果物の改ざんは409です。
- 简体中文: 需要 `artifacts:read` 及 `COMPLETED` 的色彩转换attempt。验证私有快照SHA-256后，确认嵌入ICC的hash及像素色彩空间与ticket的 `outputProfileSha256`、`policy.outputColor` 一致。使用相同的受限预览引擎转换为sRGB，不修改原始光栅或色彩转换产物。缺少输出ICC（含仅DeviceLink输出）、嵌入ICC缺失／无效／不匹配及色彩空间不匹配返回422 `preview_unsupported`；未完成返回409，未登记页面返回404，产物篡改返回409。

Additional response headers:

- `X-JRIP-Color-Attempt-Id`: completed attempt ID.
- `X-JRIP-Input-Profile-SHA256`: the color-stage output profile used as the screen conversion input.
- `X-JRIP-Source-SHA256`: the color-managed TIFF hash, not the original RIP TIFF hash.

Both endpoints share the two-decoder process limit and existing shared snapshot quota.
No printer is contacted; paper/ink simulation and physical proof acceptance remain Pending.
