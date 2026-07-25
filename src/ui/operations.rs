//! Driving file operations: dialogs in, jobs out, inverses recorded.
//!
//! The worker in [`crate::fs::ops`] does the syscalls; this decides what to ask
//! for, answers the questions it asks back, and records what happened so
//! Ctrl+Z has something true to reverse.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::TryRecvError;
use std::time::{Duration, Instant};

use adw::prelude::*;

use crate::fs::ops::{self, Event, Job, Outcome};
use crate::model::clipboard::{self as clip, FileClip, Intent};
use crate::model::format::human_bytes;
use crate::model::path::display_name;
use crate::model::preflight::Kind;
use crate::model::undo::{Moved, Operation};
use crate::ui::dialogs;
use crate::ui::progress::{Progress, SHOW_AFTER};
use crate::ui::window::Window;

/// How often the main loop drains the worker's channel.
const POLL: Duration = Duration::from_millis(50);

/// Which inverse, if any, an outcome should be recorded as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Record {
    /// Undo jobs and permanent delete: nothing to record.
    None,
    /// Copy, move, trash — read from whichever list the outcome filled in.
    Auto,
    Rename,
    Create,
}

impl Window {
    // ---- clipboard -------------------------------------------------------

    pub fn copy_selection(self: &Rc<Self>) {
        self.put_on_clipboard(Intent::Copy);
    }

    pub fn cut_selection(self: &Rc<Self>) {
        self.put_on_clipboard(Intent::Cut);
    }

    fn put_on_clipboard(self: &Rc<Self>, intent: Intent) {
        let paths = self.file_pane.selected_paths();
        if paths.is_empty() {
            self.show_toast("Nothing is selected");
            return;
        }

        let count = paths.len();
        self.clipboard.set(FileClip::new(intent, paths));

        let verb = match intent {
            Intent::Copy => "Copied",
            Intent::Cut => "Cut",
        };
        self.show_toast(&format!("{verb} {}", items(count as u64)));
    }

    pub fn paste(self: &Rc<Self>) {
        let Some(destination) = self.current_directory() else {
            self.show_toast("Cannot paste here");
            return;
        };

        let this = Rc::clone(self);
        self.clipboard.read(move |clip| {
            let Some(clip) = clip else {
                this.show_toast("The clipboard has no files in it");
                return;
            };

            // §10.1 hazard 8: cutting and pasting into the same folder is a
            // no-op, not a duplicate and certainly not a deletion.
            if clip::is_same_directory_move(&clip, &destination) {
                this.show_toast("Those files are already in this folder");
                return;
            }

            let kind = match clip.intent {
                Intent::Copy => Kind::Copy,
                Intent::Cut => Kind::Move,
            };
            let was_cut = clip.intent == Intent::Cut;

            this.run_job_then(
                this.transfer(kind, clip.paths, destination, None),
                Record::Auto,
                move |window, outcome| {
                    // The sources are gone, so the clipboard now points at
                    // nothing; forgetting it also silences the quit warning.
                    if was_cut && outcome.is_clean() {
                        window.clipboard.forget();
                    }
                },
            );
        });
    }

    pub fn duplicate_selection(self: &Rc<Self>) {
        let paths = self.file_pane.selected_paths();
        if paths.is_empty() {
            self.show_toast("Nothing is selected");
            return;
        }

        let Some(destination) = paths.first().and_then(|path| path.parent()) else {
            return;
        };
        let destination = destination.to_path_buf();

        // Every name collides with its own original, and the answer is always
        // the same, so the conflict dialog would only be in the way.
        let job = self.transfer(Kind::Copy, paths, destination, Some(ops::Action::KeepBoth));
        self.run_job(job, Record::Auto);
    }

    fn transfer(
        &self,
        kind: Kind,
        sources: Vec<PathBuf>,
        destination: PathBuf,
        blanket: Option<ops::Action>,
    ) -> Job {
        Job::Transfer {
            kind,
            sources,
            destination,
            follow_symlinks: self.config.borrow().behavior.follow_symlinks_on_copy,
            blanket,
        }
    }

    // ---- rename and creation ---------------------------------------------

    pub fn rename_selection(self: &Rc<Self>) {
        let paths = self.file_pane.selected_paths();
        let [path] = paths.as_slice() else {
            self.show_toast("Select a single item to rename");
            return;
        };

        let path = path.clone();
        let current = display_name(&path);
        let this = Rc::clone(self);

        glib::spawn_future_local(async move {
            let heading = format!("Rename “{current}”");
            let Some(name) = dialogs::ask_name(&this.window, &heading, "Rename", &current).await
            else {
                return;
            };
            if name == current {
                return;
            }

            this.run_job(Job::Rename { path, name }, Record::Rename);
        });
    }

    pub fn new_folder(self: &Rc<Self>) {
        self.create_entry(true, "New Folder", "Untitled Folder");
    }

    pub fn new_file(self: &Rc<Self>) {
        self.create_entry(false, "New File", "Untitled");
    }

    fn create_entry(
        self: &Rc<Self>,
        directory: bool,
        heading: &'static str,
        default_name: &'static str,
    ) {
        let Some(parent) = self.current_directory() else {
            self.show_toast("Cannot create anything here");
            return;
        };

        let this = Rc::clone(self);
        glib::spawn_future_local(async move {
            let suggestion = crate::model::naming::next_available(default_name, |candidate| {
                ops::exists(&parent.join(candidate))
            });

            let Some(name) = dialogs::ask_name(&this.window, heading, "Create", &suggestion).await
            else {
                return;
            };

            let path = parent.join(&name);
            this.run_job(Job::Create { path, directory }, Record::Create);
        });
    }

    // ---- trash and delete ------------------------------------------------

    pub fn trash_selection(self: &Rc<Self>) {
        let paths = self.file_pane.selected_paths();
        if paths.is_empty() {
            self.show_toast("Nothing is selected");
            return;
        }
        self.run_job(Job::Trash(paths), Record::Auto);
    }

    pub fn delete_selection(self: &Rc<Self>) {
        let paths = self.file_pane.selected_paths();
        if paths.is_empty() {
            self.show_toast("Nothing is selected");
            return;
        }

        let names: Vec<String> = paths.iter().map(|path| display_name(path)).collect();
        let this = Rc::clone(self);

        glib::spawn_future_local(async move {
            if dialogs::confirm_permanent_delete(&this.window, &names).await {
                this.run_job(Job::Delete(paths), Record::None);
            }
        });
    }

    /// §10.1 hazard 4: trash was refused, so say so and offer the alternative.
    fn offer_permanent_delete(self: &Rc<Self>, paths: Vec<PathBuf>) {
        let names: Vec<String> = paths.iter().map(|path| display_name(path)).collect();
        let this = Rc::clone(self);

        glib::spawn_future_local(async move {
            if dialogs::offer_delete_instead(&this.window, &names).await {
                this.run_job(Job::Delete(paths), Record::None);
            }
        });
    }

    // ---- undo ------------------------------------------------------------

    pub fn undo(self: &Rc<Self>) {
        let taken = self.undo.borrow_mut().take_next(&ops::Filesystem);
        self.sync_undo_action();

        let operation = match taken {
            Ok(operation) => operation,
            Err(refusal) => {
                tracing::info!(?refusal, "undo refused");
                self.show_toast(&refusal.message());
                return;
            }
        };

        let description = operation.describe_undo();
        let job = match operation {
            Operation::Trash(items) => Job::Restore(items),
            Operation::Rename { from, to } => Job::Revert(vec![Moved { from, to }]),
            Operation::Move(items) => Job::Revert(items),
            Operation::Copy(items) => Job::Discard(items),
            Operation::Create(item) => Job::Discard(vec![item]),
        };

        self.run_job_then(job, Record::None, move |window, outcome| {
            if outcome.is_clean() {
                window.show_toast(&description);
            }
        });
    }

    pub(crate) fn sync_undo_action(&self) {
        self.undo_action.set_enabled(!self.undo.borrow().is_empty());
    }

    // ---- running a job ---------------------------------------------------

    pub(crate) fn run_job(self: &Rc<Self>, job: Job, record: Record) {
        self.run_job_then(job, record, |_, _| {});
    }

    /// Start `job`, drive its dialogs, and run `then` once it has finished.
    pub(crate) fn run_job_then(
        self: &Rc<Self>,
        job: Job,
        record: Record,
        then: impl FnOnce(&Rc<Window>, &Outcome) + 'static,
    ) {
        if self.busy.replace(true) {
            self.show_toast("Hive is still busy with another operation");
            return;
        }

        let title = job.title();
        let past_tense = job.past_tense();
        let progress = Progress::new(title);

        let cancellable = gio::Cancellable::new();
        {
            let cancellable = cancellable.clone();
            progress.connect_cancel(move || cancellable.cancel());
        }

        let events = ops::spawn(job, cancellable);
        let started = Instant::now();
        let then = RefCell::new(Some(then));
        // While a question is on screen the progress dialog must not push its
        // way in front of it.
        let asking = Rc::new(std::cell::Cell::new(false));
        let this = Rc::clone(self);

        glib::timeout_add_local(POLL, move || {
            loop {
                match events.try_recv() {
                    Ok(Event::Finished(outcome)) => {
                        progress.close();
                        this.busy.set(false);
                        let outcome = this.finish_job(*outcome, record, past_tense);
                        if let Some(then) = then.borrow_mut().take() {
                            then(&this, &outcome);
                        }
                        return glib::ControlFlow::Break;
                    }
                    Ok(event) => this.apply_event(event, &progress, &asking),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        // The worker died without a word. Not expected, but the
                        // window must not be left permanently busy.
                        tracing::warn!("operation thread ended without an outcome");
                        progress.close();
                        this.busy.set(false);
                        return glib::ControlFlow::Break;
                    }
                }
            }

            if !asking.get() && started.elapsed() >= SHOW_AFTER {
                progress.present(&this.window);
            }
            glib::ControlFlow::Continue
        });
    }

    fn apply_event(
        self: &Rc<Self>,
        event: Event,
        progress: &Rc<Progress>,
        asking: &Rc<std::cell::Cell<bool>>,
    ) {
        match event {
            Event::Surveying(survey) => {
                progress.pulse(&format!("Checking {}…", items(survey.items)));
            }

            Event::Planned(plan) => {
                progress.set_strategy(plan.strategy.describe());
                progress.set_fraction(0.0, &format!("{} to go", items(plan.survey.items)));
            }

            Event::Progress {
                done_items,
                done_bytes,
                total,
                current,
            } => {
                // Bytes are the honest measure when there are any; a tree of
                // empty files has none, so fall back to counting entries.
                let fraction = if total.bytes > 0 {
                    done_bytes as f64 / total.bytes as f64
                } else if total.items > 0 {
                    done_items as f64 / total.items as f64
                } else {
                    0.0
                };

                let detail = if total.bytes > 0 {
                    format!(
                        "{current} — {} of {}",
                        human_bytes(done_bytes),
                        human_bytes(total.bytes)
                    )
                } else {
                    format!("{current} — {done_items} of {}", total.items)
                };
                progress.set_fraction(fraction, &detail);
            }

            Event::Conflict(conflict) => {
                asking.set(true);
                let this = Rc::clone(self);
                let asking = Rc::clone(asking);
                glib::spawn_future_local(async move {
                    let resolution = dialogs::resolve_conflict(&this.window, &conflict).await;
                    conflict.answer(resolution);
                    asking.set(false);
                });
            }

            Event::Finished(_) => {}
        }
    }

    /// Report what happened, record the inverse, and land somewhere valid.
    fn finish_job(
        self: &Rc<Self>,
        mut outcome: Outcome,
        record: Record,
        past_tense: &str,
    ) -> Outcome {
        if let Some(refusal) = &outcome.refusal {
            tracing::info!(?refusal, "operation refused");
            dialogs::show_refusal(&self.window, refusal);
            return outcome;
        }

        if !outcome.untrashable.is_empty() {
            self.offer_permanent_delete(std::mem::take(&mut outcome.untrashable));
        }

        self.record_inverse(&mut outcome, record);
        self.reveal_result(&outcome);

        if !outcome.errors.is_empty() {
            self.show_banner(&describe_errors(&outcome));
        }

        if let Some(summary) = summarize(&outcome, past_tense) {
            self.show_toast(&summary);
        }

        self.ensure_location_exists();

        // Removing the focused row drops focus out of the view entirely, which
        // would leave every keyboard shortcut dead until the user clicked.
        self.file_pane.focus_view();
        outcome
    }

    fn record_inverse(self: &Rc<Self>, outcome: &mut Outcome, record: Record) {
        if record == Record::None {
            return;
        }

        if !outcome.undoable {
            tracing::info!("operation too large to record an inverse for");
            return;
        }

        let trashed = std::mem::take(&mut outcome.trashed);
        let moved = std::mem::take(&mut outcome.moved);
        let created = std::mem::take(&mut outcome.created);

        let operation = match record {
            Record::Rename => match moved.into_iter().next() {
                Some(item) => Some(Operation::Rename {
                    from: item.from,
                    to: item.to,
                }),
                None => None,
            },
            Record::Create => created.into_iter().next().map(Operation::Create),
            Record::Auto => {
                if !trashed.is_empty() {
                    Some(Operation::Trash(trashed))
                } else if !moved.is_empty() {
                    Some(Operation::Move(moved))
                } else if !created.is_empty() {
                    Some(Operation::Copy(created))
                } else {
                    None
                }
            }
            Record::None => None,
        };

        if let Some(operation) = operation {
            self.undo.borrow_mut().push(operation);
            self.sync_undo_action();
        }
    }

    /// Select what the operation just produced, so the eye lands on it.
    fn reveal_result(self: &Rc<Self>, outcome: &Outcome) {
        let target = outcome
            .created
            .first()
            .map(|item| item.path.clone())
            .or_else(|| outcome.moved.first().map(|item| item.to.clone()));

        if let Some(target) = target
            && self.current_directory().as_deref() == target.parent()
        {
            self.file_pane.request_selection(target);
        }
    }

    /// §10.1 hazard 8: never sit on a directory that has just been deleted.
    pub(crate) fn ensure_location_exists(self: &Rc<Self>) {
        let Some(current) = self.current_directory() else {
            return;
        };
        if current.is_dir() {
            return;
        }

        let landing = nearest_existing(&current);
        tracing::info!(
            gone = %current.display(),
            landing = %landing.display(),
            "current folder disappeared"
        );
        self.show_banner(&format!(
            "“{}” is gone — showing {} instead",
            display_name(&current),
            display_name(&landing)
        ));
        self.navigate_to_path(&landing);
    }

    pub(crate) fn current_directory(&self) -> Option<PathBuf> {
        self.file_pane
            .location()
            .and_then(|file| file.path())
            .filter(|path| path.is_dir())
    }
}

/// Walk up until something still exists, falling back to home then the root.
fn nearest_existing(path: &Path) -> PathBuf {
    for ancestor in path.ancestors().skip(1) {
        if ancestor.is_dir() {
            return ancestor.to_path_buf();
        }
    }
    let home = crate::paths::home_dir();
    if home.is_dir() {
        home
    } else {
        PathBuf::from("/")
    }
}

fn items(count: u64) -> String {
    if count == 1 {
        "1 item".to_owned()
    } else {
        format!("{count} items")
    }
}

/// One line for the toast, or `None` when nothing at all happened.
fn summarize(outcome: &Outcome, past_tense: &str) -> Option<String> {
    if outcome.cancelled {
        return Some(if outcome.finished_items == 0 {
            "Stopped before anything changed".to_owned()
        } else {
            format!("Stopped — {} already done", items(outcome.finished_items))
        });
    }

    if outcome.finished_items == 0 && outcome.skipped == 0 {
        return None;
    }

    let mut summary = format!("{past_tense} {}", items(outcome.finished_items));
    if outcome.skipped > 0 {
        summary.push_str(&format!(", skipped {}", outcome.skipped));
    }
    Some(summary)
}

fn describe_errors(outcome: &Outcome) -> String {
    match outcome.error_count {
        0 => String::new(),
        1 => format!(
            "One item could not be handled — {}",
            outcome
                .errors
                .first()
                .map_or("unknown error", String::as_str)
        ),
        count => format!(
            "{count} items could not be handled — first: {}",
            outcome
                .errors
                .first()
                .map_or("unknown error", String::as_str)
        ),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn outcome_with(finished: u64, skipped: u64) -> Outcome {
        Outcome {
            finished_items: finished,
            skipped,
            ..Outcome::default()
        }
    }

    #[test]
    fn counts_are_pluralised() {
        assert_eq!(items(0), "0 items");
        assert_eq!(items(1), "1 item");
        assert_eq!(items(2), "2 items");
    }

    #[test]
    fn a_summary_names_what_happened() {
        assert_eq!(
            summarize(&outcome_with(3, 0), "Copied").as_deref(),
            Some("Copied 3 items")
        );
        assert_eq!(
            summarize(&outcome_with(1, 2), "Moved").as_deref(),
            Some("Moved 1 item, skipped 2")
        );
    }

    #[test]
    fn an_operation_that_did_nothing_says_nothing() {
        assert_eq!(summarize(&outcome_with(0, 0), "Copied"), None);
    }

    #[test]
    fn cancelling_reports_how_far_it_got() {
        let mut outcome = outcome_with(7, 0);
        outcome.cancelled = true;
        assert_eq!(
            summarize(&outcome, "Copied").as_deref(),
            Some("Stopped — 7 items already done")
        );

        let mut nothing = outcome_with(0, 0);
        nothing.cancelled = true;
        assert_eq!(
            summarize(&nothing, "Copied").as_deref(),
            Some("Stopped before anything changed")
        );
    }

    #[test]
    fn errors_are_summarised_with_a_count_and_an_example() {
        let mut outcome = outcome_with(2, 0);
        outcome.error_count = 3;
        outcome
            .errors
            .push("secret.txt: permission denied".to_owned());

        let text = describe_errors(&outcome);
        assert!(text.contains('3'), "{text}");
        assert!(text.contains("secret.txt"), "{text}");

        outcome.error_count = 1;
        assert!(describe_errors(&outcome).starts_with("One item"));
    }

    #[test]
    fn a_deleted_folder_falls_back_to_its_nearest_living_ancestor() {
        let dir = tempfile::tempdir().unwrap();
        let deep = dir.path().join("a/b/c");
        std::fs::create_dir_all(&deep).unwrap();

        assert_eq!(nearest_existing(&deep.join("gone")), deep);

        std::fs::remove_dir_all(dir.path().join("a/b")).unwrap();
        assert_eq!(nearest_existing(&deep), dir.path().join("a"));
    }

    #[test]
    fn a_path_with_no_living_ancestor_lands_somewhere_real() {
        let landing = nearest_existing(Path::new("/nonexistent-root-xyz/a/b"));
        assert!(landing.is_dir(), "{landing:?} should exist");
    }
}
