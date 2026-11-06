"""Stream only closed Ghostscript TIFF pages; engine stdout is discarded."""
import sys
from pathlib import Path
sys.path.append("/usr/local/lib/jrip")
from stream_pages import stream_pages
from page_selection import selection, empty_stream
from font_policy import inspect_fonts

# Do not silently fall back to the image's default substitution maps.
for name in ("cidfmap", "FAPIcidfmap"):
    policy = Path("/usr/local/lib/jrip/strict-resources") / name
    if not policy.is_file() or any(line.strip() and not line.lstrip().startswith('%') for line in policy.read_text().splitlines()):
        raise ValueError("invalid_cid_font_policy")
# The pinned GS build can substitute non-embedded CID programs despite its
# PDFNOCIDFALLBACK flag. Enforce the declared-resource policy independently.
args = sys.argv[1:]
formats = [arg.split("=", 1)[1] for arg in args if arg.startswith("--jrip-format=")]
if len(formats) != 1 or formats[0] not in ("pdf", "postscript", "eps"):
    raise ValueError("invalid_document_format")
args = [arg for arg in args if not arg.startswith("--jrip-format=")]
if formats[0] == "pdf":
    inspect_fonts("/input/document")
last = [int(arg.split("=", 1)[1]) for arg in args if arg.startswith("-dLastPage=")]
if len(last) != 1 or not 2 <= last[0] <= 10001:
    raise ValueError("invalid page limit")
labels = [arg.split("=", 1)[1] for arg in args if arg.startswith(("--jrip-pages=", "--jrip-ipp-pages="))]
chosen, total = None, None
if labels:
    if len(labels) != 1:
        raise ValueError("invalid page selection")
    chosen, total = selection(labels[0], last[0] - 1, any(arg.startswith("--jrip-ipp-pages=") for arg in args))
    args = [arg for arg in args if not arg.startswith(("--jrip-pages=", "--jrip-ipp-pages=", "-dLastPage="))]
    args.insert(0, "-sPageList=" + ",".join(map(str, chosen)))
if chosen == []:
    empty_stream(total)
    sys.exit(0)
stream_pages(["gs", *args], "tiff", len(chosen) if chosen else last[0] - 1,
             lambda page: page, total)
