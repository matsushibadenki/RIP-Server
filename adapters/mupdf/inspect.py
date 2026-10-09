"""Bounded, conservative PDF parse/content and page geometry inspection."""
import json
import math
import subprocess
import tempfile


def command(*args):
    with tempfile.TemporaryFile() as output, tempfile.TemporaryFile() as errors:
        result = subprocess.run(["mutool", *args], stdout=output, stderr=errors, timeout=45)
        output.seek(0)
        errors.seek(0)
        data, warnings = output.read(65537), errors.read(65537)
        # This Debian MuPDF build delegates ICC processing to the separate color engine.
        warnings = warnings.replace(b"warning: ICC support is not available\n", b"")
        if result.returncode or warnings or len(data) > 65536:
            raise ValueError("pdf_parse_or_content_failed")
        return data


def main():
    if command("show", "/input/document", "trailer/Encrypt").strip() != b"null":
        raise ValueError("encrypted_pdf_unsupported")
    report = json.loads(command("run", "/usr/local/lib/jrip/inspect.js"))
    for page in report["pages"]:
        for key in ("widthPoints", "heightPoints"):
            value = page[key]
            if not math.isfinite(value) or not 0 < value <= 2834.6457:
                raise ValueError("invalid_page_geometry")
    print(json.dumps(report))



if __name__ == "__main__":
    main()
