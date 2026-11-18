# Release media provenance

`hero-life-full.webp` and `oled-bricks.webp` were approved on **2026-11-18**.

All files below were captured on **2026-11-18** from a release build of
**ECP5 Emu.app 0.1.0** from dev commit **`0d53bef`**, built in a neutral folder with
path remapping. The same build was packaged as an app and ZIP.
The ZIP of that build has SHA-256:

`733154caa081dd83977ab42195341bc5cf5b9c26ac01f3e11148b22c68a38906`

Every selected take records the same GUI executable SHA-256:

`dd5e8a5efa2893105aa4378a53c97f58714f9856cb56e9463ac42f008b846886`

Take folders live under `out/site_media/` and are not tracked in Git. This table is
the durable mapping; other takes, including the Life files in the first folder,
were superseded and were not used. Every still was reviewed for paths and personal
names; none were visible.

Capture environment for every command: `ECP5_MEDIA_APP` was that release build at its
fixed package path and `ECP5_MEDIA_NEUTRAL=/private/tmp/ecp5-media/final`. Commands
below name the kit shot corresponding to each take. Capture window bounds are
logical points, including the titlebar; full-window stills retain that aspect.

| File | Take folder | Capture date | App commit | Kit command |
|---|---|---|---|---|
| [hero-life-full.webp](hero-life-full.webp) | `20261118-103159-final-ty6f63tt` | 2026-11-18 | `0d53bef` | `tools/site_media/run.sh life` |
| [window-life-full.mp4](window-life-full.mp4) | `20261118-103159-final-ty6f63tt` | 2026-11-18 | `0d53bef` | `tools/site_media/run.sh life` |
| [oled-bricks.webp](oled-bricks.webp) | `20261118-102257-final-qpk95g1x` | 2026-11-18 | `0d53bef` | `tools/site_media/run.sh oled` |
| [oled-bricks.mp4](oled-bricks.mp4) | `20261118-102257-final-qpk95g1x` | 2026-11-18 | `0d53bef` | `tools/site_media/run.sh oled` |
| [window-cross-probe.webp](window-cross-probe.webp) | `20261118-102257-final-qpk95g1x` | 2026-11-18 | `0d53bef` | `tools/site_media/run.sh cross-probe` |
| [window-cross-probe.mp4](window-cross-probe.mp4) | `20261118-102257-final-qpk95g1x` | 2026-11-18 | `0d53bef` | `tools/site_media/run.sh cross-probe` |
| [window-start.webp](window-start.webp) | `20261118-102257-final-qpk95g1x` | 2026-11-18 | `0d53bef` | `tools/site_media/run.sh start` |
| [window-new-project.webp](window-new-project.webp) | `20261118-105037-final-lyz8tljk` | 2026-11-18 | `0d53bef` | `tools/site_media/run.sh new-project` |
| [window-project.webp](window-project.webp) | `20261118-104244-final-c_uqo3_p` | 2026-11-18 | `0d53bef` | `tools/site_media/run.sh project` |
| [window-project-build.mp4](window-project-build.mp4) | `20261118-104244-final-c_uqo3_p` | 2026-11-18 | `0d53bef` | `tools/site_media/run.sh project` |
| [window-critical-path.webp](window-critical-path.webp) | `20261118-104321-final-yn14buhl` | 2026-11-18 | `0d53bef` | `tools/site_media/run.sh critical` |
| [window-waves.webp](window-waves.webp) | `20261118-104337-final-bkgokjev` | 2026-11-18 | `0d53bef` | `tools/site_media/run.sh waves` |
| [window-waves.mp4](window-waves.mp4) | `20261118-104337-final-bkgokjev` | 2026-11-18 | `0d53bef` | `tools/site_media/run.sh waves` |
| [window-riscv-terminal.webp](window-riscv-terminal.webp) | `20261118-104408-final-qnm21_1j` | 2026-11-18 | `0d53bef` | `tools/site_media/run.sh terminal` |
| [window-riscv-terminal.mp4](window-riscv-terminal.mp4) | `20261118-104408-final-qnm21_1j` | 2026-11-18 | `0d53bef` | `tools/site_media/run.sh terminal` |
| [window-life-full-zoom.webp](window-life-full-zoom.webp) | `20261118-104531-final-lscxjttq` | 2026-11-18 | `0d53bef` | `tools/site_media/run.sh life-zoom` |
| [board-closeup.webp](board-closeup.webp) | `20261118-104549-final-y2627qcr` | 2026-11-18 | `0d53bef` | `tools/site_media/run.sh board` |
| [window-tour.mp4](window-tour.mp4) | `20261118-104607-final-2gu6bkhd` | 2026-11-18 | `0d53bef` | `tools/site_media/run.sh tour` |

## Offline encoding and crop audit

Clips were re-encoded directly from each selected `SHOT.raw.mov` by the kit’s
`encode.clip`, using **H.264/libx264, CRF 28, preset veryslow, 12 fps, two encoder
threads, 1100 px width, proportional even height, yuv420p, no audio, faststart**.
The leading trim is **0.25 s**; durations are 30 s Life, 18 s OLED, 15 s cross-probe,
28 s build, 18 s Waves, 60 s terminal and 43 s walkthrough. No video is cropped.
These settings are recorded in `tools/site_media/batch.json` and used by future passes.

All existing replacement clips are below **1.5×** their previous `HEAD:site/media/NAME`
size; the new OLED clip is in the same few-hundred-KiB class. Sampled encoded frames
were inspected against scaled raw frames for text clarity. Every still is ≤250 KiB.

Cross-probe and start preceded the no-implicit-crop fix. Their aspect audit is:

| Shot | Take rect (x,y,w,h) | Raw PNG | Encoded still | Result |
|---|---|---|---|---|
| Cross-probe | 30,80,1500,898 | 3000×1796 | 1500×898 | Exact 2:1 scale; no crop |
| Start | 30,80,1500,978 | 3000×1956 | 1500×978 | Exact 2:1 scale; no crop |

Those two stills were copied unchanged; no corrective re-encoding was needed.
Other stills were also copied unchanged from the selected takes. Map detail and
board closeup use their intentionally declared layout crops and padding. Hero
comes only from `20261118-103159-final-ty6f63tt` at 1680×960, never the earlier
OS-clamped take. The New project still leaves Advanced collapsed, as captured.

Reproduce the preparation without opening a window:

```sh
python3 tools/site_media/publish.py --check
python3 tools/site_media/publish.py
```

The source mapping and profiles are in `tools/site_media/publication.json`.
The interactive tour requires gestures; `window-tour.mp4` is a scripted project
walkthrough, not a recording of the interactive Next/Show me controls.

## Prepared file hashes and sizes

| File | Previous HEAD bytes | Prepared bytes | SHA-256 |
|---|---:|---:|---|
| `hero-life-full.webp` | 194904 | 130298 | `df8e4777990b903788f3bd6c17e21fb7cd36cda51255c9591bd2d029ca573dcc` |
| `window-life-full.mp4` | 1542829 | 2018854 | `ff08d7a1fba3d14145bdea8373946bfbee3d6ebe0f83bb1adc7b79170ba18000` |
| `oled-bricks.webp` | new | 95334 | `9e98183949dabe29b70ba63d13c5db98697057994de1edbb8e3e7ffa967e9999` |
| `oled-bricks.mp4` | new | 298425 | `b85f8227f94d95e0f472d3d04f97045a06d631e335aa8dd77ea5c48467a9aa04` |
| `window-cross-probe.webp` | 69362 | 66280 | `60d315fe206396acb76158fa9dd5bcba4f6d949cdd046de2b3e221d81db6f2a6` |
| `window-cross-probe.mp4` | 193899 | 144825 | `53a68733c051d1ba99fd895af09593fa6f7a667cd726fdffdac47a21bd1ef43e` |
| `window-start.webp` | 39116 | 29166 | `6e139d25ae4389c96cb4e6051694927e22ef5843df59e7a088e93acc6e1929ed` |
| `window-new-project.webp` | new | 26776 | `2100eccbdd92f696ba0d11e96b04448b2f52f7db8b2bf755f3ef1f85a0b44cbb` |
| `window-project.webp` | 98676 | 103188 | `e58d889897fd1da619e97a043793d0fe01dd1404cb8fa11d6417262fd7b581f4` |
| `window-project-build.mp4` | 239649 | 230475 | `d0f4f4a2e29073a379d6329147f813b2deafede6e2051cb037d04378ce9d3acb` |
| `window-critical-path.webp` | 88796 | 79412 | `860545699d5c043a6d09e0ae2105976415b8a3f503d7633fb5b061d87e8d42f5` |
| `window-waves.webp` | 74764 | 69074 | `bb7eb3812ab94fd303539475c92244bee38adf7b29e45a10e67818bce10116aa` |
| `window-waves.mp4` | 214718 | 193271 | `edbbecf3b6dbbb0dd1b23c9cab347bf05c8b16b20d2772f2d62c1db048938c58` |
| `window-riscv-terminal.webp` | 142160 | 137110 | `c51e6fcd57096d5c4bb1b92f3c4b1040754dfc07cbd38c4389bfa553a981bc05` |
| `window-riscv-terminal.mp4` | 654590 | 546116 | `9bdc611c66f83382e76c1096329eb63bbd42395f02119277e427b6b7b7cda092` |
| `window-life-full-zoom.webp` | 2247798 | 60388 | `7043463ad1053db88441f6e385852a118ada1dcc3411a65f371b4985d53ca22e` |
| `board-closeup.webp` | 83838 | 54076 | `86fba12e2cdbfc8035908bc92cf99b315a40d79ab36b0ed575f43522ff6d5933` |
| `window-tour.mp4` | 435749 | 353306 | `0b089dbaf85533766cd640bc5ca56da342e20b350d5bd99d4a95e8a8487e4b55` |
