//! The sidebar: Pinned, Places, Recents, Devices.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;

use crate::fs::places::{self, Place, PlaceKind};
use crate::fs::volumes::{self, MountCandidate};

/// How many recent files to list.
const RECENT_LIMIT: usize = 8;

type NavigateHandler = Rc<dyn Fn(gio::File)>;

pub struct Sidebar {
    container: gtk::Box,
    pinned: Section,
    places: Section,
    recents: Section,
    devices: Section,
    on_navigate: RefCell<Option<NavigateHandler>>,
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
        list.set_selection_mode(gtk::SelectionMode::Single);
        list.add_css_class("navigation-sidebar");

        container.append(&label);
        container.append(&list);
        Self { container, list }
    }

    fn clear(&self) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
    }

    /// Hide the whole group — label included — when it has no rows.
    fn sync_visibility(&self) {
        self.container
            .set_visible(self.list.first_child().is_some());
    }
}

impl Sidebar {
    pub fn new() -> Rc<Self> {
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

        let sidebar = Rc::new(Self {
            container,
            pinned,
            places,
            recents,
            devices,
            on_navigate: RefCell::new(None),
        });

        sidebar.rebuild_places();
        sidebar.rebuild_recents();
        sidebar.rebuild_devices();
        sidebar.pinned.sync_visibility();
        sidebar.watch_volumes();
        sidebar.watch_recents();

        sidebar
    }

    pub fn widget(&self) -> gtk::ScrolledWindow {
        // No width_request: AdwOverlaySplitView governs sidebar width, and a hard
        // request on top becomes an unshrinkable floor added to the content's
        // minimum, pushing the window's minimum past what a tiled layout allows.
        gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .child(&self.container)
            .build()
    }

    pub fn connect_navigate(self: &Rc<Self>, handler: impl Fn(gio::File) + 'static) {
        *self.on_navigate.borrow_mut() = Some(Rc::new(handler));
    }

    fn navigate(&self, file: gio::File) {
        let handler = self.on_navigate.borrow().clone();
        if let Some(handler) = handler {
            handler(file);
        }
    }

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
            section.list.unselect_all();
        }

        let Some(location) = location else {
            return;
        };
        let Some(path) = location.path() else {
            return;
        };

        let home = crate::paths::home_dir();
        let resolved = places::resolve(&home, glib_special_dir, |p| p.is_dir(), &trash_access());
        for (index, place) in resolved.iter().enumerate() {
            if place.path.as_deref() == Some(path.as_path())
                && let Some(row) = self.places.list.row_at_index(index as i32)
            {
                self.places.list.select_row(Some(&row));
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

/// True when `path` is somewhere under the user's home.
pub fn is_under_home(path: &Path) -> bool {
    crate::model::path::is_ancestor(&crate::paths::home_dir(), path)
}
