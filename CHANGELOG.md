# Changelog

## Unreleased

### Changed
- Starts in well under a second instead of several: signum now draws with OpenGL
  instead of wgpu, which no longer wakes the discrete GPU on hybrid-graphics laptops.

## 0.1.0 — 2026-10-31

### Added
- Debian package via `cargo deb`, with install instructions in the README.
- App icon: shown in the window, taskbar and dock, and embedded in the Windows .exe.
  A Linux .desktop file registers signum for opening PDFs.
- X and ✔ buttons place a cross or a checkmark, e.g. for ticking boxes.
- Free-text stamp: type into the Text field and click 🗛 to place it.

### Changed
- Toolbar buttons are more compact: "Open…" is now 🗁, "Signature" is now "Sign",
  and "Set signature…" is now a ⚙ joined to it.
- The Place field sits directly left of Date and is joined to it; it is half as wide.
- On Linux, signum runs through XWayland when available, so dropping files on the
  window works under Wayland sessions.
