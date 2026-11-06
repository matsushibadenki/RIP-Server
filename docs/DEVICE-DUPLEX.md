# Device duplex / 実機向け両面設定 / 设备双面设置

🟢 [Done] Explicit delivery preview, delivery submission, follow-on pipelines and
physical PrintIntent preflight accept `sides`:

```json
{"sides":"two-sided-long-edge"}
```

Values: `one-sided` (default), `two-sided-long-edge`, `two-sided-short-edge`.
REST: `POST /api/v1/jobs/{id}/delivery-preview`, `/deliveries`, `/pipelines`.
MCP: `print_preview_delivery`, `print_deliver_job`, `print_create_pipeline`.
For `POST /api/v1/print/pipeline-preflights`, use the existing requirement form:

```json
{"sides":{"mode":"required","value":"two-sided-short-edge"}}
```

日本語: 明示Delivery・後続pipeline・実機PrintIntentで片面／長辺／短辺とじを指定できます。省略は片面で、プリンターの属性順から自動的に両面へ変更しません。MCPにも同じ設定を追加しています。

简体中文: 显式Delivery、后续pipeline及实体PrintIntent可指定单面、长边或短边装订。缺省为单面，不因设备属性顺序而自动改为双面。MCP使用相同设置。

## Capability checks / 能力照合 / 能力检查

Duplex requires an exact match in `sides-supported` and
a recognized `pwg-raster-document-sheet-back` (`normal`, `rotated`, `flipped`, or `manual-tumble`). Missing side capability retains the legacy
single-sided path only. Wrong types and duplicate sheet-back attributes are rejected;
unknown sheet-back values never enable duplex. The common Printer
stage exposes only executable sides; `deliveryIntersection.deliverySides` describes
physical binding, while raster-export `sides` stays one-sided. Sheet-back is bound
into the capability revision. Settings are checked at preparation and again by the
Native IPP adapter immediately before Print-Job. A changed capability yields
`REJECTED / unsupported_sides_or_sheet_back` with no Print-Job, including changes between supported modes.

日本語: 両面には `sides-supported` の一致と対応する4種類の裏面方式のいずれかが必須です。能力省略時は片面だけを許可し、型不正・裏面属性重複は拒否します。能力モデルに実行可能なとじ方向だけを公開し、裏面方式もrevisionへ拘束します。準備時・送信直前に再検査し、変化していた場合は送信せず拒否します。

简体中文: 双面要求 `sides-supported` 精确匹配及支持的四种背面方式之一。缺少能力时仅允许兼容单面；拒绝错误类型及重复背面属性。能力模型仅公布可执行装订方式，并将背面方式绑定revision。准备及发送前重新检查，能力变化时拒绝且不发送。

## Persistence and approval / 永続化・承認 / 持久化及审批

`DeliveryTicket.sides` and pipeline requests persist the binding. One-sided internal
serialization omits the field to preserve legacy JSON/hash behavior; absent old
fields deserialize as one-sided. Physical contracts always state `sides`, and duplex
contracts also state the actual `sheetBack`. The internal `sheetBack` field defaults to normal for legacy records and is omitted for normal; other modes are explicit. Request replay with different binding
conflicts. Pipeline child matching and unified-ticket reads verify the same binding.
Physical approvals accept `sides`; a long-edge approval cannot authorize simplex
or short-edge. The DB admission transaction independently rechecks the approved
binding, including legacy omitted-side approvals (one-sided only).

日本語: ticket・pipeline・契約・承認へ設定を固定し、再起動後も保持します。内部の片面属性省略で旧JSON／hashを維持します。同じrequestIdでのとじ方向変更や、別方向の承認流用は拒否し、キュー登録トランザクションでも再検査します。

简体中文: 设置绑定ticket、pipeline、契约及审批，重启后保留。内部单面字段省略以保留旧JSON／hash。相同requestId更改装订或跨方向复用审批会被拒绝，队列入库事务也重新检查。

The [duplex encoder](NORMAL-DUPLEX-RASTER.md) generates matching Duplex/Tumble and
per-page transform headers and rearranges backside pixels into the device-native
coordinates. Transforms are applied in place, preserving channel order. The worker validates the entire stored PWG against the bound setting,
media, geometry and hash. Native IPP submits explicit job-group `sides`, `copies`
and fidelity. Selected pages form one document; binding applies to that document,
not the original page positions. Odd page counts are not padded by the server.
Existing UNKNOWN and no-automatic-resend rules still apply.

日本語: PWGの全ページのDuplex／Tumbleとticketを照合し、IPPのsides・copies・fidelityを明示します。ページ選択後の文書にとじ方向を適用し、奇数ページへの空白追加や複製はしません。UNKNOWN時の自動再送禁止は維持します。

简体中文: 检查全部PWG页的Duplex／Tumble与ticket一致，并显式提交IPP sides、copies及fidelity。装订作用于选页后的文档，不自动补空白或复制页面；保留UNKNOWN不自动重发规则。

🟢 [Done] Local IPP tests verify long/short edge, preview, exact wire settings,
restart, approval mismatch, changed sheet-back, physical preflight and page-selected
pipelines. Storage tests cover atomic approval and legacy records; MCP tests cover
all three tools, invalid enums and default single-sided forwarding.

🟢 [Done] Rotated/flipped/manual-tumble transforms, exact-mode ticket binding and live mode-change rejection. Independent CUPS tests verify all pixels and headers in 144 combinations (binding, mode, color, page count, odd/even/non-square geometry).

日本語: 全4方式の画素変換・永続ticketへの方式固定・送信直前の方式変更拒否を実装しました。独立CUPSで144組を検証します。

简体中文: 已实现四种模式的像素变换、持久ticket绑定及发送前模式变化拒绝；独立CUPS验证144种组合。

🟠 [Next] Console/virtual-IPP integration. Artifact-only intent resolution and virtual IPP remain single-sided.

⭕️ [Pending] Actual printer orientation, odd-sheet behavior, collation and cancellation
acceptance; mock acceptance is not physical paper verification.
