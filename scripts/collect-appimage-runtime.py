#!/usr/bin/env python3
"""Bundle the pinned AppImage runtime's notices and LGPL rebuildable source."""
import hashlib
import json
from pathlib import Path
import sys
import tarfile
from urllib.request import Request, urlopen

COMMIT = "dd6cebedcbddde9c82f89b011e8e1d40b6e43868"
FILES = [
    ("type2-runtime-source.tar.gz", f"https://codeload.github.com/AppImage/type2-runtime/tar.gz/{COMMIT}", "f5fec23be76e50e2445ed2d018bac49b367490fc483c62c4637e99ec705d27ba"),
    ("fuse-3.15.0.tar.xz", "https://github.com/libfuse/libfuse/releases/download/fuse-3.15.0/fuse-3.15.0.tar.xz", "70589cfd5e1cff7ccd6ac91c86c01be340b227285c5e200baa284e401eea2ca0"),
    ("squashfuse-0.5.2.tar.gz", "https://codeload.github.com/vasi/squashfuse/tar.gz/0.5.2", "db0238c5981dabbd80ee09ae15387f390091668ca060a7bc38047912491443d3"),
    ("musl-COPYRIGHT", "https://git.musl-libc.org/cgit/musl/plain/COPYRIGHT?h=v1.2.5", "f9bc4423732350eb0b3f7ed7e91d530298476f8fec0c6c427a1c04ade22655af"),
    ("zstd-LICENSE", "https://raw.githubusercontent.com/facebook/zstd/v1.5.7/LICENSE", "7055266497633c9025b777c78eb7235af13922117480ed5c674677adc381c9d8"),
    ("zlib-LICENSE", "https://raw.githubusercontent.com/madler/zlib/v1.3.1/LICENSE", "845efc77857d485d91fb3e0b884aaa929368c717ae8186b66fe1ed2495753243"),
    ("mimalloc-LICENSE", "https://raw.githubusercontent.com/microsoft/mimalloc/v2.1.7/LICENSE", "19c99805e7a44a34b297a75d1edea9985e300066dfc024d5c99d4236d4573b5d"),
]


def main(output):
    output = Path(output)
    output.mkdir(parents=True, exist_ok=False)
    provenance = {}
    for name, url, expected in FILES:
        request = Request(url, headers={"User-Agent": "NNTPBrowser-runtime-notices"})
        with urlopen(request, timeout=60) as response:
            data = response.read(32 * 1024 * 1024 + 1)
        if len(data) > 32 * 1024 * 1024 or hashlib.sha256(data).hexdigest() != expected:
            raise ValueError(f"source size or checksum mismatch: {name}")
        (output / name).write_bytes(data)
        provenance[name] = {"url": url, "sha256": expected}
    # Read specific regular files; never extract archive members as paths.
    for archive, member, name in [
        ("type2-runtime-source.tar.gz", f"type2-runtime-{COMMIT}/LICENSE", "runtime-LICENSE"),
        ("fuse-3.15.0.tar.xz", "fuse-3.15.0/LGPL2.txt", "libfuse-LGPL2.txt"),
        ("fuse-3.15.0.tar.xz", "fuse-3.15.0/LICENSE", "libfuse-LICENSE"),
        ("squashfuse-0.5.2.tar.gz", "squashfuse-0.5.2/LICENSE", "squashfuse-LICENSE"),
    ]:
        with tarfile.open(output / archive) as source:
            info = source.getmember(member)
            if not info.isfile() or info.size > 1024 * 1024:
                raise ValueError(f"invalid notice member: {member}")
            with source.extractfile(info) as stream:
                (output / name).write_bytes(stream.read())
    (output / "SOURCES.json").write_text(json.dumps(provenance, indent=2) + "\n")
    (output / "README.md").write_text(f"""# AppImage runtime notices and source

This AppImage embeds upstream type2-runtime release 20251108, commit {COMMIT}.
The runtime is a separate executable prepended to the application filesystem;
NNTPBrowser itself remains MIT. The runtime statically links LGPL libfuse.
Its complete libfuse 3.15.0 source is included, along with the runtime source,
libfuse patch, relinking/build recipes and squashfuse 0.5.2 source. The two
library archive checksums match the pinned upstream install recipe.

To rebuild or change the runtime, unpack type2-runtime-source.tar.gz and read
BUILD.md, scripts/common/install-dependencies.sh and scripts/build-runtime.sh.
Use the included libfuse/squashfuse archives in place of their download commands;
apply patches/libfuse/mount.c.diff as the recipe specifies. The upstream Docker
or Alpine chroot recipes build and statically relink the runtime. They reference
Alpine 3.21 package feeds, which do not pin every native dependency version, so
this is rebuildable source rather than a promise of bit-identical output.
The application's normal executable is independent of this relinking process.
Use appimagetool --runtime-file with a rebuilt runtime to repackage an extracted
AppDir. Modification of the LGPL component is permitted under its terms.

The runtime LICENSE names musl, libfuse, squashfuse, Zstandard and zlib; its
Makefile also links mimalloc. Included permissive-library notice texts have
explicit provenance in SOURCES.json. Their source tags identify notice-text
origins, not unverifiable claims about every Alpine package version in the
upstream binary. Zstandard's BSD alternative is selected. See the actual
runtime, LGPL and third-party texts for copyright and distribution terms.

Extract this directory using --appimage-extract without launching the GUI.
""")
    print("AppImage runtime notices and covered source collected")


if __name__ == "__main__":
    try:
        if len(sys.argv) != 2:
            raise ValueError("usage: collect-appimage-runtime.py <new-output-directory>")
        main(sys.argv[1])
    except (OSError, ValueError, KeyError) as error:
        print(error, file=sys.stderr)
        sys.exit(2)
