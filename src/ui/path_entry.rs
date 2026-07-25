//! The `Ctrl+L` path entry, with Tab completion.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk::gdk;

use crate::model::completion::{self, Candidate};

type ActivateHandler = Rc<dyn Fn(PathBuf)>;
type CancelHandler = Rc<dyn Fn()>;

pub struct PathEntry {
    entry: gtk::Entry,
    on_activate: RefCell<Option<ActivateHandler>>,
    on_cancel: RefCell<Option<CancelHandler>>,
}

impl PathEntry {
    pub fn new() -> Rc<Self> {
        // No width_request. This entry lives in a GtkStack alongside the
        // breadcrumb, and a Stack's minimum width is the maximum over all its
        // children whether or not they are showing — so a request here becomes
        // the header bar's floor, and through it the whole window's. hexpand
        // gives it the room it needs when it is actually visible.
        let entry = gtk::Entry::builder()
            .placeholder_text("Type a path…")
            .hexpand(true)
            .build();
        entry.add_css_class("hive-path-entry");

        let this = Rc::new(Self {
            entry,
            on_activate: RefCell::new(None),
            on_cancel: RefCell::new(None),
        });

        this.wire();
        this
    }

    pub fn widget(&self) -> &gtk::Entry {
        &self.entry
    }

    pub fn connect_activate(self: &Rc<Self>, handler: impl Fn(PathBuf) + 'static) {
        *self.on_activate.borrow_mut() = Some(Rc::new(handler));
    }

    pub fn connect_cancel(self: &Rc<Self>, handler: impl Fn() + 'static) {
        *self.on_cancel.borrow_mut() = Some(Rc::new(handler));
    }

    /// Show `path` and select it all, so typing replaces it.
    pub fn focus_with(&self, path: &Path) {
        self.entry.set_text(&path.to_string_lossy());
        self.entry.grab_focus();
        self.entry.select_region(0, -1);
    }

    fn wire(self: &Rc<Self>) {
        let this = Rc::clone(self);
        self.entry.connect_activate(move |entry| {
            let text = entry.text().to_string();
            if text.trim().is_empty() {
                this.cancel();
                return;
            }
            let expanded = completion::expand(text.trim(), &crate::paths::home_dir());
            let handler = this.on_activate.borrow().clone();
            if let Some(handler) = handler {
                handler(PathBuf::from(expanded));
            }
        });

        let controller = gtk::EventControllerKey::new();
        let this = Rc::clone(self);
        controller.connect_key_pressed(move |_, key, _, _| match key {
            gdk::Key::Escape => {
                this.cancel();
                glib::Propagation::Stop
            }
            gdk::Key::Tab => {
                this.complete();
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        });
        self.entry.add_controller(controller);
    }

    fn cancel(&self) {
        let handler = self.on_cancel.borrow().clone();
        if let Some(handler) = handler {
            handler();
        }
    }

    /// Complete the current text and put the caret at the end.
    fn complete(&self) {
        let text = self.entry.text().to_string();
        let home = crate::paths::home_dir();
        let result = completion::complete(&text, &home, list_directory);

        if result.matches.is_empty() {
            return;
        }

        if result.text != text {
            self.entry.set_text(&result.text);
        }
        self.entry.set_position(-1);
    }
}

/// Enumerate `dir` for the completion model.
///
/// Synchronous, which is the one place Hive reads a directory on the main
/// thread. It is bounded and deliberate: it runs only on an explicit Tab press,
/// against a single directory, and reads names only. Making it async would mean
/// the completion landing after the user has typed more, which is worse.
fn list_directory(dir: &Path) -> Vec<Candidate> {
    let file = gio::File::for_path(dir);
    let Ok(enumerator) = file.enumerate_children(
        "standard::name,standard::type",
        gio::FileQueryInfoFlags::NONE,
        gio::Cancellable::NONE,
    ) else {
        return Vec::new();
    };

    let mut candidates = Vec::new();
    for info in enumerator.flatten() {
        let name = info.name();
        let Some(name) = name.to_str() else {
            continue;
        };
        candidates.push(Candidate {
            name: name.to_owned(),
            is_dir: info.file_type() == gio::FileType::Directory,
        });
    }
    candidates
}
