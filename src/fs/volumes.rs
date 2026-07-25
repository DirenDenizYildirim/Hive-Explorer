//! Which mounts belong in the sidebar's Devices section.

use std::path::Path;

/// What the sidebar needs to know about a candidate mount.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MountCandidate<'a> {
    /// Mount point. `None` for a volume that is not currently mounted.
    pub mount_point: Option<&'a Path>,
    /// The filesystem type, when the backend reports one.
    pub filesystem: Option<&'a str>,
    /// Backend says the media can be removed.
    pub is_removable: bool,
    /// Backend offers an eject action.
    pub can_eject: bool,
    /// Backend offers an unmount action.
    pub can_unmount: bool,
    /// GIO considers this a shadowed mount (superseded by another).
    pub is_shadowed: bool,
}

/// Mount points that are part of the OS rather than something the user browses.
const SYSTEM_MOUNT_POINTS: [&str; 5] = ["/", "/boot", "/boot/efi", "/efi", "/usr"];

/// Directory prefixes that only ever hold kernel and runtime plumbing.
const SYSTEM_PREFIXES: [&str; 6] = ["/proc", "/sys", "/dev", "/run", "/tmp", "/var"];

/// Filesystem types that are never user storage.
const PSEUDO_FILESYSTEMS: [&str; 14] = [
    "proc",
    "sysfs",
    "devtmpfs",
    "devpts",
    "tmpfs",
    "cgroup",
    "cgroup2",
    "securityfs",
    "debugfs",
    "tracefs",
    "configfs",
    "fusectl",
    "pstore",
    "efivarfs",
];

/// Whether a mount is worth showing in Devices.
///
/// GIO's built-in Unix volume monitor reports the mount table even without gvfs,
/// so "is a backend present" is never false and cannot gate the section.
pub fn is_user_relevant(candidate: &MountCandidate<'_>) -> bool {
    if candidate.is_shadowed {
        return false;
    }

    if candidate.is_removable || candidate.can_eject {
        return true;
    }

    if candidate
        .filesystem
        .is_some_and(|fs| PSEUDO_FILESYSTEMS.contains(&fs))
    {
        return false;
    }

    let Some(mount_point) = candidate.mount_point else {
        return false;
    };

    if SYSTEM_MOUNT_POINTS
        .iter()
        .any(|system| Path::new(system) == mount_point)
    {
        return false;
    }

    if SYSTEM_PREFIXES
        .iter()
        .any(|prefix| mount_point.starts_with(prefix))
    {
        return false;
    }

    candidate.can_unmount || mount_point.starts_with("/mnt") || mount_point.starts_with("/media")
}

/// Whether the Devices section should be rendered at all.
pub fn should_show_section(candidates: &[MountCandidate<'_>]) -> bool {
    candidates.iter().any(is_user_relevant)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn at(path: &str) -> MountCandidate<'_> {
        MountCandidate {
            mount_point: Some(Path::new(path)),
            ..MountCandidate::default()
        }
    }

    #[test]
    fn a_usb_stick_is_relevant() {
        let usb = MountCandidate {
            mount_point: Some(Path::new("/run/media/diren/USB")),
            filesystem: Some("vfat"),
            is_removable: true,
            can_eject: true,
            can_unmount: true,
            is_shadowed: false,
        };
        assert!(is_user_relevant(&usb));
    }

    #[test]
    fn removable_media_beats_the_system_prefix_rule() {
        let usb = MountCandidate {
            mount_point: Some(Path::new("/run/media/diren/CARD")),
            is_removable: true,
            ..MountCandidate::default()
        };
        assert!(is_user_relevant(&usb));
    }

    #[test]
    fn the_root_filesystem_is_not_a_device_row() {
        assert!(!is_user_relevant(&at("/")));
        assert!(!is_user_relevant(&at("/boot")));
        assert!(!is_user_relevant(&at("/boot/efi")));
        assert!(!is_user_relevant(&at("/usr")));
    }

    #[test]
    fn pseudo_filesystems_are_excluded() {
        for fs in ["proc", "sysfs", "tmpfs", "cgroup2", "devtmpfs"] {
            let candidate = MountCandidate {
                mount_point: Some(Path::new("/somewhere")),
                filesystem: Some(fs),
                ..MountCandidate::default()
            };
            assert!(!is_user_relevant(&candidate), "{fs} should be excluded");
        }
    }

    #[test]
    fn system_prefixes_are_excluded() {
        for path in [
            "/proc/self",
            "/sys/fs/cgroup",
            "/dev/shm",
            "/run/user/1000",
            "/tmp/x",
            "/var/lib",
        ] {
            assert!(!is_user_relevant(&at(path)), "{path} should be excluded");
        }
    }

    #[test]
    fn an_extra_internal_disk_is_relevant() {
        let data = MountCandidate {
            mount_point: Some(Path::new("/mnt/data")),
            filesystem: Some("ext4"),
            can_unmount: true,
            ..MountCandidate::default()
        };
        assert!(is_user_relevant(&data));

        assert!(is_user_relevant(&at("/mnt/archive")));
        assert!(is_user_relevant(&at("/media/backup")));
    }

    #[test]
    fn shadowed_mounts_are_hidden() {
        let shadowed = MountCandidate {
            mount_point: Some(Path::new("/run/media/diren/USB")),
            is_removable: true,
            can_eject: true,
            is_shadowed: true,
            ..MountCandidate::default()
        };
        assert!(!is_user_relevant(&shadowed));
    }

    #[test]
    fn an_unmounted_non_removable_volume_is_not_shown() {
        let candidate = MountCandidate {
            mount_point: None,
            filesystem: Some("ext4"),
            ..MountCandidate::default()
        };
        assert!(!is_user_relevant(&candidate));
    }

    #[test]
    fn the_target_machine_hides_the_whole_section() {
        let mounts = [
            at("/"),
            at("/boot"),
            MountCandidate {
                mount_point: Some(Path::new("/run/user/1000")),
                filesystem: Some("tmpfs"),
                ..MountCandidate::default()
            },
            MountCandidate {
                mount_point: Some(Path::new("/sys/fs/cgroup")),
                filesystem: Some("cgroup2"),
                ..MountCandidate::default()
            },
        ];
        assert!(!should_show_section(&mounts));
    }

    #[test]
    fn plugging_in_one_stick_reveals_the_section() {
        let mounts = [
            at("/"),
            at("/boot"),
            MountCandidate {
                mount_point: Some(Path::new("/run/media/diren/USB")),
                filesystem: Some("vfat"),
                is_removable: true,
                can_eject: true,
                can_unmount: true,
                is_shadowed: false,
            },
        ];
        assert!(should_show_section(&mounts));
    }

    #[test]
    fn an_empty_monitor_hides_the_section() {
        assert!(!should_show_section(&[]));
    }

    #[test]
    fn a_mount_point_sharing_a_prefix_string_is_not_excluded() {
        let candidate = MountCandidate {
            mount_point: Some(Path::new("/variant")),
            can_unmount: true,
            ..MountCandidate::default()
        };
        assert!(is_user_relevant(&candidate));

        let under_var = MountCandidate {
            mount_point: Some(Path::new("/var/lib/thing")),
            can_unmount: true,
            ..MountCandidate::default()
        };
        assert!(!is_user_relevant(&under_var));
    }
}
