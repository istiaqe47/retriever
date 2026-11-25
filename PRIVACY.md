# Privacy policy: Retriever and its browser extension

**Short version: everything stays on your computer.** Retriever and its extension have no
servers, no accounts, no analytics, no ads and no tracking. Nothing is sent to the Retriever
developers or to anyone else.

## What the extension handles, and where it goes

To hand a download from your browser to the Retriever app **on the same computer**, the
extension passes it:

| Data | Why | Where it goes |
|---|---|---|
| The download's address, the page it came from, and its file name | So Retriever can download it | The Retriever app on your computer |
| The cookies and headers the browser would send for that one download | So downloads that need you to be signed in still work | The Retriever app on your computer |
| A page's links (only when you use "Download all links" or "Download selected links") | So you can pick which to download | The Retriever app on your computer |
| Whether a video plays on the page you're on, and its address (for the Download button) | So the button can offer the video's qualities | The Retriever app on your computer |

The extension reaches the app through the browser's **native messaging** (a helper program the
app installs), or, for browsers that can't start that helper, through a connection to
`127.0.0.1` (your own computer) that you allow once in Retriever.

Retriever then downloads from the site the address points to, as your browser would have.
Videos are looked up with yt-dlp, which talks to that video's site.

## What is stored

- **In the browser** (the extension's storage): your extension settings, the sites you chose
  to leave to the browser or hide the button on, and the key that connects this browser to
  Retriever.
- **In Retriever** (`~/.local/share/retriever/` and `~/.config/retriever/`, readable only by
  you): your download list, including the address and cookies of each download so it can
  resume, and your settings. Removing a download from the list deletes its saved data.

## What is never done

- No data is sent to the developers, sold, or shared with anyone.
- No browsing history is kept: the extension only acts on downloads, links and videos you hand
  to Retriever or that its "take downloads" setting catches, and on the site's name when you
  point at a video (so Retriever can say whether its button shows there). Retriever doesn't
  keep a list of the sites you visit.
- Nothing leaves your computer except the downloads you start, which go to the sites they
  come from.

## Contact

Questions: open an issue at <https://github.com/istiaqe47/retriever/issues>.

*Last updated: 24 November 2026.*
