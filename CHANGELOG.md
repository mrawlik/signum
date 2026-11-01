# Changelog

## 0.1.2 — 2026-11-01

### Added
- Prebuilt Debian package for Debian 12, Ubuntu 24.04 and newer (amd64). It is
  attached to each GitHub release and can be installed and updated with apt from
  https://apt.mrawlik.com.

## 0.1.1 — 2026-10-31

### Changed
- Starts in well under a second instead of several: signum now draws with OpenGL
  instead of wgpu, which no longer wakes the discrete GPU on hybrid-graphics laptops.

### Fixed
- KDE's launch animation stops as soon as the window appears, instead of bouncing
  for a few more seconds: the window now names its desktop file.

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
