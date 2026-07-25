//! The Appearance dialog: flavor, accent, follow-system, window chrome.
//!
//! Every control writes straight to the config and re-applies the stylesheet.
//! There is no OK button and nothing to confirm, because applying is the only
//! way to see whether a theme is the one you wanted.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;

use crate::theme::palette::Accent;
use crate::theme::system::{self, SystemScheme};
use crate::ui::window::Window;

/// Open the dialog over `window`.
pub fn present(window: &Rc<Window>) {
    Editor::build(window).present();
}

/// One theme, reduced to what the switcher needs.
#[derive(Clone)]
struct Entry {
    id: String,
    name: String,
    dark: bool,
    built_in: bool,
}

struct Editor {
    window: Rc<Window>,
    dialog: adw::PreferencesDialog,
    flavor: adw::ComboRow,
    follow: adw::SwitchRow,
    light: adw::ComboRow,
    dark: adw::ComboRow,
    themes_row: adw::ActionRow,
    swatches: Vec<gtk::ToggleButton>,
    entries: RefCell<Vec<Entry>>,
    /// True while the dialog is writing its own widgets, so the `notify`
    /// handlers do not treat that as the user changing something.
    updating: Cell<bool>,
}

impl Editor {
    fn build(window: &Rc<Window>) -> Rc<Self> {
        let dialog = adw::PreferencesDialog::new();
        dialog.set_title("Appearance");
        dialog.set_search_enabled(false);

        let page = adw::PreferencesPage::new();

        let (theme_group, flavor) = theme_group();
        let (accent_group, swatches) = accent_group();
        let (system_group, follow, light, dark) = system_group();
        let (chrome_group, rounding, shadow) = chrome_group(window);
        let (custom_group, themes_row, open_button, reload_button) = custom_group();

        page.add(&theme_group);
        page.add(&accent_group);
        page.add(&system_group);
        page.add(&chrome_group);
        page.add(&custom_group);
        dialog.add(&page);

        let editor = Rc::new(Self {
            window: Rc::clone(window),
            dialog,
            flavor,
            follow,
            light,
            dark,
            themes_row,
            swatches,
            entries: RefCell::new(Vec::new()),
            updating: Cell::new(true),
        });

        editor.wire_flavor();
        editor.wire_accent();
        editor.wire_system();
        editor.wire_chrome(&rounding, &shadow);
        editor.wire_custom(&open_button, &reload_button);

        editor.refresh();
        editor
    }

    fn present(self: &Rc<Self>) {
        self.dialog.present(Some(self.window.widget()));
    }

    /// Rebuild every model and selection from the current config and registry.
    fn refresh(self: &Rc<Self>) {
        // Cannot be done in `build` because the entries come from the registry,
        // which reload replaces underneath us.
        let entries = self.read_entries();

        let guard = Guard::new(&self.updating);

        let all = gtk::StringList::new(&[]);
        let lights = gtk::StringList::new(&[]);
        let darks = gtk::StringList::new(&[]);
        for entry in &entries {
            all.append(&entry.name);
            if entry.dark {
                darks.append(&entry.name);
            } else {
                lights.append(&entry.name);
            }
        }

        self.flavor.set_model(Some(&all));
        self.light.set_model(Some(&lights));
        self.dark.set_model(Some(&darks));

        let (flavor, follow, light, dark, accent) = {
            let config = self.window.config.borrow();
            (
                config.appearance.flavor.clone(),
                config.appearance.follow_system,
                config.appearance.light_flavor.clone(),
                config.appearance.dark_flavor.clone(),
                config.appearance.accent,
            )
        };

        select(&self.flavor, &entries, |e| e.id == flavor, |_| true);
        select(&self.light, &entries, |e| e.id == light, |e| !e.dark);
        select(&self.dark, &entries, |e| e.id == dark, |e| e.dark);

        self.follow.set_active(follow);
        for swatch in &self.swatches {
            let is_current = swatch.widget_name() == accent.id();
            swatch.set_active(is_current);
        }

        // Stored after the widgets, so no handler can read a list that no
        // longer matches the models it is indexing into.
        *self.entries.borrow_mut() = entries;
        drop(guard);

        self.sync_sensitivity();
        self.sync_subtitles();
    }

    fn read_entries(&self) -> Vec<Entry> {
        let registry = self.window.registry.borrow();
        let built_in = crate::theme::Registry::built_in().ids();

        registry
            .all()
            .iter()
            .map(|palette| Entry {
                id: palette.id.to_string(),
                name: palette.name.to_string(),
                dark: palette.dark,
                built_in: built_in.contains(&palette.id),
            })
            .collect()
    }

    /// The theme id a combo row is currently pointing at.
    fn selected_id(&self, row: &adw::ComboRow, filter: impl Fn(&Entry) -> bool) -> Option<String> {
        let index = row.selected() as usize;
        self.entries
            .borrow()
            .iter()
            .filter(|entry| filter(entry))
            .nth(index)
            .map(|entry| entry.id.clone())
    }

    // ---- wiring ----------------------------------------------------------

    fn wire_flavor(self: &Rc<Self>) {
        let this = Rc::clone(self);
        self.flavor.connect_selected_notify(move |row| {
            if this.updating.get() {
                return;
            }
            let Some(id) = this.selected_id(row, |_| true) else {
                return;
            };
            this.window.config.borrow_mut().appearance.flavor = id;
            this.apply();
        });
    }

    fn wire_accent(self: &Rc<Self>) {
        for swatch in &self.swatches {
            let this = Rc::clone(self);
            swatch.connect_toggled(move |button| {
                if this.updating.get() {
                    return;
                }
                let Some(accent) = Accent::from_id(&button.widget_name()) else {
                    return;
                };

                // A grouped toggle button can be switched off as well as on.
                // There is always exactly one accent, so put it back rather
                // than leaving the picker showing nothing selected.
                if !button.is_active() {
                    if this.window.config.borrow().appearance.accent == accent {
                        let guard = Guard::new(&this.updating);
                        button.set_active(true);
                        drop(guard);
                    }
                    return;
                }

                this.window.config.borrow_mut().appearance.accent = accent;
                this.apply();
            });
        }
    }

    fn wire_system(self: &Rc<Self>) {
        let this = Rc::clone(self);
        self.follow.connect_active_notify(move |row| {
            if this.updating.get() {
                return;
            }
            this.window.config.borrow_mut().appearance.follow_system = row.is_active();
            this.sync_sensitivity();
            this.apply();
        });

        let this = Rc::clone(self);
        self.light.connect_selected_notify(move |row| {
            if this.updating.get() {
                return;
            }
            let Some(id) = this.selected_id(row, |entry| !entry.dark) else {
                return;
            };
            this.window.config.borrow_mut().appearance.light_flavor = id;
            this.apply();
        });

        let this = Rc::clone(self);
        self.dark.connect_selected_notify(move |row| {
            if this.updating.get() {
                return;
            }
            let Some(id) = this.selected_id(row, |entry| entry.dark) else {
                return;
            };
            this.window.config.borrow_mut().appearance.dark_flavor = id;
            this.apply();
        });
    }

    fn wire_chrome(self: &Rc<Self>, rounding: &adw::SwitchRow, shadow: &adw::SwitchRow) {
        let this = Rc::clone(self);
        rounding.connect_active_notify(move |row| {
            if this.updating.get() {
                return;
            }
            this.window
                .config
                .borrow_mut()
                .appearance
                .client_side_rounding = row.is_active();
            this.apply();
        });

        let this = Rc::clone(self);
        shadow.connect_active_notify(move |row| {
            if this.updating.get() {
                return;
            }
            this.window
                .config
                .borrow_mut()
                .appearance
                .client_side_shadow = row.is_active();
            this.apply();
        });
    }

    fn wire_custom(self: &Rc<Self>, open: &gtk::Button, reload: &gtk::Button) {
        let this = Rc::clone(self);
        open.connect_clicked(move |_| {
            let directory = crate::paths::themes_dir();
            if let Err(error) = std::fs::create_dir_all(&directory) {
                tracing::warn!(%error, path = %directory.display(), "could not create themes dir");
                this.window.show_toast("Could not open the themes folder");
                return;
            }
            this.dialog.close();
            this.window.navigate_to_path(&directory);
        });

        let this = Rc::clone(self);
        reload.connect_clicked(move |_| {
            let count = this.window.reload_themes();
            this.refresh();
            this.window
                .show_toast(&format!("Reloaded — {count} themes available"));
        });
    }

    // ---- applying --------------------------------------------------------

    fn apply(self: &Rc<Self>) {
        self.window.apply_theme();
        self.window.save_config();
        self.window.rebuild_flavor_menu();
        self.sync_subtitles();
    }

    /// Following the system takes the explicit flavor out of play, so say so
    /// rather than leaving a control that silently does nothing.
    fn sync_sensitivity(self: &Rc<Self>) {
        let following = self.follow.is_active();
        self.flavor.set_sensitive(!following);
        self.light.set_sensitive(following);
        self.dark.set_sensitive(following);
    }

    fn sync_subtitles(self: &Rc<Self>) {
        let active = self.window.active_theme_id();
        let entries = self.entries.borrow();
        let custom = entries
            .iter()
            .find(|entry| entry.id == active)
            .is_some_and(|entry| !entry.built_in);

        self.flavor.set_subtitle(&if custom {
            format!("{active} — from your themes folder")
        } else {
            active.clone()
        });

        self.follow.set_subtitle(match system::current() {
            SystemScheme::Dark => "The desktop reports: dark",
            SystemScheme::Light => "The desktop reports: light",
            SystemScheme::Unknown => "No desktop preference available — your chosen flavor is used",
        });

        let count = entries.len();
        let custom_count = entries.iter().filter(|entry| !entry.built_in).count();
        self.themes_row.set_subtitle(&format!(
            "{} — {count} available, {custom_count} of them yours",
            crate::paths::themes_dir().display()
        ));
    }
}

/// Restores the update guard even if a handler returns early.
struct Guard<'a>(&'a Cell<bool>);

impl<'a> Guard<'a> {
    fn new(cell: &'a Cell<bool>) -> Self {
        cell.set(true);
        Self(cell)
    }
}

impl Drop for Guard<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

/// Point a combo row at the entry matching `wanted`, among those passing `filter`.
fn select(
    row: &adw::ComboRow,
    entries: &[Entry],
    wanted: impl Fn(&Entry) -> bool,
    filter: impl Fn(&Entry) -> bool,
) {
    let index = entries
        .iter()
        .filter(|e| filter(e))
        .position(wanted)
        .unwrap_or(0);
    row.set_selected(index as u32);
}

// ---- widget construction -------------------------------------------------

fn theme_group() -> (adw::PreferencesGroup, adw::ComboRow) {
    let group = adw::PreferencesGroup::new();
    group.set_title("Theme");

    let flavor = adw::ComboRow::new();
    flavor.set_title("Flavor");
    group.add(&flavor);

    (group, flavor)
}

fn accent_group() -> (adw::PreferencesGroup, Vec<gtk::ToggleButton>) {
    let group = adw::PreferencesGroup::new();
    group.set_title("Accent");
    group.set_description(Some("Selection, focus rings, and the active sidebar row."));

    // A grid rather than a flow box: a `GtkFlowBoxChild` takes the focus for
    // its child, so Tab would land on the wrapper and Space would do nothing.
    // Seven columns of 24 px still fit the 500 px floor.
    const COLUMNS: i32 = 7;
    let grid = gtk::Grid::builder()
        .row_spacing(10)
        .column_spacing(10)
        .halign(gtk::Align::Center)
        .margin_top(10)
        .margin_bottom(10)
        .build();

    let mut swatches = Vec::with_capacity(Accent::ALL.len());
    let mut group_leader: Option<gtk::ToggleButton> = None;

    for (index, accent) in Accent::ALL.into_iter().enumerate() {
        let button = gtk::ToggleButton::new();
        button.set_widget_name(accent.id());
        button.set_tooltip_text(Some(accent.display_name()));
        button.add_css_class("hive-swatch");
        button.add_css_class(&format!("hive-accent-{}", accent.id()));

        match &group_leader {
            Some(leader) => button.set_group(Some(leader)),
            None => group_leader = Some(button.clone()),
        }

        let index = i32::try_from(index).unwrap_or(0);
        grid.attach(&button, index % COLUMNS, index / COLUMNS, 1, 1);
        swatches.push(button);
    }

    // The row is only a container. Left focusable it takes the tab stop for
    // itself and the swatches inside become unreachable from the keyboard.
    let row = adw::PreferencesRow::new();
    row.set_activatable(false);
    row.set_focusable(false);
    row.set_child(Some(&grid));
    group.add(&row);

    (group, swatches)
}

fn system_group() -> (
    adw::PreferencesGroup,
    adw::SwitchRow,
    adw::ComboRow,
    adw::ComboRow,
) {
    let group = adw::PreferencesGroup::new();
    group.set_title("Follow the system");

    let follow = adw::SwitchRow::new();
    follow.set_title("Match the desktop's light or dark setting");
    group.add(&follow);

    let light = adw::ComboRow::new();
    light.set_title("When light");
    group.add(&light);

    let dark = adw::ComboRow::new();
    dark.set_title("When dark");
    group.add(&dark);

    (group, follow, light, dark)
}

fn chrome_group(window: &Rc<Window>) -> (adw::PreferencesGroup, adw::SwitchRow, adw::SwitchRow) {
    let group = adw::PreferencesGroup::new();
    group.set_title("Window");
    group.set_description(Some(
        "Hyprland already rounds and shadows windows. Turning these on draws a second one.",
    ));

    let (rounding_on, shadow_on) = {
        let config = window.config.borrow();
        (
            config.appearance.client_side_rounding,
            config.appearance.client_side_shadow,
        )
    };

    let rounding = adw::SwitchRow::new();
    rounding.set_title("Rounded corners");
    rounding.set_active(rounding_on);
    group.add(&rounding);

    let shadow = adw::SwitchRow::new();
    shadow.set_title("Drop shadow");
    shadow.set_active(shadow_on);
    group.add(&shadow);

    (group, rounding, shadow)
}

fn custom_group() -> (
    adw::PreferencesGroup,
    adw::ActionRow,
    gtk::Button,
    gtk::Button,
) {
    let group = adw::PreferencesGroup::new();
    group.set_title("Your own themes");
    group.set_description(Some(
        "Drop a .toml file in the themes folder and it appears in the list. \
         A theme whose id matches a built-in replaces it.",
    ));

    let row = adw::ActionRow::new();
    row.set_title("Themes folder");
    row.set_subtitle_lines(2);

    let open = gtk::Button::with_label("Open");
    open.set_valign(gtk::Align::Center);
    open.add_css_class("flat");
    row.add_suffix(&open);

    let reload = gtk::Button::with_label("Reload");
    reload.set_valign(gtk::Align::Center);
    reload.add_css_class("flat");
    row.add_suffix(&reload);

    group.add(&row);

    (group, row, open, reload)
}
