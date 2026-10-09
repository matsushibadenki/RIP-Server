# Explicit color processing / 明示的色処理 / 显式颜色处理

`rip-color` is a separate Rust library and CLI backed by statically built LittleCMS 2. It accepts explicit ICC bytes, 8-bit RGB/CMYK/Gray samples and a versioned policy. Four ICC rendering intents, black point compensation, CMYK K-only/K-plane preservation, direct DeviceLink and CMYK TAC are represented explicitly. Absolute colorimetric with black preservation is rejected; DeviceLink already defines its own mapping and rejects an additional black-preservation or black-point-compensation request. TAC is limited to 100–400%, uses a LittleCMS ink-limiting link and caps 8-bit quantization while retaining K and CMY proportions. This does not implement overprint, trapping, separation previews, gamut reports, DeviceN or 16-bit processing.

日本語：`rip-color` は独立したRustライブラリ／CLIです。ICCを明示指定し、8-bit RGB／CMYK／Gray、4種類のレンダリングインテント、黒点補正、CMYKのK-only／K-plane保持、DeviceLink、TACを扱います。絶対測色＋黒版保持や、DeviceLinkへの追加の黒点補正／黒版保持は黙って変更せず拒否します。TACは100–400%で、LittleCMSのインキ制限と8-bit量子化上限を適用します。オーバープリント、Trapping、分版プレビュー、gamut解析、DeviceN、16-bitは未実装です。

简体中文：`rip-color` 是独立Rust库／CLI，显式指定ICC，支持8-bit RGB／CMYK／Gray、四种渲染意图、黑点补偿、CMYK的K-only／K-plane保持、DeviceLink和TAC。拒绝绝对色度＋黑版保持及对DeviceLink追加黑点补偿／黑版保持，不会静默替换。TAC为100–400%，应用LittleCMS墨量限制及8-bit量化上限。尚未实现叠印、Trapping、分色预览、gamut分析、DeviceN及16-bit。

Example policy / 設定例 / 示例配置:

```json
{
  "schemaVersion": 1,
  "inputColor": "rgb",
  "outputColor": "rgb",
  "intent": "relative_colorimetric",
  "blackPointCompensation": true,
  "blackPreservation": "none",
  "tacPercent": null
}
```

```sh
cargo run --locked -p rip-color -- input.tiff input.icc output.icc policy.json 300 output.tiff > color-report.json
```

For DeviceLink use the link as `input.icc` and `-` instead of an output profile. The TIFF adapter accepts one page, rejects non-8-bit layouts, multiple images, rotated orientation, wrong DPI and embedded source profiles that do not match the supplied ICC. Geometry is preserved. A normal output profile is embedded into TIFF; a DeviceLink is not embedded as a normal output profile. Publication uses a private staging file and refuses overwriting an existing output. Profiles are bounded to 4 MiB with ICC signature/declared-size/tag-range checks; input and output sample buffers are each bounded to 64 MiB. The CLI also bounds source-file reads and policy JSON. The CLI itself is not an OS sandbox. API-triggered transforms run through the isolated Color Worker described below.

日本語：DeviceLinkは入力ICCに指定し、出力ICCには `-` を指定します。TIFFは1ページのみで、8-bit以外、複数画像、回転、DPI不一致、埋め込みICCとの不一致を拒否します。寸法を保持し、通常の出力ICCを埋め込みます。DeviceLinkは通常ICCとして埋め込みません。私有一時ファイルから公開し、既存出力を上書きしません。ICCは4MiB、サンプルは入力・出力それぞれ64MiBを上限とします。CLI自体はOSサンドボックスではありません。APIからの変換は下記の隔離Color Workerで実行します。

简体中文：DeviceLink作为输入ICC，输出ICC使用 `-`。TIFF仅接受单页，拒绝非8-bit、多图像、旋转、DPI不符及内嵌ICC不匹配，保留尺寸并嵌入普通输出ICC；DeviceLink不作为普通ICC嵌入。通过私有临时文件发布，拒绝覆盖已有输出。ICC最大4MiB，输入／输出采样缓冲各最大64MiB。CLI本身不是OS沙箱。API触发的转换通过下述隔离Color Worker执行。

The report records input/output profile SHA-256, source/output sample SHA-256, policy SHA-256, pixel count, DeviceLink use and maximum CMYK ink percentage. The policy digest is the fixed Rust JSON encoding, not ACX canonicalization. Tests cover sRGB identity, geometry/profile preservation, malformed envelopes, unsupported settings, DeviceLink TAC and pure-K preservation. Dedicated K-only/K-plane quality acceptance with real printer profiles remains Pending. The RIP worker keeps its original output; an explicit color attempt generates a separate registered result. Delivery accepts that result only when its profile matches the administrator-configured printer target. The ordinary PWG conversion entry point still rejects ICC-tagged TIFF; the registered entry point verifies the exact expected ICC hash.

日本語：レポートにprofile・サンプル・設定のhash、画素数、DeviceLink利用、最大インキ量を記録します。設定digestはRustの固定JSON形式でACX共通形式ではありません。sRGB一致、寸法／ICC保持、異常入力、DeviceLink TAC、純K保持を検証済みです。実機profileでのK-only／K-plane品質試験はPendingです。RIPワーカーの原本出力を保持し、明示的な色処理attemptで別成果物を生成します。Deliveryは管理者のプリンターprofile設定と一致する色成果物だけを扱います。通常PWG入口はICC付きTIFFを拒否し、登録済み入口で期待ICC hashを照合します。

简体中文：报告记录profile、采样及策略hash、像素数、DeviceLink使用情况和最大墨量。策略digest为固定Rust JSON，不是ACX通用编码。已测试sRGB一致、尺寸／ICC保持、异常输入、DeviceLink TAC和纯K保持。真实打印机profile的K-only／K-plane质量验收为Pending。保留RIP原始输出，通过显式颜色尝试生成独立成果物。Delivery仅接受与管理员配置的打印机profile一致的结果。普通PWG入口拒绝带ICC的TIFF，注册入口验证预期ICC hash。

Implementation reference: [LittleCMS Rust profile API](https://github.com/kornelski/rust-lcms2/blob/master/src/profile.rs), [transform API](https://docs.rs/lcms2/latest/lcms2/struct.Transform.html).

The administrator CLI can register immutable ICC bytes and inspect their metadata. The identifier is the exact bytes' SHA-256. Duplicate registration returns the same record; reads verify size, envelope and digest, rejecting corruption and symlinks. Keep the registry root administrator-owned. This registry checks ICC structure without invoking LittleCMS; semantic parsing happens during transformation. DeviceLink policy intent must match the link's header intent.

日本語：管理者CLIでICCを登録・照会できます。識別子は原本bytesのSHA-256で、同じICCは同じ記録になります。読み出し時に容量・構造・hashを確認し、破損とシンボリックリンクを拒否します。登録rootは管理者のみが変更できるようにしてください。登録ではICC構造を確認し、LittleCMSによる意味の解析は変換時に行います。DeviceLinkの指定インテントはICCヘッダーと一致する必要があります。

简体中文：管理员CLI可注册及查询ICC，标识为原始bytes的SHA-256，重复注册返回相同记录。读取时校验大小、结构和hash，拒绝损坏及符号链接。注册root应仅允许管理员修改。注册只检查ICC结构，LittleCMS语义解析在转换时进行。DeviceLink策略意图必须与ICC头一致。

```sh
cargo run --locked -p rip-color -- register ./profiles ./output.icc
cargo run --locked -p rip-color -- describe ./profiles <sha256>
```


## API and isolated worker / APIと隔離Worker / API与隔离Worker

Build and run / 構築・起動 / 构建及启动:

```sh
docker build -f adapters/color/Dockerfile -t jrip-color:local .
cargo run --locked -p jrip-server --bin jrip-color-worker
```

The worker uses the same `JRIP_DATA_ROOT` and `JRIP_API_TOKEN` as the API. The API never invokes Docker. Profile registration performs bounded Rust envelope checks; native ICC parsing and pixel transformation occur in a networkless, read-only container with capabilities dropped, non-root host UID, 384 MiB memory, one CPU, 32 processes, bounded file sizes, 80 MiB output tmpfs and a 120-second page timeout. Only a two-file archive (`result.tiff`, `report.json`) is accepted. Each page is verified against the original artifact hash; output geometry, color and embedded ICC are checked. Total published color TIFF is bounded to 1 GiB. Administrator-owned data roots are required; the shared snapshot budget does not include all color staging bytes.

日本語：APIと同じ `JRIP_DATA_ROOT`／`JRIP_API_TOKEN` を使用します。APIはDockerを呼びません。ICC登録はRustで容量・構造を確認し、ネイティブICC解析・画素変換はネットワークなし、root FS読み取り専用、capabilityなし、非root UID、384MiB、CPU 1、32 process、ファイル上限、出力tmpfs 80MiB、1ページ120秒のコンテナーで行います。成果物hashと寸法・色型・埋め込みICCを確認し、色TIFFの合計は1GiBまでです。データrootは管理者所有が必要で、共有snapshot予算に全Color staging容量が含まれるわけではありません。

简体中文：使用与API相同的 `JRIP_DATA_ROOT`／`JRIP_API_TOKEN`。API不调用Docker。ICC注册通过Rust检查容量及结构，原生ICC解析和像素转换在无网络、只读root FS、无capability、非root UID、384MiB内存、单CPU、32进程、文件限制、80MiB输出tmpfs及每页120秒超时的容器内执行。校验成果物hash、尺寸、色型及内嵌ICC，颜色TIFF总计最大1GiB。数据root须由管理员管理；共享snapshot预算不包含全部Color staging容量。

| Endpoint | Scope | Behavior / 動作 / 行为 |
| --- | --- | --- |
| `GET /api/v1/color/capabilities` | `jobs:read` | Supported constraints, revision and expiry / 能力・制約・版・期限 / 能力、约束、revision及期限 |
| `POST /api/v1/color/profiles` | `settings:write` | Raw ICC body, 4 MiB maximum; immutable registration / ICC原本登録 / 注册ICC原始bytes |
| `GET /api/v1/color/profiles/{sha256}` | `jobs:read` | Verified profile metadata / 登録情報 / 注册元数据 |
| `POST /api/v1/jobs/{id}/color-attempts` | `jobs:submit` + `artifacts:read` | Queue completed RIP source with pinned revision and policy / 完了RIPを版固定して予約 / 固定revision与策略后排队 |
| `GET /api/v1/color/attempts/{id}` | `jobs:read` | Durable state, ticket and result / 永続状態・結果 / 持久状态及结果 |
| `GET /api/v1/color/attempts/{id}/events?after=0&limit=50` | `jobs:read` | Resumable per-page events / ページ履歴 / 可恢复页事件 |
| `POST /api/v1/color/attempts/{id}/cancel` | `jobs:control` | Expected revision; queued attempts only / 待機中のみ取消 / 仅取消排队尝试 |
| `GET /api/v1/color/attempts/{id}/artifacts/{name}` | `artifacts:read` | Full hash verification, private snapshot, ETag and single HTTP Range / hash検証配信 / hash验证下载 |

Example request / 投入例 / 提交示例:

```json
{
  "requestId": "6b315f81-89db-4d3e-8b2a-d2816e974dc1",
  "expectedRevision": 4,
  "inputProfileSha256": "<registered lowercase SHA-256>",
  "outputProfileSha256": "<registered lowercase SHA-256>",
  "policy": {
    "schemaVersion": 1,
    "inputColor": "rgb",
    "outputColor": "rgb",
    "intent": "relative_colorimetric",
    "blackPointCompensation": true,
    "blackPreservation": "none",
    "tacPercent": null
  }
}
```

For DeviceLink omit the output hash (or use JSON null). Identical request UUID/ticket replays the same attempt; changed content conflicts. States are QUEUED → RUNNING → COMPLETED/FAILED, with queued cancellation. A single host worker lock prevents concurrent passes. Per-page CAS updates renew a 600-second lease; expired running attempts are cleaned up and failed, never silently retried. A new request UUID is required for a new attempt. Color events carry completed pages, total pages, progress, revision and reason; they never mean physical print completion. Active color attempts block source deletion and retention. Maintenance can remove abandoned `color-stage-*` directories older than one hour only when their writer lock is free. Profile files remain immutable and have no automatic garbage collection.

日本語：DeviceLinkは出力hashを省略またはnullにします。同じUUID／ticketは同じattemptを返し、変更は競合です。待機→実行→完了／失敗で、待機中のみ取消できます。単一ホストのWorker lockとrevisionで競合を防ぎ、ページごとに600秒leaseを更新します。期限切れはコンテナーを回収して失敗にし、自動再試行しません。新試行には新UUIDを使います。進捗は色TIFFの生成を示し、実機印刷完了ではありません。実行・待機中は原本削除／保持期限清掃を防ぎます。1時間超の孤立stagingはwriter lock取得時のみ清掃します。ICC登録の自動削除はありません。

简体中文：DeviceLink省略输出hash或使用null。同一UUID／ticket返回同一尝试，修改内容冲突。排队→运行→完成／失败，仅排队时可取消。单主机Worker lock和revision防止冲突，每页更新600秒lease。过期尝试清理容器后标记失败，不自动重试；新尝试使用新UUID。进度表示颜色TIFF生成，不代表实体打印完成。活动尝试阻止原稿删除及保留清理；超过一小时的孤立staging仅在writer lock空闲时清理。ICC注册没有自动垃圾回收。

MCP exposes `color_get_capabilities`, `color_apply`, `color_get_attempt`, `color_get_events`, `color_cancel_attempt`, `color_get_profile`; submission requires both `jobs:submit` and `artifacts:read`. `print_deliver_job` accepts `colorAttemptId`. Registration remains an administrator API/CLI operation; MCP never accepts inline ICC or an arbitrary host path.

日本語：MCPに上記6つの色処理toolを追加しました。実機送信に `colorAttemptId` を指定できます。登録は管理API／CLIで行い、MCPはICC本文や任意ホストパスを受け付けません。

简体中文：MCP提供上述六个颜色工具，实体投递可指定 `colorAttemptId`。注册通过管理API／CLI执行，MCP不接受ICC正文或任意主机路径。

Each page records `sourceProfileEvidence`: `embedded-match` means the source TIFF ICC matched the supplied hash; `explicit-unembedded` means the source had no ICC and the input profile is an operator assumption, accompanied by `SOURCE_PROFILE_UNEMBEDDED`. This does not prove the RIP engine's original implicit color conversion. / 各ページにICC埋め込み一致または未埋め込み時の明示指定を記録します。未埋め込みは操作者の判断として警告し、RIPエンジンの元の暗黙色変換を証明するものではありません。 / 每页记录ICC内嵌匹配或无内嵌时的显式指定；后者属于操作者判断并附警告，不证明RIP引擎原先的隐式颜色转换。

If Docker container removal fails or times out, the attempt remains FAILED and a durable cleanup marker is retained. Subsequent worker passes retry only container removal; they do not retry color conversion. Source deletion/retention stays blocked until removal succeeds. / コンテナー回収失敗・タイムアウトはFAILEDと永続清掃記録に残し、後続Workerは回収のみを再試行します。色変換を再実行せず、回収成功まで原本削除・保持期限清掃を防ぎます。 / 容器清理失败或超时会保留FAILED及持久清理标记。后续Worker仅重试清理，不重试颜色转换；清理成功前阻止原稿删除及保留清理。
