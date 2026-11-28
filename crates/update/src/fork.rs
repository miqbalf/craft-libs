//! udinCraft: the miqbalf fork of the Craft apps, whose builds carry no Developer ID or
//! Authenticode signature.
//!
//! An app is a fork app when its [`App::repo`] is one of [`OWNER`]'s repositories, and a release
//! is a fork release when, in addition, its tag is a udinCraft release tag ([`TAG_PREFIX`]) from
//! that owner. For a fork release only, the installs accept what the fork publishes:
//!
//! - macOS: an ad-hoc signed bundle that passes `codesign --verify --deep --strict` and is the
//!   app's own bundle identifier, instead of ArtCraft's Developer ID team;
//! - Windows: an unsigned MSI or program, once the download has passed its SHA-256 check against
//!   the release's checksums (which every install requires, fork or not).
//!
//! Everything else, the ArtCraft apps and their Developer ID and publisher checks, is unchanged.
//! The fork's trust is HTTPS from github.com/miqbalf plus the release's `SHA256SUMS.txt`.

use crate::{App, Release};

/// The GitHub owner whose repositories publish the fork's releases.
pub const OWNER: &str = "miqbalf";

/// The tag prefix of the fork's releases: `udincraft-v<version>-<YYYYMMDD>`.
pub const TAG_PREFIX: &str = "udincraft-v";

/// Whether `repo` (`owner/name`) is one of [`OWNER`]'s.
fn owned(repo: &str) -> bool {
    repo.split_once('/').is_some_and(|(owner, name)| owner.eq_ignore_ascii_case(OWNER) && !name.is_empty() && !name.contains('/'))
}

/// Whether `app` is configured for the fork's releases.
pub fn is_fork_app(app: &App) -> bool {
    owned(app.repo)
}

/// Whether `release` is a fork release of a fork app: a [`TAG_PREFIX`] tag read from one of
/// [`OWNER`]'s repositories.
pub fn is_fork_release(app: &App, release: &Release) -> bool {
    let repo = if release.repo.is_empty() { app.repo } else { release.repo.as_str() };
    is_fork_app(app) && owned(repo) && release.tag.starts_with(TAG_PREFIX)
}

/// The macOS bundle identifier a fork build must carry, when `app`'s is one `codesign` can be
/// asked about safely (letters, digits, `.` and `-`).
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(crate) fn bundle_identifier(app: &App) -> Option<String> {
    let id = app.bundle_id();
    (!id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')).then_some(id)
}

/// The Windows publisher check, for the publishers of this program (`ours`) and of the file it
/// would install (`theirs`), each an error when the file isn't signed. A signed program accepts
/// only a file from its own publisher. An unsigned one accepts nothing, except a fork app's, which
/// accepts its fork release's files: their SHA-256 was checked against the release's checksums
/// before the update got here.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
pub(crate) fn publisher_check<P: PartialEq>(app: &App, file: &str, ours: Result<P, String>, theirs: Result<P, String>) -> Result<(), String> {
    match ours {
        Ok(ours) => {
            let theirs = theirs.map_err(|e| format!("{file} isn't signed ({e})"))?;
            if ours == theirs { Ok(()) } else { Err(format!("{file} isn't signed by {}'s publisher", app.name)) }
        }
        Err(_) if is_fork_app(app) => Ok(()),
        Err(e) => Err(format!("this copy of {} isn't signed ({e}), so it doesn't install updates", app.name)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FORK: App = App { repo: "miqbalf/wordcraft", developer: "udinCraft", ..crate::suite::WORDCRAFT };

    fn release(tag: &str, repo: &str) -> Release {
        Release { tag: tag.into(), repo: repo.into(), ..Release::default() }
    }

    #[test]
    fn only_a_udincraft_tag_from_miqbalf_is_a_fork_release() {
        assert!(is_fork_app(&FORK));
        assert!(!is_fork_app(&crate::suite::WORDCRAFT));
        assert!(is_fork_release(&FORK, &release("udincraft-v0.18.1-20261127", "")));
        assert!(is_fork_release(&FORK, &release("udincraft-v0.18.1-20261127", "MiqbalF/wordcraft")));
        // An upstream tag, another owner's repository, or an upstream app: not the fork's.
        assert!(!is_fork_release(&FORK, &release("v0.19.1", "")));
        assert!(!is_fork_release(&FORK, &release("udincraft-v0.18.1-20261127", "storytold/wordcraft")));
        assert!(!is_fork_release(&crate::suite::WORDCRAFT, &release("udincraft-v0.18.1-20261127", "miqbalf/wordcraft")));
        // The launcher's own fork release, carried with its repository.
        let launcher = App { repo: "miqbalf/craft-launcher", ..crate::suite::LAUNCHER };
        assert!(is_fork_release(&launcher, &release("udincraft-v0.19.3-20261201", "miqbalf/craft-launcher")));
        assert!(!owned("miqbalf/"), "no repository name");
        assert!(!owned("miqbalf/a/b"));
    }

    #[test]
    fn the_fork_bundle_identifier_is_the_apps_own() {
        assert_eq!(bundle_identifier(&FORK).as_deref(), Some("ai.storyteller.wordcraft"));
        let odd = App { bundle_id_override: Some("x\" or anchor apple"), ..FORK };
        assert_eq!(bundle_identifier(&odd), None);
    }

    #[test]
    fn an_unsigned_file_installs_only_for_a_fork_app() {
        let unsigned = || Err::<String, _>("no signature".to_string());
        let signed = |who: &str| Ok::<_, String>(who.to_string());
        // Upstream: unchanged. Same publisher only; an unsigned side refuses.
        let up = &crate::suite::WORDCRAFT;
        assert_eq!(publisher_check(up, "x.msi", signed("ArtCraft"), signed("ArtCraft")), Ok(()));
        assert!(publisher_check(up, "x.msi", signed("ArtCraft"), signed("Other")).unwrap_err().contains("publisher"));
        assert!(publisher_check(up, "x.msi", signed("ArtCraft"), unsigned()).unwrap_err().contains("isn't signed"));
        assert!(publisher_check(up, "x.msi", unsigned(), unsigned()).unwrap_err().contains("doesn't install updates"));
        // The fork's unsigned copy installs its unsigned release.
        assert_eq!(publisher_check(&FORK, "x.msi", unsigned(), unsigned()), Ok(()));
        // A signed copy, even of a fork app, still wants its own publisher.
        assert!(publisher_check(&FORK, "x.msi", signed("Someone"), unsigned()).is_err());
    }
}
