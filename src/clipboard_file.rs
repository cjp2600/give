//! Persist clipboard payloads before passing their paths to a shell.
use anyhow::{Context, Result, ensure};
use std::{fs::OpenOptions, io::Write, path::PathBuf};

pub struct Image {
    pub extension: &'static str,
    pub bytes: Vec<u8>,
}

/// A private, unique directory per paste. Keep it after success: the command or
/// another process can read these paths long after the clipboard has changed.
pub fn save(images: Vec<Image>) -> Result<Vec<PathBuf>> {
    ensure!(!images.is_empty(), "No clipboard images");
    let directory = tempfile::Builder::new()
        .prefix("give-clipboard-")
        .tempdir()
        .context("Could not create a clipboard temporary directory")?;
    let mut paths = Vec::new();
    for (index, image) in images.into_iter().enumerate() {
        ensure!(
            matches!(
                image.extension,
                "png" | "jpg" | "webp" | "gif" | "svg" | "bmp" | "tiff"
            ),
            "Unsupported clipboard image format"
        );
        let path = directory
            .path()
            .join(format!("image-{}.{}", index + 1, image.extension));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&path)
            .context("Could not save the clipboard image")?;
        file.write_all(&image.bytes)
            .context("Could not write the clipboard image")?;
        paths.push(path.canonicalize()?);
    }
    let _ = directory.keep();
    Ok(paths)
}

pub fn shell_paths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| format!("'{}' ", path.to_string_lossy().replace('\'', "'\\''")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saves_exact_bytes_to_unique_private_files_that_outlive_clipboard() {
        let make = || {
            save(vec![Image {
                extension: "png",
                bytes: vec![0, 255, 17, 32],
            }])
            .unwrap()
        };
        let first = make();
        let second = make();
        assert_ne!(first, second);
        for path in first.into_iter().chain(second) {
            assert!(path.is_absolute());
            assert_eq!(std::fs::read(&path).unwrap(), [0, 255, 17, 32]);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
            std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        }
    }
    #[test]
    fn quotes_paths_as_literal_shell_arguments() {
        let paths = vec![PathBuf::from("/tmp/a b/it's $(touch nope).png")];
        let quoted = shell_paths(&paths);
        let output = std::process::Command::new("/bin/sh")
            .args(["-c", &format!("printf '%s' {quoted}")])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            paths[0].to_str().unwrap()
        );
    }
}
