//! The file-operation engine: everything that touches many files, off the main
//! thread.
//!
//! One worker thread runs a [`Job`] and reports [`Event`]s down a channel the
//! main loop drains. Conflicts travel the other way as a blocking round trip:
//! the worker parks on a reply channel while the UI shows a dialog, which keeps
//! every policy decision on the main thread and every syscall off it.
//!
//! The worker builds `gio` objects from paths inside its own thread rather than
//! receiving them, so nothing GTK-shaped crosses the boundary.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, SyncSender, sync_channel};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use gio::prelude::*;

use crate::fs::trash::{self, TrashError};
use crate::model::naming;
use crate::model::path::display_name;
use crate::model::preflight::{self, Kind, Plan, Refusal, Strategy, Survey};
use crate::model::undo::{Created, Moved, Stamp, Trashed};

/// How often the worker is allowed to push a progress event.
const REPORT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(80);

/// Error messages kept verbatim; the rest are counted only.
const MAX_ERRORS: usize = 20;

/// Above this many recorded entries an operation is reported as not undoable
/// rather than kept, since the inverse would no longer fit a bounded stack.
const MAX_UNDO_ITEMS: usize = 10_000;

/// Guards against symlink loops when following links is switched on.
const MAX_DEPTH: usize = 64;

/// What the worker has been asked to do.
#[derive(Debug)]
pub enum Job {
    Transfer {
        kind: Kind,
        sources: Vec<PathBuf>,
        destination: PathBuf,
        follow_symlinks: bool,
        /// A decision made up front, so the worker never asks. Duplicate uses
        /// it: the conflict is with the original and the answer is always
        /// "keep both", so a dialog would only be in the way.
        blanket: Option<Action>,
    },
    Trash(Vec<PathBuf>),
    /// Permanent delete. Never recorded for undo — it has no inverse.
    Delete(Vec<PathBuf>),
    Rename {
        path: PathBuf,
        name: String,
    },
    Create {
        path: PathBuf,
        directory: bool,
    },
    /// Undo of a trash.
    Restore(Vec<Trashed>),
    /// Undo of a move or rename.
    Revert(Vec<Moved>),
    /// Undo of a copy or a creation.
    Discard(Vec<Created>),
}

impl Job {
    /// Title for the progress dialog.
    pub const fn title(&self) -> &'static str {
        match self {
            Job::Transfer {
                kind: Kind::Copy, ..
            } => "Copying",
            Job::Transfer {
                kind: Kind::Move, ..
            } => "Moving",
            Job::Trash(_) => "Moving to Trash",
            Job::Delete(_) => "Deleting",
            Job::Rename { .. } => "Renaming",
            Job::Create { .. } => "Creating",
            Job::Restore(_) => "Restoring from Trash",
            Job::Revert(_) => "Undoing",
            Job::Discard(_) => "Undoing",
        }
    }

    /// How the summary toast names what happened.
    pub const fn past_tense(&self) -> &'static str {
        match self {
            Job::Transfer {
                kind: Kind::Copy, ..
            } => "Copied",
            Job::Transfer {
                kind: Kind::Move, ..
            } => "Moved",
            Job::Trash(_) => "Moved to Trash",
            Job::Delete(_) => "Deleted",
            Job::Rename { .. } => "Renamed",
            Job::Create { .. } => "Created",
            Job::Restore(_) => "Restored",
            Job::Revert(_) | Job::Discard(_) => "Undid",
        }
    }
}

/// Size, time and kind of one end of a conflict, for the dialog to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Facts {
    pub size: u64,
    pub modified: i64,
    pub is_dir: bool,
}

impl Facts {
    fn of(metadata: &std::fs::Metadata) -> Self {
        let stamp = Stamp::of(metadata);
        Self {
            size: stamp.size,
            modified: stamp.modified,
            is_dir: metadata.is_dir(),
        }
    }
}

/// What to do about a name that is already taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Replace,
    Skip,
    /// Keep both, under an automatically generated name.
    KeepBoth,
    Cancel,
}

/// The dialog's answer, plus whether it stands for the rest of the operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolution {
    pub action: Action,
    pub apply_to_all: bool,
}

impl Resolution {
    pub const fn once(action: Action) -> Self {
        Self {
            action,
            apply_to_all: false,
        }
    }
}

/// A name collision the worker cannot decide by itself.
#[derive(Debug)]
pub struct Conflict {
    pub source: PathBuf,
    pub target: PathBuf,
    pub source_facts: Option<Facts>,
    pub target_facts: Option<Facts>,
    /// The name "Keep both" would use, so the dialog can show it.
    pub suggested_name: String,
    reply: SyncSender<Resolution>,
}

impl Conflict {
    /// Release the worker. Dropping without answering cancels, which is the
    /// safe reading of a dialog that was dismissed.
    pub fn answer(self, resolution: Resolution) {
        let _ = self.reply.send(resolution);
    }
}

impl Drop for Conflict {
    fn drop(&mut self) {
        let _ = self.reply.try_send(Resolution::once(Action::Cancel));
    }
}

/// Progress and questions travelling from the worker to the main loop.
#[derive(Debug)]
pub enum Event {
    /// Walking the sources. Totals are partial, so show an indeterminate bar.
    Surveying(Survey),
    /// The walk finished; the strategy and totals are now known.
    Planned(Plan),
    Progress {
        done_items: u64,
        done_bytes: u64,
        total: Survey,
        current: String,
    },
    Conflict(Conflict),
    Finished(Box<Outcome>),
}

/// What actually happened, including everything undo needs.
#[derive(Debug, Default)]
pub struct Outcome {
    /// Entries that did not exist before, so removing them is a true inverse.
    pub created: Vec<Created>,
    pub moved: Vec<Moved>,
    pub trashed: Vec<Trashed>,
    /// Paths whose filesystem has no trash — §10.1 hazard 4.
    pub untrashable: Vec<PathBuf>,
    pub strategy: Option<Strategy>,
    pub refusal: Option<Refusal>,
    pub errors: Vec<String>,
    pub error_count: u64,
    pub skipped: u64,
    pub finished_items: u64,
    pub cancelled: bool,
    /// False when the operation was too large to record an inverse for.
    pub undoable: bool,
    /// Wall-clock nanoseconds at which the last entry was written.
    pub completed_at: i64,
}

impl Outcome {
    pub fn is_clean(&self) -> bool {
        self.errors.is_empty() && !self.cancelled && self.refusal.is_none()
    }
}

/// Start a job. The receiver must be drained from the main loop.
pub fn spawn(job: Job, cancellable: gio::Cancellable) -> Receiver<Event> {
    let (events, receiver) = std::sync::mpsc::channel();
    let fallback = events.clone();

    if let Err(error) = std::thread::Builder::new()
        .name("hive-ops".to_owned())
        .spawn(move || {
            let mut worker = Worker::new(events, cancellable);
            worker.run(job);
        })
    {
        // Out of threads. Report it the same way any other failure arrives, so
        // the caller has one path to handle rather than two.
        tracing::error!(%error, "could not start the operation thread");
        let mut outcome = Outcome::default();
        outcome.errors.push(format!("could not start: {error}"));
        outcome.error_count = 1;
        let _ = fallback.send(Event::Finished(Box::new(outcome)));
    }

    receiver
}

/// Whether to keep walking or unwind because the user cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flow {
    Continue,
    Cancel,
}

/// Copy or move, and whether a move can be a rename here.
#[derive(Debug, Clone, Copy)]
struct Mode {
    kind: Kind,
    same_filesystem: bool,
    follow_symlinks: bool,
}

struct Worker {
    events: Sender<Event>,
    cancellable: gio::Cancellable,
    total: Survey,
    done_items: u64,
    done_bytes: u64,
    last_report: Instant,
    /// An "apply to all" answer, remembered so the worker stops asking.
    blanket: Option<Action>,
    outcome: Outcome,
}

impl Worker {
    fn new(events: Sender<Event>, cancellable: gio::Cancellable) -> Self {
        Self {
            events,
            cancellable,
            total: Survey::default(),
            done_items: 0,
            done_bytes: 0,
            last_report: Instant::now(),
            blanket: None,
            outcome: Outcome {
                undoable: true,
                ..Outcome::default()
            },
        }
    }

    fn run(&mut self, job: Job) {
        match job {
            Job::Transfer {
                kind,
                sources,
                destination,
                follow_symlinks,
                blanket,
            } => {
                self.blanket = blanket;
                self.transfer(kind, sources, destination, follow_symlinks);
            }
            Job::Trash(paths) => self.trash(paths),
            Job::Delete(paths) => self.delete(paths),
            Job::Rename { path, name } => self.rename_one(path, &name),
            Job::Create { path, directory } => self.create_one(path, directory),
            Job::Restore(items) => self.restore(items),
            Job::Revert(items) => self.revert(items),
            Job::Discard(items) => self.discard(items),
        }

        self.outcome.cancelled |= self.cancellable.is_cancelled();
        self.outcome.completed_at = now_nanos();
        self.outcome.undoable &=
            self.outcome.created.len() + self.outcome.moved.len() <= MAX_UNDO_ITEMS;

        let outcome = std::mem::take(&mut self.outcome);
        let _ = self.events.send(Event::Finished(Box::new(outcome)));
    }

    // ---- reporting -------------------------------------------------------

    fn cancelled(&self) -> bool {
        self.cancellable.is_cancelled()
    }

    fn report(&mut self, current: &Path) {
        if self.last_report.elapsed() < REPORT_INTERVAL {
            return;
        }
        self.last_report = Instant::now();
        let _ = self.events.send(Event::Progress {
            done_items: self.done_items,
            done_bytes: self.done_bytes,
            total: self.total,
            current: display_name(current),
        });
    }

    fn error(&mut self, context: &Path, message: impl std::fmt::Display) {
        self.outcome.error_count += 1;
        if self.outcome.errors.len() < MAX_ERRORS {
            self.outcome
                .errors
                .push(format!("{}: {message}", display_name(context)));
        }
        tracing::warn!(path = %context.display(), %message, "operation error");
    }

    /// Ask the main thread, unless a previous answer already covers this.
    fn resolve(&mut self, source: &Path, target: &Path, suggested: &OsStr) -> Action {
        if let Some(action) = self.blanket {
            return action;
        }

        let (reply, answers) = sync_channel(1);
        let conflict = Conflict {
            source: source.to_path_buf(),
            target: target.to_path_buf(),
            source_facts: std::fs::symlink_metadata(source)
                .ok()
                .map(|m| Facts::of(&m)),
            target_facts: std::fs::symlink_metadata(target)
                .ok()
                .map(|m| Facts::of(&m)),
            suggested_name: suggested.to_string_lossy().into_owned(),
            reply,
        };

        if self.events.send(Event::Conflict(conflict)).is_err() {
            return Action::Cancel;
        }

        // Blocking here is the point: this thread is not the main loop, and the
        // alternative is guessing what the user wanted.
        let Ok(resolution) = answers.recv() else {
            return Action::Cancel;
        };

        if resolution.apply_to_all {
            self.blanket = Some(resolution.action);
        }
        resolution.action
    }

    // ---- copy and move ---------------------------------------------------

    fn transfer(
        &mut self,
        kind: Kind,
        sources: Vec<PathBuf>,
        destination: PathBuf,
        follow_symlinks: bool,
    ) {
        // Canonical forms are for the recursion check only. Operating on them
        // would resolve a selected symlink to its target, which both copies the
        // wrong thing and files it under the wrong name.
        let resolved: Vec<PathBuf> = sources.iter().map(|path| canonical(path)).collect();
        if let Err(refusal) = preflight::validate(&resolved, &canonical(&destination)) {
            self.outcome.refusal = Some(refusal);
            return;
        }

        for source in &sources {
            if std::fs::symlink_metadata(source).is_err() {
                self.outcome.refusal = Some(Refusal::Missing(source.clone()));
                return;
            }
        }

        let Some(survey) = self.survey(&sources, follow_symlinks) else {
            return;
        };
        self.total = survey;

        let same_filesystem = sources
            .iter()
            .all(|source| same_filesystem(source, &destination));
        let strategy = preflight::strategy(kind, same_filesystem);
        let plan = preflight::plan(kind, strategy, survey);

        if let Err(refusal) = preflight::check_space(plan.required, free_space(&destination)) {
            self.outcome.refusal = Some(refusal);
            return;
        }

        self.outcome.strategy = Some(strategy);
        let _ = self.events.send(Event::Planned(plan));

        let mode = Mode {
            kind,
            same_filesystem,
            follow_symlinks,
        };

        for source in &sources {
            if self.cancelled() {
                break;
            }
            if self.place(source, &destination, mode, true, &mut Vec::new()) == Flow::Cancel {
                break;
            }
        }
    }

    /// Walk the sources for a total, reporting as it goes so a slow walk shows
    /// something. Returns `None` when the user cancelled mid-walk.
    fn survey(&mut self, sources: &[PathBuf], follow_symlinks: bool) -> Option<Survey> {
        let mut survey = Survey::default();
        for source in sources {
            self.survey_entry(source, follow_symlinks, &mut survey, &mut Vec::new())?;
        }
        Some(survey)
    }

    fn survey_entry(
        &mut self,
        path: &Path,
        follow_symlinks: bool,
        survey: &mut Survey,
        chain: &mut Vec<(u64, u64)>,
    ) -> Option<()> {
        if self.cancelled() {
            return None;
        }

        let Ok(metadata) = std::fs::symlink_metadata(path) else {
            // Vanished between selection and the walk; the transfer will report it.
            return Some(());
        };

        let is_link = metadata.is_symlink();
        if is_link && !follow_symlinks {
            survey.add_file(0);
            return Some(());
        }

        let resolved = if is_link {
            std::fs::metadata(path).ok()?
        } else {
            metadata
        };

        if !resolved.is_dir() {
            survey.add_file(resolved.len());
        } else {
            survey.add_directory();
            let Some(guard) = enter(&resolved, chain) else {
                return Some(());
            };
            if let Ok(listing) = std::fs::read_dir(path) {
                for entry in listing.flatten() {
                    self.survey_entry(&entry.path(), follow_symlinks, survey, chain)?;
                }
            }
            chain.truncate(guard);
        }

        if self.last_report.elapsed() >= REPORT_INTERVAL {
            self.last_report = Instant::now();
            let _ = self.events.send(Event::Surveying(*survey));
        }
        Some(())
    }

    /// Put `source` inside `dest_dir`, resolving a name collision first.
    ///
    /// `record` is false once an enclosing directory has already been recorded
    /// as created: removing that directory removes everything below it, so
    /// recording the contents too would make undo do the same work twice.
    fn place(
        &mut self,
        source: &Path,
        dest_dir: &Path,
        mode: Mode,
        record: bool,
        chain: &mut Vec<(u64, u64)>,
    ) -> Flow {
        if self.cancelled() {
            return Flow::Cancel;
        }

        let Some(name) = source.file_name().map(OsStr::to_os_string) else {
            self.error(source, "has no name");
            return Flow::Continue;
        };

        let mut target = dest_dir.join(&name);
        let mut replacing = false;

        if exists(&target) {
            let suggested = keep_both_name(dest_dir, &name);
            match self.resolve(source, &target, &suggested) {
                Action::Cancel => return Flow::Cancel,
                Action::Skip => {
                    self.outcome.skipped += 1;
                    return Flow::Continue;
                }
                Action::Replace => replacing = true,
                Action::KeepBoth => target = dest_dir.join(&suggested),
            }
        }

        self.transfer_entry(source, &target, replacing, mode, record, chain)
    }

    fn transfer_entry(
        &mut self,
        source: &Path,
        target: &Path,
        replacing: bool,
        mode: Mode,
        record: bool,
        chain: &mut Vec<(u64, u64)>,
    ) -> Flow {
        let Ok(metadata) = std::fs::symlink_metadata(source) else {
            self.error(source, "vanished before it could be transferred");
            return Flow::Continue;
        };

        let source_is_dir = if metadata.is_symlink() && mode.follow_symlinks {
            std::fs::metadata(source).is_ok_and(|resolved| resolved.is_dir())
        } else {
            metadata.is_dir()
        };

        // Two directories merge. Anything else replaces, which means the old
        // entry has to go first.
        let merging = replacing && source_is_dir && target.is_dir() && !is_symlink(target);
        let replaced = replacing && !merging;

        if replaced && let Err(error) = remove_tree(target) {
            self.error(target, error);
            return Flow::Continue;
        }

        // Undo of a copy deletes what it created, which cannot bring back what
        // Replace destroyed — so a replaced target is deliberately not recorded.
        // A move is different: putting the source back destroys nothing.
        let record = record && (mode.kind == Kind::Move || !replaced);
        let existed = merging;

        if source_is_dir {
            self.transfer_directory(source, target, existed, mode, record, chain)
        } else {
            self.transfer_file(source, target, existed, mode, record)
        }
    }

    fn transfer_directory(
        &mut self,
        source: &Path,
        target: &Path,
        existed: bool,
        mode: Mode,
        record: bool,
        chain: &mut Vec<(u64, u64)>,
    ) -> Flow {
        // A same-filesystem move of a whole directory is one rename, as long as
        // nothing is in the way.
        if mode.kind == Kind::Move && mode.same_filesystem && !existed {
            match std::fs::rename(source, target) {
                Ok(()) => {
                    self.finish_entry(source, target, 0, mode, record, false);
                    return Flow::Continue;
                }
                Err(error) if error.raw_os_error() != Some(18) => {
                    self.error(source, error);
                    return Flow::Continue;
                }
                // EXDEV: the filesystem check was wrong. Fall through and copy.
                Err(_) => {}
            }
        }

        if !existed && let Err(error) = std::fs::create_dir(target) {
            self.error(target, error);
            return Flow::Continue;
        }

        let Ok(resolved) = std::fs::metadata(source) else {
            self.error(source, "vanished while being read");
            return Flow::Continue;
        };
        let Some(guard) = enter(&resolved, chain) else {
            self.error(source, "symlink loop; not followed");
            return Flow::Continue;
        };

        let errors_before = self.outcome.error_count;
        let mut flow = Flow::Continue;

        match std::fs::read_dir(source) {
            Ok(listing) => {
                for entry in listing.flatten() {
                    // The directory itself is the recorded unit when Hive
                    // created it; a merge has to record each child instead.
                    let child_record = record && existed;
                    if self.place(&entry.path(), target, mode, child_record, chain) == Flow::Cancel
                    {
                        flow = Flow::Cancel;
                        break;
                    }
                }
            }
            Err(error) => self.error(source, error),
        }

        chain.truncate(guard);

        if !existed {
            copy_metadata(source, target);
        }

        let subtree_clean = self.outcome.error_count == errors_before && flow == Flow::Continue;
        self.finish_entry(source, target, 0, mode, record && !existed, !subtree_clean);

        // Cross-filesystem move: the source only goes once its copy is whole.
        if mode.kind == Kind::Move
            && subtree_clean
            && exists(source)
            && let Err(error) = std::fs::remove_dir(source)
        {
            self.error(source, error);
        }

        flow
    }

    fn transfer_file(
        &mut self,
        source: &Path,
        target: &Path,
        existed: bool,
        mode: Mode,
        record: bool,
    ) -> Flow {
        if mode.kind == Kind::Move && mode.same_filesystem {
            match std::fs::rename(source, target) {
                Ok(()) => {
                    let size = std::fs::symlink_metadata(target)
                        .map(|m| m.len())
                        .unwrap_or(0);
                    self.finish_entry(source, target, size, mode, record, false);
                    return Flow::Continue;
                }
                Err(error) if error.raw_os_error() != Some(18) => {
                    self.error(source, error);
                    return Flow::Continue;
                }
                Err(_) => {}
            }
        }

        let mut flags = gio::FileCopyFlags::ALL_METADATA;
        if existed {
            flags |= gio::FileCopyFlags::OVERWRITE;
        }
        if !mode.follow_symlinks {
            flags |= gio::FileCopyFlags::NOFOLLOW_SYMLINKS;
        }

        let from = gio::File::for_path(source);
        let to = gio::File::for_path(target);

        // A multi-gigabyte file is one entry but many seconds, so the bar has
        // to move from inside gio's callback rather than only between files.
        let base = self.done_bytes;
        let events = self.events.clone();
        let total = self.total;
        let done_items = self.done_items;
        let name = display_name(source);
        let mut last_report = self.last_report;
        let mut latest = 0i64;

        let result = {
            let mut progress = |current: i64, _total: i64| {
                latest = current;
                if last_report.elapsed() >= REPORT_INTERVAL {
                    last_report = Instant::now();
                    let _ = events.send(Event::Progress {
                        done_items,
                        done_bytes: base + u64::try_from(current).unwrap_or(0),
                        total,
                        current: name.clone(),
                    });
                }
            };
            from.copy(&to, flags, Some(&self.cancellable), Some(&mut progress))
        };
        self.last_report = last_report;

        match result {
            Ok(()) => {
                self.done_bytes = base + u64::try_from(latest).unwrap_or(0);
                self.finish_entry(source, target, 0, mode, record, false);

                if mode.kind == Kind::Move
                    && let Err(error) = std::fs::remove_file(source)
                {
                    self.error(source, error);
                }
                Flow::Continue
            }
            Err(error) if error.matches(gio::IOErrorEnum::Cancelled) => Flow::Cancel,
            Err(error) => {
                self.error(source, error.message());
                Flow::Continue
            }
        }
    }

    /// Count an entry, record its inverse, and push a progress event.
    fn finish_entry(
        &mut self,
        source: &Path,
        target: &Path,
        bytes: u64,
        mode: Mode,
        record: bool,
        failed: bool,
    ) {
        self.done_items += 1;
        self.done_bytes += bytes;
        self.outcome.finished_items += 1;

        if record && !failed {
            match mode.kind {
                Kind::Move => self.outcome.moved.push(Moved {
                    from: source.to_path_buf(),
                    to: target.to_path_buf(),
                }),
                Kind::Copy => {
                    if let Some(created) = created_entry(target) {
                        self.outcome.created.push(created);
                    }
                }
            }
        }

        self.report(source);
    }

    // ---- trash, delete and the undo jobs ---------------------------------

    fn trash(&mut self, paths: Vec<PathBuf>) {
        self.total = Survey {
            items: paths.len() as u64,
            bytes: 0,
        };

        for path in paths {
            if self.cancelled() {
                break;
            }
            match trash::trash(&path, &self.cancellable) {
                Ok(item) => {
                    self.outcome.trashed.push(item);
                    self.done_items += 1;
                    self.outcome.finished_items += 1;
                }
                Err(TrashError::NotSupported) => self.outcome.untrashable.push(path.clone()),
                Err(TrashError::Failed(message)) => self.error(&path, message),
            }
            self.report(&path);
        }
    }

    fn delete(&mut self, paths: Vec<PathBuf>) {
        let Some(survey) = self.survey(&paths, false) else {
            return;
        };
        self.total = survey;
        self.outcome.undoable = false;

        for path in paths {
            if self.cancelled() {
                break;
            }
            match remove_tree(&path) {
                Ok(()) => {
                    self.done_items += 1;
                    self.outcome.finished_items += 1;
                }
                Err(error) => self.error(&path, error),
            }
            self.report(&path);
        }
    }

    fn rename_one(&mut self, path: PathBuf, name: &str) {
        self.total = Survey { items: 1, bytes: 0 };
        match rename(&path, name) {
            Ok(target) => {
                self.outcome.moved.push(Moved {
                    from: path,
                    to: target,
                });
                self.done_items = 1;
                self.outcome.finished_items = 1;
            }
            Err(message) => self.error(&path, message),
        }
    }

    fn create_one(&mut self, path: PathBuf, directory: bool) {
        self.total = Survey { items: 1, bytes: 0 };

        if exists(&path) {
            self.error(&path, "already exists");
            return;
        }

        let created = if directory {
            std::fs::create_dir(&path)
        } else {
            std::fs::File::create_new(&path).map(drop)
        };

        match created {
            Ok(()) => match created_entry(&path) {
                Some(entry) => {
                    self.outcome.created.push(entry);
                    self.done_items = 1;
                    self.outcome.finished_items = 1;
                }
                None => self.error(&path, "vanished immediately after being created"),
            },
            Err(error) => self.error(&path, error),
        }
    }

    fn restore(&mut self, items: Vec<Trashed>) {
        self.total = Survey {
            items: items.len() as u64,
            bytes: 0,
        };
        for item in items {
            if self.cancelled() {
                break;
            }
            match trash::restore(&item) {
                Ok(()) => {
                    self.done_items += 1;
                    self.outcome.finished_items += 1;
                }
                Err(message) => self.error(&item.original, message),
            }
            self.report(&item.original);
        }
    }

    fn revert(&mut self, items: Vec<Moved>) {
        self.total = Survey {
            items: items.len() as u64,
            bytes: 0,
        };
        for item in items {
            if self.cancelled() {
                break;
            }
            match std::fs::rename(&item.to, &item.from) {
                Ok(()) => {
                    self.done_items += 1;
                    self.outcome.finished_items += 1;
                }
                // Undoing a cross-filesystem move needs the same copy-then-delete.
                Err(error) if error.raw_os_error() == Some(18) => {
                    let mode = Mode {
                        kind: Kind::Move,
                        same_filesystem: false,
                        follow_symlinks: false,
                    };
                    self.transfer_entry(&item.to, &item.from, false, mode, false, &mut Vec::new());
                }
                Err(error) => self.error(&item.to, error),
            }
            self.report(&item.to);
        }
    }

    fn discard(&mut self, items: Vec<Created>) {
        self.total = Survey {
            items: items.len() as u64,
            bytes: 0,
        };
        self.outcome.undoable = false;

        for item in items {
            if self.cancelled() {
                break;
            }
            match remove_tree(&item.path) {
                Ok(()) => {
                    self.done_items += 1;
                    self.outcome.finished_items += 1;
                }
                Err(error) => self.error(&item.path, error),
            }
            self.report(&item.path);
        }
    }
}

// ---- free functions ------------------------------------------------------

/// Rename with a two-step detour when only the case changes.
///
/// On a case-insensitive or case-preserving filesystem the direct rename either
/// fails or unlinks the file — §10.1 hazard 6 — so the detour is unconditional
/// rather than conditional on detecting the filesystem, which cannot be done
/// reliably. The staging name is hidden and lives in the same directory, so the
/// rename stays atomic.
pub fn rename(path: &Path, new_name: &str) -> Result<PathBuf, String> {
    let Some(parent) = path.parent() else {
        return Err("cannot rename the filesystem root".to_owned());
    };
    let old_name = display_name(path);
    let target = parent.join(new_name);

    if naming::is_case_only_rename(&old_name, new_name) {
        let staging = parent.join(naming::case_rename_staging(new_name, |candidate| {
            exists(&parent.join(candidate))
        }));
        std::fs::rename(path, &staging).map_err(|error| error.to_string())?;
        return std::fs::rename(&staging, &target)
            .map(|()| target)
            .map_err(|error| {
                // Put it back rather than leaving a staging name behind.
                let _ = std::fs::rename(&staging, path);
                error.to_string()
            });
    }

    if exists(&target) {
        return Err(format!("“{new_name}” already exists"));
    }

    std::fs::rename(path, &target)
        .map(|()| target)
        .map_err(|error| error.to_string())
}

/// How many entries undo's validation walk will look at before giving up.
///
/// Giving up reports "unknown", which validation reads as modified and so
/// refuses — the safe direction, and the one that keeps Ctrl+Z from stalling
/// the main loop on a tree with a hundred thousand files in it.
const MAX_VALIDATION_WALK: usize = 100_000;

/// The real filesystem, as [`crate::model::undo`] validation sees it.
pub struct Filesystem;

impl crate::model::undo::Probe for Filesystem {
    fn exists(&self, path: &Path) -> bool {
        exists(path)
    }

    fn stamp(&self, path: &Path) -> Option<Stamp> {
        std::fs::symlink_metadata(path)
            .ok()
            .map(|metadata| Stamp::of(&metadata))
    }

    fn newest_within(&self, path: &Path) -> Option<i64> {
        let mut budget = MAX_VALIDATION_WALK;
        newest_within(path, &mut budget)
    }
}

fn newest_within(path: &Path, budget: &mut usize) -> Option<i64> {
    if *budget == 0 {
        return Some(i64::MAX);
    }
    *budget -= 1;

    let metadata = std::fs::symlink_metadata(path).ok()?;
    let mut newest = Stamp::of(&metadata).modified;

    if metadata.is_dir()
        && !metadata.is_symlink()
        && let Ok(listing) = std::fs::read_dir(path)
    {
        for entry in listing.flatten() {
            if let Some(found) = newest_within(&entry.path(), budget) {
                newest = newest.max(found);
            }
        }
    }

    Some(newest)
}

/// Record what an entry looked like the moment Hive finished creating it.
pub fn created_entry(path: &Path) -> Option<Created> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    Some(Created {
        path: path.to_path_buf(),
        stamp: Stamp::of(&metadata),
        is_dir: metadata.is_dir(),
        completed_at: now_nanos(),
    })
}

/// `symlink_metadata`, so a broken link counts as present.
pub fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_symlink())
}

/// Resolve for comparison, falling back to the lexical form when it cannot be.
fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| crate::model::path::normalize(path))
}

/// Remove a file, a link, or a whole tree.
pub fn remove_tree(path: &Path) -> Result<(), std::io::Error> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };

    if metadata.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

/// Best-effort permissions and timestamps for a directory Hive created.
fn copy_metadata(source: &Path, target: &Path) {
    let Ok(metadata) = std::fs::metadata(source) else {
        return;
    };
    if let Err(error) = std::fs::set_permissions(target, metadata.permissions()) {
        tracing::debug!(%error, path = %target.display(), "could not copy permissions");
    }
}

/// Track directory identity so a symlink loop cannot be walked forever.
///
/// Returns the chain length to truncate back to, or `None` if this directory is
/// already on the current path.
fn enter(metadata: &std::fs::Metadata, chain: &mut Vec<(u64, u64)>) -> Option<usize> {
    use std::os::unix::fs::MetadataExt;

    let identity = (metadata.dev(), metadata.ino());
    if chain.len() >= MAX_DEPTH || chain.contains(&identity) {
        return None;
    }
    let guard = chain.len();
    chain.push(identity);
    Some(guard)
}

/// The name "Keep both" would use, byte-safe for names that are not UTF-8.
fn keep_both_name(dest_dir: &Path, name: &OsStr) -> OsString {
    if let Some(text) = name.to_str() {
        return OsString::from(naming::next_available(text, |candidate| {
            exists(&dest_dir.join(candidate))
        }));
    }

    for attempt in 1..=u32::MAX {
        let suffix = if attempt <= 1 {
            " (copy)".to_owned()
        } else {
            format!(" (copy {attempt})")
        };
        let mut bytes = name.as_bytes().to_vec();
        bytes.extend_from_slice(suffix.as_bytes());
        let candidate = OsString::from_vec(bytes);
        if !exists(&dest_dir.join(&candidate)) {
            return candidate;
        }
    }
    name.to_os_string()
}

/// Free bytes at `path`, or `None` when the filesystem does not report any.
fn free_space(path: &Path) -> Option<u64> {
    gio::File::for_path(path)
        .query_filesystem_info(gio::FILE_ATTRIBUTE_FILESYSTEM_FREE, gio::Cancellable::NONE)
        .ok()
        .map(|info| info.attribute_uint64(gio::FILE_ATTRIBUTE_FILESYSTEM_FREE))
}

/// Compare `id::filesystem`, which decides rename versus copy-then-delete.
fn same_filesystem(source: &Path, destination: &Path) -> bool {
    let source = source.parent().unwrap_or(source);
    match (filesystem_id(source), filesystem_id(destination)) {
        (Some(a), Some(b)) => a == b,
        // Unknown means assume different: a needless copy is slow, a rename
        // that turns out to cross a boundary fails halfway.
        _ => false,
    }
}

fn filesystem_id(path: &Path) -> Option<String> {
    gio::File::for_path(path)
        .query_info(
            gio::FILE_ATTRIBUTE_ID_FILESYSTEM,
            gio::FileQueryInfoFlags::NONE,
            gio::Cancellable::NONE,
        )
        .ok()
        .and_then(|info| info.attribute_string(gio::FILE_ATTRIBUTE_ID_FILESYSTEM))
        .map(|id| id.to_string())
}

fn now_nanos() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|since| i64::try_from(since.as_nanos()).ok())
        .unwrap_or(i64::MAX)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// Run a job to completion, answering every conflict with `answer`.
    fn run(job: Job, answer: impl Fn(&Conflict) -> Resolution) -> Outcome {
        let events = spawn(job, gio::Cancellable::new());
        let mut finished = None;

        while let Ok(event) = events.recv() {
            match event {
                Event::Conflict(conflict) => {
                    let resolution = answer(&conflict);
                    conflict.answer(resolution);
                }
                Event::Finished(outcome) => finished = Some(*outcome),
                _ => {}
            }
        }

        finished.expect("the worker always finishes with an outcome")
    }

    fn never_conflicts(conflict: &Conflict) -> Resolution {
        panic!("unexpected conflict on {}", conflict.target.display());
    }

    fn copy_job(sources: &[PathBuf], destination: &Path) -> Job {
        Job::Transfer {
            kind: Kind::Copy,
            sources: sources.to_vec(),
            destination: destination.to_path_buf(),
            follow_symlinks: false,
            blanket: None,
        }
    }

    fn move_job(sources: &[PathBuf], destination: &Path) -> Job {
        Job::Transfer {
            kind: Kind::Move,
            sources: sources.to_vec(),
            destination: destination.to_path_buf(),
            follow_symlinks: false,
            blanket: None,
        }
    }

    /// A source and destination directory side by side, on one filesystem.
    fn workspace() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let root = tempfile::tempdir().expect("tempdir");
        let src = root.path().join("src");
        let dst = root.path().join("dst");
        std::fs::create_dir_all(&src).expect("src");
        std::fs::create_dir_all(&dst).expect("dst");
        (root, src, dst)
    }

    #[test]
    fn copying_a_file_leaves_both_copies() {
        let (_root, src, dst) = workspace();
        let file = src.join("notes.txt");
        std::fs::write(&file, b"hello").unwrap();

        let outcome = run(copy_job(std::slice::from_ref(&file), &dst), never_conflicts);

        assert!(outcome.is_clean(), "{:?}", outcome.errors);
        assert_eq!(std::fs::read(dst.join("notes.txt")).unwrap(), b"hello");
        assert!(file.exists(), "the source stays put");
        assert_eq!(outcome.created.len(), 1);
        assert_eq!(outcome.created[0].path, dst.join("notes.txt"));
    }

    #[test]
    fn copying_a_tree_recreates_every_level() {
        let (_root, src, dst) = workspace();
        let tree = src.join("project");
        std::fs::create_dir_all(tree.join("a/b")).unwrap();
        std::fs::write(tree.join("a/b/deep.txt"), b"deep").unwrap();
        std::fs::write(tree.join("top.txt"), b"top").unwrap();
        std::fs::write(tree.join("empty.bin"), b"").unwrap();

        let outcome = run(copy_job(&[tree], &dst), never_conflicts);

        assert!(outcome.is_clean(), "{:?}", outcome.errors);
        assert_eq!(
            std::fs::read(dst.join("project/a/b/deep.txt")).unwrap(),
            b"deep"
        );
        assert_eq!(std::fs::read(dst.join("project/top.txt")).unwrap(), b"top");
        assert_eq!(
            std::fs::metadata(dst.join("project/empty.bin"))
                .unwrap()
                .len(),
            0
        );

        // One recorded entry: removing the folder removes everything below it.
        assert_eq!(outcome.created.len(), 1, "{:?}", outcome.created);
        assert_eq!(outcome.created[0].path, dst.join("project"));
        assert!(outcome.created[0].is_dir);
    }

    #[test]
    fn a_move_within_one_filesystem_is_a_rename() {
        let (_root, src, dst) = workspace();
        let tree = src.join("project");
        std::fs::create_dir_all(tree.join("a")).unwrap();
        std::fs::write(tree.join("a/x"), b"x").unwrap();

        let outcome = run(move_job(std::slice::from_ref(&tree), &dst), never_conflicts);

        assert_eq!(outcome.strategy, Some(Strategy::Rename));
        assert!(!tree.exists(), "the source is gone");
        assert_eq!(std::fs::read(dst.join("project/a/x")).unwrap(), b"x");
        assert_eq!(outcome.moved.len(), 1);
        assert_eq!(outcome.moved[0].from, tree);
    }

    #[test]
    fn a_conflict_answered_with_skip_leaves_the_target_alone() {
        let (_root, src, dst) = workspace();
        std::fs::write(src.join("a.txt"), b"new").unwrap();
        std::fs::write(dst.join("a.txt"), b"old").unwrap();

        let outcome = run(copy_job(&[src.join("a.txt")], &dst), |_| {
            Resolution::once(Action::Skip)
        });

        assert_eq!(std::fs::read(dst.join("a.txt")).unwrap(), b"old");
        assert_eq!(outcome.skipped, 1);
        assert!(outcome.created.is_empty(), "nothing was created");
    }

    #[test]
    fn a_conflict_answered_with_replace_overwrites_but_is_not_undoable() {
        let (_root, src, dst) = workspace();
        std::fs::write(src.join("a.txt"), b"new").unwrap();
        std::fs::write(dst.join("a.txt"), b"old").unwrap();

        let outcome = run(copy_job(&[src.join("a.txt")], &dst), |_| {
            Resolution::once(Action::Replace)
        });

        assert_eq!(std::fs::read(dst.join("a.txt")).unwrap(), b"new");
        // Undo deletes what Hive created; it cannot bring back what Replace
        // destroyed, so a replaced file is deliberately not recorded.
        assert!(outcome.created.is_empty(), "{:?}", outcome.created);
    }

    #[test]
    fn a_conflict_answered_with_keep_both_writes_a_second_file() {
        let (_root, src, dst) = workspace();
        std::fs::write(src.join("a.txt"), b"new").unwrap();
        std::fs::write(dst.join("a.txt"), b"old").unwrap();

        let seen = std::sync::Mutex::new(Vec::new());
        let outcome = run(copy_job(&[src.join("a.txt")], &dst), |conflict| {
            seen.lock()
                .map(|mut names| names.push(conflict.suggested_name.clone()))
                .ok();
            Resolution::once(Action::KeepBoth)
        });

        assert_eq!(
            seen.lock().unwrap().as_slice(),
            ["a (copy).txt"],
            "the dialog is told the name it would use"
        );
        assert_eq!(std::fs::read(dst.join("a.txt")).unwrap(), b"old");
        assert_eq!(std::fs::read(dst.join("a (copy).txt")).unwrap(), b"new");
        assert_eq!(outcome.created[0].path, dst.join("a (copy).txt"));
    }

    #[test]
    fn apply_to_all_stops_the_worker_asking_again() {
        let (_root, src, dst) = workspace();
        for name in ["a", "b", "c", "d"] {
            std::fs::write(src.join(name), b"new").unwrap();
            std::fs::write(dst.join(name), b"old").unwrap();
        }

        let asked = std::sync::atomic::AtomicUsize::new(0);
        let sources: Vec<PathBuf> = ["a", "b", "c", "d"].iter().map(|n| src.join(n)).collect();
        let outcome = run(copy_job(&sources, &dst), |_| {
            asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Resolution {
                action: Action::Skip,
                apply_to_all: true,
            }
        });

        assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(outcome.skipped, 4);
    }

    #[test]
    fn copying_a_folder_into_its_own_subtree_moves_no_bytes() {
        let (_root, src, _dst) = workspace();
        let data = src.join("data");
        std::fs::create_dir_all(data.join("inner")).unwrap();
        std::fs::write(data.join("big.bin"), vec![0u8; 1024]).unwrap();

        let outcome = run(
            copy_job(std::slice::from_ref(&data), &data.join("inner")),
            never_conflicts,
        );

        assert!(matches!(
            outcome.refusal,
            Some(Refusal::IntoOwnSubtree { .. })
        ));
        assert_eq!(outcome.finished_items, 0, "refused before starting");
        assert!(!data.join("inner/data").exists());
    }

    #[test]
    fn copying_a_folder_onto_itself_is_refused() {
        let (_root, src, _dst) = workspace();
        let data = src.join("data");
        std::fs::create_dir(&data).unwrap();

        let outcome = run(
            copy_job(std::slice::from_ref(&data), &data),
            never_conflicts,
        );
        assert!(matches!(outcome.refusal, Some(Refusal::OntoItself(_))));
    }

    #[test]
    fn a_source_that_vanished_is_refused_rather_than_half_run() {
        let (_root, src, dst) = workspace();
        let outcome = run(
            copy_job(&[src.join("never-existed")], &dst),
            never_conflicts,
        );
        assert!(matches!(outcome.refusal, Some(Refusal::Missing(_))));
    }

    #[test]
    fn a_symlink_is_copied_as_a_link_by_default() {
        let (_root, src, dst) = workspace();
        std::fs::write(src.join("target.txt"), b"contents").unwrap();
        std::os::unix::fs::symlink("target.txt", src.join("link")).unwrap();

        let outcome = run(copy_job(&[src.join("link")], &dst), never_conflicts);

        assert!(outcome.is_clean(), "{:?}", outcome.errors);
        let copied = std::fs::symlink_metadata(dst.join("link")).unwrap();
        assert!(copied.is_symlink(), "the link itself was copied");
        assert_eq!(
            std::fs::read_link(dst.join("link")).unwrap(),
            PathBuf::from("target.txt")
        );
    }

    #[test]
    fn a_symlink_is_resolved_when_following_is_switched_on() {
        let (_root, src, dst) = workspace();
        std::fs::write(src.join("target.txt"), b"contents").unwrap();
        std::os::unix::fs::symlink(src.join("target.txt"), src.join("link")).unwrap();

        let outcome = run(
            Job::Transfer {
                kind: Kind::Copy,
                sources: vec![src.join("link")],
                destination: dst.clone(),
                follow_symlinks: true,
                blanket: None,
            },
            never_conflicts,
        );

        assert!(outcome.is_clean(), "{:?}", outcome.errors);
        let copied = std::fs::symlink_metadata(dst.join("link")).unwrap();
        assert!(!copied.is_symlink(), "the target was copied, not the link");
        assert_eq!(std::fs::read(dst.join("link")).unwrap(), b"contents");
    }

    #[test]
    fn a_broken_symlink_copies_without_an_error() {
        let (_root, src, dst) = workspace();
        std::os::unix::fs::symlink("/nowhere-at-all", src.join("broken")).unwrap();

        let outcome = run(copy_job(&[src.join("broken")], &dst), never_conflicts);

        assert!(outcome.is_clean(), "{:?}", outcome.errors);
        assert!(
            std::fs::symlink_metadata(dst.join("broken"))
                .unwrap()
                .is_symlink()
        );
    }

    #[test]
    fn a_symlink_loop_does_not_run_forever() {
        let (_root, src, dst) = workspace();
        let loop_dir = src.join("loop");
        std::fs::create_dir(&loop_dir).unwrap();
        std::os::unix::fs::symlink(&loop_dir, loop_dir.join("self")).unwrap();

        let outcome = run(
            Job::Transfer {
                kind: Kind::Copy,
                sources: vec![loop_dir],
                destination: dst.clone(),
                follow_symlinks: true,
                blanket: None,
            },
            never_conflicts,
        );

        // It stops; whether it reports an error is secondary to terminating.
        assert!(dst.join("loop").is_dir());
        assert!(
            outcome.finished_items < 200,
            "walked {} entries",
            outcome.finished_items
        );
    }

    #[test]
    fn names_with_newlines_and_invalid_utf8_are_transferred() {
        let (_root, src, dst) = workspace();
        let odd = src.join("two\nlines.txt");
        let raw = src.join(OsStr::from_bytes(b"bad\xffname"));
        std::fs::write(&odd, b"a").unwrap();
        std::fs::write(&raw, b"b").unwrap();

        let outcome = run(copy_job(&[odd, raw], &dst), never_conflicts);

        assert!(outcome.is_clean(), "{:?}", outcome.errors);
        assert_eq!(std::fs::read(dst.join("two\nlines.txt")).unwrap(), b"a");
        assert_eq!(
            std::fs::read(dst.join(OsStr::from_bytes(b"bad\xffname"))).unwrap(),
            b"b"
        );
    }

    #[test]
    fn merging_two_folders_records_only_the_files_it_added() {
        let (_root, src, dst) = workspace();
        std::fs::create_dir(src.join("shared")).unwrap();
        std::fs::write(src.join("shared/new.txt"), b"new").unwrap();
        std::fs::create_dir(dst.join("shared")).unwrap();
        std::fs::write(dst.join("shared/kept.txt"), b"kept").unwrap();

        let outcome = run(copy_job(&[src.join("shared")], &dst), |_| {
            Resolution::once(Action::Replace)
        });

        assert_eq!(std::fs::read(dst.join("shared/kept.txt")).unwrap(), b"kept");
        assert_eq!(std::fs::read(dst.join("shared/new.txt")).unwrap(), b"new");

        // Undo must not delete the folder that was already there.
        let recorded: Vec<&Path> = outcome.created.iter().map(|c| c.path.as_path()).collect();
        assert_eq!(recorded, [dst.join("shared/new.txt").as_path()]);
    }

    #[test]
    fn deleting_removes_a_whole_tree_and_records_no_inverse() {
        let (_root, src, _dst) = workspace();
        let tree = src.join("junk");
        std::fs::create_dir_all(tree.join("a")).unwrap();
        std::fs::write(tree.join("a/x"), b"x").unwrap();

        let outcome = run(Job::Delete(vec![tree.clone()]), never_conflicts);

        assert!(!tree.exists());
        assert!(!outcome.undoable, "permanent delete has no inverse");
        assert!(outcome.created.is_empty() && outcome.moved.is_empty());
    }

    #[test]
    fn a_cancelled_job_stops_and_says_so() {
        let (_root, src, dst) = workspace();
        for index in 0..64 {
            std::fs::write(src.join(format!("f{index}")), vec![0u8; 4096]).unwrap();
        }
        let sources: Vec<PathBuf> = (0..64).map(|i| src.join(format!("f{i}"))).collect();

        let cancellable = gio::Cancellable::new();
        cancellable.cancel();
        let events = spawn(copy_job(&sources, &dst), cancellable);

        let mut outcome = None;
        while let Ok(event) = events.recv() {
            if let Event::Finished(finished) = event {
                outcome = Some(*finished);
            }
        }

        let outcome = outcome.expect("an outcome");
        assert!(outcome.cancelled);
        assert!(!outcome.is_clean());
    }

    #[test]
    fn reverting_a_move_puts_everything_back() {
        let (_root, src, dst) = workspace();
        std::fs::write(src.join("a.txt"), b"a").unwrap();

        let moved = run(move_job(&[src.join("a.txt")], &dst), never_conflicts);
        assert!(!src.join("a.txt").exists());

        let back = run(Job::Revert(moved.moved), never_conflicts);
        assert!(back.is_clean(), "{:?}", back.errors);
        assert_eq!(std::fs::read(src.join("a.txt")).unwrap(), b"a");
        assert!(!dst.join("a.txt").exists());
    }

    #[test]
    fn discarding_a_copy_removes_exactly_what_was_created() {
        let (_root, src, dst) = workspace();
        std::fs::create_dir(src.join("tree")).unwrap();
        std::fs::write(src.join("tree/a"), b"a").unwrap();
        std::fs::write(dst.join("untouched.txt"), b"keep").unwrap();

        let copied = run(copy_job(&[src.join("tree")], &dst), never_conflicts);
        assert!(dst.join("tree/a").exists());

        let undone = run(Job::Discard(copied.created), never_conflicts);
        assert!(undone.is_clean(), "{:?}", undone.errors);
        assert!(!dst.join("tree").exists());
        assert_eq!(std::fs::read(dst.join("untouched.txt")).unwrap(), b"keep");
        assert!(src.join("tree/a").exists(), "the original is untouched");
    }

    #[test]
    fn a_rename_job_reports_where_the_entry_ended_up() {
        let (_root, src, _dst) = workspace();
        let path = src.join("before.txt");
        std::fs::write(&path, b"x").unwrap();

        let outcome = run(
            Job::Rename {
                path: path.clone(),
                name: "after.txt".to_owned(),
            },
            never_conflicts,
        );

        assert!(outcome.is_clean(), "{:?}", outcome.errors);
        assert_eq!(outcome.moved.len(), 1);
        assert_eq!(outcome.moved[0].from, path);
        assert_eq!(outcome.moved[0].to, src.join("after.txt"));
        assert!(src.join("after.txt").exists());
    }

    #[test]
    fn a_create_job_records_what_it_made() {
        let (_root, src, _dst) = workspace();

        let folder = run(
            Job::Create {
                path: src.join("New Folder"),
                directory: true,
            },
            never_conflicts,
        );
        assert!(folder.is_clean(), "{:?}", folder.errors);
        assert!(src.join("New Folder").is_dir());
        assert_eq!(folder.created.len(), 1);
        assert!(folder.created[0].is_dir);

        let file = run(
            Job::Create {
                path: src.join("notes.txt"),
                directory: false,
            },
            never_conflicts,
        );
        assert!(file.is_clean(), "{:?}", file.errors);
        assert_eq!(std::fs::metadata(src.join("notes.txt")).unwrap().len(), 0);
        assert!(!file.created[0].is_dir);
    }

    #[test]
    fn creating_over_something_that_exists_is_refused_not_truncated() {
        let (_root, src, _dst) = workspace();
        std::fs::write(src.join("taken.txt"), b"important").unwrap();

        let outcome = run(
            Job::Create {
                path: src.join("taken.txt"),
                directory: false,
            },
            never_conflicts,
        );

        assert!(!outcome.is_clean());
        assert!(outcome.created.is_empty());
        assert_eq!(std::fs::read(src.join("taken.txt")).unwrap(), b"important");
    }

    #[test]
    fn duplicating_uses_a_standing_answer_rather_than_asking() {
        let (_root, src, _dst) = workspace();
        std::fs::write(src.join("a.txt"), b"x").unwrap();

        let outcome = run(
            Job::Transfer {
                kind: Kind::Copy,
                sources: vec![src.join("a.txt")],
                destination: src.clone(),
                follow_symlinks: false,
                blanket: Some(Action::KeepBoth),
            },
            never_conflicts,
        );

        assert!(outcome.is_clean(), "{:?}", outcome.errors);
        assert_eq!(std::fs::read(src.join("a (copy).txt")).unwrap(), b"x");
        assert!(src.join("a.txt").exists());
    }

    #[test]
    fn a_plain_rename_moves_the_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, b"x").unwrap();

        let renamed = rename(&path, "b.txt").unwrap();
        assert_eq!(renamed, dir.path().join("b.txt"));
        assert!(!path.exists());
        assert_eq!(std::fs::read(&renamed).unwrap(), b"x");
    }

    #[test]
    fn a_case_only_rename_survives_the_two_step_detour() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("foo");
        std::fs::write(&path, b"contents").unwrap();

        let renamed = rename(&path, "Foo").unwrap();
        assert_eq!(renamed, dir.path().join("Foo"));
        assert_eq!(std::fs::read(&renamed).unwrap(), b"contents");

        // Nothing staged was left behind.
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name())
            .filter(|name| name.to_string_lossy().starts_with(".hive-rename"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn renaming_onto_an_existing_name_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a"), b"a").unwrap();
        std::fs::write(dir.path().join("b"), b"b").unwrap();

        assert!(rename(&dir.path().join("a"), "b").is_err());
        assert_eq!(std::fs::read(dir.path().join("b")).unwrap(), b"b");
    }

    #[test]
    fn removing_a_tree_takes_files_links_and_directories() {
        let dir = tempfile::tempdir().unwrap();
        let tree = dir.path().join("tree");
        std::fs::create_dir_all(tree.join("nested")).unwrap();
        std::fs::write(tree.join("nested/a"), b"a").unwrap();
        std::os::unix::fs::symlink("/nowhere", tree.join("broken")).unwrap();

        remove_tree(&tree).unwrap();
        assert!(!tree.exists());
    }

    #[test]
    fn removing_something_already_gone_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        remove_tree(&dir.path().join("never-existed")).unwrap();
    }

    #[test]
    fn a_broken_symlink_counts_as_present() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("broken");
        std::os::unix::fs::symlink("/nowhere-at-all", &link).unwrap();

        assert!(exists(&link), "a broken link still occupies its name");
        assert!(
            !link.exists(),
            "and std's exists() disagrees, which is the point"
        );
    }

    #[test]
    fn keep_both_skips_names_that_are_taken() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"").unwrap();
        std::fs::write(dir.path().join("a (copy).txt"), b"").unwrap();

        let name = keep_both_name(dir.path(), OsStr::new("a.txt"));
        assert_eq!(name, OsString::from("a (copy 2).txt"));
    }

    #[test]
    fn keep_both_handles_names_that_are_not_utf8() {
        let dir = tempfile::tempdir().unwrap();
        let raw = OsString::from_vec(b"bad\xffname".to_vec());
        std::fs::write(dir.path().join(&raw), b"").unwrap();

        let name = keep_both_name(dir.path(), &raw);
        assert_eq!(name.as_bytes(), b"bad\xffname (copy)");
    }

    #[test]
    fn the_same_directory_is_on_the_same_filesystem_as_itself() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a");
        std::fs::write(&file, b"x").unwrap();
        assert!(same_filesystem(&file, dir.path()));
    }

    #[test]
    fn free_space_is_reported_for_a_real_directory() {
        let dir = tempfile::tempdir().unwrap();
        assert!(free_space(dir.path()).is_some());
    }

    #[test]
    fn a_symlink_loop_is_entered_only_once() {
        let dir = tempfile::tempdir().unwrap();
        let metadata = std::fs::metadata(dir.path()).unwrap();

        let mut chain = Vec::new();
        assert!(enter(&metadata, &mut chain).is_some());
        assert!(
            enter(&metadata, &mut chain).is_none(),
            "already on the path"
        );
    }

    #[test]
    fn depth_is_capped_even_without_a_repeat() {
        let dir = tempfile::tempdir().unwrap();
        let metadata = std::fs::metadata(dir.path()).unwrap();

        let mut chain: Vec<(u64, u64)> = (0..MAX_DEPTH as u64).map(|n| (n, n)).collect();
        assert!(enter(&metadata, &mut chain).is_none());
    }

    #[test]
    fn a_created_entry_records_size_and_kind() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a");
        std::fs::write(&file, b"hello").unwrap();

        let created = created_entry(&file).unwrap();
        assert_eq!(created.stamp.size, 5);
        assert!(!created.is_dir);
        assert!(created.completed_at > 0);

        let folder = created_entry(dir.path()).unwrap();
        assert!(folder.is_dir);

        assert!(created_entry(&dir.path().join("nope")).is_none());
    }

    #[test]
    fn every_job_has_a_progress_title() {
        let jobs = [
            Job::Transfer {
                kind: Kind::Copy,
                sources: Vec::new(),
                destination: PathBuf::new(),
                follow_symlinks: false,
                blanket: None,
            },
            Job::Transfer {
                kind: Kind::Move,
                sources: Vec::new(),
                destination: PathBuf::new(),
                follow_symlinks: false,
                blanket: Some(Action::KeepBoth),
            },
            Job::Trash(Vec::new()),
            Job::Delete(Vec::new()),
            Job::Restore(Vec::new()),
            Job::Revert(Vec::new()),
            Job::Discard(Vec::new()),
        ];
        for job in jobs {
            assert!(!job.title().is_empty());
        }
    }
}
