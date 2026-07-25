//! Folder colors and sidebar pinning, from the window's side.
//!
//! Both are small decisions about folders the user has singled out, and both
//! persist: colors in `folder-colors.toml`, pins in `config.toml`. Neither ever
//! writes anything into the folders themselves.

use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;

use crate::colors;
use crate::model::path::display_name;
use crate::theme::palette::Accent;
use crate::ui::sidebar::PinEvent;
use crate::ui::window::Window;

impl Window {
    /// Actions and the listeners both features need.
    ///
    /// No accelerators: pinning and colouring are reached from the context
    /// menu, which `Menu` and `Shift+F10` open, so neither needs to claim a key
    /// of its own.
    pub(crate) fn wire_folders(self: &Rc<Self>) {
        let pin = gio::SimpleAction::new("pin", None);
        let this = Rc::clone(self);
        pin.connect_activate(move |_, _| this.pin_selection());
        self.window.add_action(&pin);

        let unpin = gio::SimpleAction::new("unpin", None);
        let this = Rc::clone(self);
        unpin.connect_activate(move |_, _| this.unpin_selection());
        self.window.add_action(&unpin);

        // Also reachable without the mouse: the context menu opens on Menu or
        // Shift+F10, and the swatch grid inside it takes arrows and Space.
        let color = gio::SimpleAction::new("folder-color", Some(glib::VariantTy::STRING));
        let this = Rc::clone(self);
        color.connect_activate(move |_, parameter| {
            let Some(id) = parameter.and_then(glib::Variant::str) else {
                return;
            };
            this.set_folder_color(Accent::from_id(id));
        });
        self.window.add_action(&color);

        // Dropping a folder on the sidebar, reordering, and unpinning all land
        // here, whether they started in the sidebar or in the pane.
        let this = Rc::clone(self);
        self.sidebar.connect_pins_changed(move |event| {
            this.save_config();
            match event {
                PinEvent::Added(0) => this.show_toast("Already pinned"),
                PinEvent::Added(1) => this.show_toast("Pinned 1 folder"),
                PinEvent::Added(count) => this.show_toast(&format!("Pinned {count} folders")),
                PinEvent::Reordered => {}
                PinEvent::Removed(name) => this.show_toast(&format!("Unpinned “{name}”")),
            }
        });

        // Lazy pruning, per the build spec: stale entries go when the directory
        // they were in is next listed, and never in a startup scan.
        let this = Rc::clone(self);
        self.file_pane
            .directory_list()
            .connect_loading_notify(move |list| {
                if !list.is_loading() {
                    this.prune_folder_colors();
                }
            });
    }

    pub(crate) fn is_pinned(&self, path: &std::path::Path) -> bool {
        self.sidebar.is_pinned(path)
    }

    fn pin_selection(self: &Rc<Self>) {
        let folders = self.file_pane.selected_directories();
        if folders.is_empty() {
            self.show_toast("Only folders can be pinned");
            return;
        }
        self.sidebar.pin(&folders);
    }

    fn unpin_selection(self: &Rc<Self>) {
        let folders = self.file_pane.selected_directories();
        let mut removed = 0usize;
        for path in &folders {
            if self.sidebar.unpin(path) {
                removed += 1;
            }
        }
        if removed == 0 {
            self.show_toast("Not pinned");
        }
    }

    /// Colour every selected folder, or clear them when `accent` is `None`.
    pub fn set_folder_color(self: &Rc<Self>, accent: Option<Accent>) {
        let folders = self.file_pane.selected_directories();
        if folders.is_empty() {
            self.show_toast("Only folders can be colored");
            return;
        }

        let (changed, rejected) = {
            let Ok(mut store) = self.colors.try_borrow_mut() else {
                self.show_toast("Busy — try that again");
                return;
            };
            store.set_all(folders.iter().map(PathBuf::as_path), accent)
        };

        if changed > 0 {
            self.save_folder_colors();
            self.file_pane.refresh_folder_colors();
            self.sidebar.refresh_folder_colors();
        }

        // A TOML key is text, so a folder whose name is not valid UTF-8 cannot
        // be stored under one. Saying so beats a colour that silently does not
        // stick, or a lossy key that would colour some other folder.
        if let Some(first) = rejected.first() {
            tracing::warn!(path = %first.display(), "folder name is not valid UTF-8");
            self.show_toast(&format!(
                "“{}” cannot be colored — its name is not valid text",
                display_name(first)
            ));
            return;
        }

        if changed == 0 {
            return;
        }

        let what = if changed == 1 {
            format!("“{}”", display_name(&folders[0]))
        } else {
            format!("{changed} folders")
        };
        self.show_toast(&match accent {
            Some(accent) => format!("{what} is now {}", accent.display_name().to_lowercase()),
            None => format!("Cleared the color on {what}"),
        });
    }

    /// Drop stored colors for folders that are no longer in this directory.
    ///
    /// Only the entries recorded for *this* directory are candidates, and the
    /// existence check runs off the main thread — nothing here walks a tree,
    /// and the usual case is no candidates and no work at all.
    fn prune_folder_colors(self: &Rc<Self>) {
        let Some(directory) = self.current_directory() else {
            return;
        };

        let candidates = match self.colors.try_borrow() {
            Ok(store) if !store.is_empty() => store.paths_in(&directory),
            _ => return,
        };
        if candidates.is_empty() {
            return;
        }

        let this = Rc::clone(self);
        glib::spawn_future_local(async move {
            let Ok(gone) = gio::spawn_blocking(move || {
                candidates
                    .into_iter()
                    .filter(|path| !path.is_dir())
                    .collect::<Vec<PathBuf>>()
            })
            .await
            else {
                return;
            };

            if gone.is_empty() {
                return;
            }

            let dropped = match this.colors.try_borrow_mut() {
                Ok(mut store) => store.forget_all(gone.iter().map(PathBuf::as_path)),
                Err(_) => return,
            };

            if dropped > 0 {
                tracing::debug!(dropped, "pruned folder colors for paths that are gone");
                this.save_folder_colors();
            }
        });
    }

    /// Persist the folder colors, reporting failure without interrupting.
    pub(crate) fn save_folder_colors(&self) {
        let path = crate::paths::folder_colors_file();
        let Ok(store) = self.colors.try_borrow() else {
            return;
        };
        if let Err(error) = colors::save(&path, &store) {
            tracing::warn!(%error, path = %path.display(), "could not save folder colors");
            self.show_toast("Could not save folder colors");
        }
    }
}
