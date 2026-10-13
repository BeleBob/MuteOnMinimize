# Changelog

## Unreleased

- The settings page no longer overflows sideways in English: the background option has a short
  checkbox label and a wrapped explanation below it.

## 0.1.0 — 2026-10-13

First release.

- Selected applications are muted while all their windows are minimized and, by default, while
  another window is in front of them on their screen (Alt+Tab). A window on another monitor does
  not count. Muting fades out over a second; sound comes back at once.
- Applications are selected on the settings page, from the window menu (Alt+F3) or with
  Meta+Ctrl+M on the active window.
- Works with PulseAudio and native PipeWire streams, audio from child processes (Electron),
  Wine/Proton games and Flatpak apps.
- The user's own mute is never touched and nothing ends up in WirePlumber's saved state.
  Sound comes back when the service stops, KWin exits or the application closes.
- English, Russian, Ukrainian and German.
- `install.sh` for a per-user install, a Makefile, and AUR packages `muteonminimize` and
  `muteonminimize-git`.
