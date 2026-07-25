//! Clickable path breadcrumb for the header bar.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;

type NavigateHandler = Rc<dyn Fn(gio::File)>;

/// Segments rendered inline before the trail starts collapsing.
const MAX_INLINE_SEGMENTS: usize = 5;

/// How many trailing segments always stay visible when the trail collapses.
const TAIL_SEGMENTS: usize = 3;

/// Characters shown per ancestor segment before it ellipsizes.
const ANCESTOR_WIDTH_CHARS: i32 = 10;

/// The current directory gets more room than its ancestors.
const CURRENT_WIDTH_CHARS: i32 = 24;

pub struct Breadcrumb {
    container: gtk::Box,
    viewport: gtk::ScrolledWindow,
    on_navigate: RefCell<Option<NavigateHandler>>,
}

impl Breadcrumb {
    pub fn new() -> Rc<Self> {
        let container = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        container.add_css_class("hive-breadcrumb");
        container.set_valign(gtk::Align::Center);

        // The trail must never dictate how narrow the window can be. Collapsing
        // and ellipsizing bound how *wide* it gets, but a Box still reports the
        // sum of its children as its minimum, and through the header bar that
        // becomes the whole window's minimum — which a tiling compositor is
        // under no obligation to honour. A viewport with
        // propagate_natural_width(false) reports ~0 instead.
        let viewport = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::External)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_width(false)
            .hexpand(true)
            .child(&container)
            .build();

        Rc::new(Self {
            container,
            viewport,
            on_navigate: RefCell::new(None),
        })
    }

    pub fn widget(&self) -> &gtk::ScrolledWindow {
        &self.viewport
    }

    pub fn connect_navigate(self: &Rc<Self>, handler: impl Fn(gio::File) + 'static) {
        *self.on_navigate.borrow_mut() = Some(Rc::new(handler));
    }

    /// Rebuild the trail for `location`.
    pub fn set_location(self: &Rc<Self>, location: &gio::File) {
        self.clear();

        let Some(path) = location.path() else {
            let label = gtk::Label::new(Some(&location.uri()));
            label.add_css_class("hive-breadcrumb-current");
            self.container.append(&label);
            return;
        };

        let home = crate::paths::home_dir();
        let segments = segments_for(&path, &home);
        let plan = collapse(&segments, MAX_INLINE_SEGMENTS, TAIL_SEGMENTS);
        let last = segments.len().saturating_sub(1);

        let root_is_slash = segments.first().is_some_and(|s| s.label == "/");

        for (position, step) in plan.into_iter().enumerate() {
            let after_root_slash = position == 1 && root_is_slash;
            if position > 0 && !after_root_slash {
                let separator = gtk::Label::new(Some("/"));
                separator.add_css_class("hive-breadcrumb-separator");
                self.container.append(&separator);
            }

            match step {
                Step::Segment(index) => {
                    let segment = &segments[index];
                    let button = gtk::Button::with_label(&segment.label);
                    button.add_css_class("flat");
                    let is_current = index == last;
                    if is_current {
                        button.add_css_class("hive-breadcrumb-current");
                    }

                    if let Some(label) =
                        button.child().and_then(|c| c.downcast::<gtk::Label>().ok())
                    {
                        // Ellipsizing is what lets the trail shrink: a plain
                        // label reports its full text as its minimum width,
                        // which propagates up and makes the split view demand
                        // more room than a tiled window may be given.
                        label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                        label.set_max_width_chars(if is_current {
                            CURRENT_WIDTH_CHARS
                        } else {
                            ANCESTOR_WIDTH_CHARS
                        });
                        label.set_tooltip_text(Some(&segment.label));
                    }

                    let target = gio::File::for_path(&segment.path);
                    let breadcrumb = Rc::clone(self);
                    button.connect_clicked(move |_| {
                        breadcrumb.emit(&target);
                    });

                    self.container.append(&button);
                }
                Step::Overflow(hidden) => {
                    self.container
                        .append(&self.overflow_button(&segments, hidden));
                }
            }
        }

        self.scroll_to_current();
    }

    /// Keep the directory the user is in visible, clipping ancestors instead.
    ///
    /// Deferred, because the adjustment's upper bound is not known until the
    /// new buttons have been allocated. Also called when the breadcrumb comes
    /// back from behind the path entry, which resets the scroll position.
    pub fn scroll_to_current(&self) {
        let adjustment = self.viewport.hadjustment();
        glib::idle_add_local_once(move || {
            adjustment.set_value(adjustment.upper());
        });
    }

    /// The `…` button standing in for collapsed middle segments.
    fn overflow_button(
        self: &Rc<Self>,
        segments: &[Segment],
        hidden: std::ops::Range<usize>,
    ) -> gtk::MenuButton {
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let popover = gtk::Popover::new();
        popover.set_child(Some(&list));

        for index in hidden {
            let segment = &segments[index];
            let entry = gtk::Button::with_label(&segment.label);
            entry.add_css_class("flat");
            if let Some(label) = entry.child().and_then(|c| c.downcast::<gtk::Label>().ok()) {
                label.set_xalign(0.0);
            }

            let target = gio::File::for_path(&segment.path);
            let breadcrumb = Rc::clone(self);
            let popover = popover.clone();
            entry.connect_clicked(move |_| {
                popover.popdown();
                breadcrumb.emit(&target);
            });
            list.append(&entry);
        }

        let button = gtk::MenuButton::new();
        button.set_label("…");
        button.add_css_class("flat");
        button.set_tooltip_text(Some("Show the rest of the path"));
        button.set_popover(Some(&popover));
        button
    }

    fn emit(&self, target: &gio::File) {
        let handler = self.on_navigate.borrow().clone();
        if let Some(handler) = handler {
            handler(target.clone());
        }
    }

    fn clear(&self) {
        while let Some(child) = self.container.first_child() {
            self.container.remove(&child);
        }
    }
}

/// One clickable step in the trail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub label: String,
    pub path: PathBuf,
}

/// What to render at one position in the trail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// An index into the segment list.
    Segment(usize),
    /// A collapsed run, rendered as the `…` overflow button.
    Overflow(std::ops::Range<usize>),
}

/// Decide which segments render inline and which fold into the overflow button.
pub fn collapse(segments: &[Segment], max_inline: usize, tail: usize) -> Vec<Step> {
    if segments.len() <= max_inline {
        return (0..segments.len()).map(Step::Segment).collect();
    }

    let tail_start = segments.len() - tail;
    let mut steps = Vec::with_capacity(tail + 2);
    steps.push(Step::Segment(0));
    steps.push(Step::Overflow(1..tail_start));
    steps.extend((tail_start..segments.len()).map(Step::Segment));
    steps
}

/// Build the trail for `path`, collapsing the home prefix to a single "Home".
pub fn segments_for(path: &Path, home: &Path) -> Vec<Segment> {
    let normalized = crate::model::path::normalize(path);

    if crate::model::path::is_ancestor(home, &normalized) {
        let mut segments = vec![Segment {
            label: "Home".to_owned(),
            path: home.to_path_buf(),
        }];

        if let Ok(relative) = normalized.strip_prefix(home) {
            let mut running = home.to_path_buf();
            for component in relative.components() {
                running.push(component.as_os_str());
                segments.push(Segment {
                    label: component.as_os_str().to_string_lossy().into_owned(),
                    path: running.clone(),
                });
            }
        }
        return segments;
    }

    crate::model::path::breadcrumb_segments(&normalized)
        .into_iter()
        .map(|p| Segment {
            label: if p == Path::new("/") {
                "/".to_owned()
            } else {
                crate::model::path::display_name(&p)
            },
            path: p,
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn home_collapses_to_a_single_segment() {
        let home = Path::new("/home/diren");
        let segments = segments_for(Path::new("/home/diren/Downloads/photos"), home);

        let labels: Vec<&str> = segments.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, ["Home", "Downloads", "photos"]);

        assert_eq!(segments[0].path, PathBuf::from("/home/diren"));
        assert_eq!(segments[1].path, PathBuf::from("/home/diren/Downloads"));
        assert_eq!(
            segments[2].path,
            PathBuf::from("/home/diren/Downloads/photos")
        );
    }

    #[test]
    fn home_itself_is_one_segment() {
        let home = Path::new("/home/diren");
        let segments = segments_for(home, home);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].label, "Home");
        assert_eq!(segments[0].path, home);
    }

    #[test]
    fn paths_outside_home_show_the_full_trail() {
        let home = Path::new("/home/diren");
        let segments = segments_for(Path::new("/usr/share/icons"), home);

        let labels: Vec<&str> = segments.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, ["/", "usr", "share", "icons"]);
        assert_eq!(segments[0].path, PathBuf::from("/"));
        assert_eq!(segments[3].path, PathBuf::from("/usr/share/icons"));
    }

    #[test]
    fn the_root_is_a_single_segment() {
        let segments = segments_for(Path::new("/"), Path::new("/home/diren"));
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].label, "/");
    }

    #[test]
    fn a_sibling_of_home_does_not_collapse() {
        let home = Path::new("/home/diren");
        let segments = segments_for(Path::new("/home/di/work"), home);
        let labels: Vec<&str> = segments.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, ["/", "home", "di", "work"]);
    }

    #[test]
    fn dot_components_are_normalized_away() {
        let home = Path::new("/home/diren");
        let segments = segments_for(Path::new("/home/diren/./Downloads/../Downloads"), home);
        let labels: Vec<&str> = segments.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, ["Home", "Downloads"]);
    }

    fn seg(label: &str) -> Segment {
        Segment {
            label: label.to_owned(),
            path: PathBuf::from(label),
        }
    }

    #[test]
    fn short_trails_render_every_segment_inline() {
        let segments: Vec<Segment> = ["Home", "a", "b"].iter().map(|s| seg(s)).collect();
        let plan = collapse(&segments, 5, 3);
        assert_eq!(
            plan,
            vec![Step::Segment(0), Step::Segment(1), Step::Segment(2)]
        );
    }

    #[test]
    fn a_trail_exactly_at_the_limit_does_not_collapse() {
        let segments: Vec<Segment> = ["Home", "a", "b", "c", "d"]
            .iter()
            .map(|s| seg(s))
            .collect();
        let plan = collapse(&segments, 5, 3);
        assert_eq!(plan.len(), 5);
        assert!(!plan.iter().any(|s| matches!(s, Step::Overflow(_))));
    }

    #[test]
    fn deep_trails_collapse_the_middle_and_keep_the_ends() {
        let segments: Vec<Segment> = ["Home", "a", "b", "c", "d", "e", "f"]
            .iter()
            .map(|s| seg(s))
            .collect();
        let plan = collapse(&segments, 5, 3);
        assert_eq!(
            plan,
            vec![
                Step::Segment(0),
                Step::Overflow(1..4),
                Step::Segment(4),
                Step::Segment(5),
                Step::Segment(6),
            ]
        );
    }

    #[test]
    fn no_segment_becomes_unreachable_when_collapsed() {
        let segments: Vec<Segment> = (0..40).map(|i| seg(&format!("level{i}"))).collect();
        let plan = collapse(&segments, 5, 3);

        let mut covered: Vec<usize> = Vec::new();
        for step in &plan {
            match step {
                Step::Segment(index) => covered.push(*index),
                Step::Overflow(range) => covered.extend(range.clone()),
            }
        }
        covered.sort_unstable();
        assert_eq!(covered, (0..40).collect::<Vec<usize>>());
    }

    #[test]
    fn the_first_segment_always_stays_inline() {
        let segments: Vec<Segment> = (0..20).map(|i| seg(&format!("l{i}"))).collect();
        let plan = collapse(&segments, 5, 3);
        assert_eq!(plan.first(), Some(&Step::Segment(0)));
    }

    #[test]
    fn names_with_odd_characters_still_produce_segments() {
        let home = Path::new("/home/diren");
        let segments = segments_for(Path::new("/home/diren/two\nlines"), home);
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[1].label, "two\nlines");
    }
}
