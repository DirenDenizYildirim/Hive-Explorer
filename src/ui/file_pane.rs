//! The file pane: one model stack, two views.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib::clone;

use crate::config::ViewMode;
use crate::model::filter::{self, FilterInput, FilterSpec};
use crate::model::sort::{self, SortKeyData, SortSpec};

/// Attributes requested from every entry.
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
    /// A plain box wrapping the view stack.
    ///
    /// The context-menu popover parents itself here rather than to the stack,
    /// which manages its children as pages and is a poor host for one it does
    /// not know about.
    container: gtk::Box,
    stack: gtk::Stack,
    directory_list: gtk::DirectoryList,
    filter_model: gtk::FilterListModel,
    sort_model: gtk::SortListModel,
    selection: gtk::MultiSelection,
    custom_filter: gtk::CustomFilter,
    custom_sorter: gtk::CustomSorter,
    state: Rc<RefCell<ViewState>>,
    column_view: gtk::ColumnView,
    grid_view: gtk::GridView,
    /// A path to select once it appears.
    ///
    /// `--select` and Alt+Up both name a file that may not have been enumerated
    /// yet — `DirectoryList` fills in progressively, so selecting eagerly would
    /// usually miss. The request is retried as entries arrive and dropped when
    /// loading finishes without it showing up.
    pending_selection: RefCell<Option<PathBuf>>,
    mode: Cell<ViewMode>,
    /// Set by a row's own right-click handler, read by the view's.
    ///
    /// Neither `GtkColumnView` nor `GtkGridView` can name the row under a
    /// point, so the rows report themselves: the row gesture runs first in the
    /// bubble phase and leaves its position here for the view gesture to take.
    right_clicked: Rc<Cell<Option<u32>>>,
}

impl FilePane {
    pub fn new(filter_spec: FilterSpec, sort_spec: SortSpec) -> Rc<Self> {
        let state = Rc::new(RefCell::new(ViewState {
            filter: filter_spec,
            sort: sort_spec,
        }));

        let directory_list = gtk::DirectoryList::new(Some(ATTRIBUTES), gio::File::NONE);
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

        let right_clicked = Rc::new(Cell::new(None));
        let column_view = build_column_view(&selection, &right_clicked);
        let grid_view = build_grid_view(&selection, &right_clicked);

        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::None);
        stack.add_named(&scrolled(&column_view), Some(ViewMode::List.id()));
        stack.add_named(&scrolled(&grid_view), Some(ViewMode::Grid.id()));

        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        container.append(&stack);
        container.add_css_class("hive-file-pane");

        Rc::new(Self {
            container,
            stack,
            directory_list,
            filter_model,
            sort_model,
            selection,
            custom_filter,
            custom_sorter,
            state,
            column_view,
            grid_view,
            pending_selection: RefCell::new(None),
            mode: Cell::new(ViewMode::List),
            right_clicked,
        })
    }

    pub fn widget(&self) -> &gtk::Box {
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
    pub fn set_location(&self, file: &gio::File) {
        self.directory_list.set_file(Some(file));
    }

    pub fn location(&self) -> Option<gio::File> {
        self.directory_list.file()
    }

    pub fn is_loading(&self) -> bool {
        self.directory_list.is_loading()
    }

    /// Switch between list and grid. A stack page change only; nothing re-enumerates.
    pub fn set_view_mode(&self, mode: ViewMode) {
        self.mode.set(mode);
        self.stack.set_visible_child_name(mode.id());
    }

    pub fn view_mode(&self) -> ViewMode {
        self.mode.get()
    }

    /// Move keyboard focus into whichever view is showing.
    pub fn focus_view(&self) {
        let focused = match self.mode.get() {
            ViewMode::List => self.column_view.grab_focus(),
            ViewMode::Grid => self.grid_view.grab_focus(),
        };
        if !focused {
            tracing::debug!("file pane refused focus");
        }
    }

    pub fn select_all(&self) {
        self.selection.select_all();
    }

    pub fn unselect_all(&self) {
        self.selection.unselect_all();
    }

    /// The first selected position, for type-ahead and keyboard movement.
    pub fn selected_position(&self) -> Option<u32> {
        let selection = self.selection.selection();
        (0..self.selection.n_items()).find(|index| selection.contains(*index))
    }

    /// Select exactly one row and bring it into view.
    pub fn select_only(&self, position: u32) {
        if position >= self.selection.n_items() {
            return;
        }
        self.selection.select_item(position, true);
        self.scroll_to(position);
    }

    pub fn scroll_to(&self, position: u32) {
        if position >= self.selection.n_items() {
            return;
        }
        match self.mode.get() {
            ViewMode::List => {
                self.column_view
                    .scroll_to(position, None, gtk::ListScrollFlags::FOCUS, None)
            }
            ViewMode::Grid => self
                .grid_view
                .scroll_to(position, gtk::ListScrollFlags::FOCUS, None),
        }
    }

    /// Position of `path` in the current sorted, filtered view.
    pub fn position_of_path(&self, path: &Path) -> Option<u32> {
        (0..self.selection.n_items()).find(|index| {
            self.file_at(*index)
                .and_then(|file| file.path())
                .is_some_and(|candidate| candidate == path)
        })
    }

    /// Ask for `path` to be selected once it has been enumerated.
    pub fn request_selection(&self, path: PathBuf) {
        *self.pending_selection.borrow_mut() = Some(path);
        self.apply_pending_selection();
    }

    /// Try to satisfy an outstanding selection request.
    pub fn apply_pending_selection(&self) {
        let Some(path) = self.pending_selection.borrow().clone() else {
            return;
        };

        if let Some(position) = self.position_of_path(&path) {
            self.pending_selection.borrow_mut().take();
            self.select_only(position);
            return;
        }

        // Enumeration finished and it never appeared: the file was deleted, or
        // is filtered out. Drop the request rather than leaving it to fire on
        // some later directory.
        if !self.is_loading() {
            tracing::debug!(path = %path.display(), "requested selection never appeared");
            self.pending_selection.borrow_mut().take();
        }
    }

    /// Clear any outstanding request, on navigating elsewhere.
    pub fn clear_pending_selection(&self) {
        self.pending_selection.borrow_mut().take();
    }

    /// Display names in view order, for type-ahead.
    pub fn visible_names(&self) -> Vec<String> {
        (0..self.selection.n_items())
            .map(|index| {
                self.info_at(index)
                    .map(|info| display_name(&info))
                    .unwrap_or_default()
            })
            .collect()
    }

    pub fn set_show_hidden(&self, show_hidden: bool) {
        {
            let mut state = self.state.borrow_mut();
            if state.filter.show_hidden == show_hidden {
                return;
            }
            state.filter.show_hidden = show_hidden;
        }
        self.custom_filter.changed(gtk::FilterChange::Different);
    }

    pub fn set_query(&self, query: &str) {
        let change = {
            let mut state = self.state.borrow_mut();
            if state.filter.query == query {
                return;
            }
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

    /// Run `handler` on a right-click, with the row's position when there is one.
    ///
    /// Coordinates are relative to the pane's own container, so the caller can
    /// point a popover straight at them.
    pub fn connect_context_menu(&self, handler: impl Fn(Option<u32>, f64, f64) + 'static) {
        let handler = Rc::new(handler);

        for view in [
            self.column_view.clone().upcast::<gtk::Widget>(),
            self.grid_view.clone().upcast::<gtk::Widget>(),
        ] {
            let gesture = gtk::GestureClick::builder().button(3).build();
            let handler = Rc::clone(&handler);
            let right_clicked = Rc::clone(&self.right_clicked);
            let container = self.container.clone();
            let source = view.clone();

            gesture.connect_pressed(move |gesture, _, x, y| {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                let position = right_clicked.take();
                let point = source
                    .compute_point(&container, &gtk::graphene::Point::new(x as f32, y as f32))
                    .unwrap_or_else(|| gtk::graphene::Point::new(x as f32, y as f32));
                handler(position, f64::from(point.x()), f64::from(point.y()));
            });

            view.add_controller(gesture);
        }
    }

    /// Notify when enumeration fails: permission denied, vanished, unmounted.
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

/// Let a row announce itself when right-clicked, and select it if it was not.
fn watch_right_click(
    item: &gtk::ListItem,
    child: &impl IsA<gtk::Widget>,
    right_clicked: &Rc<Cell<Option<u32>>>,
    selection: &gtk::MultiSelection,
) {
    let gesture = gtk::GestureClick::builder().button(3).build();
    let right_clicked = Rc::clone(right_clicked);
    let selection = selection.clone();
    let item = item.clone();

    gesture.connect_pressed(move |_, _, _, _| {
        let position = item.position();
        right_clicked.set(Some(position));

        // Acting on a row the user cannot see highlighted is a good way to
        // delete the wrong thing.
        if !selection.selection().contains(position) {
            selection.select_item(position, true);
        }
    });

    child.as_ref().add_controller(gesture);
}

fn build_column_view(
    selection: &gtk::MultiSelection,
    right_clicked: &Rc<Cell<Option<u32>>>,
) -> gtk::ColumnView {
    let view = gtk::ColumnView::builder()
        .model(selection)
        .show_row_separators(false)
        .show_column_separators(false)
        .reorderable(false)
        .hexpand(true)
        .vexpand(true)
        .build();

    view.set_enable_rubberband(true);
    view.append_column(&name_column(selection, right_clicked));

    let size_column = text_column("Size", |info| {
        if is_directory(info) {
            String::new()
        } else {
            crate::model::format::human_bytes(info.size().max(0) as u64)
        }
    });
    view.append_column(&size_column);

    // gio hands back a UTC timestamp; formatting it directly would show every
    // file three hours off here, and disagree with the conflict dialog.
    let modified_column = text_column("Modified", |info| {
        info.modification_date_time()
            .and_then(|time| time.to_local().ok())
            .and_then(|local| local.format("%Y-%m-%d %H:%M").ok())
            .map(|text| text.to_string())
            .unwrap_or_default()
    });
    view.append_column(&modified_column);

    let type_column = text_column("Type", |info| {
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
    });
    view.append_column(&type_column);

    view
}

fn name_column(
    selection: &gtk::MultiSelection,
    right_clicked: &Rc<Cell<Option<u32>>>,
) -> gtk::ColumnViewColumn {
    let factory = gtk::SignalListItemFactory::new();

    let selection = selection.clone();
    let right_clicked = Rc::clone(right_clicked);
    factory.connect_setup(move |_, item| {
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
        watch_right_click(item, &row, &right_clicked, &selection);
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

fn build_grid_view(
    selection: &gtk::MultiSelection,
    right_clicked: &Rc<Cell<Option<u32>>>,
) -> gtk::GridView {
    let factory = gtk::SignalListItemFactory::new();

    let for_setup = selection.clone();
    let right_clicked = Rc::clone(right_clicked);
    factory.connect_setup(move |_, item| {
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
        watch_right_click(item, &cell, &right_clicked, &for_setup);
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
        .enable_rubberband(true)
        .hexpand(true)
        .vexpand(true)
        .build()
}

/// The name to show, falling back through display-name, name, then the URI.
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

/// Used only when gio reports no symbolic icon at all.
fn fallback_icon(info: &gio::FileInfo) -> &'static str {
    if is_directory(info) {
        "folder-symbolic"
    } else {
        "text-x-generic-symbolic"
    }
}
