//! The file clipboard.
//!
//! Two payloads go on together: `x-special/gnome-copied-files`, which carries
//! the cut-versus-copy distinction every other file manager reads, and
//! `text/uri-list`, which almost everything else reads. A plain-text form rides
//! along so pasting into a terminal gives the paths. Both are accepted on the
//! way back in — see [`crate::model::clipboard`] for the formats themselves.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gdk;

use crate::model::clipboard::{self, FileClip, GNOME_MIME, Intent, MIME_TYPES, URI_LIST_MIME};

/// Wraps the display clipboard with Hive's own view of what it put there.
pub struct FileClipboard {
    clipboard: gdk::Clipboard,
    /// What Hive last placed. Only meaningful while `clipboard.is_local()`.
    placed: RefCell<Option<FileClip>>,
}

impl FileClipboard {
    pub fn for_widget(widget: &impl IsA<gtk::Widget>) -> Rc<Self> {
        Rc::new(Self {
            clipboard: widget.as_ref().clipboard(),
            placed: RefCell::new(None),
        })
    }

    /// Offer a set of files, in every format worth offering.
    pub fn set(&self, clip: FileClip) {
        if clip.is_empty() {
            return;
        }

        let gnome = glib::Bytes::from_owned(clipboard::to_gnome(&clip).into_bytes());
        let uris = glib::Bytes::from_owned(clipboard::to_uri_list(&clip.paths).into_bytes());
        let text = clip
            .paths
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("\n");

        let union = gdk::ContentProvider::new_union(&[
            gdk::ContentProvider::for_bytes(GNOME_MIME, &gnome),
            gdk::ContentProvider::for_bytes(URI_LIST_MIME, &uris),
            gdk::ContentProvider::for_value(&text.to_value()),
        ]);

        if let Err(error) = self.clipboard.set_content(Some(&union)) {
            tracing::warn!(%error, "could not take the clipboard");
            return;
        }

        tracing::debug!(count = clip.paths.len(), intent = ?clip.intent, "clipboard set");
        *self.placed.borrow_mut() = Some(clip);
    }

    /// Read whatever files are on the clipboard, from any application.
    pub fn read(self: &Rc<Self>, on_ready: impl FnOnce(Option<FileClip>) + 'static) {
        let clipboard = self.clipboard.clone();

        glib::spawn_future_local(async move {
            let formats = MIME_TYPES;
            let read = clipboard
                .read_future(&formats, glib::Priority::DEFAULT)
                .await;

            let clip = match read {
                Ok((stream, mime)) => match drain(&stream).await {
                    Some(text) => clipboard::parse(&mime, &text),
                    None => None,
                },
                Err(error) => {
                    // An empty clipboard reports as an error, which is not worth
                    // showing anyone.
                    tracing::debug!(%error, "clipboard held nothing Hive can paste");
                    None
                }
            };

            on_ready(clip);
        });
    }

    /// True while Hive still owns a file clipboard.
    ///
    /// §10.1 hazard 2: on Wayland the clipboard is owned by the client process,
    /// so quitting throws the content away. `is_local` goes false the moment
    /// another application takes ownership, which is exactly when the warning
    /// stops being useful.
    pub fn owns_files(&self) -> bool {
        self.placed.borrow().is_some() && self.clipboard.is_local()
    }

    /// A description of what would be lost, for the quit warning.
    pub fn describe_owned(&self) -> Option<String> {
        let placed = self.placed.borrow();
        let clip = placed.as_ref()?;
        let verb = match clip.intent {
            Intent::Copy => "copied",
            Intent::Cut => "cut",
        };
        Some(match clip.paths.as_slice() {
            [only] => format!("You {verb} “{}”.", crate::model::path::display_name(only)),
            many => format!("You {verb} {} files.", many.len()),
        })
    }

    /// Forget what Hive placed, without disturbing the clipboard itself.
    pub fn forget(&self) {
        self.placed.borrow_mut().take();
    }
}

/// Read a clipboard stream to the end.
async fn drain(stream: &gio::InputStream) -> Option<String> {
    let sink = gio::MemoryOutputStream::new_resizable();
    let spliced = sink
        .splice_future(
            stream,
            gio::OutputStreamSpliceFlags::CLOSE_SOURCE | gio::OutputStreamSpliceFlags::CLOSE_TARGET,
            glib::Priority::DEFAULT,
        )
        .await;

    if let Err(error) = spliced {
        tracing::warn!(%error, "could not read the clipboard payload");
        return None;
    }

    // Payloads are URIs, so they are ASCII by construction; anything else is
    // not something Hive can turn into paths.
    String::from_utf8(sink.steal_as_bytes().to_vec()).ok()
}
