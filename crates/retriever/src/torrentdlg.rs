//! Add Torrent: a magnet link or a .torrent file, its files to choose, where to save it.

use crate::manager::{Extra, rt};
use crate::model::{self, TorrentSpec};
use crate::torrents;
use crate::window::UiRc;
use adw::prelude::*;
use gtk::{gio, glib};
use mdm_torrent::{Meta, Source};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

/// A link (a magnet) or a .torrent file's bytes.
pub enum Given {
    Link(String),
    File(Vec<u8>, String),
}

pub fn dialog(ui: &UiRc, given: Option<Given>) {
    torrents::ensure();
    let folder = Rc::new(RefCell::new(ui.manager.borrow().settings.folder.clone()));
    let dialog = adw::Dialog::builder()
        .title("Add Torrent")
        .content_width(560)
        .content_height(600)
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

    let magnet = adw::EntryRow::builder()
        .title("Magnet Link")
        .show_apply_button(true)
        .build();
    let open = adw::ActionRow::builder()
        .title("Open a .torrent File…")
        .activatable(true)
        .build();
    open.add_prefix(&gtk::Image::from_icon_name("document-open-symbolic"));
    let source = adw::PreferencesGroup::new();
    source.add(&magnet);
    source.add(&open);

    let spinner = crate::native::spinner();
    let said = gtk::Label::builder().wrap(true).xalign(0.0).build();
    said.add_css_class("dim-label");
    let state = gtk::Box::builder().spacing(12).visible(false).build();
    state.append(&spinner);
    state.append(&said);
    let state_group = adw::PreferencesGroup::new();
    state_group.add(&state);

    let name_row = adw::ActionRow::builder().title("Torrent").build();
    name_row.add_css_class("property");
    let save_row = adw::ActionRow::builder()
        .title("Save To")
        .subtitle(folder.borrow().display().to_string())
        .activatable(true)
        .build();
    save_row.add_prefix(&gtk::Image::from_icon_name("folder-symbolic"));
    save_row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    let info = adw::PreferencesGroup::builder().visible(false).build();
    info.add(&name_row);
    info.add(&save_row);
    let files_group = adw::PreferencesGroup::builder()
        .title("Files")
        .visible(false)
        .build();

    let page = adw::PreferencesPage::new();
    page.add(&source);
    page.add(&state_group);
    page.add(&info);
    page.add(&files_group);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&page));
    dialog.set_child(Some(&view));

    {
        let folder = folder.clone();
        let dialog = dialog.clone();
        save_row.connect_activated(move |row| {
            let chooser = gtk::FileDialog::builder()
                .title("Save Torrents To")
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

    let meta: Rc<RefCell<Option<(Meta, String)>>> = Rc::default();
    let checks: Rc<RefCell<Vec<(usize, gtk::CheckButton)>>> = Rc::default();
    let say = {
        let state = state.clone();
        let spinner = spinner.clone();
        let said = said.clone();
        move |text: &str, busy: bool| {
            state.set_visible(!text.is_empty());
            spinner.set_visible(busy);
            said.set_label(text);
        }
    };
    let read = {
        let meta = meta.clone();
        let checks = checks.clone();
        let start = start.clone();
        let info = info.clone();
        let files_group = files_group.clone();
        let name_row = name_row.clone();
        let say = say.clone();
        move |src: Source, link: String| {
            *meta.borrow_mut() = None;
            start.set_sensitive(false);
            info.set_visible(false);
            files_group.set_visible(false);
            let magnet = matches!(src, Source::Magnet(_));
            say(
                if magnet {
                    "Asking the peers for the torrent's details…"
                } else {
                    "Reading the torrent…"
                },
                true,
            );
            let meta = meta.clone();
            let checks = checks.clone();
            let start = start.clone();
            let info = info.clone();
            let files_group = files_group.clone();
            let name_row = name_row.clone();
            let say = say.clone();
            glib::spawn_future_local(async move {
                // The session starts with the app's first torrent: a moment.
                let mut waited = 0;
                while torrents::session().is_none() {
                    if let Some(why) = torrents::failed() {
                        say(&format!("Torrents can't start: {why}"), false);
                        return;
                    }
                    if waited > 300 {
                        say("The torrent session didn't start", false);
                        return;
                    }
                    glib::timeout_future(Duration::from_millis(100)).await;
                    waited += 1;
                }
                let Some(t) = torrents::session() else { return };
                let job = rt()
                    .spawn(async move { t.metadata(&src, Duration::from_secs(180), None).await });
                match job.await {
                    Ok(Ok(m)) => {
                        name_row.set_subtitle(&format!(
                            "{} · {}",
                            m.name,
                            model::size_text(m.total())
                        ));
                        // Its files, each to tick (all ticked).
                        let mut c = checks.borrow_mut();
                        c.clear();
                        let shown: Vec<(usize, &mdm_torrent::FileEntry)> = m
                            .files
                            .iter()
                            .enumerate()
                            .filter(|(_, f)| !f.padding)
                            .collect();
                        if shown.len() > 1 {
                            for (i, f) in shown.iter().take(500) {
                                let check = gtk::CheckButton::builder().active(true).build();
                                let row = adw::ActionRow::builder()
                                    .title(glib::markup_escape_text(&f.path.display().to_string()))
                                    .subtitle(model::size_text(f.size))
                                    .activatable_widget(&check)
                                    .build();
                                row.add_prefix(&check);
                                files_group.add(&row);
                                c.push((*i, check));
                            }
                            files_group.set_visible(true);
                        }
                        info.set_visible(true);
                        say("", false);
                        start.set_sensitive(true);
                        *meta.borrow_mut() = Some((m, link));
                    }
                    Ok(Err(e)) => say(&format!("Couldn't read the torrent: {e}"), false),
                    Err(e) => say(&format!("Couldn't read the torrent: {e}"), false),
                }
            });
        }
    };
    {
        let read = read.clone();
        let say = say.clone();
        magnet.connect_apply(move |e| match Source::magnet(&e.text()) {
            Some(src) => read(src, e.text().trim().to_owned()),
            None => say("A magnet link starts with “magnet:?”", false),
        });
    }
    {
        let read = read.clone();
        let say = say.clone();
        let dialog = dialog.clone();
        open.connect_activated(move |_| {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("Torrents"));
            filter.add_suffix("torrent");
            filter.add_mime_type("application/x-bittorrent");
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let chooser = gtk::FileDialog::builder()
                .title("Open a Torrent")
                .filters(&filters)
                .build();
            let parent = dialog.root().and_downcast::<gtk::Window>();
            let read = read.clone();
            let say = say.clone();
            chooser.open(parent.as_ref(), gio::Cancellable::NONE, move |r| {
                if let Ok(f) = r
                    && let Some(p) = f.path()
                {
                    match mdm_torrent::read_file(&p) {
                        Ok(bytes) => read(Source::File(bytes), p.display().to_string()),
                        Err(e) => say(&format!("Couldn't open it: {e}"), false),
                    }
                }
            });
        });
    }
    match given {
        Some(Given::Link(l)) => {
            magnet.set_text(&l);
            if let Some(src) = Source::magnet(&l) {
                read(src, l);
            }
        }
        Some(Given::File(bytes, name)) => read(Source::File(bytes), name),
        None => {}
    }

    {
        let ui = ui.clone();
        let dialog = dialog.clone();
        start.connect_clicked(move |_| {
            let m = meta.borrow();
            let Some((m, link)) = m.as_ref() else { return };
            let dir = folder.borrow().clone();
            let files: Option<Vec<usize>> = {
                let c = checks.borrow();
                if c.is_empty() || c.iter().all(|(_, b)| b.is_active()) {
                    None
                } else {
                    Some(
                        c.iter()
                            .filter(|(_, b)| b.is_active())
                            .map(|(i, _)| *i)
                            .collect(),
                    )
                }
            };
            if files.as_ref().is_some_and(Vec::is_empty) {
                return;
            }
            let size: u64 = m
                .files
                .iter()
                .enumerate()
                .filter(|(i, f)| !f.padding && files.as_ref().is_none_or(|w| w.contains(i)))
                .map(|(_, f)| f.size)
                .sum();
            let path = torrents::torrent_file(&m.info_hash);
            if let Some(d) = path.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            if std::fs::write(&path, &m.torrent).is_err() {
                return;
            }
            // Several files: a folder named after it; one: straight into the folder.
            let folder = m.folder_in(&dir);
            let several = m.files.iter().filter(|f| !f.padding).count() > 1;
            let spec = TorrentSpec {
                hash: m.info_hash.clone(),
                torrent: path,
                files,
            };
            let id = ui.manager.borrow_mut().add_torrent(
                link,
                &m.name,
                size,
                folder.clone(),
                spec,
                &Extra::default(),
            );
            {
                let mut mm = ui.manager.borrow_mut();
                mm.set_file(
                    id,
                    if several {
                        folder
                    } else {
                        m.files
                            .iter()
                            .find(|f| !f.padding)
                            .map(|f| folder.join(&f.path))
                            .unwrap_or(folder)
                    },
                );
            }
            ui.after_change();
            dialog.close();
        });
    }
    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| {
            dialog.close();
        });
    }
    dialog.present(Some(&ui.window));
}
