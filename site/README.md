# Documentation site

The site is an [mdBook](https://rust-lang.github.io/mdBook/) built from the repository's own
documents. `docs/`, `ROADMAP.md`, `PLAN.md`, the examples' and tools' READMEs and the hardware
records in `docs/inventory/**/README.md` stay the single source: the build copies them, it never
edits them.

| Path | Contents |
|---|---|
| `pages/` | The pages written for the site: introduction, getting started, architecture overview, how it is verified, related work |
| `media/` | Screenshots and clips of the window, recorded from the running program |
| `build.py` | Stages the pages and the copied documents into `build/src`, writes `SUMMARY.md`, rewrites links to files that are not pages (source code, JSON records) to GitHub, runs mdBook into `book/`, then checks every internal link and anchor of the result |
| `book.toml` | mdBook settings |

## Build

Needs Python 3 and mdBook 0.5.4 on `PATH` (`brew install mdbook`, or the release binary
from GitHub).

```sh
python3 site/build.py            # site/book/index.html; fails on a broken link
python3 site/build.py --serve    # the same, served with live reload
```

Links to source files point at the `main` branch on GitHub; `--ref NAME` points them at another
branch or tag. The `Documentation site` workflow (`.github/workflows/site.yml`) builds the site on
every pull request; it is deployed to GitHub Pages only when the workflow is started by hand.
