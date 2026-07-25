//! Following the system light/dark preference, best effort.
//!
//! Read from the freedesktop appearance portal rather than from libadwaita's
//! `StyleManager`. `StyleManager::is_dark` reports the *effective* appearance,
//! which Hive itself forces to match the active palette — asking it what the
//! system wants would only echo back what Hive just told it. The portal answers
//! independently, and answering "no preference" is a real answer that means
//! Hive should keep its own choice.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gio::prelude::*;

const PORTAL_BUS: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SETTINGS_INTERFACE: &str = "org.freedesktop.portal.Settings";
const NAMESPACE: &str = "org.freedesktop.appearance";
const KEY: &str = "color-scheme";

/// The portal call is fire-and-forget; this only bounds a hung portal.
const TIMEOUT_MS: i32 = 3_000;

/// What the system says about light and dark, when it says anything at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SystemScheme {
    /// No portal, no answer yet, or an explicit "no preference".
    #[default]
    Unknown,
    Light,
    Dark,
}

impl SystemScheme {
    /// Map the portal's `color-scheme` value.
    ///
    /// `0` is "no preference", which is not the same as light: it means nothing
    /// has an opinion, so Hive should keep its own rather than invent one.
    pub const fn from_portal(value: u32) -> Self {
        match value {
            1 => SystemScheme::Dark,
            2 => SystemScheme::Light,
            _ => SystemScheme::Unknown,
        }
    }
}

/// What the user asked for, independent of what the system says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preference<'a> {
    pub flavor: &'a str,
    pub follow_system: bool,
    pub light_flavor: &'a str,
    pub dark_flavor: &'a str,
}

/// The theme id that should be active.
///
/// An explicit flavor always wins and always works: following the system is
/// opt-in, and even then a portal that says nothing leaves the explicit choice
/// in place rather than guessing light.
pub fn resolve<'a>(preference: &Preference<'a>, system: SystemScheme) -> &'a str {
    if !preference.follow_system {
        return preference.flavor;
    }

    match system {
        SystemScheme::Dark => preference.dark_flavor,
        SystemScheme::Light => preference.light_flavor,
        SystemScheme::Unknown => preference.flavor,
    }
}

type Listener = Rc<dyn Fn(SystemScheme)>;

thread_local! {
    /// Last value the portal reported, on the main thread.
    static CURRENT: Cell<SystemScheme> = const { Cell::new(SystemScheme::Unknown) };
    static LISTENERS: RefCell<Vec<Listener>> = const { RefCell::new(Vec::new()) };
    static STARTED: Cell<bool> = const { Cell::new(false) };
}

/// The most recent answer, or `Unknown` before one arrives.
pub fn current() -> SystemScheme {
    CURRENT.with(Cell::get)
}

/// Run `on_change` whenever the system preference changes.
///
/// Starting the portal conversation is deferred to an async task, so a portal
/// that never answers costs nothing but stays `Unknown`. Startup never waits on
/// it and never fails because of it.
pub fn watch(on_change: impl Fn(SystemScheme) + 'static) {
    LISTENERS.with_borrow_mut(|listeners| listeners.push(Rc::new(on_change)));

    if STARTED.with(|started| started.replace(true)) {
        return;
    }
    glib::spawn_future_local(async move { connect().await });
}

fn publish(scheme: SystemScheme) {
    if CURRENT.with(|current| current.replace(scheme)) == scheme {
        return;
    }
    tracing::debug!(?scheme, "system colour scheme");

    let listeners = LISTENERS.with_borrow(Clone::clone);
    for listener in listeners {
        listener(scheme);
    }
}

async fn connect() {
    let proxy = gio::DBusProxy::for_bus_future(
        gio::BusType::Session,
        gio::DBusProxyFlags::DO_NOT_LOAD_PROPERTIES | gio::DBusProxyFlags::DO_NOT_AUTO_START,
        None,
        PORTAL_BUS,
        PORTAL_PATH,
        SETTINGS_INTERFACE,
    )
    .await;

    let proxy = match proxy {
        Ok(proxy) => proxy,
        Err(error) => {
            // No portal on this machine. Not an error worth showing anyone.
            tracing::debug!(%error, "appearance portal unavailable; not following the system");
            return;
        }
    };

    // The proxy delivers on the thread-default context, which is the main one,
    // but the binding still demands a Send closure — so hop through an idle
    // callback to reach the listeners.
    proxy.connect_g_signal(|_, _, signal, parameters| {
        if signal != "SettingChanged" {
            return;
        }
        let Some((namespace, key, value)) = parameters.get::<(String, String, glib::Variant)>()
        else {
            return;
        };
        if namespace != NAMESPACE || key != KEY {
            return;
        }
        if let Some(scheme) = unwrap_scheme(&value) {
            glib::idle_add_once(move || publish(scheme));
        }
    });

    let read = proxy
        .call_future(
            "Read",
            Some(&(NAMESPACE, KEY).to_variant()),
            gio::DBusCallFlags::NONE,
            TIMEOUT_MS,
        )
        .await;

    match read {
        Ok(reply) => match reply.child_value(0).pipe_scheme() {
            Some(scheme) => {
                // Logged even when nothing changes, so `--verbose` shows that the
                // portal was reached at all rather than leaving silence to mean
                // both "no preference" and "never answered".
                tracing::debug!(?scheme, "appearance portal answered");
                publish(scheme);
            }
            None => tracing::debug!("appearance portal returned an unexpected shape"),
        },
        Err(error) => {
            tracing::debug!(%error, "appearance portal did not answer");
        }
    }
}

/// Peel however many layers of variant the portal wrapped the value in.
fn unwrap_scheme(value: &glib::Variant) -> Option<SystemScheme> {
    let mut current = value.clone();
    for _ in 0..4 {
        if let Some(number) = current.get::<u32>() {
            return Some(SystemScheme::from_portal(number));
        }
        current = current.as_variant()?;
    }
    None
}

/// Small helper so the call site above reads in one direction.
trait PipeScheme {
    fn pipe_scheme(&self) -> Option<SystemScheme>;
}

impl PipeScheme for glib::Variant {
    fn pipe_scheme(&self) -> Option<SystemScheme> {
        unwrap_scheme(self)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn preference(follow_system: bool) -> Preference<'static> {
        Preference {
            flavor: "mocha",
            follow_system,
            light_flavor: "latte",
            dark_flavor: "macchiato",
        }
    }

    #[test]
    fn an_explicit_flavor_wins_whatever_the_system_says() {
        for system in [
            SystemScheme::Unknown,
            SystemScheme::Light,
            SystemScheme::Dark,
        ] {
            assert_eq!(resolve(&preference(false), system), "mocha");
        }
    }

    #[test]
    fn following_the_system_picks_the_matching_flavor() {
        assert_eq!(resolve(&preference(true), SystemScheme::Dark), "macchiato");
        assert_eq!(resolve(&preference(true), SystemScheme::Light), "latte");
    }

    #[test]
    fn a_silent_portal_leaves_the_configured_flavor_in_place() {
        // Not the light flavor: "no answer" is not "prefer light", and guessing
        // would flip the theme on every machine without a portal.
        assert_eq!(resolve(&preference(true), SystemScheme::Unknown), "mocha");
    }

    #[test]
    fn the_portal_value_zero_means_no_preference_not_light() {
        assert_eq!(SystemScheme::from_portal(0), SystemScheme::Unknown);
        assert_eq!(SystemScheme::from_portal(1), SystemScheme::Dark);
        assert_eq!(SystemScheme::from_portal(2), SystemScheme::Light);
    }

    #[test]
    fn an_unknown_portal_value_is_treated_as_no_preference() {
        for value in [3u32, 7, 99, u32::MAX] {
            assert_eq!(SystemScheme::from_portal(value), SystemScheme::Unknown);
        }
    }

    #[test]
    fn a_nested_variant_is_unwrapped_to_a_scheme() {
        // The portal's Read returns a variant wrapping a variant wrapping the
        // value, and implementations vary in how many layers they add.
        let bare = 1u32.to_variant();
        assert_eq!(unwrap_scheme(&bare), Some(SystemScheme::Dark));

        let once = glib::Variant::from_variant(&2u32.to_variant());
        assert_eq!(unwrap_scheme(&once), Some(SystemScheme::Light));

        let twice = glib::Variant::from_variant(&once);
        assert_eq!(unwrap_scheme(&twice), Some(SystemScheme::Light));
    }

    #[test]
    fn a_value_of_the_wrong_type_is_not_a_scheme() {
        assert_eq!(unwrap_scheme(&"prefer-dark".to_variant()), None);
        assert_eq!(unwrap_scheme(&true.to_variant()), None);
    }

    #[test]
    fn the_default_scheme_is_unknown() {
        assert_eq!(SystemScheme::default(), SystemScheme::Unknown);
    }
}
