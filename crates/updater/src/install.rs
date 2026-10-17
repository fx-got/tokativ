use std::fs;
use std::io::{Read as _, Seek as _};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use tempfile::TempDir;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Installation {
    Windows {
        executable: PathBuf,
    },
    MacBundle {
        bundle: PathBuf,
        executable: PathBuf,
    },
    Managed,
}

impl Installation {
    pub(crate) fn detect(executable: PathBuf) -> Self {
        if executable.starts_with("/nix/store") {
            return Self::Managed;
        }
        if cfg!(windows) {
            return Self::Windows { executable };
        }
        if let Some(macos) = executable.parent()
            && macos.file_name().is_some_and(|name| name == "MacOS")
            && let Some(contents) = macos.parent()
            && contents.file_name().is_some_and(|name| name == "Contents")
            && let Some(bundle) = contents.parent()
            && bundle
                .extension()
                .is_some_and(|extension| extension == "app")
        {
            return Self::MacBundle {
                bundle: bundle.to_owned(),
                executable,
            };
        }
        Self::Managed
    }

    pub fn can_install(&self) -> bool {
        !matches!(self, Self::Managed)
    }
}

#[derive(Serialize, Deserialize)]
struct Plan {
    parent: u32,
    payload: PathBuf,
    installation: Installation,
}

pub(crate) fn validate(payload: &Path, installation: &Installation, version: &str) -> Result<()> {
    match installation {
        Installation::Windows { .. } => {
            let mut file = fs::File::open(payload)?;
            let mut dos = [0u8; 64];
            file.read_exact(&mut dos)?;
            ensure!(
                &dos[..2] == b"MZ",
                "The download is not a Windows installer"
            );
            let offset = u32::from_le_bytes(dos[60..64].try_into()?);
            ensure!(
                offset <= 16 * 1024 * 1024,
                "Invalid Windows executable header"
            );
            file.seek(std::io::SeekFrom::Start(offset.into()))?;
            let mut signature = [0u8; 4];
            file.read_exact(&mut signature)?;
            ensure!(&signature == b"PE\0\0", "Invalid Windows installer");
        }
        Installation::MacBundle { bundle, .. } => {
            let extracted = unpack(payload)?;
            validate_bundle(&extracted, version)?;
            // Fail before quitting or replacing anything if this install needs administrator access.
            tempfile::Builder::new()
                .prefix(".tokativ-write-check-")
                .tempdir_in(bundle.parent().context("The app has no parent folder")?)
                .context(
                    "Cannot update this app in its current folder. Install the download manually",
                )?;
        }
        Installation::Managed => bail!("Update this installation with its package manager"),
    }
    Ok(())
}

fn unpack(payload: &Path) -> Result<PathBuf> {
    let destination = payload
        .parent()
        .context("The download has no folder")?
        .join("unpacked");
    fs::create_dir_all(&destination)?;
    let gzip = flate2::read::GzDecoder::new(fs::File::open(payload)?);
    let mut archive = tar::Archive::new(gzip);
    let mut bytes = 0u64;
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let mut components = path.components().filter(|c| *c != Component::CurDir);
        ensure!(
            components.next() == Some(Component::Normal("Tokativ.app".as_ref()))
                && components.all(|c| matches!(c, Component::Normal(_))),
            "Invalid path in the update archive"
        );
        // Release bundles contain a static executable and its metadata, with no external links.
        let kind = entry.header().entry_type();
        ensure!(
            kind.is_file() || kind.is_dir(),
            "Unexpected link in the update archive"
        );
        bytes = bytes
            .checked_add(entry.size())
            .context("Archive size overflow")?;
        ensure!(
            bytes <= 1024 * 1024 * 1024,
            "The update archive is too large"
        );
        ensure!(
            entry.unpack_in(&destination)?,
            "Unsafe path in the update archive"
        );
    }
    let bundle = destination.join("Tokativ.app");
    ensure!(
        bundle.join("Contents/MacOS/tokativ").is_file(),
        "The update has no Tokativ executable"
    );
    Ok(bundle)
}

#[cfg(target_os = "macos")]
fn validate_bundle(bundle: &Path, version: &str) -> Result<()> {
    for (key, expected) in [
        ("CFBundleIdentifier", "io.github.nolight132.tokativ"),
        ("CFBundleExecutable", "tokativ"),
        ("CFBundleShortVersionString", version),
    ] {
        let output = Command::new("/usr/bin/plutil")
            .args(["-extract", key, "raw", "-o", "-"])
            .arg(bundle.join("Contents/Info.plist"))
            .output()?;
        ensure!(
            output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == expected,
            "The downloaded app has an unexpected {key}"
        );
    }
    let output = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(bundle)
        .output()?;
    ensure!(
        output.status.success(),
        "The downloaded app's code signature is invalid: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn validate_bundle(_: &Path, _: &str) -> Result<()> {
    bail!("App bundles can only be installed on macOS")
}

pub(crate) fn launch(folder: TempDir, payload: PathBuf, installation: Installation) -> Result<()> {
    let helper = folder.path().join(if cfg!(windows) {
        "update-helper.exe"
    } else {
        "update-helper"
    });
    fs::copy(std::env::current_exe()?, &helper).context("Preparing the update helper")?;
    #[cfg(target_os = "macos")]
    {
        // The copy is now a standalone executable, outside the original signed app bundle.
        let signed = Command::new("/usr/bin/codesign")
            .args(["--force", "--sign", "-"])
            .arg(&helper)
            .status()?;
        ensure!(
            signed.success(),
            "Could not prepare the update helper's code signature"
        );
    }
    let manifest = folder.path().join("install.json");
    fs::write(
        &manifest,
        serde_json::to_vec(&Plan {
            parent: std::process::id(),
            payload,
            installation,
        })?,
    )?;
    queue_cleanup(folder.path());
    let mut command = Command::new(helper);
    command
        .arg("apply-update")
        .arg(&manifest)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().context("Starting the update helper")?;
    let deadline = Instant::now() + Duration::from_secs(15);
    while !folder.path().join("ready").is_file() {
        ensure!(
            child.try_wait()?.is_none(),
            "The update helper could not start. Tokativ will stay open"
        );
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("The update helper did not respond. Tokativ will stay open");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = folder.keep();
    Ok(())
}

/// The hidden helper waits for the daemon to exit; failed installs reopen the old app.
pub fn apply(manifest: &Path) -> Result<()> {
    let plan: Plan = serde_json::from_slice(&fs::read(manifest)?)?;
    fs::write(manifest.with_file_name("ready"), b"ready")?;
    let result = wait_for_parent(plan.parent).and_then(|_| install(&plan));
    if let Err(error) = &result {
        let _ = fs::create_dir_all(super::state_dir());
        let _ = fs::write(
            super::state_dir().join("last-error.txt"),
            format!("{error:#}"),
        );
        // Do not open a second instance when the original daemon has not exited.
        if !parent_running(plan.parent) {
            let _ = reopen(&plan.installation);
        }
    } else if let Some(folder) = manifest.parent() {
        // On Windows the running helper cannot delete itself. Remove the payload now; the
        // next launch clears the remaining helper after this process has exited.
        let _ = fs::remove_dir_all(folder);
    }
    result
}

pub fn take_error() -> Option<String> {
    cleanup();
    let path = super::state_dir().join("last-error.txt");
    let error = fs::read_to_string(&path).ok()?;
    let _ = fs::remove_file(path);
    Some(error)
}

fn queue_cleanup(folder: &Path) {
    let path = super::state_dir().join("cleanup.json");
    let mut pending: Vec<PathBuf> = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    pending.push(folder.to_owned());
    let _ = fs::create_dir_all(super::state_dir());
    if let Ok(bytes) = serde_json::to_vec(&pending) {
        let _ = fs::write(path, bytes);
    }
}

fn cleanup() {
    let path = super::state_dir().join("cleanup.json");
    let Some(pending) = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Vec<PathBuf>>(&bytes).ok())
    else {
        return;
    };
    let temporary = std::env::temp_dir().canonicalize().ok();
    let mut remaining = Vec::new();
    for folder in pending {
        let Ok(canonical) = folder.canonicalize() else {
            continue;
        };
        if canonical.parent() != temporary.as_deref()
            || !canonical
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("tokativ-update-"))
        {
            continue;
        }
        if fs::remove_dir_all(&canonical).is_err() {
            remaining.push(folder);
        }
    }
    if remaining.is_empty() {
        let _ = fs::remove_file(path);
    } else if let Ok(bytes) = serde_json::to_vec(&remaining) {
        let _ = fs::write(path, bytes);
    }
}

fn wait_for_parent(pid: u32) -> Result<()> {
    ensure!(
        pid > 1 && pid != std::process::id(),
        "Invalid updater parent"
    );
    let deadline = Instant::now() + Duration::from_secs(60);
    while parent_running(pid) {
        ensure!(
            Instant::now() < deadline,
            "Tokativ did not finish closing. Try the update again"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn parent_running(pid: u32) -> bool {
    // SAFETY: signal zero only checks whether this process still exists.
    (unsafe { libc::kill(pid as libc::pid_t, 0) == 0 })
        || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(windows)]
fn parent_running(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_INVALID_PARAMETER, GetLastError, WAIT_OBJECT_0,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };
    // SAFETY: the handle is only waited on, and always closed in this call.
    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if handle.is_null() {
            return GetLastError() != ERROR_INVALID_PARAMETER;
        }
        let running = WaitForSingleObject(handle, 0) != WAIT_OBJECT_0;
        CloseHandle(handle);
        running
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn parent_running(_: u32) -> bool {
    false
}

fn install(plan: &Plan) -> Result<()> {
    match &plan.installation {
        Installation::Windows { .. } => install_windows(&plan.payload),
        Installation::MacBundle { bundle, .. } => install_macos(&plan.payload, bundle),
        Installation::Managed => bail!("Update this installation with its package manager"),
    }
}

#[cfg(windows)]
fn install_windows(payload: &Path) -> Result<()> {
    let status = Command::new(payload)
        .args([
            "/VERYSILENT",
            "/SUPPRESSMSGBOXES",
            "/NORESTART",
            "/SP-",
            "/UPDATE=1",
        ])
        .arg(format!("/LOG={}", payload.with_extension("log").display()))
        .status()
        .context("Starting the update installer")?;
    ensure!(
        status.success(),
        "The Windows installer failed ({status}). The previous app will reopen"
    );
    Ok(())
}

#[cfg(not(windows))]
fn install_windows(_: &Path) -> Result<()> {
    bail!("Windows installers require Windows")
}

#[cfg(target_os = "macos")]
fn install_macos(payload: &Path, bundle: &Path) -> Result<()> {
    let parent = bundle.parent().context("The app has no parent folder")?;
    let work = tempfile::Builder::new()
        .prefix(".tokativ-update-")
        .tempdir_in(parent)?;
    let staged = work.path().join("Tokativ.app");
    let source = payload
        .parent()
        .context("The update has no folder")?
        .join("unpacked/Tokativ.app");
    let copy = Command::new("/usr/bin/ditto")
        .arg(&source)
        .arg(&staged)
        .status()?;
    ensure!(
        copy.success(),
        "Could not stage the new app beside the current app"
    );
    let result = replace(bundle, &staged, &work.path().join("previous.app"), || {
        open_bundle(bundle)
    });
    if result.is_err() {
        // Keep any backup if rollback itself failed.
        let _ = work.keep();
    }
    result
}

#[cfg(not(target_os = "macos"))]
fn install_macos(_: &Path, _: &Path) -> Result<()> {
    bail!("App bundles require macOS")
}

#[cfg(any(target_os = "macos", test))]
fn replace(
    target: &Path,
    staged: &Path,
    backup: &Path,
    launch: impl FnOnce() -> Result<()>,
) -> Result<()> {
    fs::rename(target, backup).context("Keeping the previous app for rollback")?;
    if let Err(error) = fs::rename(staged, target) {
        fs::rename(backup, target).with_context(|| {
            format!("Cannot restore the previous app from {}", backup.display())
        })?;
        return Err(error).context("Installing the new app");
    }
    if let Err(error) = launch() {
        fs::rename(target, staged).context("Moving the failed update aside")?;
        fs::rename(backup, target).with_context(|| {
            format!("Cannot restore the previous app from {}", backup.display())
        })?;
        return Err(error).context("Opening the updated app");
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn open_bundle(bundle: &Path) -> Result<()> {
    ensure!(
        Command::new("/usr/bin/open")
            .arg("-n")
            .arg(bundle)
            .status()?
            .success(),
        "macOS could not open Tokativ"
    );
    Ok(())
}

fn reopen(installation: &Installation) -> Result<()> {
    #[cfg(target_os = "macos")]
    if let Installation::MacBundle { bundle, .. } = installation {
        return open_bundle(bundle);
    }
    if let Installation::Windows { executable } = installation {
        Command::new(executable)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_rolls_back_when_launch_or_move_fails() {
        let folder = tempfile::tempdir().unwrap();
        let target = folder.path().join("app");
        let next = folder.path().join("next");
        let backup = folder.path().join("backup");
        fs::write(&target, "old").unwrap();
        fs::write(&next, "new").unwrap();
        assert!(replace(&target, &next, &backup, || bail!("launch failed")).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "old");
        fs::remove_file(&next).unwrap();
        assert!(replace(&target, &next, &backup, || Ok(())).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "old");
        fs::write(&next, "new").unwrap();
        replace(&target, &next, &backup, || Ok(())).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "new");
        assert_eq!(fs::read_to_string(&backup).unwrap(), "old");
    }

    #[test]
    fn recognises_bundles_and_package_manager_installs() {
        assert!(matches!(
            Installation::detect("/nix/store/xxx/bin/tokativ".into()),
            Installation::Managed
        ));
        if !cfg!(windows) {
            assert!(matches!(
                Installation::detect("/Applications/Tokativ.app/Contents/MacOS/tokativ".into()),
                Installation::MacBundle { .. }
            ));
            assert!(matches!(
                Installation::detect("/usr/local/bin/tokativ".into()),
                Installation::Managed
            ));
        }
    }

    #[test]
    fn archive_cannot_extract_foreign_paths_or_links() {
        let folder = tempfile::tempdir().unwrap();
        for (path, symlink) in [
            ("Other.app/file", false),
            ("Tokativ.app/Contents/link", true),
        ] {
            let archive = folder.path().join("update.tar.gz");
            let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
                fs::File::create(&archive).unwrap(),
                flate2::Compression::fast(),
            ));
            let mut header = tar::Header::new_gnu();
            header.set_size(0);
            header.set_mode(0o755);
            if symlink {
                header.set_entry_type(tar::EntryType::Symlink);
                header.set_link_name("/tmp").unwrap();
            }
            header.set_cksum();
            tar.append_data(&mut header, path, &[][..]).unwrap();
            tar.into_inner().unwrap().finish().unwrap();
            assert!(unpack(&archive).is_err());
        }
    }
}
