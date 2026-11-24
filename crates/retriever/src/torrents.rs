//! The torrent session (MDM's, on librqbit): started the first time a torrent needs it, and
//! kept for the app's life. Its calls that wait run on the engine's runtime.

use crate::manager::rt;
use crate::model;
use mdm_torrent::{Config, Torrents};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

static SESSION: OnceLock<Arc<Torrents>> = OnceLock::new();
static STARTING: AtomicBool = AtomicBool::new(false);
static FAILED: Mutex<Option<String>> = Mutex::new(None);
/// What went wrong adding or starting a torrent, by hash, for the list to say.
static ERRORS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

fn errors() -> &'static Mutex<HashMap<String, String>> {
    ERRORS.get_or_init(Default::default)
}

pub fn state_dir() -> PathBuf {
    model::data_folder().join("torrents")
}

/// Where a torrent's .torrent is kept (to add it again at the next start).
pub fn torrent_file(hash: &str) -> PathBuf {
    state_dir().join("files").join(format!("{hash}.torrent"))
}

pub fn session() -> Option<Arc<Torrents>> {
    SESSION.get().cloned()
}

/// Why the session couldn't start, if it couldn't.
pub fn failed() -> Option<String> {
    FAILED.lock().ok().and_then(|f| f.clone())
}

/// Starts the session if it isn't (yet): it is ready a moment later ([`session`]).
pub fn ensure() {
    if SESSION.get().is_some() || STARTING.swap(true, Ordering::SeqCst) {
        return;
    }
    rt().spawn(async {
        let _ = std::fs::create_dir_all(state_dir());
        let base = Config {
            // The files are the ones this app hands it, in the folders the person chose.
            require_authorization: false,
            ..Config::new(state_dir().join("session"))
        };
        // Every address (IPv4 and IPv6) first; a system without IPv6 gets IPv4 only (then
        // without the DHT, which wants both), and one whose listener can't open at all still
        // downloads from the peers its trackers name.
        let tries = [
            base.clone(),
            Config {
                listen_ip: Some(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)),
                ..base.clone()
            },
            Config {
                listen_ip: Some(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)),
                dht: false,
                lsd: false,
                ..base.clone()
            },
            Config {
                listen: false,
                upnp: false,
                utp: false,
                dht: false,
                lsd: false,
                ..base
            },
        ];
        let mut last = String::new();
        for cfg in tries {
            match Torrents::start(cfg).await {
                Ok(t) => {
                    let _ = SESSION.set(Arc::new(t));
                    return;
                }
                Err(e) => last = e.to_string(),
            }
        }
        if let Ok(mut f) = FAILED.lock() {
            *f = Some(last);
        }
        STARTING.store(false, Ordering::SeqCst);
    });
}

pub fn take_error(hash: &str) -> Option<String> {
    errors().lock().ok().and_then(|mut e| e.remove(hash))
}

fn note_error(hash: &str, e: impl std::fmt::Display) {
    if let Ok(mut m) = errors().lock() {
        m.insert(hash.to_owned(), e.to_string());
    }
}

/// Started (added first if the session hasn't it), from where it stopped.
pub fn start(hash: String, folder: PathBuf, files: Option<Vec<usize>>, limit: Option<u64>) {
    let Some(t) = session() else { return };
    rt().spawn(async move {
        let r = if t.has(&hash) {
            t.resume(&hash).await
        } else {
            match std::fs::read(torrent_file(&hash)) {
                Ok(bytes) => t
                    .add(&bytes, &folder, files, Vec::new(), false)
                    .await
                    .map(|_| ()),
                Err(e) => {
                    note_error(&hash, format!("its .torrent is gone: {e}"));
                    return;
                }
            }
        };
        match r {
            Ok(()) => {
                let _ = t.set_torrent_limits(&hash, limit, None);
            }
            Err(e) => note_error(&hash, e),
        }
    });
}

pub fn pause(hash: String) {
    let Some(t) = session() else { return };
    rt().spawn(async move {
        let _ = t.pause(&hash).await;
    });
}

pub fn remove(hash: String, files: bool) {
    let path = torrent_file(&hash);
    match session() {
        Some(t) => {
            rt().spawn(async move {
                let _ = t.remove(&hash, files).await;
                let _ = std::fs::remove_file(path);
            });
        }
        None => {
            let _ = std::fs::remove_file(path);
        }
    }
}

pub fn set_limit(hash: &str, limit: Option<u64>) {
    if let Some(t) = session() {
        let _ = t.set_torrent_limits(hash, limit, None);
    }
}
