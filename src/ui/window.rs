//! The main window: sidebar, header bar, file pane, status line.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;

use crate::colors;
use crate::config::{self, Config, ViewMode, defaults};
use crate::fs::open;
use crate::model::History;
use crate::model::filter::FilterSpec;
use crate::model::sort::SortSpec;
use crate::model::undo;
use crate::theme::{Registry, StyleOptions, ThemeProvider, system};
use crate::ui::breadcrumb::Breadcrumb;
use crate::ui::clipboard::FileClipboard;
use crate::ui::context_menu;
use crate::ui::debounce::Debouncer;
use crate::ui::dialogs;
use crate::ui::file_pane::FilePane;
use crate::ui::keys;
use crate::ui::path_entry::PathEntry;
use crate::ui::preferences;
use crate::ui::properties;
use crate::ui::sidebar::Sidebar;
use crate::ui::status_bar::StatusBar;
use crate::ui::thumbnails::Thumbnailer;

/// A menu or accelerator handler, of which there are enough to want a name.
type WindowAction = Box<dyn Fn(&Rc<Window>)>;

pub struct Window {
    pub(crate) window: adw::ApplicationWindow,
    pub(crate) file_pane: Rc<FilePane>,
    pub(crate) sidebar: Rc<Sidebar>,
    breadcrumb: Rc<Breadcrumb>,
    status: Rc<StatusBar>,
    banner: adw::Banner,
    toasts: adw::ToastOverlay,
    split: adw::OverlaySplitView,
    pub(crate) config: Rc<RefCell<Config>>,
    pub(crate) registry: Rc<RefCell<Registry>>,
    /// Folder colors, by accent slot and absolute path.
    pub(crate) colors: Rc<RefCell<colors::Store>>,
    theme: ThemeProvider,
    status_debouncer: RefCell<Option<Debouncer>>,
    history: RefCell<History<String>>,
    /// True while replaying history, so back/forward do not push new entries.
    replaying: std::cell::Cell<bool>,
    path_entry: Rc<PathEntry>,
    title_stack: gtk::Stack,
    search_bar: gtk::SearchBar,
    search_entry: gtk::SearchEntry,
    back_button: gtk::Button,
    forward_button: gtk::Button,
    up_button: gtk::Button,
    /// Inverses for everything destructive, bounded and in-session only.
    pub(crate) undo: RefCell<undo::Stack>,
    pub(crate) clipboard: Rc<FileClipboard>,
    /// One operation at a time; queueing is out of scope for v1.
    pub(crate) busy: std::cell::Cell<bool>,
    pub(crate) undo_action: gio::SimpleAction,
    last_toast: RefCell<Option<adw::Toast>>,
    /// The Flavor submenu, rebuilt whenever the theme list changes.
    flavor_menu: gio::Menu,
    pub(crate) flavor_action: gio::SimpleAction,
}

impl Window {
    pub fn new(
        app: &adw::Application,
        config: Rc<RefCell<Config>>,
        registry: Rc<RefCell<Registry>>,
        colors: Rc<RefCell<colors::Store>>,
        theme: ThemeProvider,
    ) -> Rc<Self> {
        let (filter_spec, sort_spec, view_mode, thumbnail_limits) = {
            let config = config.borrow();
            (
                FilterSpec::new(config.view.show_hidden, ""),
                SortSpec::new(
                    config.view.sort_key,
                    config.view.sort_order,
                    config.view.folders_first,
                ),
                config.view.mode,
                config.thumbnails,
            )
        };

        let thumbnails = Thumbnailer::new(crate::paths::thumbnail_cache_dir(), thumbnail_limits);
        let file_pane = FilePane::new(filter_spec, sort_spec, Rc::clone(&colors), thumbnails);
        file_pane.set_view_mode(view_mode);

        let sidebar = Sidebar::new(Rc::clone(&config), Rc::clone(&colors));
        let breadcrumb = Breadcrumb::new();
        let status = Rc::new(StatusBar::new());

        let banner = adw::Banner::new("");
        banner.set_revealed(false);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("hive-content");
        content.append(&banner);
        content.append(file_pane.widget());
        content.append(status.widget());

        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&content));

        // The title slot holds either the breadcrumb or the Ctrl+L entry.
        let path_entry = PathEntry::new();
        let title_stack = gtk::Stack::new();
        title_stack.set_transition_type(gtk::StackTransitionType::None);
        title_stack.add_named(breadcrumb.widget(), Some("breadcrumb"));
        title_stack.add_named(path_entry.widget(), Some("entry"));
        title_stack.set_visible_child_name("breadcrumb");

        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&title_stack));

        let search_entry = gtk::SearchEntry::new();
        search_entry.set_placeholder_text(Some("Filter this folder…"));
        search_entry.set_hexpand(true);
        // GtkSearchBar's revealer only collapses vertically, so a closed search
        // bar still contributes its entry's width minimum to the whole window.
        // Ask for one character; hexpand gives it the room it needs when open.
        search_entry.set_width_chars(1);
        search_entry.set_max_width_chars(-1);
        let search_bar = gtk::SearchBar::builder().child(&search_entry).build();
        search_bar.set_show_close_button(true);

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.add_top_bar(&search_bar);
        toolbar.set_content(Some(&toasts));

        let split = adw::OverlaySplitView::builder()
            .sidebar(sidebar.widget())
            .content(&toolbar)
            .max_sidebar_width(f64::from(defaults::SIDEBAR_WIDTH_PX))
            .min_sidebar_width(200.0)
            .sidebar_width_fraction(0.22)
            .build();

        let back_button = nav_button("go-previous-symbolic", "Back (Alt+Left)");
        let forward_button = nav_button("go-next-symbolic", "Forward (Alt+Right)");
        let up_button = nav_button("go-up-symbolic", "Parent folder (Alt+Up)");
        header.pack_start(&back_button);
        header.pack_start(&forward_button);
        header.pack_start(&up_button);

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Hive")
            .content(&split)
            .width_request(360)
            .height_request(300)
            .default_width(1100)
            .default_height(700)
            .build();

        let breakpoint = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
            adw::BreakpointConditionLengthType::MaxWidth,
            f64::from(defaults::SIDEBAR_BREAKPOINT_PX),
            adw::LengthUnit::Px,
        ));
        breakpoint.add_setter(&split, "collapsed", Some(&true.to_value()));
        window.add_breakpoint(breakpoint);

        let clipboard = FileClipboard::for_widget(&window);
        let undo_action = gio::SimpleAction::new("undo", None);
        undo_action.set_enabled(false);

        let active_flavor = config.borrow().appearance.flavor.clone();
        let flavor_action = gio::SimpleAction::new_stateful(
            "flavor",
            Some(glib::VariantTy::STRING),
            &active_flavor.to_variant(),
        );

        let this = Rc::new(Self {
            window,
            file_pane,
            sidebar,
            breadcrumb,
            status,
            banner,
            toasts,
            split,
            config,
            registry,
            colors,
            theme,
            status_debouncer: RefCell::new(None),
            history: RefCell::new(History::empty()),
            replaying: std::cell::Cell::new(false),
            path_entry,
            title_stack,
            search_bar,
            search_entry,
            back_button,
            forward_button,
            up_button,
            undo: RefCell::new(undo::Stack::new()),
            clipboard,
            busy: std::cell::Cell::new(false),
            undo_action,
            last_toast: RefCell::new(None),
            flavor_menu: gio::Menu::new(),
            flavor_action,
        });

        this.build_header(&header);
        this.wire_navigation();
        this.wire_folders();
        this.wire_search();
        this.wire_path_entry();
        this.wire_status();
        this.wire_thumbnails();
        this.wire_animations();
        this.install_actions(app);
        this.install_operation_actions(app);
        this.wire_close_request();
        context_menu::install(&this);
        this.apply_theme();

        // Best effort: if the portal never answers, this simply never fires and
        // the configured flavor stays in place.
        let watcher = Rc::downgrade(&this);
        system::watch(move |_| {
            if let Some(window) = watcher.upgrade()
                && window.config.borrow().appearance.follow_system
            {
                window.apply_theme();
            }
        });

        this
    }

    pub fn widget(&self) -> &adw::ApplicationWindow {
        &self.window
    }

    pub fn present(&self) {
        self.window.present();
        // Focus only takes during construction once the window is realized, so
        // the first listing gets the keyboard here rather than in `navigate`.
        self.file_pane.focus_view();
    }

    fn build_header(self: &Rc<Self>, header: &adw::HeaderBar) {
        let toggle_sidebar = gtk::ToggleButton::new();
        toggle_sidebar.set_icon_name("sidebar-show-symbolic");
        toggle_sidebar.set_tooltip_text(Some("Toggle sidebar (F9)"));
        toggle_sidebar.set_active(true);
        toggle_sidebar
            .bind_property("active", &self.split, "show-sidebar")
            .bidirectional()
            .sync_create()
            .build();
        header.pack_start(&toggle_sidebar);

        let menu = gio::Menu::new();

        let create_section = gio::Menu::new();
        create_section.append(Some("New Folder…"), Some("win.new-folder"));
        create_section.append(Some("New File…"), Some("win.new-file"));
        menu.append_section(None, &create_section);

        let edit_section = gio::Menu::new();
        edit_section.append(Some("Undo"), Some("win.undo"));
        edit_section.append(Some("Paste"), Some("win.paste"));
        menu.append_section(None, &edit_section);

        let view_section = gio::Menu::new();
        view_section.append(Some("Show Hidden Files"), Some("win.toggle-hidden"));
        view_section.append(Some("Grid View"), Some("win.toggle-view"));
        view_section.append(Some("Properties"), Some("win.properties"));
        menu.append_section(None, &view_section);

        // The flavor switcher lives here as well as in the dialog: switching is
        // a thing you do often and idly, and it should not cost a dialog.
        let theme_section = gio::Menu::new();
        theme_section.append_submenu(Some("Flavor"), &self.flavor_menu);
        theme_section.append(Some("Appearance…"), Some("win.appearance"));
        menu.append_section(None, &theme_section);
        self.rebuild_flavor_menu();

        let app_section = gio::Menu::new();
        app_section.append(Some("About Hive"), Some("win.about"));
        menu.append_section(None, &app_section);

        let menu_button = gtk::MenuButton::new();
        // F10 is the conventional way into an application's primary menu, and
        // it is the only route to the flavor switcher without a mouse.
        menu_button.set_primary(true);
        menu_button.set_icon_name("open-menu-symbolic");
        menu_button.set_tooltip_text(Some("Main menu"));
        menu_button.set_menu_model(Some(&menu));
        header.pack_end(&menu_button);

        let view_toggle = gtk::Button::new();
        view_toggle.set_tooltip_text(Some("Switch between list and grid (Ctrl+T)"));
        let mode = self.config.borrow().view.mode;
        view_toggle.set_icon_name(view_icon(mode));
        let this = Rc::clone(self);
        let button = view_toggle.clone();
        view_toggle.connect_clicked(move |_| {
            let next = this.config.borrow().view.mode.toggled();
            this.set_view_mode(next);
            button.set_icon_name(view_icon(next));
        });
        header.pack_end(&view_toggle);
    }

    fn wire_navigation(self: &Rc<Self>) {
        let this = Rc::clone(self);
        self.sidebar.connect_navigate(move |file| {
            this.navigate_to(&file);
        });

        let this = Rc::clone(self);
        self.breadcrumb.connect_navigate(move |file| {
            this.navigate_to(&file);
        });

        // Activating a directory enters it; activating a file opens it in its
        // handler application.
        let this = Rc::clone(self);
        self.file_pane.connect_activate(move |file, is_dir| {
            if is_dir {
                this.navigate_to(&file);
            } else {
                this.open_file(&file);
            }
        });

        let this = Rc::clone(self);
        self.file_pane.connect_error(move |error| {
            this.handle_enumeration_error(&error);
        });

        let this = Rc::clone(self);
        self.back_button.connect_clicked(move |_| this.go_back());

        let this = Rc::clone(self);
        self.forward_button
            .connect_clicked(move |_| this.go_forward());

        let this = Rc::clone(self);
        self.up_button.connect_clicked(move |_| this.go_up());

        // Mouse side buttons. GTK reports them as buttons 8 and 9.
        let side_buttons = gtk::GestureClick::builder().button(0).build();
        let this = Rc::clone(self);
        side_buttons.connect_pressed(move |gesture, _, _, _| match gesture.current_button() {
            8 => {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                this.go_back();
            }
            9 => {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                this.go_forward();
            }
            _ => {}
        });
        self.window.add_controller(side_buttons);

        // Type-ahead, and hjkl when it is switched on.
        let config = Rc::clone(&self.config);
        let this = Rc::clone(self);
        keys::install(
            &self.file_pane,
            move || config.borrow().behavior.vim_keys,
            move |action| match action {
                keys::Action::Parent => this.go_up(),
                keys::Action::Activate => this.activate_selection(),
            },
        );

        // Keep a pending --select request alive until the entry shows up.
        let this = Rc::clone(self);
        self.file_pane
            .connect_items_changed(move || this.file_pane.apply_pending_selection());
    }

    /// Point the whole window at `file`, recording it in history.
    pub fn navigate_to(self: &Rc<Self>, file: &gio::File) {
        self.navigate_internal(file, true);
    }

    fn navigate_internal(self: &Rc<Self>, file: &gio::File, record: bool) {
        let uri = file.uri().to_string();
        tracing::debug!(%uri, record, "navigating");

        if record && !self.replaying.get() {
            self.history.borrow_mut().push(uri.clone());
        }

        self.banner.set_revealed(false);
        self.file_pane.clear_pending_selection();
        self.file_pane.set_location(file);
        self.breadcrumb.set_location(file);
        self.sidebar.sync_selection(Some(file));
        self.status.update_free_space_for(file);
        self.sync_navigation_buttons();

        let title = file
            .path()
            .map(|path| {
                let name = crate::model::path::display_name(&path);
                if name == "/" { "/".to_owned() } else { name }
            })
            .unwrap_or_else(|| file.uri().to_string());
        self.window.set_title(Some(&format!("{title} — Hive")));

        if let Some(debouncer) = self.status_debouncer.borrow().as_ref() {
            debouncer.flush();
        }

        // Arriving somewhere should leave the keyboard in the list. Without
        // this, focus stays wherever it was — a sidebar row, a breadcrumb
        // button — and arrows and type-ahead do nothing until you click.
        if self.title_stack.visible_child_name().as_deref() != Some("entry") {
            self.file_pane.focus_view();
        }
    }

    pub fn go_back(self: &Rc<Self>) {
        let target = self.history.borrow_mut().back().cloned();
        if let Some(uri) = target {
            self.replay(&uri);
        }
    }

    pub fn go_forward(self: &Rc<Self>) {
        let target = self.history.borrow_mut().forward().cloned();
        if let Some(uri) = target {
            self.replay(&uri);
        }
    }

    /// Navigate without touching the history cursor, which `back`/`forward`
    /// have already moved.
    fn replay(self: &Rc<Self>, uri: &str) {
        self.replaying.set(true);
        self.navigate_internal(&gio::File::for_uri(uri), false);
        self.replaying.set(false);
    }

    /// Go to the parent directory, selecting the folder just left.
    pub fn go_up(self: &Rc<Self>) {
        let Some(current) = self.file_pane.location() else {
            return;
        };
        let Some(parent) = current.parent() else {
            return;
        };

        self.navigate_to(&parent);

        // Select the directory we came from so the eye lands where it left off.
        if let Some(path) = current.path() {
            self.file_pane.request_selection(path);
        }
    }

    /// Open whatever is selected: enter a directory, or launch a file.
    pub fn activate_selection(self: &Rc<Self>) {
        let Some(position) = self.file_pane.selected_position() else {
            return;
        };
        let Some(file) = self.file_pane.file_at(position) else {
            return;
        };
        let is_dir = self
            .file_pane
            .info_at(position)
            .is_some_and(|info| info.file_type() == gio::FileType::Directory);

        if is_dir {
            self.navigate_to(&file);
        } else {
            self.open_file(&file);
        }
    }

    /// Launch `file` with its handler application.
    fn open_file(self: &Rc<Self>, file: &gio::File) {
        let context = gtk::prelude::WidgetExt::display(&self.window).app_launch_context();
        match open::open(file, Some(&context)) {
            Ok(()) => tracing::debug!(uri = %file.uri(), "opened"),
            Err(error) => {
                tracing::warn!(%error, uri = %file.uri(), "could not open file");
                self.show_toast(&format!("Could not open: {error}"));
            }
        }
    }

    /// Select `path` once it appears, revealing its parent first if needed.
    pub fn reveal(self: &Rc<Self>, path: &Path) {
        let Some(parent) = path.parent() else {
            self.navigate_to_path(path);
            return;
        };

        let already_there = self
            .file_pane
            .location()
            .and_then(|file| file.path())
            .is_some_and(|current| current == parent);

        if !already_there {
            self.navigate_to_path(parent);
        }
        self.file_pane.request_selection(path.to_path_buf());
    }

    fn sync_navigation_buttons(&self) {
        let history = self.history.borrow();
        self.back_button.set_sensitive(history.can_go_back());
        self.forward_button.set_sensitive(history.can_go_forward());
        self.up_button.set_sensitive(
            self.file_pane
                .location()
                .is_some_and(|file| file.parent().is_some()),
        );
    }

    fn wire_search(self: &Rc<Self>) {
        let this = Rc::clone(self);
        self.search_entry.connect_search_changed(move |entry| {
            this.file_pane.set_query(&entry.text());
            if let Some(debouncer) = this.status_debouncer.borrow().as_ref() {
                debouncer.trigger();
            }
        });

        // Closing the bar must clear the filter, or entries stay hidden with no
        // visible reason why.
        let this = Rc::clone(self);
        self.search_bar
            .connect_search_mode_enabled_notify(move |bar| {
                if !bar.is_search_mode() {
                    this.search_entry.set_text("");
                    this.file_pane.set_query("");
                    this.file_pane.focus_view();
                    if let Some(debouncer) = this.status_debouncer.borrow().as_ref() {
                        debouncer.trigger();
                    }
                }
            });

        let this = Rc::clone(self);
        self.search_entry.connect_stop_search(move |_| {
            this.search_bar.set_search_mode(false);
        });

        // Enter in the filter box moves to the results.
        let this = Rc::clone(self);
        self.search_entry.connect_activate(move |_| {
            this.file_pane.focus_view();
            if this.file_pane.selected_position().is_none() {
                this.file_pane.select_only(0);
            }
        });
    }

    fn wire_path_entry(self: &Rc<Self>) {
        // Coming back from the entry resets the breadcrumb's scroll position,
        // which would otherwise hide the directory the user is actually in.
        let breadcrumb = Rc::clone(&self.breadcrumb);
        self.title_stack
            .connect_visible_child_name_notify(move |stack| {
                if stack.visible_child_name().as_deref() == Some("breadcrumb") {
                    breadcrumb.scroll_to_current();
                }
            });

        let this = Rc::clone(self);
        self.path_entry.connect_activate(move |path| {
            this.title_stack.set_visible_child_name("breadcrumb");

            if path.is_dir() {
                this.navigate_to_path(&path);
            } else if path.exists() {
                this.reveal(&path);
            } else {
                this.show_toast(&format!("{} does not exist", path.display()));
            }
            this.file_pane.focus_view();
        });

        let this = Rc::clone(self);
        self.path_entry.connect_cancel(move || {
            this.title_stack.set_visible_child_name("breadcrumb");
            this.file_pane.focus_view();
        });
    }

    /// Show the Ctrl+L path entry, pre-filled with the current location.
    pub fn open_path_entry(self: &Rc<Self>) {
        let current = self
            .file_pane
            .location()
            .and_then(|file| file.path())
            .unwrap_or_else(crate::paths::home_dir);

        self.title_stack.set_visible_child_name("entry");
        self.path_entry.focus_with(&current);
    }

    pub fn navigate_to_path(self: &Rc<Self>, path: &Path) {
        self.navigate_to(&gio::File::for_path(path));
    }

    pub fn navigate_home(self: &Rc<Self>) {
        self.navigate_to_path(&crate::paths::home_dir());
    }

    pub fn location(&self) -> Option<gio::File> {
        self.file_pane.location()
    }

    /// Enumeration failed. Never fatal, never a modal.
    fn handle_enumeration_error(self: &Rc<Self>, error: &glib::Error) {
        tracing::warn!(%error, "directory enumeration failed");

        let vanished = error.matches(gio::IOErrorEnum::NotFound)
            || error.matches(gio::IOErrorEnum::NotMounted)
            || error.matches(gio::IOErrorEnum::HostNotFound);

        if vanished {
            // §10.1 hazard 8: renaming or deleting the folder being viewed must
            // land somewhere real, and the nearest surviving parent is closer to
            // where the user was than Home is.
            self.ensure_location_exists();
            return;
        }

        let message = if error.matches(gio::IOErrorEnum::PermissionDenied) {
            "Permission denied — you do not have access to this folder".to_owned()
        } else {
            format!("Could not read this folder: {}", error.message())
        };
        self.show_banner(&message);
    }

    /// Re-read the free space shown in the status line.
    ///
    /// Navigating asks once; a copy or a delete is the other thing that makes
    /// the number wrong, and a stale "241 GiB free" after emptying a folder is
    /// exactly the reading someone would act on.
    pub(crate) fn refresh_free_space(&self) {
        if let Some(location) = self.file_pane.location() {
            self.status.update_free_space_for(&location);
        }
    }

    pub fn show_banner(&self, message: &str) {
        self.banner.set_title(message);
        self.banner.set_revealed(true);
    }

    /// Show a transient status message.
    ///
    /// The previous one is dismissed first: these report what just happened, and
    /// a queue would leave the newest result hidden behind stale ones for as
    /// long as it takes them to time out.
    pub fn show_toast(&self, message: &str) {
        if let Some(previous) = self.last_toast.borrow_mut().take() {
            previous.dismiss();
        }
        let toast = adw::Toast::new(message);
        self.toasts.add_toast(toast.clone());
        *self.last_toast.borrow_mut() = Some(toast);
    }

    fn wire_status(self: &Rc<Self>) {
        let this = Rc::clone(self);
        let debouncer = Debouncer::new(defaults::DEBOUNCE_MS, move || {
            this.status.set_counts(
                this.file_pane.visible_count(),
                this.file_pane.total_count(),
                this.file_pane.selected_count(),
                this.file_pane.is_loading(),
            );
        });

        *self.status_debouncer.borrow_mut() = Some(debouncer.clone());

        self.file_pane
            .connect_items_changed(move || debouncer.trigger());
    }

    /// Repaint rows as thumbnails arrive.
    ///
    /// Coalesced on the same 150 ms window as the status line: a directory of
    /// photographs finishes decoding a few at a time, and one pass over the
    /// visible rows picks up everything that landed in between.
    fn wire_thumbnails(self: &Rc<Self>) {
        let pane = Rc::clone(&self.file_pane);
        let debouncer = Debouncer::new(defaults::DEBOUNCE_MS, move || pane.refresh_thumbnails());
        self.file_pane
            .thumbnails()
            .connect_ready(move || debouncer.trigger());
    }

    /// Follow `gtk-enable-animations`, including when it changes mid-session.
    ///
    /// The setting reaches the stylesheet through `apply_theme`, which zeroes
    /// every duration, and the widget-side animations through the pane. Reading
    /// it once at startup would leave a user who turns animations off still
    /// watching them until the next launch.
    fn wire_animations(self: &Rc<Self>) {
        let settings = gtk::Settings::for_display(&WidgetExt::display(&self.window));
        self.file_pane
            .set_animations(settings.is_gtk_enable_animations());

        let this = Rc::clone(self);
        settings.connect_gtk_enable_animations_notify(move |settings| {
            let enabled = settings.is_gtk_enable_animations();
            tracing::debug!(enabled, "animation setting changed");
            this.file_pane.set_animations(enabled);
            this.apply_theme();
        });
    }

    /// Properties for the selection, or for the folder being viewed.
    fn show_properties(self: &Rc<Self>) {
        let mut paths = self.file_pane.selected_paths();
        if paths.is_empty()
            && let Some(current) = self.file_pane.location().and_then(|file| file.path())
        {
            paths.push(current);
        }
        properties::present(self, paths);
    }

    fn install_actions(self: &Rc<Self>, app: &adw::Application) {
        let toggle_hidden = gio::SimpleAction::new_stateful(
            "toggle-hidden",
            None,
            &self.config.borrow().view.show_hidden.to_variant(),
        );
        let this = Rc::clone(self);
        toggle_hidden.connect_activate(move |action, _| {
            let next = !this.config.borrow().view.show_hidden;
            action.set_state(&next.to_variant());
            this.set_show_hidden(next);
        });
        self.window.add_action(&toggle_hidden);

        let toggle_view = gio::SimpleAction::new("toggle-view", None);
        let this = Rc::clone(self);
        toggle_view.connect_activate(move |_, _| {
            let next = this.config.borrow().view.mode.toggled();
            this.set_view_mode(next);
        });
        self.window.add_action(&toggle_view);

        let toggle_sidebar = gio::SimpleAction::new("toggle-sidebar", None);
        let this = Rc::clone(self);
        toggle_sidebar.connect_activate(move |_, _| {
            this.split.set_show_sidebar(!this.split.shows_sidebar());
        });
        self.window.add_action(&toggle_sidebar);

        let about = gio::SimpleAction::new("about", None);
        let this = Rc::clone(self);
        about.connect_activate(move |_, _| this.show_about());
        self.window.add_action(&about);

        let appearance = gio::SimpleAction::new("appearance", None);
        let this = Rc::clone(self);
        appearance.connect_activate(move |_, _| preferences::present(&this));
        self.window.add_action(&appearance);

        let this = Rc::clone(self);
        self.flavor_action
            .connect_activate(move |action, parameter| {
                let Some(id) = parameter.and_then(glib::Variant::str) else {
                    return;
                };
                action.set_state(&id.to_variant());
                this.set_flavor(id);
            });
        self.window.add_action(&self.flavor_action);

        let simple = [
            (
                "go-back",
                Box::new(|w: &Rc<Window>| w.go_back()) as Box<dyn Fn(&Rc<Window>)>,
            ),
            ("go-forward", Box::new(|w: &Rc<Window>| w.go_forward())),
            ("go-up", Box::new(|w: &Rc<Window>| w.go_up())),
            ("go-home", Box::new(|w: &Rc<Window>| w.navigate_home())),
            ("open-path", Box::new(|w: &Rc<Window>| w.open_path_entry())),
            (
                "select-all",
                Box::new(|w: &Rc<Window>| w.file_pane.select_all()),
            ),
            (
                "find",
                Box::new(|w: &Rc<Window>| {
                    w.search_bar.set_search_mode(true);
                    w.search_entry.grab_focus();
                }),
            ),
            ("properties", Box::new(|w: &Rc<Window>| w.show_properties())),
        ];

        for (name, handler) in simple {
            let action = gio::SimpleAction::new(name, None);
            let this = Rc::clone(self);
            action.connect_activate(move |_, _| handler(&this));
            self.window.add_action(&action);
        }

        app.set_accels_for_action("win.toggle-hidden", &["<Control>h"]);
        app.set_accels_for_action("win.toggle-view", &["<Control>t"]);
        app.set_accels_for_action("win.toggle-sidebar", &["F9"]);
        app.set_accels_for_action("win.go-back", &["<Alt>Left"]);
        app.set_accels_for_action("win.go-forward", &["<Alt>Right"]);
        // Backspace is the traditional parent key; it only reaches here when no
        // text entry has focus, since entries consume it first.
        app.set_accels_for_action("win.go-up", &["<Alt>Up"]);
        app.set_accels_for_action("win.go-home", &["<Alt>Home"]);
        app.set_accels_for_action("win.open-path", &["<Control>l"]);
        app.set_accels_for_action("win.find", &["<Control>f"]);
        app.set_accels_for_action("win.appearance", &["<Control>comma"]);
    }

    /// Actions for everything in §8's operation list, plus their accelerators.
    fn install_operation_actions(self: &Rc<Self>, app: &adw::Application) {
        let operations: [(&str, WindowAction); 11] = [
            ("copy", Box::new(|w: &Rc<Window>| w.copy_selection())),
            ("cut", Box::new(|w: &Rc<Window>| w.cut_selection())),
            ("paste", Box::new(|w: &Rc<Window>| w.paste())),
            (
                "duplicate",
                Box::new(|w: &Rc<Window>| w.duplicate_selection()),
            ),
            ("rename", Box::new(|w: &Rc<Window>| w.rename_selection())),
            ("trash", Box::new(|w: &Rc<Window>| w.trash_selection())),
            ("delete", Box::new(|w: &Rc<Window>| w.delete_selection())),
            ("new-folder", Box::new(|w: &Rc<Window>| w.new_folder())),
            ("new-file", Box::new(|w: &Rc<Window>| w.new_file())),
            (
                "activate-selection",
                Box::new(|w: &Rc<Window>| w.activate_selection()),
            ),
            (
                "empty-trash-hint",
                Box::new(|w: &Rc<Window>| w.show_trash_hint()),
            ),
        ];

        for (name, handler) in operations {
            let action = gio::SimpleAction::new(name, None);
            let this = Rc::clone(self);
            action.connect_activate(move |_, _| handler(&this));
            self.window.add_action(&action);
        }

        let this = Rc::clone(self);
        self.undo_action.connect_activate(move |_, _| this.undo());
        self.window.add_action(&self.undo_action);

        let open_with = gio::SimpleAction::new("open-with", Some(glib::VariantTy::STRING));
        let this = Rc::clone(self);
        open_with.connect_activate(move |_, parameter| {
            if let Some(id) = parameter.and_then(glib::Variant::str) {
                this.open_selection_with(id);
            }
        });
        self.window.add_action(&open_with);

        // New Folder means nothing to a text entry, so it can stay global.
        app.set_accels_for_action("win.new-folder", &["<Control><Shift>n"]);
        self.install_pane_shortcuts();
    }

    /// Shortcuts that a text entry must be able to take first.
    ///
    /// These cannot be application accelerators: those run above the focused
    /// widget, so `Delete` in the rename dialog would trash the selection
    /// instead of deleting a character, and Ctrl+A in the path entry would
    /// select every file rather than the text. A shortcut controller on the
    /// window in the bubble phase inverts that — the focused widget gets first
    /// refusal, and anything it does not want reaches the file actions. They
    /// still work with focus on the sidebar or a header button, which a
    /// controller scoped to the views would not.
    fn install_pane_shortcuts(self: &Rc<Self>) {
        const SHORTCUTS: [(&str, &str); 14] = [
            ("<Control>c", "win.copy"),
            ("<Control>x", "win.cut"),
            ("<Control>v", "win.paste"),
            ("<Control>d", "win.duplicate"),
            ("<Control>z", "win.undo"),
            ("<Control>a", "win.select-all"),
            ("F2", "win.rename"),
            // Keypad first: GTK shows the last-registered accelerator in
            // menus, and "Delete" reads better there than "Delete (keypad)".
            ("KP_Delete", "win.trash"),
            ("Delete", "win.trash"),
            ("<Shift>Delete", "win.delete"),
            ("Menu", "win.context-menu"),
            ("<Shift>F10", "win.context-menu"),
            // Both conventions, since file managers disagree about which one
            // opens Properties and muscle memory does not read release notes.
            ("<Control>i", "win.properties"),
            ("<Alt>Return", "win.properties"),
        ];

        let controller = gtk::ShortcutController::new();
        controller.set_scope(gtk::ShortcutScope::Local);
        controller.set_propagation_phase(gtk::PropagationPhase::Bubble);

        for (accelerator, action) in SHORTCUTS {
            let Some(trigger) = gtk::ShortcutTrigger::parse_string(accelerator) else {
                tracing::warn!(accelerator, "unparseable shortcut; skipping");
                continue;
            };
            controller.add_shortcut(gtk::Shortcut::new(
                Some(trigger),
                Some(gtk::NamedAction::new(action)),
            ));
        }

        self.window.add_controller(controller);
    }

    /// Launch the selection with a specific application, from "Open With".
    fn open_selection_with(self: &Rc<Self>, desktop_id: &str) {
        let Some(app) = gio::DesktopAppInfo::new(desktop_id) else {
            self.show_toast("That application is no longer installed");
            return;
        };

        let context = gtk::prelude::WidgetExt::display(&self.window).app_launch_context();
        for path in self.file_pane.selected_paths() {
            let file = gio::File::for_path(&path);
            if let Err(error) = open::open_with(&file, app.upcast_ref(), Some(&context)) {
                tracing::warn!(%error, path = %path.display(), "open with failed");
                self.show_toast(&format!("Could not open: {error}"));
            }
        }
    }

    fn show_trash_hint(self: &Rc<Self>) {
        self.show_toast("Emptying the Trash is not part of Hive; use your Trash folder");
    }

    /// §10.1 hazard 2: warn before the clipboard dies with the process.
    fn wire_close_request(self: &Rc<Self>) {
        let this = Rc::clone(self);
        self.window.connect_close_request(move |window| {
            if !this.config.borrow().behavior.warn_clipboard_on_quit {
                return glib::Propagation::Proceed;
            }
            if !this.clipboard.owns_files() {
                return glib::Propagation::Proceed;
            }

            let Some(what) = this.clipboard.describe_owned() else {
                return glib::Propagation::Proceed;
            };

            let window = window.clone();
            let this = Rc::clone(&this);
            glib::spawn_future_local(async move {
                if dialogs::confirm_quit_with_clipboard(&window, &what).await {
                    // Cleared first so the handler lets the second attempt past.
                    this.clipboard.forget();
                    window.close();
                }
            });

            glib::Propagation::Stop
        });
    }

    fn show_about(&self) {
        let about = adw::AboutDialog::builder()
            .application_name("Hive")
            .application_icon(crate::paths::APP_ID)
            .developer_name("Diren Deniz Yildirim")
            .version(env!("CARGO_PKG_VERSION"))
            .comments("Minimal pastel explorer.")
            .license_type(gtk::License::MitX11)
            .build();
        about.present(Some(&self.window));
    }

    pub fn set_view_mode(self: &Rc<Self>, mode: ViewMode) {
        self.file_pane.set_view_mode(mode);
        self.config.borrow_mut().view.mode = mode;
        self.save_config();
    }

    pub fn set_show_hidden(self: &Rc<Self>, show_hidden: bool) {
        self.file_pane.set_show_hidden(show_hidden);
        self.config.borrow_mut().view.show_hidden = show_hidden;
        self.save_config();
        if let Some(debouncer) = self.status_debouncer.borrow().as_ref() {
            debouncer.trigger();
        }
    }

    /// Regenerate and apply the stylesheet from the current config.
    ///
    /// Swapping one `CssProvider`'s content restyles every widget in place: no
    /// restart, no widget re-creation, and the file pane's model stack is never
    /// touched, so switching flavors does not re-enumerate the directory.
    pub fn apply_theme(&self) {
        let config = self.config.borrow();
        let registry = self.registry.borrow();

        let id = system::resolve(
            &system::Preference {
                flavor: &config.appearance.flavor,
                follow_system: config.appearance.follow_system,
                light_flavor: &config.appearance.light_flavor,
                dark_flavor: &config.appearance.dark_flavor,
            },
            system::current(),
        );

        let palette = registry.get_or_default(id);
        let options = StyleOptions {
            accent: config.appearance.accent,
            client_side_rounding: config.appearance.client_side_rounding,
            client_side_shadow: config.appearance.client_side_shadow,
            animations: gtk::Settings::for_display(&WidgetExt::display(&self.window))
                .is_gtk_enable_animations(),
        };

        ThemeProvider::sync_color_scheme(palette);
        self.theme.apply(palette, &options);
    }

    /// The palette currently on screen, for the preferences dialog.
    pub(crate) fn active_theme_id(&self) -> String {
        let config = self.config.borrow();
        system::resolve(
            &system::Preference {
                flavor: &config.appearance.flavor,
                follow_system: config.appearance.follow_system,
                light_flavor: &config.appearance.light_flavor,
                dark_flavor: &config.appearance.dark_flavor,
            },
            system::current(),
        )
        .to_owned()
    }

    /// Switch flavor from the menu, live.
    pub(crate) fn set_flavor(self: &Rc<Self>, id: &str) {
        {
            let mut config = self.config.borrow_mut();
            if config.appearance.flavor == id {
                return;
            }
            // Picking a flavor by hand is an explicit choice, and an explicit
            // choice always wins — so it takes the app off follow-system rather
            // than being silently ignored.
            config.appearance.flavor = id.to_owned();
            config.appearance.follow_system = false;
        }

        self.apply_theme();
        self.save_config();

        let name = self
            .registry
            .borrow()
            .get(id)
            .map(|palette| palette.name.to_string())
            .unwrap_or_else(|| id.to_owned());
        self.show_toast(&format!("Theme: {name}"));
    }

    /// Rebuild the Flavor submenu from the registry, marking the active one.
    pub(crate) fn rebuild_flavor_menu(self: &Rc<Self>) {
        self.flavor_menu.remove_all();

        for palette in self.registry.borrow().all() {
            let item = gio::MenuItem::new(Some(&palette.name), None);
            item.set_action_and_target_value(
                Some("win.flavor"),
                Some(&palette.id.as_ref().to_variant()),
            );
            self.flavor_menu.append_item(&item);
        }

        let active = self.config.borrow().appearance.flavor.clone();
        self.flavor_action.set_state(&active.to_variant());
    }

    /// Re-read `themes/` so a theme dropped in there appears without a restart.
    pub(crate) fn reload_themes(self: &Rc<Self>) -> usize {
        let reloaded = Registry::load(&crate::paths::themes_dir());
        let failures = reloaded.errors().len();
        for error in reloaded.errors() {
            tracing::warn!(%error, "ignoring theme file");
        }

        let count = reloaded.all().len();
        *self.registry.borrow_mut() = reloaded;
        self.rebuild_flavor_menu();
        self.apply_theme();

        if failures > 0 {
            self.show_toast(&format!(
                "{count} themes loaded, {failures} skipped as unreadable"
            ));
        }
        count
    }

    /// Persist the config, reporting failures without interrupting the user.
    pub(crate) fn save_config(&self) {
        let path = crate::paths::config_file();
        if let Err(error) = config::save(&path, &self.config.borrow()) {
            tracing::warn!(%error, path = %path.display(), "could not save config");
            self.show_toast("Could not save settings");
        }
    }
}

fn nav_button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    button.add_css_class("flat");
    button.set_tooltip_text(Some(tooltip));
    button.set_sensitive(false);
    button
}

fn view_icon(mode: ViewMode) -> &'static str {
    match mode {
        ViewMode::List => "view-grid-symbolic",
        ViewMode::Grid => "view-list-symbolic",
    }
}
