"""Bounded PDF page selection, shared by both sandbox render adapters."""
import re


def selection(label, limit, ipp=False):
    from pypdf import PdfReader
    if not re.fullmatch(r"[1-9][0-9]*(?:-[1-9][0-9]*)?(?:,[1-9][0-9]*(?:-[1-9][0-9]*)?)*", label):
        raise ValueError("invalid page selection")
    ranges = []
    for item in label.split(","):
        ends = [int(value) for value in item.split("-")]
        first, last = ends[0], ends[-1]
        if not 1 <= first <= last <= (2147483647 if ipp else min(100, limit)) or (ranges and first <= ranges[-1][1]):
            raise ValueError("invalid page selection")
        ranges.append((first, last))
        if len(ranges) > 100:
            raise ValueError("range count limit")
    with open("/input/document", "rb") as source:
        reader = PdfReader(source, strict=True)
        if reader.is_encrypted:
            raise ValueError("encrypted PDF")
        total = len(reader.pages)
    if not 1 <= total <= min(100, limit) or (not ipp and ranges[-1][1] > total):
        raise ValueError("page range out of bounds")
    return [page for first,last in ranges for page in range(first, min(last,total)+1)], total


def empty_stream(total):
    import struct
    import sys
    sys.stdout.buffer.write(b"JRIP-PAGES-2\n" + struct.pack(">I", total) + bytes(12))
    sys.stdout.buffer.flush()
