# 日本語 PostScript / PDF RIP Server 開発仕様書

## 1. プロジェクト概要

### 1.1 仮称

**J-RIP Server**

Japanese PostScript / PDF Raster Image Processor Server

### 1.2 目的

PostScript、EPS、PDFを入力として受け取り、日本語フォントを含む文書を高精度に解釈・ラスタライズし、一般的なネットワークプリンター、業務用プリンター、将来的にはデジタル印刷機・大判プリンター等へ出力可能なネットワークRIPサーバーを開発する。

単なるPostScript→画像変換ソフトではなく、

- 印刷ジョブ管理
- PostScript/PDF解釈
- 日本語CID/CMap処理
- フォント管理
- プリフライト
- CMYKカラーマネジメント
- オーバープリント
- スポットカラー
- 高解像度ラスタ処理
- ハーフトーン
- IPP
- IPPS
- PWG Raster
- CUPS連携
- プリンター個別Backend

を統合した印刷基盤とする。

---

# 2. 基本設計思想

システムを、

```text
Document Interpretation
Rendering
Color Processing
Raster Processing
Printer Transport
Job Management
```

の6層に分離する。

特に重要なのは、

```text
RIP Core ≠ Printer Driver
```

とすることである。

RIP Coreは特定メーカーのプリンターについて知らない。

RIP Coreの標準出力として、

```text
Canonical Raster Format
```

を定義し、各Printer BackendがCanonical Rasterから実機向けデータへ変換する。

これにより、

```text
                     ┌─────────────────┐
                     │    RIP Core     │
                     └────────┬────────┘
                              │
                    Canonical Raster
                              │
             ┌────────────────┼────────────────┐
             │                │                │
        PWG Raster           TIFF          Raw Raster
             │                │                │
       IPP Printer       File Export     Vendor Backend
```

とする。

---

# 3. 想定用途

主要用途は以下とする。

### 一般オフィス

Mac / Windows / Linux

↓

IPP / IPPS

↓

J-RIP Server

↓

ネットワークプリンター

### DTP

Adobe Illustrator / InDesign / Acrobat等

↓

PS / EPS / PDF

↓

J-RIP Server

↓

CMYK RIP

↓

業務プリンター

### 大判印刷

PDF / PS

↓

高解像度Tile RIP

↓

ICC / DeviceLink

↓

Halftone

↓

Printer Backend

↓

大判プリンター

---

# 4. 対応プラットフォーム

## 4.1 サーバー

第一ターゲット：

```text
Linux x86_64
Linux ARM64
```

第二ターゲット：

```text
macOS Apple Silicon
```

Windows Serverは将来対応とする。

本番環境のRIPサーバーはLinuxを基本とする。

プリンターとLAN内で通信する性質上、クラウド専用サービスではなく、

```text
Local / Edge RIP Server
```

を基本とする。

中央管理機能をクラウド化する場合は、

```text
Cloud Control Plane
        │
      HTTPS
        │
Local RIP Server
        │
       LAN
        │
     Printer
```

とする。

印刷データそのものをクラウド経由させることは必須としない。

---

# 5. 技術スタック

## 5.1 RIP Server

```text
Rust
Tokio
Axum
Serde
SQLx
Tracing
```

を基本とする。

Rustを選択する理由は、

- メモリ安全性
- 高並列処理
- 非同期I/O
- CライブラリとのFFI
- Linux/macOS対応
- 長時間稼働サーバーへの適性

による。

---

# 6. 外部エンジン

## 6.1 PostScript / PDF

実装方針追記（2026-09-18）：MuPDFとGhostscriptのハイブリッド構成とする。PDFはMuPDFを既定とし、PostScript / EPSはGhostscriptを使用する。PDFのGhostscript明示指定を許可し、処理失敗時の暗黙のエンジン切替は行わない。現在の実装範囲と制限はREADMEとROADMAPを参照。

第一候補：

```text
Ghostscript / GhostPDL
```

PostScript Level 3互換インタプリタを独自開発することは初期段階では行わない。

Ghostscriptは、

```text
PS
EPS
PDF
```

の解釈エンジンとして利用する。

実装方式は初期段階では、

```text
RIP Worker
    ↓
Ghostscript subprocess
```

とする。

将来的に必要に応じて、

```text
Rust
 ↓
FFI
 ↓
libgs
```

へ移行できるよう抽象化する。

Ghostscript自体はPostScript/PDFインタプリタとグラフィックスライブラリを提供しているため、この用途に適している。

---

# 7. Ghostscriptライセンス

非常に重要。

Ghostscriptには大きく、

```text
AGPL
Commercial License
```

の2系統が存在する。

J-RIP Serverを完全なオープンソースとしてAGPL互換条件で公開する場合はAGPL版を利用できる。

一方、

```text
有償ソフト
クローズドソース
プリンターOEM組み込み
商用RIP
```

として販売する場合には、Artifexとの商用ライセンス契約を前提に設計する。

したがってGhostscript依存部分は、

```text
src/adapters/interpreter/
```

へ完全分離する。

将来的に別のPDF/PSエンジンへ差し替えられる構造とする。

---

# 8. 日本語フォントエンジン

使用候補：

```text
FreeType
HarfBuzz
ICU
Adobe CMap Resources
```

HarfBuzzはUnicode列を実際のglyph IDと配置情報へ変換するshaping engineとして使用する。

OpenType、

```text
OTF
TTF
TTC
CFF
CFF2
Variable Font
```

を扱う。

ただし、PostScript/PDFにすでに配置済みglyph情報が存在する場合に、無条件でHarfBuzzによる再組版を行ってはいけない。

原則：

```text
Embedded Glyph
     ↓
Document指定を尊重

Unicode Text
     ↓
Font Mapping
     ↓
HarfBuzz
```

とする。

---

# 9. 日本語CID対応

重点対応：

```text
Adobe-Japan1
Identity-H
Identity-V
90ms-RKSJ-H
90ms-RKSJ-V
UniJIS-UTF16-H
UniJIS-UTF16-V
UniJIS2004-UTF16-H
UniJIS2004-UTF16-V
```

など。

CMap処理を専用ライブラリとして分離する。

```text
Japanese Text
      │
Encoding Detector
      │
CMap Resolver
      │
CID Resolver
      │
Glyph Resolver
      │
Font Face
```

とする。

---

# 10. 縦書き

日本語RIPとして縦書きを第一級機能として扱う。

対応項目：

```text
Vertical CMap
vert
vrt2
縦中横
句読点
括弧
長音記号
約物
縦書き用glyph
```

PDF/PS内に既に縦書き配置が指定されている場合、その位置情報を優先する。

---

# 11. フォント解決

フォント検索順序：

```text
1. PDF/PS Embedded Font
2. Job supplied font
3. RIP registered font
4. User font directory
5. System font
6. Configured substitution
7. Emergency fallback
```

フォント置換は黙って実行しない。

Job Reportへ、

```text
Original:
Ryumin-Light

Substitution:
Noto Serif CJK JP

Reason:
Font unavailable
```

のように記録する。

---

# 12. Font Database

内部DB：

```text
fonts

id
family_name
postscript_name
full_name
style
weight
path
format
sha256
collection_index
glyph_count
unicode_ranges
adobe_collection
created_at
updated_at
```

を持つ。

フォントファイルを登録時に解析し、

```text
PostScript Name
Family
Style
Unicode cmap
CID collection
OpenType features
Font embedding permission
```

等を保存する。

---

# 13. RIPパイプライン

基本処理：

```text
INPUT

 ↓

Job Validation

 ↓

Format Detection

 ↓

Preflight

 ↓

PS/PDF Interpretation

 ↓

Display List

 ↓

Tile Rasterization

 ↓

Color Conversion

 ↓

Black Generation

 ↓

Ink Limiting

 ↓

Halftoning

 ↓

Raster Packing

 ↓

Printer Backend

 ↓

OUTPUT
```

---

# 14. 内部Display List

Ghostscriptから直接プリンターへ送らず、将来的には中間表現を保持できる構造を用意する。

概念的には、

```rust
enum DrawCommand {
    FillPath,
    StrokePath,
    DrawGlyph,
    DrawImage,
    SetClip,
    SetBlendMode,
    BeginTransparencyGroup,
    EndTransparencyGroup,
}
```

のような表示命令列である。

MVPではGhostscript内部レンダリングを使用して構わない。

Phase 2以降で独自Display Listを導入する。

---

# 15. Canonical Raster

RIP Core内部の共通ラスター形式を定義する。

基本：

```text
RGB8
RGB16
Gray8
Gray16

CMYK8
CMYK16

DeviceN8
DeviceN16
```

印刷用途の内部標準は、

```text
CMYK16
```

を推奨する。

最終出力時に8bit等へ量子化する。

---

# 16. タイルRIP

巨大ページを一括展開しない。

標準Tile：

```text
1024 × 1024 px
```

を基本値とする。

変更可能：

```text
256
512
1024
2048
4096
```

ページ：

```text
Page
 ├─ Tile 0
 ├─ Tile 1
 ├─ Tile 2
 ├─ Tile 3
 └─ ...
```

各Tileについて、

```text
Rasterize
 ↓
Color Transform
 ↓
Halftone
 ↓
Compression
 ↓
Spool
 ↓
Release
```

する。

---

# 17. メモリ制御

RIPプロセス全体に、

```text
memory_budget
```

を設定する。

例：

```text
memory_budget = 4 GB
tile_cache = 1 GB
font_cache = 512 MB
ICC_cache = 256 MB
job_buffer = 512 MB
reserve = remaining
```

とする。

RAM量に応じ自動計算可能とする。

OOM発生によりOSからkillされる設計は禁止。

---

# 18. 並列処理

ページ間並列：

```text
Page 1 → Worker A
Page 2 → Worker B
Page 3 → Worker C
```

タイル間並列：

```text
Tile A ─┐
Tile B ─┼→ Ordered Output
Tile C ─┤
Tile D ─┘
```

の双方をサポートする。

ただしプリンターへの出力順序は保証する。

---

# 19. Job Scheduler

状態：

```text
RECEIVED
VALIDATING
PREFLIGHT
QUEUED
RIPPING
COLOR_PROCESSING
HALFTONING
SPOOLING
PRINTING
COMPLETED

HELD
CANCELLED
FAILED
```

とする。

---

# 20. Job Priority

```text
Low
Normal
High
Urgent
```

を持つ。

さらに、

```text
priority integer 0–100
```

を内部値として持たせてもよい。

---

# 21. Job Manifest

例：

```json
{
  "job_id": "01J-RIP-000001",
  "name": "catalog-2026.pdf",
  "format": "application/pdf",
  "copies": 2,
  "media": "iso_a4_210x297mm",
  "dpi": 1200,
  "color_mode": "cmyk",
  "duplex": "two-sided-long-edge",
  "printer_id": "printer-01",
  "render_intent": "relative-colorimetric",
  "black_point_compensation": true,
  "preflight": true
}
```

---

# 22. REST API

ベース：

```text
/api/v1
```

### Jobs

```text
POST   /api/v1/jobs
GET    /api/v1/jobs
GET    /api/v1/jobs/:id
DELETE /api/v1/jobs/:id

POST /api/v1/jobs/:id/cancel
POST /api/v1/jobs/:id/hold
POST /api/v1/jobs/:id/release
POST /api/v1/jobs/:id/reprint
```

### Printers

```text
GET    /api/v1/printers
POST   /api/v1/printers
GET    /api/v1/printers/:id
PATCH  /api/v1/printers/:id
DELETE /api/v1/printers/:id
```

### Fonts

```text
GET    /api/v1/fonts
POST   /api/v1/fonts
DELETE /api/v1/fonts/:id
```

### Profiles

```text
GET    /api/v1/color-profiles
POST   /api/v1/color-profiles
DELETE /api/v1/color-profiles/:id
```

---

# 23. IPP Server

REST APIとは別にIPP Printerとして振る舞う。

例：

```text
ipps://rip.local/ipp/print/office
ipps://rip.local/ipp/print/proof
ipps://rip.local/ipp/print/production
```

仮想プリンターを複数公開できるようにする。

IPP Everywhereを基本プロトコルとする。

---

# 24. IPP対応Operation

初期対応：

```text
Get-Printer-Attributes
Validate-Job
Create-Job
Send-Document
Print-Job
Get-Job-Attributes
Get-Jobs
Cancel-Job
```

将来：

```text
Hold-Job
Release-Job
Pause-Printer
Resume-Printer
```

等。

---

# 25. IPPS

ネットワーク印刷は原則として、

```text
ipps://
```

を推奨する。

TLS証明書を利用する。

自己署名証明書と正式証明書の両方に対応。

---

# 26. Printer Discovery

LAN内のIPP Printerを、

```text
DNS-SD
mDNS
IPP
```

で探索する。

管理画面から、

```text
プリンターを検索
```

すると、

```text
Canon ...
Epson ...
Ricoh ...
HP ...
```

などを検出できるようにする。

---

# 27. Printer Capability Detection

IPPから、

```text
media-supported
printer-resolution-supported
print-color-mode-supported
sides-supported
document-format-supported
urf-supported
pwg-raster-document-type-supported
```

等を取得する。

プリンター固有値をJobへ直接混入させず、

```text
PrinterCapability
```

内部構造体に正規化する。

---

# 28. Printer Backend Interface

概念：

```rust
trait PrinterBackend {
    fn discover();
    fn capabilities();
    fn validate_job();
    fn begin_job();
    fn send_page();
    fn end_job();
    fn cancel_job();
    fn status();
}
```

とする。

---

# 29. Backend

初期実装：

```text
PWG Raster Backend
IPP Backend
TIFF Backend
PNG Backend
Raw Raster Backend
```

次段階：

```text
PCL Backend
PCL XL Backend
PostScript Passthrough
Vendor Backend
```

---

# 30. PWG Raster

最重要標準出力の一つ。

CUPS/OpenPrintingがPWG Raster APIを提供しているため、それを活用する。

設定：

```text
resolution
color space
bit depth
media
duplex
sheet-back transform
```

等をPrinter Capabilityから生成する。

---

# 31. CUPS

J-RIP Server自体をCUPS依存にはしない。

しかし、

```text
CUPS Adapter
```

を実装する。

構造：

```text
RIP Core
   │
   ├── Native IPP
   │
   └── CUPS Adapter
```

とする。

macOS/Linuxの既存印刷システムから容易に利用できるようにする。

---

# 32. Color Management

色管理エンジン：

```text
LittleCMS 2
```

を第一候補とする。

LittleCMSはICCベースの色変換、DeviceLink等を扱えるため適している。

---

# 33. ICC Pipeline

標準：

```text
Document Color
       ↓
Source ICC
       ↓
PCS
       ↓
Output ICC
       ↓
Printer CMYK
```

必要に応じて、

```text
DeviceLink
```

を利用する。

---

# 34. Rendering Intent

対応：

```text
Perceptual
Relative Colorimetric
Absolute Colorimetric
Saturation
```

Black Point Compensation：

```text
ON/OFF
```

を指定可能とする。

---

# 35. RGB処理

RGB文書について、

```text
RGB
 ↓
Source RGB Profile
 ↓
PCS
 ↓
Printer ICC
 ↓
CMYK
```

とする。

Profile未指定時は設定されたDefault RGB Profileを使う。

---

# 36. CMYK処理

CMYK文書について、

```text
CMYK
 ↓
Source CMYK Profile
 ↓
Printer CMYK Profile
```

を基本とする。

ただし印刷物では黒版保持が重要なため、

```text
Preserve Black
Preserve CMYK Primaries
```

機能を別途実装する。

---

# 37. Total Area Coverage

TAC：

```text
Total Ink Coverage
```

制限を実装する。

例：

```text
300%
320%
340%
```

プリンタープロファイル単位で設定する。

---

# 38. Black Generation

設定：

```text
GCR
UCR
Black Start
Black Max
Black Strength
```

をPrinter Profileへ保存可能にする。

---

# 39. Spot Color

将来的に、

```text
DeviceN
Separation
Spot Color
```

へ対応。

例：

```text
PANTONE
DIC
TOYO
Custom Ink
White
Clear
Metallic
```

Canonical RasterはCMYKだけでなくDeviceNを許容する。

---

# 40. Overprint

印刷用途では必須。

対応：

```text
Overprint Fill
Overprint Stroke
Overprint Mode 0
Overprint Mode 1
```

PDF/PS側指定を尊重する。

---

# 41. Transparency

PDF 1.4以降の、

```text
Transparency Group
Blend Mode
Soft Mask
Alpha
Knockout Group
Isolation Group
```

を正しく処理する。

MVPではGhostscript側処理を利用する。

---

# 42. Halftone Engine

Halftoningを独立モジュールにする。

```text
src/raster/halftone/
```

初期：

```text
Ordered Dither
Error Diffusion
```

商用印刷向け：

```text
AM Screening
FM Screening
Hybrid Screening
```

を後から追加する。

---

# 43. AM Screening

設定例：

```text
C 15°
M 75°
Y 0°
K 45°
```

またはプリンター特性に応じたscreen angleを設定する。

LPI：

```text
85
100
133
150
175
200
```

等。

---

# 44. FM Screening

Stochastic Screeningを実装可能な構造にする。

将来的に、

```text
Blue Noise Mask
```

ベースのFMスクリーンを採用可能。

---

# 45. Preflight Engine

J-RIP Serverの重要な付加価値とする。

ジョブ投入時に印刷上の問題を解析する。

---

# 46. Preflight項目

最低限：

```text
Missing Font
Font Substitution
Non Embedded Font

RGB Object
CMYK Object
Spot Color

Low Resolution Image

Transparency

Overprint

Page Size

Bleed

TrimBox

CropBox

MediaBox

Broken ICC Profile

Unsupported PDF Feature
```

---

# 47. 日本語固有Preflight

特徴的機能として、

```text
CID mapping failure
CMap mismatch
Vertical CMap mismatch
Missing Japanese glyph
IVS unsupported
Font collection mismatch
JIS90/JIS2004 mismatch
Substitution differences
```

等を検査する。

---

# 48. Missing Glyph Report

例：

```text
Font:
A-OTF-RyuminPro-Regular

Glyph:
U+9AD9

CID:
8705

Status:
Missing

Fallback:
Noto Serif CJK JP

Page:
12
```

といったレポートを生成する。

---

# 49. Preview

RIP結果をWeb UIから確認可能にする。

低解像度：

```text
144 dpi
```

程度のpreview rasterを別途生成する。

表示：

```text
Composite
C
M
Y
K
Spot
```

を切り替えられるようにする。

---

# 50. Separation Preview

製版用途向けに、

```text
Cyan
Magenta
Yellow
Black
Spot 1
Spot 2
```

を個別表示できるようにする。

Overprint確認にも利用する。

---

# 51. Web管理画面

管理UI：

```text
React
```

を推奨。

サーバーと完全分離する。

管理画面：

```text
Dashboard
Jobs
Printers
RIP Queues
Fonts
Color Profiles
Preflight
Settings
Logs
Users
```

---

# 52. Dashboard

表示：

```text
Active jobs
Queue
RIP speed
CPU
RAM
Disk
Printer state
Error state
Jobs today
Pages today
```

---

# 53. Job画面

ジョブごとに、

```text
Preview
Pages
Dimensions
DPI
Colors
Fonts
ICC
Printer
Status
Progress
Preflight warnings
RIP duration
Print duration
```

を表示する。

---

# 54. RIP進捗

例：

```text
Interpreting       100%
Rasterizing         73%
Color Processing    69%
Halftoning           62%
Spooling             54%
```

と段階別に表示する。

単純な0～100%表示だけにしない。

---

# 55. データベース

基本：

```text
PostgreSQL
```

とする。

ただし単体RIPアプライアンス用途ではSQLiteも選択可能なRepository abstractionを設ける。

---

# 56. DB主要テーブル

```text
jobs
job_pages
job_files
job_events

printers
printer_capabilities
printer_profiles

fonts
font_aliases

color_profiles
device_links

users
roles

settings

audit_logs
```

---

# 57. ファイルストレージ

巨大PDF/ラスターをDBへ格納しない。

```text
/var/lib/jrip/
```

を標準data rootとする。

例：

```text
/var/lib/jrip/jobs/
/var/lib/jrip/spool/
/var/lib/jrip/cache/
/var/lib/jrip/fonts/
/var/lib/jrip/icc/
/var/lib/jrip/previews/
/var/lib/jrip/logs/
```

---

# 58. Job Storage

例：

```text
/var/lib/jrip/jobs/
└── 01JXYZ...
    ├── source/
    │   └── document.pdf
    ├── manifest.json
    ├── preflight.json
    ├── preview/
    ├── raster/
    └── logs/
```

---

# 59. プロジェクトディレクトリ構成

```text
/jrip
├── Cargo.toml
├── Cargo.lock
│
├── crates/
│   ├── rip-core/
│   ├── rip-interpreter/
│   ├── rip-font/
│   ├── rip-cmap/
│   ├── rip-color/
│   ├── rip-raster/
│   ├── rip-halftone/
│   ├── rip-preflight/
│   ├── rip-printer/
│   ├── rip-ipp/
│   ├── rip-storage/
│   ├── rip-jobs/
│   └── rip-common/
│
├── services/
│   ├── jrip-server/
│   ├── jrip-worker/
│   └── jrip-cli/
│
├── adapters/
│   ├── ghostscript/
│   ├── cups/
│   ├── pwg/
│   └── printer/
│
├── web/
│   └── admin/
│
├── resources/
│   ├── cmap/
│   ├── profiles/
│   └── presets/
│
├── tests/
│   ├── corpus/
│   ├── regression/
│   ├── fonts/
│   ├── pdf/
│   ├── ps/
│   └── printers/
│
└── docs/
```

---

# 60. Rust Workspace

各機能をcrate分割する。

特に、

```text
rip-core
```

はGhostscriptやCUPSへ直接依存してはならない。

依存関係：

```text
                 rip-common
                     │
         ┌───────────┼───────────┐
         │           │           │
     rip-font    rip-color   rip-raster
         │           │           │
         └───────────┼───────────┘
                     │
                  rip-core
                     │
              rip-interpreter
                     │
               Ghostscript
```

とする。

---

# 61. Interpreter abstraction

```text
DocumentInterpreter
```

インターフェースを定義。

```text
GhostscriptInterpreter
PDFInterpreter
FutureInterpreter
```

へ交換できるようにする。

PostScript/PDFの実装詳細をCoreへ漏らさない。

---

# 62. Workerプロセス分離

GhostscriptはRIP Server本体と同一プロセスで実行しない構成を第一案とする。

```text
jrip-server
     │
     │ IPC
     ↓
jrip-worker
     │
     ↓
Ghostscript
```

とする。

理由：

```text
クラッシュ隔離
メモリ制限
CPU制限
セキュリティ
Job cancellation
timeout
```

である。

---

# 63. Sandbox

PSはプログラミング言語であるため、外部から送信されたPostScriptは信頼しない。

Workerは、

```text
Dedicated user
Read-only root
No shell
No network
Limited filesystem
CPU quota
Memory quota
Process quota
Timeout
```

環境で実行する。

Linuxでは、

```text
namespaces
seccomp
cgroups
```

利用を検討する。

---

# 64. Job Limits

設定可能：

```text
max_document_size
max_pages
max_rip_time
max_memory
max_temp_storage
max_image_dimension
max_nested_objects
```

等。

異常PDF/PSによるDoSを防ぐ。

---

# 65. 認証

Web/API：

```text
Admin
Operator
Viewer
```

ロール。

認証方式：

```text
Local Account
OIDC
LDAP
```

を将来的に追加可能。

---

# 66. Audit Log

記録：

```text
Job upload
Job print
Job cancel
Printer add
Printer change
Font install
ICC install
User login
Configuration change
```

---

# 67. ログ

Rust：

```text
tracing
```

を利用。

JSON structured log。

例：

```json
{
  "job_id": "...",
  "page": 12,
  "tile": 27,
  "stage": "color_transform",
  "duration_ms": 13
}
```

---

# 68. Observability

Prometheus形式：

```text
jrip_jobs_total
jrip_jobs_failed
jrip_pages_total
jrip_rip_seconds
jrip_tiles_total
jrip_memory_bytes
jrip_spool_bytes
jrip_printer_errors
```

等を公開可能とする。

---

# 69. RIP性能計測

重要指標：

```text
Pages/min
Pixels/sec
Tiles/sec
Color transform pixels/sec
Peak RAM
Spool throughput
First-page latency
```

を測定する。

---

# 70. キャッシュ

以下をキャッシュする。

```text
Font Face
Parsed CMap
ICC Transform
DeviceLink
Reusable image
Halftone mask
```

キャッシュキーにはファイル名ではなく、

```text
SHA-256
```

を基本利用する。

---

# 71. Apple Silicon

macOS版では、

```text
ARM64 native
```

とする。

初期版はCPU最適化を優先する。

SIMD：

```text
NEON
```

を活用。

将来的に、

```text
Metal
Accelerate
vImage
```

による、

```text
Color conversion
Image scaling
Halftone
Raster filtering
```

等の高速化を検討する。

PostScript解釈そのものをGPU化することは優先しない。

---

# 72. Linux最適化

x86_64：

```text
AVX2
AVX-512
```

ARM64：

```text
NEON
SVE
```

をfeature detectionして利用可能な設計とする。

Scalar implementationも必ず維持する。

---

# 73. DPI

初期対応：

```text
300
600
1200
2400
```

任意DPIも指定可能とする。

内部では、

```text
xdpi
ydpi
```

を別々に保持する。

---

# 74. 大判対応

最大ページサイズをA系列に限定しない。

Roll Mediaを考慮し、

```text
width
height
```

を64bit安全な値で扱う。

ラスタバッファサイズ計算にはoverflow検査を必須とする。

---

# 75. Image Resampling

画像拡大縮小：

```text
Nearest
Bilinear
Bicubic
Lanczos
```

等。

印刷品質モードでは高品質フィルタを使用。

線画については画像と異なるresamplingルールを設定可能にする。

---

# 76. Vector Rendering

ベジェ曲線等を高解像度で直接Rasterizeする。

低解像度bitmapへ一度変換してから拡大する設計は禁止。

---

# 77. Thin Line Preservation

高解像度印刷では、

```text
Hairline
Thin Line
```

が消えないよう補正オプションを設ける。

---

# 78. Black Text Preservation

黒文字：

```text
RGB 0,0,0
CMYK 0,0,0,100
```

の扱いを設定可能とする。

設定：

```text
Pure K Black
Rich Black
Source Preserve
```

---

# 79. Trapping

MVP対象外。

将来：

```text
Spread
Choke
Centerline
Sliding trap
```

などの自動Trappingを追加可能な処理ステージを確保する。

---

# 80. PDF/X

段階的に、

```text
PDF/X-1a
PDF/X-3
PDF/X-4
```

のプリフライトを追加する。

PDF/X準拠そのものを独自PDFレンダラーで再実装するのではなく、まずValidation層として扱う。

---

# 81. PostScript

対象：

```text
PostScript Level 1
PostScript Level 2
PostScript Level 3
EPS
```

ただし実際の互換性はGhostscriptエンジンに依存する。

---

# 82. DSC

PostScriptについて、

```text
%%BoundingBox
%%Pages
%%DocumentFonts
%%DocumentNeededResources
```

等のDSCコメントを解析し、Preflight/Job情報へ利用する。

---

# 83. PDF parser

RIPとは別に軽量PDF metadata parserを用意してもよい。

用途：

```text
Page count
MediaBox
CropBox
TrimBox
BleedBox
PDF version
Encryption
Fonts
ICC
OutputIntent
```

をGhostscript起動前に高速取得する。

---

# 84. 暗号化PDF

Password protected PDFは、

```text
HELD
```

状態とし、管理画面からpassword入力可能とする。

passwordを平文DB保存しない。

---

# 85. ジョブ再印刷

原稿から再RIP：

```text
Re-RIP
```

と、

既存Spoolから再印刷：

```text
Re-Spool
```

を分離する。

設定変更時はRe-RIPが必要。

---

# 86. 再現性

同じ、

```text
Source
RIP Version
Font Set
ICC Profile
Preset
```

なら可能な限り同じRasterを生成する。

Job Manifestへ、

```text
engine_version
ghostscript_version
font_hashes
icc_hashes
preset_version
```

を記録する。

---

# 87. RIP Preset

例えば、

```text
Office
Proof
Production
Photo
Line Art
Newspaper
Large Format
```

というPresetを用意する。

Presetに、

```text
DPI
ICC
Rendering intent
Black preservation
TAC
Halftone
Compression
```

を保存する。

---

# 88. Error分類

```text
INPUT_ERROR
PDF_ERROR
POSTSCRIPT_ERROR

FONT_ERROR
CMAP_ERROR

COLOR_ERROR
ICC_ERROR

RASTER_ERROR

PRINTER_ERROR
NETWORK_ERROR

RESOURCE_LIMIT

INTERNAL_ERROR
```

とする。

---

# 89. 自動リトライ

プリンター通信失敗は、

```text
retry
```

可能。

しかし、

```text
PostScript syntax error
Missing mandatory font
Broken PDF
```

等を無限retryしてはならない。

Error categoryごとにretry policyを持つ。

---

# 90. Crash Recovery

Job状態をDBへ永続化し、

サーバー再起動後、

```text
RIPPING
COLOR_PROCESSING
HALFTONING
```

状態だったジョブを検出する。

安全なcheckpointが存在しなければ再RIP。

Spool完了済みの場合は再Spool可能とする。

---

# 91. Atomic Spool

未完成ラスターをプリンターへ送らないモードを用意する。

```text
RIP All
 ↓
Validate
 ↓
Print
```

Production用途ではこれを選択可能。

低レイテンシ用途では、

```text
RIP page
 ↓
Print page
```

ストリーミングにも対応する。

---

# 92. テスト戦略

最重要部分。

Golden Master方式を採用する。

基準PDF/PSに対し、

```text
expected raster hash
expected pixel result
```

を保持する。

単純SHA比較だけでなく、

```text
pixel difference
Delta E
edge difference
```

も利用する。

---

# 93. 日本語テストコーパス

必須：

```text
ひらがな
カタカナ
漢字
半角
全角
JIS90
JIS2004
異体字
IVS
縦書き
ルビ
縦中横
欧文混植
旧字体
CID直接指定
```

---

# 94. Fontテスト

代表：

```text
Noto Sans CJK JP
Noto Serif CJK JP
Source Han Sans
Source Han Serif
```

に加えて、ライセンス上テスト可能な商用日本語フォントでも検証する。

---

# 95. Colorテスト

ICC test chartを利用。

測定：

```text
RGB → CMYK
CMYK → CMYK
Gray preservation
Black preservation
Spot
Overprint
Transparency
```

---

# 96. Differential Test

同一データを、

```text
Ghostscript CLI
J-RIP Server
```

でRIPして比較する。

J-RIP側のAdapterによる誤差を検出する。

---

# 97. Fuzzing

対象：

```text
PDF parser
PS metadata parser
CMap parser
ICC parser
Raster parser
IPP parser
```

Rustのfuzz testingを導入する。

外部ファイルを読む箇所は特に重点的に行う。

---

# 98. CI

GitHub Actions等で、

```text
cargo fmt
cargo clippy
cargo test
integration tests
security audit
license scan
```

を実行する。

大型RIP regression testsはself-hosted runnerへ分離可能。

---

# 99. SBOM

商用配布を考慮して、

```text
CycloneDX
SPDX
```

等でSBOMを生成する。

Ghostscriptを含め、ライセンス監査をCIへ組み込む。

---

# 100. 配布

Linux：

```text
deb
rpm
container
```

macOS：

```text
pkg
dmg
```

将来的にはRIP Appliance用イメージも用意する。

---

# 101. Container

Server/APIはContainer化可能。

しかし物理プリンターとの、

```text
USB
mDNS
LAN discovery
```

等が必要なため、

```text
Container-only
```

へ依存しない。

LAN RIP Applianceではnative serviceも正式対応する。

---

# 102. systemd

Linuxでは、

```text
jrip-server.service
jrip-worker.service
```

として管理。

Workerは複数起動可能とする。

---

# 103. Scaling

単体：

```text
RIP Server
 ├ Worker 1
 ├ Worker 2
 ├ Worker 3
 └ Worker 4
```

複数ノード：

```text
              Scheduler
                  │
       ┌──────────┼──────────┐
       ↓          ↓          ↓
    RIP-01     RIP-02     RIP-03
```

と拡張できるようJob Queue abstractionを持つ。

---

# 104. クラウド連携

中央管理が必要になった場合：

```text
XServer VPS Cloud等
App/API
Managed PostgreSQL
Object Storage / NFS

       │
     HTTPS
       │

On-Premise RIP Node
       │
     Printer
```

というControl Plane / Data Plane分離を推奨する。

印刷の中核処理はプリンター近傍で完結させる。

WorkerからManaged PostgreSQLへ直接アクセスさせる構造には依存せず、

```text
HTTPS API
Job Queue
```

を介して疎結合にする。

---

# 105. MVP

最初に完成させる範囲を明確化する。

## MVP 1

```text
PDF
PostScript

↓ Ghostscript

RGB/CMYK Raster

↓ LittleCMS

CMYK

↓

PWG Raster / TIFF

↓

IPP Printer
```

日本語：

```text
Embedded Japanese Font
Adobe-Japan1
基本CMap
横書き
縦書き
Font substitution
```

に対応。

---

# 106. MVP 2

追加：

```text
IPP Server
Job Queue
Web UI
Printer Discovery
Font Manager
ICC Manager
Preflight
Preview
```

ここで実用RIP Serverとして成立させる。

---

# 107. Production 1

追加：

```text
CMYK16
Tile RIP
DeviceLink
Black preservation
TAC
Overprint
Separation preview
Advanced Japanese preflight
```

---

# 108. Production 2

追加：

```text
AM Screening
FM Screening
DeviceN
Spot Color
Trapping
Vendor Backend
Cluster RIP
```

商用印刷向けへ進む。

---

# 109. 実装優先順位

最優先：

```text
Job Core
Interpreter Adapter
Ghostscript
Tile Raster
PWG Raster
IPP
Japanese Font
CMap
ICC
```

次：

```text
Web UI
Preflight
Preview
Separation
Printer discovery
```

その後：

```text
Halftone
Spot
DeviceN
Trapping
Vendor Backend
GPU
```

とする。

HalftoneやGPU最適化を先に作らない。

---

# 110. 品質目標

一般用途：

```text
600dpi A4
数秒以内でFirst Page
```

を目標。

Production：

```text
1200dpi
CMYK
複数ページ
```

を実用的速度で連続RIP。

最大解像度：

```text
2400dpi+
```

を設計上許容する。

ただし性能値はハードウェアと文書複雑度に大きく依存するため、固定保証値にはしない。

---

# 111. メモリ目標

A0/A1等でもページ全体RasterをRAMに置かない。

通常ジョブでは、

```text
RAM使用量 ≒ Tile cache + interpreter + font/color cache
```

となるよう設計。

ページピクセル数に比例してRAMが増加する構造を避ける。

---

# 112. セキュリティ原則

外部PS/PDFは、

```text
Untrusted Code / Data
```

として扱う。

特にPostScriptは実行言語である。

したがって、

```text
Interpreter isolation
Filesystem restriction
Network restriction
Resource limit
Timeout
Input validation
```

をMVP時点から実装する。

---

# 113. 非目標

初期版では以下を自作しない。

```text
PostScript language interpreter
Complete PDF interpreter
OpenType rasterizer
ICC engine
CUPS replacement
```

これら成熟したOSS/商用エンジンを利用する。

独自開発する価値が高い部分は、

```text
RIP orchestration
Japanese font/CMap handling
Preflight
Canonical Raster
Tile processing
Color policy
Printer abstraction
Job management
Production workflow
```

とする。

---

# 114. 将来の独自RIP Core

J-RIPが成熟した場合のみ、

```text
Ghostscript
```

への依存を部分的に減らすことを検討する。

特にPDFについては、

```text
PDF Parser
      ↓
Display List
      ↓
Own Raster Engine
```

へ進化できる。

一方PostScriptは言語仕様と互換性問題が非常に大きいため、完全独自実装の優先順位は低くする。

---

# 115. 最終アーキテクチャ

```text
                 ┌───────────────────────────┐
                 │       Client Devices      │
                 │ macOS / Windows / Linux   │
                 └─────────────┬─────────────┘
                               │
                        IPP / IPPS / REST
                               │
                 ┌─────────────▼─────────────┐
                 │       J-RIP Server        │
                 │                           │
                 │ Auth / API / Scheduler    │
                 └─────────────┬─────────────┘
                               │
                        Job Message Bus
                               │
                 ┌─────────────▼─────────────┐
                 │        RIP Worker         │
                 └─────────────┬─────────────┘
                               │
                 ┌─────────────▼─────────────┐
                 │     Document Preflight    │
                 └─────────────┬─────────────┘
                               │
             ┌─────────────────▼────────────────┐
             │      Interpreter Adapter         │
             │      Ghostscript / Future        │
             └─────────────────┬────────────────┘
                               │
             ┌─────────────────▼────────────────┐
             │ Japanese Font / CID / CMap       │
             │ FreeType / HarfBuzz / Resources │
             └─────────────────┬────────────────┘
                               │
                 ┌─────────────▼─────────────┐
                 │       Tile Rasterizer     │
                 └─────────────┬─────────────┘
                               │
                 ┌─────────────▼─────────────┐
                 │     Color Management      │
                 │       LittleCMS 2         │
                 └─────────────┬─────────────┘
                               │
                 ┌─────────────▼─────────────┐
                 │   Black / TAC / DeviceN   │
                 └─────────────┬─────────────┘
                               │
                 ┌─────────────▼─────────────┐
                 │       Halftone Engine     │
                 └─────────────┬─────────────┘
                               │
                 ┌─────────────▼─────────────┐
                 │     Canonical Raster      │
                 └─────────────┬─────────────┘
                               │
             ┌─────────────────┼──────────────────┐
             │                 │                  │
             ▼                 ▼                  ▼
       PWG Raster          TIFF/Proof       Vendor Raster
             │                                    │
             ▼                                    ▼
       IPP Everywhere                       Printer Backend
             │                                    │
             └──────────────────┬─────────────────┘
                                ▼
                            PRINTER
```

---

# 116. 開発上最も重要な判断

このプロジェクトでは「Ghostscriptを呼び出す印刷サーバー」を作るだけでは不十分である。

長期的な製品価値を持たせるため、

```text
Document Interpreter
        ↓
RIP Processing
        ↓
Canonical Raster
        ↓
Printer Backend
```

という境界を明確に設ける。

これにより、

PostScript/PDF処理エンジン、

日本語処理、

カラーマネジメント、

ハーフトーン、

プリンター通信

を独立して改良できる。

日本語RIPとしての独自性は特に、

```text
Adobe-Japan1
CID
CMap
縦書き
異体字
Font substitution
Japanese Preflight
```

の精度に置く。

そしてプリンター互換性については特定メーカー独自仕様へ最初から依存せず、

```text
IPP Everywhere
PWG Raster
```

を第一の標準インターフェースとする。

これをJ-RIP Serverの基本アーキテクチャとする。