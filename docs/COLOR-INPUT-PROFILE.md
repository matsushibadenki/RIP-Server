# Color input profile evidence / 色処理入力profile証跡 / 颜色处理输入profile证据

## English

🟢 [Done] `GET /api/v1/jobs/{id}/color-input-profile` inspects the completed job's
TIFF artifacts before color conversion. It requires both `jobs:read` and
`artifacts:read`. This inspects the input to the Color stage, not the original
PDF/PostScript's internal profiles or the RIP engine's preservation of them.

Each page is inspected from a private, hash-verified artifact snapshot under the
existing shared snapshot budget. The response records page number, artifact hash,
geometry, color space, embedded profile hash, registry availability and agreement
with the job manifest color mode. The completed job revision and artifact manifest
are checked again before returning. Modified or missing artifacts fail the request;
there is no partial-success evidence response.

| `metadata.profileState` | Meaning |
|---|---|
| `MISSING` | No embedded ICC; never assume sRGB or a device profile |
| `INVALID` | Embedded bytes fail bounded Rust ICC-envelope checks |
| `COLOR_SPACE_MISMATCH` | ICC header color space differs from TIFF sample layout |
| `DEVICE_LINK` | Embedded DeviceLink is not treated as a source characterization profile |
| `VALIDATED_ENVELOPE` | Envelope and TIFF color space agree; not full ICC or colorimetric validation |

When all pages have the same registered envelope-valid profile and agree with the
manifest, status is `EXPLICIT_PROFILE_AVAILABLE` with `candidateProfileSha256`.
Otherwise it is `EXPLICIT_SELECTION_REQUIRED`, candidate is null and page evidence
shows the problem. `mixedProfiles` is true if multiple envelope-valid hashes occur.
Missing/invalid pages also prevent a candidate, even if remaining pages agree.
Registry availability is an observation, not a guarantee that it will still exist
when a later transform starts.

`automaticSelection` is always false. This GET does not register embedded ICC,
create policies, queue conversion or authorize execution. Submit an explicit policy
through existing APIs; all normal validation is repeated there. The JCS
`evidenceDigest` covers the `evidence` object (profile, job ID, source revision and
pages). It is an integrity fingerprint, not a provider signature, approval or
execution token. The response contains English/Japanese/Simplified Chinese messages.

Limits: 32 pages, 64 MiB per artifact, 256 MiB total; an over-limit request is refused
rather than inspecting a silent subset. TIFF metadata inspection uses existing bounded
Rust decoder limits and accepts only the supported single-page RGB/CMYK/gray 8-bit
layouts. It does not decode pixels for gamut measurement or invoke native LCMS.
Oversized ICC tags and unsupported/malformed TIFF layouts fail the request.

Tests cover all five profile states, registered/unavailable ICC, mixed-page profiles,
manifest-bound evidence digest, missing scope and artifact tampering. These run
without Docker using synthetic TIFF fixtures. They do not certify actual printer
profiles or the RIP engine's source-profile handling.

🟢 [Done] Original-document declarations: see [SOURCE-COLOR-PROFILES.md](SOURCE-COLOR-PROFILES.md).
🟢 [Done] [Gamut warnings](GAMUT-WARNINGS.md). 🟠 [Next] Artifact-retention integration. ⭕️ [Pending] Acceptance of actual RIP output and real device profiles.

## 日本語

🟢 [Done] 上記GETで、完了jobのColor工程入力TIFFを検査します。`jobs:read` と `artifacts:read` が必要です。
原稿PDF／PostScript内のprofileやRIPでの保持状態を推定する機能ではありません。
原稿hashと照合した私有snapshotからページ別のICC・色空間・登録状況・manifestとの一致を確認し、
最後にjob revisionとartifact一覧を再確認します。改変や欠落は失敗とし、途中までの成功結果は返しません。

未埋込、不正、色空間不一致、DeviceLink、envelope検証済みを区別します。
全ページが同じ登録済みprofileでmanifestにも一致した場合だけ候補hashを返します。
混在や欠落がある場合は明示選択が必要です。RGBだからsRGBと推定せず、ICCの自動登録や変換投入もしません。
証跡のJCS digestは整合性の指紋であり、署名・承認・実行tokenではありません。

32ページ・各64 MiB・合計256 MiBまでで、超過時に一部だけを検査しません。
RustでTIFF／ICCの有界な構造を確認し、LCMSのnative parserや画素の色域測定は呼び出しません。
模擬TIFFで状態分類・登録欠落・ページ間混在・権限・artifact改変を検証します。
🟢 [Done] 原稿内部の宣言解析は [追加仕様](SOURCE-COLOR-PROFILES.md) を参照してください。
🟢 [Done] [gamut警告](GAMUT-WARNINGS.md)。🟠 [Next] artifact保持期限への統合。
⭕️ [Pending] 実RIP出力と実機ICCの受入試験。

## 简体中文

🟢 [Done] 上述GET检查已完成job的Color阶段输入TIFF，需要 `jobs:read` 和 `artifacts:read`。
不推断原始PDF／PostScript内部profile或RIP对它们的保留情况。
从hash验证的私有snapshot检查逐页ICC、颜色空间、注册状态及manifest一致性，返回前重新检查job revision与artifact列表。
缺失或篡改使请求失败，不返回部分成功证据。

区分未嵌入、无效、颜色空间不匹配、DeviceLink及envelope验证通过。
仅当全部页面使用同一已注册profile且符合manifest时返回候选hash；混合或缺失时要求显式选择。
不因RGB而假定sRGB，不自动注册ICC或投入转换。JCS证据digest不是签名、审批或执行token。

限制为32页、每页64 MiB、总计256 MiB，超限拒绝，不静默只检查部分页面。
只用Rust执行有界结构检查，不调用LCMS native parser或进行像素色域测量。
用模拟TIFF验证状态分类、注册缺失、混合profile、权限及artifact篡改。
🟢 [Done] 原稿内部声明分析见 [追加规格](SOURCE-COLOR-PROFILES.md)。
🟢 [Done] [gamut警告](GAMUT-WARNINGS.md)。🟠 [Next] artifact保留期限集成。
⭕️ [Pending] 实际RIP输出及设备ICC验收。
