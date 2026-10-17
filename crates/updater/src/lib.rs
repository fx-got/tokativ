//! Stable GitHub releases, verified downloads and installation outside the running process.

mod install;

use std::fs::File;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context as _, Result, ensure};
use reqwest::blocking::Client;
use semver::Version;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use tempfile::TempDir;

pub use install::{Installation, apply, take_error};

pub const RELEASES: &str = "https://github.com/nolight132/tokativ/releases";
const API: &str = "https://api.github.com/repos/nolight132/tokativ/releases/latest";
const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    WindowsIntel,
    WindowsArm,
    MacIntel,
    MacArm,
}

impl Platform {
    pub fn current() -> Option<Self> {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("windows", "x86_64") => Some(Self::WindowsIntel),
            ("windows", "aarch64") => Some(Self::WindowsArm),
            ("macos", "x86_64") => Some(Self::MacIntel),
            ("macos", "aarch64") => Some(Self::MacArm),
            _ => None,
        }
    }

    fn asset(self, version: &str) -> String {
        let suffix = match self {
            Self::WindowsIntel => "x86_64-pc-windows-msvc-setup.exe",
            Self::WindowsArm => "aarch64-pc-windows-msvc-setup.exe",
            Self::MacIntel => "x86_64-apple-darwin.app.tar.gz",
            Self::MacArm => "aarch64-apple-darwin.app.tar.gz",
        };
        format!("tokativ-v{version}-{suffix}")
    }
}

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub notes: String,
    pub page: String,
    asset: Asset,
    checksums: Asset,
}

#[derive(Clone, Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

/// A successful check with no newer stable version returns `None`.
pub fn check(current: &str, platform: Platform) -> Result<Option<Release>> {
    let client = client(Duration::from_secs(20))?;
    let response = client.get(API).send().context("Checking for updates")?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let release = response.error_for_status()?.json::<GithubRelease>()?;
    select(release, current, platform)
}

fn select(release: GithubRelease, current: &str, platform: Platform) -> Result<Option<Release>> {
    let version = release
        .tag_name
        .strip_prefix('v')
        .unwrap_or(&release.tag_name);
    let candidate = Version::parse(version).context("The release has an invalid version")?;
    let current = Version::parse(current)?;
    if release.draft
        || release.prerelease
        || !candidate.pre.is_empty()
        || !candidate.cmp_precedence(&current).is_gt()
    {
        return Ok(None);
    }
    let asset_name = platform.asset(version);
    let find = |name: &str| -> Result<Asset> {
        let asset = release
            .assets
            .iter()
            .find(|asset| asset.name == name)
            .with_context(|| format!("The release has no {name} download"))?;
        let expected = format!("{RELEASES}/download/{}/{name}", release.tag_name);
        ensure!(
            asset.browser_download_url == expected,
            "Unexpected release download URL"
        );
        ensure!(
            asset.size > 0 && asset.size <= MAX_DOWNLOAD,
            "Invalid download size"
        );
        Ok(asset.clone())
    };
    Ok(Some(Release {
        version: version.to_owned(),
        notes: release.body.clone().unwrap_or_default(),
        page: format!("{RELEASES}/tag/{}", release.tag_name),
        asset: find(&asset_name)?,
        checksums: find("SHA256SUMS")?,
    }))
}

fn client(timeout: Duration) -> Result<Client> {
    Ok(Client::builder()
        .user_agent(concat!("Tokativ/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(timeout)
        .https_only(true)
        .build()?)
}

/// Ready to install. Dropping this removes the downloaded files.
pub struct Prepared {
    folder: TempDir,
    payload: PathBuf,
    installation: Installation,
}

/// Downloads exactly the expected asset, verifies SHA-256 and checks the macOS bundle.
pub fn download(
    release: &Release,
    installation: Installation,
    progress: impl Fn(u64, u64),
) -> Result<Prepared> {
    ensure!(
        installation.can_install(),
        "This installation is managed externally"
    );
    let client = client(Duration::from_secs(600))?;
    let mut checksums = Vec::new();
    client
        .get(&release.checksums.browser_download_url)
        .send()?
        .error_for_status()?
        .take(64 * 1024 + 1)
        .read_to_end(&mut checksums)?;
    ensure!(
        checksums.len() <= 64 * 1024,
        "The checksum file is too large"
    );
    let expected = checksum(std::str::from_utf8(&checksums)?, &release.asset.name)?;
    let folder = tempfile::Builder::new()
        .prefix("tokativ-update-")
        .tempdir()?;
    let payload = folder.path().join(&release.asset.name);
    let response = client
        .get(&release.asset.browser_download_url)
        .send()?
        .error_for_status()
        .context("Downloading the update")?;
    let mut file = File::create(&payload)?;
    receive(response, &mut file, release.asset.size, &expected, progress)?;
    install::validate(&payload, &installation, &release.version)?;
    Ok(Prepared {
        folder,
        payload,
        installation,
    })
}

fn receive(
    mut response: impl Read,
    file: &mut File,
    total: u64,
    expected: &str,
    progress: impl Fn(u64, u64),
) -> Result<()> {
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut done = 0;
    progress(done, total);
    loop {
        let count = response.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        done += count as u64;
        ensure!(
            done <= total && done <= MAX_DOWNLOAD,
            "The download is larger than expected"
        );
        file.write_all(&buffer[..count])?;
        hasher.update(&buffer[..count]);
        progress(done, total);
    }
    file.sync_all()?;
    ensure!(done == total, "The download was interrupted");
    ensure!(
        format!("{:x}", hasher.finalize()) == expected,
        "The update failed its SHA-256 check"
    );
    Ok(())
}

fn checksum(text: &str, name: &str) -> Result<String> {
    let mut found = None;
    for line in text.lines() {
        let Some((hash, file)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        if file.trim().trim_start_matches('*') != name {
            continue;
        }
        ensure!(found.is_none(), "Duplicate checksum for the update");
        ensure!(
            hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid SHA-256 checksum"
        );
        found = Some(hash.to_ascii_lowercase());
    }
    found.context("The release is missing the update's SHA-256 checksum")
}

impl Prepared {
    /// Starts an independent helper. Only after this succeeds should the daemon quit.
    pub fn install(self) -> Result<()> {
        install::launch(self.folder, self.payload, self.installation)
    }
}

pub fn installation() -> Result<Installation> {
    let executable = std::env::current_exe()?.canonicalize()?;
    Ok(Installation::detect(executable))
}

pub(crate) fn state_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("tokativ/updates")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(version: &str, platform: Platform) -> GithubRelease {
        let tag = format!("v{version}");
        let assets = [platform.asset(version), "SHA256SUMS".to_owned()]
            .into_iter()
            .map(|name| Asset {
                browser_download_url: format!("{RELEASES}/download/{tag}/{name}"),
                name,
                size: 123,
            })
            .collect();
        GithubRelease {
            tag_name: tag,
            body: Some("Changes".into()),
            draft: false,
            prerelease: false,
            assets,
        }
    }

    #[test]
    fn selects_native_installer_or_bundle() {
        for platform in [
            Platform::WindowsIntel,
            Platform::WindowsArm,
            Platform::MacIntel,
            Platform::MacArm,
        ] {
            let selected = select(release("0.4.0", platform), "0.3.0", platform)
                .unwrap()
                .unwrap();
            assert_eq!(selected.asset.name, platform.asset("0.4.0"));
        }
        assert!(
            select(
                release("0.4.0", Platform::WindowsArm),
                "0.3.0",
                Platform::WindowsIntel
            )
            .is_err()
        );
    }

    #[test]
    fn ignores_downgrades_prereleases_and_metadata_only_changes() {
        for (next, current) in [
            ("0.3.0", "0.3.0"),
            ("0.2.0", "0.3.0"),
            ("0.4.0-beta.1", "0.3.0"),
            ("0.3.0+build2", "0.3.0+build1"),
        ] {
            assert!(
                select(release(next, Platform::MacArm), current, Platform::MacArm)
                    .unwrap()
                    .is_none()
            );
        }
        let mut draft = release("0.4.0", Platform::MacArm);
        draft.draft = true;
        assert!(select(draft, "0.3.0", Platform::MacArm).unwrap().is_none());
    }

    #[test]
    fn rejects_foreign_downloads_and_invalid_checksums() {
        let mut foreign = release("0.4.0", Platform::MacArm);
        foreign.assets[0].browser_download_url = "https://example.com/update".into();
        assert!(select(foreign, "0.3.0", Platform::MacArm).is_err());
        let hash = "ab".repeat(32);
        assert_eq!(
            checksum(&format!("{hash}  update.exe\n"), "update.exe").unwrap(),
            hash
        );
        assert!(checksum("bad  update.exe", "update.exe").is_err());
        assert!(
            checksum(
                &format!("{hash}  update.exe\n{hash}  update.exe"),
                "update.exe"
            )
            .is_err()
        );
        assert!(checksum(&format!("{hash}  other.exe"), "update.exe").is_err());
    }

    #[test]
    fn verifies_payload_before_an_install_can_be_prepared() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("payload");
        let payload = b"verified update";
        let hash = format!("{:x}", Sha256::digest(payload));
        let mut output = File::create(&path).unwrap();
        receive(
            &payload[..],
            &mut output,
            payload.len() as u64,
            &hash,
            |_, _| {},
        )
        .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), payload);
        for (bytes, size, expected) in [
            (&b"tampered update"[..], 15, hash.as_str()),
            (&payload[..], 100, hash.as_str()),
            (&payload[..], 2, hash.as_str()),
            (&payload[..], payload.len() as u64, "00"),
        ] {
            let mut output = File::create(&path).unwrap();
            assert!(receive(bytes, &mut output, size, expected, |_, _| {}).is_err());
        }
    }
}
