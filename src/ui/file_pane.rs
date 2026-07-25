//! The file pane: one model stack, two views.
//!
//! ```text
//! gtk::DirectoryList        async, incremental, monitored
//!   └─ gtk::FilterListModel   hidden-file toggle, Ctrl+F substring filter
//!        └─ gtk::SortListModel  name / size / modified / type, folders-first
//!             └─ gtk::MultiSelection
//!                  └─ gtk::ColumnView | gtk::GridView
//! ```
//!
//! `DirectoryList` already does progressive loading, cancellation on
//! navigate-away, and directory monitoring, all upstream-tested. Hive adds no
//! enumeration logic of its own — the decision rules live in `crate::model` as
//! plain functions, and the `CustomFilter`/`CustomSorter` here are thin shims
//! that read a `gio::FileInfo` into those functions' input types.
//!
//! Both views bind to the *same* selection model, so toggling between them is a
//! stack page switch and never re-enumerates the directory.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib::clone;

use crate::config::ViewMode;
use crate::model::filter::{self, FilterInput, FilterSpec};
use crate::model::sort::{self, SortKeyData, SortSpec};

/// Attributes requested from every entry.
///
/// Kept to what the view actually renders and sorts by. Asking for more makes
/// enumeration slower on large directories; `standard::file` is supplied by
/// `DirectoryList` itself and does not need requesting.
const ATTRIBUTES: &str = concat!(
    "standard::name,",
    "standard::display-name,",
    "standard::type,",
    "standard::size,",
    "standard::is-hidden,",
    "standard::is-backup,",
    "standard::is-symlink,",
    "standard::symlink-target,",
    "standard::symbolic-icon,",
    "standard::content-type,",
    "time::modified",
);

/// Mutable view state read by the filter and sorter shims.
#[derive(Debug, Clone, Default)]
struct ViewState {
    filter: FilterSpec,
    sort: SortSpec,
}

pub struct FilePane {
    container: gtk::Stack,
    directory_list: gtk::DirectoryList,
    filter_model: gtk::FilterListModel,
    sort_model: gtk::SortListModel,
    selection: gtk::MultiSelection,
    custom_filter: gtk::CustomFilter,
    custom_sorter: gtk::CustomSorter,
    state: Rc<RefCell<ViewState>>,
    column_view: gtk::ColumnView,
    grid_view: gtk::GridView,
}

impl FilePane {
    pub fn new(filter_spec: FilterSpec, sort_spec: SortSpec) -> Rc<Self> {
        let state = Rc::new(RefCell::new(ViewState {
            filter: filter_spec,
            sort: sort_spec,
        }));

        let directory_list = gtk::DirectoryList::new(Some(ATTRIBUTES), gio::File::NONE);
        // Enumerate below the frame clock so a huge directory cannot starve
        // redraws: the list stays interactive while it fills.
        directory_list.set_io_priority(glib::Priority::DEFAULT_IDLE);
        directory_list.set_monitored(true);

        let custom_filter = gtk::CustomFilter::new(clone!(
            #[strong]
            state,
            move |object| {
                let Some(info) = object.downcast_ref::<gio::FileInfo>() else {
                    return true;
                };
                let name = display_name(info);
                let input = FilterInput::new(&name, info.is_hidden(), info.is_backup());
                filter::matches(&input, &state.borrow().filter)
            }
        ));

        let filter_model =
            gtk::FilterListModel::new(Some(directory_list.clone()), Some(custom_filter.clone()));
        // Filter in chunks so a 100k-entry directory does not block the frame.
        filter_model.set_incremental(true);

        let custom_sorter = gtk::CustomSorter::new(clone!(
            #[strong]
            state,
            move |a, b| {
                let (Some(a), Some(b)) = (
                    a.downcast_ref::<gio::FileInfo>(),
                    b.downcast_ref::<gio::FileInfo>(),
                ) else {
                    return gtk::Ordering::Equal;
                };

                let a_name = display_name(a);
                let b_name = display_name(b);
                let a_type = content_type(a);
                let b_type = content_type(b);

                let left = SortKeyData {
                    name: &a_name,
                    is_dir: is_directory(a),
                    size: a.size(),
                    modified: modified_seconds(a),
                    content_type: &a_type,
                };
                let right = SortKeyData {
                    name: &b_name,
                    is_dir: is_directory(b),
                    size: b.size(),
                    modified: modified_seconds(b),
                    content_type: &b_type,
                };

                sort::compare(&left, &right, state.borrow().sort).into()
            }
        ));

        let sort_model =
            gtk::SortListModel::new(Some(filter_model.clone()), Some(custom_sorter.clone()));
        sort_model.set_incremental(true);

        let selection = gtk::MultiSelection::new(Some(sort_model.clone()));

        let column_view = build_column_view(&selection);
        let grid_view = build_grid_view(&selection);

        let container = gtk::Stack::new();
        container.set_transition_type(gtk::StackTransitionType::None);
        container.add_named(&scrolled(&column_view), Some(ViewMode::List.id()));
        container.add_named(&scrolled(&grid_view), Some(ViewMode::Grid.id()));
        container.add_css_class("hive-file-pane");

        Rc::new(Self {
            container,
            directory_list,
            filter_model,
            sort_model,
            selection,
            custom_filter,
            custom_sorter,
            state,
            column_view,
            grid_view,
        })
    }

    pub fn widget(&self) -> &gtk::Stack {
        &self.container
    }

    pub fn selection(&self) -> &gtk::MultiSelection {
        &self.selection
    }

    pub fn directory_list(&self) -> &gtk::DirectoryList {
        &self.directory_list
    }

    pub fn column_view(&self) -> &gtk::ColumnView {
        &self.column_view
    }

    pub fn grid_view(&self) -> &gtk::GridView {
        &self.grid_view
    }

    /// Number of entries currently passing the filter.
    pub fn visible_count(&self) -> u32 {
        self.filter_model.n_items()
    }

    /// Number of entries the directory reported, before filtering.
    pub fn total_count(&self) -> u32 {
        self.directory_list.n_items()
    }

    pub fn selected_count(&self) -> u32 {
        self.selection.selection().size() as u32
    }

    /// Point the pane at a new location.
    ///
    /// `DirectoryList` cancels any in-flight enumeration for the previous
    /// directory, so navigating away from a slow or unresponsive mount returns
    /// immediately rather than waiting for it to finish.
    pub fn set_location(&self, file: &gio::File) {
        self.directory_list.set_file(Some(file));
    }

    pub fn location(&self) -> Option<gio::File> {
        self.directory_list.file()
    }

    pub fn is_loading(&self) -> bool {
        self.directory_list.is_loading()
    }

    /// Switch between list and grid. Purely a stack page change: the models are
    /// untouched, so nothing re-enumerates and the selection is preserved.
    pub fn set_view_mode(&self, mode: ViewMode) {
        self.container.set_visible_child_name(mode.id());
    }

    pub fn set_show_hidden(&self, show_hidden: bool) {
        {
            let mut state = self.state.borrow_mut();
            if state.filter.show_hidden == show_hidden {
                return;
            }
            state.filter.show_hidden = show_hidden;
        }
        // Showing more or fewer entries is not a refinement in either
        // direction, so the filter has to be re-evaluated from scratch.
        self.custom_filter.changed(gtk::FilterChange::Different);
    }

    pub fn set_query(&self, query: &str) {
        let change = {
            let mut state = self.state.borrow_mut();
            if state.filter.query == query {
                return;
            }
            // Typing another character can only remove matches; telling GTK that
            // lets it skip re-testing entries already filtered out.
            let change = if query.starts_with(state.filter.query.as_str()) {
                gtk::FilterChange::MoreStrict
            } else if state.filter.query.starts_with(query) {
                gtk::FilterChange::LessStrict
            } else {
                gtk::FilterChange::Different
            };
            state.filter.query = query.to_owned();
            change
        };
        self.custom_filter.changed(change);
    }

    pub fn set_sort(&self, spec: SortSpec) {
        {
            let mut state = self.state.borrow_mut();
            if state.sort == spec {
                return;
            }
            state.sort = spec;
        }
        self.custom_sorter.changed(gtk::SorterChange::Different);
    }

    pub fn sort_spec(&self) -> SortSpec {
        self.state.borrow().sort
    }

    /// Paths of the selected entries, in view order.
    pub fn selected_paths(&self) -> Vec<PathBuf> {
        let selection = self.selection.selection();
        let mut paths = Vec::with_capacity(selection.size() as usize);
        for index in 0..self.selection.n_items() {
            if selection.contains(index)
                && let Some(file) = self.file_at(index)
                && let Some(path) = file.path()
            {
                paths.push(path);
            }
        }
        paths
    }

    /// The `gio::File` at a position in the *sorted, filtered* view.
    pub fn file_at(&self, position: u32) -> Option<gio::File> {
        let object = self.selection.item(position)?;
        let info = object.downcast_ref::<gio::FileInfo>()?;
        file_of(info)
    }

    pub fn info_at(&self, position: u32) -> Option<gio::FileInfo> {
        self.selection
            .item(position)
            .and_then(|object| object.downcast::<gio::FileInfo>().ok())
    }

    /// Run `handler` when a row is activated (double-click or Enter).
    pub fn connect_activate(&self, handler: impl Fn(gio::File, bool) + 'static) {
        let handler = Rc::new(handler);

        for (view, _) in [
            (self.column_view.clone().upcast::<gtk::Widget>(), ()),
            (self.grid_view.clone().upcast::<gtk::Widget>(), ()),
        ] {
            if let Some(column_view) = view.downcast_ref::<gtk::ColumnView>() {
                let handler = Rc::clone(&handler);
                let selection = self.selection.clone();
                column_view.connect_activate(move |_, position| {
                    dispatch_activate(&selection, position, &handler);
                });
            } else if let Some(grid_view) = view.downcast_ref::<gtk::GridView>() {
                let handler = Rc::clone(&handler);
                let selection = self.selection.clone();
                grid_view.connect_activate(move |_, position| {
                    dispatch_activate(&selection, position, &handler);
                });
            }
        }
    }

    /// Notify when the set of visible items changes, for the status line.
    pub fn connect_items_changed(&self, handler: impl Fn() + 'static) {
        let handler = Rc::new(handler);

        let h = Rc::clone(&handler);
        self.filter_model
            .connect_items_changed(move |_, _, _, _| h());

        let h = Rc::clone(&handler);
        self.sort_model.connect_items_changed(move |_, _, _, _| h());

        let h = Rc::clone(&handler);
        self.selection.connect_selection_changed(move |_, _, _| h());

        let h = Rc::clone(&handler);
        self.directory_list.connect_loading_notify(move |_| h());
    }

    /// Notify when enumeration reports an error — permission denied, a vanished
    /// directory, an unmounted device.
    pub fn connect_error(&self, handler: impl Fn(glib::Error) + 'static) {
        self.directory_list.connect_error_notify(move |list| {
            if let Some(error) = list.error() {
                handler(error);
            }
        });
    }
}

fn dispatch_activate(
    selection: &gtk::MultiSelection,
    position: u32,
    handler: &Rc<impl Fn(gio::File, bool) + 'static>,
) {
    let Some(object) = selection.item(position) else {
        return;
    };
    let Some(info) = object.downcast_ref::<gio::FileInfo>() else {
        return;
    };
    let Some(file) = file_of(info) else {
        return;
    };
    handler(file, is_directory(info));
}

fn scrolled(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .hexpand(true)
        .vexpand(true)
        .child(child)
        .build()
}

// ---- Column view --------------------------------------------------------

fn build_column_view(selection: &gtk::MultiSelection) -> gtk::ColumnView {
    let view = gtk::ColumnView::builder()
        .model(selection)
        .show_row_separators(false)
        .show_column_separators(false)
        .reorderable(false)
        .hexpand(true)
        .vexpand(true)
        .build();

    view.append_column(&name_column());
    view.append_column(&text_column("Size", |info| {
        if is_directory(info) {
            // A directory's own size is meaningless, and computing the real
            // answer is an unbounded tree walk we refuse to do implicitly.
            String::new()
        } else {
            crate::model::format::human_bytes(info.size().max(0) as u64)
        }
    }));
    view.append_column(&text_column("Modified", |info| {
        info.modification_date_time()
            .and_then(|dt| dt.format("%Y-%m-%d %H:%M").ok())
            .map(|s| s.to_string())
            .unwrap_or_default()
    }));
    view.append_column(&text_column("Type", |info| {
        if is_directory(info) {
            "Folder".to_owned()
        } else {
            let content = content_type(info);
            if content.is_empty() {
                String::new()
            } else {
                gio::functions::content_type_get_description(&content).to_string()
            }
        }
    }));

    view
}

fn name_column() -> gtk::ColumnViewColumn {
    let factory = gtk::SignalListItemFactory::new();

    factory.connect_setup(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let icon = gtk::Image::new();
        icon.set_pixel_size(16);
        icon.add_css_class("hive-item-icon");
        let label = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .build();
        label.add_css_class("hive-file-name");
        row.append(&icon);
        row.append(&label);
        item.set_child(Some(&row));
    });

    factory.connect_bind(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(info) = item.item().and_then(|o| o.downcast::<gio::FileInfo>().ok()) else {
            return;
        };
        let Some(row) = item.child().and_then(|c| c.downcast::<gtk::Box>().ok()) else {
            return;
        };

        let mut child = row.first_child();
        if let Some(icon) = child.clone().and_then(|c| c.downcast::<gtk::Image>().ok()) {
            match info.symbolic_icon() {
                Some(gicon) => icon.set_from_gicon(&gicon),
                None => icon.set_icon_name(Some(fallback_icon(&info))),
            }
            if is_directory(&info) {
                icon.add_css_class("hive-folder-icon");
            } else {
                icon.remove_css_class("hive-folder-icon");
            }
        }

        child = child.and_then(|c| c.next_sibling());
        if let Some(label) = child.and_then(|c| c.downcast::<gtk::Label>().ok()) {
            let name = display_name(&info);
            label.set_text(&name);
            // A newline in a filename would otherwise make the row grow; keep
            // it single-line and show the real name in the tooltip.
            label.set_single_line_mode(true);
            label.set_tooltip_text(Some(&name));
            if info.is_symlink() {
                label.add_css_class("hive-file-symlink");
            } else {
                label.remove_css_class("hive-file-symlink");
            }
        }
    });

    gtk::ColumnViewColumn::builder()
        .title("Name")
        .factory(&factory)
        .expand(true)
        .resizable(true)
        .build()
}

fn text_column(
    title: &str,
    extract: impl Fn(&gio::FileInfo) -> String + 'static,
) -> gtk::ColumnViewColumn {
    let factory = gtk::SignalListItemFactory::new();

    factory.connect_setup(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let label = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .single_line_mode(true)
            .build();
        label.add_css_class("hive-file-meta");
        item.set_child(Some(&label));
    });

    factory.connect_bind(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(info) = item.item().and_then(|o| o.downcast::<gio::FileInfo>().ok()) else {
            return;
        };
        if let Some(label) = item.child().and_then(|c| c.downcast::<gtk::Label>().ok()) {
            label.set_text(&extract(&info));
        }
    });

    gtk::ColumnViewColumn::builder()
        .title(title)
        .factory(&factory)
        .resizable(true)
        .build()
}

// ---- Grid view ----------------------------------------------------------

fn build_grid_view(selection: &gtk::MultiSelection) -> gtk::GridView {
    let factory = gtk::SignalListItemFactory::new();

    factory.connect_setup(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let cell = gtk::Box::new(gtk::Orientation::Vertical, 6);
        cell.set_halign(gtk::Align::Center);
        cell.set_width_request(96);

        let icon = gtk::Image::new();
        icon.set_pixel_size(48);
        icon.add_css_class("hive-item-icon");

        let label = gtk::Label::builder()
            .justify(gtk::Justification::Center)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .lines(2)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .max_width_chars(12)
            .build();
        label.add_css_class("hive-file-name");

        cell.append(&icon);
        cell.append(&label);
        item.set_child(Some(&cell));
    });

    factory.connect_bind(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(info) = item.item().and_then(|o| o.downcast::<gio::FileInfo>().ok()) else {
            return;
        };
        let Some(cell) = item.child().and_then(|c| c.downcast::<gtk::Box>().ok()) else {
            return;
        };

        let mut child = cell.first_child();
        if let Some(icon) = child.clone().and_then(|c| c.downcast::<gtk::Image>().ok()) {
            match info.symbolic_icon() {
                Some(gicon) => icon.set_from_gicon(&gicon),
                None => icon.set_icon_name(Some(fallback_icon(&info))),
            }
            if is_directory(&info) {
                icon.add_css_class("hive-folder-icon");
            } else {
                icon.remove_css_class("hive-folder-icon");
            }
        }

        child = child.and_then(|c| c.next_sibling());
        if let Some(label) = child.and_then(|c| c.downcast::<gtk::Label>().ok()) {
            let name = display_name(&info);
            label.set_text(&name);
            label.set_tooltip_text(Some(&name));
        }
    });

    gtk::GridView::builder()
        .model(selection)
        .factory(&factory)
        .max_columns(24)
        .min_columns(1)
        .hexpand(true)
        .vexpand(true)
        .build()
}

// ---- FileInfo accessors -------------------------------------------------

/// The name to show, falling back through display-name, name, then the URI.
///
/// Filenames on Linux are bytes and need not be valid UTF-8. gio's
/// `display-name` is already lossily converted, which is what we want: the file
/// stays visible and selectable rather than disappearing from the listing.
fn display_name(info: &gio::FileInfo) -> String {
    let display = info.display_name();
    if !display.is_empty() {
        return display.to_string();
    }
    if let Some(name) = info.name().to_str() {
        return name.to_owned();
    }
    info.name().to_string_lossy().into_owned()
}

fn content_type(info: &gio::FileInfo) -> String {
    info.content_type()
        .map(|c| c.to_string())
        .unwrap_or_default()
}

fn is_directory(info: &gio::FileInfo) -> bool {
    info.file_type() == gio::FileType::Directory
}

fn modified_seconds(info: &gio::FileInfo) -> i64 {
    info.modification_date_time()
        .map(|dt| dt.to_unix())
        .unwrap_or(0)
}

/// The `gio::File` `DirectoryList` attaches to each entry.
fn file_of(info: &gio::FileInfo) -> Option<gio::File> {
    info.attribute_object("standard::file")
        .and_then(|object| object.downcast::<gio::File>().ok())
}

/// Used only when gio has no symbolic icon at all — a broken symlink, or an
/// entry whose content type could not be determined.
fn fallback_icon(info: &gio::FileInfo) -> &'static str {
    if is_directory(info) {
        "folder-symbolic"
    } else {
        "text-x-generic-symbolic"
    }
}
