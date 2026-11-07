# Verified TIFF artifact descriptors / 検証済みTIFF成果物契約 / 已验证TIFF产物描述

## 日本語

🟢 [Done] profile `org.jrip.raster-artifact-descriptor-v1`で、登録済みTIFF成果物の小さなdescriptorを取得する。

| REST GET | MCP tool | kind |
|---|---|---|
| `/api/v1/jobs/{jobId}/artifacts/{name}/descriptor` | `rip_describe_artifact` | `RasterArtifact` |
| `/api/v1/color/attempts/{colorAttemptId}/artifacts/{name}/descriptor` | `color_describe_artifact` | `ColorManagedArtifact` |

どちらも`jobs:read`と`artifacts:read`が必要。MCP引数はjobIdまたはcolorAttemptIdと、`page-000001.tiff`〜`page-000100.tiff`のname。RIP／Colorの共通Capability Modelからprofileとendpoint templateを発見できる。取得は実行権限を付与せず、再描画や色変換も行わない。

返却値はSHA-256 contentId・bytes・MIME・単一ファイルのpageCount=1・出力ページ番号・保存済み原稿ページ対応、寸法・8bit色空間・ICC hashと状態、実測DPI、生成元job／engineまたはcolor attempt／policy、取得reference、保持設定referenceを含む。大容量画像、ICC本体、ホストパス、tokenは含めない。engineやpolicyは保存済み生成記録であり、画素から再推定したものではない。旧記録で元ページ対応やengineが不明な場合はnull。

検証は既存の容量予約付き私有snapshotへコピーし、登録サイズ・hashを照合してから、Rust TIFF decoderで全画素を読む。単ページ・向き1・8bit RGB／CMYK／gray・最大64MiBの復号sampleを対象とし、複数画像・不正sample・非対応layoutを拒否する。previewと共有の同時decoder数は2。metadataのICC状態は容量制限とenvelope／色空間検査の結果で、色精度の認証ではない。原稿側にICCがない場合もMISSINGを明示し、仮のprofileを割り当てない。

DPIは`resolution.unit="dots-per-inch"`、x／yはそれぞれ`[分子,分母]`。インチとcmを正確な有理数へ正規化し、単位なし・必要情報なしはnull。ticketのDPIをファイルの実測値として代用しない。色成果物は保存ticketのoutputColorとoutputProfileSha256も照合する。DeviceLinkで出力ICCがない場合はMISSINGと変換元DeviceLink hashを別々に記録する。

`retention.retainUntil`は既存のjob-family最低保持境界であり、実際の削除日時ではない。未設定はnull、`expiresAt`も削除時刻を保証できないためnull。保持設定の変更でdescriptorのretention revisionは変わるが、contentIdは変わらない。応答は`private, no-store`。取得referenceにも通常の認証・hash検証を適用する。これはhashに基づく内容識別であり、新しい重複排除object storeや永続取得保証ではない。

検証：有理数DPI／複数画像の単体試験、scope・hash・symlink・不正TIFF・原稿ページ対応・再起動・保持更新のAPI試験、MCPの実TIFF・名前検査、実MuPDF／Ghostscript workerのPDF／PS／EPS成果物、Docker ICC／DeviceLink変換結果を確認。色ticketとICC／色空間の不一致を拒否する。

🟢 [Done] SourceDocumentの読取descriptorは[SOURCE-DESCRIPTORS.md](SOURCE-DESCRIPTORS.md)を参照。

🟠 [Next] 原稿のページ別寸法／ICC、全工程の公開時に固定する完全なartifact envelope、object store参照と確定した保持／移管契約。今回のTIFF descriptorで全artifact契約の完了とはしない。

## English

🟢 [Done] Raster and color-managed TIFFs expose `org.jrip.raster-artifact-descriptor-v1` through the REST endpoints and MCP tools above. Both jobs:read and artifacts:read are required. RIP/Color capability extensions advertise the profile and endpoint template. Names are bounded canonical page filenames; no paths or URLs are accepted as MCP artifact names.

The descriptor includes content-addressed SHA-256 identity, byte count, MIME, one page per file, output/original page mapping, dimensions, 8-bit color space, ICC hash/state, measured rational DPI, stored producer provenance, authenticated retrieval reference and retention reference. It carries no pixels, ICC bytes, host paths or credentials. Legacy unknown facts remain null. Producer records are not inferred from pixels.

A quota-reserved private snapshot must match the registered hash and size. A bounded Rust decoder then verifies all samples: one image, orientation 1, supported 8-bit RGB/CMYK/gray and at most 64MiB decoded samples. Two decoder slots are shared with preview. ICC envelope/color-space checks do not certify color accuracy. Missing profiles remain missing. DPI is an exact reduced rational pair for each axis in dots-per-inch, converting explicit inch/cm units; unknown units/resolution stay null rather than borrowing ticket DPI.

Color descriptors additionally match ticket output space/profile. DeviceLink output without embedded ICC records MISSING and the separate input DeviceLink digest. Retention points to the current minimum family retention policy, not a deletion schedule. expiresAt is null; changing the hold does not change contentId. Responses are private/no-store. Hash identity does not imply a new deduplicated object store or perpetual availability.

Tests cover bounded TIFF facts, authorization, tampering, symlinks, unsupported data, mapping, restart/retention changes, real-TIFF MCP access and name rejection, real MuPDF/Ghostscript output and Docker ICC/DeviceLink output with mismatch rejection.

🟢 [Done] SourceDocument read descriptors are covered in [SOURCE-DESCRIPTORS.md](SOURCE-DESCRIPTORS.md).

🟠 [Next] Per-page source geometry/ICC, fully fixed publication envelopes, object-store references and complete retention/transfer contracts remain unfinished.

## 简体中文

🟢 [Done] 上述REST与MCP提供`org.jrip.raster-artifact-descriptor-v1`。需要jobs:read与artifacts:read，通过RIP／Color能力扩展发现profile及endpoint template。MCP只接受规范页面文件名，不接受任意路径或URL。

描述包含SHA-256内容标识、容量、MIME、每文件1页、输出及原稿页对应、尺寸、8位色彩空间、ICC hash与状态、有理数实测DPI、保存的生成来源、鉴权读取引用及保留设置引用。不嵌入图像、ICC原文、主机路径或凭证，旧记录未知信息为null。来源记录不是从像素反推。

先以配额保护的私有snapshot验证登记hash与大小，再由Rust decoder验证全部像素。支持单页、方向1、8位RGB／CMYK／gray及最多64MiB解码sample，与preview共享2个decoder名额。ICC envelope与色彩空间检查不表示色彩精度认证。缺失ICC保持MISSING，不分配虚假profile。DPI按显式英寸／厘米单位换算为dots-per-inch的约分有理数，x／y各为[分子,分母]；未知单位或缺少分辨率为null，不用ticket DPI代替。

颜色产物还核对ticket输出空间与ICC。DeviceLink输出无内嵌ICC时记录MISSING，并单独记录输入DeviceLink hash。retention引用现有作业族最低保留设置，不是删除计划；expiresAt为null。保留变化不改变contentId。响应private/no-store，内容标识不等于新增去重object store或永久可用保证。

测试覆盖TIFF事实、权限、篡改、符号链接、无效数据、页对应、重启及保留更新、真实TIFF的MCP读取与名称拒绝、实际MuPDF／Ghostscript及Docker ICC／DeviceLink产物和不匹配拒绝。

🟢 [Done] SourceDocument读取描述见[SOURCE-DESCRIPTORS.md](SOURCE-DESCRIPTORS.md)。

🟠 [Next] 原稿各页尺寸／ICC、完整固定发布envelope、object-store引用及完整保留／转移契约仍未完成。

🟢 [Done] DeliveryArtifactの読取・検証 / Verified delivery reads / 已验证投递读取：[DELIVERY-ARTIFACT-DESCRIPTORS.md](DELIVERY-ARTIFACT-DESCRIPTORS.md)。
