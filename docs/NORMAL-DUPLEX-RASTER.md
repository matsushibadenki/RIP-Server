# Normal duplex raster / normal方式の両面ラスター / normal双面光栅

🟢 [Done] `rip-raster` can encode long-edge and short-edge duplex for the native
`pwg-raster-document-sheet-back=normal` coordinate system.

```sh
rip-raster output.pwg 300 rgb --duplex-normal=long-edge page1.tiff page2.tiff
rip-raster output.pwg 300 cmyk --duplex-normal=short-edge page1.tiff page2.tiff
```

Omit the flag for the existing simplex behavior. This command only writes a PWG
file; it does not contact or print to a device. The caller must first verify that
the destination uses `normal` sheet-back. Other coordinate systems require the general explicit-layout API below and must not be treated as normal implicitly.

日本語: `normal` の裏面座標系に対する長辺・短辺とじPWG生成を追加しました。フラグ省略時は従来の片面です。CLIはファイル生成だけを行い、印刷機へ送信しません。利用側で機器が `normal` に対応することを先に確認してください。その他の座標系を暗黙にnormalとして扱う機能はありません。

简体中文: 新增针对 `normal` 背面坐标系的长边／短边双面PWG生成。省略标志时保持单面。CLI仅生成文件，不联系打印机；调用方必须先验证目标设备使用normal，不能将其他坐标系静默当作normal。

## Encoding / 符号化 / 编码

- `Duplex=1`; `Tumble=0` for long-edge, `1` for short-edge.
- CrossFeedTransform and FeedTransform remain `1` on both front and back pages.
- Pixel order and geometry remain unchanged in this native normal coordinate system.
- Source page order is preserved. Odd jobs are not padded; pages and copies are not duplicated.
- Existing 8-bit RGB/CMYK/black polarity, DPI, layout, ICC hash, input/output budget and atomic no-overwrite rules still apply.

日本語: Duplex／Tumbleにとじ方向を記録し、normalでは表裏とも変換係数1を使います。画素・寸法・原稿順を維持し、奇数末尾の空白やページ複製は追加しません。既存の色・DPI・ICC・容量制限と、上書きしない原子的な公開を維持します。

简体中文: 通过Duplex／Tumble记录装订方向；normal的正反面变换系数均为1。保留像素、尺寸及原稿顺序，不自动补空白背面或复制页面。沿用颜色、DPI、ICC、容量限制及原子不覆盖发布规则。

This implements the normal rows of [PWG 5102.4, Table 9](https://ftp.pwg.org/pub/pwg/candidates/cs-ippraster10-20120420-5102.4.pdf).

## Rust API / Rust接口

`DuplexBinding::{LongEdge, ShortEdge}` selects binding.
`convert_normal_duplex` supports a writer and optional already-verified ICC digest;
errors may leave a prefix, so use private staging. `convert_normal_duplex_file`
publishes atomically without overwriting an existing output. The raw CLI has no
ICC override, matching its existing policy for profiled input.

`inspect_normal_duplex(bytes, binding)` validates every page against the requested
binding and identity transforms, rejecting mixed simplex/duplex headers, conflicting
binding, unsupported transforms and truncated data. The existing `inspect(bytes)`
continues accepting simplex only. The legacy Native IPP submission helpers still reject duplex. The explicit settings
path now supports it with bound sides and live capability checks; see [DEVICE-DUPLEX.md](DEVICE-DUPLEX.md).

日本語: 専用生成・検査APIを用意しました。既存の片面検査は両面データを拒否するため、旧Native IPP経路がsidesの拘束なしで送信することはありません。現在は設定拘束と能力照合付きの明示経路を追加しています（DEVICE-DUPLEX.md）。

简体中文: 提供专用生成及检查API。现有单面检查继续拒绝双面数据，防止旧Native IPP路径在未绑定sides时发送。现已新增绑定设置及能力检查的显式路径（DEVICE-DUPLEX.md）。

## Verification / 検証 / 验证

🟢 [Done] Tests cover explicit binding, mixed/header tampering, truncated input,
ICC binding, CLI invalid/missing arguments, atomic publication and no overwrite.
The independent macOS CUPS reader decodes long/short-edge RGB, CMYK and black jobs
with both odd (3) and even (4) pages: twelve combinations. It verifies native
Duplex/Tumble/transform headers and every pixel on both front and back pages.
Distinct per-page/row/corner samples detect reversal, rotation or duplication.
The portable Rust tests run on all platforms; the independent CUPS test currently
runs on macOS, matching the project's existing CUPS reader test.

日本語: 独立したmacOS CUPSリーダーで、長辺／短辺×RGB／CMYK／黒×奇数／偶数の12組を読み、ヘッダーと全画素を検証します。移植可能なRust試験は他OSでも実行します。ICC拘束、不正入力、混在ヘッダー、切り詰め、CLI、原子的公開も検査します。

简体中文: 独立macOS CUPS读取器验证长边／短边×RGB／CMYK／黑×奇数／偶数共12种组合的头部及全部像素。可移植Rust测试在其他系统上也运行，并检查ICC绑定、无效输入、混合头部、截断、CLI及原子发布。

🟢 [Done] Native IPP sides/sheet-back checks, persistent ticket/approval binding and physical preflight/API integration: [DEVICE-DUPLEX.md](DEVICE-DUPLEX.md).

🟢 [Done] All native sheet-back pixel transforms; see the extension below.

⭕️ [Pending] Real-printer duplex orientation, odd-sheet and collation acceptance.


## Additional native coordinates / 追加の裏面座標系 / 其他背面坐标系

🟢 [Done] `DuplexLayout` and `SheetBack::{Normal, Rotated, Flipped, ManualTumble}`
now support every Table 9 combination. `convert_duplex`, `convert_duplex_file`,
and `inspect_duplex` accept the explicit layout. The normal helpers retain their
previous behavior. The general CLI requires an explicit sheet-back mode:

```sh
rip-raster output.pwg 300 rgb --duplex=long-edge --sheet-back=rotated page1.tiff page2.tiff
```

Front pages always retain identity coordinates. On back pages, flipped/long-edge
reverses rows, flipped/short-edge reverses pixels within rows, rotated/long-edge
and manual-tumble/short-edge reverse both axes; remaining combinations are identity.
Each reversed pixel keeps its RGB/CMYK channel order. In-place swaps avoid a second
page-sized allocation. Headers describe the same transformed bitmap coordinates.
`inspect_duplex` checks transforms at every front/back position. Some modes have
identical transforms for a given binding; exact device mode is therefore also bound
in the delivery ticket and rechecked by Native IPP, not inferred from bytes alone.

日本語: 表面は常に恒等変換、裏面だけにPWG Table 9の行・列反転を適用します。色チャンネル順を維持し、インプレース交換で追加のページ配列を確保しません。同一ビット配置になる方式同士でも、Delivery ticketと機器能力の方式一致を検査します。

简体中文: 正面保持恒等变换，仅背面按PWG Table 9反转行或列。保留颜色通道顺序，原地交换不分配第二个整页数组。即使模式产生相同位图布局，仍检查Delivery ticket与设备能力模式一致。

Independent CUPS verification now covers 144 combinations, including all four modes,
three colors, both bindings, odd/even page counts, odd dimensions and non-square pages.
Normal-only APIs and the default simplex inspector remain compatible.
