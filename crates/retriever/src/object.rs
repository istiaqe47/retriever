//! One row of the download list as a GObject: the list's cells, its filters and sorters read
//! these properties only (never the manager), so they can run while the manager changes.

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use std::cell::{Cell, RefCell};

mod imp {
    use super::*;

    #[derive(glib::Properties, Default)]
    #[properties(wrapper_type = super::DlObject)]
    pub struct DlObject {
        #[property(get, set)]
        id: Cell<u64>,
        /// "dl-<id>": the name of every cell's widget, so a click finds its row.
        #[property(get, set)]
        widget_name: RefCell<String>,
        #[property(get, set)]
        name: RefCell<String>,
        #[property(get, set, nullable)]
        gicon: RefCell<Option<gio::Icon>>,
        #[property(get, set)]
        size_text: RefCell<String>,
        #[property(get, set)]
        size_bytes: Cell<u64>,
        #[property(get, set)]
        status_text: RefCell<String>,
        /// model::Status as a number (0 queued … 4 failed).
        #[property(get, set)]
        status: Cell<u32>,
        /// The file's kind (model::Category as a number).
        #[property(get, set)]
        kind: Cell<u32>,
        #[property(get, set)]
        fraction: Cell<f64>,
        #[property(get, set)]
        show_progress: Cell<bool>,
        #[property(get, set)]
        show_label: Cell<bool>,
        #[property(get, set)]
        speed_text: RefCell<String>,
        #[property(get, set)]
        eta_text: RefCell<String>,
        #[property(get, set)]
        added: Cell<i64>,
        #[property(get, set)]
        added_text: RefCell<String>,
        #[property(get, set)]
        tooltip: RefCell<String>,
        /// The row's small icon (its file type's, symbolic), in a circle.
        #[property(get, set, nullable)]
        symbolic: RefCell<Option<gio::Icon>>,
        /// The row's right side: its speed (or size), and under it what's left (or when).
        #[property(get, set)]
        detail: RefCell<String>,
        #[property(get, set)]
        detail2: RefCell<String>,
        /// The row's own button: pause, resume, or show the file.
        #[property(get, set)]
        action_icon: RefCell<String>,
        #[property(get, set)]
        action_tip: RefCell<String>,
        /// Select: its check is shown (`picking`) and ticked (`picked`).
        #[property(get, set)]
        picking: Cell<bool>,
        #[property(get, set)]
        picked: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DlObject {
        const NAME: &'static str = "MdmDlObject";
        type Type = super::DlObject;
    }

    #[glib::derived_properties]
    impl ObjectImpl for DlObject {}
}

glib::wrapper! {
    pub struct DlObject(ObjectSubclass<imp::DlObject>);
}

impl DlObject {
    pub fn new(id: u64) -> Self {
        let o: Self = glib::Object::builder().build();
        o.set_id(id);
        o.set_widget_name(format!("dl-{id}"));
        o
    }

    /// Sets a text property only when it changed (no needless redraws every tick).
    pub fn put(&self, prop: &str, value: &str) {
        if self.property::<String>(prop) != value {
            self.set_property(prop, value);
        }
    }
}
