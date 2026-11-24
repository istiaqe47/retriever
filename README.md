# Retriever

A download manager for GNOME (GTK 4 + libadwaita): files with many connections at once,
videos, torrents, link grabbing, archive extraction, a scheduler and browser integration. It
follows your system's light/dark style, accent and icon theme.

## Install on Arch / CachyOS

```sh
tar xf retriever-1.0.0.tar.gz
cd retriever-1.0.0
makepkg -si
```

Then open **Retriever** from the app grid, or run `retriever`. `retriever <link>` opens Add URL
with that link; `retriever file.torrent` or a magnet link opens Add Torrent. Retriever is offered
for magnet links and .torrent files in Files and the browsers.

Optional: `sudo pacman -S deno` (all of YouTube's qualities), `sudo pacman -S 7zip` (7z and RAR).

## Install on Debian / Ubuntu

Ubuntu 24.04 or newer, Debian 13 or newer, and the distributions built on them (Linux Mint 22,
Pop!_OS 24.04, Zorin OS 18…): download `retriever_1.0.0-1_amd64.deb`, then

```sh
sudo apt install ./retriever_1.0.0-1_amd64.deb
```

To build the .deb yourself, see `packaging/debian/build-deb.sh`.

## What it does

- **Files**: many connections per file, HTTP, HTTPS, FTP, FTPS; pause and resume, also after a
  crash, a restart or closing the app; lost connections reconnect, failures that pass are
  retried by themselves.
- **Videos** from YouTube and the other sites yt-dlp knows, or any page that plays one: + ▾ →
  Download Video, pick a quality.
- **Torrents and magnet links**: + ▾ → Add Torrent, choose the files; sharing after they finish
  is a setting.
- **Grab links** from a page: + ▾ → Grab Links, filter by type or name, download the ticked.
- **Archives** extract by themselves when done (zip, tar, gz, xz; 7z and RAR with 7-Zip), or
  from the right-click menu.
- **Scheduler** (main menu → Scheduler): scheduled downloads start and stop at set times every
  day; afterwards nothing, quit, suspend or shut down (with a minute to call it off). The
  computer is kept from suspending while they wait.
- **Speed limits**: for all downloads (Preferences) or one (its progress window).
- **Remove Finished Downloads** (Preferences): finished downloads leave the list as soon as
  they finish (switching it on clears the finished ones at once); their files stay where they
  were saved.
- **Browsers**: with Retriever's extension, downloads, a page's links,
  videos and magnet links come here with the browser's cookies.
- Categories, a queue, search, notifications, no suspend while downloading.
- Each download is a card with its own pause / resume (or show) button and a ⋮ menu (also
  right click, a long press, or the Menu key); the header's Select picks several at once, acted
  on from the bar at the bottom.

## Browser downloads

Retriever comes with its browser extension, installed in `/usr/share/retriever/extension/`.

**Chrome, Chromium, Brave, Vivaldi, Edge, Opera**

1. Open `chrome://extensions` (Brave: `brave://extensions`).
2. Turn on **Developer mode** (top right).
3. **Load unpacked**, and choose `/usr/share/retriever/extension/chromium`.

**Firefox, Zen, Floorp** (Firefox installs only add-ons Mozilla signed)

1. Make a free account at addons.mozilla.org and get an API key at
   https://addons.mozilla.org/developers/addon/api/key/ (a JWT issuer and a secret).
2. `sudo pacman -S nodejs npm`, then
   `sh /usr/share/retriever/extension/sign-firefox.sh <JWT issuer> <JWT secret>`.
3. Open the signed `.xpi` it saves in `~/Downloads` with Firefox. It is yours only (unlisted),
   and Firefox keeps it updated no further: sign again after an update of Retriever.

LibreWolf installs it unsigned: set `xpinstall.signatures.required` to false in `about:config`,
then open `/usr/share/retriever/extension/retriever-firefox.xpi`. To try it in any Firefox
without signing, `about:debugging` → This Firefox → Load Temporary Add-on (until Firefox closes).

Downloads, a page's links, videos and magnet links then come to Retriever with the browser's
cookies; the toolbar button lists your downloads. Preferences → Browsers turns catching off.
Browsers that can't start Retriever's helper (Zen and other Firefox-based browsers that keep
their own folder, Flatpak and Snap browsers) connect to Retriever directly while it's open, on
127.0.0.1: the first time, Retriever asks whether to allow the browser. Preferences → Browsers →
Connected Browsers forgets them; Connect Other Browsers turns this off.

## Keys

| Keys | Does |
|---|---|
| Ctrl+N | Add URL (a link on the clipboard is filled in) |
| Ctrl+R / Ctrl+P | Resume / pause the selected downloads |
| Delete (in the list) | Delete the focused or selected downloads |
| Menu or Shift+F10 | The focused download's menu |
| Ctrl+A (in the list) | Select every download |
| Ctrl+? | Keyboard shortcuts |
| Ctrl+F or typing | Search |
| Ctrl+, | Preferences |
| Ctrl+Q | Quit (downloads pause, and resume next time) |

## Files

- The list: `~/.local/share/retriever/downloads.json` (readable by you only: it keeps browser
  cookies)
- Settings: `~/.config/retriever/settings.json`
- Torrents: `~/.local/share/retriever/torrents/`
- The browser connection: `~/.local/share/retriever/browser/`

## License

GPL-3.0-or-later. Based on Mister Download Manager (GPL-3.0); its copyright notices are kept
in the source.
