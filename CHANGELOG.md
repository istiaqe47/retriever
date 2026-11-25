# Changelog

## 1.0.1 (25 November 2026)

**New**
- Works with Mister Download Manager's browser extension too (Chrome Web Store, Firefox
  add-on or loaded by hand), unless Mister Download Manager itself is installed.

**GNOME look**
- A download's window uses GNOME's own progress bar.
- Dimmed text uses libadwaita 1.7's style where available.

**Fixes**
- yt-dlp and ffmpeg installed with pipx (`~/.local/bin`) are found, also when Retriever is
  opened from the app grid.
- Install hints give the apt command on Ubuntu and Debian, pacman's on Arch.
- Downloads set to start at the scheduled time say so when the scheduler is off, instead of
  never starting.
- The browser helper no longer offers Mister Download Manager's Chrome Web Store extension.
- About and the software centre link to the project page and its issues.

**Firefox extension 1.0.3**: submitted to addons.mozilla.org. It declares what it hands to the
app, and `sign-firefox.sh` can be run again after every update.

## 1.0.0 (24 November 2026)

First release: a GNOME (GTK 4 + libadwaita) download manager on Mister Download Manager's
engine, for Arch/CachyOS and Ubuntu/Debian.

- Multi-connection downloads that resume after anything, with automatic retries.
- Videos (yt-dlp), torrents and magnet links, Grab Links From a Page, archive extraction,
  scheduler, speed limits, Remove Finished Downloads, select mode with Select All.
- Retriever's own browser extension for Firefox and Chromium browsers, GNOME-styled, with a
  Download button over videos; Zen, Flatpak and Snap browsers connect directly while Retriever
  is open, after one click on **Allow**.
