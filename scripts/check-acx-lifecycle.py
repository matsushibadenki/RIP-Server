"""Independent schema + cross-record checks for trusted Rust-generated test bundles.

Not a general purpose untrusted receipt verifier or normative ACX implementation.
"""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from jsonschema import Draft202012Validator, FormatChecker

ROOT = Path(__file__).resolve().parents[1]

def check(bundle, public):
    manifest, pf, accepted, result = (bundle[k] for k in ("manifest", "preflight", "accepted", "result"))
    for name, records in (("manifest", [manifest]), ("receipt", [accepted, result])):
        schema = json.loads((ROOT / "tests/acx" / f"acx-{name}.schema.json").read_text())
        Draft202012Validator.check_schema(schema)
        for record in records:
            Draft202012Validator(schema, format_checker=FormatChecker()).validate(record)
    # All canonicalization and Ed25519 verification are performed by Node, independent of Rust.
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "bundle.json"
        path.write_text(json.dumps(bundle, ensure_ascii=False), encoding="utf-8")
        subprocess.run(["node", str(ROOT / "scripts/check-acx-lifecycle.mjs"), str(path)], check=True, capture_output=True)
        for receipt in (accepted, result):
            path.write_text(json.dumps(receipt, ensure_ascii=False), encoding="utf-8")
            subprocess.run(["node", str(ROOT / "scripts/check-receipt-jcs-interop.mjs"), str(public), str(path)], check=True, capture_output=True)

if __name__ == "__main__":
    try:
        check(json.loads(Path(sys.argv[1]).read_text()), sys.argv[2])
    except Exception as error:
        print(f"ACX lifecycle rejected: {error}", file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError):
            print(error.stderr.decode("utf-8", errors="replace"), file=sys.stderr)
        sys.exit(1)
    print("ACX schemas, lifecycle bindings, hashes and pinned signatures verified independently")
