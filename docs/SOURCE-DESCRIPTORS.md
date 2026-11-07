# SourceDocument descriptors / 原稿descriptor / 原稿描述

## 日本語

🟢 [Done] `GET /api/v1/jobs/{jobId}/source/descriptor`とMCP `print_describe_source({jobId})`で、profile `org.jrip.source-artifact-descriptor-v1`を取得する。MCP resourceは`jrip://jobs/{jobId}/source/descriptor`。どの経路もjobs:readとartifacts:readの両方が必要。

元ファイルが保持されていることを確認し、最大32MiB・通常ファイル・canonical path・保存済みticketの形式／容量／hashを照合する。既存の容量予約・共有予算・coordinator・cleanupを使う私有snapshotへ全体をコピーし、登録SHA-256を検証する。形式ヘッダーも登録formatと照合する。読取によってDocker、RIP、原稿URIのダウンロード、構造検証、ジョブ投入を開始しない。

返却値はSourceDocumentのURI、SHA-256 contentId、容量、MIME、job／受付時刻、データ取得reference、最低保持設定referenceを含む。contentIdは文書内容の識別であり、object storeへの重複排除保存や永久取得保証ではない。ホストパス・token・本文はdescriptorへ含めず、応答はprivate/no-store。保持期限が未設定ならretainUntilはnull、削除日時を保証しないためexpiresAtもnull。

`integrityVerified:true`は今回読み取ったbytesが登録内容と一致することを示す。`structureVerified`は、同じhash・容量・format・jobに結び付く保存済みのstrict PDF inspection成功証跡があるときだけtrue。これはbounded検査であり、PDF/A適合、全埋込objectの正常性や印刷品質の認証ではない。PS／EPSはboundedInterpretationVerifiedで既存workerの解釈成功を区別し、structureVerifiedをtrueにしない。

成功証跡がない場合はvalidationとpageCountがnull、検証flagはfalse。これは「検証して失敗した」とも「一度も検証していない」とも断定しない。実行状態・失敗理由はjob／eventから取得する。証跡があればpageCount・validatedAt・scope・検証reportへのreferenceとprofile解析digestを返す。原稿が複数の色空間や寸法を持ち得るため、出力manifestから原稿の色・寸法・ICCを推測せず、現在これらはnull。検証状態が変わっても元ticket・requestHash・contentIdを変更しない。

原稿本体は`GET /api/v1/jobs/{jobId}/source`で取得する。登録MIME、固定のsource.pdf／source.ps／source.eps添付名、SHA-256 ETag、Range／If-Rangeを提供し、本文は検証済みsnapshotから配信する。元ファイルが後から変わっても配信中のbytesは変わらない。欠落はsource_unavailable、改変・形式不一致・symlink・容量超過は拒否する。

MCPでは`jrip://jobs/{jobId}/source/chunks/{offset}/{length}`を使う。offsetは先頭ゼロのない非負整数、lengthは1〜32768バイト。base64 blobと既存artifact-chunk metadata（ETag・Content-Range・取得サイズ）を返す。原稿URIは識別用で、MCPで全体を無制限に読むURIではない。上記descriptor／chunk URIを発見して使う。毎回原稿全体をhash検証するため、分割しても検証コストは全体分になる。両scopeが揃う主体だけにtemplateを公開し、chunkは購読対象にしない。

検証：未検査文書のfalse／null、同容量改変、形式不一致、symlink・欠落・容量・quota、配信中の原稿置換、Range／If-Range、再起動、3形式のMIMEと状態不変、MCPの範囲・scope・改変拒否、実MuPDF／Ghostscript前後の証跡との結合を確認。

🟠 [Next] ページ別の原稿寸法／ICCの共通表現、全工程の固定publication envelopeとobject store／保持・移管契約。

## English

🟢 [Done] The REST/MCP endpoints above expose `org.jrip.source-artifact-descriptor-v1`. Both jobs:read and artifacts:read are required. A retained regular source file, canonical path, admission ticket when present, format, size and registered hash are checked. The entire source (at most 32MiB) is copied into the existing quota-reserved private snapshot mechanism. Read requests perform no interpretation, external URI download, validation submission or job execution.

The descriptor carries URI, content-addressed hash, bytes, MIME, admission provenance, authenticated content reference and minimum-retention reference; it contains no source body, host paths or credentials. Responses are private/no-store. Hash identity does not imply a deduplicated object store or perpetual availability. retainUntil may be null and expiresAt is null rather than an invented deletion deadline.

Integrity verification is independent of structure. structureVerified requires matching stored strict-PDF inspection evidence; its scope remains bounded, not PDF/A certification, exhaustive embedded-object verification or printed appearance. PS/EPS interpreter evidence is reported through boundedInterpretationVerified without claiming strict PDF structure checks. Missing success evidence gives null validation/page count and false flags; it does not distinguish failure from never-run. Job/events provide execution state. Recorded page count, time, scope, report reference and profile-analysis digest are exposed when available. Source-wide dimensions, color space and ICC remain null rather than inferred from requested output. Refined evidence does not mutate ticket/request/content identity.

The source REST endpoint supplies the admitted MIME, a fixed safe attachment name, SHA-256 ETag and byte ranges from verified snapshots. Concurrent replacement cannot alter a response already admitted. MCP reads bounded 1–32768-byte chunks through the dedicated template; canonical offsets, two-scope checks, bounded blob responses and artifact-chunk ETag/range metadata apply. Use descriptor/chunk resources, not an unbounded read of the logical source URI. Every chunk still verifies the whole file and uses existing snapshot quotas.

Tests cover unvalidated evidence, same-size tampering, format mismatch, symlinks/missing/oversized files, quota, response immutability, range/resume, restart, three MIME formats, scoped MCP chunks and real MuPDF/Ghostscript evidence before/after rendering.

🟠 [Next] Per-page source geometry/ICC contracts, fixed publication envelopes and object-store/retention/transfer contracts remain unfinished.

## 简体中文

🟢 [Done] 上述REST／MCP提供`org.jrip.source-artifact-descriptor-v1`，同时需要jobs:read与artifacts:read。验证已保留的普通文件、canonical path、存在的接收ticket、格式、容量及登记hash。原稿最多32MiB，通过现有配额、共享预算、coordinator及cleanup机制复制到私有snapshot。读取不启动解释器、外部URI下载、结构检查或作业执行。

描述包含URI、SHA-256内容标识、大小、MIME、接收来源、鉴权数据引用与最低保留引用，不包含原文、主机路径或凭证。响应private/no-store。内容标识不表示去重object store或永久可用。retainUntil可为null，expiresAt为null，不虚构删除时间。

integrityVerified只表示当前bytes匹配登记内容。structureVerified仅在存在匹配hash、大小、格式及job的strict PDF检查成功证据时为true；这是有界检查，不是PDF/A认证、全部内嵌对象检查或纸面质量认证。PS／EPS以boundedInterpretationVerified表示解释器证据，不宣称strict PDF结构检查。缺少成功证据时validation、pageCount为null，flag为false，不据此判定失败或从未检查；执行状态由job及event提供。有证据时返回页数、时间、scope、report引用及profile解析摘要。原稿可能有多种尺寸与颜色，当前不从输出配置推断原稿尺寸、颜色或ICC，相关字段为null。证据更新不修改ticket、requestHash或contentId。

REST原稿下载提供登记MIME、固定安全附件名、SHA-256 ETag与Range／If-Range，从验证snapshot配信，原文件后续替换不改变正在传输的bytes。MCP使用专用source/chunks模板，offset为规范非负整数，length为1至32768，返回有限base64 blob及ETag／Content-Range。逻辑原稿URI不是无限整体读取资源，应使用descriptor及chunk URI。每个分块仍验证整个文件并遵守原有配额；模板仅向同时具备两个scope的主体公开，不支持chunk订阅。

测试覆盖未知验证状态、等长篡改、格式不符、符号链接、缺失及超限、quota、传输不变性、范围恢复、重启、三种MIME、MCP范围与权限、实际MuPDF／Ghostscript前后证据。

🟠 [Next] 原稿各页尺寸／ICC、固定publication envelope及object-store／保留／转移契约仍未完成。

🟢 [Done] DeliveryArtifactの読取・検証 / Verified delivery reads / 已验证投递读取：[DELIVERY-ARTIFACT-DESCRIPTORS.md](DELIVERY-ARTIFACT-DESCRIPTORS.md)。

## PDF display geometry / PDFの表示寸法 / PDF显示尺寸

🟢 [Done] New strict-PDF worker validations persist `sourceGeometry` with profile `org.jrip.pdf-display-bounds-v1`, unit `point-1/72-inch`, meaning `crop-rotation-user-unit-display-bounds`, and an ordered `pages` array of `{page,widthPoints,heightPoints}`. The strict MuPDF inspection measures every source page before range selection, including when Ghostscript performs the later rasterization. Rust checks complete one-based ordering, 1–100 pages, finite positive dimensions ≤2834.6457 points and agreement with the validated source count. The worker-lease/source-hash-bound validation record and source descriptor expose the same data, including after restart. Reads do not run an interpreter. Legacy records, PS and EPS return null; output raster geometry is never substituted. These are effective display bounds, not separate MediaBox/TrimBox/BleedBox coordinates, output pixel dimensions, or an ICC assignment.

🟢 [Done] 新しいstrict PDF検証では全原稿ページの表示寸法を保存する。後段の描画がGhostscriptでも、厳密検査にはMuPDFを用いる。ページ範囲選択前の全ページを対象にし、順序・件数・有限の正寸法・最大寸法をRustでも検証する。単位は1/72インチのpoint、CropBox・回転・UserUnitを反映した表示範囲である。worker leaseと原稿hashに結び付く記録とdescriptorで同じ値を返し、再起動後も保持する。取得時に再解釈しない。旧記録・PS／EPSはnullとし、出力ラスターの寸法で推測しない。MediaBox／TrimBox／BleedBox個別座標やICC割当を証明する情報ではない。

🟢 [Done] 新strict PDF验证保存所有原稿页面的显示尺寸，即使后续由Ghostscript栅格化，严格检查仍使用MuPDF。在页范围筛选前检查全部页面；Rust验证连续页号、完整页数、有限正尺寸及上限。单位为1/72英寸point，反映CropBox、旋转和UserUnit。与worker lease及原稿hash绑定的记录和descriptor返回相同数据，重启后保留，读取时不重新解释。旧记录、PS及EPS为null，不以输出栅格尺寸推测。这不是各MediaBox／TrimBox／BleedBox坐标或ICC分配证明。

🟠 [Next] Per-page ICC usage and full source box semantics, PS/EPS geometry, fixed publication envelopes and storage lifecycle / ページ別ICC・個別box・PS／EPS寸法・公開envelopeと保存契約 / 逐页ICC、各box、PS／EPS尺寸、发布envelope与存储生命周期。

🟢 [Done] PDF page/resource ICC declaration associations / PDFページresourceのICC宣言対応 / PDF页面resource的ICC声明关联：[SOURCE-COLOR-PROFILES.md](SOURCE-COLOR-PROFILES.md#page-associations--ページ対応--页面关联)。`pageProfileDeclarations` includes indexed profiles and digest, not an executed-use claim; absent legacy/PS data remains null. / 使用実態の証明とは区別し、旧記録・PSの不明値はnull。 / 区分实际使用证明，旧记录及PS未知值为null。
