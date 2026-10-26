"""Regression for minimal CI installs silently omitting JSON Schema date formats."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import unittest

from jsonschema import Draft202012Validator, ValidationError

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/check-acx-lifecycle.py"
spec = importlib.util.spec_from_file_location("acx_lifecycle_checker", SCRIPT)
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)
SCHEMA = json.loads((ROOT / "tests/acx/acx-receipt.schema.json").read_text())


class RequiredDateTime(unittest.TestCase):
    def setUp(self):
        self.validator = Draft202012Validator(
            SCHEMA["properties"]["issuedAt"], format_checker=checker.FORMAT_CHECKER
        )

    def test_rfc3339_timestamps_are_accepted(self):
        for timestamp in [
            "2026-10-27T12:34:56Z",
            "2026-10-27T21:34:56+09:00",
            "2024-02-29T12:34:56.123Z",
            "2026-10-27t12:34:56z",
        ]:
            with self.subTest(timestamp=timestamp):
                self.validator.validate(timestamp)

    def test_resigned_but_malformed_dates_cannot_pass_schema_validation(self):
        for timestamp in [
            "tampered", "", "2026-10-27", "2026-10-27T12:34:56",
            "2026-02-29T12:34:56Z", "2026-02-30T12:34:56Z",
            "2026-10-27T25:34:56Z", "2026-10-27T12:34:56+25:00",
            None, 123,
        ]:
            with self.subTest(timestamp=timestamp):
                with self.assertRaises(ValidationError):
                    self.validator.validate(timestamp)

    def test_missing_required_dependency_fails_instead_of_disabling_formats(self):
        result = subprocess.run(
            [sys.executable, "-c",
             "import runpy,sys; sys.modules['rfc3339_validator']=None; runpy.run_path(sys.argv[1])",
             str(SCRIPT)], capture_output=True, text=True, check=False,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("rfc3339_validator", result.stderr)


if __name__ == "__main__":
    unittest.main()
