# RIP font catalog / RIPフォント一覧 / RIP字体列表

🟢 [Done] The right-aligned Fonts tab lists font registration names verified in the actual RIP images. It supports case-insensitive search by registration/resolved name, engine filtering, use-format labels and resolved font names. The Jobs tab remains the default.

`GET /api/v1/fonts` requires `jobs:read`, returns `org.jrip.font-catalog-v1`, and uses `Cache-Control: private, no-store`. The process shares a 30-second inventory snapshot and single-flight scan across listeners. Image IDs and observation/expiry times identify the snapshot; unavailable engines are explicitly marked rather than replaced by a fabricated list.

- MuPDF: the actual image's `font_policy.STANDARD14` is read and all 14 fonts are exercised in a generated, tiny PDF. Rows represent permitted standard-14 PDF substitutions, not a guarantee of original outlines.
- Ghostscript: `/Font resourceforall` candidates must resolve through `findfont` with the worker's NOFONTPATH/NOPLATFONTS/NONATIVEFONTMAP and nonexistent substitution target. Both the registration name and returned FontName are listed. Non-standard-14 rows are labelled PS/EPS only; These built-in entries do not authorize arbitrary PDF fallback. Separately registered programs can resolve compatible missing PDF resources, as described in FONT-INSTALLATION.md.
- Host fonts, arbitrary font files merely installed in an image, missing CID aliases, embedded document fonts and glyph coverage are not inferred from these rows. Embedded Type1/TrueType/CFF/CID and Type3 remain governed by the document font policy.

Inspection uses immutable image IDs obtained from the two configured local image tags, no pull, no network, read-only root, non-root image users, dropped capabilities, no-new-privileges, 128 MiB memory, 64 PIDs, half a CPU and 16 MiB temporary storage. Output is bounded to 128 KiB, names/rows are validated and execution has deadlines with container cleanup. Inspection survives an HTTP client's departure to finish cleanup. No source job, print approval or printer is touched.

## 日本語

右端の「フォント」タブで、RIPが実際に解決できる登録名・エンジン・使用形式・代替先を表示します。検索とエンジン別絞り込みに対応し、既定のタブは引き続きジョブです。

内蔵フォントのPDF使用は、原稿内の埋め込み書体または非埋め込みType1の標準14書体が基本です。追加登録したプログラムは、登録名・形式が適合する非埋め込みPDFリソースにも使用します。それ以外のGhostscript登録名はPS／EPS用です。MacのFont Bookや、コンテナ内にファイルが置かれているだけの書体を、利用可能と見なして表示することはありません。PDF内の埋め込み書体や文字収録範囲は原稿ごとの検査が必要です。

実際のイメージを隔離して読み取り、30秒間のスナップショットを共有します。エンジンを取得できなければ取得不可と表示します。フォント一覧の取得によって印刷・承認・原稿投入は発生しません。

## 简体中文

右侧“字体”选项卡显示实际RIP环境能够解析的注册名、引擎、可用格式及实际字体，并支持搜索与引擎筛选。默认仍为任务选项卡。

内置字体用于PDF时，以文档内嵌字体或名称完全匹配的非内嵌Type1标准14替代字体为基础。另行注册的程序也可解析名称及格式兼容的非内嵌PDF资源。其他Ghostscript注册名仅用于PS／EPS。不会将Mac字体或仅存在于容器文件系统中的字体自动视为可用。内嵌字体及字符覆盖须逐文档检查。

在隔离环境读取实际镜像并共享30秒快照，无法获取的引擎明确标记为不可用。获取列表不会提交原稿、审批或打印。

Registered static font programs can also be installed from this tab. See [FONT-INSTALLATION.md](FONT-INSTALLATION.md) for supported resource types, permissions and limits. / 登録フォントの追加と使用条件は同文書を参照。 / 字体安装及支持范围请参阅上述文档。
