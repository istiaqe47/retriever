//! What the GTK interface keeps on disk (the download list and its settings), the categories
//! and the words and numbers the list shows.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const APP_ID: &str = "io.github.retriever.Retriever";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Queued,
    Downloading,
    Paused,
    Done,
    Failed,
}

/// What a download is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    #[default]
    File,
    /// From a web page, by yt-dlp.
    Video,
    Torrent,
}

/// A video's download as chosen in Download Video.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct VideoSpec {
    /// What yt-dlp downloads: the page, or the video the page declares.
    pub address: String,
    /// yt-dlp's `-f`.
    pub format: String,
    /// The file's name without its extension (yt-dlp adds it).
    pub stem: String,
    #[serde(default)]
    pub expected: Option<u64>,
}

/// A torrent as added: its hash, its .torrent (kept to add it again), the files chosen.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TorrentSpec {
    pub hash: String,
    pub torrent: PathBuf,
    #[serde(default)]
    pub files: Option<Vec<usize>>,
}

/// One download of the list, as it is saved.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    pub id: u64,
    pub url: String,
    /// The folder it saves into.
    pub folder: PathBuf,
    /// Its file, once the server has named it (resumes go there).
    #[serde(default)]
    pub file: Option<PathBuf>,
    pub name: String,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub downloaded: u64,
    pub status: Status,
    /// Unix seconds.
    pub added: i64,
    #[serde(default)]
    pub finished: Option<i64>,
    #[serde(default)]
    pub error: Option<String>,
    pub connections: u32,
    #[serde(default)]
    pub resumable: Option<bool>,
    /// Sent with every request: what the browser had for it (Referer, Cookie, User-Agent).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<(String, String)>,
    /// `name` came with the link (the browser's), and is the file's name.
    #[serde(default)]
    pub name_given: bool,
    #[serde(default)]
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<VideoSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub torrent: Option<TorrentSpec>,
    /// Its own speed limit, KB/s; 0: none (the one for all downloads still holds).
    #[serde(default)]
    pub limit_kib: u64,
    /// Waits for the scheduler's start time, and stops at its stop time.
    #[serde(default)]
    pub scheduled: bool,
    /// What extracting it did ("Extracted to …", or why not).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extracted: Option<String>,
}

impl Item {
    pub fn category(&self) -> Category {
        Category::of(&self.name)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub folder: PathBuf,
    pub connections: u32,
    pub simultaneous: u32,
    /// KiB/s for all downloads together; 0 = unlimited.
    pub speed_limit: u64,
    /// 0 follows the system, 1 light, 2 dark.
    pub style: u32,
    pub notify: bool,
    pub system_proxy: bool,
    /// Downloads from the browsers (Retriever's extension) come here.
    pub browser_capture: bool,
    /// Browsers without the native connection (Zen, Flatpak browsers…) reach Retriever on
    /// 127.0.0.1 while it is open, once allowed (localdoor.rs).
    pub local_door: bool,
    /// The extension's Download button over videos in the browsers (with yt-dlp installed).
    pub video_button: bool,
    /// That button as its icon only, over every video.
    pub video_button_small: bool,
    /// The sites it stays off (space-separated; a name takes its subdomains, `*` matches).
    pub video_button_off: String,
    /// Sites whose downloads stay with the browser (space-separated, as above).
    pub browser_sites: String,
    /// An archive (zip, tar, gz, xz; 7z and RAR with 7-Zip) is extracted beside it when done.
    pub extract: bool,
    /// Finished torrents go on sharing; off, they stop when done.
    pub seed: bool,
    /// A finished download leaves the list at once (its file stays where it was saved).
    pub forget_finished: bool,
    pub schedule: Schedule,
    pub width: i32,
    pub height: i32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            folder: dirs::download_dir()
                .or_else(|| dirs::home_dir().map(|h| h.join("Downloads")))
                .unwrap_or_else(|| PathBuf::from(".")),
            connections: 8,
            simultaneous: 3,
            speed_limit: 0,
            style: 0,
            notify: true,
            system_proxy: true,
            browser_capture: true,
            local_door: true,
            video_button: true,
            video_button_small: false,
            video_button_off: String::new(),
            browser_sites: String::new(),
            extract: true,
            seed: false,
            forget_finished: false,
            schedule: Schedule::default(),
            width: 1000,
            height: 620,
        }
    }
}

/// The scheduler: scheduled downloads start at `start` and stop at `stop`, every day; when the
/// ones it started are done, `then`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Schedule {
    pub enabled: bool,
    /// Minutes after midnight.
    pub start: u32,
    pub stop_enabled: bool,
    pub stop: u32,
    pub then: After,
}

impl Default for Schedule {
    fn default() -> Self {
        Schedule {
            enabled: false,
            start: 2 * 60,
            stop_enabled: true,
            stop: 8 * 60,
            then: After::Nothing,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum After {
    #[default]
    Nothing,
    Quit,
    Suspend,
    PowerOff,
}

impl After {
    pub const ALL: [After; 4] = [After::Nothing, After::Quit, After::Suspend, After::PowerOff];
    pub fn label(self) -> &'static str {
        match self {
            After::Nothing => "Nothing",
            After::Quit => "Quit Retriever",
            After::Suspend => "Suspend the Computer",
            After::PowerOff => "Shut Down the Computer",
        }
    }
}

pub fn data_folder() -> PathBuf {
    data_dir()
}

/// The keys of the browsers that connect without the native connection (localdoor.rs).
pub fn browser_keys_path() -> PathBuf {
    config_dir().join("browser-keys.json")
}

fn config_dir() -> PathBuf {
    moved(dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")))
}

fn data_dir() -> PathBuf {
    moved(dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")))
}

/// `base`/retriever; the folder of its earlier names (Downpour, and `mdm-gtk` before it) is
/// moved there once.
fn moved(base: PathBuf) -> PathBuf {
    let dir = base.join("retriever");
    if !dir.exists()
        && let Some(old) = ["downpour", "mdm-gtk"]
            .iter()
            .map(|n| base.join(n))
            .find(|o| o.is_dir())
    {
        let _ = std::fs::rename(&old, &dir);
    }
    dir
}

fn settings_path() -> PathBuf {
    config_dir().join("settings.json")
}

fn list_path() -> PathBuf {
    data_dir().join("downloads.json")
}

/// Written beside, then renamed into place: a crash never leaves half a file.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    {
        // Readable by its owner only: the list keeps the cookies a browser gave a download.
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(tmp, path)
}

pub fn load_settings() -> Settings {
    std::fs::read(settings_path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_settings(s: &Settings) {
    if let Ok(b) = serde_json::to_vec_pretty(s)
        && let Err(e) = write_atomic(&settings_path(), &b)
    {
        eprintln!("retriever: settings not saved: {e}");
    }
}

pub fn load_list() -> Vec<Item> {
    let path = list_path();
    let Ok(bytes) = std::fs::read(&path) else {
        return Vec::new();
    };
    match serde_json::from_slice(&bytes) {
        Ok(list) => list,
        Err(e) => {
            // Kept aside, never overwritten by the empty list that follows.
            let aside = path.with_extension(format!("json.broken-{}", now()));
            eprintln!(
                "retriever: {} unreadable ({e}); kept as {}",
                path.display(),
                aside.display()
            );
            let _ = std::fs::rename(&path, &aside);
            Vec::new()
        }
    }
}

pub fn save_list(items: &[Item]) {
    if let Ok(b) = serde_json::to_vec_pretty(items)
        && let Err(e) = write_atomic(&list_path(), &b)
    {
        eprintln!("retriever: download list not saved: {e}");
    }
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The sidebar's entries: the list's views, then the file kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    All,
    Unfinished,
    Finished,
    Compressed,
    Documents,
    Music,
    Programs,
    Video,
    Other,
}

impl Category {
    pub const SIDEBAR: [Category; 8] = [
        Category::All,
        Category::Unfinished,
        Category::Finished,
        Category::Compressed,
        Category::Documents,
        Category::Music,
        Category::Programs,
        Category::Video,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Category::All => "All Downloads",
            Category::Unfinished => "Unfinished",
            Category::Finished => "Finished",
            Category::Compressed => "Compressed",
            Category::Documents => "Documents",
            Category::Music => "Music",
            Category::Programs => "Programs",
            Category::Video => "Video",
            Category::Other => "Other",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Category::All => "folder-download-symbolic",
            Category::Unfinished => "network-transmit-receive-symbolic",
            Category::Finished => "object-select-symbolic",
            Category::Compressed => "package-x-generic-symbolic",
            Category::Documents => "x-office-document-symbolic",
            Category::Music => "audio-x-generic-symbolic",
            Category::Programs => "application-x-executable-symbolic",
            Category::Video => "video-x-generic-symbolic",
            Category::Other => "text-x-generic-symbolic",
        }
    }

    /// A file's kind, by its extension.
    pub fn of(name: &str) -> Category {
        let lower = name.to_ascii_lowercase();
        let ext = lower.rsplit('.').next().unwrap_or("");
        // Two-part endings first (.tar.gz and the like end in their compressor's).
        match ext {
            "zip" | "rar" | "7z" | "tar" | "gz" | "tgz" | "xz" | "txz" | "bz2" | "tbz2" | "zst"
            | "lz" | "lzma" | "cab" | "arj" | "z" => Category::Compressed,
            "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "odt" | "ods" | "odp"
            | "txt" | "csv" | "md" | "epub" | "rtf" | "djvu" | "mobi" => Category::Documents,
            "mp3" | "flac" | "ogg" | "oga" | "opus" | "m4a" | "wav" | "aac" | "wma" | "aiff"
            | "mka" => Category::Music,
            "exe" | "msi" | "deb" | "rpm" | "appimage" | "pkg" | "dmg" | "iso" | "img" | "apk"
            | "flatpak" | "flatpakref" | "sh" | "run" | "bin" | "jar" | "snap" => {
                Category::Programs
            }
            "mp4" | "mkv" | "webm" | "avi" | "mov" | "m4v" | "flv" | "wmv" | "ts" | "mpg"
            | "mpeg" | "3gp" => Category::Video,
            _ => Category::Other,
        }
    }

    pub fn holds(self, item: &Item) -> bool {
        match self {
            Category::All => true,
            Category::Unfinished => item.status != Status::Done,
            Category::Finished => item.status == Status::Done,
            kind => item.category() == kind,
        }
    }
}

/// A file name for a link before its server has named it: the path's last part.
pub fn name_from_url(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| {
            u.path_segments()
                .and_then(|mut s| s.next_back().map(str::to_owned))
        })
        .filter(|s| !s.is_empty())
        .map(|s| percent_decode(&s))
        .unwrap_or_else(|| {
            url::Url::parse(url)
                .ok()
                .and_then(|u| u.host_str().map(str::to_owned))
                .unwrap_or_else(|| url.to_owned())
        })
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(v) = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn size_text(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1000.0 && u < UNITS.len() - 1 {
        v /= 1000.0;
        u += 1;
    }
    if u == 0 {
        format!("{bytes} B")
    } else if v >= 100.0 {
        format!("{v:.0} {}", UNITS[u])
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

pub fn speed_text(bps: f64) -> String {
    if bps < 1.0 {
        String::new()
    } else {
        format!("{}/s", size_text(bps as u64))
    }
}

pub fn eta_text(secs: f64) -> String {
    if !secs.is_finite() || secs <= 0.0 {
        return String::new();
    }
    let s = secs.round() as u64;
    let (h, m, sec) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 99 {
        "more than 4 days".into()
    } else if h > 0 {
        format!("{h} h {m} min")
    } else if m > 0 {
        format!("{m} min {sec} s")
    } else {
        format!("{sec} s")
    }
}

pub fn date_text(unix: i64) -> String {
    gtk::glib::DateTime::from_unix_local(unix)
        .ok()
        .and_then(|d| {
            let today = gtk::glib::DateTime::now_local().ok()?;
            let fmt = if d.ymd() == today.ymd() {
                "Today %H:%M"
            } else if d.year() == today.year() {
                "%b %e %H:%M"
            } else {
                "%b %e %Y"
            };
            d.format(fmt).ok().map(|s| s.to_string())
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_kinds() {
        assert_eq!(
            name_from_url("https://x.org/a/b%20c.tar.gz?x=1"),
            "b c.tar.gz"
        );
        assert_eq!(name_from_url("https://x.org/"), "x.org");
        assert_eq!(Category::of("b c.tar.gz"), Category::Compressed);
        assert_eq!(Category::of("Movie.MKV"), Category::Video);
        assert_eq!(Category::of("readme"), Category::Other);
        assert_eq!(
            safe_name("../../etc/passwd").as_deref(),
            Some("_.._etc_passwd")
        );
        assert_eq!(safe_name(".bashrc").as_deref(), Some("bashrc"));
        assert_eq!(safe_name("  "), None);
    }

    #[test]
    fn sizes() {
        assert_eq!(size_text(512), "512 B");
        assert_eq!(size_text(1_500_000), "1.5 MB");
        assert_eq!(size_text(180_000_000), "180 MB");
        assert_eq!(eta_text(125.0), "2 min 5 s");
    }
}

/// A name a browser or a server gave, as a file name in its folder: no folders, no hidden
/// file, nothing the file system refuses.
pub fn safe_name(name: &str) -> Option<String> {
    let n: String = name
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let n = n.trim().trim_start_matches('.').trim();
    (!n.is_empty() && n.len() <= 240).then(|| n.to_owned())
}

/// A space-separated site list with `host` added (once) or taken off.
pub fn site_list(list: &str, host: &str, add: bool) -> String {
    let host = host.trim().to_ascii_lowercase();
    let mut v: Vec<&str> = list
        .split_whitespace()
        .filter(|h| !h.eq_ignore_ascii_case(&host))
        .collect();
    if add && !host.is_empty() {
        v.push(&host);
    }
    v.join(" ")
}

#[cfg(test)]
mod site_list_tests {
    #[test]
    fn adds_once_and_takes_off() {
        let l = super::site_list("a.test", "www.youtube.com", true);
        assert_eq!(l, "a.test www.youtube.com");
        assert_eq!(super::site_list(&l, "WWW.youtube.com", true), l);
        assert_eq!(super::site_list(&l, "www.youtube.com", false), "a.test");
    }
}

/// How to install a program here: pacman's command on Arch and its family, the given one on
/// Debian, Ubuntu and theirs (whose names and sources differ).
pub fn install_hint(arch: &str, debian: &str) -> String {
    if std::path::Path::new("/usr/bin/pacman").exists() {
        format!("sudo pacman -S {arch}")
    } else if std::path::Path::new("/usr/bin/apt").exists() {
        debian.to_owned()
    } else {
        format!("install {arch}")
    }
}
