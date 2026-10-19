use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::Result;

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

/// An output file in the destination directory, published only after writing succeeds.
pub(crate) struct AtomicOutput {
    destination: PathBuf,
    temporary: PathBuf,
    committed: bool,
}

impl AtomicOutput {
    pub(crate) fn new(destination: &str) -> Result<Self> {
        let requested = Path::new(destination);
        let resolved = match fs::symlink_metadata(requested) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                Some(fs::canonicalize(requested).map_err(|error| {
                    format!("[qbix] could not resolve output symlink {destination}: {error}")
                })?)
            }
            Ok(_) => None,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(format!(
                    "[qbix] could not inspect output {destination}: {error}"
                ));
            }
        };
        let destination = resolved.as_deref().unwrap_or(requested);
        match fs::metadata(destination) {
            Ok(metadata) if !metadata.is_file() => {
                return Err(format!(
                    "[qbix] output is not a regular file: {}",
                    destination.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "[qbix] could not inspect output {}: {error}",
                    destination.display()
                ));
            }
        }
        let parent = destination.parent().unwrap_or_else(|| Path::new("."));
        let name = destination
            .file_name()
            .ok_or_else(|| format!("[qbix] invalid output path: {}", destination.display()))?;

        for _ in 0..1000 {
            let mut temp_name = name.to_os_string();
            temp_name.push(format!(
                ".qbix-tmp-{}-{}",
                std::process::id(),
                NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed)
            ));
            let temporary = parent.join(temp_name);
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
            {
                Ok(file) => {
                    drop(file);
                    let output = Self {
                        destination: destination.to_path_buf(),
                        temporary,
                        committed: false,
                    };
                    match fs::metadata(destination) {
                        Ok(metadata) if metadata.is_file() => {
                            fs::set_permissions(&output.temporary, metadata.permissions())
                                .map_err(|error| {
                                    format!(
                                        "[qbix] could not preserve permissions for {}: {error}",
                                        destination.display()
                                    )
                                })?;
                        }
                        Ok(_) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => {
                            return Err(format!(
                                "[qbix] could not inspect output {}: {error}",
                                destination.display()
                            ));
                        }
                    }
                    return Ok(output);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(format!(
                        "[qbix] could not create temporary output for {}: {error}",
                        destination.display()
                    ));
                }
            }
        }
        Err(format!(
            "[qbix] could not reserve a temporary output for {}",
            destination.display()
        ))
    }

    pub(crate) fn path(&self) -> Result<&str> {
        self.temporary
            .to_str()
            .ok_or_else(|| "[qbix] temporary output path is not valid UTF-8".to_string())
    }

    pub(crate) fn commit(mut self) -> Result<()> {
        fs::rename(&self.temporary, &self.destination).map_err(|error| {
            format!(
                "[qbix] could not replace output {}: {error}",
                self.destination.display()
            )
        })?;
        self.committed = true;
        Ok(())
    }
}

impl Drop for AtomicOutput {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.temporary);
        }
    }
}
