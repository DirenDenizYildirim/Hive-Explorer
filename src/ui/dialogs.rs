//! The dialogs file operations need: naming, conflicts, and refusals.
//!
//! Errors that happen *during* an operation are not here — those surface as
//! banners and toasts, because a stack of modal dialogs is worse than the
//! failure it reports. What is here are the questions that have to be answered
//! before Hive is willing to touch anything.

use adw::prelude::*;

use crate::fs::ops::{Action, Conflict, Facts, Resolution};
use crate::model::format::human_bytes;
use crate::model::naming;
use crate::model::path::{display_name, split_extension};
use crate::model::preflight::Refusal;

/// Ask for a filename, refusing to enable the action until it is usable.
pub async fn ask_name(
    parent: &impl IsA<gtk::Widget>,
    heading: &str,
    action_label: &str,
    initial: &str,
) -> Option<String> {
    let entry = gtk::Entry::builder()
        .text(initial)
        .activates_default(true)
        .build();

    // Select the stem only, so typing replaces the name and keeps `.txt`.
    let (stem, _) = split_extension(initial);
    let stem_length = i32::try_from(stem.chars().count()).unwrap_or(-1);
    entry.select_region(0, stem_length);

    let hint = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .visible(false)
        .build();
    hint.add_css_class("caption");
    hint.add_css_class("error");

    let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
    column.append(&entry);
    column.append(&hint);

    let dialog = adw::AlertDialog::new(Some(heading), None);
    dialog.set_extra_child(Some(&column));
    dialog.add_responses(&[("cancel", "Cancel"), ("accept", action_label)]);
    dialog.set_response_appearance("accept", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("accept"));
    dialog.set_close_response("cancel");

    let validate = {
        let dialog = dialog.clone();
        let hint = hint.clone();
        move |entry: &gtk::Entry| match naming::validate(&entry.text()) {
            Ok(()) => {
                dialog.set_response_enabled("accept", true);
                hint.set_visible(false);
            }
            Err(error) => {
                dialog.set_response_enabled("accept", false);
                hint.set_text(error.message());
                hint.set_visible(true);
            }
        }
    };
    validate(&entry);
    entry.connect_changed(validate);

    dialog.set_focus(Some(&entry));

    let response = dialog.choose_future(Some(parent)).await;
    (response == "accept").then(|| entry.text().to_string())
}

/// Replace / Skip / Rename, optionally for the rest of the operation.
pub async fn resolve_conflict(parent: &impl IsA<gtk::Widget>, conflict: &Conflict) -> Resolution {
    let name = display_name(&conflict.target);
    let folder = conflict
        .target
        .parent()
        .map(display_name)
        .unwrap_or_else(|| "the destination".to_owned());

    let heading = if conflict.target_facts.is_some_and(|facts| facts.is_dir) {
        format!("A folder named “{name}” is already in {folder}")
    } else {
        format!("A file named “{name}” is already in {folder}")
    };

    let body = format!(
        "Replacing it cannot be undone.\n\nExisting: {}\nNew: {}",
        describe(conflict.target_facts),
        describe(conflict.source_facts),
    );

    let apply_to_all = gtk::CheckButton::with_label("Do this for everything else");
    apply_to_all.set_halign(gtk::Align::Center);

    let dialog = adw::AlertDialog::new(Some(&heading), Some(&body));
    dialog.set_extra_child(Some(&apply_to_all));
    dialog.add_responses(&[
        ("cancel", "Cancel"),
        ("skip", "Skip"),
        ("keep", &format!("Keep Both ({})", conflict.suggested_name)),
        ("replace", "Replace"),
    ]);
    dialog.set_response_appearance("replace", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("keep"));
    dialog.set_close_response("cancel");

    let response = dialog.choose_future(Some(parent)).await;

    Resolution {
        action: match response.as_str() {
            "replace" => Action::Replace,
            "skip" => Action::Skip,
            "keep" => Action::KeepBoth,
            _ => Action::Cancel,
        },
        // Cancelling stops everything, so remembering it would be meaningless.
        apply_to_all: apply_to_all.is_active() && response != "cancel",
    }
}

/// Confirm a permanent delete. There is no undo for this and the dialog says so.
pub async fn confirm_permanent_delete(parent: &impl IsA<gtk::Widget>, names: &[String]) -> bool {
    let heading = match names {
        [only] => format!("Permanently delete “{only}”?"),
        many => format!("Permanently delete {} items?", many.len()),
    };

    let dialog = adw::AlertDialog::new(
        Some(&heading),
        Some("This cannot be undone. The items will not go to the Trash."),
    );
    dialog.add_responses(&[("cancel", "Cancel"), ("delete", "Delete")]);
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    dialog.choose_future(Some(parent)).await == "delete"
}

/// §10.1 hazard 4: this filesystem has no trash. Offer the only alternative
/// explicitly rather than silently doing nothing or silently deleting.
pub async fn offer_delete_instead(parent: &impl IsA<gtk::Widget>, names: &[String]) -> bool {
    let heading = match names {
        [only] => format!("“{only}” cannot go to the Trash"),
        many => format!("{} items cannot go to the Trash", many.len()),
    };

    let dialog = adw::AlertDialog::new(
        Some(&heading),
        Some(
            "The filesystem they are on has no Trash. Removable drives formatted \
             as FAT or exFAT are the usual case, and some temporary filesystems \
             behave the same way.\n\nDelete them permanently instead?",
        ),
    );
    dialog.add_responses(&[("cancel", "Cancel"), ("delete", "Delete Permanently")]);
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    dialog.choose_future(Some(parent)).await == "delete"
}

/// Refuse a transfer before it starts, showing why.
pub fn show_refusal(parent: &impl IsA<gtk::Widget>, refusal: &Refusal) {
    let dialog = adw::AlertDialog::new(Some(refusal.title()), Some(&refusal.message()));
    dialog.add_responses(&[("close", "Close")]);
    dialog.set_default_response(Some("close"));
    dialog.set_close_response("close");
    dialog.present(Some(parent));
}

/// §10.1 hazard 2: the Wayland clipboard dies with the process.
pub async fn confirm_quit_with_clipboard(parent: &impl IsA<gtk::Widget>, what: &str) -> bool {
    let dialog = adw::AlertDialog::new(
        Some("Quit and lose the clipboard?"),
        Some(&format!(
            "{what} Clipboard contents belong to the application that put them \
             there, so closing Hive discards them and nothing will paste.\n\n\
             Paste them somewhere first, or quit anyway.",
        )),
    );
    dialog.add_responses(&[("stay", "Don't Quit"), ("quit", "Quit Anyway")]);
    dialog.set_response_appearance("quit", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("stay"));
    dialog.set_close_response("stay");

    dialog.choose_future(Some(parent)).await == "quit"
}

/// One side of a conflict, as a line the dialog can show.
fn describe(facts: Option<Facts>) -> String {
    let Some(facts) = facts else {
        return "unknown".to_owned();
    };

    let when = glib::DateTime::from_unix_local(facts.modified / 1_000_000_000)
        .and_then(|time| time.format("%Y-%m-%d %H:%M"))
        .map(|text| text.to_string())
        .unwrap_or_else(|_| "unknown date".to_owned());

    if facts.is_dir {
        format!("folder, modified {when}")
    } else {
        format!("{}, modified {when}", human_bytes(facts.size))
    }
}
