//! The main window: sidebar, header bar, file pane, status line.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;

use crate::config::{self, Config, ViewMode, defaults};
use crate::fs::open;
use crate::model::History;
use crate::model::filter::FilterSpec;
use crate::model::sort::SortSpec;
use crate::theme::{Registry, StyleOptions, ThemeProvider};
use crate::ui::breadcrumb::Breadcrumb;
use crate::ui::debounce::Debouncer;
use crate::ui::file_pane::FilePane;
use crate::ui::keys;
use crate::ui::path_entry::PathEntry;
use crate::ui::sidebar::Sidebar;
use crate::ui::status_bar::StatusBar;

pub struct Window {
    window: adw::ApplicationWindow,
    file_pane: Rc<FilePane>,
    sidebar: Rc<Sidebar>,
    breadcrumb: Rc<Breadcrumb>,
    status: Rc<StatusBar>,
    banner: adw::Banner,
    toasts: adw::ToastOverlay,
    split: adw::OverlaySplitView,
    config: Rc<RefCell<Config>>,
    registry: Rc<Registry>,
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
}

impl Window {
    pub fn new(
        app: &adw::Application,
        config: Rc<RefCell<Config>>,
        registry: Rc<Registry>,
        theme: ThemeProvider,
    ) -> Rc<Self> {
        let (filter_spec, sort_spec, view_mode) = {
            let config = config.borrow();
            (
                FilterSpec::new(config.view.show_hidden, ""),
                SortSpec::new(
                    config.view.sort_key,
                    config.view.sort_order,
                    config.view.folders_first,
                ),
                config.view.mode,
            )
        };

        let file_pane = FilePane::new(filter_spec, sort_spec);
        file_pane.set_view_mode(view_mode);

        let sidebar = Sidebar::new();
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
            .sidebar(&sidebar.widget())
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
        });

        this.build_header(&header);
        this.wire_navigation();
        this.wire_search();
        this.wire_path_entry();
        this.wire_status();
        this.install_actions(app);
        this.apply_theme();

        this
    }

    pub fn widget(&self) -> &adw::ApplicationWindow {
        &self.window
    }

    pub fn present(&self) {
        self.window.present();
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
        let view_section = gio::Menu::new();
        view_section.append(Some("Show Hidden Files"), Some("win.toggle-hidden"));
        view_section.append(Some("Grid View"), Some("win.toggle-view"));
        menu.append_section(None, &view_section);

        let app_section = gio::Menu::new();
        app_section.append(Some("About Hive"), Some("win.about"));
        menu.append_section(None, &app_section);

        let menu_button = gtk::MenuButton::new();
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
            let where_it_was = self
                .file_pane
                .location()
                .and_then(|file| file.path())
                .map(|path| crate::model::path::display_name(&path))
                .unwrap_or_else(|| "That location".to_owned());

            self.show_banner(&format!(
                "{where_it_was} is no longer available — showing Home"
            ));
            self.navigate_home();
            self.show_banner(&format!(
                "{where_it_was} is no longer available — showing Home"
            ));
            return;
        }

        let message = if error.matches(gio::IOErrorEnum::PermissionDenied) {
            "Permission denied — you do not have access to this folder".to_owned()
        } else {
            format!("Could not read this folder: {}", error.message())
        };
        self.show_banner(&message);
    }

    pub fn show_banner(&self, message: &str) {
        self.banner.set_title(message);
        self.banner.set_revealed(true);
    }

    pub fn show_toast(&self, message: &str) {
        self.toasts.add_toast(adw::Toast::new(message));
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
        app.set_accels_for_action("win.select-all", &["<Control>a"]);
        app.set_accels_for_action("win.find", &["<Control>f"]);
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
    pub fn apply_theme(&self) {
        let config = self.config.borrow();
        let palette = self.registry.get_or_default(&config.appearance.flavor);
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

    /// Persist the config, reporting failures without interrupting the user.
    fn save_config(&self) {
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
