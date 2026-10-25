#!/usr/bin/env python3
"""Fail when a Markdown link points at a path that does not exist."""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LINK = re.compile(r"\[[^\]]*\]\(([^)]+)\)")
SKIP_PREFIXES = ("http://", "https://", "mailto:", "#")


def tracked_markdown() -> list[Path]:
    result = subprocess.run(
        ["git", "ls-files", "*.md"],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        return sorted(ROOT.rglob("*.md"))
    return [ROOT / line for line in result.stdout.splitlines() if line]


def main() -> int:
    broken: list[str] = []
    for path in tracked_markdown():
        text = path.read_text(encoding="utf-8")
        in_fence = False
        for number, line in enumerate(text.splitlines(), start=1):
            if line.lstrip().startswith("```"):
                in_fence = not in_fence
                continue
            if in_fence:
                continue
            line = re.sub(r"`[^`]*`", "", line)
            for target in LINK.findall(line):
                target = target.split("#", 1)[0].strip()
                if not target or target.startswith(SKIP_PREFIXES):
                    continue
                target = target.strip("<>")
                resolved = (path.parent / target).resolve()
                if not resolved.exists():
                    broken.append(
                        f"{path.relative_to(ROOT)}:{number}: {target} does not exist"
                    )
    if broken:
        print("\n".join(broken), file=sys.stderr)
        return 1
    print("documentation links resolve")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
