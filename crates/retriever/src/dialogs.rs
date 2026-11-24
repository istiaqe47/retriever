//! Add URL, a download's progress window, deleting, preferences and about.

use crate::manager::{self, receiving};
use crate::model::{self, Status};
use crate::window::{UiRc, apply_style};
use adw::prelude::*;
use gtk::{gio, glib};
use mdm_engine::download::{ConnStatus, State};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

/// The links in a text: one per line or separated by spaces, http(s) and ftp(s) only.
pub fn links_in(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter_map(|w| {
            url::Url::parse(w.trim_matches(|c| c == '"' || c == '\'' || c == '<' || c == '>')).ok()
        })
        .filter(mdm_engine::supported)
        .map(|u| u.to_string())
        .collect()
}

pub fn add_url(ui: &UiRc, text: Option<String>) {
    add_url_with(ui, text, manager::Extra::default());
}

/// Add URL with what came with the link (a browser's: its headers, the file's name, a folder).
pub fn add_url_with(ui: &UiRc, text: Option<String>, extra: manager::Extra) {
    let (folder, connections) = {
        let m = ui.manager.borrow();
        (
            extra
                .folder
                .clone()
                .unwrap_or_else(|| m.settings.folder.clone()),
            m.settings.connections,
        )
    };
    let dialog = adw::Dialog::builder()
        .title("Add URL")
        .content_width(520)
        .build();

    let cancel = gtk::Button::with_mnemonic("_Cancel");
    let start = gtk::Button::with_mnemonic("_Download");
    start.add_css_class("suggested-action");
    start.set_sensitive(false);
    let header = adw::HeaderBar::builder()
        .show_start_title_buttons(false)
        .show_end_title_buttons(false)
        .build();
    header.pack_start(&cancel);
    header.pack_end(&start);

    let address = adw::EntryRow::builder().title("Address").build();
    let found = gtk::Label::new(None);
    found.add_css_class("dim-label");
    found.add_css_class("caption");
    address.add_suffix(&found);

    let folder = Rc::new(RefCell::new(folder));
    let save_row = adw::ActionRow::builder()
        .title("Save To")
        .subtitle(folder.borrow().display().to_string())
        .activatable(true)
        .build();
    save_row.add_prefix(&gtk::Image::from_icon_name("folder-symbolic"));
    save_row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    {
        let folder = folder.clone();
        let dialog = dialog.clone();
        save_row.connect_activated(move |row| {
            let chooser = gtk::FileDialog::builder()
                .title("Save Downloads To")
                .initial_folder(&gio::File::for_path(&*folder.borrow()))
                .build();
            let folder = folder.clone();
            let row = row.clone();
            let parent = dialog.root().and_downcast::<gtk::Window>();
            chooser.select_folder(parent.as_ref(), gio::Cancellable::NONE, move |r| {
                if let Ok(f) = r
                    && let Some(p) = f.path()
                {
                    row.set_subtitle(&p.display().to_string());
                    *folder.borrow_mut() = p;
                }
            });
        });
    }
    let conns = adw::SpinRow::with_range(1.0, 32.0, 1.0);
    conns.set_title("Connections");
    conns.set_subtitle("Parts downloaded at the same time");
    conns.set_value(connections as f64);

    // The browser's name for the file, to change if wanted.
    let name_row = adw::EntryRow::builder().title("File Name").build();
    if let Some(n) = &extra.name {
        name_row.set_text(n);
    }
    name_row.set_visible(extra.name.is_some());

    let group = adw::PreferencesGroup::new();
    group.add(&address);
    group.add(&name_row);
    group.add(&save_row);
    group.add(&conns);
    let when = adw::ComboRow::builder()
        .title("Start")
        .model(&gtk::StringList::new(&[
            "Now",
            "Later (Paused)",
            "At the Scheduled Time",
        ]))
        .selected(if extra.scheduled {
            2
        } else if extra.later {
            1
        } else {
            0
        })
        .build();
    group.add(&when);
    let page = adw::PreferencesPage::new();
    page.add(&group);

    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&page));
    dialog.set_child(Some(&view));

    let check = {
        let start = start.clone();
        let found = found.clone();
        move |text: &str| {
            let n = links_in(text).len();
            start.set_sensitive(n > 0);
            found.set_label(&match n {
                0 | 1 => String::new(),
                n => format!("{n} links"),
            });
        }
    };
    {
        let check = check.clone();
        address.connect_changed(move |e| check(&e.text()));
    }
    if let Some(t) = text {
        address.set_text(t.trim());
    } else {
        // A link on the clipboard is offered.
        let address = address.clone();
        let clip = ui.window.clipboard();
        glib::spawn_future_local(async move {
            if let Ok(Some(t)) = clip.read_text_future().await
                && !links_in(&t).is_empty()
                && address.text().is_empty()
            {
                address.set_text(t.trim());
                address.grab_focus();
            }
        });
    }

    let go = {
        let ui = ui.clone();
        let dialog = dialog.clone();
        let address = address.clone();
        move || {
            let links = links_in(&address.text());
            if links.is_empty() {
                return;
            }
            let folder = folder.borrow().clone();
            let n = conns.value() as u32;
            let mut x = extra.clone();
            x.later = when.selected() == 1;
            x.scheduled = when.selected() == 2;
            if x.name.is_some() {
                let typed = name_row.text().trim().to_owned();
                x.name = (!typed.is_empty()).then_some(typed);
            }
            {
                let mut m = ui.manager.borrow_mut();
                for l in &links {
                    m.add_with(l, folder.clone(), n, &x);
                }
            }
            ui.after_change();
            dialog.close();
        }
    };
    {
        let go = go.clone();
        start.connect_clicked(move |_| go());
    }
    address.connect_entry_activated(move |_| go());
    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| {
            dialog.close();
        });
    }
    dialog.set_default_widget(Some(&start));
    dialog.set_focus(Some(&address));
    dialog.present(Some(&ui.window));
}

pub fn confirm_delete(ui: &UiRc, ids: Vec<u64>) {
    let n = ids.len();
    let (heading, body) = if n == 1 {
        let name = ui
            .manager
            .borrow()
            .entry(ids[0])
            .map(|e| e.item.name.clone())
            .unwrap_or_default();
        (
            "Delete Download?".to_owned(),
            format!(
                "“{name}” is removed from the list. Unfinished downloads lose what they downloaded so far."
            ),
        )
    } else {
        (
            format!("Delete {n} Downloads?"),
            "They are removed from the list. Unfinished downloads lose what they downloaded so far."
                .to_owned(),
        )
    };
    let d = adw::AlertDialog::new(Some(&heading), Some(&body));
    d.add_responses(&[
        ("cancel", "_Cancel"),
        ("remove", "_Remove from List"),
        ("delete", "_Delete Files Too"),
    ]);
    d.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    d.set_default_response(Some("remove"));
    d.set_close_response("cancel");
    let ui2 = ui.clone();
    d.connect_response(None, move |_, r| {
        let files = match r {
            "remove" => false,
            "delete" => true,
            _ => return,
        };
        ui2.manager.borrow_mut().remove(&ids, files);
        ui2.after_change();
    });
    d.present(Some(&ui.window));
}

thread_local! {
    static PROGRESS: RefCell<HashMap<u64, adw::Dialog>> = RefCell::new(HashMap::new());
}

struct ProgressRows {
    percent: gtk::Label,
    headline: gtk::Label,
    size: gtk::Label,
    done: gtk::Label,
    rate: gtk::Label,
    eta: gtk::Label,
    resume: gtk::Label,
    folder: adw::ActionRow,
    segs: gtk::DrawingArea,
    conns: adw::PreferencesGroup,
    conn_rows: RefCell<Vec<adw::ActionRow>>,
    pause: gtk::Button,
    open: gtk::Button,
    extracted: adw::ActionRow,
}

fn value_row(group: &adw::PreferencesGroup, title: &str) -> gtk::Label {
    let row = adw::ActionRow::builder().title(title).build();
    let l = gtk::Label::builder()
        .xalign(1.0)
        .selectable(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    l.add_css_class("numeric");
    row.add_suffix(&l);
    group.add(&row);
    l
}

/// A download's dialog (a sheet over the main window, as GNOME's own): its numbers, its parts and connections, pause and open.
pub fn progress(ui: &UiRc, id: u64) {
    if PROGRESS.with(|p| p.borrow().contains_key(&id)) {
        return;
    }
    let Some(url) = ui.manager.borrow().entry(id).map(|e| e.item.url.clone()) else {
        return;
    };
    let title = adw::WindowTitle::new("", "");
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&title));

    let info = adw::PreferencesGroup::new();
    let addr = adw::ActionRow::builder()
        .title("Address")
        .subtitle(&url)
        .subtitle_selectable(true)
        .build();
    addr.add_css_class("property");
    info.add(&addr);
    let size = value_row(&info, "File Size");
    let done = value_row(&info, "Downloaded");
    let rate = value_row(&info, "Transfer Rate");
    let eta = value_row(&info, "Time Left");
    let resume = value_row(&info, "Resume Capability");
    let limit = adw::SpinRow::with_range(0.0, 1_000_000.0, 64.0);
    limit.set_title("Speed Limit");
    limit.set_subtitle("KB/s for this download; 0 is none");
    limit.set_value(
        ui.manager
            .borrow()
            .entry(id)
            .map_or(0, |e| e.item.limit_kib) as f64,
    );
    info.add(&limit);
    {
        let ui = ui.clone();
        limit.connect_value_notify(move |r| {
            ui.manager.borrow_mut().set_limit(id, r.value() as u64);
        });
    }
    let extracted = adw::ActionRow::builder()
        .title("Archive")
        .subtitle_selectable(true)
        .visible(false)
        .build();
    extracted.add_css_class("property");
    let folder = adw::ActionRow::builder()
        .title("Saved In")
        .subtitle_selectable(true)
        .build();
    folder.add_css_class("property");
    info.add(&folder);
    info.add(&extracted);

    // At the top: how far it is, in big, and its parts.
    let percent = gtk::Label::builder().xalign(0.0).build();
    percent.add_css_class("title-1");
    percent.add_css_class("numeric");
    let headline = gtk::Label::builder()
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    headline.add_css_class("dim-label");
    let segs = gtk::DrawingArea::builder()
        .content_height(14)
        .margin_top(6)
        .tooltip_text(
            "Each part of the file and how far it came; a mark where a connection's part starts",
        )
        .build();
    segs.add_css_class("accent");
    segs.update_property(&[gtk::accessible::Property::Label(
        "The file's parts and how far each has come",
    )]);
    let hero = gtk::Box::new(gtk::Orientation::Vertical, 4);
    hero.append(&percent);
    hero.append(&headline);
    hero.append(&segs);
    let top = adw::PreferencesGroup::new();
    top.add(&hero);

    let pause = gtk::Button::builder().label("Pause").build();
    pause.add_css_class("pill");
    let open = gtk::Button::builder().label("Open").build();
    open.add_css_class("pill");
    open.add_css_class("suggested-action");
    let buttons = gtk::Box::builder()
        .spacing(12)
        .halign(gtk::Align::Center)
        .margin_top(12)
        .margin_bottom(12)
        .build();
    buttons.append(&pause);
    buttons.append(&open);

    let open_folder = gtk::Button::builder()
        .icon_name("folder-open-symbolic")
        .tooltip_text("Open Containing Folder")
        .build();
    header.pack_start(&open_folder);

    let conns = adw::PreferencesGroup::builder()
        .title("Connections")
        .build();

    let page = adw::PreferencesPage::new();
    page.add(&top);
    page.add(&info);
    page.add(&conns);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&page));
    view.add_bottom_bar(&buttons);

    let win = adw::Dialog::builder()
        .content_width(500)
        .content_height(640)
        .child(&view)
        .build();

    let rows = Rc::new(ProgressRows {
        percent,
        headline,
        size,
        done,
        rate,
        eta,
        resume,
        folder,
        segs: segs.clone(),
        conns,
        conn_rows: RefCell::new(Vec::new()),
        pause: pause.clone(),
        open: open.clone(),
        extracted: extracted.clone(),
    });

    // The parts: each one's span of the file, filled as far as it came.
    {
        let ui = ui.clone();
        segs.set_draw_func(move |area, cr, w, h| {
            let (w, h) = (w as f64, h as f64);
            // The track in the text's colour, faint; the parts in the system's accent
            // (the area has the `accent` style class, whose colour is the accent's).
            let fg = area.parent().map_or_else(|| area.color(), |p| p.color());
            let ac = area.color();
            let accent = (ac.red() as f64, ac.green() as f64, ac.blue() as f64);
            rounded(cr, 0.0, 0.0, w, h, h / 2.0);
            cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.12);
            let _ = cr.fill_preserve();
            cr.clip();
            let m = ui.manager.borrow();
            let Some(e) = m.entry(id) else { return };
            cr.set_source_rgb(accent.0, accent.1, accent.2);
            match (&e.snap, e.item.size) {
                (Some(s), Some(total)) if total > 0 && !s.segments.is_empty() => {
                    let x = |b: u64| b as f64 / total as f64 * w;
                    // Done is everything outside the parts left.
                    let mut left: Vec<(u64, u64)> = s
                        .segments
                        .iter()
                        .map(|g| (g.cursor.min(g.end), g.end))
                        .collect();
                    left.sort();
                    let mut at = 0u64;
                    for (a, b) in &left {
                        if *a > at {
                            cr.rectangle(x(at), 0.0, x(*a) - x(at), h);
                        }
                        at = at.max(*b);
                    }
                    if at < total {
                        cr.rectangle(x(at), 0.0, w - x(at), h);
                    }
                    let _ = cr.fill();
                    // Where each part starts.
                    cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.55);
                    for g in &s.segments {
                        cr.rectangle(x(g.start).max(0.0), 0.0, 1.5, h);
                    }
                    let _ = cr.fill();
                }
                _ => {
                    let f = manager::fraction(&e.item);
                    cr.rectangle(0.0, 0.0, w * f, h);
                    let _ = cr.fill();
                }
            }
        });
    }
    {
        let ui = ui.clone();
        pause.connect_clicked(move |_| {
            let st = ui.manager.borrow().entry(id).map(|e| e.item.status);
            match st {
                Some(Status::Downloading | Status::Queued) => ui.manager.borrow_mut().pause(&[id]),
                Some(Status::Paused | Status::Failed) => ui.manager.borrow_mut().resume(&[id]),
                Some(Status::Done) => {}
                None => return,
            }
            ui.after_change();
        });
    }
    {
        let ui = ui.clone();
        open.connect_clicked(move |_| ui.open_file(id));
    }
    {
        let ui = ui.clone();
        open_folder.connect_clicked(move |_| ui.open_folder(id));
    }

    let update = {
        let ui = ui.clone();
        let rows = rows.clone();
        let title = title.clone();
        let win = win.clone();
        move || -> bool {
            let m = ui.manager.borrow();
            let Some(e) = m.entry(id) else {
                win.close();
                return false;
            };
            let it = &e.item;
            title.set_title(&it.name);
            win.set_title(&it.name);
            rows.percent.set_label(&match (it.status, it.size) {
                (Status::Done, _) => "Complete".to_owned(),
                (_, Some(t)) if t > 0 => format!("{:.1}%", manager::fraction(it) * 100.0),
                _ => model::size_text(it.downloaded),
            });
            // A failure in its own words (the whole of it as the tooltip).
            rows.headline.set_label(&match (&it.error, it.status) {
                (Some(err), Status::Failed) => format!("{}: {err}", manager::status_words(e)),
                _ => detailed_status(e),
            });
            rows.headline.set_tooltip_text(it.error.as_deref());
            rows.size.set_label(
                &it.size
                    .map(model::size_text)
                    .unwrap_or_else(|| "Unknown".into()),
            );
            rows.done.set_label(&match it.size {
                Some(t) if t > 0 => format!(
                    "{} ({:.2}%)",
                    model::size_text(it.downloaded),
                    it.downloaded as f64 / t as f64 * 100.0
                ),
                _ => model::size_text(it.downloaded),
            });
            let downloading = it.status == Status::Downloading;
            rows.rate.set_label(&if downloading {
                model::speed_text(e.speed)
            } else {
                String::new()
            });
            rows.eta.set_label(&match it.size {
                Some(t) if downloading && e.speed > 1.0 => {
                    model::eta_text(t.saturating_sub(it.downloaded) as f64 / e.speed)
                }
                _ => String::new(),
            });
            rows.extracted.set_visible(it.extracted.is_some());
            rows.extracted
                .set_subtitle(it.extracted.as_deref().unwrap_or_default());
            rows.resume.set_label(match it.resumable {
                Some(true) => "Yes",
                Some(false) => "No",
                None => "Unknown",
            });
            rows.folder.set_subtitle(
                &it.file
                    .as_ref()
                    .and_then(|f| f.parent().map(|p| p.to_path_buf()))
                    .unwrap_or_else(|| it.folder.clone())
                    .display()
                    .to_string(),
            );
            rows.segs.queue_draw();
            rows.pause.set_label(match it.status {
                Status::Paused | Status::Failed => "Resume",
                _ => "Pause",
            });
            rows.pause.set_visible(it.status != Status::Done);
            rows.open.set_visible(it.status == Status::Done);

            // One row per connection.
            let conns = e
                .snap
                .as_ref()
                .map(|s| s.connections.clone())
                .unwrap_or_default();
            rows.conns.set_visible(!conns.is_empty());
            let mut cr = rows.conn_rows.borrow_mut();
            while cr.len() < conns.len() {
                let r = adw::ActionRow::new();
                r.add_css_class("property");
                rows.conns.add(&r);
                cr.push(r);
            }
            for (i, r) in cr.iter().enumerate() {
                match conns.get(i) {
                    Some(c) => {
                        r.set_visible(true);
                        r.set_title(&format!(
                            "{}. {}",
                            i + 1,
                            match &c.status {
                                ConnStatus::Connecting => "Connecting…".to_owned(),
                                ConnStatus::Receiving => "Receiving data…".to_owned(),
                                ConnStatus::Retrying(err) => format!("Retrying: {err}"),
                                ConnStatus::Finished => "Finished".to_owned(),
                                ConnStatus::Failed(err) => format!("Failed: {err}"),
                                ConnStatus::Retired { cap, .. } =>
                                    format!("Closed (the server allows {cap})"),
                            }
                        ));
                        r.set_subtitle(&format!(
                            "{}{}",
                            model::size_text(c.downloaded),
                            if c.speed > 0 {
                                format!(" · {}", model::speed_text(c.speed as f64))
                            } else {
                                String::new()
                            }
                        ));
                    }
                    None => r.set_visible(false),
                }
            }
            true
        }
    };
    update();
    let weak = win.downgrade();
    glib::timeout_add_local(Duration::from_millis(500), move || {
        if weak.upgrade().is_none() || !update() {
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
    win.connect_closed(move |_| {
        PROGRESS.with(|p| p.borrow_mut().remove(&id));
    });
    PROGRESS.with(|p| p.borrow_mut().insert(id, win.clone()));
    win.present(Some(&ui.window));
}

fn detailed_status(e: &manager::Entry) -> String {
    match (e.item.status, e.snap.as_ref()) {
        (Status::Downloading, Some(s)) if s.state == State::Downloading => {
            let n = receiving(s);
            format!(
                "Receiving data on {n} connection{}",
                if n == 1 { "" } else { "s" }
            )
        }
        _ => manager::status_words(e),
    }
}

fn rounded(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    use std::f64::consts::PI;
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.close_path();
}

pub fn preferences(ui: &UiRc) {
    preferences_at(ui, None);
}

/// Preferences, opened at one of its pages (`browsers`), or at General.
pub fn preferences_at(ui: &UiRc, at: Option<&str>) {
    // Already open (the extension's button asked twice): that one, at the page asked for.
    if let Some(open) = ui
        .window
        .visible_dialog()
        .and_downcast::<adw::PreferencesDialog>()
    {
        if let Some(at) = at {
            open.set_visible_page_name(at);
        }
        return;
    }
    let s = ui.manager.borrow().settings.clone();
    let settings = Rc::new(RefCell::new(s));
    let dialog = adw::PreferencesDialog::new();
    dialog.set_title("Preferences");
    let page = adw::PreferencesPage::builder()
        .title("General")
        .icon_name("preferences-system-symbolic")
        .build();

    // Downloads
    let g = adw::PreferencesGroup::builder().title("Downloads").build();
    let folder_row = adw::ActionRow::builder()
        .title("Download Folder")
        .subtitle(settings.borrow().folder.display().to_string())
        .activatable(true)
        .build();
    folder_row.add_suffix(&gtk::Image::from_icon_name("folder-open-symbolic"));
    {
        let settings = settings.clone();
        let dialog = dialog.clone();
        folder_row.connect_activated(move |row| {
            let chooser = gtk::FileDialog::builder()
                .title("Download Folder")
                .initial_folder(&gio::File::for_path(&settings.borrow().folder))
                .build();
            let settings = settings.clone();
            let row = row.clone();
            let parent = dialog.root().and_downcast::<gtk::Window>();
            chooser.select_folder(parent.as_ref(), gio::Cancellable::NONE, move |r| {
                if let Ok(f) = r
                    && let Some(p) = f.path()
                {
                    row.set_subtitle(&p.display().to_string());
                    settings.borrow_mut().folder = p;
                }
            });
        });
    }
    g.add(&folder_row);
    let conns = adw::SpinRow::with_range(1.0, 32.0, 1.0);
    conns.set_title("Connections per Download");
    conns.set_subtitle("More connections are faster, if the server allows them");
    conns.set_value(settings.borrow().connections as f64);
    g.add(&conns);
    let simult = adw::SpinRow::with_range(1.0, 20.0, 1.0);
    simult.set_title("Downloads at Once");
    simult.set_subtitle("The others wait in the queue");
    simult.set_value(settings.borrow().simultaneous as f64);
    g.add(&simult);
    let extract = adw::SwitchRow::builder()
        .title("Extract Archives When Done")
        .subtitle("zip, tar, gz and xz; 7z and RAR with 7-Zip installed")
        .active(settings.borrow().extract)
        .build();
    g.add(&extract);
    let forget = adw::SwitchRow::builder()
        .title("Remove Finished Downloads")
        .subtitle("Clear them from the list as soon as they finish; their files stay")
        .active(settings.borrow().forget_finished)
        .build();
    g.add(&forget);
    {
        // On: the finished ones in the list go now.
        let ui = ui.clone();
        forget.connect_active_notify(move |r| {
            if !r.is_active() {
                return;
            }
            let n = ui.manager.borrow_mut().clear_finished();
            ui.after_change();
            if n > 0 {
                ui.toast(&format!(
                    "{n} finished download{} cleared from the list",
                    if n == 1 { "" } else { "s" }
                ));
            }
        });
    }
    page.add(&g);

    // Network
    // Videos and torrents
    let g = adw::PreferencesGroup::builder()
        .title("Videos and Torrents")
        .build();
    let found = |name: &str, p: Option<std::path::PathBuf>, how: &str| {
        let row = adw::ActionRow::builder().title(name).build();
        row.add_css_class("property");
        row.set_subtitle(&match p {
            Some(p) => format!("Found: {}", p.display()),
            None => format!("Not installed: {how}"),
        });
        row
    };
    g.add(&found(
        "yt-dlp",
        crate::video::ytdlp(),
        "sudo pacman -S yt-dlp",
    ));
    g.add(&found(
        "ffmpeg",
        crate::video::ffmpeg(),
        "sudo pacman -S ffmpeg",
    ));
    let seed = adw::SwitchRow::builder()
        .title("Keep Sharing Finished Torrents")
        .subtitle("Off, a torrent stops once its files are in")
        .active(settings.borrow().seed)
        .build();
    g.add(&seed);
    page.add(&g);

    let g = adw::PreferencesGroup::builder().title("Network").build();
    let limit = adw::SpinRow::with_range(0.0, 1_000_000.0, 64.0);
    limit.set_title("Speed Limit");
    limit.set_subtitle("KB/s for all downloads together; 0 is unlimited");
    limit.set_value(settings.borrow().speed_limit as f64);
    g.add(&limit);
    let proxy = adw::SwitchRow::builder()
        .title("Use System Proxy")
        .subtitle("The proxy in the environment (https_proxy, http_proxy, no_proxy)")
        .active(settings.borrow().system_proxy)
        .build();
    g.add(&proxy);
    page.add(&g);


    // Appearance
    let g = adw::PreferencesGroup::builder().title("Appearance").build();
    let style = adw::ComboRow::builder()
        .title("Style")
        .model(&gtk::StringList::new(&["Follow System", "Light", "Dark"]))
        .selected(settings.borrow().style.min(2))
        .build();
    g.add(&style);
    let notify = adw::SwitchRow::builder()
        .title("Notifications")
        .subtitle("Say when a download finishes or fails")
        .active(settings.borrow().notify)
        .build();
    g.add(&notify);
    page.add(&g);
    // Browsers: a page of its own (the extension's "Button settings…" opens it).
    let bpage = adw::PreferencesPage::builder()
        .title("Browsers")
        .name("browsers")
        .icon_name("web-browser-symbolic")
        .build();
    let g = adw::PreferencesGroup::builder()
        .title("Browser Integration")
        .description("With Retriever's extension in your browser, its downloads come here")
        .build();
    let capture = adw::SwitchRow::builder()
        .title("Catch Browser Downloads")
        .subtitle("Firefox, Chrome, Chromium, Brave, Vivaldi, Edge, Zen, LibreWolf…")
        .active(settings.borrow().browser_capture)
        .build();
    g.add(&capture);
    // How to add Retriever's own extension: the page it comes with.
    let help = adw::ActionRow::builder()
        .title("Install the Browser Extension")
        .subtitle("For Firefox, Chrome and the browsers built on them")
        .activatable(true)
        .build();
    help.add_suffix(&gtk::Image::from_icon_name("adw-external-link-symbolic"));
    {
        let dialog = dialog.clone();
        help.connect_activated(move |_| {
            let page = gio::File::for_path("/usr/share/retriever/extension/chromium/help.html");
            let parent = dialog.root().and_downcast::<gtk::Window>();
            gtk::UriLauncher::new(&format!("{}#chromium", page.uri())).launch(
                parent.as_ref(),
                gio::Cancellable::NONE,
                |_| {},
            );
        });
    }
    g.add(&help);
    // Browsers the helper can't reach (Zen, Flatpak browsers…) come in through the local door.
    let door = adw::SwitchRow::builder()
        .title("Connect Other Browsers")
        .subtitle("Browsers that can't start Retriever's helper, like Zen or Flatpak browsers, connect while Retriever is open")
        .active(settings.borrow().local_door)
        .build();
    door.connect_active_notify(|r| crate::localdoor::set_on(r.is_active()));
    g.add(&door);
    let known = crate::localdoor::connected();
    let paired = adw::ActionRow::builder()
        .title("Connected Browsers")
        .subtitle(if known.is_empty() { "None yet".to_owned() } else { known.join(", ") })
        .build();
    let forget_keys = gtk::Button::builder()
        .label("_Forget")
        .use_underline(true)
        .valign(gtk::Align::Center)
        .sensitive(!known.is_empty())
        .build();
    {
        let paired = paired.clone();
        forget_keys.connect_clicked(move |b| {
            crate::localdoor::forget_all();
            paired.set_subtitle("None yet");
            b.set_sensitive(false);
        });
    }
    paired.add_suffix(&forget_keys);
    door.bind_property("active", &paired, "sensitive").sync_create().build();
    g.add(&paired);
    let browser_sites = adw::EntryRow::builder()
        .title("Sites Left to the Browser")
        .text(settings.borrow().browser_sites.as_str())
        .build();
    browser_sites.set_tooltip_text(Some(
        "Separated by spaces. A name takes its subdomains too; * matches anything",
    ));
    g.add(&browser_sites);
    bpage.add(&g);

    let have_ytdlp = crate::video::ytdlp().is_some();
    let g = adw::PreferencesGroup::builder()
        .title("Download Button on Videos")
        .description(if have_ytdlp {
            "Shown over videos in the browser: one click downloads, its arrow lists the qualities"
        } else {
            "Needs yt-dlp: sudo pacman -S yt-dlp"
        })
        .build();
    let video_button = adw::SwitchRow::builder()
        .title("Show the Button")
        .active(settings.borrow().video_button)
        .sensitive(have_ytdlp)
        .build();
    g.add(&video_button);
    let video_small = adw::SwitchRow::builder()
        .title("Small Button")
        .subtitle("Only its icon, over every video")
        .active(settings.borrow().video_button_small)
        .build();
    g.add(&video_small);
    let video_off = adw::EntryRow::builder()
        .title("Sites Without the Button")
        .text(settings.borrow().video_button_off.as_str())
        .build();
    video_off.set_tooltip_text(Some(
        "Separated by spaces. \"Never on this site\" in the button's menu adds one here",
    ));
    g.add(&video_off);
    video_button
        .bind_property("active", &video_small, "sensitive")
        .sync_create()
        .build();
    video_button
        .bind_property("active", &video_off, "sensitive")
        .sync_create()
        .build();
    if !have_ytdlp {
        video_small.set_sensitive(false);
        video_off.set_sensitive(false);
    }
    bpage.add(&g);

    dialog.add(&page);
    dialog.add(&bpage);
    if let Some(at) = at {
        dialog.set_visible_page_name(at);
    }

    {
        let settings = settings.clone();
        style.connect_selected_notify(move |c| {
            settings.borrow_mut().style = c.selected();
            apply_style(c.selected());
        });
    }
    let ui2 = ui.clone();
    dialog.connect_closed(move |_| {
        let mut s = settings.borrow().clone();
        s.connections = conns.value() as u32;
        s.simultaneous = simult.value() as u32;
        s.speed_limit = limit.value() as u64;
        s.system_proxy = proxy.is_active();
        s.notify = notify.is_active();
        s.browser_capture = capture.is_active();
        s.extract = extract.is_active();
        s.seed = seed.is_active();
        s.forget_finished = forget.is_active();
        s.local_door = door.is_active();
        s.video_button = video_button.is_active();
        s.video_button_small = video_small.is_active();
        let tidy = |t: glib::GString| t.split_whitespace().collect::<Vec<_>>().join(" ");
        s.video_button_off = tidy(video_off.text());
        s.browser_sites = tidy(browser_sites.text());
        crate::browser::write_rules(&s);
        ui2.manager.borrow_mut().apply_settings(s);
        ui2.after_change();
    });
    dialog.present(Some(&ui.window));
}

pub fn about(ui: &UiRc) {
    let a = adw::AboutDialog::builder()
        .application_name("Retriever")
        .application_icon(model::APP_ID)
        .version(env!("CARGO_PKG_VERSION"))
        .developer_name("Retriever contributors")
        .comments("Download files, videos and torrents")
        .copyright("Based on Mister Download Manager, GPL-3.0")
        .license_type(gtk::License::Gpl30)
        .build();
    a.present(Some(&ui.window));
}

/// Two spin buttons for a time of day (minutes after midnight).
fn time_picker(minutes: u32) -> (gtk::Box, gtk::SpinButton, gtk::SpinButton) {
    let two = |s: &gtk::SpinButton| {
        s.connect_output(|s| {
            s.set_text(&format!("{:02}", s.value() as u32));
            glib::Propagation::Stop
        });
    };
    let h = gtk::SpinButton::with_range(0.0, 23.0, 1.0);
    let m = gtk::SpinButton::with_range(0.0, 59.0, 1.0);
    for s in [&h, &m] {
        s.set_wrap(true);
        s.set_valign(gtk::Align::Center);
        s.set_numeric(true);
        two(s);
    }
    h.set_value((minutes / 60) as f64);
    m.set_value((minutes % 60) as f64);
    let b = gtk::Box::builder()
        .spacing(4)
        .valign(gtk::Align::Center)
        .build();
    b.append(&h);
    b.append(&gtk::Label::new(Some(":")));
    b.append(&m);
    (b, h, m)
}

/// The scheduler: when scheduled downloads start and stop, and what then.
pub fn scheduler(ui: &UiRc) {
    let s = ui.manager.borrow().settings.schedule.clone();
    let dialog = adw::PreferencesDialog::new();
    dialog.set_title("Scheduler");
    dialog.set_search_enabled(false);
    let page = adw::PreferencesPage::new();
    let g = adw::PreferencesGroup::builder()
        .description("Downloads added to start “At the Scheduled Time”, or scheduled from their right-click menu, start at the start time and stop at the stop time, every day. Retriever keeps the computer from suspending while they wait: leave it switched on.")
        .build();
    let on = adw::SwitchRow::builder()
        .title("Run the Scheduler")
        .active(s.enabled)
        .build();
    g.add(&on);
    let (start_box, sh, sm) = time_picker(s.start);
    let start = adw::ActionRow::builder().title("Start At").build();
    start.add_suffix(&start_box);
    g.add(&start);
    let stop_on = adw::SwitchRow::builder()
        .title("Stop At a Time")
        .active(s.stop_enabled)
        .build();
    g.add(&stop_on);
    let (stop_box, eh, em) = time_picker(s.stop);
    let stop = adw::ActionRow::builder().title("Stop At").build();
    stop.add_suffix(&stop_box);
    stop_on
        .bind_property("active", &stop, "sensitive")
        .sync_create()
        .build();
    g.add(&stop);
    let labels: Vec<&str> = model::After::ALL.iter().map(|a| a.label()).collect();
    let then = adw::ComboRow::builder()
        .title("When They Are Done")
        .model(&gtk::StringList::new(&labels))
        .selected(
            model::After::ALL
                .iter()
                .position(|a| *a == s.then)
                .unwrap_or(0) as u32,
        )
        .build();
    g.add(&then);
    page.add(&g);

    let g2 = adw::PreferencesGroup::new();
    let n = ui
        .manager
        .borrow()
        .entries
        .iter()
        .filter(|e| e.item.scheduled && e.item.status != Status::Done)
        .count();
    let now = {
        let ui = ui.clone();
        let dialog = dialog.clone();
        crate::native::button_row(
            &format!(
                "Start the {n} Scheduled Download{} Now",
                if n == 1 { "" } else { "s" }
            ),
            "media-playback-start-symbolic",
            move || {
                ui.manager.borrow_mut().start_scheduled();
                ui.after_change();
                dialog.close();
            },
        )
    };
    now.set_sensitive(n > 0);
    g2.add(&now);
    page.add(&g2);
    dialog.add(&page);

    let ui2 = ui.clone();
    dialog.connect_closed(move |_| {
        let mut st = ui2.manager.borrow().settings.clone();
        st.schedule = model::Schedule {
            enabled: on.is_active(),
            start: sh.value() as u32 * 60 + sm.value() as u32,
            stop_enabled: stop_on.is_active(),
            stop: eh.value() as u32 * 60 + em.value() as u32,
            then: model::After::ALL[then.selected().min(3) as usize],
        };
        ui2.manager.borrow_mut().apply_settings(st);
        ui2.after_change();
    });
    dialog.present(Some(&ui.window));
}

/// The scheduler's downloads are done: `then`, after a minute that can be called off.
pub fn after_done(ui: &UiRc, then: model::After) {
    let what = match then {
        model::After::Nothing => return,
        model::After::Quit => "Retriever quits",
        model::After::Suspend => "The computer suspends",
        model::After::PowerOff => "The computer shuts down",
    };
    let d = adw::AlertDialog::new(Some("Scheduled Downloads Done"), None);
    d.add_responses(&[("cancel", "_Cancel"), ("now", "Do It _Now")]);
    d.set_response_appearance("now", adw::ResponseAppearance::Destructive);
    d.set_close_response("cancel");
    let left = Rc::new(std::cell::Cell::new(60u32));
    d.set_body(&format!("{what} in {} seconds.", left.get()));
    let act = {
        let ui = ui.clone();
        move || {
            let cmd = match then {
                model::After::Suspend => Some("suspend"),
                model::After::PowerOff => Some("poweroff"),
                _ => None,
            };
            if let Some(c) = cmd {
                // logind lets the person at the computer do it (no password).
                let _ = std::process::Command::new("systemctl").arg(c).spawn();
            }
            if then != model::After::Suspend {
                ui.quit();
            }
        }
    };
    let done = Rc::new(std::cell::Cell::new(false));
    {
        let done = done.clone();
        let act = act.clone();
        d.connect_response(None, move |_, r| {
            if done.replace(true) {
                return;
            }
            if r == "now" {
                act();
            }
        });
    }
    {
        let d = d.clone();
        glib::timeout_add_seconds_local(1, move || {
            if done.get() {
                return glib::ControlFlow::Break;
            }
            let n = left.get().saturating_sub(1);
            left.set(n);
            if n == 0 {
                done.set(true);
                d.close();
                act();
                return glib::ControlFlow::Break;
            }
            d.set_body(&format!("{what} in {n} seconds."));
            glib::ControlFlow::Continue
        });
    }
    ui.window.present();
    d.present(Some(&ui.window));
}
