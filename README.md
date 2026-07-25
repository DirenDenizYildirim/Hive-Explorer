# Hive

Minimal pastel explorer — a file manager for Hyprland, written in Rust with GTK 4 and libadwaita.

Priority order: **stability > visual polish > feature count.** A small set of operations that never fail is
worth more than a wide feature set that occasionally corrupts or hangs.

> **Status: milestones (a) through (e) of 6.** The window, sidebar, theming system, config layer,
> non-blocking progressive listing, navigation, full keyboard support, the CLI, and the whole file-operation
> layer — copy, move, rename, trash, delete, undo, with conflict handling and pre-flight checks — are in
> place, as is the theming UI — flavor switcher, live swapping, accent setting and follow-system — and now
> folder colors and sidebar pinning. Thumbnails and the properties dialog are not built yet.
> See [Roadmap](#roadmap).

---

## Application ID

```
dev.diren.Hive
```

This is the GTK application ID and therefore the Wayland `app_id`. It is stable — Hyprland `windowrulev2`
rules can be written against it and will not break. See [Hyprland window rules](#hyprland-window-rules).

---

## Requirements

| | |
|---|---|
| **Runtime** | `gtk4` ≥ 4.20, `libadwaita` ≥ 1.9 |
| **Build** | `rust` ≥ 1.89, `cargo` |
| **Optional** | `gvfs` — required for the Devices sidebar section |
| **Optional** | `udisks2` — automounting removable media |

Hive assumes no desktop environment. It does not require GNOME session services, XSETTINGS, or a settings
daemon. Wayland only — there are no X11 code paths.

### Toolchain note

Hive pins `gtk4` 0.10.3 / `libadwaita` 0.8.1 / `glib` + `gio` 0.21.5 rather than the newest gtk4-rs release.
The current generation (`gtk4` 0.11, `glib` 0.22) declares an MSRV of **1.92**, which does not build on
Rust 1.89. The pinned versions expose GTK feature level `v4_20` and libadwaita `v1_9`, so essentially nothing
is given up against a GTK 4.22 / libadwaita 1.9.2 system.

---

## Build and run

```bash
cargo build --release      # build
cargo test                 # unit tests, no display required
cargo run                  # run from the source tree
```

With `just`:

```bash
just run          # run against Wayland
just debug        # run with debug logging
just check        # fmt + clippy + test + build
just install      # makepkg -si
```

`just` is a convenience only. Every target is a thin wrapper over plain `cargo`, and nothing in the build or
the verification path depends on it.

### Arch package

```bash
makepkg -si
```

Package name is `hive-explorer`; the binary it installs is `hive`. (`hive` is taken on the AUR by
`apache-hive`.)

---

## Command line

| Form | Behavior |
|---|---|
| `hive` | Open the home directory. |
| `hive PATH` | Open `PATH`. If `PATH` is a file, open its parent directory and preselect the file. |
| `hive --select PATH` | Always reveal: open `PATH`'s parent and preselect `PATH`, even for a directory. |
| `hive --verbose` | Raise the log level to debug. |
| `hive --help` / `--version` | Print help or version. |

`--select` is the stable form for "reveal in file manager" from other applications. Its behavior will not
change.

**Relative paths resolve against the invoking shell's working directory**, not the running instance's. `hive .`
is resolved to an absolute path in the process you typed it in, before anything is handed to an
already-running Hive. Getting this wrong would make `hive .` silently open the wrong folder.

A second `hive` invocation activates the existing window and navigates it rather than starting a second
process (`gio::ApplicationFlags::HANDLES_OPEN`).

---

## Keybindings

| Key | Action |
|---|---|
| `Double-click` / `Enter` | Enter a directory, or open a file in its handler application |
| `Alt+Left` / `Alt+Right` | Back / forward through history |
| Mouse buttons 8 / 9 | Back / forward |
| `Alt+Up` / `Backspace` | Parent folder (selects the folder you came from) |
| `Alt+Home` | Home |
| `Ctrl+L` | Path entry — `Tab` completes, `Enter` goes, `Escape` cancels |
| `Ctrl+F` | Filter this folder as you type; `Escape` clears and closes |
| `Ctrl+A` | Select all |
| `Ctrl+H` | Toggle hidden files |
| `Ctrl+T` | Toggle list / grid view |
| `Ctrl+,` | Appearance settings |
| `F10` | Main menu (including the flavor switcher) |
| `F9` | Toggle sidebar |
| Arrows, `Home`/`End`, `Page Up`/`Down` | Move the selection |
| Click-drag, `Ctrl+click`, `Shift+click` | Multi-select |
| Type any letters | Type-ahead jump to the next matching name |
| `Right-click`, `Menu`, `Shift+F10` | Context menu |

File operations:

| Key | Action |
|---|---|
| `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | Copy / cut / paste |
| `Ctrl+D` | Duplicate — always keeps both, never asks |
| `F2` | Rename |
| `Delete` | Move to Trash |
| `Shift+Delete` | Delete permanently, behind a confirmation |
| `Ctrl+Shift+N` | New folder |
| `Ctrl+Z` | Undo |

Type-ahead is why `Ctrl+F` is an explicit shortcut rather than start-typing-to-search: bare typing jumps to a
name, and only `Ctrl+F` opens the filter.

The file-operation keys are shortcuts on the window in GTK's *bubble* phase, not application accelerators.
That ordering matters: an application accelerator is handled above the focused widget, so `Delete` inside the
rename dialog would trash your selection instead of deleting a character, and `Ctrl+A` in the path entry would
select every file rather than the text. In the bubble phase the focused widget gets first refusal and only
what it does not want reaches the file actions — while they still work with focus on the sidebar or a header
button, which a shortcut scoped to the list would not.

New File has no key. It is on the main menu and the context menu; there is no conventional binding for it and
inventing one costs a key that something else may want.

### Vim keys

Off by default. Set `vim_keys = true` under `[behavior]`:

| Key | Action |
|---|---|
| `j` / `k` | Down / up |
| `h` | Parent folder |
| `l` | Enter / open |
| `gg` / `G` | First / last entry |

---

## Configuration

`$XDG_CONFIG_HOME/hive/config.toml`, written atomically and schema-versioned.

If the file is malformed it is backed up to `config.toml.bak`, defaults are regenerated, and a banner
explains what happened. **Hive never refuses to launch because of its own config.** A typo in a single
enum-valued key (say `accent = "chartreuse"`) falls back to that key's default and leaves every other setting
alone, rather than discarding the file.

```toml
version = 1

[appearance]
flavor = "mocha"              # latte | frappe | macchiato | mocha, or a user theme id
accent = "mauve"              # any of the 14 accent slots
follow_system = false         # best-effort light/dark following
light_flavor = "latte"
dark_flavor = "mocha"
client_side_rounding = false  # let Hyprland round the window
client_side_shadow = false    # let Hyprland shadow the window

[view]
mode = "list"                 # list | grid
show_hidden = false
sort_key = "name"             # name | size | modified | type
sort_order = "ascending"      # ascending | descending
folders_first = true

[thumbnails]
enabled = true
max_pixels = 256
max_file_bytes = 33554432     # 32 MiB
max_directory_entries = 2000  # disable thumbnails above this many entries

[behavior]
follow_symlinks_on_copy = false
warn_clipboard_on_quit = true
vim_keys = false

[sidebar]
pinned = []
```

`pinned` under `[sidebar]` is the pinned-folder list, in the order the sidebar shows them. It is normalized on
read: duplicates and relative paths are dropped, so hand-editing it cannot produce two rows for one folder.

Other locations:

| | |
|---|---|
| Folder colors | `$XDG_CONFIG_HOME/hive/folder-colors.toml` |
| Thumbnail cache | `$XDG_CACHE_HOME/hive/thumbnails/` |
| Logs | `$XDG_STATE_HOME/hive/logs/` (daily rotation, 5 files kept) |
| User themes | `$XDG_CONFIG_HOME/hive/themes/` |

---

## Theming

Four Catppuccin flavors ship built in: **Latte**, **Frappé**, **Macchiato**, **Mocha**. First launch defaults
to Mocha with a mauve accent and does not wait on the appearance portal.

The four flavors are Rust constants. The stylesheet is generated at runtime and loaded through a single
`gtk::CssProvider` that is swapped on change — there are no hand-written per-flavor stylesheets to drift out
of sync, and switching rebuilds no widgets.

### Switching

- **Menu → Flavor** switches immediately. `F10` opens the menu without a mouse.
- **Menu → Appearance…**, or `Ctrl+,`, opens the full dialog: flavor, accent, follow-system, window chrome,
  and your themes folder.

Everything applies the moment you pick it. There is no OK button, because applying is the only way to see
whether a theme is the one you wanted. Swapping the provider's content restyles the existing widgets in
place — nothing is rebuilt and the directory is not re-read, so a switch does not lose your scroll position
or selection.

The **accent** is separate from the flavor and drives selection, focus rings, and the active sidebar row. The
picker shows the fourteen slots in the colours of the flavor you are currently in, so you choose against what
you will actually see.

### Following the system

Off by default. When on, Hive picks your chosen light flavor or dark flavor to match the desktop.

The preference is read from the **freedesktop appearance portal**, not from libadwaita's `StyleManager`.
`StyleManager::is_dark` reports the *effective* appearance, which Hive itself forces to match the active
palette — asking it what the system wants only echoes back what Hive just told it.

Three outcomes, and the last one is the common case on a bare Hyprland install:

| The portal says | Hive uses |
|---|---|
| prefer dark | your dark flavor |
| prefer light | your light flavor |
| no preference, or no portal at all | **your configured flavor**, unchanged |

"No preference" is a real answer and is not the same as light — guessing light would flip the theme on every
machine without a desktop portal backend. An explicit flavor always wins and always works: picking one from
the menu turns follow-system off rather than being silently overridden, and startup never waits on the portal.

### A note on system GTK themes

Hive's stylesheet loads one step above `GTK_STYLE_PROVIDER_PRIORITY_USER`, so it outranks a theme symlinked
into `~/.config/gtk-4.0/gtk.css`. That is necessary but not sufficient.

Hand-written GTK4 themes — the large generated ones, Catppuccin's own among them — paint widgets with
**literal colours** rather than reading libadwaita's `--view-bg-color` and friends. Such a theme never asks
Hive's question, so no provider priority can answer it: overriding a custom property does nothing when
nothing reads it. Hive therefore declares every surface it cares about **concretely**, on the selectors such
a theme targets, as well as setting the custom properties for libadwaita's own widgets.

Without that, a light flavor half-applies: sidebar and header turn light while the file pane keeps the system
theme's dark background and the sidebar labels keep its light text. If you write a theme for another toolkit
and see something similar, this is why.

### Adding your own theme

A theme is any file that fills the same slots the built-ins do. Drop a `.toml` into
`$XDG_CONFIG_HOME/hive/themes/` and it appears in the flavor switcher — no recompile, no code change. Hive
ships that directory empty.

```toml
id = "my-theme"       # stable identifier, stored in config.toml
name = "My Theme"     # shown in the switcher
dark = true           # drives the libadwaita color scheme and elevation direction

[accents]
rosewater = "#f5e0dc"
flamingo  = "#f2cdcd"
pink      = "#f5c2e7"
mauve     = "#cba6f7"
red       = "#f38ba8"
maroon    = "#eba0ac"
peach     = "#fab387"
yellow    = "#f9e2af"
green     = "#a6e3a1"
teal      = "#94e2d5"
sky       = "#89dceb"
sapphire  = "#74c7ec"
blue      = "#89b4fa"
lavender  = "#b4befe"

[neutrals]
crust     = "#11111b"   # deepest — behind everything
mantle    = "#181825"   # header bar, sidebar
base      = "#1e1e2e"   # content surface
surface0  = "#313244"
surface1  = "#45475a"
surface2  = "#585b70"
overlay0  = "#6c7086"
overlay1  = "#7f849c"
overlay2  = "#9399b2"
subtext0  = "#a6adc8"
subtext1  = "#bac2de"
text      = "#cdd6f4"   # foreground
```

Two rules worth knowing:

- **The fourteen accent names are a fixed contract, not Catppuccin data.** Folder colors are stored by slot
  name, so a folder tagged `mauve` renders through whichever theme is active. A theme with nothing to do with
  Catppuccin just maps its own colors onto these slots, and existing folder colors keep working. If accent
  names were free-form, every stored folder color would break the moment you switched to a theme that did not
  define that exact name.
- **A user theme whose `id` matches a built-in replaces it in place**, keeping its position in the switcher.
  That lets you adjust one flavor without ending up with a near-duplicate entry.

A malformed theme file is skipped with a banner; it never blocks startup.

**Reload** in the Appearance dialog re-reads the folder without restarting, so a theme you are editing can be
checked by saving and clicking once. **Open** takes you to the folder in Hive itself, creating it if needed.

---

## Folder colors

Right-click a folder → **Color** → a grid of the fourteen accent slots, plus **None**. The swatches are drawn
in the colors of the flavor that is loaded, so what you pick is what you get. The whole selection is colored
at once, and the change applies immediately in both the file pane and the sidebar.

The color tints **the folder's symbolic icon only** — never the row background, never the label.

- **Stored by slot name, never as hex**, in `$XDG_CONFIG_HOME/hive/folder-colors.toml`, keyed by absolute
  path. A folder tagged `mauve` is Mocha mauve in Mocha and Latte mauve in Latte, and it still resolves under
  a user theme that maps its own colors onto the same fourteen slots.
- **Nothing is ever written into your folders.** No `.directory`, no dotfiles, no xattrs. If you delete
  `folder-colors.toml`, every folder goes back to plain and nothing else is affected.
- The context-menu entry appears only when the selection contains a folder, rather than sitting there greyed
  out.

Reachable without a mouse: `Menu` or `Shift+F10` opens the context menu, arrows walk to **Color**, `Return`
opens the grid, arrows move between swatches, `Return` applies.

---

## Pinned folders

The **Pinned** section sits at the top of the sidebar and persists in `config.toml`.

- **Pin:** right-click a folder → **Pin to Sidebar**, or drag it from the file pane onto the sidebar.
- **Reorder:** drag a pinned row up or down. Dropping on the upper half of a row lands above it, the lower
  half below it, so the bottom of the list is reachable.
- **Unpin:** right-click a pinned row → **Unpin**, or right-click the folder in the pane →
  **Unpin from Sidebar**.

Pinned rows carry the folder's color, which is what makes a long list scannable at a glance.

**Dragging is deliberately Hive-only.** The payload is a private boxed type, not a `GFile` or a
`text/uri-list`, so no other application can accept the drag. Dragging files *out* to other applications is
deferred to v1.1, and half of it working by accident would be worse than none of it. The cost is that
starting a drag on a folder row no longer starts a rubber-band selection there — begin the rubber band on
empty space instead, exactly as in every other file manager.

---

## Hyprland window rules

```conf
# Float Hive and give it a fixed size — good for a quick-open workflow.
windowrulev2 = float, class:^(dev\.diren\.Hive)$
windowrulev2 = size 1100 700, class:^(dev\.diren\.Hive)$
windowrulev2 = center, class:^(dev\.diren\.Hive)$

# Or keep it tiled but let dialogs float.
windowrulev2 = float, class:^(dev\.diren\.Hive)$, title:^(Properties|Rename|Delete).*$

# Hive ships with client-side rounding and shadow off, so the compositor's own
# rounding is the only one drawn. Nothing to disable here.
windowrulev2 = opacity 0.98 0.94, class:^(dev\.diren\.Hive)$

# Bind a quick-open key.
bind = SUPER, E, exec, hive
```

Confirm the class at runtime with:

```bash
hyprctl clients | grep -i dev.diren.Hive
```

---

## File operations

Everything that touches more than one file runs on a worker thread. The UI never blocks on the filesystem,
including on a mount that has stopped answering.

**Before a copy or move starts**, off the main thread and cancellable:

1. Copying or moving a folder **into its own subtree, or onto itself, is refused** — compared on canonicalized
   paths, before a single byte moves. This is the classic recursive data-eater.
2. The sources are walked for a total byte and item count. The walk shows an indeterminate indicator, because
   until it finishes there is no honest denominator to show.
3. Free space at the target is read via gio's `filesystem::free` and compared. If it does not fit, Hive
   **refuses before starting** and shows required versus available, rather than dying at 90% with half a tree
   written. A filesystem that reports no figure is not treated as full.
4. Source and target `id::filesystem` are compared. A same-filesystem move is an instant rename; anything else
   copies. The progress dialog says which, because that decides whether you wait or walk away.

A cross-filesystem move is copy-then-delete, and each source is removed only after **its own** copy has fully
succeeded. Cancelling therefore leaves every file in exactly one place — never zero, never two.

**Conflicts are never resolved silently.** A name that is already taken raises a dialog offering
**Replace**, **Keep Both**, **Skip** and **Cancel**, showing the size and date of both sides, with a
"do this for everything else" checkbox. Keep Both shows the exact name it would use. Two directories with the
same name **merge**; anything else replaces.

The progress dialog only appears once an operation has been running for 400 ms, so the common case of copying
a handful of files never puts a dialog on screen at all.

### Undo (`Ctrl+Z`)

A bounded in-session stack of **20 entries**, not persisted across restarts. No redo in v1.

| Operation | Inverse |
|---|---|
| Trash | Restore from the recorded trash location — never guessed by name |
| Rename | Rename back |
| Move | Move back to the original parent |
| Copy / duplicate | Delete the files that were created |
| New folder / new file | Delete it |

**Permanent delete is never on the stack.** It has no inverse and cannot be represented, let alone pushed.

**Undo never destroys data.** Before applying an inverse Hive re-validates: does the source still exist, is the
target path still free, and has anything changed since? If a copied file has been *edited* since the copy,
undoing would discard those edits, so the entry is refused with a toast explaining why and dropped. For a
copied *folder* the check looks below the top level — a folder's own mtime says nothing about a file edited
three levels down, and deleting the folder would take that edit with it.

Two consequences worth knowing:

- **A replaced file is not recorded.** Undoing a copy deletes what Hive created, which cannot bring back what
  Replace destroyed — so deleting it too would only make things worse. Replaced targets are deliberately left
  out. The conflict dialog says "Replacing it cannot be undone" for this reason. A *move* is different:
  putting the source back destroys nothing, so it is recorded.
- **A partially completed operation records only what actually completed**, and an operation that created more
  than 10,000 recordable entries is reported as not undoable rather than storing an inverse that no longer
  fits a bounded stack.

Only one operation runs at a time. Queueing is out of scope for v1.

---

## Decided behavior for known hazards

Every item here is a trap that file managers routinely fall into. Each has a decided behavior rather than an
accident.

| # | Hazard | Decision | Status |
|---|---|---|---|
| 1 | **Recursive folder size is an unbounded tree walk** | Never computed automatically. The Properties dialog shows size behind an explicit **Calculate** button, run off-thread, cancellable, updating incrementally. | milestone (f) |
| 2 | **Wayland clipboard dies with the process** | Copying a file and then closing Hive loses the clipboard, because Wayland clipboard content is owned by the client. Closing the window while Hive still owns a file clipboard asks first — **Don't Quit** / **Quit Anyway** — and names what would be lost. It asks *only* while Hive actually owns it: the moment another application takes the clipboard the warning stops. Set `warn_clipboard_on_quit = false` to turn it off, or run `wl-clip-persist`, which makes the whole problem moot. | done |
| 3 | **Clipboard format interop** | Hive offers and accepts both `text/uri-list` and `x-special/gnome-copied-files`; the latter carries the cut-vs-copy distinction. A plain-text form rides along so pasting into a terminal gives the paths. A `text/uri-list` arriving from another application says nothing about intent and is read as a **copy** — treating an ambiguous paste as a move would delete that application's files. | done |
| 4 | **Trash fails on removable drives** | FAT/exFAT cannot host the per-mount `.Trash-$uid` the freedesktop spec wants, and neither can `tmpfs`. `Delete` detects the failure and offers permanent delete through a clear dialog. It never silently does nothing, and never silently permanently deletes instead. | done |
| 5 | **Thumbnail cache staleness** | The cache is keyed on **(path, mtime, size)**, not path alone, so an edited image does not show its old thumbnail forever. | milestone (f) |
| 6 | **Case-only rename** (`foo` → `Foo`) | Detected and performed as a two-step rename through a hidden staging name in the same directory. Unconditional rather than conditional on detecting the filesystem, because that cannot be done reliably and the direct rename may *destroy* the file rather than merely fail. If the second step fails the staging name is renamed back. | done |
| 7 | **Symlink copy semantics** | **Default: copy the link itself, not its target** (`NOFOLLOW_SYMLINKS`), so a broken link copies as a broken link rather than failing. Set `follow_symlinks_on_copy = true` under `[behavior]` to copy targets instead; there is no per-operation modifier, because a destructive default that changes with a held key is exactly the implicit behavior the spec rules out. | done |
| 8 | **Self-referential operations** | Renaming or deleting the directory currently being viewed walks up to the **nearest surviving ancestor** rather than sitting on a dead path or jumping all the way Home. Cut-then-paste into the same directory is a no-op, not a duplicate and not a deletion. Copying or moving a folder into its own subtree, or onto itself, is refused before a single byte moves. | done |

Three more that folder colors and pinning brought with them:

- **A colored folder that is renamed or moved loses its color.** Colors are keyed by absolute path, and Hive
  does not follow a rename. The stale entry is ignored on read and pruned lazily — when the directory it
  named is next listed, those few paths are checked for existence off the main thread and the dead ones
  dropped. There is no startup scan and nothing walks a tree. The visible consequence: create a new folder
  with the same name in the same place before that directory is revisited and it inherits the old color.
  *If you would rather colors followed a rename, say so — it is a small change, but it is not what the build
  spec asked for.*
- **A folder whose name is not valid UTF-8 cannot be colored.** A TOML key is text, and writing a lossy
  approximation of the name would eventually color some *other* folder. Hive refuses and says so in a toast
  rather than storing a color that silently applies to the wrong thing.
- **A pinned folder that no longer exists is dimmed, not removed.** The pinned list is yours; Hive does not
  edit it on your behalf. The row goes italic and grey with "not found" in its tooltip, and right-click →
  Unpin removes it when you decide to.

Two more decisions that are already live in milestone (a):

- **Places that point nowhere are omitted.** Each place resolves via `glib::user_special_dir()`, falls back to
  the conventional `~/Name`, and if neither exists on disk the row is not rendered at all. Hive does not offer
  to create the missing directories. Without `xdg-user-dirs` installed, that correctly yields Home, Downloads,
  and Trash and nothing else.
- **Devices is hidden unless it has something worth showing.** `gio::VolumeMonitor` is never empty even
  without `gvfs` — GIO has a built-in Unix volume monitor that reports the mount table — so "no backend"
  cannot be detected by asking whether a backend exists. Hive instead shows the section only when at least one
  mount is user-relevant (removable or ejectable media, or a real mount outside `/`, `/boot`, and the
  pseudo-filesystems). Otherwise it is hidden silently: no empty section, no error, no nag banner.

---

## Architecture

```
src/
  model/     sort comparators, filter predicate, path normalization,
             the undo stack and its re-validation, copy/move pre-flight,
             clipboard payload formats, trashinfo parsing, the pinned
             list and its reordering                                  — no GTK, unit-tested
  config/    versioned schema, atomic writes, malformed-file recovery  — no GTK, unit-tested
  colors/    the folder-color store: slot names by absolute path,
             lenient reads, lazy pruning                              — no GTK, unit-tested
  theme/     palette types, Catppuccin constants, CSS generator,
             follow-system resolution                                 — no GTK except provider.rs
  fs/        places resolution, volume relevance, the off-thread
             operation worker, trashing and restoring                 — policy is plain Rust
  ui/        window, sidebar, breadcrumb, file pane, status bar,
             clipboard, dialogs, progress, context menu, the colour
             picker and the pin drag payload                          — GTK layer, holds no policy
  app.rs     adw::Application, HANDLES_OPEN, startup
  cli.rs     argument parsing                                         — unit-tested
```

The listing is built on GTK's own list infrastructure rather than a hand-rolled enumerator:

```
gtk::DirectoryList          async, incremental, monitored, io_priority tuned
  └─ gtk::FilterListModel     hidden-file toggle, substring filter
       └─ gtk::SortListModel    name / size / modified / type, folders-first
            └─ gtk::MultiSelection
                 └─ gtk::ColumnView (list)  |  gtk::GridView (grid)
```

`DirectoryList` already provides progressive loading, cancellation on navigate-away, and directory
monitoring, all upstream-tested. Both views bind to the *same* selection model, so switching between list and
grid is a stack page change and never re-enumerates.

Decision rules live in `model/` as plain functions; the `CustomFilter` and `CustomSorter` are thin shims that
read a `gio::FileInfo` into those functions' input types. That is what lets `cargo test` cover the rules that
decide what you see without needing a display.

**`std::fs` carve-out.** User-facing filesystem work goes through `gio`, so trash, mounts, URI handling, and
monitoring match the rest of the desktop. Three modules are exempt and use `std::fs`: `config`, the
folder-color store, and the read-only theme-registry scan. All three must stay GTK/gio-free to remain
unit-testable without a display, and all three touch only Hive's own config area. `config` and `colors` share
one write-then-rename helper (`config::atomic`) rather than each carrying its own copy — an atomic write is
exactly the code that must not drift between two callers.

---

## Testing

```bash
cargo test                                # unit tests, no display needed
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

`unwrap`, `expect`, `panic!`, `todo!`, and `unimplemented!` are denied crate-wide by clippy. Tests opt out
with a module-level `#![allow(...)]`. A file manager that panics is a file manager that loses your work.

### Torture directory

```bash
./scripts/make-torture-dir.py /tmp/hive-torture          # standard
./scripts/make-torture-dir.py /tmp/hive-torture --big    # plus 50,000 files
./scripts/make-torture-dir.py /tmp/hive-torture --clean  # remove
```

Generates deep nesting, invalid-UTF-8 and newline-containing filenames, broken symlinks, a symlink loop, a
directory symlink pointing at its own parent, a zero-byte file, a sparse 4 GiB file, and unreadable
files and directories. Python 3 stdlib only.

### Manual QA checklist

Every item marked *(a)* through *(e)* is verifiable now; the rest arrive with their milestone.

**Listing and stability**

- [ ] *(a)* A directory with 50,000 files stays scrollable and responsive while it loads.
- [ ] *(a)* The item count in the status line rises progressively and settles at the right number.
- [ ] *(a)* Navigating away from a large directory mid-load cancels immediately, with no stall.
- [ ] *(a)* A permission-denied directory shows a banner and does not crash.
- [ ] *(a)* Broken symlinks, symlink loops, and a directory symlink to its own parent all list without hanging.
- [ ] *(a)* Filenames containing newlines and invalid UTF-8 appear, select, and sort.
- [ ] *(a)* A zero-byte file and a sparse 4 GiB file both show sensible sizes.
- [ ] *(a)* `Ctrl+H` reveals dotfiles and editor backups; the count updates.
- [ ] *(a)* Folders sort ahead of files, and `file2` sorts before `file10`.
- [ ] *(a)* Deleting the directory being viewed while it is open does not leave a dead window.
- [ ] Files that vanish mid-operation are handled without a crash.
- [ ] An unresponsive network mount never freezes the UI.

**Navigation**

- [ ] `Alt+Left`/`Alt+Right` walk history; the buttons grey out at each end.
- [ ] Navigating somewhere new after going back discards the forward trail.
- [ ] `Alt+Up` goes to the parent and selects the folder you just left.
- [ ] Mouse side buttons go back and forward.
- [ ] `Ctrl+L` shows the path; `Tab` completes a unique match and appends `/`.
- [ ] `Ctrl+L` then `Escape` restores the breadcrumb and returns focus to the list.
- [ ] `Ctrl+F` filters as you type; `Escape` clears the filter, not just the box.
- [ ] Typing letters with the list focused jumps to the next matching name.
- [ ] Double-clicking a file opens it in the right application.
- [ ] `hive --select FILE` from a second terminal reveals it in the running window, with no second process.

**Layout under a tiling compositor**

- [ ] *(a)* Usable at 500×400: nothing clipped, unreachable, or overflowing.
- [ ] *(a)* The sidebar collapses to an overlay below 640 px width and returns above it.
- [ ] *(a)* Usable up to ultrawide without the layout falling apart.
- [ ] *(a)* Resizing to arbitrary geometry via `hyprctl dispatch resizewindowpixel` never clips content.
- [ ] *(a)* A deep path scrolls the breadcrumb rather than forcing the window wider.

**Theming**

- [ ] *(a)* First launch is Mocha with a mauve accent, with no portal dependency.
- [ ] *(a)* Icons and text are crisp at `scale = 1.20`.
- [ ] *(a)* A malformed file in `themes/` is skipped with a banner; Hive still starts.
- [ ] *(a)* A user theme whose `id` matches a built-in replaces it in place.
- [ ] *(d)* Switching flavors applies live, with no restart, no flicker, and no view rebuild.
- [ ] *(d)* A light flavor applies **completely** — file pane, sidebar text and header icons all follow.
- [ ] *(d)* A theme dropped into `themes/` appears after Reload, without a restart.
- [ ] *(d)* Changing the accent recolours selection and focus rings immediately.
- [ ] *(d)* With follow-system on and no portal preference, the configured flavor stays put.
- [ ] *(e)* A folder colored `mauve` is Mocha mauve in Mocha and Latte mauve in Latte.

**Folder colors**

- [ ] *(e)* Right-click a folder → Color shows the fourteen swatches in the loaded flavor, plus None.
- [ ] *(e)* Picking a color repaints the icon immediately, in the pane and in the sidebar, with a toast.
- [ ] *(e)* None clears the color.
- [ ] *(e)* Coloring a multi-selection colors all of them in one go.
- [ ] *(e)* Colors survive a restart, and only the icon is tinted — not the row, not the label.
- [ ] *(e)* Deleting a colored folder drops its entry the next time that directory is listed.
- [ ] *(e)* A folder whose name is invalid UTF-8 refuses with an explanation instead of failing silently.
- [ ] *(e)* Nothing appears inside the folders themselves — no `.directory`, no dotfiles, no xattrs.
- [ ] *(e)* The Color grid is reachable and operable from the keyboard alone.

**Pinning**

- [ ] *(e)* Right-click a folder → Pin to Sidebar adds it, with a toast, and it survives a restart.
- [ ] *(e)* Dragging a folder from the pane onto the sidebar pins it, including when nothing is pinned yet.
- [ ] *(e)* Dragging a pinned row up or down reorders it, and the order survives a restart.
- [ ] *(e)* Dropping on the lower half of the last row moves an item to the end of the list.
- [ ] *(e)* Right-click a pinned row → Unpin removes it.
- [ ] *(e)* Unpin from Sidebar in the pane's context menu removes it.
- [ ] *(e)* A pinned folder that no longer exists is dimmed rather than deleted, and can still be unpinned.
- [ ] *(e)* Dragging a folder over another application refuses the drop — Hive's drags stay inside Hive.
- [ ] *(e)* Rubber-band selection still works when started on empty space in the pane.

**Config**

- [ ] *(a)* Deleting `config.toml` regenerates defaults silently on next launch.
- [ ] *(a)* Corrupting `config.toml` backs it up to `.bak`, regenerates, and shows a banner.
- [ ] *(a)* A typo in one enum key falls back for that key alone and preserves the rest.
- [ ] *(a)* View mode and hidden-file state persist across a restart.

**Sidebar**

- [ ] *(a)* Places lists only directories that exist; no dead rows.
- [ ] *(a)* Devices is absent entirely when no removable media is attached.
- [ ] Plugging in a USB stick makes Devices appear; ejecting removes it.
- [ ] Yanking a drive mid-browse falls back to Home with a non-blocking banner and never crashes.

**File operations**

- [ ] *(c)* Copy/move conflicts offer Replace / Keep Both / Skip / Cancel with "do this for everything else" — never a silent overwrite.
- [ ] *(c)* Copying a directory into its own subtree, or onto itself, is refused before a single byte moves.
- [ ] *(c)* Insufficient free space is refused up front, showing required vs available.
- [ ] *(c)* A cross-filesystem move deletes each source only after its own copy fully succeeds.
- [ ] *(c)* A same-filesystem move reports as instant; a cross-filesystem move reports as copy-then-delete.
- [ ] *(c)* Cancelling mid-copy stops promptly and reports how many items were already done.
- [ ] *(c)* `Ctrl+Z` reverses trash, rename, move, copy, duplicate, and create.
- [ ] *(c)* Undo of a trash restores from the recorded location and removes the trash entry.
- [ ] *(c)* Undo refuses, with an explanation, when a copied file has been edited since the copy.
- [ ] *(c)* Permanent delete never appears on the undo stack.
- [ ] *(c)* `Delete` on a filesystem with no Trash offers permanent delete instead of failing silently.
- [ ] *(c)* Copying a symlink copies the link, not its target; a broken link copies without an error.
- [ ] *(c)* A file with a newline or invalid UTF-8 in its name copies, pastes, and shows the right name.
- [ ] *(c)* `Delete` and `Ctrl+A` inside the rename dialog and the path entry edit text — they do not touch files.
- [ ] *(c)* Cut-then-paste into the same folder is a no-op, not a duplicate and not a deletion.
- [ ] *(c)* Deleting the folder being viewed lands on the nearest surviving parent, not Home.
- [ ] *(c)* Closing the window while Hive owns a file clipboard asks before quitting.
- [ ] Pasting into Nautilus or Thunar works, and Hive pastes what they copied — including cut.
- [ ] A large copy onto a slow USB stick shows progress and cancels promptly.

---

## Roadmap

- **(a) — done.** Window, sidebar, progressive non-blocking directory listing, theme constants + generator +
  `CssProvider`, config load/save with tests, `adw::Application` with `HANDLES_OPEN`.
- **(b) — done.** History with back/forward, parent, mouse side buttons, `Ctrl+L` path entry with tab
  completion, `Ctrl+F` filter, `Ctrl+A`, type-ahead, optional vim keys, opening files through `gio::AppInfo`,
  `--select` reveal, and single-instance handoff.
- **(c) — done.** Copy, cut, paste, move, rename, duplicate, trash, permanent delete, new folder and new
  file, all on a worker thread with progress and cancel; the copy/move pre-flight checks; conflict handling;
  and a 20-entry undo stack with re-validation. Clipboard interop with other file managers, and the trash,
  case-only-rename, symlink and self-reference hazards.
- **(d) — done.** Theming UI: flavor switcher, live swap, accent setting, follow-system, user-theme
  directory.
- **(e) — done.** Folder colors stored by accent slot with lazy pruning, and sidebar pinning with
  drag-to-pin, drag-to-reorder and right-click unpin.
- **(f)** Polish: thumbnails, properties dialog with opt-in cancellable recursive size, status line, animations.

Deliberately out of scope for v1: tabs, split panes, terminal embedding, archive management, network-mount UI,
bulk rename, video thumbnails, and redo.

---

## License

MIT. See [LICENSE](LICENSE).
