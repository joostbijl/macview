<p align="center">
  <img src="packaging/icon.png" width="128" alt="MacView">
</p>

# MacView

**A fast, native macOS image viewer, in the spirit of IrfanView.**

One window. The image fills it. Space steps to the next file in the folder,
F switches between fit and 1:1, Enter goes full screen.
A dozen keyboard shortcuts do everything you actually do to a photo or
screenshot: rotate, flip, resize, crop, save as. Nothing else.

Pure Rust on AppKit (via [`objc2`](https://github.com/madsmtm/objc2)). No
web view, no cross-platform toolkit, no Electron. The binary is a few MB
and opens in well under a second. Apple Silicon, macOS 13 or newer.

> IrfanView has a beaver. So does MacView.

## Install

You need Rust (stable) and the Xcode command line tools (`xcode-select --install`).

```sh
git clone https://github.com/joostbijl/macview
cd macview
./packaging/bundle.sh          # cargo build --release + assemble target/MacView.app
cp -r target/MacView.app /Applications/
```

The bundle is ad-hoc signed, so it runs locally without an Apple developer
account. First launch: right-click an image in Finder → Open With → MacView.
To make it the default viewer: select an image, ⌘I, "Open with:" MacView,
"Change All…".

To run without bundling: `cargo run --release -- ~/Pictures/photo.jpg`.

## Using it

Open an image any way you like: Open With from Finder, drag a file or a
folder onto the window, ⌘O, or `macview photo.jpg` from a shell. The
window title shows the file name, its position in the folder, pixel size,
zoom and whether it has unsaved edits.

### Keyboard

| Key | Action |
| --- | --- |
| **Space** / **Backspace**, → / ←, ↓ / ↑ | Next / previous image in the folder (sorted by name, wraps around) |
| **Home** / **End** | First / last image |
| **F** | Next zoom mode: fit to window → actual size (1:1) → fit to width → … |
| **1** or double-click | Toggle fit-to-window ↔ actual size |
| **Enter** | Full screen on/off (plain IrfanView-style: no animation, no Space switch; Esc also leaves it) |
| **+** / **−** | Zoom in / out |
| ⌘0 / ⌘1 / ⌘2 | Fit to window / actual size (1:1) / fit to width |
| ⌘-scroll, pinch | Zoom at the cursor |
| scroll, ⌥-drag | Pan a zoomed image |
| **R** / **L** | Rotate 90° right / left |
| ⌘⇧L | Rotate 180° |
| **H** / **V** | Flip horizontal / vertical |
| ⌘R | Resize / Resample… |
| ⌘⌥⇧R | Canvas Size… (add a border or trim) |
| drag | Select a rectangle |
| ⌘Y | Crop to selection |
| Esc | Clear selection |
| ⌘⇧G / ⌘⇧N / ⌘⇧U | Grayscale / Negative / Auto adjust colours |
| ⌘Z | Undo (12 steps) |
| ⌘S | Save (same file, same format; JPEG at quality 92) |
| ⌘⇧S | Save As… (JPEG with quality slider, PNG, TIFF, GIF, BMP) |
| ⌘C | Copy image (or the selection) to the clipboard |
| ⌘V | Paste clipboard as a new image |
| **P** | Properties (Esc closes it) |
| **Delete** | Move file to Trash |
| **F2** | Rename file |
| ⌘⌥R | Reveal in Finder |
| ⌘⇧R | Reload from disk |
| ⌘⇧T | Checkerboard behind transparent images on/off |

Every shortcut is also in the menu bar, so you can look them up there.

### Properties (P)

File name, folder, size, modification time, pixel dimensions, megapixels,
aspect ratio, current zoom, dpi and print size in cm, bit depth, colour
space, and the complete EXIF dictionary as macOS reads it (camera, lens,
exposure, ISO, date, GPS, …). Shown in a separate resizable window.

### Resize / Resample (⌘R)

Enter a width and/or height in pixels or percent — with "keep aspect
ratio" on, the field you changed drives the other — or pick a preset
(long side 640/1024/1600/2048/4096, half, double). Four resampling
qualities; "High" is what you want for photos, "Nearest neighbour" for
pixel art and screenshots you're enlarging.

### Formats

Opens anything macOS ImageIO decodes: JPEG, PNG, HEIC/HEIF, TIFF, GIF,
WebP, BMP, PSD, AVIF, ICO/ICNS, PDF (first page), OpenEXR, PBM/PGM/PPM
and camera RAW (CR2, CR3, NEF, ARW, DNG, RAF, ORF, RW2, …).

Saves JPEG, PNG, TIFF, GIF and BMP.

## What it is not

Deliberately out of scope, now and later: batch conversion and renaming,
command-line processing, slideshows, screen capture, scanning, printing,
e-mail, audio/video, paint tools, watermarks, effects and plug-in filters,
IPTC editing, multipage TIFF, icon extraction. Use IrfanView on Windows,
or a real editor, for those.

## Known limitations (0.1)

- Rotating a JPEG re-encodes it on save; there is no lossless JPEG
  transform yet.
- Save As can't write HEIC or WebP (opening them works).
- Properties shows EXIF only, not IPTC or XMP.
- The folder list is read once when a file is opened; files added to the
  folder afterwards show up after Reload (⌘⇧R) or reopening.

## Code layout

| File | Contents |
| --- | --- |
| `src/main.rs` | NSApplication bootstrap, argv handling |
| `src/app.rs` | App delegate: window, document state (folder list, undo), every menu action |
| `src/view.rs` | `ImageView`, an NSView subclass: drawing, zoom/pan, selection, drag-and-drop |
| `src/ops.rs` | Decode, rasterise, rotate/flip/crop/resize/canvas, colour ops, encode |
| `src/dialogs.rs` | Resize, Canvas Size, Save As, Rename and Properties UI |
| `src/menu.rs` | Menu bar and key equivalents |
| `src/files.rs` | Folder listing and file metadata |
| `packaging/` | `Info.plist`, the app icon and the `.app` bundle script |

Pixel operations work on a plain 8-bit RGBA `Vec<u8>`; AppKit is only used
to decode, resample (`NSImage` drawing with a chosen interpolation) and
encode (`NSBitmapImageRep`). Objective-C classes are declared with
`objc2::define_class!`; two things that bite there are documented in the
code: an explicit `return` inside an ObjC method skips the macro's
`bool → BOOL` conversion, and ObjC action methods can't be called as Rust
functions.

## Licence

MIT. The beaver is MacView's own.
