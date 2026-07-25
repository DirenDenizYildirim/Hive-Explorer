//! The right-click menu over the file pane.
//!
//! Two menus, chosen by what was clicked: one for a selection, one for the
//! folder itself. Both are rebuilt on every click, because "Open With" depends
//! on the content type of whatever is selected right now.

use std::rc::Rc;

use adw::prelude::*;

use crate::fs::open;
use crate::ui::color_picker;
use crate::ui::window::Window;

/// Roughly one row down from the top of the pane, for the keyboard-opened menu.
const ROW_HEIGHT_GUESS: i32 = 40;

/// Attach the menu to the window's file pane.
pub fn install(window: &Rc<Window>) {
    // Constructed with a real, empty model rather than NULL, so the popover is
    // fully built before the first `set_menu_model`.
    let popover = gtk::PopoverMenu::from_model(Some(&gio::Menu::new()));
    popover.set_has_arrow(false);
    popover.set_halign(gtk::Align::Start);
    popover.set_parent(window.file_pane.widget());

    // A popover parented to a widget has to be unparented before that widget
    // goes away, or GTK complains on teardown.
    {
        let popover = popover.clone();
        window
            .file_pane
            .widget()
            .connect_destroy(move |_| popover.unparent());
    }

    let this = Rc::clone(window);
    let by_pointer = popover.clone();
    window
        .file_pane
        .connect_context_menu(move |position, x, y| {
            let model = if position.is_some() {
                selection_menu(&this)
            } else {
                this.file_pane.unselect_all();
                folder_menu()
            };

            by_pointer.set_menu_model(Some(&model));
            attach_color_picker(&this, &by_pointer);
            by_pointer.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            by_pointer.popup();
        });

    // The Menu key and Shift+F10 are how the menu is reached without a mouse.
    // There is no pointer to aim at, so it opens over the pane itself.
    let action = gio::SimpleAction::new("context-menu", None);
    let this = Rc::clone(window);
    action.connect_activate(move |_, _| {
        let model = if this.file_pane.selected_count() > 0 {
            selection_menu(&this)
        } else {
            folder_menu()
        };
        popover.set_menu_model(Some(&model));
        attach_color_picker(&this, &popover);

        // There is no pointer position to use, so anchor it near the top of the
        // pane. Leaving the rectangle unset lets it inherit the last click.
        let pane = this.file_pane.widget();
        let anchor = gtk::gdk::Rectangle::new(pane.width() / 4, ROW_HEIGHT_GUESS, 1, 1);
        popover.set_pointing_to(Some(&anchor));
        popover.popup();
    });
    window.widget().add_action(&action);
}

fn selection_menu(window: &Rc<Window>) -> gio::Menu {
    let menu = gio::Menu::new();

    let open = gio::Menu::new();
    open.append(Some("Open"), Some("win.activate-selection"));
    if let Some(submenu) = open_with_menu(window) {
        open.append_submenu(Some("Open With"), &submenu);
    }
    menu.append_section(None, &open);

    // Colouring and pinning are folder-only, so the entries appear only when
    // there is a folder in the selection rather than sitting there greyed out.
    let folders = window.file_pane.selected_directories();
    if !folders.is_empty() {
        let folder = gio::Menu::new();
        folder.append_submenu(Some("Color"), &color_menu());

        if folders.iter().all(|path| window.is_pinned(path)) {
            folder.append(Some("Unpin from Sidebar"), Some("win.unpin"));
        } else {
            folder.append(Some("Pin to Sidebar"), Some("win.pin"));
        }
        menu.append_section(None, &folder);
    }

    let clipboard = gio::Menu::new();
    clipboard.append(Some("Cut"), Some("win.cut"));
    clipboard.append(Some("Copy"), Some("win.copy"));
    clipboard.append(Some("Paste"), Some("win.paste"));
    menu.append_section(None, &clipboard);

    let edit = gio::Menu::new();
    edit.append(Some("Rename…"), Some("win.rename"));
    edit.append(Some("Duplicate"), Some("win.duplicate"));
    menu.append_section(None, &edit);

    let remove = gio::Menu::new();
    remove.append(Some("Move to Trash"), Some("win.trash"));
    remove.append(Some("Delete Permanently…"), Some("win.delete"));
    menu.append_section(None, &remove);

    menu
}

fn folder_menu() -> gio::Menu {
    let menu = gio::Menu::new();

    let create = gio::Menu::new();
    create.append(Some("New Folder…"), Some("win.new-folder"));
    create.append(Some("New File…"), Some("win.new-file"));
    menu.append_section(None, &create);

    let clipboard = gio::Menu::new();
    clipboard.append(Some("Paste"), Some("win.paste"));
    clipboard.append(Some("Select All"), Some("win.select-all"));
    menu.append_section(None, &clipboard);

    let view = gio::Menu::new();
    view.append(Some("Show Hidden Files"), Some("win.toggle-hidden"));
    view.append(Some("Undo"), Some("win.undo"));
    menu.append_section(None, &view);

    menu
}

/// A submenu holding nothing but a placeholder for the swatch grid.
fn color_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let item = gio::MenuItem::new(None, None);
    item.set_attribute_value("custom", Some(&color_picker::CUSTOM_ID.to_variant()));
    menu.append_item(&item);
    menu
}

/// Put the real swatch grid where the placeholder is.
///
/// `GtkPopoverMenu` builds its widgets from the model, so this has to run after
/// every `set_menu_model` — and with a fresh grid each time, because the one
/// from the previous model has been torn down along with it.
fn attach_color_picker(window: &Rc<Window>, popover: &gtk::PopoverMenu) {
    if window.file_pane.selected_directories().is_empty() {
        return;
    }

    let picker = color_picker::build(window, popover);
    if !popover.add_child(&picker, color_picker::CUSTOM_ID) {
        tracing::warn!("could not place the folder-color swatches in the menu");
    }
}

/// Applications registered for the first selected file.
///
/// Resolved through `gio::AppInfo`, which reads `.desktop` files directly, so
/// nothing here depends on a desktop environment being present.
fn open_with_menu(window: &Rc<Window>) -> Option<gio::Menu> {
    let paths = window.file_pane.selected_paths();
    let first = paths.first()?;
    if first.is_dir() {
        return None;
    }

    let handlers = open::handlers_for(&gio::File::for_path(first));
    if handlers.is_empty() {
        return None;
    }

    let menu = gio::Menu::new();
    for app in handlers.iter().take(12) {
        let Some(id) = app.id() else {
            continue;
        };
        let item = gio::MenuItem::new(Some(&app.name()), None);
        item.set_action_and_target_value(Some("win.open-with"), Some(&id.to_variant()));
        menu.append_item(&item);
    }

    (menu.n_items() > 0).then_some(menu)
}
