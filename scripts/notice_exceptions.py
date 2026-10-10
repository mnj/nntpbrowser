"""Explicit, pinned declarations for legacy crates without upstream license files."""
import hashlib

DISPATCH_REVISION = "82d6c7a5b75dc0c71c3f46f87bb6c16a476f7748"
DISPATCH_MANIFEST_SHA256 = "05c60b829931ef7f1f130735843c11f5cf9cc69546eca5fc941d014781c68e31"
MIT_TEMPLATE = """The MIT License (canonical template)
Source: https://opensource.org/license/mit

Copyright <YEAR> <COPYRIGHT HOLDER>

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.
"""


def declaration_texts(package, folder, revision, download):
    """Return only the audited dispatch exception; never infer terms for others."""
    repository = (package.get("repository") or "").replace("http://", "https://", 1)
    if (package["name"], package["version"], package.get("license"), repository, revision) != (
            "dispatch", "0.2.0", "MIT", "https://github.com/SSheldon/rust-dispatch", DISPATCH_REVISION):
        return None
    manifest = (folder / "Cargo.toml.orig").read_bytes()
    source = f"https://raw.githubusercontent.com/SSheldon/rust-dispatch/{revision}/Cargo.toml"
    if hashlib.sha256(manifest).hexdigest() != DISPATCH_MANIFEST_SHA256 or download(source) != manifest:
        raise ValueError("dispatch 0.2.0 pinned upstream manifest does not match registry declaration")
    explanation = f"""Audited legacy declaration exception: dispatch 0.2.0

The pinned upstream repository and published crate contain no separate LICENSE
file. Their byte-identical original Cargo.toml declares license = "MIT" and
authors = ["Steven Sheldon"]. The original declaration is included verbatim.
Steven Sheldon is attributed here as the author declared by upstream.

LICENSE-MIT-STANDARD.txt contains the canonical MIT template, not a license
file supplied by upstream. Its copyright placeholders are intentionally retained:
upstream supplied no separate copyright notice or year, and none is invented.
This documented exception is not a general license inference or certification.

Upstream revision: {revision}
Original manifest SHA256: {DISPATCH_MANIFEST_SHA256}
Original manifest: {source}
Canonical terms: https://opensource.org/license/mit
""".encode()
    return [("Cargo.toml.orig", source, manifest),
            ("NOTICE-EXCEPTION.txt", "Generated explanation of the pinned upstream declaration", explanation),
            ("LICENSE-MIT-STANDARD.txt", "https://opensource.org/license/mit (canonical template)", MIT_TEMPLATE.encode())]
