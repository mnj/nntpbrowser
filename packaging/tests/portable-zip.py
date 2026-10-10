#!/usr/bin/env python3
"""Exercise Windows' actual ZIP builder with old and hidden upstream files."""
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import zipfile

root = Path(__file__).resolve().parents[2]
script = (root / "packaging/build-windows.ps1").read_text()
code = re.search(r"^\s*\$ZipCode = '([^']+)'$", script, re.MULTILINE).group(1)
with tempfile.TemporaryDirectory(prefix="nntpbrowser-portable-zip-") as temp:
    directory = Path(temp)
    payload = directory / "nntpbrowser_0.1.0_windows_x64"
    files = {
        "nntpbrowser.exe": b"gui executable",
        "nntpbrowser-cli.exe": b"console executable",
        "share/doc/nntpbrowser/third-party/.source/license.txt": b"upstream terms",
    }
    for name, data in files.items():
        path = payload / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        # Cargo's registry sources can intentionally retain Unix-epoch times.
        os.utime(path, (0, 0))
    archive = directory / "portable.zip"
    subprocess.run([sys.executable, "-c", code, str(payload), str(archive)], check=True)
    with zipfile.ZipFile(archive) as zipped:
        assert zipped.testzip() is None
        assert set(zipped.namelist()) == {f"{payload.name}/{name}" for name in files}
        for name, data in files.items():
            entry = f"{payload.name}/{name}"
            assert zipped.read(entry) == data
            assert zipped.getinfo(entry).date_time == (1980, 1, 1, 0, 0, 0)
print("Windows portable ZIP timestamp and complete-payload regression passed")
