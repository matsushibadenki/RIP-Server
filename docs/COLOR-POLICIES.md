# Persistent Color Policy / 永続Color Policy / 持久Color Policy

## English

🟢 [Done] Immutable, digest-addressed color policies are stored in SQLite. Each
revision pins the input/output ICC profile hashes, transform settings and English,
Japanese and Simplified Chinese labels. It is a reusable explicit preset, not an
automatically applied global default. A change creates a new digest; there is no
in-place update or delete endpoint.

`POST /api/v1/color/policies` requires `settings:write` and accepts:

```json
{
  "definition": {
    "schemaVersion": 1,
    "labels": {"en": "Proof", "ja": "校正", "zh-CN": "校样"},
    "inputProfileSha256": "<registered 64-character lowercase hash>",
    "outputProfileSha256": "<registered 64-character lowercase hash>",
    "policy": {
      "schemaVersion": 1,
      "inputColor": "rgb", "outputColor": "rgb",
      "intent": "relative_colorimetric",
      "blackPointCompensation": false,
      "blackPreservation": "none", "tacPercent": null
    }
  },
  "reason": "Approved proof settings"
}
```

The profile envelopes, color spaces and existing Policy constraints are checked
before storage. DeviceLink uses a null output hash under the existing rules.
The response contains `profile:org.jrip.color-policy-v1`, raw hex `digest`,
`digestProfile:org.jrip.jcs-safe-v1`, definition, first creator, reason and time.
The digest is SHA-256 of the JCS definition only. Creator/reason/time are audit
metadata, not part of the content identity or a signature. Concurrent identical
registrations preserve the first record. The policy and a creation audit record
commit atomically; repeated registration does not create another policy revision.
HTTP access auditing remains separate. Registration means configured settings,
not measured gamut, successful transformation, human approval or output quality.

`GET /api/v1/color/policies?limit=50&after=<digest>` returns `policies` and
`nextAfter` (null on the last page), ordered by digest. Limit is 1–100.
`GET /api/v1/color/policies/{digest}` retrieves an exact revision and rechecks its
content digest. Both reads require `jobs:read`.

`POST /api/v1/jobs/{id}/color-attempts/from-policy` accepts only:

```json
{"requestId":"<uuid>","expectedRevision":3,"policyDigest":"<saved digest>"}
```

It requires `jobs:submit` and `artifacts:read`, loads the saved definition and
reuses the existing source-revision, ICC, artifact and queue checks. Inline overrides
are rejected. The resulting ticket pins `policyReference` and the full settings;
the worker result also carries that reference. Request-ID replay is idempotent;
reusing an ID with another policy conflicts. Direct inline-policy submission remains
supported. Existing per-transform `policySha256` retains its historical encoding;
it is distinct from the JCS registry digest covering labels and ICC references.

Tests exercise registration/replay, new revisions, rejected invalid settings and
missing profiles, scope checks, bounded cursor paging, restart persistence,
reference-based submission, replay conflicts and override rejection. Docker is not
needed for these API/storage checks.

⭕️ [Pending] Actual Docker conversion with a saved policy reference has not yet been completed. 🔴 [Later] Mutable named aliases, retirement/approval workflows,
automatic input-profile inference, interactive gamut visualization and artifact-retention integration.

## 日本語

🟢 [Done] ICC profileのhash・変換設定・3言語の名称をSQLiteへ不変のColor Policyとして保存します。
設定変更はJCS digestの異なる新版になり、既存版を上書きしません。暗黙の既定値にも設定しません。
上記POSTには `settings:write` と理由が必要です。ICCと設定を確認し、初回登録者・理由・時刻を保存します。
同じ定義の再登録は初回記録を維持し、policyと作成監査は同一トランザクションで保存します。
公開鍵署名や人の承認、色域・印刷品質の保証を意味しません。

一覧・版指定取得は `jobs:read`、保存版からの変換投入は `jobs:submit` と `artifacts:read` が必要です。
`from-policy` へrequestId・expectedRevision・policyDigestを渡し、上書き設定は受け付けません。
既存の原稿revision・ICC・artifact検査を通し、ticketへ設定のsnapshotと `policyReference` を記録します。
同一requestIdの再送は同じattemptへ戻り、別policyへの使い回しは拒否します。worker結果にも参照を残します。
従来のinline指定は維持し、従来の `policySha256` と新しいregistry digestは区別します。

登録・権限・版分離・cursor・再起動・再送競合をテストします。
⭕️ [Pending] 保存版を使う実変換の受入試験は未完了です。
🔴 [Later] 名前alias、廃止／承認workflow、入力profileの自動推定、色域の可視化、artifact保持期限との統合。

## 简体中文

🟢 [Done] 将ICC profile hash、转换设置及三种语言名称作为不可变Color Policy保存在SQLite。
设置变更生成新的JCS digest版本，不覆盖旧版，不自动设为全局默认值。
注册需要 `settings:write` 及原因，检查ICC和策略约束，原子保存policy及创建审计。
相同定义重放保留首次创建者、原因和时间；这些审计元数据不属于内容digest，也不表示签名、人工审批或质量保证。

列表及精确版本读取需要 `jobs:read`；`from-policy` 投入需要 `jobs:submit` 和 `artifacts:read`，
仅接受requestId、expectedRevision及policyDigest，不允许inline覆盖。复用现有原稿revision、ICC和artifact检查，
在ticket及worker结果中记录 `policyReference`。重复requestId返回同一attempt，换用其他policy则冲突。
原inline接口和历史 `policySha256` 编码保持不变，与新registry digest区分。

验证注册、权限、版本、cursor、重启及重放冲突。
⭕️ [Pending] 保存版策略的实际转换验收尚未完成。
🔴 [Later] 名称alias、停用／审批workflow、输入profile自动推断、色域可视化及artifact保留期限集成。
