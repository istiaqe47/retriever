//! Download Video: a page's video looked up by yt-dlp (the system's), its quality chosen, and
//! downloaded by yt-dlp with MDM's engine for its streams.

use crate::manager::{Extra, rt};
use crate::model::VideoSpec;
use crate::window::UiRc;
use adw::prelude::*;
use gtk::{gio, glib};
use mdm_tools::resolve::{self, Looked};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

/// A program: the person's own first (`~/.local/bin`, where `pipx install yt-dlp` puts a current
/// yt-dlp, often newer than the distribution's), then the PATH, then the usual folders: an app
/// opened from the app grid may not have the shell's PATH.
fn which(name: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let path = std::env::var_os("PATH").unwrap_or_default();
    home.iter()
        .map(|h| h.join(".local/bin"))
        .chain(std::env::split_paths(&path))
        .chain(["/usr/local/bin", "/usr/bin", "/bin"].map(PathBuf::from))
        .filter(|d| d.is_absolute())
        .map(|d| d.join(name))
        .find(|p| {
            std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
}

pub fn ytdlp() -> Option<PathBuf> {
    which("yt-dlp")
}

pub fn ffmpeg() -> Option<PathBuf> {
    which("ffmpeg")
}

/// What a look-up found, kept for the Download button.
struct Found {
    page: String,
    address: String,
    title: String,
    choices: Vec<resolve::Choice>,
}

pub fn dialog(ui: &UiRc, page: Option<String>, extra: Extra) {
    let folder = Rc::new(RefCell::new(
        extra
            .folder
            .clone()
            .unwrap_or_else(|| ui.manager.borrow().settings.folder.clone()),
    ));
    let dialog = adw::Dialog::builder()
        .title("Download Video")
        .content_width(540)
        // Room for the look-up's answer.
        .content_height(520)
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

    let address = adw::EntryRow::builder()
        .title("Page Address")
        .show_apply_button(true)
        .build();
    let look = adw::PreferencesGroup::builder()
        .description("YouTube and the other sites yt-dlp knows, or any page that plays a video")
        .build();
    look.add(&address);

    let spinner = crate::native::spinner();
    let said = gtk::Label::builder().wrap(true).xalign(0.0).build();
    crate::native::dim(&said);
    let state = gtk::Box::builder().spacing(12).visible(false).build();
    state.append(&spinner);
    state.append(&said);

    let title_row = adw::ActionRow::builder()
        .title("Video")
        .subtitle_selectable(true)
        .build();
    title_row.add_css_class("property");
    let quality = adw::ComboRow::builder().title("Quality").build();
    let save_row = adw::ActionRow::builder()
        .title("Save To")
        .subtitle(folder.borrow().display().to_string())
        .activatable(true)
        .build();
    save_row.add_prefix(&gtk::Image::from_icon_name("folder-symbolic"));
    save_row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    let choice = adw::PreferencesGroup::builder().visible(false).build();
    choice.add(&title_row);
    choice.add(&quality);
    choice.add(&save_row);

    let page_ = adw::PreferencesPage::new();
    page_.add(&look);
    let state_group = adw::PreferencesGroup::new();
    state_group.add(&state);
    page_.add(&state_group);
    page_.add(&choice);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&page_));
    dialog.set_child(Some(&view));

    {
        let folder = folder.clone();
        let dialog = dialog.clone();
        save_row.connect_activated(move |row| {
            let chooser = gtk::FileDialog::builder()
                .title("Save Videos To")
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

    let found: Rc<RefCell<Option<Found>>> = Rc::default();
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
    let lookup = {
        let found = found.clone();
        let start = start.clone();
        let choice = choice.clone();
        let title_row = title_row.clone();
        let quality = quality.clone();
        let say = say.clone();
        move |page: String| {
            let page = page.trim().to_owned();
            if url::Url::parse(&page).is_err() {
                say("That isn't a web address", false);
                return;
            }
            let Some(exe) = ytdlp() else {
                say(
                    &format!("yt-dlp isn't installed. Install it with: {}", crate::model::install_hint("yt-dlp", "pipx install yt-dlp")),
                    false,
                );
                return;
            };
            *found.borrow_mut() = None;
            start.set_sensitive(false);
            choice.set_visible(false);
            say("Looking the video up…", true);
            let can_merge = ffmpeg().is_some();
            let p = page.clone();
            let job = rt().spawn_blocking(move || {
                let reader = mdm_tools::streams::Engine {
                    handle: rt().handle().clone(),
                    opts: Default::default(),
                };
                mdm_tools::declared::look_up(&exe, &p, &Default::default(), &reader)
                    .map(|(looked, _, declared)| (looked, declared.map(|d| d.address)))
                    .map_err(|e| e.to_string())
            });
            let found = found.clone();
            let start = start.clone();
            let choice = choice.clone();
            let title_row = title_row.clone();
            let quality = quality.clone();
            let say = say.clone();
            glib::spawn_future_local(async move {
                let result = job.await.map_err(|e| e.to_string()).and_then(|r| r);
                match result {
                    Ok((Looked::Video(v), declared)) => {
                        let choices = resolve::choices(&v, can_merge);
                        if choices.is_empty() {
                            say("The page has nothing to download", false);
                            return;
                        }
                        let labels: Vec<String> = choices.iter().map(label).collect();
                        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
                        quality.set_model(Some(&gtk::StringList::new(&refs)));
                        quality.set_selected(0);
                        title_row.set_subtitle(&v.title);
                        let mut note = String::new();
                        if v.needs_js_runtime {
                            note = format!("More qualities are offered with Deno installed ({})", crate::model::install_hint("deno", "curl -fsSL https://deno.land/install.sh | sh"));
                        } else if !can_merge {
                            note = format!("Picture and sound come apart without ffmpeg ({})", crate::model::install_hint("ffmpeg", "sudo apt install ffmpeg"));
                        }
                        say(&note, false);
                        choice.set_visible(true);
                        start.set_sensitive(true);
                        *found.borrow_mut() = Some(Found {
                            address: declared.unwrap_or_else(|| page.clone()),
                            page,
                            title: v.title,
                            choices,
                        });
                    }
                    Ok((Looked::List(l), _)) => say(
                        &format!(
                            "This page is a list of videos ({}{}): open one video's page",
                            l.title,
                            l.total.map(|n| format!(", {n} videos")).unwrap_or_default()
                        ),
                        false,
                    ),
                    Err(why) => say(&format!("yt-dlp couldn't look it up: {why}"), false),
                }
            });
        }
    };
    {
        let lookup = lookup.clone();
        address.connect_apply(move |e| lookup(e.text().to_string()));
    }
    {
        let lookup = lookup.clone();
        address.connect_entry_activated(move |e| lookup(e.text().to_string()));
    }
    if let Some(p) = page {
        address.set_text(&p);
        lookup(p);
    }

    {
        let ui = ui.clone();
        let dialog = dialog.clone();
        start.connect_clicked(move |_| {
            let f = found.borrow();
            let Some(f) = f.as_ref() else { return };
            let Some(c) = f.choices.get(quality.selected() as usize) else {
                return;
            };
            let dir = folder.borrow().clone();
            let ext = resolve::audio_conversion(&c.selector)
                .map(|a| a.format.to_owned())
                .unwrap_or_else(|| {
                    if c.ext.is_empty() {
                        "mp4".into()
                    } else {
                        c.ext.clone()
                    }
                });
            // A stem no file in the folder has, whatever the extension: yt-dlp takes any
            // `<stem>.*` there as its own.
            let name = format!("{}.{ext}", mdm_engine::naming::sanitize(&f.title));
            let free = mdm_engine::naming::stem_destination(&dir, &name, |_| false);
            let stem = free
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "video".into());
            let spec = VideoSpec {
                address: f.address.clone(),
                format: c.selector.clone(),
                stem: stem.clone(),
                expected: c.size,
            };
            ui.manager
                .borrow_mut()
                .add_video(&f.page, dir, &format!("{stem}.{ext}"), spec, &extra);
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
    dialog.set_focus(Some(&address));
    dialog.present(Some(&ui.window));
}

/// "1080p · H.264 · 532 MB".
fn label(c: &resolve::Choice) -> String {
    match c.size {
        Some(s) => format!(
            "{} · {}{}",
            c.label,
            if c.approx { "~" } else { "" },
            crate::model::size_text(s)
        ),
        None => c.label.clone(),
    }
}
