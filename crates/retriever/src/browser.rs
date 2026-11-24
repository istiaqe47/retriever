//! The browsers: the Mister Download Manager extension's native host (`misterdownloadmanager-nmhost`,
//! MDM's own, unchanged) hands its downloads to this app over MDM's command channel
//! (`mdm_ipc`), and asks it for the toolbar button's list of downloads.

use crate::dialogs;
use crate::manager::Extra;
use crate::model::{self, Category, Status};
use crate::window::UiRc;
use gtk::glib;
use gtk::prelude::GtkWindowExt;
use mdm_ipc::api::{self, DownloadInfo, RecentActAnswer, RecentAnswer, recent_why};
use mdm_ipc::{Command, Incoming};
use std::time::Duration;

/// Listens for the browser host, and keeps its rules and registration current.
pub fn start(ui: &UiRc) {
    write_rules(&ui.manager.borrow().settings);
    repair_host();
    let (tx, rx) = std::sync::mpsc::channel::<Incoming>();
    if let Err(e) = mdm_ipc::serve(tx, || {}) {
        eprintln!("retriever: no browser integration: {e}");
        return;
    }
    let ui = ui.clone();
    glib::timeout_add_local(Duration::from_millis(120), move || {
        while let Ok(inc) = rx.try_recv() {
            handle(&ui, inc);
        }
        glib::ControlFlow::Continue
    });
}

/// The extension's settings, where the host reads them (`mdm_ipc::capture::CaptureRules::load`:
/// the data folder's downloads.json): its video button only with yt-dlp installed. Never
/// written over the original app's own list.
pub fn write_rules(s: &crate::model::Settings) {
    let Some(dir) = mdm_ipc::data_dir() else {
        return;
    };
    let path = dir.join("downloads.json");
    if let Ok(bytes) = std::fs::read(&path) {
        let ours = serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .is_some_and(|v| {
                ["retriever", "downpour", "mdm_gtk"]
                    .iter()
                    .any(|k| v.get(k).is_some())
            });
        if !ours {
            eprintln!(
                "retriever: {} isn't Retriever's; its settings decide what the browser hands over",
                path.display()
            );
            return;
        }
    }
    let rules = serde_json::json!({
        "retriever": true,
        "settings": {
            "advanced_integration": s.browser_capture,
            "video_panel": s.video_button && crate::video::ytdlp().is_some(),
            "video_panel_sites": s.video_button_off,
            "video_panel_compact": s.video_button_small,
            "excluded_sites": format!("{} {}", mdm_ipc::capture::DEFAULT_EXCLUDED_SITES, s.browser_sites).trim(),
            "torrent_handler": true,
        }
    });
    if mdm_ipc::private_dir(&dir).is_ok() {
        let _ = mdm_ipc::private_write(&path, rules.to_string().as_bytes());
    }
}

/// The host's registration with each installed browser (a browser installed later, a home
/// folder's manifest gone), as the original app does at every start. Packaged system-wide
/// manifests are left as they are.
fn repair_host() {
    let Some(host) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join("retriever-host")))
        .filter(|h| h.exists())
    else {
        return;
    };
    std::thread::spawn(move || {
        let _ = std::process::Command::new(host)
            .arg("--repair")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    });
}

fn handle(ui: &UiRc, inc: Incoming) {
    let Incoming { cmd: c, reply } = inc;
    let answer = |line: String| {
        if let Some(r) = &reply {
            let _ = r.try_send(line);
        }
    };
    if c.exit {
        if let Some(app) = gtk::gio::Application::default() {
            gtk::gio::prelude::ActionGroupExt::activate_action(&app, "quit", None);
        }
        return;
    }
    if let Some(ask) = &c.recent {
        answer(recent(ui, ask));
        return;
    }
    if let Some(act) = &c.recent_act {
        answer(api::line(&recent_act(ui, &act.act, act.id)));
        return;
    }
    // The extension's "Button settings…" (and its Options): Preferences, at Browsers.
    if c.show.as_deref().is_some_and(|p| p.starts_with("options")) {
        ui.window.present();
        dialogs::preferences_at(ui, Some("browsers"));
        answer("ok".into());
        return;
    }
    // The video button's "Never on <site>" and its "Turn on" again; the popup's "Leave this
    // site to the browser" and its Undo. Saved, and the rules rewritten for the host.
    let sites = [
        (&c.panel_hide_site, true, true),
        (&c.panel_show_site, true, false),
        (&c.exclude_site, false, true),
        (&c.include_site, false, false),
    ];
    if let Some((host, button, add)) = sites
        .iter()
        .find_map(|(h, b, a)| h.as_deref().map(|h| (h, *b, *a)))
    {
        let mut s = ui.manager.borrow().settings.clone();
        if button {
            s.video_button_off = crate::model::site_list(&s.video_button_off, host, add);
        } else {
            s.browser_sites = crate::model::site_list(&s.browser_sites, host, add);
        }
        write_rules(&s);
        ui.manager.borrow_mut().apply_settings(s);
        answer("ok".into());
        return;
    }
    // The video button: its quality menu is left to the Download Video window.
    if c.video_lookup.is_some() || c.video_poll.is_some() || c.video_pick.is_some() {
        answer(api::line(&api::VideoLookupAnswer::of(
            api::lookup::USE_WINDOW,
        )));
        return;
    }
    if let Some(page) = &c.video_page {
        if reply.is_some() {
            answer("ok".into());
        }
        let extra = Extra {
            headers: headers_of(&c, page),
            ..Default::default()
        };
        ui.window.present();
        crate::video::dialog(ui, Some(page.clone()), extra);
        return;
    }
    // A torrent: a .torrent the browser downloaded, or a magnet link.
    if let Some(bytes) = &c.torrent_file {
        if reply.is_some() {
            answer("ok".into());
        }
        ui.window.present();
        let name = c.name.clone().unwrap_or_else(|| "torrent".into());
        crate::torrentdlg::dialog(
            ui,
            Some(crate::torrentdlg::Given::File(bytes.clone(), name)),
        );
        return;
    }
    if let Some(u) = c
        .url
        .as_ref()
        .filter(|u| mdm_torrent::Source::magnet(u).is_some())
    {
        if reply.is_some() {
            answer("ok".into());
        }
        ui.window.present();
        crate::torrentdlg::dialog(ui, Some(crate::torrentdlg::Given::Link(u.clone())));
        return;
    }
    let mut links: Vec<String> = c.links.clone();
    let from_page = !links.is_empty();
    if links.is_empty()
        && let Some(u) = &c.url
    {
        links.push(u.clone());
    }
    links.retain(|l| !dialogs::links_in(l).is_empty());
    if !links.is_empty() {
        let extra = Extra {
            headers: headers_of(&c, &links[0]),
            // The browser's name for one file; a page's links are named by their servers.
            name: (links.len() == 1).then(|| c.name.clone()).flatten(),
            folder: c.dir.as_ref().map(std::path::PathBuf::from),
            later: c.later,
            scheduled: false,
        };
        if reply.is_some() {
            answer("ok".into());
        }
        if c.silent {
            let (folder, conns) = {
                let m = ui.manager.borrow();
                (
                    extra
                        .folder
                        .clone()
                        .unwrap_or_else(|| m.settings.folder.clone()),
                    m.settings.connections,
                )
            };
            {
                let mut m = ui.manager.borrow_mut();
                for l in &links {
                    m.add_with(l, folder.clone(), conns, &extra);
                }
            }
            ui.after_change();
        } else if from_page {
            // A page's or a selection's links: picked in Grab Links.
            ui.window.present();
            crate::grabber::dialog(ui, c.page_url.clone().or(c.referer.clone()), links, extra);
        } else {
            ui.window.present();
            dialogs::add_url_with(ui, Some(links.join("\n")), extra);
        }
        return;
    }
    if reply.is_some() {
        // A question this edition has no answer for: the host says the app is older.
        answer(error_line("not in the GTK edition"));
        return;
    }
    // A plain start, or "show" something: the window.
    if !c.startup && !c.hidden {
        ui.window.present();
    }
}

fn error_line(why: &str) -> String {
    api::line(&api::ErrorAnswer {
        error: why.into(),
        code: None,
    })
}

/// The headers the browser sent for the link: its page (Referer), its cookies, its agent.
fn headers_of(c: &Command, link: &str) -> Vec<(String, String)> {
    let mut h = Vec::new();
    if let Some(r) = c.referer.as_ref().filter(|r| !r.is_empty()) {
        h.push(("Referer".into(), r.clone()));
    }
    if let Some(ua) = c.user_agent.as_ref().filter(|u| !u.is_empty()) {
        h.push(("User-Agent".into(), ua.clone()));
    }
    let cookies = match c.cookies.as_ref().filter(|c| !c.is_empty()) {
        Some(s) => Some(s.clone()),
        None => cookie_header(&c.cookie_jar, link),
    };
    if let Some(cookies) = cookies {
        h.push(("Cookie".into(), cookies));
    }
    h
}

/// The jar's cookies the browser would send to `link` (its host and path, HTTPS for secure).
fn cookie_header(jar: &[mdm_ipc::BrowserCookie], link: &str) -> Option<String> {
    let u = url::Url::parse(link).ok()?;
    let host = u.host_str()?.to_ascii_lowercase();
    let https = u.scheme() == "https";
    let path = u.path();
    let now = model::now() as f64;
    let sent: Vec<String> = jar
        .iter()
        .filter(|k| {
            let d = k.domain.trim_start_matches('.').to_ascii_lowercase();
            let host_ok = if k.host_only {
                host == d
            } else {
                host == d || host.ends_with(&format!(".{d}"))
            };
            host_ok
                && (k.path.is_empty() || path.starts_with(&k.path))
                && (!k.secure || https)
                && k.expires.is_none_or(|t| t > now)
        })
        .map(|k| format!("{}={}", k.name, k.value))
        .collect();
    (!sent.is_empty()).then(|| sent.join("; "))
}

fn status_word(s: Status) -> &'static str {
    match s {
        Status::Queued => "Queued",
        Status::Downloading => "Downloading",
        Status::Paused => "Stopped",
        Status::Done => "Complete",
        Status::Failed => "Error",
    }
}

/// The toolbar button's list (#1496 of MDM): the newest downloads, as the popup asks.
fn recent(ui: &UiRc, ask: &mdm_ipc::RecentAsk) -> String {
    let m = ui.manager.borrow();
    let list: Vec<DownloadInfo> = m
        .entries
        .iter()
        .map(|e| {
            let it = &e.item;
            let running = it.status == Status::Downloading;
            let speed = if running { e.speed as u64 } else { 0 };
            DownloadInfo {
                id: it.id,
                file: it.name.clone(),
                url: it.url.clone(),
                path: it
                    .file
                    .clone()
                    .unwrap_or_else(|| it.folder.clone())
                    .display()
                    .to_string(),
                status: status_word(it.status).into(),
                size: it.size,
                downloaded: it.downloaded,
                speed,
                added: Some(model::date_text(it.added)),
                category: match it.category() {
                    Category::Other => "General".into(),
                    c => c.title().into(),
                },
                error: it.error.clone().filter(|_| it.status == Status::Failed),
                left: match it.size {
                    Some(t) if running && speed > 0 => {
                        Some(t.saturating_sub(it.downloaded) / speed)
                    }
                    _ => None,
                },
                exists: (it.status == Status::Done)
                    .then(|| it.file.as_ref().is_some_and(|f| f.exists())),
                finished: it.finished,
                kind: None,
                redownload: !running,
                icon: None,
            }
        })
        .collect();
    let speed = m.total_speed() as u64;
    api::line(&RecentAnswer::of(list, ask, speed))
}

/// One of the popup's buttons on one download.
fn recent_act(ui: &UiRc, act: &str, id: u64) -> RecentActAnswer {
    if act == "clear-done" {
        let ids = ui
            .manager
            .borrow()
            .ids_where(|e| e.item.status == Status::Done);
        ui.manager.borrow_mut().remove(&ids, false);
        ui.after_change();
        return RecentActAnswer::done();
    }
    let Some((status, file)) = ui
        .manager
        .borrow()
        .entry(id)
        .map(|e| (e.item.status, e.item.file.clone()))
    else {
        return RecentActAnswer::not(recent_why::GONE);
    };
    let running = matches!(status, Status::Downloading | Status::Queued);
    let exists = file.as_ref().is_some_and(|f| f.exists());
    let answer = match act {
        "open" if status != Status::Done => RecentActAnswer::not(recent_why::CANT),
        "open" if !exists => RecentActAnswer::not(recent_why::MISSING),
        "open" => {
            ui.open_file(id);
            RecentActAnswer::done()
        }
        "folder" => {
            ui.open_folder(id);
            RecentActAnswer::done()
        }
        "copy-file" | "share" => RecentActAnswer::not(recent_why::UNSUPPORTED),
        "pause" if !running => RecentActAnswer::not(recent_why::CANT),
        "pause" => {
            ui.manager.borrow_mut().pause(&[id]);
            RecentActAnswer::done()
        }
        "resume" if !matches!(status, Status::Paused | Status::Failed) => {
            RecentActAnswer::not(recent_why::CANT)
        }
        "resume" => {
            ui.manager.borrow_mut().resume(&[id]);
            RecentActAnswer::done()
        }
        "redownload" | "remove" | "delete" if running => RecentActAnswer::not(recent_why::RUNNING),
        "redownload" => {
            ui.manager.borrow_mut().restart(&[id]);
            RecentActAnswer::done()
        }
        "remove" | "delete" => {
            ui.manager.borrow_mut().remove(&[id], act == "delete");
            RecentActAnswer::done()
        }
        _ => RecentActAnswer::not(recent_why::UNKNOWN),
    };
    if answer.done {
        ui.after_change();
    }
    answer
}
