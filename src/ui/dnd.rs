//! Drag payloads, and the drop targets that accept them.
//!
//! A drag out of the file pane carries three payloads, and each has one job.
//! `GdkFileList` is the one the rest of the desktop understands: GDK serializes
//! it to `text/uri-list`, which is what a browser's upload field or a chat
//! window reads. [`FileDrag`] is a boxed type private to Hive carrying the same
//! paths losslessly; a drop that finds it knows the drag started here, and that
//! is what makes an internal drag a move where an external one is a copy.
//! [`FolderDrag`] carries the folders alone, and the sidebar's pin targets
//! accept nothing else — dragging plain files at the sidebar pins nothing.

use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gdk;

/// Set on a container while a drop would land in the folder it is showing.
pub const DROP_CLASS: &str = "hive-drop-active";

/// Set on a folder row while a drop would land inside that folder.
pub const DROP_INTO_CLASS: &str = "hive-drop-into";

/// Folders being dragged onto the sidebar to pin, or between pinned rows.
#[derive(Debug, Clone, glib::Boxed)]
#[boxed_type(name = "HiveFolderDrag")]
pub struct FolderDrag {
    pub paths: Vec<PathBuf>,
}

impl FolderDrag {
    pub fn new(paths: Vec<PathBuf>) -> Self {
        Self { paths }
    }

    /// Wrap the payload for a `GtkDragSource`.
    pub fn content(self) -> gdk::ContentProvider {
        gdk::ContentProvider::for_value(&self.to_value())
    }
}

/// Everything a drag out of the file pane carries, in Hive's own terms.
///
/// The paths travel as bytes rather than as URIs that have to be parsed back,
/// and the type's presence is how a drop tells a Hive drag from a foreign one.
#[derive(Debug, Clone, glib::Boxed)]
#[boxed_type(name = "HiveFileDrag")]
pub struct FileDrag {
    pub paths: Vec<PathBuf>,
}

impl FileDrag {
    pub fn new(paths: Vec<PathBuf>) -> Self {
        Self { paths }
    }

    pub fn content(self) -> gdk::ContentProvider {
        gdk::ContentProvider::for_value(&self.to_value())
    }
}

/// What a drag starting in the file pane is carrying.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Dragged {
    /// Everything being dragged, files and folders alike.
    pub paths: Vec<PathBuf>,
    /// Just the folders, which are the only thing the sidebar will pin.
    pub folders: Vec<PathBuf>,
}

/// What a drop is asking Hive to do with the files it carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropAction {
    Copy,
    Move,
}

/// Every payload a drag out of the file pane offers, or `None` for nothing.
pub fn payload(dragged: Dragged) -> Option<gdk::ContentProvider> {
    if dragged.paths.is_empty() {
        return None;
    }

    let files: Vec<gio::File> = dragged.paths.iter().map(gio::File::for_path).collect();
    let list = gdk::FileList::from_array(&files);

    let mut providers = vec![
        gdk::ContentProvider::for_value(&list.to_value()),
        FileDrag::new(dragged.paths).content(),
    ];

    if !dragged.folders.is_empty() {
        providers.push(FolderDrag::new(dragged.folders).content());
    }

    Some(gdk::ContentProvider::new_union(&providers))
}

/// The local paths a drop is carrying, whatever format it arrived in.
///
/// Anything that is not a file on this machine — an `https` URI dragged out of
/// a browser, a file on a share that was never mounted — has no path, and comes
/// back as nothing rather than as a path that looks real and is not.
pub fn dropped_paths(value: &glib::Value) -> Vec<PathBuf> {
    if let Ok(drag) = value.get::<FileDrag>() {
        return drag.paths;
    }
    if let Ok(list) = value.get::<gdk::FileList>() {
        return list.files().iter().filter_map(|file| file.path()).collect();
    }
    if let Ok(file) = value.get::<gio::File>() {
        return file.path().into_iter().collect();
    }

    tracing::warn!("a drop carried a format Hive asked for and cannot read");
    Vec::new()
}

/// Copy or move, given where the drag came from and which keys are held.
///
/// A drag that started in Hive is a move, the way it is in every file manager:
/// dragging a file into a folder puts it there rather than leaving a copy
/// behind. A drag from another application is a copy — that application still
/// owns its file, and most sources on a desktop offer nothing else. Ctrl asks
/// for a copy and Shift for a move, and neither can ask for something the source
/// has not offered: Shift over a drag that will only be copied is refused while
/// the pointer is still over the row, rather than quietly copying instead.
pub fn resolve(
    own: bool,
    offered: gdk::DragAction,
    modifiers: gdk::ModifierType,
) -> Option<DropAction> {
    // An offer of nothing at all is treated as a copy. Some sources never fill
    // the field in, and refusing every one of them would be worse than assuming
    // the safe half of the two.
    let can_copy = offered.contains(gdk::DragAction::COPY) || offered.is_empty();
    let can_move = offered.contains(gdk::DragAction::MOVE);

    let ctrl = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
    let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);

    // Both keys at once is no clearer than neither, so it says nothing.
    match (ctrl, shift) {
        (true, false) => return can_copy.then_some(DropAction::Copy),
        (false, true) => return can_move.then_some(DropAction::Move),
        _ => {}
    }

    match (own, can_move, can_copy) {
        (true, true, _) => Some(DropAction::Move),
        (_, _, true) => Some(DropAction::Copy),
        (_, true, false) => Some(DropAction::Move),
        _ => None,
    }
}

/// A drop target that accepts files, from Hive or from any other application.
///
/// `destination` is asked for the folder a drop would land in, and `None`
/// refuses the drop outright — that is how a row which is not a folder lets the
/// pane behind it have the drop instead. `perform` is handed the paths and what
/// the drop asked to do with them. `class` is set on the target's own widget
/// while the pointer is over it, so the landing place is visible.
pub fn file_target(
    class: &'static str,
    destination: impl Fn() -> Option<PathBuf> + 'static,
    perform: impl Fn(Vec<PathBuf>, PathBuf, DropAction) + 'static,
) -> gtk::DropTarget {
    let target = gtk::DropTarget::new(
        glib::Type::INVALID,
        gdk::DragAction::COPY | gdk::DragAction::MOVE,
    );
    target.set_types(&[
        FileDrag::static_type(),
        gdk::FileList::static_type(),
        gio::File::static_type(),
    ]);

    let destination = Rc::new(destination);

    let asked = Rc::clone(&destination);
    target.connect_accept(move |_, _| asked().is_some());

    let entered = Rc::clone(&destination);
    target.connect_enter(move |target, _, _| {
        if let Some(widget) = target.widget() {
            widget.add_css_class(class);
        }
        offer(target, &entered)
    });

    let moved = Rc::clone(&destination);
    target.connect_motion(move |target, _, _| offer(target, &moved));

    target.connect_leave(move |target| {
        if let Some(widget) = target.widget() {
            widget.remove_css_class(class);
        }
    });

    target.connect_drop(move |target, value, _, _| {
        // `leave` does arrive after a drop, but only once the operation this
        // starts has already had its say; clearing it here keeps the row from
        // staying lit under a progress dialog.
        if let Some(widget) = target.widget() {
            widget.remove_css_class(class);
        }

        let Some(folder) = destination() else {
            return false;
        };
        let Some(action) = asking(target) else {
            return false;
        };

        perform(dropped_paths(value), folder, action);
        true
    });

    target
}

/// A drop target that accepts folders being pinned.
pub fn drop_target(handler: impl Fn(Vec<PathBuf>) -> bool + 'static) -> gtk::DropTarget {
    let target = gtk::DropTarget::new(FolderDrag::static_type(), gdk::DragAction::COPY);
    target.connect_drop(move |_, value, _, _| match value.get::<FolderDrag>() {
        Ok(drag) => handler(drag.paths),
        Err(error) => {
            tracing::warn!(%error, "drop carried something other than folders");
            false
        }
    });
    target
}

/// The action to report while the pointer is over a target, in GDK's terms.
fn offer(
    target: &gtk::DropTarget,
    destination: &Rc<impl Fn() -> Option<PathBuf> + 'static>,
) -> gdk::DragAction {
    if destination().is_none() {
        return gdk::DragAction::empty();
    }

    match asking(target) {
        Some(DropAction::Copy) => gdk::DragAction::COPY,
        Some(DropAction::Move) => gdk::DragAction::MOVE,
        None => gdk::DragAction::empty(),
    }
}

/// What the drag under way is asking for, if Hive can do it at all.
fn asking(target: &gtk::DropTarget) -> Option<DropAction> {
    let drop = target.current_drop()?;
    let own = drop.formats().contains_type(FileDrag::static_type());
    resolve(own, drop.actions(), modifiers(target))
}

/// Which modifier keys are held right now.
///
/// The event being handled carries them, except when it does not: a drag holds
/// a grab, and the drop events GDK synthesizes under one can arrive with an
/// empty state. The keyboard is then the only witness left.
fn modifiers(target: &gtk::DropTarget) -> gdk::ModifierType {
    let state = target.current_event_state();
    if !state.is_empty() {
        return state;
    }

    gdk::Display::default()
        .and_then(|display| display.default_seat())
        .and_then(|seat| seat.keyboard())
        .map(|keyboard| keyboard.modifier_state())
        .unwrap_or_else(gdk::ModifierType::empty)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const BOTH: gdk::DragAction = gdk::DragAction::COPY.union(gdk::DragAction::MOVE);
    const NONE: gdk::ModifierType = gdk::ModifierType::empty();
    const CTRL: gdk::ModifierType = gdk::ModifierType::CONTROL_MASK;
    const SHIFT: gdk::ModifierType = gdk::ModifierType::SHIFT_MASK;

    #[test]
    fn a_drag_from_hive_moves_and_a_drag_from_elsewhere_copies() {
        assert_eq!(resolve(true, BOTH, NONE), Some(DropAction::Move));
        assert_eq!(resolve(false, BOTH, NONE), Some(DropAction::Copy));
    }

    #[test]
    fn ctrl_copies_and_shift_moves_whichever_side_the_drag_came_from() {
        assert_eq!(resolve(true, BOTH, CTRL), Some(DropAction::Copy));
        assert_eq!(resolve(false, BOTH, SHIFT), Some(DropAction::Move));
    }

    #[test]
    fn both_modifiers_together_say_nothing_and_the_default_stands() {
        assert_eq!(resolve(true, BOTH, CTRL | SHIFT), Some(DropAction::Move));
        assert_eq!(resolve(false, BOTH, CTRL | SHIFT), Some(DropAction::Copy));
    }

    #[test]
    fn what_the_source_never_offered_is_not_on_the_table() {
        let copy_only = gdk::DragAction::COPY;
        assert_eq!(resolve(true, copy_only, NONE), Some(DropAction::Copy));
        assert_eq!(
            resolve(false, copy_only, SHIFT),
            None,
            "shift cannot move what the source will only let us copy"
        );

        let move_only = gdk::DragAction::MOVE;
        assert_eq!(resolve(false, move_only, NONE), Some(DropAction::Move));
        assert_eq!(resolve(true, move_only, CTRL), None);
    }

    #[test]
    fn an_empty_offer_is_read_as_a_copy_rather_than_a_refusal() {
        let empty = gdk::DragAction::empty();
        assert_eq!(resolve(false, empty, NONE), Some(DropAction::Copy));
        assert_eq!(resolve(true, empty, NONE), Some(DropAction::Copy));
        assert_eq!(resolve(true, empty, SHIFT), None);
    }

    #[test]
    fn a_link_only_offer_is_refused_rather_than_guessed_at() {
        assert_eq!(resolve(true, gdk::DragAction::LINK, NONE), None);
        assert_eq!(resolve(false, gdk::DragAction::LINK, CTRL), None);
    }
}
