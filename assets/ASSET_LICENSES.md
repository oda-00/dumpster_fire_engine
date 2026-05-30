# Bundled asset licenses

Third-party open-source assets vendored to flesh out the editor and GUI. Each is
under a permissive / open license; see the source repositories for authoritative
per-asset license text.

## 3D models — `assets/models/*.glb`
From the **Khronos glTF Sample Assets** repository
(<https://github.com/KhronosGroup/glTF-Sample-Assets>):

| File | License | Author / source |
|---|---|---|
| `Box.glb` | Public domain (CC0) | Khronos |
| `BoxTextured.glb` | Public domain (CC0) | Khronos |
| `BoxVertexColors.glb` | Public domain (CC0) | Khronos |
| `Duck.glb` | CC-BY 4.0 | Sony (via Khronos sample set) |
| `Fox.glb` | Model CC0 1.0 (PixelMannen); rig/animation CC-BY 4.0 (@tomkranis) |

(`BrainStem.glb` and other pre-existing models retain their original Khronos
sample-asset licenses.)

## 2D icons — `assets/icons/lucide/*.svg`
From **Lucide** (<https://github.com/lucide-icons/lucide>), **ISC License**.
A curated editor toolbar set: box, move, rotate-3d, scale-3d, grip, grid-3x3,
eye, eye-off, trash-2, copy, undo-2, redo-2, plus, minus, mouse-pointer-2,
pen-tool, layers, settings, axis-3d, ruler.

## Notes
- glTF binaries were validated on download (sfnt/`glTF` magic) to avoid the
  HTML-masquerading-as-asset problem.
- Lucide icons are vector (SVG); rasterizing them into the UI glyph/icon atlas
  (`src/resource_manager/ui_manager/atlas.rs`) is a follow-up.
