//! The model catalog and its files under `$XDG_DATA_HOME/tokativ/models/<engine>`. Every file is
//! fetched from Hugging Face and checked against a known SHA-256 before the folder counts as
//! installed.

use std::fs::{self, File};
use std::io::{BufWriter, Read as _, Write as _};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context as _, Result, bail};
use sha2::{Digest as _, Sha256};
use tokativ_config::{EngineKind, models_dir};

/// One file of a model: where it comes from, the name the engine loads it by, and what it must
/// hash to.
pub struct Remote {
    pub url: &'static str,
    pub name: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

const PARAKEET: &[Remote] = &[
    Remote {
        url: "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/encoder-model.int8.onnx",
        name: "encoder-model.int8.onnx",
        size: 652_183_999,
        sha256: "6139d2fa7e1b086097b277c7149725edbab89cc7c7ae64b23c741be4055aff09",
    },
    Remote {
        url: "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/decoder_joint-model.int8.onnx",
        name: "decoder_joint-model.int8.onnx",
        size: 18_202_004,
        sha256: "eea7483ee3d1a30375daedc8ed83e3960c91b098812127a0d99d1c8977667a70",
    },
    Remote {
        url: "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/nemo128.onnx",
        name: "nemo128.onnx",
        size: 139_764,
        sha256: "a9fde1486ebfcc08f328d75ad4610c67835fea58c73ba57e3209a6f6cf019e9f",
    },
    Remote {
        url: "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/vocab.txt",
        name: "vocab.txt",
        size: 93_939,
        sha256: "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d",
    },
];

const GIGAAM: &[Remote] = &[
    Remote {
        url: "https://huggingface.co/istupakov/gigaam-v3-onnx/resolve/main/v3_e2e_ctc.int8.onnx",
        name: "model.int8.onnx",
        size: 224_893_347,
        sha256: "2e3fcb7a7b66030336fd10c2fcfb033bd1dc7e1bf238fe5cfd83b1d0cfc9d28e",
    },
    Remote {
        url: "https://huggingface.co/istupakov/gigaam-v3-onnx/resolve/main/v3_e2e_ctc_vocab.txt",
        name: "vocab.txt",
        size: 2_007,
        sha256: "142de7570b3de5b3035ce111a89c228e80e6085273731d944093ddf24fa539cd",
    },
];

/// The files a model is made of.
pub fn files(kind: EngineKind) -> &'static [Remote] {
    match kind {
        EngineKind::Parakeet => PARAKEET,
        EngineKind::Gigaam => GIGAAM,
    }
}

/// The download size of a model in bytes.
pub fn download_size(kind: EngineKind) -> u64 {
    files(kind).iter().map(|file| file.size).sum()
}

/// The folder a model is loaded from.
pub fn dir(kind: EngineKind) -> PathBuf {
    models_dir().join(kind.id())
}

/// The bytes a model takes on disk when all its files are present, or `None` when any is
/// missing or the wrong size.
pub fn installed(kind: EngineKind) -> Option<u64> {
    let dir = dir(kind);
    files(kind).iter().try_fold(0, |total, file| {
        let size = fs::metadata(dir.join(file.name)).ok()?.len();
        (size == file.size).then_some(total + size)
    })
}

/// Removes a model's folder.
pub fn delete(kind: EngineKind) -> Result<()> {
    let dir = dir(kind);
    match fs::remove_dir_all(&dir) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(error).with_context(|| format!("removing {}", dir.display()))
        }
        _ => Ok(()),
    }
}

/// The error a download returns when it was cancelled.
#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("download cancelled")
    }
}

impl std::error::Error for Cancelled {}

/// Downloads a model into a staging folder, verifies every file, then moves the folder into
/// place. `progress` gets the bytes done and the total as they arrive. Setting `cancel` stops
/// the download with [`Cancelled`] and leaves the staging folder for the next attempt to
/// overwrite. Blocks, so call it from a thread of its own.
pub fn download(kind: EngineKind, progress: &dyn Fn(u64, u64), cancel: &AtomicBool) -> Result<()> {
    let target = dir(kind);
    let staging = target.with_extension("partial");
    fs::create_dir_all(&staging).with_context(|| format!("creating {}", staging.display()))?;
    let total = download_size(kind);
    let client = reqwest::blocking::Client::builder()
        .timeout(None)
        .build()
        .context("building the HTTP client")?;
    let mut done = 0;
    progress(done, total);

    for file in files(kind) {
        let path = staging.join(file.name);
        if fs::metadata(&path).is_ok_and(|meta| meta.len() == file.size)
            && hash_file(&path)? == file.sha256
        {
            done += file.size;
            progress(done, total);
            continue;
        }
        let mut response = client
            .get(file.url)
            .send()
            .and_then(|response| response.error_for_status())
            .with_context(|| format!("requesting {}", file.url))?;
        let mut out = BufWriter::new(
            File::create(&path).with_context(|| format!("creating {}", path.display()))?,
        );
        let mut hasher = Sha256::new();
        let mut buffer = vec![0; 1 << 16];
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(Cancelled.into());
            }
            let read = response
                .read(&mut buffer)
                .with_context(|| format!("reading {}", file.url))?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            out.write_all(&buffer[..read])
                .with_context(|| format!("writing {}", path.display()))?;
            done += read as u64;
            progress(done, total);
        }
        out.flush()?;
        let hash = format!("{:x}", hasher.finalize());
        if hash != file.sha256 {
            fs::remove_file(&path).ok();
            bail!("{} does not match its checksum", file.name);
        }
    }

    delete(kind)?;
    fs::rename(&staging, &target)
        .with_context(|| format!("moving {} into place", target.display()))?;
    Ok(())
}

fn hash_file(path: &std::path::Path) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}
