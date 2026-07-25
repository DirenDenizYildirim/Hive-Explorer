//! The drag payload for pinning, and nothing else.
//!
//! Dragging files *out* to other applications is deferred to v1.1, and half of
//! it working by accident would be worse than none of it. So the payload is a
//! boxed type private to Hive rather than a `GFile` or a `text/uri-list`: GDK
//! has no serializer for it, so no other application can be persuaded to accept
//! the drag, while inside Hive the value passes through untouched.

use std::path::PathBuf;

use adw::prelude::*;

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
    pub fn content(self) -> gtk::gdk::ContentProvider {
        gtk::gdk::ContentProvider::for_value(&self.to_value())
    }
}

/// A drop target that accepts folders being pinned.
pub fn drop_target(handler: impl Fn(Vec<PathBuf>) -> bool + 'static) -> gtk::DropTarget {
    let target = gtk::DropTarget::new(FolderDrag::static_type(), gtk::gdk::DragAction::COPY);
    target.connect_drop(move |_, value, _, _| match value.get::<FolderDrag>() {
        Ok(drag) => handler(drag.paths),
        Err(error) => {
            tracing::warn!(%error, "drop carried something other than folders");
            false
        }
    });
    target
}
