# Registered RIP fonts / RIPフォントの追加 / 添加RIP字体

🟢 [Done] Select files or drag them onto the Fonts tab to normalize and install static TTF/OTF and all faces of TTC/OTC. Admin access is required; a refresh/restart is unnecessary. Uploads are limited to 16 MiB, 32 collection faces, 128 registered names and 128 MiB of active programs. Installation affects subsequent jobs. The library persists under `DATA_ROOT/font-store`.

フォントタブでファイルを選択、またはドロップすると、静的TTF／OTFとTTC／OTCの全フェイスを正規化・追加できます。管理者権限が必要です。再起動は不要で、以後のジョブから利用します。1ファイル16 MiB、コレクション32フェイス、登録128書体・128 MiBまで。登録内容は `DATA_ROOT/font-store` に保存されます。

在字体标签中选择或拖入静态TTF／OTF及TTC／OTC，即可规范化并注册所有字体。需要管理员权限，无需重启，供后续作业使用。限制：每文件16 MiB、集合32字体、注册128字体及128 MiB，保存在 `DATA_ROOT/font-store`。

## Resolution / 解決方法 / 字体解析

- Preserve PostScript registration names, glyphs, layout and hinting tables; recalculate SFNT checksums and discard invalidated DSIG signatures. Do not rebuild outlines or rename fonts.
- PDF: resolve exact non-embedded BaseFont names only. Supported resource/program combinations: TrueType/TTF, simple Type1/non-CID OpenType CFF, Identity-H/V CIDFontType2/TTF and CIDFontType0/CID OpenType CFF. Incompatible resources fail explicitly. Existing embedded programs are untouched. A bounded derived PDF is generated inside the sandbox; the original document remains unchanged.
- PS/EPS: Ghostscript uses an exact-name Fontmap alongside its built-ins. Unknown names still fail; no generic fallback is enabled.
- Each job pins an immutable manifest and verifies font program hashes. Same-name/same-content imports are idempotent; different content or built-in name collisions are rejected.

登録名・字形・組版・ヒンティングを保持します。PDFは登録名と形式が適合する非埋め込みリソースのみ解決します。既存の埋め込み書体と原稿は変更せず、サンドボックス内で派生PDFを作ります。PS／EPSはGhostscriptのFontmapで解決します。ジョブごとに不変の登録内容とハッシュを固定し、同名の異なる書体・内蔵書体の上書きを拒否します。

保留注册名、字形、排版及微调表。PDF仅解析名称完全匹配且格式兼容的非内嵌字体；原稿及已有内嵌字体不变，仅在沙箱中生成派生PDF。PS／EPS通过Ghostscript Fontmap解析。每个作业固定不可变清单并验证哈希，拒绝同名不同内容及覆盖内置字体。

## API and approvals / APIと承認 / API及审批

`POST /api/v1/fonts`, binary `application/octet-stream`, admin scope. Returns `org.jrip.font-installation-v1`, installed metadata and snapshot SHA-256. `GET /api/v1/fonts` includes registered programs for both engines. With persistent-settings approvals enabled, `font-install` requires a separate administrator's single-use approval bound to the exact file bytes and previous registry digest. The console accepts that approval ID; batch files require distinct approvals.

設定変更承認が有効な場合、ファイルの内容と変更前の登録状態に結びつく、別管理者の単回承認が必要です。画面で承認IDを入力できます。この場合はファイル単位で承認・追加してください。

启用持久设置审批时，需要另一管理员针对文件内容及原清单签发的一次审批。界面可输入审批ID，此模式下请逐个文件审批及添加。

## Reused code and limits / 移植元と制限 / 来源及限制

The MIT-licensed `/Users/Shared/Program/Tauri/Fonts` project's `font-core/src/color_fonts.rs` SFNT assembler and face-extraction design were copied/adapted into `crates/rip-font`. See its `LICENSE` and `ORIGIN.json`. The source project is unchanged. Adobe Fonts' discovery skill contains no conversion engine and is not used to download fonts.

MITライセンスの上記プロジェクトからSFNT組立処理とフェイス抽出設計を移植しました。変換エンジン全体の移植ではありません。元プロジェクトは変更していません。Adobe Fontsの検索スキルは変換・取得に使用していません。

从上述MIT项目移植SFNT组装及字体提取设计，并非完整转换引擎，源项目未更改。Adobe Fonts搜索技能不参与转换或下载。

🔴 [Later] Variable/color fonts, WOFF/WOFF2, legacy Type1/DFONT and other outline conversions. Use a static outline TTF/OTF export for now. Restricted embedding and bitmap-only flags are rejected. Font use must follow the font's license.

🔴 [Later] 可変・カラー書体、WOFF／WOFF2、旧Type1／DFONT等。現在は静的な通常アウトラインのTTF／OTFを書き出して使用してください。埋め込み禁止・ビットマップ限定フラグのある書体は拒否します。書体の利用条件に従ってください。

🔴 [Later] 可变／彩色字体、WOFF／WOFF2、旧Type1／DFONT等。目前请使用静态轮廓TTF／OTF导出。拒绝禁止嵌入或仅限位图的字体，请遵守字体许可。

## Verification / 検証 / 验证

Conversion tests preserve original tables/checksums, extract mixed-outline collections and reject unsupported/embedding-restricted programs. Docker tests load static TTF and CFF OTF through both full RIP adapters, verify original-byte preservation and inspection evidence, and exercise Ghostscript PS with built-ins. API tests cover admin authorization, approval binding, persistence and catalog entries after reopening. Browser tests exercise file selection/drop and EN/JA/简体中文 at 320–1440px.

⭕️ [Pending] Physical printer validation remains on hold until paper is available and each physical print has explicit approval. / 実機印刷は用紙の準備と印刷ごとの承認待ちです。 / 实机打印等待备纸及逐次审批。
