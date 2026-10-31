# signum

Minimal egui app to stamp an SVG signature and a date (+ optional place) onto PDFs.

## Decisions

- **Rendering pages:** `hayro` (pure Rust) -> egui textures.
- **Writing:** `lopdf`, incremental update (append-only, original bytes untouched).
- **Stamps are SVGs.** Signature = user SVG; date stamp = generated SVG
  (`2026-10-31` or `Place, 2026-10-31`) with a bundled font.
  - Preview: `resvg` -> texture.
  - Output: `svg2pdf` -> Form XObject, drawn via an appended page content stream.
- **Coordinates:** stamps stored in PDF user space (points) per page; UI maps
  to/from screen, handling CropBox offset and `/Rotate`.
- **Persistence:** eframe `persistence` (signature SVG text, place).
- **Date:** `jiff`, local date, ISO format.
- **Platforms:** Linux, Windows, macOS.

## UX

- Open: CLI arg (`signum file.pdf`, i.e. "Open with") or drop a `.pdf`.
- Set signature: drop an `.svg`.
- Pages in one vertical scroll, fit to width.
- Toolbar: `Signature`, `Date` (stamp follows cursor, click to place), place text field.
- Any number of stamps. Drag to move; hover + wheel / Up/Down to resize;
  hover + Del or right-click to remove.
- Ctrl+S saves in place; "Save copy" writes `name.signed.pdf`.
