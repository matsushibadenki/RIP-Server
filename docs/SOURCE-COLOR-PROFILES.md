# Source ICC analysis / 原稿内部ICC解析 / 原稿内部ICC分析

## English

🟢 [Done] The standard worker passively analyzes original PDF/PostScript/EPS from
the same private SHA-256-checked source snapshot used for rendering. Parsing runs
inside the existing networkless, resource-limited MuPDF container, never in HTTP/MCP
frontends. Rebuild `jrip-mupdf:local` after this update:

```sh
docker build -f adapters/mupdf/Dockerfile -t jrip-mupdf:local .
```

The image pins pypdf 6.21.2 and its compatibility dependency; the analyzer rejects
another parser version. This release includes parser robustness/security fixes;
see the [official release](https://github.com/py-pdf/pypdf/releases/tag/6.21.2).
MuPDF still performs the separate strict PDF/page-content gate; this passive parser
does not replace it. PS/EPS rendering still uses the selected Ghostscript engine.
PS/EPS jobs also need the updated MuPDF image for passive profile analysis.

| Source | Evidence | Deliberate scope |
|---|---|---|
| PDF | `PDF_ICC_BASED` streams, `PDF_OUTPUT_INTENT` destination streams | Reachable catalog dictionary/array graph; resource and XObject dictionaries, cycles bounded; content streams are not executed |
| PS/EPS | `PS_DSC_ICC` | Static ICC DSC comments; Hex and ASCII85, physical Bytes/Lines counts and unknown count `-1`; counted binary/data blocks skipped |

The PS convention follows [ICC embedding §9.2](https://www.color.org/icc301.pdf).
A DSC declaration may be unused or appear inside other PostScript constructs;
it never proves that an interpreter used that profile. Dynamic CIEBased/CRD
construction, external profiles, screen previews and arbitrary executable
PostScript are not evaluated. PDF declarations similarly do not prove actual paint
operations, all inline-image uses, ICC use by the renderer or colorimetric correctness.
OutputIntent remains a distinct output-purpose declaration and is never chosen as
an input profile. Shared PDF streams are inspected once per role.

Each entry contains role, bounded ICC-envelope state, SHA-256, byte size and color
space when readable. ICCBased component count is checked against the header.
DeviceLink, malformed envelopes, invalid component counts, external stream files,
unsupported filters/DecodeParms, decompression excess and missing declarations in an
OutputIntent generate explicit warning codes. No profile bytes, source paths,
passwords or document labels are returned. No profile is registered or selected.
Messages are supplied in English, Japanese and Simplified Chinese.

Limits: source 32 MiB; graph 4096 visits/depth 32; 64 profile observations; each ICC
4 MiB; aggregate decoded ICC 16 MiB; output 64 KiB. Unfiltered and single FlateDecode
ICC streams are supported. ICC stream decoding uses a bounded decompressor rather
than unbounded pypdf `get_data()`. General PDF parsing remains contained by the
container's memory/CPU/time limits. Encryption, parser errors, incomplete graph
traversal and unsupported cases produce `INCOMPLETE`, not a claim of no profiles.
`DECLARATIONS_INSPECTED` means only completion within the stated passive scope;
an empty list does not prove the document has no color management.

The worker checks source hash/size again against the analysis, then appends it once
to source-validation evidence while its current RIPPING lease remains valid.
Historical reports are not rewritten. The nested evidence is:

```text
GET /api/v1/jobs/{id}/source-validation   (jobs:read)
  colorProfileAnalysis.analysis          profile org.jrip.source-color-profiles-v1
  colorProfileAnalysis.digestProfile     org.jrip.jcs-safe-v1
  colorProfileAnalysis.analysisDigest    sha256:<JCS hash of analysis>
```

The result Receipt includes this source-validation evidence under
`proof.result.sourceValidation`. An absent field means an older/in-flight report,
not absence of ICC. A digest is an integrity fingerprint, not independent provider
authentication; configured Receipt signatures retain their separate role.
Diagnostics do not alter the RIP/color policy. Failure to run the analysis process
or bind its source prevents successful publication; a reported `INCOMPLETE` warning
is diagnostic and does not itself change the existing rendering policy.

Validation commands:

```sh
python3 -m venv /tmp/jrip-source-profiles
/tmp/jrip-source-profiles/bin/pip install -r adapters/source_profiles/requirements.txt
/tmp/jrip-source-profiles/bin/python -m unittest discover -s adapters/source_profiles -p 'test_*.py' -v
cargo test -p jrip-server -p rip-storage -p rip-interpreter -p rip-color --test source_profiles --test source_validation --locked -- --ignored
```

Fixtures include synthetic ICC envelopes, compressed PDF ICCBased plus CMYK
OutputIntent, shared/cyclic references, PS ASCII85/Hex, count errors, opaque data,
encryption, external references and a decompression bomb. These test envelope
handling, not usable device-profile quality. Storage tests cover source/run binding,
expired leases, one-time append, restart and immutable terminal evidence. Docker
tests cover the analyzer and the normal worker, including propagation of a valid
LCMS-generated sRGB profile embedded in PostScript into the result Receipt.

⭕️ [Pending] Actual customer documents, complex unsupported stream filters and
real device color acceptance. 🔴 [Later] Executed color-space usage maps and complete
PS/CIE/CRD color semantics. This is not PDF/X or ICC conformance certification.

## 日本語

🟢 [Done] 原稿PDF／PS／EPSを、描画と同じhash確認済み私有snapshotから隔離コンテナー内で解析します。
PDFのICCBasedとOutputIntent、PS/EPSのICC DSC宣言を別々に記録します。PDFの参照循環、共有stream、
ICCの圧縮展開、component数、PSの物理byte／line数を有界に検査し、不正・外部参照・未対応形式は警告にします。
OutputIntentを入力profileとして採用せず、ICCの自動登録・自動選択・RIP設定変更も行いません。

PDFはcatalogから到達する辞書／配列、PSは静的DSC宣言が対象です。宣言が描画に使用されたとは断定せず、
動的CIEBased／CRD、外部profile、画面preview、実行された全色設定の解析とは区別します。
空の一覧もICC不在の保証ではありません。暗号化・上限・未対応の解析は `INCOMPLETE` として記録します。
上限は原稿32 MiB、4096訪問・深さ32、64件、ICC各4 MiB・合計16 MiB、出力64 KiBです。

上記MuPDFイメージの再構築が必要です。通常workerのみが解析し、HTTP／MCP側へDocker権限は追加しません。
原稿hash・byte数・run・有効なleaseを確認し、公開前のsource-validationへ一度だけ追記します。
既存の証跡APIと結果ReceiptからJCS digest付き `colorProfileAnalysis` を取得できます。
古い／処理中の記録でfieldがない場合を「ICCなし」と解釈しないでください。

Python fixture、保存処理、実Docker解析、PDF／PS／EPS通常workerとReceiptへの伝播を検証します。
模擬ICCは構造試験用で、実機色再現品質の証明ではありません。
⭕️ [Pending] 実顧客原稿・未対応filter・実機ICCの受入確認。
🔴 [Later] 実際に使用された色空間の対応表とPS／CIE／CRD全体の色意味解析。

## 简体中文

🟢 [Done] 在隔离容器中，从与渲染相同的hash验证私有snapshot分析原始PDF／PS／EPS。
分别记录PDF ICCBased、OutputIntent和PS/EPS ICC DSC声明；有界处理引用循环、共享stream、解压、
component数量及DSC物理byte／line计数。无效、外部引用及不支持的形式生成警告。
不把OutputIntent当作输入profile，不自动注册、选择ICC或更改RIP设置。

PDF范围为catalog可达的字典／数组图，PS范围为静态DSC声明；不证明声明被实际使用。
不执行动态CIEBased／CRD、外部profile或preview解析。空列表不保证原稿没有ICC。
加密、上限及不支持情况记录为 `INCOMPLETE`。限制为原稿32 MiB、4096次访问、深度32、64条、
ICC每条4 MiB／总计16 MiB、输出64 KiB。

须重建上述MuPDF镜像。仅可信worker执行，不给HTTP／MCP增加Docker权限。
检查源hash、byte数、run及有效lease，在发布前一次性追加到source-validation。
现有证据API及结果Receipt包含带JCS digest的 `colorProfileAnalysis`。
旧记录或处理中的字段缺失不表示没有ICC。

用Python fixture、存储测试及实际Docker分析、PDF／PS／EPS worker与Receipt传递验证。
模拟ICC仅验证结构处理，不证明设备色彩质量。
⭕️ [Pending] 实际客户原稿、不支持filter及设备ICC验收。
🔴 [Later] 实际使用的色空间映射及完整PS／CIE／CRD颜色语义。
