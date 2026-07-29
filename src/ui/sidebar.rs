//! The sidebar: Pinned, Places, Recents, Devices.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;

use crate::colors;
use crate::config::Config;
use crate::fs::places::{self, Place, PlaceKind};
use crate::fs::volumes::{self, MountCandidate};
use crate::model::pins;
use crate::ui::dnd::{self, FolderDrag};

/// How many recent files to list.
const RECENT_LIMIT: usize = 8;

/// Marks the row for the location currently being viewed.
const CURRENT_ROW_CLASS: &str = "hive-current";

/// Marks a pinned folder that is no longer on disk.
const MISSING_ROW_CLASS: &str = "hive-missing";

/// Marks the sidebar while a folder is being dragged over it.
const DROP_CLASS: &str = dnd::DROP_CLASS;

type NavigateHandler = Rc<dyn Fn(gio::File)>;
type PinsHandler = Rc<dyn Fn(PinEvent)>;

/// What just happened to the pinned list, for the window to report and save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinEvent {
    /// Folders pinned. Zero means everything dropped was already pinned, which
    /// a drag onto the sidebar can legitimately produce.
    Added(usize),
    /// An existing pin was dragged to a new position.
    Reordered,
    Removed(String),
}

pub struct Sidebar {
    scroller: gtk::ScrolledWindow,
    container: gtk::Box,
    pinned: Section,
    places: Section,
    recents: Section,
    devices: Section,
    config: Rc<RefCell<Config>>,
    colors: Rc<RefCell<colors::Store>>,
    on_navigate: RefCell<Option<NavigateHandler>>,
    on_pins_changed: RefCell<Option<PinsHandler>>,
    /// One menu for the whole pinned list, aimed at whichever row was clicked.
    pinned_menu: gtk::Popover,
    pinned_menu_target: RefCell<Option<PathBuf>>,
    /// Bumped on every pinned rebuild, so a late existence check knows whether
    /// the rows it was told about are still the rows on screen.
    pinned_generation: Cell<u64>,
}

/// A labelled group of rows that hides itself when empty.
struct Section {
    container: gtk::Box,
    list: gtk::ListBox,
}

impl Section {
    fn new(title: &str) -> Self {
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);

        let label = gtk::Label::builder().label(title).xalign(0.0).build();
        label.add_css_class("hive-section-label");

        let list = gtk::ListBox::new();
        // The highlight tracks the current location, which is not the same
        // thing as a GTK selection: selection also moves on focus and on click,
        // so a row could stay lit while the view was somewhere else entirely.
        // Hive marks the current row with a class it controls instead.
        list.set_selection_mode(gtk::SelectionMode::None);
        list.add_css_class("navigation-sidebar");

        container.append(&label);
        container.append(&list);
        Self { container, list }
    }

    /// Remove the rows, and only the rows.
    ///
    /// Walking `first_child` would also pick up anything else parented to the
    /// list — a popover, say — which `GtkListBox` refuses to remove, and the
    /// loop would then never terminate.
    fn clear(&self) {
        while let Some(row) = self.list.row_at_index(0) {
            self.list.remove(&row);
        }
    }

    /// Drop the current-location marker from every row in this group.
    fn clear_current(&self) {
        let mut child = self.list.first_child();
        while let Some(widget) = child {
            widget.remove_css_class(CURRENT_ROW_CLASS);
            child = widget.next_sibling();
        }
    }

    /// Hide the whole group — label included — when it has no rows.
    fn sync_visibility(&self) {
        self.container
            .set_visible(self.list.first_child().is_some());
    }
}

impl Sidebar {
    pub fn new(config: Rc<RefCell<Config>>, colors: Rc<RefCell<colors::Store>>) -> Rc<Self> {
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        container.add_css_class("hive-sidebar");

        let pinned = Section::new("Pinned");
        let places = Section::new("Places");
        let recents = Section::new("Recents");
        let devices = Section::new("Devices");

        container.append(&pinned.container);
        container.append(&places.container);
        container.append(&recents.container);
        container.append(&devices.container);

        // No width_request: AdwOverlaySplitView governs sidebar width, and a
        // hard request on top becomes an unshrinkable floor added to the
        // content's minimum, pushing the window's minimum past what a tiled
        // layout allows.
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .child(&container)
            .build();

        let sidebar = Rc::new(Self {
            scroller,
            container,
            pinned,
            places,
            recents,
            devices,
            config,
            colors,
            on_navigate: RefCell::new(None),
            on_pins_changed: RefCell::new(None),
            pinned_menu: gtk::Popover::new(),
            pinned_menu_target: RefCell::new(None),
            pinned_generation: Cell::new(0),
        });

        sidebar.build_pinned_menu();
        sidebar.rebuild_pinned();
        sidebar.rebuild_places();
        sidebar.rebuild_recents();
        sidebar.rebuild_devices();
        sidebar.watch_volumes();
        sidebar.watch_recents();
        sidebar.accept_drops();

        sidebar
    }

    pub fn widget(&self) -> &gtk::ScrolledWindow {
        &self.scroller
    }

    pub fn connect_navigate(self: &Rc<Self>, handler: impl Fn(gio::File) + 'static) {
        *self.on_navigate.borrow_mut() = Some(Rc::new(handler));
    }

    /// Called after the pinned list changes, for saving and reporting.
    pub fn connect_pins_changed(self: &Rc<Self>, handler: impl Fn(PinEvent) + 'static) {
        *self.on_pins_changed.borrow_mut() = Some(Rc::new(handler));
    }

    fn navigate(&self, file: gio::File) {
        let handler = self.on_navigate.borrow().clone();
        if let Some(handler) = handler {
            handler(file);
        }
    }

    fn announce(&self, event: PinEvent) {
        let handler = self.on_pins_changed.borrow().clone();
        if let Some(handler) = handler {
            handler(event);
        }
    }

    // ---- pinned ----------------------------------------------------------

    pub fn is_pinned(&self, path: &Path) -> bool {
        pins::contains(&self.config.borrow().sidebar.pinned, path)
    }

    /// Pin `paths` at the end of the list. Returns how many were added.
    pub fn pin(self: &Rc<Self>, paths: &[PathBuf]) -> usize {
        let added = {
            let mut config = self.config.borrow_mut();
            pins::add(&mut config.sidebar.pinned, paths)
        };
        self.rebuild_pinned();
        self.announce(PinEvent::Added(added));
        added
    }

    /// Pin or move `paths` to `index`, which is how a drop lands.
    fn pin_at(self: &Rc<Self>, paths: &[PathBuf], index: usize) {
        let added = {
            let mut config = self.config.borrow_mut();
            pins::insert(&mut config.sidebar.pinned, paths, index)
        };
        self.rebuild_pinned();
        self.announce(if added > 0 {
            PinEvent::Added(added)
        } else {
            PinEvent::Reordered
        });
    }

    pub fn unpin(self: &Rc<Self>, path: &Path) -> bool {
        let removed = {
            let mut config = self.config.borrow_mut();
            pins::remove(&mut config.sidebar.pinned, path)
        };
        if removed {
            self.rebuild_pinned();
            self.announce(PinEvent::Removed(crate::model::path::display_name(path)));
        }
        removed
    }

    /// Rebuild the pinned rows from the config. Cheap: there are a handful.
    pub fn rebuild_pinned(self: &Rc<Self>) {
        // A hand-edited config can hold duplicates or relative paths; neither
        // should ever become a row.
        let pinned = {
            let mut config = self.config.borrow_mut();
            config.sidebar.pinned = pins::normalize(&config.sidebar.pinned);
            config.sidebar.pinned.clone()
        };

        self.pinned.clear();
        self.pinned_generation
            .set(self.pinned_generation.get().wrapping_add(1));

        for (index, path) in pinned.iter().enumerate() {
            let row = self.pin_row(index, path);
            self.pinned.list.append(&row);
        }

        self.pinned.sync_visibility();
        self.check_pinned_exist(pinned);
    }

    fn pin_row(self: &Rc<Self>, index: usize, path: &Path) -> gtk::ListBoxRow {
        let row = row_widget("folder-symbolic", &crate::model::path::display_name(path));
        row.set_tooltip_text(Some(&path.to_string_lossy()));

        // A pinned folder wears its colour too — it is the same folder icon,
        // and it is what makes a long pinned list scannable.
        if let Some(icon) = icon_of(&row) {
            icon.add_css_class("hive-folder-icon");
            if let Ok(store) = self.colors.try_borrow()
                && let Some(accent) = store.get(path)
            {
                icon.add_css_class(accent.css_class());
            }
        }

        let target = gio::File::for_path(path);
        let sidebar = Rc::clone(self);
        let gesture = gtk::GestureClick::new();
        gesture.connect_released(move |gesture, _, _, _| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            sidebar.navigate(target.clone());
        });
        row.add_controller(gesture);

        // Right-click to unpin.
        let sidebar = Rc::clone(self);
        let owner = path.to_path_buf();
        let menu_row = row.clone();
        let menu = gtk::GestureClick::builder().button(3).build();
        menu.connect_pressed(move |gesture, _, x, y| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            sidebar.open_pinned_menu(&menu_row, &owner, x, y);
        });
        row.add_controller(menu);

        // Drag this row to reorder it.
        let dragged = path.to_path_buf();
        let source = gtk::DragSource::new();
        source.set_actions(gtk::gdk::DragAction::COPY);
        let paintable_row = row.clone();
        source.connect_prepare(move |source, _, _| {
            let paintable = gtk::WidgetPaintable::new(Some(&paintable_row));
            source.set_icon(Some(&paintable), 0, 0);
            Some(FolderDrag::new(vec![dragged.clone()]).content())
        });
        row.add_controller(source);

        // Drop onto this row to land here. Past the halfway line means after
        // it, which is the only way to reach the position below the last row
        // without aiming at the empty space under the list.
        //
        // Only `motion` reports where in the row the pointer is, so the answer
        // is kept here for `drop`, which does not.
        let below = Rc::new(Cell::new(false));

        let sidebar = Rc::clone(self);
        let drop_below = Rc::clone(&below);
        let target = dnd::drop_target(move |paths| {
            let at = if drop_below.get() { index + 1 } else { index };
            sidebar.pin_at(&paths, at);
            true
        });

        let motion_row = row.clone();
        target.connect_motion(move |_, _, y| {
            let height = motion_row.height();
            below.set(height > 0 && y > f64::from(height) / 2.0);
            gtk::gdk::DragAction::COPY
        });

        // GTK hands the drag to one target at a time, so moving onto a pinned
        // row takes it away from the sidebar's own target. Without this the
        // whole sidebar's highlight blinks off exactly when the pointer is
        // over the rows it is aiming at.
        let container = self.container.clone();
        target.connect_enter(move |_, _, _| {
            container.add_css_class(DROP_CLASS);
            gtk::gdk::DragAction::COPY
        });

        let container = self.container.clone();
        target.connect_leave(move |_| container.remove_css_class(DROP_CLASS));

        row.add_controller(target);

        row
    }

    fn build_pinned_menu(self: &Rc<Self>) {
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("hive-row-menu");

        let unpin = gtk::Button::with_label("Unpin");
        unpin.add_css_class("flat");
        unpin.set_halign(gtk::Align::Fill);

        let sidebar = Rc::clone(self);
        unpin.connect_clicked(move |_| {
            sidebar.pinned_menu.popdown();
            let target = sidebar.pinned_menu_target.borrow_mut().take();
            if let Some(path) = target {
                sidebar.unpin(&path);
            }
        });

        content.append(&unpin);
        self.pinned_menu.set_child(Some(&content));
        self.pinned_menu.set_has_arrow(false);
        self.pinned_menu.set_position(gtk::PositionType::Bottom);
        // Parented to the section rather than the list: the list is emptied and
        // refilled on every change, and a popover is not one of its rows.
        self.pinned_menu.set_parent(&self.pinned.container);

        // A popover parented to a widget has to go before that widget does, or
        // GTK complains on teardown.
        let popover = self.pinned_menu.clone();
        self.pinned
            .container
            .connect_destroy(move |_| popover.unparent());
    }

    fn open_pinned_menu(&self, row: &gtk::ListBoxRow, path: &Path, x: f64, y: f64) {
        *self.pinned_menu_target.borrow_mut() = Some(path.to_path_buf());

        let point = row
            .compute_point(
                &self.pinned.container,
                &gtk::graphene::Point::new(x as f32, y as f32),
            )
            .unwrap_or_else(|| gtk::graphene::Point::new(x as f32, y as f32));
        self.pinned_menu
            .set_pointing_to(Some(&gtk::gdk::Rectangle::new(
                point.x() as i32,
                point.y() as i32,
                1,
                1,
            )));
        self.pinned_menu.popup();
    }

    /// Mark pinned folders that are no longer there, off the main thread.
    ///
    /// A pin is never removed on the user's behalf — it is their list — but a
    /// row that leads nowhere should say so rather than looking live.
    fn check_pinned_exist(self: &Rc<Self>, pinned: Vec<PathBuf>) {
        if pinned.is_empty() {
            return;
        }

        let generation = self.pinned_generation.get();
        let sidebar = Rc::downgrade(self);

        glib::spawn_future_local(async move {
            let Ok(missing) = gio::spawn_blocking(move || {
                pinned
                    .into_iter()
                    .enumerate()
                    .filter(|(_, path)| !path.is_dir())
                    .map(|(index, _)| index)
                    .collect::<Vec<usize>>()
            })
            .await
            else {
                return;
            };

            let Some(sidebar) = sidebar.upgrade() else {
                return;
            };
            // The list was rebuilt while we were off checking; those indices
            // no longer mean anything, and the rebuild started its own check.
            if sidebar.pinned_generation.get() != generation {
                return;
            }

            for index in missing {
                if let Some(row) = sidebar.pinned.list.row_at_index(index as i32) {
                    row.add_css_class(MISSING_ROW_CLASS);
                    let existing = row.tooltip_text().unwrap_or_default();
                    row.set_tooltip_text(Some(&format!("{existing} — not found")));
                }
            }
        });
    }

    /// Accept folders dropped anywhere on the sidebar, which appends them.
    ///
    /// This is what makes pinning work when nothing is pinned yet: with an
    /// empty list the Pinned section is hidden, so there is no row to aim at.
    fn accept_drops(self: &Rc<Self>) {
        let sidebar = Rc::clone(self);
        let target = dnd::drop_target(move |paths| {
            let at = sidebar.config.borrow().sidebar.pinned.len();
            sidebar.pin_at(&paths, at);
            true
        });

        let container = self.container.clone();
        target.connect_enter(move |_, _, _| {
            container.add_css_class(DROP_CLASS);
            gtk::gdk::DragAction::COPY
        });

        let container = self.container.clone();
        target.connect_leave(move |_| container.remove_css_class(DROP_CLASS));

        self.scroller.add_controller(target);
    }

    /// Repaint pinned folder icons after a colour change.
    pub fn refresh_folder_colors(self: &Rc<Self>) {
        self.rebuild_pinned();
    }

    // ---- places, recents, devices ----------------------------------------

    fn rebuild_places(self: &Rc<Self>) {
        self.places.clear();

        let home = crate::paths::home_dir();
        let resolved = places::resolve(
            &home,
            glib_special_dir,
            |path| path.is_dir(),
            &trash_access(),
        );

        for place in resolved {
            let row = self.place_row(&place);
            self.places.list.append(&row);
        }
        self.places.sync_visibility();
    }

    fn place_row(self: &Rc<Self>, place: &Place) -> gtk::ListBoxRow {
        let row = row_widget(place.icon_name(), place.label());

        let file = match &place.path {
            Some(path) => gio::File::for_path(path),
            None => gio::File::for_uri(&place.uri()),
        };

        let sidebar = Rc::clone(self);
        let gesture = gtk::GestureClick::new();
        gesture.connect_released(move |gesture, _, _, _| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            sidebar.navigate(file.clone());
        });
        row.add_controller(gesture);

        row
    }

    fn rebuild_recents(self: &Rc<Self>) {
        self.recents.clear();

        let manager = gtk::RecentManager::default();
        let mut shown = 0usize;

        for item in manager.items() {
            if shown >= RECENT_LIMIT {
                break;
            }
            let Some(uri) = Some(item.uri()) else {
                continue;
            };
            if !uri.starts_with("file://") {
                continue;
            }
            let file = gio::File::for_uri(&uri);
            let Some(path) = file.path() else {
                continue;
            };
            if !path.exists() {
                continue;
            }

            let display = item.display_name();
            let name = if display.is_empty() {
                crate::model::path::display_name(&path)
            } else {
                display.to_string()
            };

            let row = row_widget("document-open-recent-symbolic", &name);
            row.set_tooltip_text(Some(&path.to_string_lossy()));

            let parent = file.parent().unwrap_or_else(|| file.clone());
            let sidebar = Rc::clone(self);
            let gesture = gtk::GestureClick::new();
            gesture.connect_released(move |gesture, _, _, _| {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                sidebar.navigate(parent.clone());
            });
            row.add_controller(gesture);

            self.recents.list.append(&row);
            shown += 1;
        }

        self.recents.sync_visibility();
    }

    fn watch_recents(self: &Rc<Self>) {
        let sidebar = Rc::clone(self);
        gtk::RecentManager::default().connect_changed(move |_| {
            sidebar.rebuild_recents();
        });
    }

    fn rebuild_devices(self: &Rc<Self>) {
        self.devices.clear();

        let monitor = gio::VolumeMonitor::get();
        let mounts = monitor.mounts();

        let mut rows: Vec<(gio::Mount, String)> = Vec::new();
        for mount in mounts {
            let root = mount.root();
            let path = root.path();

            let candidate = MountCandidate {
                mount_point: path.as_deref(),
                filesystem: None,
                is_removable: mount
                    .volume()
                    .and_then(|volume| volume.drive())
                    .is_some_and(|drive| drive.is_removable()),
                can_eject: mount.can_eject(),
                can_unmount: mount.can_unmount(),
                is_shadowed: mount.is_shadowed(),
            };

            if volumes::is_user_relevant(&candidate) {
                rows.push((mount.clone(), mount.name().to_string()));
            }
        }

        for (mount, name) in rows {
            let row = self.device_row(&mount, &name);
            self.devices.list.append(&row);
        }

        self.devices.sync_visibility();
    }

    fn device_row(self: &Rc<Self>, mount: &gio::Mount, name: &str) -> gtk::ListBoxRow {
        let row = gtk::ListBoxRow::new();
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        content.set_margin_start(12);
        content.set_margin_end(6);

        let icon = gtk::Image::new();
        icon.set_pixel_size(16);
        icon.set_from_gicon(&mount.symbolic_icon());

        let label = gtk::Label::builder()
            .label(name)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .single_line_mode(true)
            .build();

        content.append(&icon);
        content.append(&label);

        if mount.can_eject() || mount.can_unmount() {
            let eject = gtk::Button::from_icon_name("media-eject-symbolic");
            eject.add_css_class("flat");
            eject.set_tooltip_text(Some("Eject"));
            eject.set_valign(gtk::Align::Center);

            let mount_for_eject = mount.clone();
            eject.connect_clicked(move |_| {
                let operation = gtk::MountOperation::new(gtk::Window::NONE);
                if mount_for_eject.can_eject() {
                    mount_for_eject.eject_with_operation(
                        gio::MountUnmountFlags::NONE,
                        Some(&operation),
                        gio::Cancellable::NONE,
                        |result| {
                            if let Err(error) = result {
                                tracing::warn!(%error, "eject failed");
                            }
                        },
                    );
                } else {
                    mount_for_eject.unmount_with_operation(
                        gio::MountUnmountFlags::NONE,
                        Some(&operation),
                        gio::Cancellable::NONE,
                        |result| {
                            if let Err(error) = result {
                                tracing::warn!(%error, "unmount failed");
                            }
                        },
                    );
                }
            });
            content.append(&eject);
        }

        row.set_child(Some(&content));

        let target = mount.root();
        let sidebar = Rc::clone(self);
        let gesture = gtk::GestureClick::new();
        gesture.connect_released(move |gesture, _, _, _| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            sidebar.navigate(target.clone());
        });
        row.add_controller(gesture);

        row
    }

    fn watch_volumes(self: &Rc<Self>) {
        let monitor = gio::VolumeMonitor::get();

        let sidebar = Rc::clone(self);
        monitor.connect_mount_added(move |_, _| sidebar.rebuild_devices());

        let sidebar = Rc::clone(self);
        monitor.connect_mount_removed(move |_, _| sidebar.rebuild_devices());

        let sidebar = Rc::clone(self);
        monitor.connect_mount_changed(move |_, _| sidebar.rebuild_devices());

        let sidebar = Rc::clone(self);
        monitor.connect_volume_added(move |_, _| sidebar.rebuild_devices());

        let sidebar = Rc::clone(self);
        monitor.connect_volume_removed(move |_, _| sidebar.rebuild_devices());
    }

    /// Highlight the row matching `location`, clearing every other section.
    pub fn sync_selection(&self, location: Option<&gio::File>) {
        for section in [&self.pinned, &self.places, &self.recents, &self.devices] {
            section.clear_current();
        }

        let Some(location) = location else {
            return;
        };
        let Some(path) = location.path() else {
            return;
        };

        let pinned = self.config.borrow().sidebar.pinned.clone();
        for (index, pin) in pinned.iter().enumerate() {
            if pin == &path
                && let Some(row) = self.pinned.list.row_at_index(index as i32)
            {
                row.add_css_class(CURRENT_ROW_CLASS);
            }
        }

        let home = crate::paths::home_dir();
        let resolved = places::resolve(&home, glib_special_dir, |p| p.is_dir(), &trash_access());
        for (index, place) in resolved.iter().enumerate() {
            if place.path.as_deref() == Some(path.as_path())
                && let Some(row) = self.places.list.row_at_index(index as i32)
            {
                row.add_css_class(CURRENT_ROW_CLASS);
                return;
            }
        }
    }
}

/// Ask glib for an XDG user directory.
fn trash_access() -> places::TrashAccess {
    let supported = gio::Vfs::default()
        .supported_uri_schemes()
        .iter()
        .any(|scheme| scheme == "trash");
    places::detect_trash_access(&glib::user_data_dir(), supported, |path| path.is_dir())
}

fn glib_special_dir(kind: PlaceKind) -> Option<std::path::PathBuf> {
    let directory = match kind {
        PlaceKind::Documents => glib::UserDirectory::Documents,
        PlaceKind::Downloads => glib::UserDirectory::Downloads,
        PlaceKind::Pictures => glib::UserDirectory::Pictures,
        PlaceKind::Videos => glib::UserDirectory::Videos,
        PlaceKind::Music => glib::UserDirectory::Music,
        PlaceKind::Home | PlaceKind::Trash => return None,
    };
    glib::user_special_dir(directory)
}

fn row_widget(icon_name: &str, label: &str) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    content.set_margin_start(12);
    content.set_margin_end(12);

    let icon = gtk::Image::from_icon_name(icon_name);
    icon.set_pixel_size(16);

    let text = gtk::Label::builder()
        .label(label)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .single_line_mode(true)
        .build();

    content.append(&icon);
    content.append(&text);
    row.set_child(Some(&content));
    row
}

/// The icon of a row built by [`row_widget`].
fn icon_of(row: &gtk::ListBoxRow) -> Option<gtk::Image> {
    row.child()
        .and_then(|child| child.downcast::<gtk::Box>().ok())
        .and_then(|content| content.first_child())
        .and_then(|first| first.downcast::<gtk::Image>().ok())
}

/// True when `path` is somewhere under the user's home.
pub fn is_under_home(path: &Path) -> bool {
    crate::model::path::is_ancestor(&crate::paths::home_dir(), path)
}
