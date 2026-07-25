//! Launching files with their handler application.

use adw::prelude::*;
use gtk::gdk;

/// Why a file could not be opened.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("nothing is registered to open {kind} files")]
    NoHandler { kind: String },
    #[error("{0}")]
    Launch(String),
}

/// Open `file` with its default handler.
///
/// Resolves through `gio::AppInfo`, which reads `.desktop` files directly and so
/// works with no desktop environment running. Falls back to `xdg-open` when no
/// handler is registered, since the user may have configured one by other means.
pub fn open(file: &gio::File, context: Option<&gdk::AppLaunchContext>) -> Result<(), OpenError> {
    let content_type = file
        .query_info(
            gio::FILE_ATTRIBUTE_STANDARD_CONTENT_TYPE,
            gio::FileQueryInfoFlags::NONE,
            gio::Cancellable::NONE,
        )
        .ok()
        .and_then(|info| info.content_type())
        .map(|c| c.to_string());

    if let Some(content_type) = content_type.as_deref()
        && let Some(app) = gio::AppInfo::default_for_type(content_type, false)
    {
        return app
            .launch(std::slice::from_ref(file), context)
            .map_err(|error| OpenError::Launch(error.message().to_owned()));
    }

    fall_back_to_xdg_open(file, content_type.as_deref())
}

/// Open `file` with a specific application, for the "Open With" menu.
pub fn open_with(
    file: &gio::File,
    app: &gio::AppInfo,
    context: Option<&gdk::AppLaunchContext>,
) -> Result<(), OpenError> {
    app.launch(std::slice::from_ref(file), context)
        .map_err(|error| OpenError::Launch(error.message().to_owned()))
}

/// Applications registered for `file`'s content type, best match first.
pub fn handlers_for(file: &gio::File) -> Vec<gio::AppInfo> {
    let Ok(info) = file.query_info(
        gio::FILE_ATTRIBUTE_STANDARD_CONTENT_TYPE,
        gio::FileQueryInfoFlags::NONE,
        gio::Cancellable::NONE,
    ) else {
        return Vec::new();
    };

    let Some(content_type) = info.content_type() else {
        return Vec::new();
    };

    gio::AppInfo::recommended_for_type(&content_type)
        .into_iter()
        .chain(gio::AppInfo::fallback_for_type(&content_type))
        .fold(Vec::new(), |mut apps: Vec<gio::AppInfo>, app| {
            let already = apps
                .iter()
                .any(|existing| existing.id().is_some() && existing.id() == app.id());
            if !already {
                apps.push(app);
            }
            apps
        })
}

fn fall_back_to_xdg_open(file: &gio::File, content_type: Option<&str>) -> Result<(), OpenError> {
    let uri = file.uri();
    tracing::debug!(uri = %uri, "no registered handler; trying xdg-open");

    match std::process::Command::new("xdg-open")
        .arg(uri.as_str())
        .spawn()
    {
        Ok(_) => Ok(()),
        Err(error) => {
            tracing::warn!(%error, "xdg-open failed");
            Err(OpenError::NoHandler {
                kind: content_type
                    .map(describe)
                    .unwrap_or_else(|| "this kind of".to_owned()),
            })
        }
    }
}

/// A human-readable name for a content type, for error messages.
pub fn describe(content_type: &str) -> String {
    let description = gio::functions::content_type_get_description(content_type);
    if description.is_empty() {
        content_type.to_owned()
    } else {
        description.to_string()
    }
}
