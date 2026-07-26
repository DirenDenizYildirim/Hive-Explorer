//! The Properties dialog: what a file is, and — only if asked — how big.
//!
//! §10.1 hazard 1 is the shape of this file. A directory's real size is an
//! unbounded tree walk, so opening this dialog computes nothing: the size row
//! for a folder shows a **Calculate** button, the walk runs on its own thread
//! in [`crate::fs::size`], the total updates as it counts, and the button turns
//! into Cancel while it runs. Closing the dialog cancels it too.
//!
//! Everything else is one `query_info_async` — no blocking stat on the main
//! thread, even for a file on a mount that has stopped answering.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;

use crate::fs::size;
use crate::model::format;
use crate::ui::window::Window;

/// Everything the dialog shows, in one query.
const ATTRIBUTES: &str = concat!(
    "standard::display-name,",
    "standard::type,",
    "standard::size,",
    "standard::content-type,",
    "standard::is-symlink,",
    "standard::symlink-target,",
    "unix::mode,",
    "owner::user,",
    "owner::group,",
    "time::modified,",
    "time::access,",
    "time::changed",
);

/// How often the size walk's channel is drained.
const POLL: std::time::Duration = std::time::Duration::from_millis(100);

/// Show properties for `paths`.
///
/// For a single entry the dialog is not put on screen until gio has answered.
/// It is built in one pass and never grows afterwards: a dialog sizes itself to
/// its contents when it is presented, so rows appended a moment later end up
/// clipped below the fold with nothing to say they are there.
pub fn present(window: &Rc<Window>, paths: Vec<PathBuf>) {
    if paths.is_empty() {
        window.show_toast("Select something first");
        return;
    }

    if paths.len() > 1 {
        build(window, &paths, None);
        return;
    }

    let file = gio::File::for_path(&paths[0]);
    let window = Rc::downgrade(window);
    file.query_info_async(
        ATTRIBUTES,
        // The dialog describes the thing that was selected. For a symlink that
        // is the link, not whatever it points at — the target gets its own row.
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        glib::Priority::DEFAULT,
        gio::Cancellable::NONE,
        move |result| {
            // The window can go away while a slow mount is still answering.
            let Some(window) = window.upgrade() else {
                return;
            };
            build(&window, &paths, Some(result));
        },
    );
}

/// Build the whole page and show it.
///
/// `info` is `None` for a multi-selection, which has no single set of facts to
/// report, and `Some(Err(..))` when the query failed — a dialog that says why
/// beats no dialog at all.
fn build(window: &Rc<Window>, paths: &[PathBuf], info: Option<Result<gio::FileInfo, glib::Error>>) {
    let Some(first) = paths.first() else {
        return;
    };

    let page = adw::PreferencesPage::new();
    let identity = adw::PreferencesGroup::new();
    page.add(&identity);

    let size_group = adw::PreferencesGroup::new();
    let size_row = adw::ActionRow::builder()
        .title("Size")
        .subtitle("—")
        .build();
    size_group.add(&size_row);
    page.add(&size_group);

    match info {
        None => {
            identity.add(&value_row("Selection", &format::item_count(paths.len())));
            if let Some(parent) = first.parent() {
                identity.add(&value_row("Location", &parent.display().to_string()));
            }
            install_calculate(paths, &size_row, "Add up the size of everything selected");
        }
        Some(info) => {
            identity.add(&value_row("Name", &crate::model::path::display_name(first)));
            if let Some(parent) = first.parent() {
                identity.add(&value_row("Location", &parent.display().to_string()));
            }
            describe(first, info, &identity, &page, &size_row);
        }
    }

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(
        &gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            // Without this a scroller asks for almost nothing, and the dialog
            // shrinks to one row on the grounds that the rest can be scrolled
            // to. The dialog is still clamped to the window, so a long list of
            // rows scrolls rather than growing past the edge.
            .propagate_natural_height(true)
            .child(&page)
            .build(),
    ));

    // Height is left to the content: the number of rows depends on what the
    // filesystem could answer, and a fixed height either wastes space on a
    // sparse answer or hides the permissions below a fold nobody scrolls to.
    // `adw::Dialog` clamps to the window, and the scroller covers the rest.
    let dialog = adw::Dialog::builder()
        .title("Properties")
        .content_width(460)
        .child(&toolbar)
        .build();
    dialog.present(Some(window.widget()));
}

/// Fill in type, timestamps, permissions and owner from what gio answered.
fn describe(
    path: &Path,
    info: Result<gio::FileInfo, glib::Error>,
    identity: &adw::PreferencesGroup,
    page: &adw::PreferencesPage,
    size_row: &adw::ActionRow,
) {
    let info = match info {
        Ok(info) => info,
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "could not read properties");
            size_row.set_subtitle(&format!("Unavailable — {}", error.message()));
            return;
        }
    };

    let is_directory = info.file_type() == gio::FileType::Directory;
    identity.add(&value_row("Type", &describe_type(&info, is_directory)));

    if info.is_symlink() {
        let target = info
            .symlink_target()
            .map(|target| target.display().to_string())
            .unwrap_or_else(|| "unreadable".to_owned());
        identity.add(&value_row("Links to", &target));
    }

    if is_directory {
        install_calculate(
            &[path.to_path_buf()],
            size_row,
            "Walk this folder and add up what is inside",
        );
    } else {
        size_row.set_subtitle(&exact_size(info.size().max(0) as u64));
    }

    let times = adw::PreferencesGroup::builder().title("Timestamps").build();
    let mut stamps = 0;
    for (label, attribute) in [
        ("Modified", gio::FILE_ATTRIBUTE_TIME_MODIFIED),
        ("Accessed", gio::FILE_ATTRIBUTE_TIME_ACCESS),
        ("Changed", gio::FILE_ATTRIBUTE_TIME_CHANGED),
    ] {
        if let Some(text) = timestamp(&info, attribute) {
            times.add(&value_row(label, &text));
            stamps += 1;
        }
    }
    if stamps > 0 {
        page.add(&times);
    }

    let access = adw::PreferencesGroup::builder().title("Access").build();
    let mut facts = 0;
    let mode = info.attribute_uint32(gio::FILE_ATTRIBUTE_UNIX_MODE);
    if mode != 0 {
        access.add(&value_row(
            "Permissions",
            &format!(
                "{} ({})",
                format::permissions(mode),
                format::permissions_octal(mode)
            ),
        ));
        facts += 1;
    }
    for (label, attribute) in [
        ("Owner", gio::FILE_ATTRIBUTE_OWNER_USER),
        ("Group", gio::FILE_ATTRIBUTE_OWNER_GROUP),
    ] {
        if let Some(value) = info.attribute_string(attribute) {
            access.add(&value_row(label, &value));
            facts += 1;
        }
    }
    if facts > 0 {
        page.add(&access);
    }
}

/// Put the **Calculate** button in the size row and wire it to a walk.
///
/// Nothing starts until it is pressed. While a walk runs the same button reads
/// Cancel, and the row's subtitle counts up; the row going away cancels it, so
/// a walk cannot outlive the dialog it was reporting to.
fn install_calculate(paths: &[PathBuf], size_row: &adw::ActionRow, tooltip: &str) {
    let button = gtk::Button::with_label("Calculate");
    button.set_valign(gtk::Align::Center);
    button.set_tooltip_text(Some(tooltip));
    button.add_css_class("flat");
    size_row.add_suffix(&button);
    size_row.set_activatable_widget(Some(&button));

    let running: Rc<RefCell<Option<gio::Cancellable>>> = Rc::new(RefCell::new(None));

    {
        let running = Rc::clone(&running);
        size_row.connect_destroy(move |_| {
            if let Ok(mut running) = running.try_borrow_mut()
                && let Some(cancellable) = running.take()
            {
                cancellable.cancel();
            }
        });
    }

    let paths = paths.to_vec();
    let row = size_row.clone();
    button.connect_clicked(move |button| {
        let Ok(mut slot) = running.try_borrow_mut() else {
            return;
        };
        if let Some(cancellable) = slot.take() {
            cancellable.cancel();
            button.set_label("Calculate");
            return;
        }

        let cancellable = gio::Cancellable::new();
        *slot = Some(cancellable.clone());
        drop(slot);

        button.set_label("Cancel");
        row.set_subtitle("Calculating…");

        let events = size::spawn(paths.clone(), cancellable);
        let row = row.clone();
        let button = button.clone();
        let running = Rc::clone(&running);

        glib::timeout_add_local(POLL, move || {
            loop {
                match events.try_recv() {
                    Ok(size::Event::Progress(tally)) => row.set_subtitle(&running_total(&tally)),
                    Ok(size::Event::Finished { tally, cancelled }) => {
                        row.set_subtitle(&final_total(&tally, cancelled));
                        finished(&button, &running);
                        return glib::ControlFlow::Break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        // The walker died without a word. Say so rather than
                        // leaving the row reading "Calculating…" forever.
                        tracing::warn!("the size walk ended without a total");
                        row.set_subtitle("Could not finish");
                        finished(&button, &running);
                        return glib::ControlFlow::Break;
                    }
                }
            }
            glib::ControlFlow::Continue
        });
    });
}

fn finished(button: &gtk::Button, running: &Rc<RefCell<Option<gio::Cancellable>>>) {
    button.set_label("Calculate");
    if let Ok(mut running) = running.try_borrow_mut() {
        running.take();
    }
}

fn running_total(tally: &size::Tally) -> String {
    format!(
        "{} in {}…",
        format::human_bytes(tally.bytes),
        format::item_count(tally.items() as usize)
    )
}

fn final_total(tally: &size::Tally, cancelled: bool) -> String {
    let mut text = format!(
        "{} in {}",
        exact_size(tally.bytes),
        format::item_count(tally.items() as usize)
    );
    if tally.unreadable > 0 {
        text.push_str(&format!(", {} unreadable", tally.unreadable));
    }
    if cancelled {
        text.push_str(" — stopped");
    }
    text
}

/// Human units plus the exact byte count, which is what you need when
/// comparing two copies of something.
fn exact_size(bytes: u64) -> String {
    if bytes < 1024 {
        return format::human_bytes(bytes);
    }
    format!("{} ({bytes} bytes)", format::human_bytes(bytes))
}

fn describe_type(info: &gio::FileInfo, is_directory: bool) -> String {
    if is_directory {
        return "Folder".to_owned();
    }
    let Some(content) = info.content_type() else {
        return "Unknown".to_owned();
    };
    format!(
        "{} ({content})",
        gio::functions::content_type_get_description(&content)
    )
}

/// A gio timestamp in local time, or `None` when the filesystem has no such
/// stamp — FAT has no ctime, and a network mount may report none at all.
fn timestamp(info: &gio::FileInfo, attribute: &str) -> Option<String> {
    let seconds = info.attribute_uint64(attribute);
    if seconds == 0 {
        return None;
    }
    glib::DateTime::from_unix_local(seconds as i64)
        .and_then(|time| time.format("%Y-%m-%d %H:%M:%S"))
        .ok()
        .map(|text| text.to_string())
}

fn value_row(title: &str, value: &str) -> adw::ActionRow {
    adw::ActionRow::builder()
        .title(title)
        .subtitle(value)
        .subtitle_selectable(true)
        .build()
}
