//! The download list and what runs behind it (MDM's engine for files, yt-dlp for videos, the
//! torrent session): starts, pauses, the queue, retries, the scheduler, extraction, and what
//! each row shows.

use crate::model::{self, After, Category, Item, Kind, Settings, Status, TorrentSpec, VideoSpec};
use crate::object::DlObject;
use crate::torrents;
use gtk::gio;
use gtk::prelude::*;
use mdm_engine::download::{ConnStatus, State};
use mdm_engine::limiter::Limiter;
use mdm_engine::{Destination, Download, Error, Options, Proxy, Request, Snapshot};
use mdm_tools::video::{self as tv, VideoJob};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

pub type Shared = Rc<RefCell<Manager>>;

/// What may come with a link besides its address.
#[derive(Clone, Debug, Default)]
pub struct Extra {
    pub headers: Vec<(String, String)>,
    pub name: Option<String>,
    pub folder: Option<PathBuf>,
    /// Added, not started (IDM's `/a`).
    pub later: bool,
    /// Waits for the scheduler.
    pub scheduled: bool,
}

/// The engine's runtime: downloads run on these threads, never the window's.
pub fn rt() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .thread_name("mdm-engine")
            .enable_all()
            .build()
            .expect("the download runtime")
    })
}

/// Automatic retries of a download that failed for a reason that passes (network, server busy,
/// drive asleep): up to this many in a row, each waiting longer (`retry_wait`).
const MAX_AUTO_RETRIES: u32 = 30;
/// A video's: yt-dlp's failures are more often for good (a private or removed video).
const MAX_VIDEO_RETRIES: u32 = 3;

/// What runs a download now.
pub enum Job {
    File(Download),
    Video(VideoJob),
    /// In the torrent session, by its hash.
    Torrent,
}

pub struct Entry {
    pub item: Item,
    pub obj: DlObject,
    pub dl: Option<Job>,
    /// The last snapshot of its running file download (the progress window reads it).
    pub snap: Option<Snapshot>,
    pub speed: f64,
    /// What its kind says beside its progress: "Checking pieces…", "12 peers", "Joining…".
    pub note: Option<String>,
    last: (Instant, u64),
    /// Failures in a row that were retried by themselves; reset once data comes in.
    retries: u32,
    retry_at: Option<Instant>,
    /// Started over by itself (the file changed on the server): at most twice.
    restarts: u32,
    /// Asked to pause while its download still stops.
    pausing: bool,
    /// An extraction under way: its answer comes here.
    extracting: Option<std::sync::mpsc::Receiver<String>>,
    /// A torrent asked to start, not in the session's list yet.
    since: Option<Instant>,
}

/// What one tick changed, for the window.
#[derive(Default)]
pub struct Tick {
    /// A row changed status (filters and counts need a look).
    pub changed: bool,
    pub finished: Vec<String>,
    pub failed: Vec<(String, String)>,
    /// The scheduler's downloads are done: what to do now.
    pub after: Option<After>,
}

pub struct Manager {
    pub entries: Vec<Entry>,
    pub settings: Settings,
    pub store: gio::ListStore,
    limiter: Arc<Limiter>,
    next_id: u64,
    dirty: bool,
    last_save: Instant,
    /// The app is closing: nothing starts any more.
    closing: bool,
    /// The day (y, m, d) the scheduler last started / stopped on.
    fired_start: Option<(i32, i32, i32)>,
    fired_stop: Option<(i32, i32, i32)>,
    /// The scheduler started its downloads and waits for them to end.
    pub run_active: bool,
}

impl Manager {
    pub fn new(settings: Settings) -> Shared {
        let limiter = Arc::new(Limiter::new(limit(&settings)));
        let store = gio::ListStore::new::<DlObject>();
        let mut m = Manager {
            entries: Vec::new(),
            settings,
            store,
            limiter,
            next_id: 1,
            dirty: false,
            last_save: Instant::now(),
            closing: false,
            fired_start: None,
            fired_stop: None,
            run_active: false,
        };
        for mut item in model::load_list() {
            // Running when the app last closed (or crashed): back in the queue, so it goes on.
            if item.status == Status::Downloading {
                item.status = Status::Queued;
            }
            m.next_id = m.next_id.max(item.id + 1);
            m.push(item);
        }
        if m.entries.iter().any(|e| e.item.kind == Kind::Torrent) {
            torrents::ensure();
        }
        m.schedule();
        Rc::new(RefCell::new(m))
    }

    fn push(&mut self, item: Item) {
        let obj = DlObject::new(item.id);
        obj.set_added(item.added);
        obj.set_added_text(model::date_text(item.added));
        let downloaded = item.downloaded;
        let mut e = Entry {
            item,
            obj,
            dl: None,
            snap: None,
            speed: 0.0,
            note: None,
            last: (Instant::now(), downloaded),
            retries: 0,
            retry_at: None,
            restarts: 0,
            pausing: false,
            extracting: None,
            since: None,
        };
        sync(&mut e);
        self.store.append(&e.obj);
        self.entries.push(e);
    }

    pub fn entry(&self, id: u64) -> Option<&Entry> {
        self.entries.iter().find(|e| e.item.id == id)
    }

    fn entry_mut(&mut self, id: u64) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|e| e.item.id == id)
    }

    fn new_item(&mut self, url: &str, folder: PathBuf, connections: u32, x: &Extra) -> Item {
        let id = self.next_id;
        self.next_id += 1;
        let given = x.name.as_deref().and_then(model::safe_name);
        Item {
            id,
            url: url.to_owned(),
            folder,
            file: None,
            name: given.clone().unwrap_or_else(|| model::name_from_url(url)),
            size: None,
            downloaded: 0,
            status: if x.later || x.scheduled {
                Status::Paused
            } else {
                Status::Queued
            },
            added: model::now(),
            finished: None,
            error: None,
            connections: connections.clamp(1, 32),
            resumable: None,
            headers: x.headers.clone(),
            name_given: given.is_some(),
            kind: Kind::File,
            video: None,
            torrent: None,
            limit_kib: 0,
            scheduled: x.scheduled,
            extracted: None,
        }
    }

    fn added(&mut self, item: Item) -> u64 {
        let id = item.id;
        self.push(item);
        self.dirty = true;
        self.schedule();
        self.save();
        id
    }

    #[cfg(test)]
    pub fn add(&mut self, url: &str, folder: PathBuf, connections: u32) -> u64 {
        self.add_with(url, folder, connections, &Extra::default())
    }

    /// A download with what came with its link (a browser's headers, its file name).
    pub fn add_with(&mut self, url: &str, folder: PathBuf, connections: u32, x: &Extra) -> u64 {
        let item = self.new_item(url, folder, connections, x);
        self.added(item)
    }

    /// A video from `page`, downloaded by yt-dlp as `spec` says, named `title`.
    pub fn add_video(
        &mut self,
        page: &str,
        folder: PathBuf,
        title: &str,
        spec: VideoSpec,
        x: &Extra,
    ) -> u64 {
        let mut item = self.new_item(page, folder, self.settings.connections, x);
        item.kind = Kind::Video;
        item.name = title.to_owned();
        item.size = spec.expected;
        item.video = Some(spec);
        self.added(item)
    }

    /// A torrent (its .torrent already kept at `torrents::torrent_file`).
    pub fn add_torrent(
        &mut self,
        link: &str,
        name: &str,
        size: u64,
        folder: PathBuf,
        spec: TorrentSpec,
        x: &Extra,
    ) -> u64 {
        if let Some(e) = self
            .entries
            .iter()
            .find(|e| e.item.torrent.as_ref().is_some_and(|t| t.hash == spec.hash))
        {
            // The same torrent twice is one download.
            return e.item.id;
        }
        torrents::ensure();
        let mut item = self.new_item(link, folder, self.settings.connections, x);
        item.kind = Kind::Torrent;
        item.name = name.to_owned();
        item.size = Some(size);
        item.torrent = Some(spec);
        self.added(item)
    }

    pub fn resume(&mut self, ids: &[u64]) {
        for &id in ids {
            if let Some(e) = self.entry_mut(id)
                && matches!(e.item.status, Status::Paused | Status::Failed)
            {
                e.item.status = Status::Queued;
                e.item.error = None;
                e.retries = 0;
                e.retry_at = None;
                e.pausing = false;
                sync(e);
            }
        }
        self.dirty = true;
        self.schedule();
    }

    pub fn pause(&mut self, ids: &[u64]) {
        for &id in ids {
            if let Some(e) = self.entry_mut(id) {
                match e.item.status {
                    Status::Downloading => {
                        match &e.dl {
                            Some(Job::File(dl)) => {
                                dl.cancel();
                                e.pausing = true;
                            }
                            Some(Job::Video(job)) => {
                                job.stop();
                                e.pausing = true;
                            }
                            Some(Job::Torrent) => {
                                if let Some(t) = &e.item.torrent {
                                    torrents::pause(t.hash.clone());
                                }
                                e.dl = None;
                                e.speed = 0.0;
                            }
                            None => {}
                        }
                        e.item.status = Status::Paused;
                    }
                    Status::Queued => e.item.status = Status::Paused,
                    // A failure waiting for its retry: the retry is called off.
                    Status::Failed => e.retry_at = None,
                    _ => {}
                }
                sync(e);
            }
        }
        self.dirty = true;
        self.schedule();
    }

    /// Starts over from nothing (a file that changed on the server, a broken resume state).
    pub fn restart(&mut self, ids: &[u64]) {
        for &id in ids {
            if let Some(e) = self.entry_mut(id)
                && e.dl.is_none()
                && e.item.status != Status::Downloading
                && e.item.kind != Kind::Torrent
            {
                if let Some(f) = e.item.file.take() {
                    remove_traces(&f, e.item.status != Status::Done);
                }
                e.item.downloaded = 0;
                if e.item.kind == Kind::File {
                    e.item.size = None;
                }
                e.item.error = None;
                e.item.finished = None;
                e.item.extracted = None;
                e.item.status = Status::Queued;
                e.retries = 0;
                e.retry_at = None;
                sync(e);
            }
        }
        self.dirty = true;
        self.schedule();
    }

    /// Takes downloads off the list; `files` also deletes what they saved.
    pub fn remove(&mut self, ids: &[u64], files: bool) {
        for &id in ids {
            let Some(pos) = self.entries.iter().position(|e| e.item.id == id) else {
                continue;
            };
            let e = self.entries.remove(pos);
            if let Some(p) = self.store.find(&e.obj) {
                self.store.remove(p);
            }
            let file = e.item.file.clone();
            let done = e.item.status == Status::Done;
            if let Some(t) = &e.item.torrent {
                torrents::remove(t.hash.clone(), files);
                continue;
            }
            match e.dl {
                Some(Job::File(dl)) => {
                    dl.cancel();
                    // Deleted only once the download has let go of its files.
                    rt().spawn(async move {
                        let _ = dl.wait().await;
                        if files && let Some(f) = file {
                            remove_traces(&f, true);
                        }
                    });
                }
                Some(Job::Video(job)) => {
                    job.stop();
                    if files && let Some(f) = file {
                        remove_traces(&f, true);
                    }
                }
                _ => {
                    // Off the list unfinished, its part and resume state would only take space;
                    // a finished file stays unless its file is deleted too.
                    if let Some(f) = file
                        && (files || !done)
                    {
                        remove_traces(&f, files);
                    }
                }
            }
        }
        self.dirty = true;
        self.schedule();
        self.save();
    }

    /// Where its file (or a torrent's folder) is.
    pub fn set_file(&mut self, id: u64, file: PathBuf) {
        if let Some(e) = self.entry_mut(id) {
            e.item.file = Some(file);
            self.dirty = true;
        }
    }

    /// Waits for the scheduler (`on`), or no longer does. One waiting and not running yet is
    /// held until the start time.
    pub fn set_scheduled(&mut self, ids: &[u64], on: bool) {
        for &id in ids {
            if let Some(e) = self.entry_mut(id) {
                e.item.scheduled = on;
                if on && e.item.status == Status::Queued && e.dl.is_none() {
                    e.item.status = Status::Paused;
                }
                sync(e);
            }
        }
        self.dirty = true;
        self.save();
    }

    /// Every finished download off the list (their files stay); how many.
    pub fn clear_finished(&mut self) -> usize {
        let done = self.ids_where(|e| e.item.status == Status::Done && e.extracting.is_none());
        self.remove(&done, false);
        done.len()
    }

    /// Its own speed limit (KB/s, 0: none), at once if it runs.
    pub fn set_limit(&mut self, id: u64, kib: u64) {
        let Some(e) = self.entry_mut(id) else { return };
        e.item.limit_kib = kib;
        let rate = (kib > 0).then(|| kib * 1024);
        match &e.dl {
            Some(Job::File(dl)) => dl.set_speed_limit(rate),
            Some(Job::Video(job)) => job.set_speed_limit(rate),
            Some(Job::Torrent) => {
                if let Some(t) = &e.item.torrent {
                    torrents::set_limit(&t.hash, rate);
                }
            }
            None => {}
        }
        self.dirty = true;
    }

    /// Extracts finished archives beside themselves (the context menu's Extract).
    pub fn extract(&mut self, ids: &[u64]) {
        for &id in ids {
            if let Some(e) = self.entry_mut(id)
                && e.item.status == Status::Done
                && e.extracting.is_none()
            {
                start_extract(e);
                sync(e);
            }
        }
    }

    pub fn active(&self) -> usize {
        self.entries.iter().filter(|e| e.dl.is_some()).count()
    }

    pub fn downloading(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.item.status == Status::Downloading)
            .count()
    }

    /// The scheduler is on and has downloads to start (the computer must stay awake for them).
    pub fn schedule_waiting(&self) -> bool {
        self.settings.schedule.enabled
            && self
                .entries
                .iter()
                .any(|e| e.item.scheduled && e.item.status != Status::Done)
    }

    pub fn total_speed(&self) -> f64 {
        self.entries
            .iter()
            .filter(|e| e.item.status == Status::Downloading)
            .map(|e| e.speed)
            .sum()
    }

    pub fn count(&self, c: Category) -> usize {
        self.entries.iter().filter(|e| c.holds(&e.item)).count()
    }

    /// Settings changed: the speed limit applies at once, the rest to the next start.
    pub fn apply_settings(&mut self, s: Settings) {
        self.limiter.set(limit(&s));
        self.settings = s;
        model::save_settings(&self.settings);
        self.schedule();
    }

    /// The scheduler's downloads start now (its Start Now, or its start time).
    pub fn start_scheduled(&mut self) {
        let ids = self.ids_where(|e| {
            e.item.scheduled && matches!(e.item.status, Status::Paused | Status::Failed)
        });
        self.resume(&ids);
        self.run_active = true;
    }

    pub fn stop_scheduled(&mut self) {
        let ids = self.ids_where(|e| {
            e.item.scheduled && matches!(e.item.status, Status::Downloading | Status::Queued)
        });
        self.pause(&ids);
        self.run_active = false;
    }

    /// Starts queued downloads while there is room, oldest first.
    fn schedule(&mut self) {
        if self.closing {
            return;
        }
        let room = (self.settings.simultaneous.max(1) as usize).saturating_sub(self.active());
        let mut started = 0;
        let opts = self.options();
        let session = torrents::session().is_some();
        for e in self.entries.iter_mut() {
            if started >= room {
                break;
            }
            if e.item.status == Status::Queued && e.dl.is_none() {
                if e.item.kind == Kind::Torrent && !session {
                    // Started once the session is up (a moment after the app starts).
                    torrents::ensure();
                    continue;
                }
                // Its own number of connections, as chosen in Add URL, and its own limit.
                let opts = Options {
                    max_connections: e.item.connections.clamp(1, 32),
                    speed_limit: (e.item.limit_kib > 0).then(|| e.item.limit_kib * 1024),
                    ..opts.clone()
                };
                start(e, opts);
                started += 1;
            }
        }
        if started > 0 {
            self.dirty = true;
        }
    }

    fn options(&self) -> Options {
        Options {
            max_connections: self.settings.connections.clamp(1, 32),
            // Each connection retries by itself before the download gives up.
            max_retries: 10,
            agent_fallback: Some(mdm_engine::agent::IE11_USER_AGENT.to_owned()),
            own_agent: Some(mdm_engine::agent::OWN_USER_AGENT.to_owned()),
            proxy: if self.settings.system_proxy {
                Proxy::System
            } else {
                Proxy::None
            },
            global: Some(self.limiter.clone()),
            ..Options::default()
        }
    }

    /// Every half second: the downloads, the ones that ended, extractions, the queue, the
    /// retries and the scheduler.
    pub fn tick(&mut self) -> Tick {
        let mut t = Tick::default();
        if self.closing {
            return t;
        }
        let now = Instant::now();
        let seed = self.settings.seed;
        let extract = self.settings.extract;
        let mut dirty = false;
        for e in self.entries.iter_mut() {
            // An extraction's answer.
            if let Some(rx) = &e.extracting
                && let Ok(said) = rx.try_recv()
            {
                e.extracting = None;
                e.item.extracted = Some(said);
                dirty = true;
                sync(e);
            }
            let before = e.item.status;
            match &e.dl {
                None => {
                    // A failure that passes, retried once its wait is over.
                    if e.item.status == Status::Failed && e.retry_at.is_some_and(|at| now >= at) {
                        e.retry_at = None;
                        e.item.status = Status::Queued;
                        t.changed = true;
                    }
                    if e.item.status == Status::Failed {
                        // Its countdown.
                        sync(e);
                    }
                    continue;
                }
                Some(Job::File(_)) => tick_file(e, &mut t, now, &mut dirty),
                Some(Job::Video(_)) => tick_video(e, &mut t, now),
                Some(Job::Torrent) => tick_torrent(e, &mut t, now, seed),
            }
            if before != Status::Done && e.item.status == Status::Done {
                e.item.finished = Some(model::now());
                t.finished.push(e.item.name.clone());
                if extract && e.item.kind == Kind::File && is_archive(e) {
                    start_extract(e);
                }
            }
            if before != e.item.status {
                t.changed = true;
                dirty = true;
            }
            sync(e);
        }
        if dirty {
            self.dirty = true;
        }
        if self.settings.forget_finished {
            // Off the list once finished (and extracted): the files stay. A finished torrent
            // that goes on sharing stays while it shares.
            let seed = self.settings.seed;
            let done = self.ids_where(|e| {
                e.item.status == Status::Done
                    && e.extracting.is_none()
                    && e.dl.is_none()
                    && !(seed && e.item.kind == Kind::Torrent)
            });
            if !done.is_empty() {
                self.remove(&done, false);
                t.changed = true;
            }
        }
        self.scheduler(&mut t);
        self.schedule();
        if self.dirty && self.last_save.elapsed() > Duration::from_secs(2) || t.changed {
            self.save();
        }
        t
    }

    fn scheduler(&mut self, t: &mut Tick) {
        let s = self.settings.schedule.clone();
        if s.enabled
            && let Ok(now) = gtk::glib::DateTime::now_local()
        {
            let day = now.ymd();
            let minute = (now.hour() * 60 + now.minute()) as u32;
            if minute == s.start && self.fired_start != Some(day) {
                self.fired_start = Some(day);
                self.start_scheduled();
                t.changed = true;
            }
            if s.stop_enabled && minute == s.stop && self.fired_stop != Some(day) {
                self.fired_stop = Some(day);
                self.stop_scheduled();
                t.changed = true;
            }
        }
        if self.run_active {
            let busy = self.entries.iter().any(|e| {
                e.item.scheduled
                    && (matches!(e.item.status, Status::Downloading | Status::Queued)
                        || e.retry_at.is_some()
                        || e.extracting.is_some())
            });
            if !busy {
                self.run_active = false;
                if s.then != After::Nothing {
                    t.after = Some(s.then);
                }
            }
        }
    }

    pub fn save(&mut self) {
        let items: Vec<Item> = self.entries.iter().map(|e| e.item.clone()).collect();
        model::save_list(&items);
        self.dirty = false;
        self.last_save = Instant::now();
    }

    /// The app closes: every download is paused the way Pause leaves it (its resume state
    /// written), and comes back by itself next time.
    pub fn shutdown(&mut self) {
        self.closing = true;
        let mut running = Vec::new();
        for e in self.entries.iter_mut() {
            match e.dl.take() {
                Some(Job::File(dl)) => {
                    dl.cancel();
                    running.push(dl);
                    e.item.status = Status::Queued;
                }
                Some(Job::Video(job)) => {
                    job.stop();
                    e.item.status = Status::Queued;
                }
                Some(Job::Torrent) => e.item.status = Status::Queued,
                None => {
                    if e.item.status == Status::Failed && e.retry_at.is_some() {
                        // Waiting for its retry: tried again at the next start.
                        e.item.status = Status::Queued;
                    }
                }
            }
        }
        self.save();
        rt().block_on(async move {
            let all = async {
                futures_join(running).await;
                if let Some(t) = torrents::session() {
                    t.stop().await;
                }
            };
            let _ = tokio::time::timeout(Duration::from_secs(10), all).await;
        });
    }

    pub fn ids_where(&self, f: impl Fn(&Entry) -> bool) -> Vec<u64> {
        self.entries
            .iter()
            .filter(|e| f(e))
            .map(|e| e.item.id)
            .collect()
    }
}

async fn futures_join(downloads: Vec<Download>) {
    let tasks: Vec<_> = downloads
        .into_iter()
        .map(|d| rt().spawn(async move { d.wait().await }))
        .collect();
    for t in tasks {
        let _ = t.await;
    }
}

fn limit(s: &Settings) -> Option<u64> {
    (s.speed_limit > 0).then(|| s.speed_limit * 1024)
}

/// 5 s, 10 s, 20 s … up to 2 minutes.
fn retry_wait(n: u32) -> Duration {
    Duration::from_secs((5u64 << n.saturating_sub(1).min(5)).min(120))
}

/// A failure that may pass by itself (the network, a busy server, a drive waking up) rather
/// than one that stays (the file is gone, the disk is full, the file changed).
fn passes(e: Option<&Error>) -> bool {
    let Some(e) = e else { return true };
    match e {
        Error::Status(s) => matches!(s.as_u16(), 408 | 429 | 500..=599),
        Error::FtpReply { code, .. } => !matches!(code, 530 | 532 | 550 | 553),
        Error::FileChanged
        | Error::NoRanges
        | Error::BadState(_)
        | Error::NoSpace { .. }
        | Error::DiskFull
        | Error::TooBigForDrive { .. }
        | Error::PathTooLong(_)
        | Error::ChecksumMismatch(_)
        | Error::InUse(_)
        | Error::GotWebPage
        | Error::NotTheFile { .. }
        | Error::LinkExpired
        | Error::BotCheck
        | Error::Refused(_)
        | Error::NoCertificates
        | Error::Certificate(_) => false,
        _ => true,
    }
}

fn measure(e: &mut Entry, downloaded: u64) {
    let dt = e.last.0.elapsed().as_secs_f64().max(1e-3);
    let inst = downloaded.saturating_sub(e.last.1) as f64 / dt;
    e.speed = if e.speed == 0.0 {
        inst
    } else {
        0.6 * e.speed + 0.4 * inst
    };
    e.last = (Instant::now(), downloaded);
    if downloaded > e.item.downloaded {
        // Data is coming in: the failures before are behind it.
        e.retries = 0;
    }
}

fn tick_file(e: &mut Entry, t: &mut Tick, now: Instant, dirty: &mut bool) {
    let Some(Job::File(dl)) = &e.dl else { return };
    let s = dl.snapshot();
    measure(e, s.downloaded);
    e.item.downloaded = s.downloaded;
    if s.size.is_some() {
        e.item.size = s.size;
    }
    if s.resumable.is_some() {
        e.item.resumable = s.resumable;
    }
    if let Some(f) = &s.file
        && e.item.file.as_ref() != Some(f)
    {
        e.item.file = Some(f.clone());
        if let Some(n) = f.file_name() {
            e.item.name = n.to_string_lossy().into_owned();
        }
        // Saved at once: a crash from here on resumes into this file.
        *dirty = true;
        t.changed = true;
    }
    match s.state {
        State::Done => {
            e.item.status = Status::Done;
            if let Some(size) = s.size.or(Some(s.downloaded)) {
                e.item.size = Some(size);
                e.item.downloaded = size;
            }
            e.dl = None;
            e.speed = 0.0;
        }
        State::Cancelled => {
            if e.item.status == Status::Downloading {
                e.item.status = Status::Paused;
            }
            e.dl = None;
            e.pausing = false;
            e.speed = 0.0;
            t.changed = true;
        }
        State::Failed => {
            e.dl = None;
            e.speed = 0.0;
            let why = s.error.clone().unwrap_or_else(|| "failed".into());
            if e.pausing {
                // Failed while it stopped: it is paused, as asked.
                e.pausing = false;
                e.item.status = Status::Paused;
            } else if matches!(s.failure, Some(Error::FileChanged | Error::NoRanges))
                && e.restarts < 2
            {
                // The file changed on the server, or it no longer resumes: what is on the
                // disk can't be finished, so it starts over by itself.
                e.restarts += 1;
                if let Some(f) = &e.item.file {
                    remove_traces(f, false);
                }
                e.item.downloaded = 0;
                e.item.size = None;
                e.item.status = Status::Queued;
                e.item.error = None;
            } else if passes(s.failure.as_ref()) && e.retries < MAX_AUTO_RETRIES {
                e.retries += 1;
                e.retry_at = Some(now + retry_wait(e.retries));
                e.item.status = Status::Failed;
                e.item.error = Some(why);
            } else {
                e.item.status = Status::Failed;
                e.item.error = Some(why.clone());
                t.failed.push((e.item.name.clone(), why));
            }
            t.changed = true;
        }
        _ => {}
    }
    e.snap = if e.dl.is_some() { Some(s) } else { None };
}

fn tick_video(e: &mut Entry, t: &mut Tick, now: Instant) {
    let Some(Job::Video(job)) = &e.dl else { return };
    let p = job.progress();
    measure(e, p.downloaded);
    e.item.downloaded = p.downloaded;
    if p.total.is_some() {
        e.item.size = p.total;
    }
    e.speed = p.speed;
    e.note = p.merging.map(|_| "Joining picture and sound…".into());
    match p.state {
        tv::State::Running => {}
        tv::State::Done(f) => {
            e.item.status = Status::Done;
            if let Ok(m) = std::fs::metadata(&f) {
                e.item.size = Some(m.len());
                e.item.downloaded = m.len();
            }
            if let Some(n) = f.file_name() {
                e.item.name = n.to_string_lossy().into_owned();
            }
            e.item.file = Some(f);
            e.dl = None;
            e.speed = 0.0;
            e.note = None;
        }
        tv::State::Stopped => {
            if e.item.status == Status::Downloading {
                e.item.status = Status::Paused;
            }
            e.pausing = false;
            e.dl = None;
            e.speed = 0.0;
            e.note = None;
            t.changed = true;
        }
        tv::State::Failed(why) => {
            e.dl = None;
            e.speed = 0.0;
            e.note = None;
            e.item.status = Status::Failed;
            e.item.error = Some(why.clone());
            if e.pausing {
                e.pausing = false;
                e.item.status = Status::Paused;
            } else if e.retries < MAX_VIDEO_RETRIES {
                e.retries += 1;
                e.retry_at = Some(now + retry_wait(e.retries));
            } else {
                t.failed.push((e.item.name.clone(), why));
            }
            t.changed = true;
        }
    }
}

fn tick_torrent(e: &mut Entry, t: &mut Tick, now: Instant, seed: bool) {
    let Some(spec) = e.item.torrent.clone() else {
        return;
    };
    let Some(session) = torrents::session() else {
        return;
    };
    if let Some(why) = torrents::take_error(&spec.hash) {
        e.dl = None;
        e.speed = 0.0;
        e.item.status = Status::Failed;
        e.item.error = Some(why.clone());
        t.failed.push((e.item.name.clone(), why));
        return;
    }
    let Ok(st) = session.stats(&spec.hash) else {
        // Being added; a torrent that never shows up failed to start.
        let since = *e.since.get_or_insert(now);
        e.note = Some("Starting…".into());
        if now.duration_since(since) > Duration::from_secs(60) {
            e.dl = None;
            e.item.status = Status::Failed;
            e.item.error = Some("the torrent session didn't take it".into());
        }
        return;
    };
    e.since = None;
    e.item.size = Some(st.total);
    e.item.downloaded = st.done;
    e.speed = st.down_speed;
    use mdm_torrent::Phase;
    e.note = match st.phase {
        Phase::Checking => Some(match st.checked {
            Some(c) if st.total > 0 => format!(
                "Checking pieces ({:.0}%)",
                c as f64 / st.total as f64 * 100.0
            ),
            _ => "Checking pieces…".into(),
        }),
        Phase::Downloading => Some(format!(
            "{} peer{}",
            st.peers_live,
            if st.peers_live == 1 { "" } else { "s" }
        )),
        _ => None,
    };
    match st.phase {
        Phase::Finished => {
            e.item.status = Status::Done;
            e.item.downloaded = st.total;
            if !seed {
                torrents::pause(spec.hash.clone());
            }
            e.dl = None;
            e.speed = 0.0;
            e.note = None;
        }
        Phase::Error => {
            let why = st.error.unwrap_or_else(|| "the torrent stopped".into());
            e.dl = None;
            e.speed = 0.0;
            e.item.status = Status::Failed;
            e.item.error = Some(why.clone());
            t.failed.push((e.item.name.clone(), why));
        }
        _ => {}
    }
}

fn is_archive(e: &Entry) -> bool {
    e.item
        .file
        .as_ref()
        .is_some_and(|f| matches!(mdm_extract::Format::detect(f), Ok(Some(_))))
}

/// Extracts beside the archive, on a thread of its own.
fn start_extract(e: &mut Entry) {
    let Some(archive) = e.item.file.clone() else {
        return;
    };
    let (tx, rx) = std::sync::mpsc::channel();
    e.extracting = Some(rx);
    std::thread::spawn(move || {
        let dest = archive
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let mut on_file = |_: &Path| {};
        let mut policy = mdm_extract::Policy {
            dest,
            subfolder: mdm_extract::Subfolder::Smart,
            password: None,
            limits: Default::default(),
            tool: mdm_extract::find_7zip(),
            hatchzip: None,
            in_place: false,
            runs: None,
            on_file: &mut on_file,
            cancel: None,
        };
        let said = match mdm_extract::extract(&archive, &mut policy) {
            Ok(r) => format!(
                "Extracted {} file{} to {}",
                r.files,
                if r.files == 1 { "" } else { "s" },
                r.folder.display()
            ),
            Err(mdm_extract::Error::NeedsPassword) => "Not extracted: it needs a password".into(),
            Err(mdm_extract::Error::NoTool) => {
                format!("Not extracted: 7z and RAR archives need 7-Zip ({})", crate::model::install_hint("7zip", "sudo apt install 7zip"))
            }
            Err(mdm_extract::Error::Unsupported) => "Not an archive this app extracts".into(),
            Err(e) => format!("Not extracted: {e}"),
        };
        let _ = tx.send(said);
    });
}

fn start(e: &mut Entry, opts: Options) {
    e.item.error = None;
    e.speed = 0.0;
    e.last = (Instant::now(), e.item.downloaded);
    e.since = None;
    match e.item.kind {
        Kind::File => start_file(e, opts),
        Kind::Video => start_video(e, opts),
        Kind::Torrent => {
            let Some(spec) = &e.item.torrent else { return };
            torrents::start(
                spec.hash.clone(),
                e.item.folder.clone(),
                spec.files.clone(),
                (e.item.limit_kib > 0).then(|| e.item.limit_kib * 1024),
            );
            e.dl = Some(Job::Torrent);
            e.item.status = Status::Downloading;
        }
    }
    sync(e);
}

fn start_video(e: &mut Entry, opts: Options) {
    let Some(spec) = e.item.video.clone() else {
        return;
    };
    let fail = |e: &mut Entry, why: String| {
        e.item.status = Status::Failed;
        e.item.error = Some(why);
    };
    let Some(exe) = crate::video::ytdlp() else {
        fail(e, format!("yt-dlp isn't installed ({})", crate::model::install_hint("yt-dlp", "pipx install yt-dlp")));
        return;
    };
    let _ = std::fs::create_dir_all(&e.item.folder);
    let ffmpeg = crate::video::ffmpeg();
    let rate = opts.speed_limit;
    let engine = mdm_tools::streams::Engine {
        handle: rt().handle().clone(),
        opts,
    };
    match VideoJob::start(
        &exe,
        &spec.address,
        &spec.format,
        &e.item.folder,
        &spec.stem,
        rate,
        ffmpeg.as_deref(),
        spec.expected,
        &Default::default(),
        &Default::default(),
        Some(engine),
    ) {
        Ok(job) => {
            e.dl = Some(Job::Video(job));
            e.item.status = Status::Downloading;
        }
        Err(why) => fail(e, why.to_string()),
    }
}

fn start_file(e: &mut Entry, opts: Options) {
    let Ok(url) = url::Url::parse(&e.item.url) else {
        e.item.status = Status::Failed;
        e.item.error = Some("not a valid address".into());
        return;
    };
    let mut headers = reqwest::header::HeaderMap::new();
    for (k, v) in &e.item.headers {
        if let (Ok(k), Ok(v)) = (
            reqwest::header::HeaderName::from_bytes(k.as_bytes()),
            reqwest::header::HeaderValue::from_str(v),
        ) {
            headers.append(k, v);
        }
    }
    let _in = rt().enter();
    let dl = match &e.item.file {
        // Named already: resumed from its state, or started again at the same name.
        Some(f) => Download::resume_or_start(
            mdm_engine::state::state_path(f),
            Request {
                url,
                headers,
                dest: Destination::File(f.clone()),
            },
            opts,
            None,
            e.item.downloaded,
        ),
        None => {
            let _ = std::fs::create_dir_all(&e.item.folder);
            let dest = if e.item.name_given {
                // The browser's name, numbered if a file has it already.
                Destination::Free(e.item.folder.join(&e.item.name))
            } else {
                Destination::Dir(e.item.folder.clone())
            };
            Download::start(Request { url, headers, dest }, opts)
        }
    };
    e.dl = Some(Job::File(dl));
    e.item.status = Status::Downloading;
}

/// Deletes a download's part file and resume state, and its file when `file`.
fn remove_traces(f: &Path, file: bool) {
    let _ = std::fs::remove_file(mdm_engine::naming::part_path(f));
    let _ = std::fs::remove_file(mdm_engine::state::state_path(f));
    if file {
        if f.is_dir() {
            let _ = std::fs::remove_dir_all(f);
        } else {
            let _ = std::fs::remove_file(f);
        }
    }
}

fn status_code(s: Status) -> u32 {
    match s {
        Status::Queued => 0,
        Status::Downloading => 1,
        Status::Paused => 2,
        Status::Done => 3,
        Status::Failed => 4,
    }
}

pub fn code_status(c: u32) -> Status {
    match c {
        0 => Status::Queued,
        1 => Status::Downloading,
        2 => Status::Paused,
        3 => Status::Done,
        _ => Status::Failed,
    }
}

fn kind_code(c: Category) -> u32 {
    Category::SIDEBAR
        .iter()
        .position(|k| *k == c)
        .map_or(99, |p| p as u32)
}

pub fn code_kind(c: u32) -> Category {
    Category::SIDEBAR
        .get(c as usize)
        .copied()
        .unwrap_or(Category::Other)
}

pub fn fraction(item: &Item) -> f64 {
    match item.size {
        Some(t) if t > 0 => (item.downloaded as f64 / t as f64).clamp(0.0, 1.0),
        _ if item.status == Status::Done => 1.0,
        _ => 0.0,
    }
}

/// The row's words for where its download is.
pub fn status_words(e: &Entry) -> String {
    let pct = || {
        e.item
            .size
            .filter(|t| *t > 0)
            .map(|_| format!("{:.1}%", fraction(&e.item) * 100.0))
    };
    match e.item.status {
        Status::Queued if e.item.kind == Kind::Torrent && torrents::session().is_none() => {
            match torrents::failed() {
                Some(why) => format!("Torrents unavailable: {why}"),
                None => "Starting torrents…".into(),
            }
        }
        Status::Queued => "Queued".into(),
        Status::Done if e.extracting.is_some() => "Extracting…".into(),
        Status::Done => "Complete".into(),
        Status::Paused if e.item.scheduled && !e.pausing => match pct() {
            Some(p) if e.item.downloaded > 0 => format!("Scheduled · {p}"),
            _ => "Scheduled".into(),
        },
        Status::Paused => match (e.pausing, pct()) {
            (true, _) => "Pausing…".into(),
            (false, Some(p)) => format!("Paused · {p}"),
            (false, None) => "Paused".into(),
        },
        Status::Failed => match e.retry_at {
            Some(at) => format!(
                "Retrying in {} s",
                at.saturating_duration_since(Instant::now())
                    .as_secs()
                    .max(1)
            ),
            // Why, on the row itself (it had only a tooltip).
            None => match &e.item.error {
                Some(why) => format!("Failed: {}", why.lines().next().unwrap_or(why)),
                None => "Failed".into(),
            },
        },
        Status::Downloading => {
            if let Some(n) = &e.note
                && e.item.kind != Kind::Torrent
            {
                return n.clone();
            }
            if e.item.kind != Kind::File {
                return match (pct(), &e.note) {
                    (Some(p), Some(n)) => format!("{p} · {n}"),
                    (Some(p), None) => p,
                    (None, Some(n)) => n.clone(),
                    (None, None) => "Starting…".into(),
                };
            }
            match e.snap.as_ref().map(|s| s.state) {
                None | Some(State::Probing) => {
                    if e.item.downloaded > 0 {
                        "Resuming…".into()
                    } else {
                        "Connecting…".into()
                    }
                }
                Some(State::Verifying) => "Verifying…".into(),
                Some(State::Completing | State::Merging) => "Finishing…".into(),
                Some(State::Stopping) => "Pausing…".into(),
                // Every connection lost (the network, the server): the engine reconnects.
                Some(State::Downloading)
                    if e.snap.as_ref().is_some_and(|s| {
                        receiving(s) == 0
                            && s.connections
                                .iter()
                                .any(|c| matches!(c.status, ConnStatus::Retrying(_)))
                    }) =>
                {
                    "Reconnecting…".into()
                }
                _ => pct().unwrap_or_else(|| model::size_text(e.item.downloaded)),
            }
        }
    }
}

/// How many of its connections are receiving.
pub fn receiving(s: &Snapshot) -> usize {
    s.connections
        .iter()
        .filter(|c| matches!(c.status, ConnStatus::Receiving))
        .count()
}

/// Copies an entry's state onto its row.
fn sync(e: &mut Entry) {
    let o = &e.obj;
    let it = &e.item;
    if o.name() != it.name {
        o.set_name(it.name.clone());
        let (ct, _) = gio::content_type_guess(Some(it.name.as_str()), None::<&[u8]>);
        let icon = match it.kind {
            Kind::Torrent if it.name.rsplit_once('.').is_none() => {
                gio::ThemedIcon::new("folder-download").upcast()
            }
            _ => gio::content_type_get_icon(&ct),
        };
        o.set_gicon(Some(icon));
        let sym = match it.kind {
            Kind::Torrent if it.name.rsplit_once('.').is_none() => {
                gio::ThemedIcon::new("folder-download-symbolic").upcast()
            }
            _ => gio::content_type_get_symbolic_icon(&ct),
        };
        o.set_symbolic(Some(sym));
        let kind = match it.kind {
            Kind::Video => Category::Video,
            _ => it.category(),
        };
        o.set_kind(kind_code(kind));
    }
    o.put(
        "size-text",
        &it.size.map(model::size_text).unwrap_or_default(),
    );
    if o.size_bytes() != it.size.unwrap_or(0) {
        o.set_size_bytes(it.size.unwrap_or(0));
    }
    let code = status_code(it.status);
    if o.status() != code {
        o.set_status(code);
    }
    o.put("status-text", &status_words(e));
    let f = fraction(it);
    if (o.fraction() - f).abs() > 1e-4 {
        o.set_fraction(f);
    }
    let bar = matches!(it.status, Status::Downloading | Status::Paused) && it.size.is_some();
    if o.show_progress() != bar || o.show_label() == bar {
        o.set_show_progress(bar);
        o.set_show_label(!bar);
    }
    let downloading = it.status == Status::Downloading;
    o.put(
        "speed-text",
        &if downloading {
            model::speed_text(e.speed)
        } else {
            String::new()
        },
    );
    let eta = match (downloading, it.size) {
        (true, Some(t)) if e.speed > 1.0 => {
            model::eta_text(t.saturating_sub(it.downloaded) as f64 / e.speed)
        }
        _ => String::new(),
    };
    o.put("eta-text", &eta);
    // The row's right side, its second line, its button.
    let size = it.size.map(model::size_text);
    let (d1, d2) = match it.status {
        Status::Downloading => (
            if e.speed >= 1.0 {
                model::speed_text(e.speed)
            } else {
                size.clone().unwrap_or_default()
            },
            if eta.is_empty() {
                size.map(|s| format!("{s} total")).unwrap_or_default()
            } else {
                format!("{eta} left")
            },
        ),
        Status::Done => (
            size.unwrap_or_default(),
            model::date_text(it.finished.unwrap_or(it.added)),
        ),
        _ => (
            match (it.downloaded, &size) {
                (d, Some(s)) if d > 0 => format!("{} of {s}", model::size_text(d)),
                (_, Some(s)) => s.clone(),
                _ => String::new(),
            },
            model::date_text(it.added),
        ),
    };
    o.put("detail", &d1);
    o.put("detail2", &d2);
    let (icon, tip) = match it.status {
        Status::Downloading | Status::Queued => ("media-playback-pause-symbolic", "Pause"),
        Status::Paused | Status::Failed => ("media-playback-start-symbolic", "Resume"),
        Status::Done => ("folder-open-symbolic", "Show in Folder"),
    };
    o.put("action-icon", icon);
    o.put("action-tip", tip);
}

/// Real downloads against the lab server (`MDM_LABSERVER=<path to labserver>`): pause and
/// resume, dropped connections, the server gone for a while, the app closed and opened again.
#[cfg(test)]
mod lab {
    use super::*;
    use std::process::{Child, Command};

    fn server(bin: &str, port: u16) -> Child {
        let c = Command::new(bin)
            .args([
                "--bind",
                &format!("127.0.0.1:{port}"),
                "--faults",
                "rate=2M",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("labserver");
        std::thread::sleep(Duration::from_millis(400));
        c
    }

    fn run_until(m: &Shared, secs: u64, mut done: impl FnMut(&Manager) -> bool) -> bool {
        let end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < end {
            m.borrow_mut().tick();
            if done(&m.borrow()) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        false
    }

    fn status(m: &Manager, id: u64) -> Status {
        m.entry(id).unwrap().item.status
    }

    #[test]
    fn downloads_survive_interruptions() {
        let Ok(bin) = std::env::var("MDM_LABSERVER") else {
            eprintln!("MDM_LABSERVER not set: skipped");
            return;
        };
        let tmp = std::env::temp_dir().join(format!("retriever-lab-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        check_env(&tmp);
        let dl_dir = tmp.join("dl");
        let mut srv = server(&bin, 8091);
        let s = Settings {
            folder: dl_dir.clone(),
            system_proxy: false,
            ..Settings::default()
        };
        let m = Manager::new(s.clone());
        let base = "http://127.0.0.1:8091/gen";

        // 1. Pause and resume.
        let a = m
            .borrow_mut()
            .add(&format!("{base}/40M/a.bin"), dl_dir.clone(), 4);
        assert!(run_until(&m, 20, |m| m.entry(a).unwrap().item.downloaded > 3_000_000));
        m.borrow_mut().pause(&[a]);
        assert!(run_until(&m, 20, |m| status(m, a) == Status::Paused
            && m.entry(a).unwrap().dl.is_none()));
        let file = m
            .borrow()
            .entry(a)
            .unwrap()
            .item
            .file
            .clone()
            .expect("named");
        assert!(mdm_engine::naming::part_path(&file).exists(), "part kept");
        let had = m.borrow().entry(a).unwrap().item.downloaded;
        m.borrow_mut().resume(&[a]);
        assert!(run_until(&m, 20, |m| m.entry(a).unwrap().item.downloaded
            > had + 1_000_000));

        // 2. Connections dropped all the time.
        let b = m.borrow_mut().add(
            &format!("{base}/20M/b.bin?drop_after=1M"),
            dl_dir.clone(),
            4,
        );

        // 3. The server goes away for a while (45 s: past the engine's own retries), then
        // comes back: the download goes on where it was.
        let d = m
            .borrow_mut()
            .add(&format!("{base}/8M/d.bin?rate=200K"), dl_dir.clone(), 2);
        assert!(run_until(&m, 30, |m| m.entry(d).unwrap().item.downloaded > 1_000_000));
        srv.kill().ok();
        srv.wait().ok();
        let mut seen = Vec::new();
        let end = Instant::now() + Duration::from_secs(45);
        while Instant::now() < end {
            m.borrow_mut().tick();
            let w = status_words(m.borrow().entry(d).unwrap());
            if seen.last() != Some(&w) && !w.starts_with("Retrying in") || seen.is_empty() {
                seen.push(w);
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        eprintln!(
            "while the server was gone: {seen:?} {:?}",
            m.borrow().entry(d).unwrap().item.error
        );
        let before = m.borrow().entry(d).unwrap().item.downloaded;
        srv = server(&bin, 8091);
        assert!(
            run_until(&m, 60, |m| status(m, d) == Status::Done),
            "back after the outage"
        );
        assert!(before > 1_000_000);

        // 5. A server that refuses a while (every request answered 503): the engine gives up,
        // the app tries again later by itself.
        let f = m
            .borrow_mut()
            .add(&format!("{base}/2M/f.bin?fail_first=40"), dl_dir.clone(), 2);
        let mut words = Vec::new();
        assert!(
            run_until(&m, 400, |m| {
                let w = status_words(m.entry(f).unwrap());
                if words.last() != Some(&w) {
                    words.push(w);
                }
                status(m, f) == Status::Done
            }),
            "retried until it worked: {words:?}"
        );
        eprintln!("refusing server: {words:?}");

        // 4. The app closes mid-download and opens again: it goes on by itself.
        let c = m
            .borrow_mut()
            .add(&format!("{base}/30M/c.bin"), dl_dir.clone(), 4);
        assert!(run_until(&m, 30, |m| m.entry(c).unwrap().item.downloaded > 2_000_000));
        m.borrow_mut().shutdown();
        drop(m);
        let m = Manager::new(s);
        assert_eq!(
            status(&m.borrow(), c),
            Status::Downloading,
            "resumed by itself"
        );

        let ok = run_until(&m, 120, |m| {
            [a, b, c, d].iter().all(|id| status(m, *id) == Status::Done)
        });
        for id in [a, b, c, d] {
            let mm = m.borrow();
            let e = mm.entry(id).unwrap();
            eprintln!(
                "{} {:?} {:?} {:?}",
                e.item.name, e.item.status, e.item.size, e.item.error
            );
        }
        assert!(ok, "all finished");
        for (id, size) in [(a, 40u64 << 20), (b, 20 << 20), (c, 30 << 20), (d, 8 << 20)] {
            let f = m.borrow().entry(id).unwrap().item.file.clone().unwrap();
            assert_eq!(
                std::fs::metadata(&f).unwrap().len(),
                size,
                "{}",
                f.display()
            );
            assert!(!mdm_engine::naming::part_path(&f).exists());
        }
        srv.kill().ok();
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The scheduler starts what waits for it, says what to do once it is done; one download's
    /// own speed limit holds.
    #[test]
    fn scheduler_and_speed_limit() {
        let Ok(bin) = std::env::var("MDM_LABSERVER") else {
            eprintln!("MDM_LABSERVER not set: skipped");
            return;
        };
        let tmp = std::env::temp_dir().join(format!("retriever-sched-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        check_env(&tmp);
        let mut srv = server(&bin, 8092);
        let dir = tmp.join("dl");
        let mut s = Settings {
            folder: dir.clone(),
            system_proxy: false,
            ..Settings::default()
        };
        s.schedule.then = After::Quit;
        let m = Manager::new(s);
        let base = "http://127.0.0.1:8092/gen";
        let x = Extra {
            scheduled: true,
            ..Default::default()
        };
        let a = m
            .borrow_mut()
            .add_with(&format!("{base}/3M/night.bin"), dir.clone(), 4, &x);
        run_until(&m, 2, |_| false);
        assert_eq!(
            status(&m.borrow(), a),
            Status::Paused,
            "waits for the scheduler"
        );
        m.borrow_mut().start_scheduled();
        let mut after = None;
        let end = Instant::now() + Duration::from_secs(30);
        while Instant::now() < end && after.is_none() {
            after = m.borrow_mut().tick().after;
            std::thread::sleep(Duration::from_millis(250));
        }
        assert_eq!(status(&m.borrow(), a), Status::Done);
        assert_eq!(after, Some(After::Quit), "told what to do when done");

        // 256 KB/s of its own, 8 connections or not.
        let b = m
            .borrow_mut()
            .add(&format!("{base}/20M/limited.bin"), dir.clone(), 8);
        m.borrow_mut().set_limit(b, 256);
        let started = Instant::now();
        run_until(&m, 6, |_| false);
        let got = m.borrow().entry(b).unwrap().item.downloaded;
        let rate = got as f64 / started.elapsed().as_secs_f64();
        eprintln!("limited to 256 KB/s: {:.0} KB/s", rate / 1024.0);
        assert!(rate < 400.0 * 1024.0, "over its limit: {rate}");
        m.borrow_mut().set_limit(b, 0);
        assert!(
            run_until(&m, 60, |m| status(m, b) == Status::Done),
            "no limit: done"
        );
        srv.kill().ok();
        let _ = std::fs::remove_dir_all(&tmp);
    }

    fn check_env(tmp: &std::path::Path) {
        // XDG folders of this test only: the list and settings go there.
        let cmd = std::env::var("XDG_DATA_HOME").ok();
        assert!(
            cmd.as_deref()
                .is_some_and(|d| d.starts_with(&*std::env::temp_dir().to_string_lossy())),
            "run with XDG_DATA_HOME in the temp folder ({})",
            tmp.display()
        );
    }
}
