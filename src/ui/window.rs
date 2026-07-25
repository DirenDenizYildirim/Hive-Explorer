//! The main window: sidebar, header bar, file pane, status line.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;

use crate::config::{self, Config, ViewMode, defaults};
use crate::model::filter::FilterSpec;
use crate::model::sort::SortSpec;
use crate::theme::{Registry, StyleOptions, ThemeProvider};
use crate::ui::breadcrumb::Breadcrumb;
use crate::ui::debounce::Debouncer;
use crate::ui::file_pane::FilePane;
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

        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(breadcrumb.widget()));

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&toasts));

        let split = adw::OverlaySplitView::builder()
            .sidebar(&sidebar.widget())
            .content(&toolbar)
            .max_sidebar_width(f64::from(defaults::SIDEBAR_WIDTH_PX))
            .min_sidebar_width(200.0)
            .sidebar_width_fraction(0.22)
            .build();

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
        });

        this.build_header(&header);
        this.wire_navigation();
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

        let this = Rc::clone(self);
        self.file_pane.connect_activate(move |file, is_dir| {
            if is_dir {
                this.navigate_to(&file);
            }
        });

        let this = Rc::clone(self);
        self.file_pane.connect_error(move |error| {
            this.handle_enumeration_error(&error);
        });
    }

    /// Point the whole window at `file`.
    pub fn navigate_to(self: &Rc<Self>, file: &gio::File) {
        tracing::debug!(uri = %file.uri(), "navigating");

        self.banner.set_revealed(false);
        self.file_pane.set_location(file);
        self.breadcrumb.set_location(file);
        self.sidebar.sync_selection(Some(file));
        self.status.update_free_space_for(file);

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

        app.set_accels_for_action("win.toggle-hidden", &["<Control>h"]);
        app.set_accels_for_action("win.toggle-view", &["<Control>t"]);
        app.set_accels_for_action("win.toggle-sidebar", &["F9"]);
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

fn view_icon(mode: ViewMode) -> &'static str {
    match mode {
        ViewMode::List => "view-grid-symbolic",
        ViewMode::Grid => "view-list-symbolic",
    }
}
