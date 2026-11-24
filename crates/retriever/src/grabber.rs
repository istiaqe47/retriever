//! Grab Links: every link of a web page (or the ones a browser's right-click menu sent), to
//! pick from by type and name, downloaded together.

use crate::manager::{Extra, rt};
use crate::model::{self, Category};
use crate::window::UiRc;
use adw::prelude::*;
use gtk::{gio, glib};
use std::cell::RefCell;
use std::rc::Rc;

/// The type filter's entries: a file kind, or everything (pages too).
const KINDS: [(&str, Option<Category>); 7] = [
    ("Files", None),
    ("Compressed", Some(Category::Compressed)),
    ("Documents", Some(Category::Documents)),
    ("Music", Some(Category::Music)),
    ("Programs", Some(Category::Programs)),
    ("Video", Some(Category::Video)),
    ("Every Link", Some(Category::All)),
];

struct Link {
    url: String,
    name: String,
    kind: Category,
    row: adw::ActionRow,
    check: gtk::CheckButton,
}

pub fn dialog(ui: &UiRc, page: Option<String>, given: Vec<String>, extra: Extra) {
    let folder = Rc::new(RefCell::new(
        extra
            .folder
            .clone()
            .unwrap_or_else(|| ui.manager.borrow().settings.folder.clone()),
    ));
    let dialog = adw::Dialog::builder()
        .title("Grab Links")
        .content_width(640)
        .content_height(640)
        .build();
    let cancel = gtk::Button::with_mnemonic("_Cancel");
    let start = gtk::Button::with_label("Download");
    start.add_css_class("suggested-action");
    start.set_sensitive(false);
    let header = adw::HeaderBar::builder()
        .show_start_title_buttons(false)
        .show_end_title_buttons(false)
        .build();
    header.pack_start(&cancel);
    header.pack_end(&start);

    let address = adw::EntryRow::builder()
        .title("Page Address")
        .show_apply_button(true)
        .build();
    let save_row = adw::ActionRow::builder()
        .title("Save To")
        .subtitle(folder.borrow().display().to_string())
        .activatable(true)
        .build();
    save_row.add_prefix(&gtk::Image::from_icon_name("folder-symbolic"));
    save_row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    let top = adw::PreferencesGroup::new();
    top.add(&address);
    top.add(&save_row);

    let kind = gtk::DropDown::from_strings(&KINDS.map(|(l, _)| l));
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Filter by name")
        .hexpand(true)
        .build();
    let all = gtk::Button::with_label("All");
    let none = gtk::Button::with_label("None");
    let pick = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    pick.add_css_class("linked");
    pick.append(&all);
    pick.append(&none);
    let bar = gtk::Box::builder().spacing(8).build();
    bar.append(&search);
    bar.append(&kind);
    bar.append(&pick);

    let spinner = crate::native::spinner();
    let said = gtk::Label::builder().wrap(true).xalign(0.0).build();
    said.add_css_class("dim-label");
    let state = gtk::Box::builder().spacing(12).visible(false).build();
    state.append(&spinner);
    state.append(&said);

    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_visible(false);
    list.set_selection_mode(gtk::SelectionMode::None);

    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(18)
        .margin_start(12)
        .margin_end(12)
        .build();
    body.append(&top);
    body.append(&bar);
    body.append(&state);
    body.append(&list);
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&adw::Clamp::builder().maximum_size(620).child(&body).build())
        .build();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&scrolled));
    dialog.set_child(Some(&view));

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

    let links: Rc<RefCell<Vec<Link>>> = Rc::default();
    // What is shown (the filters) and how many are ticked, on the button.
    let refresh = {
        let links = links.clone();
        let kind = kind.clone();
        let search = search.clone();
        let start = start.clone();
        move || {
            let k = KINDS[kind.selected() as usize].1;
            let q = search.text().to_lowercase();
            let mut ticked = 0;
            for l in links.borrow().iter() {
                let shown = match k {
                    None => l.kind != Category::Other,
                    Some(Category::All) => true,
                    Some(c) => l.kind == c,
                } && (q.is_empty()
                    || l.name.to_lowercase().contains(&q)
                    || l.url.to_lowercase().contains(&q));
                l.row.set_visible(shown);
                if shown && l.check.is_active() {
                    ticked += 1;
                }
            }
            start.set_label(&if ticked > 0 {
                format!("Download {ticked}")
            } else {
                "Download".into()
            });
            start.set_sensitive(ticked > 0);
        }
    };
    let fill = {
        let links = links.clone();
        let list = list.clone();
        let refresh = refresh.clone();
        move |found: Vec<String>| {
            let mut ls = links.borrow_mut();
            for l in ls.drain(..) {
                list.remove(&l.row);
            }
            let mut seen = std::collections::HashSet::new();
            for url in found {
                if !seen.insert(url.clone()) || crate::dialogs::links_in(&url).is_empty() {
                    continue;
                }
                let name = model::name_from_url(&url);
                let kind = Category::of(&name);
                let check = gtk::CheckButton::builder()
                    .active(kind != Category::Other)
                    .build();
                let row = adw::ActionRow::builder()
                    .title(glib::markup_escape_text(&name))
                    .subtitle(glib::markup_escape_text(&url))
                    .subtitle_lines(1)
                    .activatable_widget(&check)
                    .build();
                row.add_prefix(&check);
                let r = refresh.clone();
                check.connect_toggled(move |_| r());
                list.append(&row);
                ls.push(Link {
                    url,
                    name,
                    kind,
                    row,
                    check,
                });
            }
            list.set_visible(!ls.is_empty());
            drop(ls);
            refresh();
        }
    };
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
    let grab = {
        let fill = fill.clone();
        let say = say.clone();
        let headers = extra.headers.clone();
        let opts = ui.manager.borrow().settings.system_proxy;
        move |page: String| {
            let Ok(url) = url::Url::parse(page.trim()) else {
                say("That isn't a web address", false);
                return;
            };
            say("Reading the page…", true);
            let mut h = reqwest::header::HeaderMap::new();
            for (k, v) in &headers {
                if let (Ok(k), Ok(v)) = (
                    reqwest::header::HeaderName::from_bytes(k.as_bytes()),
                    reqwest::header::HeaderValue::from_str(v),
                ) {
                    h.append(k, v);
                }
            }
            let o = mdm_engine::Options {
                proxy: if opts {
                    mdm_engine::Proxy::System
                } else {
                    mdm_engine::Proxy::None
                },
                ..Default::default()
            };
            let job = rt().spawn(async move { mdm_engine::page_links(url, h, &o).await });
            let fill = fill.clone();
            let say = say.clone();
            glib::spawn_future_local(async move {
                match job.await {
                    Ok(Ok((_, found))) => {
                        let n = found.len();
                        fill(found.into_iter().map(|u| u.to_string()).collect());
                        say(
                            &if n == 0 {
                                "The page has no links".to_owned()
                            } else {
                                String::new()
                            },
                            false,
                        );
                    }
                    Ok(Err(e)) => say(&format!("Couldn't read the page: {e}"), false),
                    Err(e) => say(&format!("Couldn't read the page: {e}"), false),
                }
            });
        }
    };
    {
        let grab = grab.clone();
        address.connect_apply(move |e| grab(e.text().to_string()));
    }
    {
        let grab = grab.clone();
        address.connect_entry_activated(move |e| grab(e.text().to_string()));
    }
    {
        let r = refresh.clone();
        kind.connect_selected_notify(move |_| r());
    }
    {
        let r = refresh.clone();
        search.connect_search_changed(move |_| r());
    }
    for (button, on) in [(&all, true), (&none, false)] {
        let links = links.clone();
        button.connect_clicked(move |_| {
            for l in links.borrow().iter().filter(|l| l.row.is_visible()) {
                l.check.set_active(on);
            }
        });
    }
    if !given.is_empty() {
        // The browser's links (a page's, a selection's).
        if let Some(p) = &page {
            address.set_text(p);
        }
        fill(given);
    } else if let Some(p) = page {
        address.set_text(&p);
        grab(p);
    }
    {
        let ui = ui.clone();
        let dialog = dialog.clone();
        let links = links.clone();
        start.connect_clicked(move |_| {
            let chosen: Vec<String> = links
                .borrow()
                .iter()
                .filter(|l| l.row.is_visible() && l.check.is_active())
                .map(|l| l.url.clone())
                .collect();
            let dir = folder.borrow().clone();
            {
                let mut m = ui.manager.borrow_mut();
                let conns = m.settings.connections;
                let x = Extra {
                    name: None,
                    ..extra.clone()
                };
                for u in &chosen {
                    m.add_with(u, dir.clone(), conns, &x);
                }
            }
            ui.after_change();
            ui.toast(&format!(
                "{} download{} added",
                chosen.len(),
                if chosen.len() == 1 { "" } else { "s" }
            ));
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
