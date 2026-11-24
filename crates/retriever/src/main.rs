//! Retriever: a GNOME download manager (GTK 4 + libadwaita) on Mister Download Manager's engine.

mod browser;
mod dialogs;
mod grabber;
mod localdoor;
mod manager;
mod model;
mod native;
mod object;
mod torrentdlg;
mod torrents;
mod video;
mod window;

use adw::prelude::*;
use gtk::{gio, glib};
use std::cell::RefCell;
use std::rc::Rc;

fn main() -> glib::ExitCode {
    let app = adw::Application::builder()
        .application_id(model::APP_ID)
        // Links given on the command line (`retriever <url>`) or by another instance.
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();
    let ui: Rc<RefCell<Option<window::UiRc>>> = Rc::default();

    let get = {
        let ui = ui.clone();
        move |app: &adw::Application| -> window::UiRc {
            if let Some(u) = ui.borrow().as_ref() {
                return u.clone();
            }
            let u = window::build(app);
            *ui.borrow_mut() = Some(u.clone());
            browser::start(&u);
            localdoor::start(&u);
            u
        }
    };
    {
        let get = get.clone();
        app.connect_activate(move |app| get(app).window.present());
    }
    {
        let get = get.clone();
        app.connect_open(move |app, files, _| {
            let u = get(app);
            u.window.present();
            let mut links = Vec::new();
            for f in files {
                let uri = f.uri().to_string();
                if mdm_torrent::Source::magnet(&uri).is_some() {
                    torrentdlg::dialog(&u, Some(torrentdlg::Given::Link(uri)));
                } else if let Some(p) = f.path().filter(|p| {
                    p.extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("torrent"))
                }) {
                    // A .torrent opened from Files or the browser's downloads.
                    if let Ok(bytes) = mdm_torrent::read_file(&p) {
                        torrentdlg::dialog(
                            &u,
                            Some(torrentdlg::Given::File(bytes, p.display().to_string())),
                        );
                    }
                } else {
                    links.push(uri);
                }
            }
            if !links.is_empty() {
                dialogs::add_url(&u, Some(links.join("\n")));
            }
        });
    }

    let action = |name: &str, f: Box<dyn Fn(&adw::Application)>| {
        let a = gio::SimpleAction::new(name, None);
        let app2 = app.clone();
        a.connect_activate(move |_, _| f(&app2));
        app.add_action(&a);
    };
    {
        let get = get.clone();
        action(
            "preferences",
            Box::new(move |app| dialogs::preferences(&get(app))),
        );
    }
    {
        let get = get.clone();
        action("about", Box::new(move |app| dialogs::about(&get(app))));
    }
    {
        let get = get.clone();
        action(
            "activate-window",
            Box::new(move |app| get(app).window.present()),
        );
    }
    {
        let ui = ui.clone();
        action(
            "quit",
            Box::new(move |app| match ui.borrow().as_ref() {
                Some(u) => u.quit(),
                None => app.quit(),
            }),
        );
    }

    for (action, keys) in [
        ("win.add", &["<Control>n"][..]),
        ("win.resume", &["<Control>r"]),
        ("win.pause", &["<Control>p"]),
        ("win.search", &["<Control>f"]),
        ("app.preferences", &["<Control>comma"]),
        ("win.shortcuts", &["<Control>question"]),
        ("app.quit", &["<Control>q"]),
        ("window.close", &["<Control>w"]),
    ] {
        app.set_accels_for_action(action, keys);
    }
    // Logout, shutdown or `kill`: closed as Quit closes it, so downloads pause with their
    // progress saved and go on next time.
    manager::rt().spawn(async {
        use tokio::signal::unix::{SignalKind, signal};
        let (Ok(mut term), Ok(mut int), Ok(mut hup)) = (
            signal(SignalKind::terminate()),
            signal(SignalKind::interrupt()),
            signal(SignalKind::hangup()),
        ) else {
            return;
        };
        loop {
            tokio::select! {
                _ = term.recv() => {}
                _ = int.recv() => {}
                _ = hup.recv() => {}
            }
            glib::MainContext::default().invoke(|| {
                if let Some(app) = gio::Application::default() {
                    gio::prelude::ActionGroupExt::activate_action(&app, "quit", None);
                }
            });
        }
    });
    // Started by the browser's host (`/fromhost`): nothing to open, the host sends the link.
    let args: Vec<String> = std::env::args()
        .filter(|a| a != mdm_ipc::FROM_HOST)
        .collect();
    app.run_with_args(&args)
}
