#!/usr/bin/env python3
"""Check project-relative site resources and fragment links without dependencies."""
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import unquote, urlsplit


class Page(HTMLParser):
    def __init__(self, path):
        super().__init__()
        self.path = path
        self.ids = set()
        self.links = []
        self.feed(path.read_text())

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if "id" in attrs:
            if attrs["id"] in self.ids:
                raise ValueError(f"{self.path}: duplicate id {attrs['id']}")
            self.ids.add(attrs["id"])
        for name in ("src", "href", "poster", "data-full"):
            if attrs.get(name):
                self.links.append(attrs[name])


def main():
    site = (Path(__file__).resolve().parents[1] / "site").resolve()
    pages = {path.resolve(): Page(path) for path in site.rglob("*.html")}
    if (site / "index.html") not in pages:
        raise ValueError("site/index.html is missing")
    checked = 0
    for page in pages.values():
        for link in page.links:
            parsed = urlsplit(link)
            if parsed.scheme or parsed.netloc:
                continue
            if parsed.path.startswith("/"):
                raise ValueError(f"{page.path}: {link} bypasses the Pages project prefix")
            target = (page.path.parent / unquote(parsed.path)).resolve() if parsed.path else page.path
            if not target.is_relative_to(site):
                raise ValueError(f"{page.path}: {link} escapes site/")
            if target.is_dir():
                target /= "index.html"
            if not target.is_file():
                raise ValueError(f"{page.path}: missing resource {link}")
            if parsed.fragment and target in pages and unquote(parsed.fragment) not in pages[target].ids:
                raise ValueError(f"{page.path}: missing fragment {link}")
            checked += 1
    print(f"Site validated: {len(pages)} HTML pages, {checked} local references")


if __name__ == "__main__":
    main()
