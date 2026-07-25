//! The progress dialog for long operations.
//!
//! Presentation is deferred: an operation that finishes quickly never puts a
//! dialog on screen at all, which is most of them. The bar runs indeterminate
//! while the sources are being walked, because until that finishes there is no
//! honest denominator to show.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;

/// How long an operation runs before it is worth interrupting the user for.
pub const SHOW_AFTER: std::time::Duration = std::time::Duration::from_millis(400);

pub struct Progress {
    dialog: adw::AlertDialog,
    bar: gtk::ProgressBar,
    detail: gtk::Label,
    strategy: gtk::Label,
    presented: Cell<bool>,
    closed: Cell<bool>,
}

impl Progress {
    pub fn new(title: &str) -> Rc<Self> {
        let bar = gtk::ProgressBar::builder().show_text(false).build();

        let detail = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .single_line_mode(true)
            .build();
        detail.add_css_class("caption");
        detail.add_css_class("dim-label");

        let strategy = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .visible(false)
            .build();
        strategy.add_css_class("caption");

        let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
        column.set_width_request(260);
        column.append(&strategy);
        column.append(&bar);
        column.append(&detail);

        let dialog = adw::AlertDialog::new(Some(title), None);
        dialog.set_extra_child(Some(&column));
        dialog.add_responses(&[("cancel", "Cancel")]);
        dialog.set_close_response("cancel");

        Rc::new(Self {
            dialog,
            bar,
            detail,
            strategy,
            presented: Cell::new(false),
            closed: Cell::new(false),
        })
    }

    /// Called when the user asks to stop, including by dismissing the dialog.
    pub fn connect_cancel(self: &Rc<Self>, on_cancel: impl Fn() + 'static) {
        self.dialog.connect_response(None, move |_, _| on_cancel());
    }

    /// Show the dialog, unless the operation already finished.
    pub fn present(self: &Rc<Self>, parent: &impl IsA<gtk::Widget>) {
        if self.presented.replace(true) || self.closed.get() {
            return;
        }
        self.dialog.present(Some(parent));
    }

    pub fn is_presented(&self) -> bool {
        self.presented.get()
    }

    /// What is about to happen — a rename is instant, a copy is not.
    pub fn set_strategy(&self, text: &str) {
        self.strategy.set_text(text);
        self.strategy.set_visible(!text.is_empty());
    }

    /// No denominator yet, so pulse rather than lie about a fraction.
    pub fn pulse(&self, detail: &str) {
        self.bar.pulse();
        self.detail.set_text(detail);
    }

    pub fn set_fraction(&self, fraction: f64, detail: &str) {
        self.bar.set_fraction(fraction.clamp(0.0, 1.0));
        self.detail.set_text(detail);
    }

    pub fn close(&self) {
        if self.closed.replace(true) {
            return;
        }
        if self.presented.get() {
            self.dialog.close();
        }
    }
}
