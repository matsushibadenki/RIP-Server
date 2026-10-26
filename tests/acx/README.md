# Independent ACX fixture contract

These two schemas are byte-for-byte snapshots of the linked development ACX
repository's `schemas/acx-manifest.schema.json` and `schemas/acx-receipt.schema.json`
on 2026-10-19. They are committed locally so CI does not depend on a developer's
symlink or silently change when the evolving ACX checkout changes.

| File | SHA-256 |
|---|---|
| acx-manifest.schema.json | 24bec5a954ee691d0ef3d7c048789b45a4365198c08bde4024db1d3aaeda6bae |
| acx-receipt.schema.json | 920568d271ff26309a76f2eda551f1c36ed1113ec8446bb2770d6d56504d46ef |

Run `python3 -m pip install -r tests/acx/requirements.txt`, install Node.js, then:

```sh
cargo test -p jrip-server --test acx_lifecycle --locked -- --ignored
```

Python Draft202012Validator checks schemas and date formats. Node independently
checks profile bindings, hash construction, and pinned Ed25519 signatures. Rust
creates real preflight/commit/terminal records with the production repository and
signer; success uses explicitly synthetic artifact metadata, not a renderer.
Negative cases are re-signed with the test key: valid signatures cannot conceal
invalid cross-record identities, hashes, source identity, status or completion meaning.

The checker consumes trusted test output, not arbitrary hostile JSON. Passing
these snapshots and JRIP's experimental lifecycle checks is not normative ACX
certification, provider authenticity without a trusted key, actual TIFF validation,
or physical-print acceptance. Broader canonicalization edge cases have separate
JCS vectors and tests.

日本語：開発中ACXのschemaを上記hashで固定し、CIをローカルsymlinkから独立させます。
Pythonでschemaと日時、Nodeでprofile間の関連・digest・信頼済み公開鍵署名を検証します。
改ざんケースもテスト鍵で再署名し、正しい署名が契約不一致を隠さないことを確認します。
成功artifactは模擬情報であり、ACX認証・描画品質・実機印刷の証明ではありません。

简体中文：按上述hash固定开发中ACX的schema，使CI不依赖本地symlink。
Python检查schema及日期，Node检查记录关联、digest及可信公钥签名。
负例用测试密钥重新签名，验证有效签名不能掩盖契约不匹配。
成功artifact为模拟元数据，不证明ACX认证、渲染质量或实体打印。

Date-time validation requires the explicitly pinned `rfc3339-validator` dependency.
`jsonschema` alone may silently omit its optional `date-time` checker. The lifecycle
checker imports and registers the required validator explicitly, so missing support
fails instead of accepting malformed `issuedAt`. CI runs this regression suite before
the signed lifecycle fixtures:

```sh
python3 -m unittest discover -s tests/acx -p 'test_*.py' -v
```

日本語：`jsonschema`単体では任意依存の日時検証が省略される場合があります。
`rfc3339-validator`を必須依存として固定し、明示的に登録します。欠落時には失敗し、
不正な`issuedAt`を通しません。正常日時、不正日時、依存欠落をCIで回帰検証します。

简体中文：仅安装`jsonschema`时可能静默省略可选的日期验证。
固定必需依赖`rfc3339-validator`并显式注册；缺失时直接失败，不接受错误的`issuedAt`。
CI回归测试覆盖有效日期、无效日期及依赖缺失。
