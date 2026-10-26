# MCP artifact chunks

🟢 [Done] `jrip://jobs/{jobId}/artifacts/{name}/chunks/{offset}/{length}`

English: `resources/templates/list` advertises this resource only to identities with `artifacts:read`. Read it through `resources/read`; `name` is a registered `page-NNNNNN.tiff`, `offset` is a canonical unsigned decimal byte offset, and `length` is 1–32768 bytes. Responses use MCP base64 `blob` with `application/octet-stream`. The last chunk may be shorter. `_meta["org.jrip.artifact-chunk-v1"]` contains the SHA-256 ETag, HTTP Content-Range (including total bytes), and returned byte count. Read successive offsets and require the same ETag when assembling a file. Each read delegates to the authenticated REST artifact API, which verifies the entire immutable snapshot before returning the range. This bounds MCP response size, but each read still hashes the full artifact and uses the existing snapshot quotas. Out-of-range, missing, forbidden, or modified files fail without exposing file paths. No host path, query, arbitrary MIME, source document, or color-attempt artifact is accepted. Chunk resources do not support subscriptions. Existing event resources remain available for progress.

日本語：`artifacts:read` 権限を持つ主体だけにテンプレートを公開します。`resources/read` で登録済みの `page-NNNNNN.tiff` を取得します。offsetは先頭ゼロのない非負整数、lengthは1〜32768バイトです。最後のチャンクは短くなる場合があります。応答はMCP標準のbase64 `blob`。上記のメタデータにSHA-256 ETag、全体サイズを含むContent-Range、取得バイト数を返します。結合時はETagの一致を確認してください。各取得は既存の認証付きREST APIを利用し、ファイル全体のハッシュを検証したsnapshotから範囲を返します。応答量を制限しますが、毎回の全体検証と既存の容量制限は必要です。不正範囲・欠落・権限不足・改変はエラーになります。ホストパス、任意query、原稿、Color成果物には対応しません。チャンクの購読は行わず、進捗は既存イベントresourceで取得します。

简体中文：仅向拥有 `artifacts:read` 权限的身份公开模板。使用 `resources/read` 读取已注册的 `page-NNNNNN.tiff`。offset为无前导零的非负整数，length为1至32768字节。末尾分块可能较短。响应使用MCP标准base64 `blob`；上述元数据包含SHA-256 ETag、含总大小的Content-Range及返回字节数。拼接时应检查ETag一致。每次读取通过现有认证REST API验证完整文件的hash，再从snapshot返回范围。响应大小有界，但每次仍需完整验证并遵守snapshot配额。越界、缺失、权限不足或文件改动均失败。不支持主机路径、任意query、原稿或Color产物。分块不支持订阅，进度通过现有事件resource读取。

🔴 [Later] Color-attempt binary resources and efficient reusable verified snapshots / Color成果物と検証済みsnapshotの再利用 / Color产物及已验证snapshot复用。
