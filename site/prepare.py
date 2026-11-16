#!/usr/bin/env python3
"""Fills the site from the repository: releases from CHANGELOG.md, the docs from the README's
how-to and reference half, the sample shader the hero plays, and the app's fonts. Run before
`zola build`; what it writes is ignored."""

import re
import shutil
import sys
from pathlib import Path

SITE = Path(__file__).resolve().parent
ROOT = SITE.parent
REPO = "https://github.com/Pisosenie/apolline-wall"
LINK = re.compile(r"\]\(([^)\s]+)\)")
RELEASE = re.compile(r"^## (\d+\.\d+\.\d+) — (\d{4}-\d{2}-\d{2})\s*$")
DOCS_FROM = "## How to"
DOCS_UNTIL = "## License"


def linked(text):
    """Links into the repository go to GitHub; anchors stay, the docs page has those sections."""

    def fix(match):
        target = match.group(1)
        if re.match(r"^(https?:|mailto:|#|@/)", target):
            return match.group(0)
        path, _, anchor = target.partition("#")
        resolved = (ROOT / path).resolve().relative_to(ROOT).as_posix()
        kind = "tree" if (ROOT / resolved).is_dir() else "blob"
        return "](%s/%s/main/%s%s)" % (REPO, kind, resolved, "#" + anchor if anchor else "")

    return LINK.sub(fix, text)


def releases():
    out = SITE / "content" / "news"
    for old in out.glob("*.md"):
        if old.name != "_index.md":
            old.unlink()
    found, current, body = [], None, []
    for line in (ROOT / "CHANGELOG.md").read_text().splitlines():
        match = RELEASE.match(line)
        if match:
            if current:
                found.append((current, body))
            current, body = match.groups(), []
        elif current:
            body.append(line)
    if current:
        found.append((current, body))
    for weight, ((version, date), lines) in enumerate(found, 1):
        slug = version.replace(".", "-")
        front = (
            '+++\ntitle = "apolline-wall %s"\ndate = %s\nweight = %d\n'
            'slug = "%s"\n[extra]\nversion = "%s"\n+++\n\n'
        ) % (version, date, weight, slug, version)
        (out / (slug + ".md")).write_text(front + linked("\n".join(lines).strip()) + "\n")
    return len(found)


def docs():
    text = (ROOT / "README.md").read_text()
    start, end = text.find(DOCS_FROM), text.find(DOCS_UNTIL)
    if start < 0 or end < start:
        sys.exit("the README lost its '%s' or '%s' heading" % (DOCS_FROM, DOCS_UNTIL))
    front = (
        '+++\ntitle = "Docs"\ndescription = "How to use apolline-wall, and everything it takes."\n'
        'template = "docs.html"\n+++\n\n'
    )
    out = SITE / "content" / "docs"
    out.mkdir(exist_ok=True)
    (out / "_index.md").write_text(front + linked(text[start:end].strip()) + "\n")


def static():
    out = SITE / "static"
    shutil.rmtree(out / "fonts", ignore_errors=True)
    shutil.copytree(ROOT / "app" / "assets" / "fonts", out / "fonts")
    shutil.copy(ROOT / "tests" / "shaders" / "hills.frag", out / "hills.frag")


def main():
    count = releases()
    if count == 0:
        sys.exit("no releases read from CHANGELOG.md")
    docs()
    static()
    print("site: %d releases, docs, fonts and the shader in place" % count)


if __name__ == "__main__":
    main()
