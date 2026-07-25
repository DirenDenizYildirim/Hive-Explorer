//! The folder-color swatch grid that lives inside the context menu.
//!
//! A grid of the fourteen accent slots plus None, drawn in the colours of the
//! flavor that is actually loaded — the stylesheet paints each swatch from the
//! same slot the folder will use, so what you pick is what you get.

use std::rc::Rc;

use adw::prelude::*;

use crate::theme::palette::Accent;
use crate::ui::window::Window;

/// The id the menu model uses for the custom section.
pub const CUSTOM_ID: &str = "folder-colors";

/// Seven across fits inside the 500 px window floor.
const COLUMNS: i32 = 7;

/// Build a fresh grid bound to `window`.
///
/// A new widget each time: the menu model is rebuilt on every right-click, and
/// a custom child cannot be handed to two menus at once.
pub fn build(window: &Rc<Window>, close: &gtk::PopoverMenu) -> gtk::Box {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
    content.set_margin_top(6);
    content.set_margin_bottom(6);
    content.set_margin_start(6);
    content.set_margin_end(6);

    let grid = gtk::Grid::builder()
        .row_spacing(6)
        .column_spacing(6)
        .halign(gtk::Align::Center)
        .build();

    for (index, accent) in Accent::ALL.into_iter().enumerate() {
        let swatch = gtk::Button::new();
        swatch.add_css_class("hive-swatch");
        swatch.add_css_class(accent.css_class());
        swatch.set_tooltip_text(Some(accent.display_name()));

        let window = Rc::clone(window);
        let popover = close.clone();
        swatch.connect_clicked(move |_| {
            popover.popdown();
            window.set_folder_color(Some(accent));
        });

        let index = i32::try_from(index).unwrap_or(0);
        grid.attach(&swatch, index % COLUMNS, index / COLUMNS, 1, 1);
    }

    content.append(&grid);

    let none = gtk::Button::with_label("None");
    none.add_css_class("flat");
    let window = Rc::clone(window);
    let popover = close.clone();
    none.connect_clicked(move |_| {
        popover.popdown();
        window.set_folder_color(None);
    });
    content.append(&none);

    content
}
