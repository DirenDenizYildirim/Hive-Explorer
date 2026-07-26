//! Image thumbnails: a capped worker pool with a two-level cache.
//!
//! The rules the spec fixes — 256 px, skip sources over 32 MiB, give up
//! entirely in directories over 2000 entries — are all configurable, and all
//! decided in [`crate::model::thumbnail`] so they can be tested without a
//! display. What lives here is the machinery around them.
//!
//! Three things this file exists to guarantee:
//!
//! * **Nothing decodes on the main thread.** A bind callback only ever looks in
//!   a hash map; a miss queues work and returns immediately, and the row keeps
//!   its symbolic icon until a picture actually exists.
//! * **The pool is capped.** At most [`MAX_IN_FLIGHT`] decodes run at once, no
//!   matter how fast the user scrolls, and the backlog is bounded too: rows
//!   that scrolled away are dropped rather than decoded for nobody.
//! * **A stale thumbnail is never shown.** The disk cache is keyed on the
//!   source path and validated against its mtime and size, so editing an image
//!   invalidates it — §10.1 hazard 5.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk::gdk;
use gtk::gdk_pixbuf::Pixbuf;
use gtk::prelude::*;

use crate::config::Thumbnails as Limits;
use crate::model::thumbnail::{self, Eligibility};

/// Concurrent decodes. Small on purpose: four cover the latency of reading from
/// a cold disk, and more would only compete with the directory enumeration that
/// is usually still running when the first rows appear.
const MAX_IN_FLIGHT: usize = 4;

/// Longest queue kept. Beyond this the oldest requests are dropped, on the
/// grounds that a user who has scrolled past 256 rows is not waiting for the
/// first of them; if they scroll back, the rows rebind and ask again.
const MAX_QUEUED: usize = 256;

/// Textures kept in memory. Each is at most 256 px, so 512 of them is a few
/// tens of megabytes at worst, and it makes going back to a folder instant.
const MAX_CACHED: usize = 512;

/// One decode request.
struct Job {
    source: PathBuf,
    cache: PathBuf,
    mtime: i64,
    size: u64,
    max_pixels: i32,
}

/// A decoded image on its way back to the main thread.
///
/// Deliberately plain data: `Pixbuf` and `Texture` are GObjects, and moving one
/// across a thread boundary is exactly the kind of thing that works until it
/// doesn't. Rows are repacked tight, so the stride is implied by the width.
struct Pixels {
    width: i32,
    height: i32,
    channels: i32,
    bytes: Vec<u8>,
}

/// What the worker decided about one source file.
struct Rendered {
    source: PathBuf,
    mtime: i64,
    size: u64,
    pixels: Option<Pixels>,
}

/// A remembered result. `texture: None` means "tried, and there is no picture"
/// — a corrupt file, or a format with no loader installed. Remembering the
/// failure is what stops a broken image being re-decoded on every scroll.
struct Cached {
    mtime: i64,
    size: u64,
    texture: Option<gdk::Texture>,
}

pub struct Thumbnailer {
    cache_dir: PathBuf,
    limits: Cell<Limits>,
    /// Set when the current directory is too large to thumbnail.
    suppressed: Cell<bool>,
    cached: RefCell<HashMap<PathBuf, Cached>>,
    /// Insertion order, for eviction. Cheaper than timestamping every entry.
    order: RefCell<VecDeque<PathBuf>>,
    queue: RefCell<VecDeque<Job>>,
    /// Paths queued or decoding, so a rebinding row cannot ask twice.
    claimed: RefCell<HashSet<PathBuf>>,
    in_flight: Cell<usize>,
    on_ready: RefCell<Option<Box<dyn Fn()>>>,
}

impl Thumbnailer {
    pub fn new(cache_dir: PathBuf, limits: Limits) -> Rc<Self> {
        Rc::new(Self {
            cache_dir,
            limits: Cell::new(limits),
            suppressed: Cell::new(false),
            cached: RefCell::new(HashMap::new()),
            order: RefCell::new(VecDeque::new()),
            queue: RefCell::new(VecDeque::new()),
            claimed: RefCell::new(HashSet::new()),
            in_flight: Cell::new(0),
            on_ready: RefCell::new(None),
        })
    }

    /// Called after one or more thumbnails have arrived, coalesced by the caller.
    pub fn connect_ready(&self, handler: impl Fn() + 'static) {
        *self.on_ready.borrow_mut() = Some(Box::new(handler));
    }

    pub fn set_limits(&self, limits: Limits) {
        self.limits.set(limits);
    }

    pub fn limits(&self) -> Limits {
        self.limits.get()
    }

    /// Auto-disable in a directory with more entries than the configured cap.
    ///
    /// Returns whether the answer changed, so the caller can repaint once
    /// rather than on every entry that arrives during enumeration.
    pub fn observe_directory(&self, entries: usize) -> bool {
        let over = entries > self.limits.get().max_directory_entries;
        if over == self.suppressed.get() {
            return false;
        }
        if over {
            tracing::debug!(entries, "directory too large for thumbnails");
            self.drop_queue();
        }
        self.suppressed.set(over);
        true
    }

    pub fn is_active(&self) -> bool {
        self.limits.get().enabled && !self.suppressed.get()
    }

    /// Forget what is queued, on navigating away.
    ///
    /// The memory cache survives: going back to a folder should not re-decode
    /// it. Only the backlog goes, so a huge directory left behind cannot delay
    /// the one now on screen.
    pub fn reset(&self) {
        self.drop_queue();
    }

    fn drop_queue(&self) {
        if let Ok(mut queue) = self.queue.try_borrow_mut() {
            let dropped: Vec<PathBuf> = queue.drain(..).map(|job| job.source).collect();
            if let Ok(mut claimed) = self.claimed.try_borrow_mut() {
                for path in dropped {
                    claimed.remove(&path);
                }
            }
        }
    }

    /// The thumbnail for one entry, if there already is one.
    ///
    /// This runs inside a bind callback, so it does no I/O and takes no lock it
    /// might not get: every borrow here is a `try_borrow`, and a contended one
    /// simply means the row keeps its icon for now.
    pub fn lookup(
        self: &Rc<Self>,
        path: &Path,
        mtime: i64,
        size: u64,
        content_type: &str,
    ) -> Option<gdk::Texture> {
        if !self.is_active() {
            return None;
        }

        let limits = self.limits.get();
        if thumbnail::eligibility(content_type, size, limits.max_file_bytes) != Eligibility::Yes {
            return None;
        }

        if let Ok(cached) = self.cached.try_borrow()
            && let Some(entry) = cached.get(path)
        {
            // A hit on a file that has since been edited is not a hit.
            if entry.mtime == mtime && entry.size == size {
                return entry.texture.clone();
            }
        }

        self.request(path, mtime, size, limits);
        None
    }

    fn request(self: &Rc<Self>, path: &Path, mtime: i64, size: u64, limits: Limits) {
        let (Ok(mut claimed), Ok(mut queue)) =
            (self.claimed.try_borrow_mut(), self.queue.try_borrow_mut())
        else {
            return;
        };

        if !claimed.insert(path.to_path_buf()) {
            return;
        }

        // Newest first: what the user is looking at now matters more than what
        // they scrolled past, and the oldest end is what gets dropped.
        queue.push_front(Job {
            source: path.to_path_buf(),
            cache: thumbnail::cache_path(&self.cache_dir, path),
            mtime,
            size,
            max_pixels: limits.max_pixels.clamp(1, i32::MAX as u32) as i32,
        });

        while queue.len() > MAX_QUEUED {
            if let Some(dropped) = queue.pop_back() {
                claimed.remove(&dropped.source);
            }
        }

        drop(queue);
        drop(claimed);
        self.pump();
    }

    /// Start as many decodes as the cap allows.
    fn pump(self: &Rc<Self>) {
        while self.in_flight.get() < MAX_IN_FLIGHT {
            let Some(job) = self.take_job() else {
                return;
            };

            self.in_flight.set(self.in_flight.get() + 1);
            let this = Rc::clone(self);
            glib::spawn_future_local(async move {
                let rendered = gio::spawn_blocking(move || render(job)).await;
                this.in_flight.set(this.in_flight.get().saturating_sub(1));

                match rendered {
                    Ok(rendered) => this.store(rendered),
                    Err(_) => tracing::warn!("a thumbnail worker ended without a result"),
                }

                this.pump();
            });
        }
    }

    fn take_job(&self) -> Option<Job> {
        if !self.is_active() {
            return None;
        }
        self.queue.try_borrow_mut().ok()?.pop_front()
    }

    /// Take a finished decode, build its texture, and tell the views.
    fn store(self: &Rc<Self>, rendered: Rendered) {
        if let Ok(mut claimed) = self.claimed.try_borrow_mut() {
            claimed.remove(&rendered.source);
        }

        let texture = rendered.pixels.map(|pixels| {
            let format = if pixels.channels == 4 {
                gdk::MemoryFormat::R8g8b8a8
            } else {
                gdk::MemoryFormat::R8g8b8
            };
            let stride = (pixels.width * pixels.channels).max(1) as usize;
            gdk::MemoryTexture::new(
                pixels.width,
                pixels.height,
                format,
                &glib::Bytes::from_owned(pixels.bytes),
                stride,
            )
            .upcast::<gdk::Texture>()
        });

        let Ok(mut cached) = self.cached.try_borrow_mut() else {
            return;
        };
        let replaced = cached
            .insert(
                rendered.source.clone(),
                Cached {
                    mtime: rendered.mtime,
                    size: rendered.size,
                    texture,
                },
            )
            .is_some();

        if let Ok(mut order) = self.order.try_borrow_mut() {
            if !replaced {
                order.push_back(rendered.source);
            }
            while order.len() > MAX_CACHED {
                if let Some(oldest) = order.pop_front() {
                    cached.remove(&oldest);
                }
            }
        }
        drop(cached);

        if let Ok(handler) = self.on_ready.try_borrow()
            && let Some(handler) = handler.as_ref()
        {
            handler();
        }
    }
}

/// Decode one image, off the main thread.
///
/// Returns `Rendered` even on failure — with `pixels: None` — so the caller can
/// record "there is no thumbnail for this" and stop asking.
fn render(job: Job) -> Rendered {
    let failed = |source: PathBuf| Rendered {
        source,
        mtime: job.mtime,
        size: job.size,
        pixels: None,
    };

    if let Some(pixbuf) = load_cached(&job) {
        return Rendered {
            source: job.source,
            mtime: job.mtime,
            size: job.size,
            pixels: repack(&pixbuf),
        };
    }

    // The header alone gives the dimensions, which is what decides the target
    // size — decoding first and scaling after would defeat the point.
    let (width, height) = match Pixbuf::file_info(&job.source) {
        Some((_, width, height)) => (width, height),
        None => {
            tracing::debug!(path = %job.source.display(), "no pixbuf loader for this file");
            return failed(job.source);
        }
    };

    let (target_width, target_height) = thumbnail::scaled(width, height, job.max_pixels);
    let pixbuf = match Pixbuf::from_file_at_scale(&job.source, target_width, target_height, true) {
        Ok(pixbuf) => pixbuf.apply_embedded_orientation().unwrap_or(pixbuf),
        Err(error) => {
            tracing::debug!(path = %job.source.display(), %error, "could not decode image");
            return failed(job.source);
        }
    };

    write_cache(&job, &pixbuf);

    Rendered {
        pixels: repack(&pixbuf),
        source: job.source,
        mtime: job.mtime,
        size: job.size,
    }
}

/// Load the cached thumbnail, if there is one and it is still current.
fn load_cached(job: &Job) -> Option<Pixbuf> {
    let pixbuf = Pixbuf::from_file(&job.cache).ok()?;
    let fresh = thumbnail::is_fresh(
        pixbuf.option(thumbnail::KEY_MTIME).as_deref(),
        pixbuf.option(thumbnail::KEY_SIZE).as_deref(),
        job.mtime,
        job.size,
    );
    fresh.then_some(pixbuf)
}

/// Write the thumbnail to the cache, stamped with what it is a picture of.
///
/// Best-effort throughout: a cache that cannot be written costs a decode next
/// time and nothing else, so every failure here is a debug line rather than
/// something the user is told about. Written to a temporary name and renamed,
/// so a crash mid-write cannot leave a half-PNG that reads as corrupt.
fn write_cache(job: &Job, pixbuf: &Pixbuf) {
    let Some(parent) = job.cache.parent() else {
        return;
    };

    // gio rather than `std::fs`: this is the cache directory, and the carve-out
    // in the build spec covers the config stores only.
    let directory = gio::File::for_path(parent);
    if let Err(error) = directory.make_directory_with_parents(gio::Cancellable::NONE)
        && !error.matches(gio::IOErrorEnum::Exists)
    {
        tracing::debug!(path = %parent.display(), %error, "no thumbnail cache directory");
        return;
    }

    let mut temporary = job.cache.clone().into_os_string();
    temporary.push(".part");
    let temporary = PathBuf::from(temporary);

    let uri = gio::File::for_path(&job.source).uri();
    let mtime = job.mtime.to_string();
    let size = job.size.to_string();
    let stamped = pixbuf.savev(
        &temporary,
        "png",
        &[
            (thumbnail::KEY_URI, uri.as_str()),
            (thumbnail::KEY_MTIME, mtime.as_str()),
            (thumbnail::KEY_SIZE, size.as_str()),
        ],
    );

    if let Err(error) = stamped {
        tracing::debug!(path = %temporary.display(), %error, "could not write thumbnail");
        return;
    }

    if let Err(error) = gio::File::for_path(&temporary).move_(
        &gio::File::for_path(&job.cache),
        gio::FileCopyFlags::OVERWRITE,
        gio::Cancellable::NONE,
        None,
    ) {
        tracing::debug!(path = %job.cache.display(), %error, "could not place thumbnail");
        let _ = gio::File::for_path(&temporary).delete(gio::Cancellable::NONE);
    }
}

/// Copy a pixbuf's rows into a tightly packed buffer.
///
/// A `GdkPixbuf` pads each row to a multiple of four bytes; `GdkMemoryTexture`
/// reads `stride × height` and would run past the end of the last row if it
/// were handed the padded stride with an unpadded buffer. Repacking is one copy
/// we are making anyway to get the bytes off the worker thread.
fn repack(pixbuf: &Pixbuf) -> Option<Pixels> {
    let width = pixbuf.width();
    let height = pixbuf.height();
    let channels = pixbuf.n_channels();
    if width <= 0 || height <= 0 || !(channels == 3 || channels == 4) {
        return None;
    }

    let rowstride = pixbuf.rowstride() as usize;
    let row_bytes = (width as usize).checked_mul(channels as usize)?;
    let source = pixbuf.read_pixel_bytes();

    let mut bytes = Vec::with_capacity(row_bytes.checked_mul(height as usize)?);
    for row in 0..height as usize {
        let start = row.checked_mul(rowstride)?;
        let end = start.checked_add(row_bytes)?;
        bytes.extend_from_slice(source.get(start..end)?);
    }

    Some(Pixels {
        width,
        height,
        channels,
        bytes,
    })
}
