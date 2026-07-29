# Hive

Minimal pastel explorer — a file manager for Hyprland, written in Rust with GTK 4 and libadwaita.

Priority order: **stability > visual polish > feature count.** A small set of operations that never fail is
worth more than a wide feature set that occasionally corrupts or hangs.

> **Status: all six milestones, (a) through (f), are built.** The window, sidebar, theming system, config
> layer, non-blocking progressive listing, navigation, full keyboard support, the CLI, the whole
> file-operation layer — copy, move, rename, trash, delete, undo, with conflict handling and pre-flight
> checks — the theming UI, folder colors, sidebar pinning, and now image thumbnails, the properties dialog
> with opt-in recursive size, the status line and the animation pass. See [Roadmap](#roadmap).

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

**A second `hive` invocation opens another window** in the process that is already running, rather than
starting a second one (`gio::ApplicationFlags::HANDLES_OPEN`). Launching Hive again from a launcher is
therefore how you get a second window, the same as `Ctrl+N`; closing the last window ends the process.

Every launch is dispatched as `open`, including the one with nothing to open — home is what "no target"
means. `GApplication::run` emits an `activate` of its own on top of whatever was dispatched, and on the far
side of D-Bus the only thing that distinguishes the two is which signal arrived: so `open` makes windows and
`activate` presents the newest one, and one launch cannot become two windows.

---

## Keybindings

| Key | Action |
|---|---|
| `Double-click` / `Enter` | Enter a directory, or open a file in its handler application |
| `Alt+Left` / `Alt+Right` | Back / forward through history |
| Mouse buttons 8 / 9 | Back / forward |
| `Alt+Up` / `Backspace` | Parent folder (selects the folder you came from) |
| `Alt+Home` | Home |
| `Ctrl+N` | New window on the same folder |
| `Ctrl+L` | Path entry — `Tab` completes, `Enter` goes, `Escape` cancels |
| `Ctrl+F` | Filter this folder as you type; `Escape` clears and closes |
| `Ctrl+A` | Select all |
| `Ctrl+H` | Toggle hidden files |
| `Ctrl+T` | Toggle list / grid view |
| `Ctrl+,` | Appearance settings |
| `F10` | Main menu (including the flavor switcher) |
| `F9` | Toggle sidebar |
| Arrows, `Home`/`End`, `Page Up`/`Down` | Move the selection |
| Click-drag on empty space, `Ctrl+click`, `Shift+click` | Multi-select |
| Drag a row | Move it into a folder, out to another application, or onto the sidebar to pin |
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
| `Ctrl+I` / `Alt+Enter` | Properties — of the selection, or of this folder when nothing is selected |

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

## Sorting

**Sort** is in the main menu (`F10`) and in the right-click menu over the folder — the same submenu in both
places, so the two can never disagree about which ordering is active.

| Key | Orders by |
|---|---|
| Name | Natural order: `file2` before `file10`, case-insensitive |
| Size | Bytes. Folders never sort by their own size — they group and order by name |
| Date Modified | When the contents last changed |
| Date Added | When the entry turned up in this folder |
| Type | Content type, then name |

The direction is named after what it orders, because "ascending" answers a question nobody asked of a date:
**Newest / Oldest First** for the two dates, **Largest / Smallest First** for size, **Ascending / Descending**
for name and type. **Folders First** groups directories ahead of files under any key, and is on by default.

**Date Added is not modification time**, and that difference is the point of having it. Copying a file
preserves its modification time, so a photo taken last year and copied in this morning sorts a year deep under
Date Modified while appearing at the top under Date Added. Linux keeps no record of when a file was put into a
folder, so Hive uses the closest thing the filesystem has, in order:

1. **Creation time** (`time::created`) — ext4, btrfs and xfs all report it. This is what a file copied or
   downloaded into the folder gets.
2. **The inode's change time** (`time::changed`), which a move into the folder, a replacement or a permission
   change all update. Never absent on a local file.
3. Modification time, for a filesystem that reports neither — which is what such a filesystem knows anyway.

Ordering is **per window** while Hive is running: two windows on the same folder can be sorted differently,
which is half the reason to have two. The last choice is saved as the default for the next window, and lives in
`config.toml` under `[view]` — so it also survives a restart.

Sorting never re-reads the directory: the sorter is a comparator over the model that is already loaded, so
changing it costs one re-sort and nothing else. The selection is kept — it follows the files, not the row
numbers — and a large folder re-sorts incrementally rather than freezing the window.

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
sort_key = "name"             # name | size | modified | added | type
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

### Animations

Every transition Hive draws is **150 ms, ease-out, and nothing bouncy** — one duration for all of them, so
nothing feels slower than the thing next to it. Row and sidebar hover, header buttons, breadcrumb segments,
accent swatches, and the list ↔ grid crossfade all share it.

All of it follows **`gtk-enable-animations`**, live: turn the setting off and the stylesheet is regenerated
with every duration at zero and the view crossfade switched off, with no restart. Hive reads the setting
through `gtk::Settings` rather than assuming an XSETTINGS daemon is present.

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

## Drag and drop

Dragging works in both directions, and a drag out of Hive is a `text/uri-list` like any other — a browser's
upload field, a chat window and another file manager all accept it. Under the hood the payload is a
`GdkFileList` (which GDK serializes to `text/uri-list`, `text/plain` and the portal file-transfer types, so
sandboxed applications get it too), plus a boxed type private to Hive carrying the same paths losslessly.

**Where a drop can land:**

| Drop on | Destination |
|---|---|
| A folder row, in either view | Inside that folder |
| Anywhere else in the pane | The folder being viewed |
| The sidebar | Nothing is transferred — the sidebar pins, see below |

**Copy or move** is decided by where the drag came from, which is what the private payload is for:

| Drag | Default | `Ctrl` | `Shift` |
|---|---|---|---|
| From Hive — another window, or a folder in this one | **Move** | Copy | Move |
| From another application | **Copy** | Copy | Move, when that application offers one |

Neither key can ask for something the source never offered: `Shift` over a drag that will only be copied is
refused while the pointer is still over the row, rather than quietly copying instead. A drop reports what it
did — "Moved 3 items" — and `Ctrl+Z` reverses it, because a drop runs the same worker, the same pre-flight and
the same undo recording as `Ctrl+V`.

Two more decided cases:

- **Dropping into the folder the files are already in** is a fumble, not a request. As a move there is nothing
  to do and Hive says so. As a copy it means what `Ctrl+D` means, so it keeps both rather than raising a
  conflict dialog for every name in turn.
- **Dropping a folder onto itself** drops that folder from the drag and carries out the rest, rather than
  refusing the whole thing.

Only files on this machine can be dropped in. A `https` URI dragged out of a browser has no path, and is
refused with a message rather than turned into a path that looks real and is not.

The cost of all this: starting a drag on a row no longer starts a rubber-band selection there — begin the
rubber band on empty space instead, exactly as in every other file manager.

---

## Pinned folders

The **Pinned** section sits at the top of the sidebar and persists in `config.toml`.

- **Pin:** right-click a folder → **Pin to Sidebar**, or drag it from the file pane onto the sidebar.
- **Reorder:** drag a pinned row up or down. Dropping on the upper half of a row lands above it, the lower
  half below it, so the bottom of the list is reachable.
- **Unpin:** right-click a pinned row → **Unpin**, or right-click the folder in the pane →
  **Unpin from Sidebar**.

Pinned rows carry the folder's color, which is what makes a long list scannable at a glance.

Only folders can be pinned, and the pin payload says so: a drag carrying plain files offers the sidebar
nothing it accepts, so it pins nothing rather than pinning the parent folder or half the selection.

---

## Thumbnails

Image files get a thumbnail in place of their symbolic icon, in both views — small in the list, 64 px in the
grid. Nothing about it happens on the main thread: a row being drawn only ever looks in a hash map, and a
miss queues work and leaves the icon alone until a picture exists.

Three limits, all in `config.toml` under `[thumbnails]`:

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `true` | Turn the whole thing off |
| `max_pixels` | `256` | Longest edge of the cached image. Smaller images are never scaled up |
| `max_file_bytes` | `33554432` | Skip source files over 32 MiB rather than decoding them |
| `max_directory_entries` | `2000` | Above this many entries in one directory, stop thumbnailing entirely |

At most four images decode at once, and the backlog is capped: scroll quickly through a thousand photographs
and the ones you flew past are dropped rather than decoded for nobody. They are re-requested if you scroll
back. The queue is emptied on navigating away, so a huge folder you have left cannot delay the one you are
looking at.

**Only images.** Whatever gdk-pixbuf can decode, which on a normal Arch install means PNG, JPEG, WebP, GIF,
BMP, TIFF and — with `librsvg` present — SVG. Video thumbnails are out of scope for v1. A file that fails to
decode is remembered as having no thumbnail and is not retried.

Cached in `$XDG_CACHE_HOME/hive/thumbnails/`, one PNG per source path in a two-level fan-out directory. The
cache is never pruned automatically: it holds one file per distinct image you have ever looked at, and
re-thumbnailing an edited image *replaces* its entry rather than adding a second. Delete the directory to
reclaim the space; it rebuilds itself.

---

## Status line

One thin line along the bottom: how many items the folder holds, how many are selected, and how much space is
free where you are standing. While a directory is still enumerating the count carries an ellipsis and a
spinner; with a filter active it reads `12 of 2,102 items`.

It is fed through a **150 ms coalescing window**, so a directory monitor reporting a thousand changes in a
second redraws the line once rather than a thousand times. Free space is re-read when you navigate and again
whenever a file operation finishes — a stale "241 GiB free" after emptying a folder is exactly the number
someone would act on.

---

## Properties

`Ctrl+I` or `Alt+Enter`, or **Properties** in the context menu. With nothing selected it describes the folder
you are looking at. It shows the name, location, type and mime type, size, timestamps, permissions in both
`rwxr-xr-x` and octal form, and owner and group. Select several things and it reports the count and offers a
combined size instead.

A symlink is described as the link, not as its target — with the target on its own **Links to** row.

**A folder's size is never computed unless you ask.** The size row shows a **Calculate** button; pressing it
starts a walk on its own thread that reports a running total as it counts, and the button becomes **Cancel**.
Closing the dialog cancels it too. This is the one operation in a file manager most likely to freeze it, so
it is opt-in, off-thread, cancellable at any point, and always shows how far it got:

```
Size    3.2 GiB (3382856318 bytes) in 56065 items — stopped        [Calculate]
```

Symlinks are counted but never followed, the same rule `du` uses — which is why a symlink loop is a
non-event here. Hard links are counted once per name, so a tree of links to one large file reads larger than
the space it occupies on disk.

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
| 1 | **Recursive folder size is an unbounded tree walk** | Never computed automatically. The Properties dialog shows size behind an explicit **Calculate** button, run off-thread, cancellable at any point, updating incrementally, and reporting how far it got if you stop it. Closing the dialog cancels the walk. | done |
| 2 | **Wayland clipboard dies with the process** | Copying a file and then closing Hive loses the clipboard, because Wayland clipboard content is owned by the client. Closing the window while Hive still owns a file clipboard asks first — **Don't Quit** / **Quit Anyway** — and names what would be lost. It asks *only* while Hive actually owns it: the moment another application takes the clipboard the warning stops. Set `warn_clipboard_on_quit = false` to turn it off, or run `wl-clip-persist`, which makes the whole problem moot. | done |
| 3 | **Clipboard format interop** | Hive offers and accepts both `text/uri-list` and `x-special/gnome-copied-files`; the latter carries the cut-vs-copy distinction. A plain-text form rides along so pasting into a terminal gives the paths. A `text/uri-list` arriving from another application says nothing about intent and is read as a **copy** — treating an ambiguous paste as a move would delete that application's files. | done |
| 4 | **Trash fails on removable drives** | FAT/exFAT cannot host the per-mount `.Trash-$uid` the freedesktop spec wants, and neither can `tmpfs`. `Delete` detects the failure and offers permanent delete through a clear dialog. It never silently does nothing, and never silently permanently deletes instead. | done |
| 5 | **Thumbnail cache staleness** | Keyed on **(path, mtime, size)**, not path alone. The cache file is addressed by the path and carries the mtime and size it was made from as PNG text chunks; both must match or it is rewritten. Editing an image therefore replaces its thumbnail rather than showing the old one forever, and a same-second edit is still caught because the size changes too. | done |
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

Three more that thumbnails and the properties dialog brought with them:

- **An image that will not decode is remembered as having no thumbnail.** A corrupt file, or a format with no
  pixbuf loader installed, is tried once and then left with its symbolic icon. Retrying on every scroll would
  turn one broken file into a permanent background load.
- **The thumbnail cache is never pruned.** It holds one PNG per distinct image path you have looked at, and
  re-thumbnailing replaces rather than adds. There is no expiry sweep, because a sweep is a tree walk over
  your cache on a timer and this is a directory you can safely delete at any moment.
- **The properties dialog waits for the filesystem before it appears.** It is built in one pass from one
  `query_info`, which is what keeps every row present and the dialog sized correctly. Nothing blocks — the
  query is async — but on a mount that has stopped answering the effect is that no dialog appears rather than
  an empty one. The rest of Hive stays responsive throughout.

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
             list and its reordering, thumbnail eligibility and cache
             addressing                                               — no GTK, unit-tested
  config/    versioned schema, atomic writes, malformed-file recovery  — no GTK, unit-tested
  colors/    the folder-color store: slot names by absolute path,
             lenient reads, lazy pruning                              — no GTK, unit-tested
  theme/     palette types, Catppuccin constants, CSS generator,
             follow-system resolution                                 — no GTK except provider.rs
  fs/        places resolution, volume relevance, the off-thread
             operation worker, trashing and restoring, the cancellable
             recursive size walk                                      — policy is plain Rust
  ui/        window, sidebar, breadcrumb, file pane, status bar,
             clipboard, dialogs, progress, context menu, the colour
             picker, the drag payloads and drop targets, the thumbnail
             worker pool and the properties dialog                    — GTK layer, holds no policy
  app.rs     adw::Application, HANDLES_OPEN, startup, a window per
             launch over one shared config, theme and colour store
  cli.rs     argument parsing                                         — unit-tested
```

The listing is built on GTK's own list infrastructure rather than a hand-rolled enumerator:

```
gtk::DirectoryList          async, incremental, monitored, io_priority tuned
  └─ gtk::FilterListModel     hidden-file toggle, substring filter
       └─ gtk::SortListModel    name / size / modified / added / type, folders-first
            └─ gtk::MultiSelection
                 └─ gtk::ColumnView (list)  |  gtk::GridView (grid)
```

`DirectoryList` already provides progressive loading, cancellation on navigate-away, and directory
monitoring, all upstream-tested. Both views bind to the *same* selection model, so switching between list and
grid is a stack page change and never re-enumerates.

Decision rules live in `model/` as plain functions; the `CustomFilter` and `CustomSorter` are thin shims that
read a `gio::FileInfo` into those functions' input types. That is what lets `cargo test` cover the rules that
decide what you see without needing a display.

**`std::fs` carve-out.** User-facing filesystem work goes through `gio`, so trash, mounts, URI handling, the
thumbnail cache, and monitoring match the rest of the desktop. Three of Hive's own settings modules are
exempt and use `std::fs`: `config`, the folder-color store, and the read-only theme-registry scan. All three
must stay GTK/gio-free to remain unit-testable without a display, and all three touch only Hive's own config
area. `config` and `colors` share one write-then-rename helper (`config::atomic`) rather than each carrying
its own copy — an atomic write is exactly the code that must not drift between two callers.

The two **worker-thread walks** are the other exception, and a larger one worth naming plainly: the transfer
engine in `fs/ops.rs` and the recursive size walk in `fs/size.rs` both read metadata and list directories
with `std::fs` rather than gio. Both run on their own thread, both need `symlink_metadata` semantics
directly, and having two walks over the same trees disagree about how they read a directory would be worse
than either choice. The gio-shaped work these operations depend on — trashing, free space, URIs, launching —
still goes through gio. *If you would rather the whole engine spoke gio, that is a contained change to those
two files and worth doing deliberately rather than by drift.*

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

Every item here is verifiable now.

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
- [ ] *(e)* Dragging plain files onto the sidebar pins nothing, and transfers nothing.
- [ ] *(e)* Rubber-band selection still works when started on empty space in the pane.

**Drag and drop**

- [ ] *(g)* Dragging a row onto a folder row moves it inside, in both list and grid view.
- [ ] *(g)* Dragging between two Hive windows moves; `Ctrl` while dropping copies instead.
- [ ] *(g)* Dropping files back into the folder they came from says so, and moves nothing; with `Ctrl` it
      duplicates them instead of raising a conflict dialog.
- [ ] *(g)* Dragging a file out to a browser's upload field, or a chat window, hands over the real file.
- [ ] *(g)* Dragging files from another file manager into Hive copies them in, with progress and a toast.
- [ ] *(g)* `Ctrl+Z` reverses a drop — a move goes back, a copy is deleted.

**Sorting**

- [ ] *(g)* Sort → Date Added, Newest First puts a file copied in this morning above one edited today.
- [ ] *(g)* Switching between Date Added and Date Modified gives different orders for the same folder.
- [ ] *(g)* Choosing a date key relabels the direction to Newest / Oldest First, in both menus.
- [ ] *(g)* Two windows on one folder can be sorted differently, and the last choice is what a new window gets.
- [ ] *(g)* The ordering survives a restart, and a hand-edited nonsense `sort_key` falls back to name.

**Thumbnails**

- [ ] *(f)* A folder of photographs fills in thumbnails progressively, in both list and grid, without the
      window ever going unresponsive.
- [ ] *(f)* Scrolling fast through a large photo folder stays smooth, and thumbnails catch up behind you.
- [ ] *(f)* Editing an image updates its thumbnail the next time the folder is listed — no stale picture.
- [ ] *(f)* A corrupt image keeps its symbolic icon instead of showing a broken one, and Hive does not
      retry it endlessly.
- [ ] *(f)* A file over `max_file_bytes` is skipped and keeps its icon.
- [ ] *(f)* A directory over `max_directory_entries` shows no thumbnails at all.
- [ ] *(f)* `enabled = false` turns thumbnails off entirely on the next launch.
- [ ] *(f)* Deleting `$XDG_CACHE_HOME/hive/thumbnails/` costs one redraw and nothing else.
- [ ] *(f)* A very wide or very tall image does not distort the row or the grid cell.

**Properties**

- [ ] *(f)* `Ctrl+I` on a file shows name, location, type, exact size, timestamps, permissions, owner, group.
- [ ] *(f)* `Ctrl+I` with nothing selected describes the folder you are in.
- [ ] *(f)* Opening it on a folder computes **nothing** until Calculate is pressed.
- [ ] *(f)* Calculate counts up live, and Cancel stops it and reports how far it got.
- [ ] *(f)* Closing the dialog mid-walk stops the walk — no thread left counting.
- [ ] *(f)* A folder containing a symlink loop finishes rather than running forever.
- [ ] *(f)* A folder with unreadable subdirectories reports a count of what it could not read.
- [ ] *(f)* Properties on a multi-selection reports the count and a combined size.
- [ ] *(f)* Properties on a symlink describes the link and names its target.

**Status line and animations**

- [ ] *(f)* Item count, selection count and free space are all correct, and free space follows the folder.
- [ ] *(f)* Free space updates after a large copy or delete rather than staying stale.
- [ ] *(f)* Turning `gtk-enable-animations` off stops transitions immediately, without a restart.
- [ ] *(f)* Switching list ↔ grid crossfades briefly and never re-enumerates the directory.

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
- **(f) — done.** Image thumbnails on a capped worker pool with a two-level cache keyed on (path, mtime,
  size); the properties dialog with opt-in, cancellable, incrementally-reported recursive size; the status
  line; and the animation pass.
- **(g) — done.** Several windows — one per launch, and `Ctrl+N` — and drag and drop of files in both
  directions: out to any application as `text/uri-list`, in from any application, and between Hive's own
  windows and folders, running the same worker, pre-flight and undo recording as paste. Also the Sort menu the
  ordering options had been missing, and a Date Added key to go in it.

Deliberately out of scope for v1: tabs, split panes, terminal embedding, archive management, network-mount UI,
bulk rename, video thumbnails, and redo.

---

## License

MIT. See [LICENSE](LICENSE).
