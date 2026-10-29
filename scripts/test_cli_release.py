#!/usr/bin/env python3
"""Offline smoke test for a built/released CLI, including Windows stdout pipes."""

import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--version", required=True)
    args = parser.parse_args()
    binary = args.binary.resolve()
    env = {key: value for key, value in os.environ.items() if not key.startswith("LCEDA_")}
    version = subprocess.run(
        [str(binary), "--version"], capture_output=True, text=True,
        encoding="utf-8", timeout=30, check=True, env=env,
    )
    assert version.stdout.strip() == f"lceda {args.version}", version.stdout

    with tempfile.TemporaryDirectory(prefix="lceda-cli-release-") as temporary:
        work = Path(temporary)

        def call(command, arguments, exit_code=0):
            result = subprocess.run(
                [str(binary), "--profile", str(work / "profile"), "--json", *map(str, arguments)],
                cwd=work, capture_output=True, text=True, encoding="utf-8", timeout=30, env=env,
            )
            assert result.returncode == exit_code, (command, result.returncode, result.stdout, result.stderr)
            response = json.loads(result.stdout)
            assert response["schema_version"] == 1, response
            assert response["command"] == command, response
            assert response["ok"] == (exit_code == 0), response
            return response["data"]

        source = work / "board.csv"
        source.write_text('LCSC Part Number,Quantity,Designator\nC25804,2,"R1,R2"\n', encoding="utf-8")
        plan = work / "board.plan.json"
        imported = call("bom.import", ["bom", "import", source, "--plan", plan])
        assert imported["rows"][0]["input"]["quantity"] == 2
        assert imported["rows"][0]["input"]["designators"] == "R1,R2"
        assert not imported["ready"]
        call("bom.show", ["bom", "show", plan])
        call("bom.confirm", ["bom", "confirm", plan, "--exact"], 1)
        before = plan.read_bytes()
        call("bom.confirm", ["bom", "confirm", plan, "--select", "2=C25804"], 2)
        assert plan.read_bytes() == before
        output = work / "output"
        call("bom.export", ["bom", "export", plan, "--ad", "--merge", "-o", output], 2)
        assert not output.exists(), "An unconfirmed plan must not create export files"
        call("arguments", ["get", "C25804", "--pads-grid", "0"], 2)
        call("library.inspect", ["library", "inspect", work / "missing.SchLib"], 2)
        assert not (work / "profile" / "library.json").exists()
    print(f"PASS: lceda {args.version} CLI version, JSON pipes, BOM plans and error contracts")


if __name__ == "__main__":
    main()
