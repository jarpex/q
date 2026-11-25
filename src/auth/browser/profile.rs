//! Ephemeral Firefox login profile management.

use crate::config::config_dir;
use crate::{Error, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{remove_file_quiet, unique_suffix, FIREFOX_VERSION};

/// Creates a profile of its own for one login attempt.
///
/// The profile holds the session of the sign-in, so it lives next to the
/// browser and is removed once the cookies have been read.
///
/// # Errors
///
/// Returns an error if the profile directory cannot be created.
pub fn create_profile() -> Result<PathBuf> {
    let profile = config_dir()?
        .join("firefox")
        .join("profiles")
        .join(format!("login-{}", unique_suffix()));

    fs::create_dir_all(&profile)
        .map_err(|e| Error::System(format!("cannot create {}: {e}", profile.display())))?;

    fs::write(profile.join("user.js"), PROFILE_PREFS).map_err(|e| {
        Error::System(format!(
            "cannot write the preferences of {}: {e}",
            profile.display()
        ))
    })?;

    Ok(profile)
}

/// Removes the profiles and executables of the logins that were not finished.
///
/// A login that was interrupted leaves its profile and unique executable
/// behind. The profiles are of no use to anybody else, so they are removed
/// at the start of the next login.
///
/// # Errors
///
/// Returns an error if the profiles cannot be listed.
pub fn remove_stale_profiles() -> Result<()> {
    if let Ok(config) = config_dir() {
        let core_dir = config.join("firefox").join(FIREFOX_VERSION).join("core");
        if let Ok(entries) = fs::read_dir(core_dir) {
            for entry in entries.filter_map(std::result::Result::ok) {
                let path = entry.path();
                if path.is_file() {
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or_default();
                    let is_exe = Path::new(name)
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"));
                    if name.starts_with("q-firefox-") && is_exe {
                        remove_file_quiet(&path);
                    }
                }
            }
        }
    }

    let directory = config_dir()?.join("firefox").join("profiles");

    let Ok(entries) = fs::read_dir(&directory) else {
        return Ok(());
    };

    for entry in entries.filter_map(std::result::Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            let _ = remove_profile(&path);
        }
    }

    Ok(())
}

/// Removes a profile that is not needed any more.
///
/// Retries up to 10 times with exponential backoff if the directory is locked
/// by Windows processes or antivirus scanners.
///
/// # Errors
///
/// Returns an error if the profile cannot be removed.
pub fn remove_profile(profile: &Path) -> Result<()> {
    if !profile.exists() {
        return Ok(());
    }

    for attempt in 0..10 {
        match fs::remove_dir_all(profile) {
            Ok(()) => return Ok(()),
            Err(_) if attempt < 9 => {
                std::thread::sleep(Duration::from_millis(50 * (1 << attempt)));
            }
            Err(e) => {
                return Err(Error::System(format!(
                    "cannot remove {}: {e}",
                    profile.display()
                )))
            }
        }
    }
    Ok(())
}

/// Preferences of the profile of the login.
///
/// They turn off the pages that Firefox shows on its first start and the
/// warning about the default browser, and they shorten the interval in which
/// Firefox writes its session store, so that the cookies of the sign-in
/// become visible while the browser is still running.
///
/// The preferences that keep the browser from reaching out to the network
/// also make it start faster: the first start of a fresh profile otherwise
/// waits for the lists of safe browsing and for the region of the machine.
const PROFILE_PREFS: &str = r#"
user_pref("browser.aboutwelcome.enabled", false);
user_pref("browser.sessionstore.interval", 3000);
user_pref("browser.shell.checkDefaultBrowser", false);
user_pref("browser.privacySegmentation.createdShortcut", true);
user_pref("browser.privacySegmentation.createShortcut", false);
user_pref("browser.shell.shortcutFavicons", false);
user_pref("browser.startup.homepage", "about:blank");
user_pref("browser.startup.homepage_override.mstone", "ignore");
user_pref("browser.startup.page", 0);
user_pref("startup.homepage_welcome_url", "about:blank");
user_pref("trailhead.firstrun.didSeeAboutWelcome", true);
user_pref("browser.tabs.warnOnClose", false);
user_pref("browser.warnOnQuit", false);
user_pref("browser.taskbar.lists.enabled", false);
user_pref("datareporting.policy.dataSubmissionEnabled", false);
user_pref("toolkit.telemetry.enabled", false);
user_pref("app.update.auto", false);
user_pref("app.update.enabled", false);
user_pref("app.update.background.scheduling.enabled", false);
user_pref("app.update.background.enabled", false);
user_pref("app.update.service.enabled", false);
user_pref("default-browser-agent.enabled", false);
user_pref("browser.contentblocking.report.hide_vpn_banner", true);
user_pref("browser.discovery.enabled", false);
user_pref("browser.newtabpage.activity-stream.feeds.telemetry", false);
user_pref("browser.region.network.url", "");
user_pref("browser.region.update.enabled", false);
user_pref("browser.safebrowsing.malware.enabled", false);
user_pref("browser.safebrowsing.phishing.enabled", false);
user_pref("browser.safebrowsing.blockedURIs.enabled", false);
user_pref("browser.safebrowsing.downloads.enabled", false);
user_pref("browser.search.update", false);
user_pref("browser.uitour.enabled", false);
user_pref("captivedetect.canonicalURL", "");
user_pref("network.captive-portal-service.enabled", false);
user_pref("network.connectivity-service.enabled", false);
user_pref("network.captive-portal-service.timeout", 1);
user_pref("network.dns.disablePrefetch", true);
user_pref("network.prefetch-next", false);
user_pref("network.predictor.enabled", false);
user_pref("services.settings.server", "");
"#;

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::{remove_stale_profiles, FIREFOX_VERSION, PROFILE_PREFS};
    use crate::config::config_dir;

    #[test]
    fn the_profile_silences_the_browser() {
        assert!(PROFILE_PREFS.contains("browser.aboutwelcome.enabled"));
        assert!(PROFILE_PREFS.contains("browser.warnOnQuit"));
    }

    #[test]
    fn the_profile_keeps_the_browser_off_the_network() {
        // The first start of a fresh profile waits for the lists of safe
        // browsing and for the region of the machine. Turning those off
        // makes the window appear much sooner.
        assert!(PROFILE_PREFS.contains("browser.safebrowsing.malware.enabled"));
        assert!(PROFILE_PREFS.contains("browser.region.update.enabled"));
        assert!(PROFILE_PREFS.contains("network.captive-portal-service.enabled"));
        assert!(PROFILE_PREFS.contains("app.update.enabled"));
    }

    #[test]
    fn removes_the_profiles_of_the_logins_that_were_interrupted() -> anyhow::Result<()> {
        let stale = config_dir()?
            .join("firefox")
            .join("profiles")
            .join("login-stale");
        std::fs::create_dir_all(&stale)?;
        let stale_exec = config_dir()?
            .join("firefox")
            .join(FIREFOX_VERSION)
            .join("core")
            .join("q-firefox-stale.exe");
        if let Some(parent) = stale_exec.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&stale_exec, b"binary")?;
        std::fs::write(stale.join("cookies.sqlite"), b"cookies")?;

        remove_stale_profiles()?;

        assert!(!stale.exists(), "the stale profile is still there");
        assert!(!stale_exec.exists(), "the stale executable is still there");
        Ok(())
    }
}
