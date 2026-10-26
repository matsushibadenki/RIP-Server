# ACX settings lifecycle / ACX設定lifecycle / ACX设置生命周期

## English

🟢 [Done] Discovery advertises the experimental `org.jrip.settings-lifecycle-v1`
workflow under `capabilities[].extensions.org.jrip.print.lifecycle`. Clients can
find preflight creation/retrieval, multipart commit, acceptance/result receipts and
source-validation evidence without guessing paths. The existing direct-upload
binding remains available. These are JRIP extension endpoints, not normative ACX
endpoints or a claim of full ACX conformance.

Use the existing PrintIntent for preflight. The stored v2 response binds the ticket,
actor, scope-check evidence, expiry and JCS digest. Commit uploads the document field
with `x-jrip-preflight-digest`; current Bearer scope and preflight ownership are
checked again. There is no separate transferable authorization token, and creating
preflight never grants execution or proves source structure. The document URI is a
reference, not an instruction to fetch arbitrary URLs.

The acceptance receipt means queued, not rendered or physically printed. The
asynchronous standard worker performs source validation and rendering; query the
result receipt for terminal evidence. A result receipt is absent while the job is
only queued. Completion for this capability means published raster artifacts,
not physical output. Receipt retrieval requires jobs:read; preflight/commit require
jobs:submit. Owner binding applies to preflights, not tenant isolation of every job.
Existing HTTP deployment/TLS and credential policy remain the operator's responsibility.

The integration test follows advertised paths, independently recomputes ticket and
preflight digests, checks wrong actor/digest/document rejection without job creation,
concurrent commit deduplication, restart replay and acceptance-receipt linkage.
It does not replace source-interpreter, transport or independent ACX schema testing.

🟢 [Done] Independent validation against pinned ACX Manifest/Receipt schema snapshots and
JRIP lifecycle profile bindings, including signed acceptance and all three terminal
statuses. Python checks schema/date formats; Node checks cross-record hashes and
pinned Ed25519 signatures. Re-signed inconsistent records are rejected. CI installs
the validator before ignored tests. See [fixture contract](../tests/acx/README.md).
Successful artifact metadata is synthetic; this validates the evidence publisher,
not interpreter output or physical printing.
⭕️ [Pending] Combined lifecycle acceptance through the real Docker interpreters
with the independent conformance checker is still outstanding; cross-provider interoperability and physical printer
acceptance remain separate work.

## 日本語

🟢 [Done] discoveryのnamespaced extensionに実験的lifecycle経路を公開しました。
preflight作成・取得、multipart commit、受付／結果Receipt、原稿検証証跡をAgentが発見できます。
既存の直接投入bindingも維持します。JRIP独自extensionであり、ACX全体への適合宣言ではありません。

preflightのticket・主体・認可証跡・期限をJCS digestへ拘束し、commitで現在のscopeと所有主体を再確認します。
別の実行tokenは発行せず、preflightだけでは実行許可や原稿構造の正常性を保証しません。
原稿URIを自動取得する機能ではありません。受付Receiptはキュー投入を意味し、描画完了や実機印刷完了とは区別します。
標準workerが原稿検証と描画を行い、結果Receiptで終端の証跡を取得します。

公開経路を使う統合試験でdigest独立再計算、主体／digest／原稿不一致の拒否、同時commitの重複排除、
再起動後の再送、Receiptの関連付けを検証します。全jobのtenant分離や独立ACX schema検証とは区別します。
🟢 [Done] ACX schema固定コピーとPython／Nodeで受付・成功・失敗・取消証跡を独立検証します。
署名・digest・記録間の関連を照合し、再署名された契約不一致も拒否します。CIにも組み込みました。
成功artifactは模擬metadataで、実描画・実機印刷の検証とは区別します。
⭕️ [Pending] 独立適合checkerと実interpreterを組み合わせた統合lifecycle受入試験。

## 简体中文

🟢 [Done] discovery的namespaced extension公开实验性lifecycle路径，包含预检创建／读取、multipart提交、
接收／结果回执及原稿验证证据。保留现有直接上传binding；这是JRIP扩展，不宣称完整ACX符合性。

预检将ticket、身份、scope证据和有效期绑定到JCS摘要；提交时重新检查当前scope和归属。
不发放独立执行token；预检不等于执行授权或原稿结构验证，也不会自动获取原稿URI。
接收回执仅表示入队。标准worker执行原稿验证和渲染，终态结果回执证明光栅artifact发布，不表示实体打印。

集成测试沿公开路径独立重算摘要，检查身份／摘要／原稿不匹配拒绝、并发提交去重、重启重放及回执关联。
不将这些测试视为全部job的tenant隔离或独立ACX schema验证。
🟢 [Done] 用固定ACX schema及Python／Node独立验证接收、成功、失败及取消证据。
核对签名、digest及记录关联，拒绝重新签名的契约不一致，并纳入CI。成功artifact为模拟元数据，不证明实际渲染或打印。
⭕️ [Pending] 独立符合性checker与实际interpreter组合的完整lifecycle验收。
