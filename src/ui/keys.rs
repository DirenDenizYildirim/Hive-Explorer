//! Keyboard handling for the file pane: type-ahead and optional vim keys.
//!
//! Arrows, Home/End, Page Up/Down and Enter are handled natively by
//! `GtkColumnView` and `GtkGridView`; nothing here duplicates them.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::gdk;

use crate::model::filter;
use crate::ui::file_pane::FilePane;

/// How long a type-ahead buffer survives without another keystroke.
const TYPE_AHEAD_TIMEOUT: Duration = Duration::from_millis(1000);

/// Something the pane cannot do by itself.
pub enum Action {
    Parent,
    Activate,
}

type ActionHandler = Rc<dyn Fn(Action)>;
type VimPredicate = Rc<dyn Fn() -> bool>;

#[derive(Default)]
struct TypeAhead {
    buffer: RefCell<String>,
    last: RefCell<Option<Instant>>,
}

impl TypeAhead {
    /// Add a character, resetting the buffer if the user paused.
    fn push(&self, character: char) -> String {
        let now = Instant::now();
        let expired = self
            .last
            .borrow()
            .is_none_or(|last| now.duration_since(last) > TYPE_AHEAD_TIMEOUT);

        let mut buffer = self.buffer.borrow_mut();
        if expired {
            buffer.clear();
        }
        buffer.push(character);
        *self.last.borrow_mut() = Some(now);
        buffer.clone()
    }

    fn clear(&self) {
        self.buffer.borrow_mut().clear();
        *self.last.borrow_mut() = None;
    }
}

/// Install key handling on both views.
///
/// `vim_keys` is read through a closure rather than captured by value, so
/// toggling the setting takes effect without rebuilding the controllers.
pub fn install(
    pane: &Rc<FilePane>,
    vim_keys: impl Fn() -> bool + 'static,
    on_action: impl Fn(Action) + 'static,
) {
    let vim_keys: VimPredicate = Rc::new(vim_keys);
    let on_action: ActionHandler = Rc::new(on_action);

    // A controller belongs to exactly one widget, so each view needs its own.
    pane.column_view()
        .add_controller(controller(pane, &vim_keys, &on_action));
    pane.grid_view()
        .add_controller(controller(pane, &vim_keys, &on_action));
}

fn controller(
    pane: &Rc<FilePane>,
    vim_keys: &VimPredicate,
    on_action: &ActionHandler,
) -> gtk::EventControllerKey {
    let controller = gtk::EventControllerKey::new();

    let pane = Rc::clone(pane);
    let vim_keys = Rc::clone(vim_keys);
    let on_action = Rc::clone(on_action);
    let type_ahead = TypeAhead::default();
    // `gg` is a two-key sequence, so the first `g` has to be remembered.
    let pending_g = Cell::new(false);

    controller.connect_key_pressed(move |_, key, _, modifiers| {
        let plain = !modifiers.intersects(
            gdk::ModifierType::CONTROL_MASK
                | gdk::ModifierType::ALT_MASK
                | gdk::ModifierType::SUPER_MASK,
        );

        if key == gdk::Key::Escape {
            type_ahead.clear();
            pending_g.set(false);
            return glib::Propagation::Proceed;
        }

        // Backspace is bound here rather than as a window accelerator so it
        // cannot steal the key from the path entry or the filter box.
        if plain && key == gdk::Key::BackSpace {
            type_ahead.clear();
            on_action(Action::Parent);
            return glib::Propagation::Stop;
        }

        if plain && vim_keys() && handle_vim(key, modifiers, &pane, &pending_g, &on_action) {
            return glib::Propagation::Stop;
        }

        if plain
            && let Some(character) = key.to_unicode()
            && !character.is_control()
            && character != ' '
        {
            jump(&pane, &type_ahead.push(character));
            return glib::Propagation::Stop;
        }

        glib::Propagation::Proceed
    });

    controller
}

/// Returns true when the key was consumed as a vim binding.
fn handle_vim(
    key: gdk::Key,
    modifiers: gdk::ModifierType,
    pane: &Rc<FilePane>,
    pending_g: &Cell<bool>,
    on_action: &ActionHandler,
) -> bool {
    let shifted = modifiers.contains(gdk::ModifierType::SHIFT_MASK);

    if key == gdk::Key::G && shifted {
        pending_g.set(false);
        let count = pane.selection().n_items();
        if count > 0 {
            pane.select_only(count - 1);
        }
        return true;
    }

    if key == gdk::Key::g && !shifted {
        if pending_g.replace(false) {
            pane.select_only(0);
        } else {
            pending_g.set(true);
        }
        return true;
    }

    pending_g.set(false);

    match key {
        gdk::Key::j => move_by(pane, 1),
        gdk::Key::k => move_by(pane, -1),
        gdk::Key::h => on_action(Action::Parent),
        gdk::Key::l => on_action(Action::Activate),
        _ => return false,
    }
    true
}

/// Select the next entry matching `prefix`.
fn jump(pane: &Rc<FilePane>, prefix: &str) {
    let names = pane.visible_names();
    let borrowed: Vec<&str> = names.iter().map(String::as_str).collect();

    // A repeated single character cycles through matches; a growing prefix
    // re-matches from the current row so the selection does not jump away.
    let start = match pane.selected_position() {
        Some(position) if prefix.chars().count() == 1 => position as usize + 1,
        Some(position) => position as usize,
        None => 0,
    };

    if let Some(index) = filter::type_ahead_match(&borrowed, prefix, start) {
        pane.select_only(index as u32);
    }
}

fn move_by(pane: &Rc<FilePane>, delta: i32) {
    let count = pane.selection().n_items();
    if count == 0 {
        return;
    }

    let current = pane.selected_position().unwrap_or(0) as i64;
    let next = (current + i64::from(delta)).clamp(0, i64::from(count) - 1);
    pane.select_only(next as u32);
}
