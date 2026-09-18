"""Render PDF with MuPDF and wrap its 8-bit PAM samples in baseline TIFF.

No reshaping, resampling, or color conversion occurs in this packaging step.
Both rendering and packaging run inside the same bounded sandbox.
"""
import pathlib
import shutil
import struct
import subprocess
import sys


def pam_to_tiff(source, destination, dpi, mode):
    expected = {"gray": (1, "GRAYSCALE", 1), "rgb": (3, "RGB", 2),
                "cmyk": (4, "CMYK", 5)}
    channels, tuple_type, photometric = expected[mode]
    with open(source, "rb") as src:
        if src.readline(4) != b"P7\n":
            raise ValueError("invalid PAM signature")
        fields = {}
        for _ in range(32):
            line = src.readline(256)
            if line == b"ENDHDR\n":
                break
            if not line or not line.endswith(b"\n"):
                raise ValueError("invalid PAM header")
            if line.startswith(b"#"):
                continue
            key, value = line.decode("ascii").split(maxsplit=1)
            if key in fields:
                raise ValueError("duplicate PAM field")
            fields[key] = value.strip()
        else:
            raise ValueError("PAM header limit")
        width, height = int(fields["WIDTH"]), int(fields["HEIGHT"])
        if (int(fields["DEPTH"]) != channels or fields["MAXVAL"] != "255"
                or fields["TUPLTYPE"] != tuple_type or width < 1 or height < 1):
            raise ValueError("unsupported PAM layout")
        size = width * height * channels
        if size + 512 >= 2**32 or width >= 2**32 or height >= 2**32:
            raise ValueError("TIFF size limit")
        if pathlib.Path(source).stat().st_size - src.tell() != size:
            raise ValueError("PAM sample length mismatch")
        # A single interleaved strip is copied in bounded chunks; never load a page.
        tags = [
            (256, 4, 1, struct.pack("<I", width)),
            (257, 4, 1, struct.pack("<I", height)),
            (258, 3, channels, struct.pack("<" + "H" * channels, *([8] * channels))),
            (259, 3, 1, struct.pack("<H", 1)),
            (262, 3, 1, struct.pack("<H", photometric)),
            (273, 4, 1, struct.pack("<I", 512)),
            (277, 3, 1, struct.pack("<H", channels)),
            (278, 4, 1, struct.pack("<I", height)),
            (279, 4, 1, struct.pack("<I", size)),
            (282, 5, 1, struct.pack("<II", dpi, 1)),
            (283, 5, 1, struct.pack("<II", dpi, 1)),
            (284, 3, 1, struct.pack("<H", 1)),
            (296, 3, 1, struct.pack("<H", 2)),
        ]
        if mode == "cmyk":
            tags.extend([(332, 3, 1, struct.pack("<H", 1)),
                         (334, 3, 1, struct.pack("<H", 4))])
        header = bytearray(b"II\x2a\x00" + struct.pack("<I", 8))
        header.extend(struct.pack("<H", len(tags)))
        extra = bytearray()
        extra_offset = 8 + 2 + 12 * len(tags) + 4
        for tag, kind, count, value in sorted(tags):
            if len(value) <= 4:
                field = value.ljust(4, b"\0")
            else:
                field = struct.pack("<I", extra_offset + len(extra))
                extra.extend(value)
            header.extend(struct.pack("<HHI", tag, kind, count) + field)
        header.extend(struct.pack("<I", 0))
        header.extend(extra)
        if len(header) > 512:
            raise ValueError("TIFF header size limit")
        with open(destination, "xb") as dst:
            dst.write(header.ljust(512, b"\0"))
            shutil.copyfileobj(src, dst, length=65536)


def main():
    dpi, mode, last_page = int(sys.argv[1]), sys.argv[2], int(sys.argv[3])
    if not 72 <= dpi <= 2400 or mode not in ("rgb", "cmyk", "gray") or not 2 <= last_page <= 10001:
        raise ValueError("invalid render options")
    subprocess.run([
        "mutool", "draw", "-q", "-F", "pam", "-c", mode,
        "-r", str(dpi), "-B", "128", "-o", "/output/page-%06d.pam",
        "/input/document", "1-" + str(last_page),
    ], check=True, stdout=sys.stderr)
    pages = sorted(pathlib.Path("/output").glob("page-*.pam"))
    if not pages or len(pages) >= last_page:
        raise ValueError("empty output or page limit exceeded")
    for page in pages:
        pam_to_tiff(page, page.with_suffix(".tiff"), dpi, mode)
        page.unlink()
    subprocess.run(["tar", "-C", "/output", "-cf", "-", "."], check=True)


if __name__ == "__main__":
    main()
