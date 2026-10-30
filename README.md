# MacView

A fast, native macOS image viewer in the spirit of IrfanView: one window, the
image fills it, keyboard-driven browsing through a folder, and the handful of
edits you actually do on a photo or screenshot (rotate, flip, resize, crop,
save as). Pure Rust on AppKit via [`objc2`](https://github.com/madsmtm/objc2);
no web view, no cross-platform toolkit. Apple Silicon only.

The name is the obvious one: IrfanView's idea, on a Mac.

## Features

- Opens anything macOS ImageIO can decode: JPEG, PNG, HEIC, TIFF, GIF, WebP,
  BMP, PSD, AVIF, camera RAW, …
- Folder browsing: Space / Backspace, ← →, ↑ ↓, Home / End step through the
  images in the file's folder (sorted by name). Drag a file or folder onto the
  window, or use Open With… from Finder.
- Zoom: `+` / `-`, ⌘-scroll or pinch to zoom at the cursor, scroll to pan,
  ⌥-drag to pan. ⌘0 fit to window, ⌘1 actual size, `1` (or double-click)
  toggles between the two. Fit never enlarges.
- `F` full screen (Esc leaves it).
- `R` / `L` rotate right/left, ⌘⇧L rotate 180°, `H` / `V` flip.
- ⌘R Resize/Resample: pixels or percent, long-side presets, keep aspect
  ratio, four resampling qualities.
- Drag a selection, ⌘Y crops to it. Esc clears it. ⌘C copies the selection
  (or the whole image); ⌘V pastes the clipboard as a new image.
- Canvas Size (add a border or trim), Grayscale, Negative, Auto Adjust Colors.
- ⌘Z undo (12 steps).
- ⌘S save in place (JPEG at quality 0.92), ⌘⇧S Save As with a format
  pop-up and a JPEG quality slider: JPEG, PNG, TIFF, GIF, BMP.
- `P` Properties: file, dimensions, dpi / print size, bit depth, colour
  space and the full EXIF dictionary.
- Delete moves the file to the Trash, F2 renames it, ⌘⌥R reveals it in
  Finder.
- Transparent images are drawn over a checkerboard (⌘⇧T toggles).

### Not in scope (on purpose)

Batch conversion/rename, slideshows, screen capture, scanning, printing,
e-mail, audio/video, paint tools, watermarks, effects/plug-in filters,
IPTC editing, multipage TIFF, icon extraction, a thumbnail browser.

### Known limitations in 0.1

- Rotating a JPEG re-encodes it on save (no lossless JPEG transform yet).
- Save As cannot write HEIC or WebP (ImageIO's `CGImageDestination` isn't
  wired up yet); opening them works.
- Properties shows EXIF only; IPTC/XMP are not read.
- No app icon yet: drop a 1024×1024 `packaging/icon.png` in and the bundle
  script turns it into an `.icns`.

## Building

Requires Rust (stable) and Xcode command line tools on an Apple Silicon Mac.

```sh
cargo run --release -- ~/Pictures/some.jpg   # run straight from cargo
./packaging/bundle.sh                        # builds target/MacView.app
cp -r target/MacView.app /Applications/
```

The bundle is ad-hoc signed so it runs locally without a developer account.
To make it the default viewer: select an image in Finder, ⌘I, "Open with:"
MacView, "Change All…".

## Layout

| File | What |
| --- | --- |
| `src/main.rs` | NSApplication bootstrap |
| `src/app.rs` | App delegate: window, document state, every menu action |
| `src/view.rs` | `ImageView` (NSView subclass): drawing, zoom/pan, selection, drag-and-drop |
| `src/ops.rs` | Decode, rasterise, rotate/flip/crop/resize/canvas, colour ops, encode |
| `src/dialogs.rs` | Resize, Canvas, Save As, Rename and Properties UI |
| `src/menu.rs` | Menu bar and key equivalents |
| `src/files.rs` | Folder listing and file metadata |
| `packaging/` | `Info.plist` and the `.app` bundle script |

## Licence

MIT.
