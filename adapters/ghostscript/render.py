"""Stream only closed Ghostscript TIFF pages; engine stdout is discarded."""
import sys
sys.path.insert(0, "/usr/local/lib/jrip")
from stream_pages import stream_pages

last = [int(arg.split("=", 1)[1]) for arg in sys.argv[1:] if arg.startswith("-dLastPage=")]
if len(last) != 1 or not 2 <= last[0] <= 10001:
    raise ValueError("invalid page limit")
stream_pages(["gs", *sys.argv[1:]], "tiff", last[0] - 1, lambda page: page)
