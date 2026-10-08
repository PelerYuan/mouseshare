#!/usr/bin/env python3
"""Documentation hygiene checks run in CI.

1. Every English Markdown document has a `*.zh-CN.md` counterpart, and vice versa.
2. Every relative link in a Markdown file points at a file that exists.
3. Each English/Chinese pair links to the other (the language switcher).

Exit status is non-zero if any check fails.
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
SKIP_DIRS = {"target", "dist", ".git", "node_modules"}
LINK = re.compile(r"\[[^\]]*\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)")


def markdown_files():
    for path in ROOT.rglob("*.md"):
        if SKIP_DIRS.intersection(path.relative_to(ROOT).parts):
            continue
        if ".github" in path.relative_to(ROOT).parts and path.name not in ():
            # Templates and workflows are not user documentation.
            continue
        yield path


def slug(heading: str) -> str:
    s = heading.strip().lower()
    s = re.sub(r"[`*_~]", "", s)
    s = re.sub(r"[^\w一-鿿\- ]", "", s)
    return s.replace(" ", "-")


def anchors(path: pathlib.Path) -> set:
    found = set()
    in_code = False
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.startswith("```"):
            in_code = not in_code
        if not in_code and line.startswith("#"):
            found.add(slug(line.lstrip("#")))
    return found


def main() -> int:
    errors = []
    files = sorted(markdown_files())
    names = {p.relative_to(ROOT).as_posix() for p in files}

    for rel in sorted(names):
        if rel.endswith(".zh-CN.md"):
            en = rel[: -len(".zh-CN.md")] + ".md"
            if en not in names:
                errors.append(f"{rel}: no English original {en}")
        else:
            zh = rel[: -len(".md")] + ".zh-CN.md"
            if zh not in names:
                errors.append(f"{rel}: missing Chinese translation {zh}")
            else:
                text = (ROOT / rel).read_text(encoding="utf-8")
                if pathlib.PurePosixPath(zh).name not in text:
                    errors.append(f"{rel}: does not link to its translation {zh}")
                ztext = (ROOT / zh).read_text(encoding="utf-8")
                if pathlib.PurePosixPath(rel).name not in ztext:
                    errors.append(f"{zh}: does not link back to {rel}")

    for path in files:
        text = path.read_text(encoding="utf-8")
        text = re.sub(r"```.*?```", "", text, flags=re.S)
        for target in LINK.findall(text):
            if re.match(r"^(https?:|mailto:|#)", target):
                if target.startswith("#") and target[1:] not in anchors(path):
                    errors.append(f"{path.relative_to(ROOT)}: broken anchor {target}")
                continue
            file_part, _, frag = target.partition("#")
            dest = (path.parent / file_part).resolve()
            if not dest.exists():
                errors.append(f"{path.relative_to(ROOT)}: broken link {target}")
            elif frag and dest.suffix == ".md" and frag not in anchors(dest):
                errors.append(f"{path.relative_to(ROOT)}: broken anchor {target}")

    for e in errors:
        print("error:", e)
    print(f"checked {len(files)} Markdown files, {len(errors)} problem(s)")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
