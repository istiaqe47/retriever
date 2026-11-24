//! The newest libadwaita widgets when the system's libadwaita has them (a current Arch or
//! CachyOS has 1.8), so Retriever looks as GNOME's own apps there: the spinner (1.6), button
//! rows (1.6) and the shortcuts dialog (1.8). With an older libadwaita, what it had before.
//! They are made by type name at run time: the program is built against libadwaita 1.5.

use adw::prelude::*;
use gtk::glib;

fn have(type_name: &str) -> Option<glib::Type> {
    glib::Type::from_name(type_name).filter(|t| t.is_a(gtk::Widget::static_type()))
}

/// Makes sure libadwaita's types are registered (a type is known by name once its
/// `get_type` ran): asked of the Builder, which looks them up the same way.
fn ensure(type_name: &str) -> Option<glib::Type> {
    have(type_name).or_else(|| {
        let xml = format!(r#"<interface><object class="{type_name}" id="o"/></interface>"#);
        let b = gtk::Builder::new();
        b.add_from_string(&xml).ok()?;
        drop(b.object::<glib::Object>("o"));
        have(type_name)
    })
}

/// libadwaita's spinner (it always spins; shown or hidden), else GTK's, spinning.
pub fn spinner() -> gtk::Widget {
    if let Some(t) = ensure("AdwSpinner")
        && let Ok(w) = glib::Object::with_type(t).downcast::<gtk::Widget>()
    {
        w.set_size_request(16, 16);
        return w;
    }
    gtk::Spinner::builder().spinning(true).build().upcast()
}

/// A row that is a button (centred title), as GNOME's own dialogs have: `AdwButtonRow`, else
/// an activatable row with `icon` at its end.
pub fn button_row(title: &str, icon: &str, on: impl Fn() + 'static) -> gtk::Widget {
    if let Some(t) = ensure("AdwButtonRow")
        && let Ok(w) = glib::Object::with_type(t).downcast::<gtk::Widget>()
    {
        w.set_property("title", title);
        w.set_property("end-icon-name", icon);
        w.connect_local("activated", false, move |_| {
            on();
            None
        });
        return w;
    }
    let row = adw::ActionRow::builder()
        .title(title)
        .activatable(true)
        .build();
    row.add_suffix(&gtk::Image::from_icon_name(icon));
    row.connect_activated(move |_| on());
    row.upcast()
}

/// The keyboard shortcuts: libadwaita's dialog (1.8), else GTK's window.
pub fn shortcuts(window: &adw::ApplicationWindow, groups: &[(&str, &[(&str, &str)])]) {
    let esc = |s: &str| glib::markup_escape_text(s).to_string();
    if ensure("AdwShortcutsDialog").is_some() {
        let mut xml = String::from(r#"<interface><object class="AdwShortcutsDialog" id="d">"#);
        for (title, keys) in groups {
            xml.push_str(&format!(
                r#"<child><object class="AdwShortcutsSection"><property name="title">{}</property>"#,
                esc(title)
            ));
            for (accel, what) in *keys {
                xml.push_str(&format!(
                    r#"<child><object class="AdwShortcutsItem"><property name="title">{}</property><property name="accelerator">{}</property></object></child>"#,
                    esc(what),
                    esc(accel)
                ));
            }
            xml.push_str("</object></child>");
        }
        xml.push_str("</object></interface>");
        let b = gtk::Builder::new();
        if b.add_from_string(&xml).is_ok()
            && let Some(d) = b.object::<adw::Dialog>("d")
        {
            d.present(Some(window));
            return;
        }
    }
    let mut xml = String::from(
        r#"<interface><object class="GtkShortcutsWindow" id="w"><property name="modal">1</property><child><object class="GtkShortcutsSection"><property name="section-name">main</property>"#,
    );
    for (title, keys) in groups {
        xml.push_str(&format!(
            r#"<child><object class="GtkShortcutsGroup"><property name="title">{}</property>"#,
            esc(title)
        ));
        for (accel, what) in *keys {
            xml.push_str(&format!(
                r#"<child><object class="GtkShortcutsShortcut"><property name="accelerator">{}</property><property name="title">{}</property></object></child>"#,
                esc(accel),
                esc(what)
            ));
        }
        xml.push_str("</object></child>");
    }
    xml.push_str("</object></child></object></interface>");
    let b = gtk::Builder::new();
    if b.add_from_string(&xml).is_ok()
        && let Some(w) = b.object::<gtk::ShortcutsWindow>("w")
    {
        w.set_transient_for(Some(window));
        w.present();
    }
}
