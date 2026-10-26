# Gamut warnings / 色域警告 / 色域警告

## English

🟢 [Done] Every new `rip-color` conversion reports destination ICC gamut diagnostics
in `ColorReport.gamut`. The isolated color worker validates and stores this report,
adds its warnings to each page's existing warnings, and publishes `gamutSummary` in
the completed color attempt. Existing detail APIs/MCP color tools return these fields.

The method `lcms2-relative-colorimetric-null-proof-v1` checks the original source
samples against the explicit destination profile. It uses a separate LittleCMS
proofing transform, relative colorimetric reference intent, BPC off, GAMUT_CHECK /
SOFT_PROOFING, and a NULL output profile: in-gamut output is zero and the per-context
alarm is 65535. This follows the [LittleCMS NULL-profile gamut technique](https://www.littlecms.com/LittleCMS2.17%20tutorial.pdf).
All original pixels are checked, in blocks of 4096 with an 8 KiB temporary mask.
There is no stochastic sampling, retained mask image, UI alarm paint or transfer of
pixel data to JavaScript. Separate contexts avoid changing global alarm state.
Native proof errors and unexpected mask values are reported as unassessed.

This is an ICC/LittleCMS gamut estimate, with the engine's internal gamut LUT and
tolerances, not a physical printer measurement or exact mathematical gamut boundary.
Full pixel coverage does not remove that model approximation. It is independent of
the selected production rendering intent, BPC, K preservation and TAC. It assesses
source colors before those mappings; it does not promise the same colors would be
clipped by every production intent, or measure post-TAC color difference.

| Field under `gamut` | Meaning |
|---|---|
| `profile` | `org.jrip.gamut-warning-v1` |
| `status` | `ASSESSED` or `NOT_ASSESSED` |
| `totalPixels`, `checkedPixels` | Source pixel count and diagnostic coverage |
| `outOfGamutPixels`, `outOfGamutPercent` | Count/rate flagged by the ICC model; null if unassessed |
| `referenceIntent`, `blackPointCompensation` | `relative_colorimetric`, false |
| `sampling`, `modifiesPixels` | `all-pixels`, false |
| `reasonCode`, `warnings` | Machine-readable diagnosis and three-language messages |

`OUT_OF_GAMUT` is emitted when at least one pixel is flagged. An assessed zero count
means no alarm under this model, not certified color fidelity. DeviceLink has no
separate destination characterization profile, so it returns `NOT_ASSESSED`, null
count/rate, and `GAMUT_NOT_ASSESSED` with reason
`DEVICE_LINK_NO_SEPARATE_OUTPUT_PROFILE`. Failure to create the proof transform or
native proof errors use `PROOF_TRANSFORM_UNAVAILABLE`; an invalid mask uses
`UNEXPECTED_GAMUT_MASK`. None of these is silently replaced with zero risk.

The diagnostic does not modify output TIFF samples, ICC tags, rendering settings,
or the policy's existing SHA-256 encoding. Normal ICC conversion can still map
colors according to that policy. Gamut warnings do not automatically reject a job,
change intent, select another profile, reduce ink or reprint.

Page reports are available at:

```text
GET /api/v1/color/attempts/{id}          (jobs:read)
  result.reports[].colorReport.gamut
  result.reports[].warnings
  result.gamutSummary
```

The summary includes `coverageComplete`, assessed/unassessed page counts,
`checkedPixels`, `outOfGamutPixels`, and `outOfGamutPercentOfChecked`. When nothing
was assessed, count/rate are null. With partial coverage, count/rate describe only
checked pixels and `coverageComplete` is false. Source-profile-assumption warnings
remain separate and are preserved. Completed reports survive restart and request-ID
replay. Old stored attempts without these fields mean unmeasured, not zero gamut
risk. New workers refuse missing/inconsistent diagnostics before output publication.

Rebuild the worker image before processing new color attempts:

```sh
docker build -f adapters/color/Dockerfile -t jrip-color:local .
cargo test -p jrip-server -p rip-storage -p rip-color --lib --test color --test cli --locked
cargo test -p jrip-server -p rip-storage -p rip-color --test color --locked -- --ignored
cargo test -p jrip-server -p rip-storage -p rip-color --lib cmyk_output_and_input_profiles --locked -- --ignored
```

Tests cover sRGB identity (no warning), saturated colors into a synthetic narrow
RGB gamut, neutral grays, every-pixel/chunk-tail coverage, gray destination and
Ghostscript's standard CMYK profile in both directions. They verify unchanged
production pixels, unavailable/native-error handling, protocol tampering, CLI output,
real Docker page warnings/counts, DeviceLink null counts, source preservation,
restart and replay. The standard CMYK fixture is read from the installed Ghostscript
image for testing; it is not a customer printer characterization.

⭕️ [Pending] Acceptance against measured customer printer/media/ink profiles.
🔴 [Later] Interactive gamut masks/soft-proof previews, custom perceptual thresholds,
and policy-controlled approval/rejection based on warnings.

## 日本語

🟢 [Done] 新規のICC変換に `ColorReport.gamut` を追加し、ページ警告とjob全体の `gamutSummary` を保存します。
変換前の全画素を、明示した出力ICCに対してLittleCMSの独立proofing変換で検査します。
基準はrelative colorimetric・BPCなしです。NULL出力とcontext固有alarmを使い、4096画素ずつ判定します。
警告のために出力画素へalarm色を描かず、通常の変換結果・設定・TAC・黒版保持を変更しません。

1画素以上なら `OUT_OF_GAMUT`、判定できない場合は `GAMUT_NOT_ASSESSED` を返します。
DeviceLinkには独立した出力特性profileがないため未判定とし、件数・割合は0ではなくnullです。
proofingの内部エラーも未判定にし、古い／不整合なworker報告を正常な0件として公開しません。

全画素を走査しますが、色域そのものはICCとLittleCMS内部LUT・許容誤差による推定です。
実機測色、厳密な色域境界、選択したproduction intentでのclip件数、TAC適用後の色差とは区別します。
警告だけでjob拒否・profile変更・再印刷は行いません。

既存のcolor attempt API／MCPからページ別警告と集計を取得できます。
集計は判定済み画素に対する割合を示し、判定範囲が欠ける場合は `coverageComplete:false`、
全未判定なら件数・割合がnullです。原稿ICC未埋込の警告も保持し、再起動・再送で結果は変えません。
旧記録にfieldがない場合を「色域外0件」と扱わないでください。上記Dockerイメージの更新が必要です。

sRGB一致、狭いRGB色域、gray、CMYK入出力、ブロック末尾、出力画素不変、未判定・不整合、
CLI、実Docker worker、警告集計・再起動・再送をテストします。
⭕️ [Pending] 実顧客のプリンター・用紙・インキprofileと測色による受入確認。
🔴 [Later] gamut mask／soft proof表示、独自閾値、警告に基づく承認・拒否policy。

## 简体中文

🟢 [Done] 新ICC转换增加 `ColorReport.gamut`，并持久保存逐页警告及 `gamutSummary`。
以relative colorimetric、无BPC的独立LittleCMS proofing变换，检查原始全部像素与指定输出ICC色域。
使用NULL输出和context独立alarm，每批4096像素，不把alarm色写入输出，不改变正常转换、TAC或黑版设置。

发现至少一个像素时返回 `OUT_OF_GAMUT`；无法判定时返回 `GAMUT_NOT_ASSESSED`。
DeviceLink缺少独立输出特性profile，因此数量／比例为null而不是0。proofing内部错误同样标记为未评估，
旧版或不一致worker报告不会当作正常0像素发布。

全部像素覆盖仍是ICC及LittleCMS内部LUT／容差下的估计，不等于设备测色、精确边界、
production intent的clip数量或TAC后的色差。警告不会自动拒绝job、换profile或重新打印。

通过现有color attempt API／MCP读取逐页警告及汇总；比例仅针对已检查像素。
覆盖不全时 `coverageComplete:false`，全部未评估时数量／比例为null。
保留原稿ICC缺失警告；结果可在重启及重放后读取。历史字段缺失不表示色域风险为0。须更新上述Docker镜像。

验证sRGB一致、窄RGB色域、gray、CMYK双向、分块末尾、输出不变、未评估／不一致报告、CLI、
实际Docker worker、警告汇总、重启及重放。
⭕️ [Pending] 客户打印机／介质／油墨profile及测色验收。
🔴 [Later] gamut mask／soft proof显示、自定义阈值及基于警告的审批／拒绝policy。
