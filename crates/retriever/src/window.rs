//! The main window: categories on the side, the download list, its header bar and actions.

use crate::dialogs;
use crate::manager::{self, Manager, Shared, code_status};
use crate::model::{self, Category, Status};
use crate::object::DlObject;
use adw::prelude::*;
use gtk::{gdk, gio, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

pub struct Ui {
    pub app: adw::Application,
    pub window: adw::ApplicationWindow,
    pub manager: Shared,
    pub list: gtk::ListBox,
    /// Picking several (the header's Select): the list's selection and the action bar.
    selecting: Cell<bool>,
    select_btn: gtk::ToggleButton,
    action_bar: gtk::ActionBar,
    picked: gtk::MenuButton,
    /// The download a row's menu (or its keys) is about, outside Select.
    context: Rc<Cell<Option<u64>>>,
    row_menu: gio::Menu,
    title: adw::WindowTitle,
    counts: Vec<(Category, gtk::Label)>,
    category: Rc<Cell<Category>>,
    search: Rc<RefCell<String>>,
    filter: gtk::CustomFilter,
    stack: gtk::Stack,
    empty: adw::StatusPage,
    sorted: gtk::SortListModel,
    split: adw::NavigationSplitView,
    inhibit: Cell<Option<u32>>,
    toasts: adw::ToastOverlay,
}

pub type UiRc = Rc<Ui>;
/// What a row's own button does (set once the window is up).
type RowAction = Rc<RefCell<Option<Rc<dyn Fn(u64, RowEvent)>>>>;

/// What happened on a row.
pub enum RowEvent {
    /// Its own button (pause, resume, show).
    Button,
    /// Its ⋮ button: the menu, from it.
    Menu(gtk::Widget),
    /// Its check was ticked or unticked (Select).
    Picked,
}

pub fn build(app: &adw::Application) -> UiRc {
    let settings = model::load_settings();
    apply_style(settings.style);
    let (w, h) = (settings.width.max(360), settings.height.max(360));
    let manager = Manager::new(settings);

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Retriever")
        .default_width(w)
        .default_height(h)
        .width_request(360)
        .height_request(300)
        .build();

    // ---- The list ----
    let category = Rc::new(Cell::new(Category::All));
    let search = Rc::new(RefCell::new(String::new()));
    let filter = {
        let category = category.clone();
        let search = search.clone();
        gtk::CustomFilter::new(move |o| {
            let o = o.downcast_ref::<DlObject>().expect("a download");
            let status = code_status(o.status());
            let shown = match category.get() {
                Category::All => true,
                Category::Unfinished => status != Status::Done,
                Category::Finished => status == Status::Done,
                kind => manager::code_kind(o.kind()) == kind,
            };
            let q = search.borrow();
            shown && (q.is_empty() || o.name().to_lowercase().contains(q.as_str()))
        })
    };
    let store = manager.borrow().store.clone();
    let filtered = gtk::FilterListModel::new(Some(store), Some(filter.clone()));
    let sorted = gtk::SortListModel::new(Some(filtered), None::<gtk::Sorter>);
    // Newest first, as a browser's downloads.
    sorted.set_sorter(Some(
        &gtk::NumericSorter::builder()
            .expression(gtk::PropertyExpression::new(
                DlObject::static_type(),
                gtk::Expression::NONE,
                "added",
            ))
            .sort_order(gtk::SortType::Descending)
            .build(),
    ));
    style();
    // A card per download, as GNOME's own lists (Files' Devices & Locations, Fragments).
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .activate_on_single_click(true)
        .valign(gtk::Align::Start)
        .build();
    list.add_css_class("boxed-list");
    list.update_property(&[gtk::accessible::Property::Label("Downloads")]);
    let row_action: RowAction = Rc::default();
    {
        let row_action = row_action.clone();
        list.bind_model(Some(&sorted), move |o| {
            let o = o.downcast_ref::<DlObject>().expect("a download");
            download_row(o, row_action.clone()).upcast()
        });
    }
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(
            &adw::Clamp::builder()
                .maximum_size(900)
                .tightening_threshold(600)
                .child(&list)
                .margin_top(18)
                .margin_bottom(24)
                .margin_start(12)
                .margin_end(12)
                .build(),
        )
        .build();

    let empty = adw::StatusPage::builder()
        .icon_name("folder-download-symbolic")
        .title("No Downloads")
        .description("Add a link with the + button, or drop one here")
        .build();
    let add_btn = gtk::Button::builder()
        .label("Add URL")
        .halign(gtk::Align::Center)
        .action_name("win.add")
        .build();
    add_btn.add_css_class("pill");
    add_btn.add_css_class("suggested-action");
    empty.set_child(Some(&add_btn));

    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .build();
    stack.add_named(&scrolled, Some("list"));
    stack.add_named(&empty, Some("empty"));

    // ---- Content header ----
    let title = adw::WindowTitle::new("All Downloads", "");
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&title));
    // + adds a link; its arrow, the other ways to add.
    let adds = gio::Menu::new();
    adds.append(Some("Add URL…"), Some("win.add"));
    adds.append(Some("Download Video…"), Some("win.add-video"));
    adds.append(Some("Add Torrent…"), Some("win.add-torrent"));
    adds.append(Some("Grab Links From a Page…"), Some("win.grab"));
    let add = adw::SplitButton::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text("Add URL (Ctrl+N)")
        .dropdown_tooltip("More Ways to Add")
        .action_name("win.add")
        .menu_model(&adds)
        .build();
    header.pack_start(&add);
    let menu = gio::Menu::new();
    let s1 = gio::Menu::new();
    s1.append(Some("_Select"), Some("win.select-mode"));
    s1.append(Some("Resume All"), Some("win.resume-all"));
    s1.append(Some("Pause All"), Some("win.pause-all"));
    s1.append(Some("Delete Completed"), Some("win.delete-completed"));
    s1.append(Some("_Scheduler…"), Some("win.scheduler"));
    menu.append_section(None, &s1);
    let s2 = gio::Menu::new();
    s2.append(Some("_Preferences"), Some("app.preferences"));
    s2.append(Some("_Keyboard Shortcuts"), Some("win.shortcuts"));
    s2.append(Some("_About Retriever"), Some("app.about"));
    menu.append_section(None, &s2);
    let menu_btn = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(&menu)
        .primary(true)
        .tooltip_text("Main Menu")
        .build();
    header.pack_end(&menu_btn);
    let search_btn = gtk::ToggleButton::builder()
        .icon_name("system-search-symbolic")
        .tooltip_text("Search (Ctrl+F)")
        .build();
    header.pack_end(&search_btn);
    // Select: several downloads at once, acted on from the bar at the bottom (as Files).
    let select_btn = gtk::ToggleButton::builder()
        .icon_name("selection-mode-symbolic")
        .tooltip_text("Select")
        .build();
    header.pack_end(&select_btn);
    // "N Selected ▾": GNOME's selection menu, with Select All and Select None.
    let pick_menu = gio::Menu::new();
    pick_menu.append(Some("Select _All"), Some("win.select-all"));
    pick_menu.append(Some("Select _None"), Some("win.select-none"));
    let picked = gtk::MenuButton::builder()
        .label("Nothing Selected")
        .always_show_arrow(true)
        .menu_model(&pick_menu)
        .build();
    picked.add_css_class("flat");
    let action_bar = gtk::ActionBar::builder().revealed(false).build();
    action_bar.set_center_widget(Some(&picked));
    for (icon, tip, action) in [
        ("media-playback-start-symbolic", "Resume", "win.resume"),
        ("media-playback-pause-symbolic", "Pause", "win.pause"),
    ] {
        action_bar.pack_start(&icon_button(icon, tip, action));
    }
    let del = gtk::Button::builder()
        .label("_Delete")
        .use_underline(true)
        .action_name("win.delete")
        .build();
    del.add_css_class("destructive-action");
    action_bar.pack_end(&del);

    let search_entry = gtk::SearchEntry::builder()
        .placeholder_text("Search downloads")
        .build();
    let search_bar = gtk::SearchBar::builder()
        .child(
            &adw::Clamp::builder()
                .maximum_size(480)
                .child(&search_entry)
                .build(),
        )
        .build();
    search_bar.connect_entry(&search_entry);
    search_bar.set_key_capture_widget(Some(&window));
    search_btn
        .bind_property("active", &search_bar, "search-mode-enabled")
        .bidirectional()
        .build();

    {
        let a = gio::SimpleAction::new("search", None);
        let b = search_btn.clone();
        a.connect_activate(move |_, _| b.set_active(!b.is_active()));
        window.add_action(&a);
    }
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&stack));

    let content_view = adw::ToolbarView::new();
    content_view.add_top_bar(&header);
    content_view.add_top_bar(&search_bar);
    content_view.set_content(Some(&toasts));
    content_view.add_bottom_bar(&action_bar);
    let content = adw::NavigationPage::builder()
        .title("All Downloads")
        .tag("content")
        .child(&content_view)
        .build();

    // ---- Sidebar ----
    let sidebar_list = gtk::ListBox::new();
    sidebar_list.add_css_class("navigation-sidebar");
    let mut counts = Vec::new();
    for c in Category::SIDEBAR {
        let row_box = gtk::Box::builder()
            .spacing(12)
            .margin_start(6)
            .margin_end(6)
            .margin_top(9)
            .margin_bottom(9)
            .build();
        row_box.append(&gtk::Image::from_icon_name(c.icon()));
        row_box.append(
            &gtk::Label::builder()
                .label(c.title())
                .xalign(0.0)
                .hexpand(true)
                .build(),
        );
        let count = gtk::Label::new(None);
        crate::native::dim(&count);
        count.add_css_class("numeric");
        row_box.append(&count);
        counts.push((c, count));
        sidebar_list.append(&gtk::ListBoxRow::builder().child(&row_box).build());
    }
    sidebar_list.set_header_func(|row, _| {
        if row.index() == 3 {
            let sep = gtk::Separator::new(gtk::Orientation::Horizontal);
            sep.set_margin_top(6);
            sep.set_margin_bottom(6);
            row.set_header(Some(&sep));
        } else {
            row.set_header(None::<&gtk::Widget>);
        }
    });
    let sidebar_view = adw::ToolbarView::new();
    sidebar_view.add_top_bar(
        &adw::HeaderBar::builder()
            .title_widget(&adw::WindowTitle::new("Retriever", ""))
            .build(),
    );
    sidebar_view.set_content(Some(
        &gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&sidebar_list)
            .build(),
    ));
    let sidebar = adw::NavigationPage::builder()
        .title("Retriever")
        .tag("sidebar")
        .child(&sidebar_view)
        .build();

    let split = adw::NavigationSplitView::builder()
        .sidebar(&sidebar)
        .content(&content)
        .min_sidebar_width(200.0)
        .max_sidebar_width(260.0)
        .build();
    let bp = adw::Breakpoint::new(
        adw::BreakpointCondition::parse("max-width: 640sp").expect("a breakpoint"),
    );
    bp.add_setter(&split, "collapsed", Some(&true.to_value()));
    window.add_breakpoint(bp);
    // Phone width: Select is in the main menu (the header would not fit it).
    let phone = adw::Breakpoint::new(
        adw::BreakpointCondition::parse("max-width: 480sp").expect("a breakpoint"),
    );
    phone.add_setter(&split, "collapsed", Some(&true.to_value()));
    phone.add_setter(&select_btn, "visible", Some(&false.to_value()));
    window.add_breakpoint(phone);
    window.set_content(Some(&split));

    let ui = Rc::new(Ui {
        app: app.clone(),
        window: window.clone(),
        manager,
        list: list.clone(),
        selecting: Cell::new(false),
        select_btn: select_btn.clone(),
        action_bar: action_bar.clone(),
        picked: picked.clone(),
        context: Rc::new(Cell::new(None)),
        row_menu: row_menu(),
        title,
        counts,
        category,
        search,
        filter,
        stack,
        empty,
        sorted: sorted.clone(),
        split: split.clone(),
        inhibit: Cell::new(None),
        toasts,
    });

    // ---- Signals ----
    {
        let ui = ui.clone();
        sidebar_list.connect_row_selected(move |_, row| {
            let Some(row) = row else { return };
            let c = Category::SIDEBAR[row.index().max(0) as usize];
            ui.category.set(c);
            ui.filter.changed(gtk::FilterChange::Different);
            ui.split.set_show_content(true);
            if let Some(page) = ui.split.content() {
                page.set_title(c.title());
            }
            // From one empty category to another the list doesn't change (no items-changed):
            // the empty page is told here, or it kept the last one's name.
            ui.update_empty();
            ui.refresh();
        });
    }
    sidebar_list.select_row(sidebar_list.row_at_index(0).as_ref());
    {
        let ui = ui.clone();
        search_entry.connect_search_changed(move |e| {
            *ui.search.borrow_mut() = e.text().to_lowercase();
            ui.filter.changed(gtk::FilterChange::Different);
            ui.update_empty();
        });
    }
    {
        let ui = ui.clone();
        sorted.connect_items_changed(move |_, _, _, _| ui.update_empty());
    }
    {
        let ui = ui.clone();
        list.connect_row_activated(move |_, row| {
            let Some(id) = row_id(row.upcast_ref()) else {
                return;
            };
            if ui.selecting.get() {
                if let Some(o) = ui.object(id) {
                    o.set_picked(!o.picked());
                }
                return;
            }
            match ui.object(id).map(|o| code_status(o.status())) {
                Some(Status::Done) => ui.open_file(id),
                Some(_) => dialogs::progress(&ui, id),
                None => {}
            }
        });
    }
    {
        // Each row's own button: pause, resume, or show the file.
        let ui2 = ui.clone();
        *row_action.borrow_mut() = Some(Rc::new(move |id, ev: RowEvent| {
            match ev {
                RowEvent::Menu(w) => {
                    ui2.pop_menu(id, &w, None);
                    return;
                }
                RowEvent::Picked => {
                    ui2.update_actions();
                    return;
                }
                RowEvent::Button => {}
            }
            let st = ui2.object(id).map(|o| code_status(o.status()));
            match st {
                Some(Status::Downloading | Status::Queued) => ui2.manager.borrow_mut().pause(&[id]),
                Some(Status::Paused | Status::Failed) => ui2.manager.borrow_mut().resume(&[id]),
                Some(Status::Done) => {
                    ui2.open_folder(id);
                    return;
                }
                None => return,
            }
            ui2.after_change();
        }));
    }
    context_menu(&ui);
    drops(&ui);
    {
        let ui = ui.clone();
        select_btn.connect_toggled(move |b| ui.set_selecting(b.is_active()));
    }
    {
        // Delete in the list (not as an accelerator: text fields keep their Delete key).
        let keys = gtk::EventControllerKey::new();
        let window = window.clone();
        let ui2 = ui.clone();
        keys.connect_key_pressed(move |_, key, _, mods| {
            if key == gdk::Key::Delete && mods.is_empty() {
                gio::prelude::ActionGroupExt::activate_action(&window, "delete", None);
                return glib::Propagation::Stop;
            }
            // Ctrl+A in the list (not as an accelerator: text fields keep theirs): Select, all.
            if key == gdk::Key::a && mods == gdk::ModifierType::CONTROL_MASK {
                ui2.set_selecting(true);
                ui2.pick_all();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        list.add_controller(keys);
    }
    actions(&ui);
    for (name, all) in [("select-all", true), ("select-none", false)] {
        let a = gio::SimpleAction::new(name, None);
        let ui2 = ui.clone();
        a.connect_activate(move |_, _| {
            ui2.set_selecting(true);
            if all {
                ui2.pick_all();
            } else {
                ui2.pick_none();
            }
        });
        ui.window.add_action(&a);
    }
    {
        let keys = gtk::EventControllerKey::new();
        let ui2 = ui.clone();
        keys.connect_key_pressed(move |_, key, _, mods| {
            if key == gdk::Key::Escape && mods.is_empty() && ui2.selecting.get() {
                ui2.set_selecting(false);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(keys);
    }

    {
        let ui = ui.clone();
        window.connect_close_request(move |w| {
            let mut m = ui.manager.borrow_mut();
            let (cw, ch) = (w.default_width(), w.default_height());
            m.settings.width = cw;
            m.settings.height = ch;
            model::save_settings(&m.settings);
            m.shutdown();
            glib::Propagation::Proceed
        });
    }
    {
        let ui = ui.clone();
        glib::timeout_add_local(Duration::from_millis(500), move || {
            let t = ui.manager.borrow_mut().tick();
            if t.changed {
                ui.filter.changed(gtk::FilterChange::Different);
            }
            ui.refresh();
            for name in t.finished {
                ui.notify("Download complete", &name);
            }
            for (name, why) in t.failed {
                ui.notify("Download failed", &format!("{name}: {why}"));
            }
            if let Some(then) = t.after {
                dialogs::after_done(&ui, then);
            }
            glib::ControlFlow::Continue
        });
    }
    ui.refresh();
    ui.update_empty();
    window.present();
    ui
}

/// The Keyboard Shortcuts dialog (main menu, or Ctrl+?).
fn shortcuts(window: &adw::ApplicationWindow) {
    let groups: [(&str, &[(&str, &str)]); 3] = [
        (
            "Downloads",
            &[
                ("<Control>n", "Add URL"),
                ("<Control>r", "Resume the selected downloads"),
                ("<Control>p", "Pause the selected downloads"),
                ("Delete", "Delete the selected downloads"),
                ("Menu", "The selected download's menu (or Shift+F10)"),
                ("Return", "Open a finished download, or another's progress"),
            ],
        ),
        (
            "Window",
            &[
                ("<Control>f", "Search"),
                ("<Control>comma", "Preferences"),
                ("<Control>question", "Keyboard shortcuts"),
                ("<Control>w", "Close the window"),
                ("<Control>q", "Quit (downloads pause, and go on next time)"),
            ],
        ),
        (
            "Select",
            &[
                ("<Control>a", "Select every download (in the list)"),
                ("Escape", "Leave Select"),
            ],
        ),
    ];
    crate::native::shortcuts(window, &groups);
}

fn icon_button(icon: &str, tip: &str, action: &str) -> gtk::Button {
    gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tip)
        .action_name(action)
        .build()
}

pub fn apply_style(style: u32) {
    adw::StyleManager::default().set_color_scheme(match style {
        1 => adw::ColorScheme::ForceLight,
        2 => adw::ColorScheme::ForceDark,
        _ => adw::ColorScheme::Default,
    });
}

/// The look of a download's card: its icon in a circle, as GNOME's rows have.
fn style() {
    let css = gtk::CssProvider::new();
    css.load_from_string(
        ".dl-icon { min-width: 36px; min-height: 36px; border-radius: 999px; \
         background-color: alpha(currentColor, 0.08); }\n\
         .dl-row { padding: 10px 6px 10px 12px; }\n\
         .dl-row progressbar { margin-top: 8px; }",
    );
    if let Some(d) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &d,
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

/// A download's id, from its row (or anything in it).
fn row_id(w: &gtk::Widget) -> Option<u64> {
    let mut w = Some(w.clone());
    while let Some(widget) = w {
        if let Some(n) = widget.widget_name().strip_prefix("dl-") {
            return n.parse().ok();
        }
        w = widget.parent();
    }
    None
}

fn bound<'a>(o: &'a DlObject, prop: &'a str, to: &'a impl IsA<glib::Object>, to_prop: &'a str) {
    o.bind_property(prop, to, to_prop).sync_create().build();
}

/// One card: icon, name and where it is, its numbers on the right, its progress under them,
/// its button.
fn download_row(o: &DlObject, action: RowAction) -> gtk::ListBoxRow {
    let circle = gtk::Image::builder()
        .valign(gtk::Align::Center)
        .halign(gtk::Align::Center)
        .build();
    circle.add_css_class("dl-icon");
    bound(o, "symbolic", &circle, "gicon");

    let name = gtk::Label::builder()
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .build();
    name.add_css_class("heading");
    bound(o, "name", &name, "label");
    name.set_has_tooltip(true);
    name.connect_query_tooltip(|label, _, _, _, tip| {
        if !label.layout().is_ellipsized() {
            return false;
        }
        tip.set_text(Some(label.label().as_str()));
        true
    });
    let detail = gtk::Label::builder().xalign(1.0).build();
    detail.add_css_class("numeric");
    bound(o, "detail", &detail, "label");
    let top = gtk::Box::builder().spacing(12).build();
    top.append(&name);
    top.append(&detail);

    let status = gtk::Label::builder()
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    crate::native::dim(&status);
    status.add_css_class("caption");
    bound(o, "status-text", &status, "label");
    let detail2 = gtk::Label::builder().xalign(1.0).build();
    crate::native::dim(&detail2);
    detail2.add_css_class("caption");
    detail2.add_css_class("numeric");
    bound(o, "detail2", &detail2, "label");
    let bottom = gtk::Box::builder().spacing(12).build();
    bottom.append(&status);
    bottom.append(&detail2);

    let bar = gtk::ProgressBar::new();
    bound(o, "fraction", &bar, "fraction");
    bound(o, "show-progress", &bar, "visible");

    let text = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .build();
    text.append(&top);
    text.append(&bottom);
    text.append(&bar);

    let button = gtk::Button::builder().valign(gtk::Align::Center).build();
    button.add_css_class("flat");
    button.add_css_class("circular");
    bound(o, "action-icon", &button, "icon-name");
    bound(o, "action-tip", &button, "tooltip-text");
    {
        let id = o.id();
        let action = action.clone();
        button.connect_clicked(move |_| {
            if let Some(f) = action.borrow().as_ref() {
                f(id, RowEvent::Button);
            }
        });
    }
    let more = gtk::Button::builder()
        .icon_name("view-more-symbolic")
        .tooltip_text("More")
        .valign(gtk::Align::Center)
        .build();
    more.add_css_class("flat");
    more.add_css_class("circular");
    {
        let id = o.id();
        let action = action.clone();
        more.connect_clicked(move |b| {
            if let Some(f) = action.borrow().as_ref() {
                f(id, RowEvent::Menu(b.clone().upcast()));
            }
        });
    }

    // Select's check, before the icon.
    let check = gtk::CheckButton::builder()
        .valign(gtk::Align::Center)
        .build();
    check.add_css_class("selection-mode");
    o.bind_property("picked", &check, "active")
        .bidirectional()
        .sync_create()
        .build();
    bound(o, "picking", &check, "visible");
    {
        let id = o.id();
        let action = action.clone();
        check.connect_toggled(move |_| {
            if let Some(f) = action.borrow().as_ref() {
                f(id, RowEvent::Picked);
            }
        });
    }
    let hbox = gtk::Box::builder().spacing(12).build();
    hbox.append(&check);
    hbox.append(&circle);
    hbox.append(&text);
    hbox.append(&button);
    hbox.append(&more);
    let row = gtk::ListBoxRow::builder()
        .child(&hbox)
        .activatable(true)
        .build();
    row.add_css_class("dl-row");
    row.set_widget_name(&format!("dl-{}", o.id()));
    // No tooltip on the row (GNOME's lists have none: the address is in the download's
    // window). Only a name cut short shows itself whole, as Files does.
    row
}

/// A download's menu.
fn row_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let a = gio::Menu::new();
    a.append(Some("_Open"), Some("win.open-file"));
    a.append(Some("Open _Containing Folder"), Some("win.open-folder"));
    a.append(Some("_Properties"), Some("win.properties"));
    menu.append_section(None, &a);
    let b = gio::Menu::new();
    b.append(Some("_Resume"), Some("win.resume"));
    b.append(Some("_Pause"), Some("win.pause"));
    b.append(Some("Re_start"), Some("win.restart"));
    b.append(Some("_Scheduled"), Some("win.schedule"));
    b.append(Some("_Extract Here"), Some("win.extract"));
    menu.append_section(None, &b);
    let c = gio::Menu::new();
    c.append(Some("Copy _Address"), Some("win.copy-url"));
    c.append(Some("_Delete…"), Some("win.delete"));
    menu.append_section(None, &c);
    menu
}

/// A row's menu also comes with a right click, a long press, or the Menu key or Shift+F10.
fn context_menu(ui: &UiRc) {
    let click = gtk::GestureClick::builder()
        .button(gdk::BUTTON_SECONDARY)
        .build();
    {
        let ui2 = ui.clone();
        click.connect_pressed(move |g, _, x, y| {
            if let Some(row) = ui2.list.row_at_y(y as i32)
                && let Some(id) = row_id(row.upcast_ref())
            {
                g.set_state(gtk::EventSequenceState::Claimed);
                ui2.pop_menu(id, ui2.list.upcast_ref(), Some((x, y)));
            }
        });
    }
    ui.list.add_controller(click);
    let press = gtk::GestureLongPress::builder().touch_only(true).build();
    {
        let ui2 = ui.clone();
        press.connect_pressed(move |g, x, y| {
            if let Some(row) = ui2.list.row_at_y(y as i32)
                && let Some(id) = row_id(row.upcast_ref())
            {
                g.set_state(gtk::EventSequenceState::Claimed);
                ui2.pop_menu(id, ui2.list.upcast_ref(), Some((x, y)));
            }
        });
    }
    ui.list.add_controller(press);
    let keys = gtk::EventControllerKey::new();
    {
        let ui2 = ui.clone();
        keys.connect_key_pressed(move |_, key, _, mods| {
            let menu_key = key == gdk::Key::Menu
                || (key == gdk::Key::F10 && mods == gdk::ModifierType::SHIFT_MASK);
            if !menu_key {
                return glib::Propagation::Proceed;
            }
            if let Some(row) = ui2.focused_row()
                && let Some(id) = row_id(row.upcast_ref())
            {
                ui2.pop_menu(id, row.upcast_ref(), None);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
    }
    ui.list.add_controller(keys);
}

/// A link dropped on the window opens Add URL with it.
fn drops(ui: &UiRc) {
    let target = gtk::DropTarget::new(String::static_type(), gdk::DragAction::COPY);
    let ui2 = ui.clone();
    target.connect_drop(move |_, value, _, _| {
        if let Ok(text) = value.get::<String>() {
            dialogs::add_url(&ui2, Some(text));
            return true;
        }
        false
    });
    ui.window.add_controller(target);
}

fn actions(ui: &UiRc) {
    let add = |name: &str, f: Box<dyn Fn(&UiRc)>| {
        let a = gio::SimpleAction::new(name, None);
        let ui2 = ui.clone();
        a.connect_activate(move |_, _| f(&ui2));
        ui.window.add_action(&a);
    };
    add("add", Box::new(|ui| dialogs::add_url(ui, None)));
    add(
        "resume",
        Box::new(|ui| {
            let ids = ui.selected();
            ui.manager.borrow_mut().resume(&ids);
            ui.after_change();
        }),
    );
    add(
        "pause",
        Box::new(|ui| {
            let ids = ui.selected();
            ui.manager.borrow_mut().pause(&ids);
            ui.after_change();
        }),
    );
    add(
        "restart",
        Box::new(|ui| {
            let ids = ui.selected();
            ui.manager.borrow_mut().restart(&ids);
            ui.after_change();
        }),
    );
    add(
        "resume-all",
        Box::new(|ui| {
            let ids = ui.manager.borrow().ids_where(|_| true);
            ui.manager.borrow_mut().resume(&ids);
            ui.after_change();
        }),
    );
    add(
        "pause-all",
        Box::new(|ui| {
            let ids = ui.manager.borrow().ids_where(|_| true);
            ui.manager.borrow_mut().pause(&ids);
            ui.after_change();
        }),
    );
    add(
        "delete",
        Box::new(|ui| {
            let ids = ui.selected();
            if !ids.is_empty() {
                dialogs::confirm_delete(ui, ids);
            }
        }),
    );
    add(
        "delete-completed",
        Box::new(|ui| {
            let ids = ui
                .manager
                .borrow()
                .ids_where(|e| e.item.status == Status::Done);
            if ids.is_empty() {
                return;
            }
            ui.manager.borrow_mut().remove(&ids, false);
            ui.after_change();
            ui.toast(&format!(
                "{} completed download{} removed from the list",
                ids.len(),
                if ids.len() == 1 { "" } else { "s" }
            ));
        }),
    );
    add(
        "add-video",
        Box::new(|ui| crate::video::dialog(ui, None, Default::default())),
    );
    add(
        "add-torrent",
        Box::new(|ui| crate::torrentdlg::dialog(ui, None)),
    );
    add(
        "grab",
        Box::new(|ui| crate::grabber::dialog(ui, None, Vec::new(), Default::default())),
    );
    add("scheduler", Box::new(dialogs::scheduler));
    add("shortcuts", Box::new(|ui| shortcuts(&ui.window)));
    add(
        "select-mode",
        Box::new(|ui| ui.set_selecting(!ui.selecting.get())),
    );
    add(
        "extract",
        Box::new(|ui| {
            let ids = ui.selected();
            ui.manager.borrow_mut().extract(&ids);
            ui.after_change();
        }),
    );
    {
        // Scheduled: a check in the menu, for the selected downloads.
        let a = gio::SimpleAction::new_stateful("schedule", None, &false.to_variant());
        let ui2 = ui.clone();
        a.connect_activate(move |a, _| {
            let on = !a.state().and_then(|v| v.get::<bool>()).unwrap_or(false);
            let ids = ui2.selected();
            ui2.manager.borrow_mut().set_scheduled(&ids, on);
            a.set_state(&on.to_variant());
            ui2.after_change();
            if on && !ui2.manager.borrow().settings.schedule.enabled {
                ui2.toast("The scheduler is off: turn it on in Scheduler…");
            }
        });
        ui.window.add_action(&a);
    }
    add(
        "open-file",
        Box::new(|ui| {
            if let Some(id) = ui.selected().first() {
                ui.open_file(*id);
            }
        }),
    );
    add(
        "open-folder",
        Box::new(|ui| {
            if let Some(id) = ui.selected().first() {
                ui.open_folder(*id);
            }
        }),
    );
    add(
        "properties",
        Box::new(|ui| {
            if let Some(id) = ui.selected().first() {
                dialogs::progress(ui, *id);
            }
        }),
    );
    add(
        "copy-url",
        Box::new(|ui| {
            let urls: Vec<String> = {
                let m = ui.manager.borrow();
                ui.selected()
                    .iter()
                    .filter_map(|id| m.entry(*id).map(|e| e.item.url.clone()))
                    .collect()
            };
            if !urls.is_empty() {
                ui.window.clipboard().set_text(&urls.join("\n"));
                ui.toast("Address copied");
            }
        }),
    );
    ui.update_actions();
}

impl Ui {
    /// The downloads an action is for: Select's selection; else the one whose menu is open,
    /// or the focused one (its keys).
    pub fn selected(&self) -> Vec<u64> {
        if self.selecting.get() {
            return (0..self.sorted.n_items())
                .filter_map(|i| self.sorted.item(i).and_downcast::<DlObject>())
                .filter(|o| o.picked())
                .map(|o| o.id())
                .collect();
        }
        self.context
            .get()
            .or_else(|| self.focused_row().and_then(|r| row_id(r.upcast_ref())))
            .into_iter()
            .collect()
    }

    fn focused_row(&self) -> Option<gtk::ListBoxRow> {
        self.list
            .root()
            .and_then(|r| r.focus())
            .and_then(|f| {
                f.downcast_ref::<gtk::ListBoxRow>()
                    .cloned()
                    .or_else(|| f.ancestor(gtk::ListBoxRow::static_type()).and_downcast())
            })
            .filter(|r| r.is_ancestor(&self.list))
    }

    /// Nothing selected (Select stays on).
    pub fn pick_none(&self) {
        let store = self.manager.borrow().store.clone();
        for i in 0..store.n_items() {
            if let Some(o) = store.item(i).and_downcast::<DlObject>() {
                o.set_picked(false);
            }
        }
        self.update_actions();
    }

    /// Select every shown download.
    pub fn pick_all(&self) {
        for i in 0..self.sorted.n_items() {
            if let Some(o) = self.sorted.item(i).and_downcast::<DlObject>() {
                o.set_picked(true);
            }
        }
        self.update_actions();
    }

    /// Select on or off.
    pub fn set_selecting(&self, on: bool) {
        self.selecting.set(on);
        self.context.set(None);
        if self.select_btn.is_active() != on {
            self.select_btn.set_active(on);
        }
        // Every download's check (the hidden ones too: the filter can change while picking).
        let store = self.manager.borrow().store.clone();
        for i in 0..store.n_items() {
            if let Some(o) = store.item(i).and_downcast::<DlObject>() {
                o.set_picking(on);
                if !on {
                    o.set_picked(false);
                }
            }
        }
        self.action_bar.set_revealed(on);
        self.update_actions();
    }

    /// Download `id`'s menu, from `parent` (at `at` in it, else pointing at it).
    pub fn pop_menu(&self, id: u64, parent: &gtk::Widget, at: Option<(f64, f64)>) {
        self.context.set(Some(id));
        self.update_actions();
        let pop = gtk::PopoverMenu::from_model(Some(&self.row_menu));
        pop.set_parent(parent);
        if let Some((x, y)) = at {
            pop.set_has_arrow(false);
            pop.set_halign(gtk::Align::Start);
            pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        }
        let ctx = self.context.clone();
        pop.connect_closed(move |p| {
            let p = p.clone();
            let ctx = ctx.clone();
            // Once the menu's action ran: the keys are the focused row's again.
            glib::idle_add_local_once(move || {
                ctx.set(None);
                p.unparent();
            });
        });
        pop.popup();
    }

    /// The row object of download `id`, as the list shows it.
    pub fn object(&self, id: u64) -> Option<DlObject> {
        (0..self.sorted.n_items())
            .filter_map(|i| self.sorted.item(i).and_downcast::<DlObject>())
            .find(|o| o.id() == id)
    }

    fn selected_statuses(&self) -> Vec<Status> {
        self.selected()
            .into_iter()
            .filter_map(|id| self.object(id))
            .map(|o| code_status(o.status()))
            .collect()
    }

    fn enable(&self, name: &str, on: bool) {
        if let Some(a) = self
            .window
            .lookup_action(name)
            .and_downcast::<gio::SimpleAction>()
        {
            a.set_enabled(on);
        }
    }

    pub fn update_actions(&self) {
        let st = self.selected_statuses();
        let any = !st.is_empty();
        if self.selecting.get() {
            self.picked.set_label(&match st.len() {
                0 => "Nothing Selected".to_owned(),
                n => format!("{n} Selected"),
            });
        }
        self.enable(
            "resume",
            st.iter()
                .any(|s| matches!(s, Status::Paused | Status::Failed)),
        );
        self.enable(
            "pause",
            st.iter()
                .any(|s| matches!(s, Status::Downloading | Status::Queued | Status::Failed)),
        );
        self.enable(
            "restart",
            st.iter()
                .any(|s| matches!(s, Status::Paused | Status::Failed | Status::Done)),
        );
        self.enable("delete", any);
        self.enable("copy-url", any);
        self.enable("properties", st.len() == 1);
        self.enable("open-file", st.len() == 1 && st[0] == Status::Done);
        self.enable("open-folder", st.len() == 1);
        self.enable("extract", st.len() == 1 && st[0] == Status::Done);
        self.enable("schedule", any && st.iter().all(|s| *s != Status::Done));
        // The check shows the first selected download's.
        if let Some(a) = self
            .window
            .lookup_action("schedule")
            .and_downcast::<gio::SimpleAction>()
            && let Some(id) = self.selected().first()
            && let Ok(m) = self.manager.try_borrow()
            && let Some(e) = m.entry(*id)
        {
            a.set_state(&e.item.scheduled.to_variant());
        }
    }

    /// After a change from the window (a button, a dialog).
    pub fn after_change(&self) {
        self.filter.changed(gtk::FilterChange::Different);
        self.refresh();
    }

    /// The sidebar's counts, the title's summary, the actions, suspend.
    pub fn refresh(&self) {
        let (downloading, speed, total, waiting) = {
            let m = self.manager.borrow();
            for (c, label) in &self.counts {
                let n = m.count(*c);
                label.set_label(&if n > 0 { n.to_string() } else { String::new() });
            }
            (
                m.downloading(),
                m.total_speed(),
                m.entries.len(),
                m.schedule_waiting(),
            )
        };
        self.title.set_title(self.category.get().title());
        let shown = self.sorted.n_items() as usize;
        let mut sub = if shown == total {
            format!("{total} download{}", if total == 1 { "" } else { "s" })
        } else {
            format!("{shown} of {total}")
        };
        if downloading > 0 {
            sub.push_str(&format!(
                " · {downloading} downloading{}",
                if speed >= 1.0 {
                    format!(", {}", model::speed_text(speed))
                } else {
                    String::new()
                }
            ));
        }
        self.title.set_subtitle(&sub);
        self.update_actions();
        // No suspend while something downloads (it would cut the connections), nor while the
        // scheduler waits for its start time.
        let awake = downloading > 0 || waiting;
        match (awake, self.inhibit.get()) {
            (true, None) => {
                let c = self.app.inhibit(
                    Some(&self.window),
                    gtk::ApplicationInhibitFlags::SUSPEND,
                    Some(if downloading > 0 {
                        "Downloading files"
                    } else {
                        "Waiting for scheduled downloads"
                    }),
                );
                if c != 0 {
                    self.inhibit.set(Some(c));
                }
            }
            (false, Some(c)) => {
                self.app.uninhibit(c);
                self.inhibit.set(None);
            }
            _ => {}
        }
    }

    fn update_empty(&self) {
        let empty = self.sorted.n_items() == 0;
        if empty {
            let c = self.category.get();
            if !self.search.borrow().is_empty() {
                self.empty.set_icon_name(Some("system-search-symbolic"));
                self.empty.set_title("No Results");
                self.empty
                    .set_description(Some("No download's name matches the search"));
                if let Some(b) = self.empty.child() {
                    b.set_visible(false);
                }
            } else if c == Category::All {
                self.empty.set_icon_name(Some("folder-download-symbolic"));
                self.empty.set_title("No Downloads");
                self.empty
                    .set_description(Some("Add a link with the + button, or drop one here"));
                if let Some(b) = self.empty.child() {
                    b.set_visible(true);
                }
            } else {
                self.empty.set_icon_name(Some(c.icon()));
                self.empty.set_title(&format!("Nothing in {}", c.title()));
                self.empty.set_description(None);
                if let Some(b) = self.empty.child() {
                    b.set_visible(false);
                }
            }
        }
        self.stack
            .set_visible_child_name(if empty { "empty" } else { "list" });
    }

    /// Quits: an open dialog would only close itself, so dialogs go first.
    pub fn quit(&self) {
        while let Some(d) = self.window.visible_dialog() {
            d.force_close();
        }
        self.window.close();
    }

    pub fn toast(&self, text: &str) {
        self.toasts.add_toast(adw::Toast::new(text));
    }

    fn notify(&self, title: &str, body: &str) {
        if !self.manager.borrow().settings.notify || self.window.is_active() {
            if self.window.is_active() {
                self.toast(&format!("{title}: {body}"));
            }
            return;
        }
        let n = gio::Notification::new(title);
        n.set_body(Some(body));
        n.set_default_action("app.activate-window");
        self.app.send_notification(None, &n);
    }

    pub fn open_file(&self, id: u64) {
        let file = self
            .manager
            .borrow()
            .entry(id)
            .and_then(|e| e.item.file.clone());
        if let Some(f) = file {
            gtk::FileLauncher::new(Some(&gio::File::for_path(f))).launch(
                Some(&self.window),
                gio::Cancellable::NONE,
                |_| {},
            );
        }
    }

    pub fn open_folder(&self, id: u64) {
        let (file, folder) = match self.manager.borrow().entry(id) {
            Some(e) => (e.item.file.clone(), e.item.folder.clone()),
            None => return,
        };
        match file.filter(|f| f.exists()) {
            Some(f) => gtk::FileLauncher::new(Some(&gio::File::for_path(f)))
                .open_containing_folder(Some(&self.window), gio::Cancellable::NONE, |_| {}),
            None => gtk::FileLauncher::new(Some(&gio::File::for_path(folder))).launch(
                Some(&self.window),
                gio::Cancellable::NONE,
                |_| {},
            ),
        }
    }
}
