# DeliveryArtifact descriptors / 配信成果物の契約 / 投递产物描述

🟢 [Done] `org.jrip.delivery-artifact-descriptor-v1`

| Interface | Reference |
| --- | --- |
| REST metadata | `GET /api/v1/deliveries/{deliveryId}/artifact/descriptor` |
| REST bytes | `GET /api/v1/deliveries/{deliveryId}/artifact` |
| MCP tool | `print_describe_delivery_artifact({deliveryId})` |
| MCP metadata | `jrip://deliveries/{deliveryId}/artifact/descriptor` |
| MCP bytes | `jrip://deliveries/{deliveryId}/artifact/chunks/{offset}/{length}` |

## 日本語

すべての経路にjobs:readとartifacts:readの両方が必要。print権限は不要で、読取はプリンターへの問い合わせ・再送・状態変更を行わない。登録済みPWGを私有snapshotへコピーし、容量・SHA-256・全ページのheaderとRLE scanlineを検証する。保存ticketのDPI、documentType、media名、両面とsheet-back条件を照合する。上限はencoded 32MiB、100ページ、decoded 64MiB／ページ・合計1GiB。画素展開用の巨大バッファは作らない。

保存されたphysical contractがあればticketのdigest（org.jrip.serde-json-v1）、job／delivery ID、source hash形式、DeliveryTicket、保存ページ対応を照合する。これはローカル記録の整合性検証であり、独立した電子署名検証ではない。契約のない旧記録はphysicalContractVerified=false、sourceSha256=null。存在する契約との矛盾は409、非対応のPWG構造は422になる。プリンター設定を削除した後、再起動後、取消後も保持データを検査できる。

descriptorはcontentId、SHA-256、容量、MIME、実測ページ別寸法・DPI・色空間、保存されたcopies／sides／sheetBack、生成元、取得referenceを返す。pageCountはPWG内のページ数で、copies倍しない。原稿ページ対応が記録されていない場合はnull。ICC埋込は証明しないためprofileSha256=nullとし、色変換attemptはcolorReferenceで別に参照する。現時点の用紙寸法・装置能力や上流画素からの生成過程を再証明しない。executionは読取時の保存状態で、printedAppearanceVerified=false。

RESTは固定名delivery.pwg、SHA-256 ETag、Range／If-Range、private/no-storeを返す。応答開始後に元ファイルを変更してもsnapshotのbytesは変わらない。MCPは先頭ゼロのない非負offset、length=1〜32768でbase64 blobとETag／Content-Rangeを返す。各chunkでも全PWGを検証するため、応答上限と検証コストは別。私有snapshotの容量配額と共通decoder同時実行制限を適用する。

retentionはjob-familyの最低保持設定への参照で、削除予定や永久取得保証ではない。expiresAtはnull。ホストパス・資格情報を公開せず、読取は印刷承認を与えない。

## English

Both jobs:read and artifacts:read are required on every interface; print authority is unnecessary. Reads neither query a printer, resend data nor mutate execution. A private quota-reserved snapshot verifies the registered size/hash and all PWG headers and RLE scanlines against the saved DPI, document type, media name, duplex and sheet-back ticket. Limits are 32MiB encoded, 100 pages, 64MiB decoded per page and 1GiB total; inspection does not allocate a decoded pixel buffer.

An existing physical contract must match its org.jrip.serde-json-v1 ticket digest, IDs, source digest syntax, typed DeliveryTicket and saved source-page mapping. This proves local consistency, not independent signature authenticity. Historical records without that contract return physicalContractVerified=false and null sourceSha256; contradictory contracts fail with 409, unsupported PWG with 422. Reads survive cancellation, restart and removed printer configuration.

Descriptors expose content identity, actual per-page geometry/DPI/color space, saved copy/duplex conditions, provenance and authenticated references. pageCount counts encoded pages, not copies. Unknown source mapping stays null. profileSha256 is null because no embedded ICC is established; a color attempt is referenced separately. Current device/media capabilities and upstream pixel derivation are not re-proved. Execution is a stored-state view and printedAppearanceVerified remains false.

REST supports a fixed safe attachment name, SHA-256 ETag and Range/If-Range from the immutable private snapshot. MCP uses canonical offsets and 1–32768-byte chunks with base64 blob and ETag/Content-Range metadata. Every chunk verifies the whole PWG; snapshot quotas and the shared decoder concurrency bound still apply. Responses are private/no-store. Retention references a minimum job-family hold, not a deletion schedule or perpetual availability; expiresAt is null. No host paths, credentials or print authorization are exposed.

## 简体中文

所有接口同时要求jobs:read与artifacts:read，无需print权限。读取不查询打印机、不重发、不改变执行状态。使用有配额的私有snapshot验证登记容量、SHA-256、所有PWG header及RLE scanline，并核对保存的DPI、documentType、media名称、双面与sheet-back条件。限制为encoded 32MiB、100页、decoded每页64MiB及总计1GiB；检查不分配完整像素缓冲区。

存在physical contract时验证org.jrip.serde-json-v1 ticket摘要、ID、source hash格式、DeliveryTicket及保存的原稿页映射。这是本地记录一致性验证，不是独立电子签名认证。旧记录缺少契约时返回physicalContractVerified=false、sourceSha256=null；现有契约矛盾返回409，不支持的PWG返回422。取消、重启或移除打印机配置后仍可检查保留数据。

descriptor提供内容身份、实际逐页尺寸／DPI／色空间、份数／双面条件、生成来源与认证引用。pageCount是编码页数，不乘以copies；未知原稿映射保留null。不证明内嵌ICC，因此profileSha256=null，色转换attempt另以colorReference引用。不重新证明当前设备／纸张能力及上游像素生成关系。execution表示保存状态，printedAppearanceVerified=false。

REST支持安全固定附件名、SHA-256 ETag、Range／If-Range，原文件后续修改不改变已接收snapshot。MCP使用规范非负offset及1至32768字节length，返回base64 blob和ETag／Content-Range。每个分块仍验证整个PWG，并遵守snapshot配额和共享decoder并发限制。响应为private/no-store。retention是job-family最低保留引用，不是删除计划或永久可用保证，expiresAt=null；不公开主机路径、凭据或授予打印批准。

🟠 [Next] Fixed publication envelopes, per-page source geometry/ICC and object-store/retention/transfer contracts / 公开时固定的共通envelope、原稿逐页尺寸／ICC与存储生命周期 / 公開時の共通envelope、原稿ページ別寸法／ICCと保存・移管契約。

⭕️ [Pending] Physical printer appearance and vendor interoperability acceptance / 実機の見た目・機種別相互運用検証 / 实机外观及厂商互操作验证。
