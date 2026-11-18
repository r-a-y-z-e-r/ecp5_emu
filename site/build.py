#!/usr/bin/env python3
"""Stage the repository's documentation, build mdBook, and check rendered links.

Python standard library only. All generated files stay in site/. Hand-written
pages live in site/pages/ and use links relative to their own folder, so they
also work on GitHub. Staging at the book root rewrites those links to staged
chapters and assets or GitHub URLs. Copied chapters keep their repository paths
and link conventions. Both rendered links and hand-written pages' repository
targets are checked. SUMMARY order is editorial; discovery checks for omissions.
"""
from __future__ import annotations

import argparse
from collections import Counter
from html import unescape
from html.parser import HTMLParser
import json
import os
from pathlib import Path, PurePosixPath
import posixpath
import re
import shutil
import subprocess
import sys
from urllib.parse import quote, unquote, urlsplit, urlunsplit

SITE = Path(__file__).resolve().parent
REPO = SITE.parent
STAGE = SITE / "build/src"
BOOK = SITE / "book"
GITHUB = "https://github.com/r-a-y-z-e-r/ecp5_emu"
# Cap the generated search index while allowing mdBook's fixed 10 MB notice.
SEARCH_INDEX_BUDGET_BYTES = 15_000_000
# Local files of these kinds are copied into the book rather than linked on GitHub.
IMAGES = {".png", ".jpg", ".jpeg", ".gif", ".svg", ".webp", ".avif", ".ico", ".mp4"}

# Each (title, repository path) occurs once. None denotes a sidebar part.
CHAPTERS = [
    ("Introduction", "introduction.md"),
    ("What it does in detail", "overview.md"),
    ("Getting started", "getting-started.md"),
    ("The window", "window-guide.md"),
    ("Window loading and measurements", "docs/WINDOW.md"),
    ("Projects", "projects.md"),
    ("Architecture overview", "architecture.md"),
    ("How it is verified", "verification.md"),
    ("Tests and measured times", "tests.md"),
    ("Related work", "related-work.md"),
    ("Acknowledgements", "acknowledgements.md"),
    ("Using it", None),
    ("ULX3S examples and soft CPU", "examples/ulx3s/README.md"),
    ("Gallery", "examples/gallery/README.md"),
    ("Gallery: riscv_shell", "examples/gallery/riscv_shell/README.md"),
    ("Gallery: serv_shell", "examples/gallery/serv_shell/README.md"),
    ("Gallery: riscv_terminal", "examples/gallery/riscv_terminal/README.md"),
    ("Gallery: mandelbrot", "examples/gallery/mandelbrot/README.md"),
    ("Gallery: fir_scope", "examples/gallery/fir_scope/README.md"),
    ("Gallery: video_life", "examples/gallery/video_life/README.md"),
    ("Gallery: hdmi_plasma", "examples/gallery/hdmi_plasma/README.md"),
    ("Gallery: sdram_bench", "examples/gallery/sdram_bench/README.md"),
    ("Gallery: ebr_life", "examples/gallery/ebr_life/README.md"),
    ("Gallery: led_tennis", "examples/gallery/led_tennis/README.md"),
    ("Gallery: chip_synth", "examples/gallery/chip_synth/README.md"),
    ("Gallery: oled_bricks", "examples/gallery/oled_bricks/README.md"),
    ("Gallery: clock_domains", "examples/gallery/clock_domains/README.md"),
    ("Gallery: sha256_chains", "examples/gallery/sha256_chains/README.md"),
    ("Gallery: riscv_oled", "examples/gallery/riscv_oled/README.md"),
    ("Peripherals: HDMI and the board", "docs/PERIPHERALS.md"),
    ("Peripherals: HDMI EDID", "docs/PERIPHERALS_EDID.md"),
    ("Peripherals: the OLED", "docs/PERIPHERALS_OLED.md"),
    ("Peripherals: the SDRAM", "docs/PERIPHERALS_SDRAM.md"),
    ("Peripherals: the configuration flash", "docs/PERIPHERALS_FLASH.md"),
    ("Peripherals: the ADC", "docs/PERIPHERALS_ADC.md"),
    ("Peripherals: board headers", "docs/PERIPHERALS_HEADER.md"),
    ("Peripherals: the RTC", "docs/PERIPHERALS_RTC.md"),
    ("Peripherals: the SD card", "docs/PERIPHERALS_SD.md"),
    ("Peripherals: the audio jack", "docs/PERIPHERALS_AUDIO.md"),
    ("HDL names", "docs/HDL_NAMES.md"),
    ("Toolchain", "docs/TOOLCHAIN.md"),
    ("Building the board model", "tools/ulx3s_3d/README.md"),
    ("Board model QA", "docs/qa/ulx3s_3d/README.md"),
    ("Execution", None),
    ("Automatic execution", "docs/AUTOMATIC_EXECUTION.md"),
    ("Configuration decoder", "docs/CONFIG_DECODER.md"),
    ("Configuration and state", "docs/CONFIGURATION_STATE.md"),
    ("Structural model", "docs/STRUCTURAL_MODEL.md"),
    ("Event kernel", "docs/EVENT_KERNEL.md"),
    ("Fast mode", "docs/FAST_MODE.md"),
    ("Global controls and wake phases", "docs/SYSTEM_EXECUTION.md"),
    ("Startup and system research", "docs/SYSTEM_STARTUP.md"),
    ("Resource families", None),
    ("Logic and LUTRAM", "docs/PLC_LUTRAM.md"),
    ("Flip-flops and carry", "docs/FF_CCU2.md"),
    ("LUTRAM execution", "docs/LUTRAM_EXECUTION.md"),
    ("Memory and DSP inventory", "docs/MEMORY_DSP_RESOURCES.md"),
    ("EBR addressing", "docs/EBR_ADDRESSING.md"),
    ("EBR controls", "docs/EBR_CONTROL.md"),
    ("EBR execution", "docs/EBR_EXECUTION.md"),
    ("EBR mode overlays", "docs/EBR_MODE_OVERLAY.md"),
    ("DSP internals", "docs/DSP_INTERNALS.md"),
    ("DSP routing", "docs/DSP_ROUTING.md"),
    ("MULT18 execution", "docs/DSP_EXECUTION.md"),
    ("DSP registers", "docs/DSP_REGISTERS.md"),
    ("DSP output bank", "docs/DSP_OUTPUT.md"),
    ("DSP operand forwarding", "docs/DSP_FORWARDING.md"),
    ("Observing ALU operands", "docs/DSP_ALU_INPUTS.md"),
    ("ALU54 execution", "docs/DSP_ALU.md"),
    ("PIO and banks", "docs/PIO_BANKS.md"),
    ("IOLOGIC resources", "docs/IOLOGIC_RESOURCES.md"),
    ("IO routing", "docs/IO_ROUTING.md"),
    ("IOLOGIC execution", "docs/IOLOGIC_EXECUTION.md"),
    ("Clock networks", "docs/CLOCK_NETWORK.md"),
    ("Clock and system resources", "docs/CLOCK_SYSTEM_RESOURCES.md"),
    ("PLL execution", "docs/PLL_EXECUTION.md"),
    ("Oscillator execution", "docs/OSC_EXECUTION.md"),
    ("DQS and DDRDLL research", "docs/DQS_CONTROL.md"),
    ("Device research", None),
    ("Device passport", "docs/DEVICE_PASSPORT.md"),
    ("Resource bindings", "docs/RESOURCE_BINDINGS.md"),
    ("Resource coverage", "docs/RESOURCE_COVERAGE.md"),
    ("Settings coverage", "docs/SETTINGS_COVERAGE.md"),
    ("Refusals and qualified profiles", "docs/REFUSAL_AUDIT.md"),
    ("Research status", "docs/RESEARCH_STATUS.md"),
    ("Core milestone", "docs/CORE_STATUS.md"),
    ("Packages and JTAG", "docs/PACKAGE_JTAG.md"),
    ("Hardware validation", None),
    ("Hardware validation results and queue", "docs/HARDWARE_VALIDATION.md"),
    # Inventory READMEs are inserted here as children of HARDWARE_VALIDATION.
    ("Project history and plans", None),
    ("Roadmap", "ROADMAP.md"),
    ("Original plan", "PLAN.md"),
    ("Implementation plan", "docs/IMPLEMENTATION_PLAN.md"),
]


def inside(path: Path, root: Path) -> bool:
    return path.resolve().is_relative_to(root.resolve())


def writable(path: Path) -> Path:
    if not inside(path, SITE):
        raise ValueError(f"refusing to write outside site/: {path}")
    return path


def sources() -> dict[str, Path]:
    result = {p.name: p for p in sorted((SITE / "pages").glob("*.md"))}
    originals = [*REPO.glob("docs/*.md"),
                 *REPO.glob("docs/inventory/**/README.md"),
                 REPO / "ROADMAP.md", REPO / "PLAN.md",
                 REPO / "examples/ulx3s/README.md",
                 *REPO.glob("examples/gallery/README.md"),
                 *REPO.glob("examples/gallery/*/README.md"),
                 REPO / "tools/ulx3s_3d/README.md"]
    qa = REPO / "docs/qa/ulx3s_3d/README.md"
    if qa.is_file():
        originals.append(qa)
    for p in originals:
        if not p.is_file() or not inside(p, REPO):
            raise ValueError(f"missing or out-of-repository source: {p}")
        result[p.relative_to(REPO).as_posix()] = p
    return result


def summary(pages: dict[str, Path]) -> str:
    lines = ["# Summary", ""]
    used = []
    for title, path in CHAPTERS:
        if path is None:
            lines.extend(["", f"# {title}", ""])
            continue
        if path not in pages:
            # A document listed here was removed or renamed: leave it out, say so.
            print(f"warning: SUMMARY lists {path}, which does not exist; left out", file=sys.stderr)
            continue
        lines.append(f"- [{title}]({path})")
        used.append(path)
        if path == "docs/HARDWARE_VALIDATION.md":
            for record in sorted(p for p in pages if p.startswith("docs/inventory/")):
                # Read each record's heading: the ID preceding ':' is the series.
                heading = next(l for l in pages[record].read_text().splitlines() if l.startswith("# "))
                series = heading[2:].split(":", 1)[0].split()[0]
                lines.append(f"  - [{series}]({record})")
                used.append(record)
    if any(n != 1 for n in Counter(used).values()):
        raise ValueError(f"SUMMARY lists a page twice: {sorted(p for p, n in Counter(used).items() if n != 1)}")
    # A document added since CHAPTERS was written still gets a page, at the end,
    # so a new doc in another change never breaks the site; place it by hand later.
    missing = sorted(set(pages) - set(used))
    if missing:
        lines.extend(["", "# More documents", ""])
        for path in missing:
            heading = next((l[2:].strip() for l in pages[path].read_text().splitlines() if l.startswith("# ")), path)
            lines.append(f"- [{heading}]({path})")
            print(f"warning: {path} is not placed in CHAPTERS; added under 'More documents'", file=sys.stderr)
    return "\n".join(lines) + "\n"


def noncode(text: str) -> str:
    """Mask fenced blocks and inline code, preserving character offsets.

    Do not discard four-space lines: nested Markdown list continuations in
    the evidence records use that indentation and contain real links.
    """
    output = []
    fence = None
    for line in text.splitlines(keepends=True):
        m = re.match(r" {0,3}(`{3,}|~{3,})", line)
        if fence:
            output.append(re.sub(r"[^\n]", " ", line))
            if m and m[1][0] == fence[0] and len(m[1]) >= len(fence) and not line[m.end():].strip():
                fence = None
        elif m:
            fence = m[1]
            output.append(re.sub(r"[^\n]", " ", line))
        else:
            output.append(line)
    masked = "".join(output)
    return re.sub(r"(`+)(.+?)\1", lambda m: re.sub(r"[^\n]", " ", m[0]), masked, flags=re.S)


def destinations(text: str):
    """Yield (start, end, destination, image) for Markdown and raw HTML links.

    Balanced parentheses in inline destinations, angle destinations, titles,
    reference definitions, and HTML href/src are supported. We leave labels,
    code and prose alone; mdBook's rendered HTML is the final link authority.
    """
    masked = noncode(text)
    found = {}
    # Every "](" starts a destination, so a link around an image
    # ([![alt](picture)](target)) yields both the picture and the target.
    for m in re.finditer(r"\]\(\s*", masked):
        start = m.end()
        if start >= len(masked):
            continue
        depth, label = 0, m.start() - 1
        while label >= 0 and masked[label] != "\n":
            if masked[label] == "]":
                depth += 1
            elif masked[label] == "[":
                if depth == 0:
                    break
                depth -= 1
            label -= 1
        if label < 0 or masked[label] != "[":
            continue
        image = label > 0 and masked[label - 1] == "!"
        if masked[start] == "<":
            end = masked.find(">", start + 1)
            if end < 0:
                continue
            start += 1
        else:
            end, depth = start, 0
            while end < len(masked):
                c = masked[end]
                if c == "\\":
                    end += 2
                    continue
                if c == "(":
                    depth += 1
                elif c == ")":
                    if depth == 0:
                        break
                    depth -= 1
                elif c.isspace() and depth == 0:
                    break
                end += 1
        if end > start:
            found[(start, end)] = image
    for m in re.finditer(r"^ {0,3}\[[^\]\n]+\]:\s*(?:<([^>\n]+)>|([^\s]+))", masked, re.M):
        g = 1 if m[1] is not None else 2
        found[m.span(g)] = False
    for m in re.finditer(r"<(?:a|img|source)\b[^>]*>", masked, re.I):
        for attr in re.finditer(r"\b(href|src)\s*=\s*([\"'])(.*?)\2", m[0], re.I | re.S):
            a, b = attr.span(3)
            found[(m.start()+a, m.start()+b)] = attr[1].lower() == "src"
    for (a, b), is_image in sorted(found.items()):
        yield a, b, unescape(text[a:b]), is_image


class Stager:
    def __init__(self, pages: dict[str, Path], ref: str):
        self.pages, self.ref = pages, ref
        self.staged_paths = {source.relative_to(REPO).as_posix(): page
                             for page, source in pages.items()}
        self.rewritten = 0
        self.chapter_rewritten = 0
        self.images = set()
        self.issues = []

    def target(self, page: str, url: str, is_image: bool, line: int) -> str:
        parts = urlsplit(url)
        if parts.scheme or parts.netloc or not parts.path:
            return url
        raw = unquote(parts.path).replace("\\", "")
        original = self.pages[page].relative_to(REPO).as_posix()
        target = posixpath.normpath(posixpath.join(posixpath.dirname(original), raw))
        repo_target = REPO / target
        if not inside(repo_target, REPO):
            self.issue(page, line, url, "target escapes repository")
            return url
        staged = (self.staged_paths.get(target) if original != page
                  else target if target in self.pages else None)
        if staged is not None:
            local = (posixpath.relpath(staged, posixpath.dirname(page) or ".")
                     if original != page else raw)
            # mdBook emits README.md chapters as index.html, but its built-in
            # Markdown link rewrite emits README.html. Use the real output
            # name here without renaming or duplicating the source chapter.
            if PurePosixPath(target).name == "README.md":
                local = str(PurePosixPath(local).with_name("index.html"))
                self.chapter_rewritten += 1
            elif original == page:
                return url
            return urlunsplit(("", "", local, parts.query, parts.fragment))
        if repo_target.suffix.lower() in IMAGES and repo_target.is_file():
            out = writable(STAGE / target)
            out.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(repo_target, out)
            self.images.add(target)
            if original == page:
                return url
            local = posixpath.relpath(target, posixpath.dirname(page) or ".")
            return urlunsplit(("", "", local, parts.query, parts.fragment))
        if is_image:
            self.issue(page, line, url, "image does not exist")
        elif not repo_target.exists():
            self.issue(page, line, url, f"source target does not exist: {target}")
        kind = "tree" if repo_target.is_dir() or raw.endswith("/") else "blob"
        self.rewritten += 1
        # Encode the ref as one path component (also supports feature/foo).
        path = f"{GITHUB}/{kind}/{quote(self.ref, safe='')}/{quote(target, safe='/')}"
        return urlunsplit(("https", "github.com", urlsplit(path).path, parts.query, parts.fragment))

    def issue(self, page: str, line: int, url: str, reason: str):
        original = self.pages[page].relative_to(REPO).as_posix()
        self.issues.append({"file": original, "line": line, "target": url, "reason": reason})

    def stage(self):
        writable(STAGE)
        if STAGE.exists():
            shutil.rmtree(STAGE)
        STAGE.mkdir(parents=True)
        for page, source in self.pages.items():
            text = source.read_text(encoding="utf-8")
            edits = []
            for start, end, url, img in destinations(text):
                line = text.count("\n", 0, start) + 1
                replacement = self.target(page, url, img, line)
                if replacement != url:
                    edits.append((start, end, replacement))
            for start, end, replacement in reversed(edits):
                text = text[:start] + replacement + text[end:]
            dest = writable(STAGE / page)
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_text(text, encoding="utf-8")
        (STAGE / "SUMMARY.md").write_text(summary(self.pages), encoding="utf-8")
        report = SITE / "build/source-link-issues.json"
        report.write_text(json.dumps(self.issues, indent=2) + "\n")
        return report


def check_page_links(pages: dict[str, Path]) -> tuple[int, list[str]]:
    """Check hand-written links as GitHub resolves them, ignoring anchors."""
    checked, broken = 0, []
    for source in pages.values():
        if source.parent != SITE / "pages":
            continue
        text = source.read_text(encoding="utf-8")
        for start, end, url, is_image in destinations(text):
            parts = urlsplit(url)
            if parts.scheme or parts.netloc or not parts.path:
                continue
            checked += 1
            target = source.parent / unquote(parts.path).replace("\\", "")
            why = None
            if not inside(target, REPO):
                why = "target escapes repository"
            elif not target.exists():
                why = "missing file or directory"
            if why:
                line = text.count("\n", 0, start) + 1
                broken.append(f"{source.relative_to(REPO)}:{line}: {url} ({why})")
    return checked, broken


class Links(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.ids = set()
        self.links = []

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        for key in ("id", "name" if tag == "a" else "id"):
            if attrs.get(key):
                self.ids.add(attrs[key])
        for key in ("href", "src", "poster", "data" if tag == "object" else "src"):
            if attrs.get(key) and (self.getpos()[0], attrs[key]) not in self.links:
                self.links.append((self.getpos()[0], attrs[key]))
        if attrs.get("srcset"):
            self.links.extend((self.getpos()[0], item.strip().split()[0])
                              for item in attrs["srcset"].split(",") if item.strip())

    handle_startendtag = handle_starttag


def check_links() -> tuple[int, list[str]]:
    parsed = {}
    for p in sorted(BOOK.rglob("*.html")):
        parser = Links()
        parser.feed(p.read_text(encoding="utf-8"))
        parsed[p.resolve()] = parser
    checked, broken = 0, []
    for page, parser in parsed.items():
        for line, url in parser.links:
            parts = urlsplit(url)
            if parts.scheme or parts.netloc:
                continue
            checked += 1
            path = unquote(parts.path)
            target = (page if not path else BOOK / path.lstrip("/")
                      if path.startswith("/") else page.parent / path)
            target = target.resolve()
            if target.is_dir():
                target = target / "index.html"
            why = None
            if not inside(target, BOOK):
                why = "escapes book/"
            elif not target.is_file():
                why = "missing file"
            elif parts.fragment and target.suffix.lower() == ".html":
                anchor = unquote(parts.fragment)
                # Browsers support text fragments independently of element IDs.
                anchor = anchor.split(":~:text=", 1)[0]
                if anchor and anchor not in parsed[target].ids:
                    why = f"missing anchor #{anchor}"
            if why:
                broken.append(f"{page.relative_to(BOOK)}:{line}: {url} ({why})")
    return checked, broken


def main() -> int:
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("--ref", default="main", help="GitHub ref for unstaged files (default: main)")
    cli.add_argument("--serve", action="store_true", help="check the book, then run mdbook serve")
    args = cli.parse_args()
    mdbook = shutil.which("mdbook")
    if not mdbook:
        cli.error("mdbook is not on PATH (tested with v0.5.4)")
    pages = sources()
    stager = Stager(pages, args.ref)
    report = stager.stage()
    # Respect the output boundary even if a generated directory is a symlink.
    writable(BOOK)
    result = subprocess.run([mdbook, "build", str(SITE)], text=True,
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                            env={**os.environ, "RUST_LOG": "info"})
    (SITE / "build/mdbook.log").write_text(result.stdout, encoding="utf-8")
    print(result.stdout, end="", flush=True)
    if result.returncode:
        return result.returncode
    checked, broken = check_links()
    page_checked, page_broken = check_page_links(pages)
    warnings = [l for l in result.stdout.splitlines() if re.search(r"\bWARN(?:ING)?\b", l, re.I)]
    search_notices = [l for l in warnings if re.fullmatch(
        r"\s*WARN\s+search index is very large \([0-9]+ bytes\)\s*", l)]
    warnings = [l for l in warnings if l not in search_notices]
    search_indexes = list(BOOK.glob("searchindex*.js"))
    if len(search_indexes) != 1:
        raise ValueError(f"expected one search index, found {len(search_indexes)}")
    search_index_bytes = search_indexes[0].stat().st_size
    stats = {"pages_staged": len(pages), "images_copied": len(stager.images),
             "links_rewritten": stager.rewritten + stager.chapter_rewritten,
             "github_links_rewritten": stager.rewritten,
             "readme_links_rewritten": stager.chapter_rewritten, "links_checked": checked,
             "broken_internal_links": len(broken), "mdbook_warnings": len(warnings),
             "mdbook_search_index_notices": len(search_notices),
             "search_index_bytes": search_index_bytes,
             "search_index_budget_bytes": SEARCH_INDEX_BUDGET_BYTES,
             "page_links_checked": page_checked,
             "page_links_unresolvable_on_github": len(page_broken),
             "source_link_issues": len(stager.issues), "ref": args.ref}
    (SITE / "build/report.json").write_text(json.dumps(stats, indent=2) + "\n")
    print(f"Pages staged: {len(pages)}; images copied: {len(stager.images)}; "
          f"links rewritten: {stats['links_rewritten']} ({stager.rewritten} GitHub, "
          f"{stager.chapter_rewritten} README paths); links checked: {checked}; "
          f"broken internal links: {len(broken)}; "
          f"page links unresolvable on GitHub: {len(page_broken)}; "
          f"mdBook warnings: {len(warnings)}; "
          f"informational search index notices: {len(search_notices)}; "
          f"search index: {search_index_bytes:,} bytes "
          f"(budget {SEARCH_INDEX_BUDGET_BYTES:,}).", flush=True)
    if stager.issues:
        print(f"Source link issues: {len(stager.issues)} (rewritten to GitHub; see {report.relative_to(REPO)}).")
        for issue in stager.issues:
            print(f"  {issue['file']}:{issue['line']}: {issue['target']} ({issue['reason']})")
    if broken:
        (SITE / "build/broken-links.txt").write_text("\n".join(broken) + "\n")
        print("Broken internal links:\n" + "\n".join(f"  {b}" for b in broken), file=sys.stderr)
    if page_broken:
        print("Page links unresolvable on GitHub:\n" + "\n".join(f"  {b}" for b in page_broken), file=sys.stderr)
    if search_index_bytes > SEARCH_INDEX_BUDGET_BYTES:
        print(f"Search index exceeds budget: {search_index_bytes:,} > "
              f"{SEARCH_INDEX_BUDGET_BYTES:,} bytes.", file=sys.stderr)
    if broken or page_broken or warnings or search_index_bytes > SEARCH_INDEX_BUDGET_BYTES:
        return 1
    if args.serve:
        return subprocess.call([mdbook, "serve", str(SITE)])
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (ValueError, OSError) as error:
        print(f"site build failed: {error}", file=sys.stderr)
        sys.exit(1)
