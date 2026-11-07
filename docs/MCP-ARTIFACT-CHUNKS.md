# MCP artifact chunks

🟢 [Done] `jrip://jobs/{jobId}/artifacts/{name}/chunks/{offset}/{length}`

English: `resources/templates/list` advertises this resource only to identities with `artifacts:read`. Read it through `resources/read`; `name` is a registered `page-NNNNNN.tiff`, `offset` is a canonical unsigned decimal byte offset, and `length` is 1–32768 bytes. Responses use MCP base64 `blob` with `application/octet-stream`. The last chunk may be shorter. `_meta["org.jrip.artifact-chunk-v1"]` contains the SHA-256 ETag, HTTP Content-Range (including total bytes), and returned byte count. Read successive offsets and require the same ETag when assembling a file. Each read delegates to the authenticated REST artifact API, which verifies the entire immutable snapshot before returning the range. This bounds MCP response size, but each read still hashes the full artifact and uses the existing snapshot quotas. Out-of-range, missing, forbidden, or modified files fail without exposing file paths. This raster URI accepts no host path, query, arbitrary MIME, source document, or color-attempt artifact; sources use the separate template below. Chunk resources do not support subscriptions. Existing event resources remain available for progress.

日本語：`artifacts:read` 権限を持つ主体だけにテンプレートを公開します。`resources/read` で登録済みの `page-NNNNNN.tiff` を取得します。offsetは先頭ゼロのない非負整数、lengthは1〜32768バイトです。最後のチャンクは短くなる場合があります。応答はMCP標準のbase64 `blob`。上記のメタデータにSHA-256 ETag、全体サイズを含むContent-Range、取得バイト数を返します。結合時はETagの一致を確認してください。各取得は既存の認証付きREST APIを利用し、ファイル全体のハッシュを検証したsnapshotから範囲を返します。応答量を制限しますが、毎回の全体検証と既存の容量制限は必要です。不正範囲・欠落・権限不足・改変はエラーになります。このラスターURIはホストパス、任意query、原稿、Color成果物に対応しません。原稿は下記の専用URIを使います。チャンクの購読は行わず、進捗は既存イベントresourceで取得します。

简体中文：仅向拥有 `artifacts:read` 权限的身份公开模板。使用 `resources/read` 读取已注册的 `page-NNNNNN.tiff`。offset为无前导零的非负整数，length为1至32768字节。末尾分块可能较短。响应使用MCP标准base64 `blob`；上述元数据包含SHA-256 ETag、含总大小的Content-Range及返回字节数。拼接时应检查ETag一致。每次读取通过现有认证REST API验证完整文件的hash，再从snapshot返回范围。响应大小有界，但每次仍需完整验证并遵守snapshot配额。越界、缺失、权限不足或文件改动均失败。此栅格URI不支持主机路径、任意query、原稿或Color产物；原稿使用下面的独立模板。分块不支持订阅，进度通过现有事件resource读取。

🔴 [Later] Efficient reusable verified snapshots / 検証済みsnapshotの効率的な再利用 / 已验证snapshot的高效复用。


🟢 [Done] Verified source chunks: `jrip://jobs/{jobId}/source/chunks/{offset}/{length}`. Both jobs:read and artifacts:read are required. The same 1–32768-byte bound, canonical range syntax, private full-file snapshot verification and ETag metadata apply. Source metadata is available through `print_describe_source` and `jrip://jobs/{jobId}/source/descriptor`; no source interpreter runs on read. See [SOURCE-DESCRIPTORS.md](SOURCE-DESCRIPTORS.md).

🟢 [Done] 原稿の専用chunk URIを追加。jobs:readとartifacts:readの両方が必要で、1〜32768バイト・全体hash検証・私有snapshot・ETagを維持する。descriptorと成功した構造検証証跡は別に取得し、読取時に原稿を実行しない。

🟢 [Done] 增加原稿专用chunk URI，同时需要jobs:read与artifacts:read，保留1至32768字节、全体hash验证、私有snapshot和ETag。descriptor与成功验证证据另行读取，不在读取时执行原稿。

🟢 [Done] Delivery PWG chunks: `jrip://deliveries/{deliveryId}/artifact/chunks/{offset}/{length}`. Both jobs:read and artifacts:read; canonical ranges and 1–32768-byte bounds. Every read verifies the full PWG snapshot and saved ticket. / 配信PWGも両scope・全体検証・有界chunkに対応。 / 投递PWG支持双scope、全体验证与有界分块。See [DELIVERY-ARTIFACT-DESCRIPTORS.md](DELIVERY-ARTIFACT-DESCRIPTORS.md).

🟢 [Done] Color-managed chunks: `jrip://color/attempts/{colorAttemptId}/artifacts/{name}/chunks/{offset}/{length}`. Like the color REST download, this requires artifacts:read. Only completed attempts and registered canonical TIFF names are readable. Each chunk delegates to the private, hash-verified REST snapshot; 1–32768-byte bounds, canonical offsets, ETag/Content-Range and no subscriptions apply. Reads perform no color conversion or printing. Metadata is available separately through color_describe_artifact (jobs:read + artifacts:read).

🟢 [Done] 色変換成果物の専用URIを追加。REST取得と同じartifacts:readを要求し、完了attemptの登録済みTIFFだけを返す。1〜32768バイト・正規offset・全体hash検証・私有snapshot・ETag／Content-Rangeを維持する。読取による色変換・印刷は行わず、購読対象にはしない。descriptorは別途color_describe_artifactで取得し、こちらはjobs:readも必要。

🟢 [Done] 色转换产物使用专用URI，需要artifacts:read，仅支持已完成attempt中登记的规范TIFF名称。每次验证完整hash并使用私有snapshot，保留1至32768字节限制、规范offset、ETag／Content-Range。不执行色转换或打印，不支持订阅。描述通过color_describe_artifact另行读取，并同时要求jobs:read。
