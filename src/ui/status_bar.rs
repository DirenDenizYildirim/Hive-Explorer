//! The thin status line: item count, selection count, free space.
//!
//! Updates are debounced on a 150 ms window (see [`crate::ui::debounce`]) so a
//! directory under heavy churn cannot thrash the view.

use adw::prelude::*;

use crate::model::format;

pub struct StatusBar {
    container: gtk::Box,
    items: gtk::Label,
    selection: gtk::Label,
    free_space: gtk::Label,
    spinner: gtk::Spinner,
}

impl Default for StatusBar {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusBar {
    pub fn new() -> Self {
        let container = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        container.add_css_class("hive-status-bar");

        let spinner = gtk::Spinner::new();
        spinner.set_visible(false);

        let items = segment_label();
        let selection = segment_label();
        let free_space = segment_label();
        free_space.set_hexpand(true);
        free_space.set_xalign(1.0);

        container.append(&spinner);
        container.append(&items);
        container.append(&selection);
        container.append(&free_space);

        Self {
            container,
            items,
            selection,
            free_space,
            spinner,
        }
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.container
    }

    /// Refresh the counts.
    ///
    /// `visible` is the post-filter count and `total` the raw directory count;
    /// when a filter is hiding entries we say so, because "3 items" in a
    /// directory of 400 is otherwise alarming.
    pub fn set_counts(&self, visible: u32, total: u32, selected: u32, loading: bool) {
        let text = if loading {
            // While enumerating, the count is a running total, not a final one.
            format!("{}…", format::item_count(visible as usize))
        } else if visible == total {
            format::item_count(visible as usize)
        } else {
            format!("{} of {}", visible, format::item_count(total as usize))
        };
        self.items.set_text(&text);

        let selection_text = format::selection_count(selected as usize);
        self.selection.set_visible(!selection_text.is_empty());
        self.selection.set_text(&selection_text);

        self.spinner.set_visible(loading);
        if loading {
            self.spinner.start();
        } else {
            self.spinner.stop();
        }
    }

    pub fn set_free_space(&self, text: &str) {
        self.free_space.set_visible(!text.is_empty());
        self.free_space.set_text(text);
    }

    /// Query free space for `location` without blocking the main context.
    pub fn update_free_space_for(&self, location: &gio::File) {
        let label = self.free_space.clone();
        location.query_filesystem_info_async(
            gio::FILE_ATTRIBUTE_FILESYSTEM_FREE,
            glib::Priority::DEFAULT_IDLE,
            gio::Cancellable::NONE,
            move |result| match result {
                Ok(info) => {
                    let free = info.attribute_uint64(gio::FILE_ATTRIBUTE_FILESYSTEM_FREE);
                    let text = format!("{} free", format::human_bytes(free));
                    label.set_visible(true);
                    label.set_text(&text);
                }
                Err(error) => {
                    // Trash, or a backend that does not report free space. Not
                    // worth a banner — just say nothing.
                    tracing::debug!(%error, "free space unavailable");
                    label.set_visible(false);
                    label.set_text("");
                }
            },
        );
    }
}

fn segment_label() -> gtk::Label {
    let label = gtk::Label::builder()
        .xalign(0.0)
        .single_line_mode(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    label.add_css_class("hive-status-segment");
    label
}
