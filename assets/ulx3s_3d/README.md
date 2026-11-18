# ULX3S v3.0.8 3D model

`ulx3s_v3.0.8.glb` is the built 3D ULX3S board model shipped through git LFS.
Size: 11,507,464 bytes. SHA-256:
`b1d25e5b8640c365e0d53d6773bd7dbd409b6aeb6c14e63c06285c94206f5c9d`.
Built 2026-11-17 at 08:43 local time. No build record exists. The newest
`tools/ulx3s_3d` commit before the build was `6988aa9` (03:36); `82ae4a3`
(SW1 levers) was committed at 08:46, three minutes after the build, so the
model was most likely exported from the working tree that became `82ae4a3`.

Rebuild from the repository root with the commands in
[tools/ulx3s_3d](../../tools/ulx3s_3d/README.md#build): extract the KiCad board,
plot its layers, build the Blender scene, then export the GLB. This needs
KiCad 10, Blender 5.1, `rsvg-convert`, Pillow and numpy; output goes to
`out/ulx3s_3d/ulx3s_v3.0.8.glb`.
