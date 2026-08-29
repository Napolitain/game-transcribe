use std::{
    fs::{self, File},
    io::{BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ModelKind {
    #[default]
    TinyQ5_1,
    BaseQ5_1,
}

impl ModelKind {
    pub const ALL: [Self; 2] = [Self::TinyQ5_1, Self::BaseQ5_1];

    pub const fn label(self) -> &'static str {
        match self {
            Self::TinyQ5_1 => "Tiny Q5 (fast, 31 MB)",
            Self::BaseQ5_1 => "Base Q5 (more accurate, 57 MB)",
        }
    }

    pub fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or_default()
    }

    pub const fn index(self) -> usize {
        match self {
            Self::TinyQ5_1 => 0,
            Self::BaseQ5_1 => 1,
        }
    }

    fn spec(self) -> ModelSpec {
        match self {
            Self::TinyQ5_1 => ModelSpec {
                file_name: "ggml-tiny-q5_1.bin",
                url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny-q5_1.bin",
                sha256: "818710568da3ca15689e31a743197b520007872ff9576237bda97bd1b469c3d7",
            },
            Self::BaseQ5_1 => ModelSpec {
                file_name: "ggml-base-q5_1.bin",
                url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base-q5_1.bin",
                sha256: "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898",
            },
        }
    }
}

struct ModelSpec {
    file_name: &'static str,
    url: &'static str,
    sha256: &'static str,
}

pub fn ensure_installed(kind: ModelKind, models_dir: &Path) -> Result<PathBuf> {
    fs::create_dir_all(models_dir)
        .with_context(|| format!("failed to create model directory {}", models_dir.display()))?;
    let spec = kind.spec();
    let target = models_dir.join(spec.file_name);
    if target.exists() {
        if verify_sha256(&target, spec.sha256)? {
            return Ok(target);
        }
        bail!(
            "the installed {} model failed checksum verification; remove {} and retry",
            kind.label(),
            target.display()
        );
    }

    let temporary = models_dir.join(format!("{}.download", spec.file_name));
    let result = download_verified(spec.url, spec.sha256, &temporary);
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    crate::platform::files::replace_file(&temporary, &target)?;
    Ok(target)
}

fn download_verified(url: &str, expected_sha256: &str, destination: &Path) -> Result<()> {
    let mut response = ureq::get(url)
        .header("User-Agent", "GameTranscribe/0.1")
        .call()
        .context("model download failed")?;
    let mut reader = response.body_mut().as_reader();
    let file = File::create(destination)
        .with_context(|| format!("failed to create {}", destination.display()))?;
    let mut writer = BufWriter::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .context("failed while downloading model")?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        writer.write_all(&buffer[..read])?;
    }
    writer.flush()?;
    let actual = format!("{:x}", hasher.finalize());
    if actual != expected_sha256 {
        bail!("downloaded model checksum mismatch");
    }
    Ok(())
}

fn verify_sha256(path: &Path, expected: &str) -> Result<bool> {
    let mut reader = BufReader::new(
        File::open(path).with_context(|| format!("failed to open {}", path.display()))?,
    );
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()) == expected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_checksum_is_verified() {
        let path =
            std::env::temp_dir().join(format!("game-transcribe-hash-{}", std::process::id()));
        fs::write(&path, b"abc").unwrap();
        assert!(
            verify_sha256(
                &path,
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
            )
            .unwrap()
        );
        let _ = fs::remove_file(path);
    }
}
