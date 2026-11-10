# Raster placement / ラスターの配置 / 光栅布局

🟢 [Done] Explicit delivery and approval accept an optional `placement` object. Omitting it preserves legacy exact-size validation. Placement is stored in the delivery ticket, approval, output contract and ticket/artifact hashes; changing it invalidates a review/approval and cannot reuse an existing attempt ID for different settings.

```json
{"placement":{"scaleMode":"fit","scaleBasisPoints":10000,"rotation":90,"alignX":1,"alignY":1,"offsetXMicrons":0,"offsetYMicrons":0}}
```

- `scaleMode`: `actual`, `fit`, `custom`. `scaleBasisPoints`: 100–40000; 10000 is 100%, used by custom mode.
- `rotation`: clockwise 0/90/180/270. `alignX` and `alignY`: 0/1/2 (left/center/right and top/center/bottom).
- Offsets: signed microns, ±1000000. Positive X moves right; positive Y moves down. Rotation precedes scaling; alignment precedes offsets.
- Media pixel size uses rounded microns × DPI / 25400. Transformed image sizes and alignment use `floor(value + 0.5)`. The browser and Rust share golden-vector tests for these rules.
- Output is a paper-size canvas, with bilinear sample interpolation and white padding (RGB/gray=255, CMYK=0 before PWG black-polarity conversion). Out-of-paper samples are clipped with a preview warning. Device duplex sheet-back transforms happen after placement.
- Source TIFF and registered color output are unchanged. Source/target color space and embedded ICC verification remain enforced. Each decoded input and output page is bounded to 64 MiB; aggregate decode/composition and encoded PWG limits remain enforced. Large paper at high DPI can therefore be rejected.
- Screen preview is a scaled display of the verified TIFF and its placement, not a pixel/color proof. It does not grant authorization or query/print to a device by itself.

## 日本語

実機出力欄で拡大縮小・回転・配置・mm移動を指定します。「変更なし」は従来の寸法一致検査、「等倍」は用紙サイズのキャンバスへ等倍配置、「用紙に合わせる」は縦横比を維持した収まり、「倍率指定」は1〜400%です。左上原点で右・下が正方向。回転→拡縮→配置→移動の順に適用し、枠外は切り取ります。プレビューでも切り取りを警告します。

実出力はRustで補間・余白付加したPWGを生成し、その後に両面裏面変換を適用します。原稿TIFFと色変換成果物は変更しません。設定は承認・成果物hash・永続チケットに固定し、変更時は再確認・再承認が必要です。元画像・出力画像は各64 MiB／ページに制限します。実機の非印字領域や紙上の色再現は別途実機受入が必要です。

## 简体中文

在实体输出区设置缩放、旋转、九点位置及毫米偏移。未指定布局时仍要求尺寸完全匹配；原尺寸模式将图像放入纸张画布，适合纸张保持比例，指定比例为1–400%。以左上为原点，向右／下为正，依次旋转、缩放、对齐及偏移。裁切框外区域并在预览中警告。

Rust生成经过双线性插值及白边填充的PWG，然后应用双面背面变换。原始TIFF及色彩产物不变。配置绑定审批、产物hash及持久票据，修改后须重新检查和审批。输入及输出每页各限64 MiB。设备不可打印边距及纸上颜色仍需实机验收。

⭕️ [Pending] Physical paper-output acceptance remains paused until paper is available. / 用紙未準備のため実機受入は送信前で保留。 / 未备纸，实机验收仍停在发送前。
