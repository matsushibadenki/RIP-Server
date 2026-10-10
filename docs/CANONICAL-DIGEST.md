# Portable canonical digest

## English

`org.jrip.jcs-safe-v1` uses [RFC 8785 JCS](https://www.rfc-editor.org/rfc/rfc8785): compact UTF-8, UTF-16 property ordering, unchanged Unicode, ECMAScript number serialization (including negative zero), preserved array order. JRIP adds two input limits: depth ≤64 and every numeric value finite with absolute value ≤9,007,199,254,740,991. Large identifiers must be strings. This is a bounded profile of JCS, not support for its entire numerical range.

The shared `rip-digest` crate provides `canonical(Value)`, `digest(Value)` and `parse(bytes)`. `parse` rejects duplicate properties (including escaped equivalent names), invalid Unicode and non-JSON numbers. Structured-value callers must reject duplicates before building a Value: information discarded by another parser cannot be recovered. Float parsing enables serde_json's round-trip mode. Digest text is `sha256:` followed by lowercase hex SHA-256 over canonical bytes, with no domain prefix.

```sh
cargo run -p rip-digest --bin jrip-digest -- input.json
node scripts/check-canonical-digest.mjs target/debug/jrip-digest crates/rip-digest/tests/fixtures/portable.json
cargo test -p rip-storage --test jcs_interop -- --ignored
```

The CLI caps input at 1 MiB. The Node scripts are independent conformance checks on valid fixtures; their JSON.parse input is not a production duplicate-rejecting parser. The signature check uses a separately supplied raw public key and rejects tampering; it checks cryptography, not revocation.

New Receipt signatures use `org.jrip.receipt-ed25519-v2` and `signature.digestProfile:org.jrip.jcs-safe-v1`. Their message is the UTF-8 signature profile, one zero byte, and canonical bytes of the whole receipt after removing only signature.value. Legacy v1 signatures still verify using their original serde_json representation and domain. Stored receipts are never rewritten. Existing requestHash/resultHash fields remain governed by their existing proof digestProfile; the v2 signature canonicalizes the complete receipt, including those legacy hashes.

`GET /api/v1/pipelines/{id}/ticket` and existing MCP `print_get_pipeline_ticket` add `portableDigest:{profile,sha256,available}` for the same immutable `ticket` object. Unsupported canonical input yields available:false and CANONICAL_INPUT_UNSUPPORTED; the original ticketSha256 stays unchanged. Execution state is excluded from both ticket hashes. Storage identities and raw-file hashes retain their prior contracts.

New settings and physical pipeline preflights default to `org.jrip.settings-preflight-v2` / `org.jrip.pipeline-preflight-v2`, paired with `digestProfile:org.jrip.jcs-safe-v1`. Both requestHash and preflightDigest use that selected profile; preflightDigest excludes only its own field. Creation accepts `X-JRIP-Digest-Profile: org.jrip.serde-json-v1` to explicitly select legacy v1, or the JCS profile to select v2. Unknown or repeated headers are rejected. MCP `print_create_preflight` and `print_create_pipeline_preflight` expose the optional `digestProfile` enum. Commit uses the persisted profile, rejects mismatched profile pairs, and never renegotiates from a commit header. Persisted v1 records remain usable without rewriting. Acceptance proofs record requestHashDigestProfile and preflightDigestProfile separately; raster resultHash retains its existing proof digestProfile.

## 日本語

共通profile `org.jrip.jcs-safe-v1` はRFC 8785に基づき、UTF-16でのキー順序、数値表現、負のゼロ、Unicodeと配列順序を統一します。追加制限は深さ64以内、全数値の絶対値9,007,199,254,740,991以下です。大きな識別子は文字列にしてください。JCSの全数値範囲には対応しません。

Rust共通crateとCLIを追加しました。厳密parserは重複キー・不正Unicode・非JSON数値を拒否します。他のparserで失われた重複情報は復元できません。新しいReceipt署名はv2とJCSを使い、過去のv1署名も元の規則で検証します。Receipt本体の既存requestHash/resultHashは、それぞれの既存profileを保持します。

共通ticketのAPI/MCPには `portableDigest` を追加します。元のticket hash・ファイルhashは変更しません。新規preflightはsettings/pipelineのv2とJCSを既定とし、作成時の `X-JRIP-Digest-Profile` またはMCPの `digestProfile` で旧serde-json-v1を選択できます。requestHashとpreflightDigestは選択した方式で計算します。commitは保存済みprofileを使い、不正な組み合わせを拒否します。保存済みv1は書き換えず利用できます。受付Receiptは各hashの方式を個別に記録します。Rust/JavaScriptのhash一致とNodeによる署名・改ざん検出を検証します。参照Nodeスクリプトは適正なfixture向けの相互運用確認用であり、本番の厳密JSON parserや失効検証の代わりにはなりません。

## 简体中文

公共profile `org.jrip.jcs-safe-v1` 基于RFC 8785，统一UTF-16键排序、数字格式、负零，并保留Unicode和数组顺序。附加限制为深度≤64且全部数字绝对值≤9,007,199,254,740,991。大型标识符须使用字符串；不覆盖JCS全部数字范围。

增加共享Rust crate与CLI。严格parser拒绝重复键、无效Unicode及非JSON数字；不能恢复其他parser已丢弃的重复信息。新回执签名采用v2及JCS，历史v1仍按原规则验证。回执原有requestHash/resultHash继续使用各自旧profile。

统一ticket API/MCP增加portableDigest，原ticket hash及文件hash不变。新预检默认采用settings/pipeline v2及JCS；创建时可通过 `X-JRIP-Digest-Profile` 或MCP的 `digestProfile` 选择旧serde-json-v1。requestHash和preflightDigest均采用所选规则。提交按已保存的profile验证并拒绝不匹配组合，旧v1记录无需重写。接收回执分别记录各摘要规则。验证Rust/JavaScript摘要一致性及Node签名/篡改检测。Node参考脚本仅用于有效fixture的互操作检查，不代替生产严格JSON parser或撤销验证。
