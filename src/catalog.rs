//! Clipboard imports and recoverable deletion for the working-tree catalog.
use crate::clipboard_file::Image;
use anyhow::{Context, Result, bail, ensure};
use std::{
    ffi::OsStr,
    fs::{self, File},
    io::{self, Write},
    path::{Component, Path, PathBuf},
};

pub enum Paste {
    Files(Vec<PathBuf>),
    Images(Vec<Image>),
}

/// A folder receives a paste inside itself; a file receives it in its parent.
pub fn paste_directory(path: Option<&Path>, directory: bool) -> PathBuf {
    match path {
        Some(path) if directory => path.to_owned(),
        Some(path) => path.parent().unwrap_or(Path::new("")).to_owned(),
        None => PathBuf::new(),
    }
}

fn project_directory(root: &Path, relative: &Path) -> Result<PathBuf> {
    ensure!(
        relative
            .components()
            .all(|part| matches!(part, Component::Normal(_))),
        "Invalid catalog path"
    );
    let root = root.canonicalize().context("Could not locate project")?;
    let directory = root
        .join(relative)
        .canonicalize()
        .context("Could not locate destination folder")?;
    ensure!(
        directory.starts_with(&root) && directory.is_dir(),
        "Destination must be a folder in this project"
    );
    Ok(directory)
}

/// Stage bytes in the destination filesystem, then publish without overwriting
/// any existing entry (including dangling symlinks). Source files stay intact.
fn import_file(
    directory: &Path,
    name: &OsStr,
    write: impl FnOnce(&mut File) -> Result<()>,
) -> Result<PathBuf> {
    let mut staged =
        tempfile::NamedTempFile::new_in(directory).context("Could not create pasted file")?;
    write(staged.as_file_mut())?;
    let path = Path::new(name);
    let stem = path.file_stem().unwrap_or(name);
    for index in 0u64.. {
        let name = if index == 0 {
            name.to_owned()
        } else {
            let mut name = stem.to_os_string();
            name.push(format!(" ({index})"));
            if let Some(extension) = path.extension() {
                name.push(".");
                name.push(extension);
            }
            name
        };
        let destination = directory.join(name);
        match staged.persist_noclobber(&destination) {
            Ok(_) => return Ok(destination),
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                staged = error.file;
            }
            Err(error) => return Err(error.error).context("Could not save pasted file"),
        }
    }
    unreachable!()
}

pub fn paste(root: &Path, directory: &Path, payload: Paste) -> Result<Vec<PathBuf>> {
    let directory = project_directory(root, directory)?;
    match payload {
        Paste::Files(paths) => {
            ensure!(!paths.is_empty(), "No files in clipboard");
            // Validate the entire selection before copying anything.
            for path in &paths {
                ensure!(
                    path.is_file(),
                    "Only files can be pasted: {}",
                    path.display()
                );
            }
            paths
                .into_iter()
                .map(|path| {
                    let name = path.file_name().context("Clipboard file has no name")?;
                    import_file(&directory, name, |target| {
                        let mut source = File::open(&path)
                            .with_context(|| format!("Could not open {}", path.display()))?;
                        io::copy(&mut source, target).context("Could not copy clipboard file")?;
                        target.set_permissions(source.metadata()?.permissions())?;
                        Ok(())
                    })
                })
                .collect()
        }
        Paste::Images(images) => {
            ensure!(!images.is_empty(), "No images in clipboard");
            images
                .into_iter()
                .map(|image| {
                    ensure!(
                        matches!(
                            image.extension,
                            "png" | "jpg" | "webp" | "gif" | "svg" | "bmp" | "tiff"
                        ),
                        "Unsupported clipboard image format"
                    );
                    let name = format!("image.{}", image.extension);
                    import_file(&directory, OsStr::new(&name), |target| {
                        target
                            .write_all(&image.bytes)
                            .context("Could not write clipboard image")
                    })
                })
                .collect()
        }
    }
}

/// Delete only the selected file, keeping it recoverable in the system Trash.
pub fn trash(root: &Path, path: &Path) -> Result<()> {
    let parent = path.parent().context("No file selected")?;
    let directory = project_directory(root, parent)?;
    let name = path.file_name().context("No file selected")?;
    let path = directory.join(name);
    ensure!(
        !fs::symlink_metadata(&path)?.is_dir(),
        "Select a file to delete"
    );
    #[cfg(target_os = "macos")]
    return native::trash(&path);
    #[cfg(not(target_os = "macos"))]
    bail!("Moving files to Trash is not supported on this platform")
}

/// GPUI's clipboard API does not expose Finder file URLs.
#[cfg(target_os = "macos")]
pub fn read_clipboard() -> Option<Paste> {
    native::read_clipboard()
}

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use std::{
        ffi::{CStr, CString, c_char, c_void},
        os::unix::ffi::OsStrExt,
    };

    unsafe extern "C" {
        fn give_catalog_clipboard_files(
            context: *mut c_void,
            append: extern "C" fn(*mut c_void, *const c_char),
        );
        fn give_catalog_clipboard_image(
            context: *mut c_void,
            append: extern "C" fn(*mut c_void, *const c_char, *const u8, usize),
        );
        fn give_catalog_trash(
            path: *const c_char,
            context: *mut c_void,
            error: extern "C" fn(*mut c_void, *const c_char),
        ) -> bool;
    }

    extern "C" fn append_file(context: *mut c_void, path: *const c_char) {
        // Native callbacks are synchronous; both pointers live for this call.
        unsafe {
            let files = &mut *context.cast::<Vec<PathBuf>>();
            files.push(PathBuf::from(OsStr::from_bytes(
                CStr::from_ptr(path).to_bytes(),
            )));
        }
    }

    extern "C" fn append_image(
        context: *mut c_void,
        extension: *const c_char,
        bytes: *const u8,
        len: usize,
    ) {
        unsafe {
            let images = &mut *context.cast::<Vec<Image>>();
            let extension = match CStr::from_ptr(extension).to_bytes() {
                b"png" => "png",
                b"jpg" => "jpg",
                b"webp" => "webp",
                b"gif" => "gif",
                b"svg" => "svg",
                b"bmp" => "bmp",
                b"tiff" => "tiff",
                _ => return,
            };
            images.push(Image {
                extension,
                bytes: std::slice::from_raw_parts(bytes, len).to_vec(),
            });
        }
    }

    pub fn read_clipboard() -> Option<Paste> {
        let mut files = Vec::<PathBuf>::new();
        unsafe {
            give_catalog_clipboard_files((&mut files as *mut Vec<PathBuf>).cast(), append_file)
        };
        if !files.is_empty() {
            return Some(Paste::Files(files));
        }
        let mut images = Vec::<Image>::new();
        unsafe {
            give_catalog_clipboard_image((&mut images as *mut Vec<Image>).cast(), append_image)
        };
        (!images.is_empty()).then_some(Paste::Images(images))
    }

    extern "C" fn set_error(context: *mut c_void, error: *const c_char) {
        if !error.is_null() {
            unsafe {
                *context.cast::<String>() = CStr::from_ptr(error).to_string_lossy().into_owned();
            }
        }
    }

    pub fn trash(path: &Path) -> Result<()> {
        let path = CString::new(path.as_os_str().as_bytes())?;
        let mut error = String::from("Could not move file to Trash");
        let success = unsafe {
            give_catalog_trash(path.as_ptr(), (&mut error as *mut String).cast(), set_error)
        };
        if success { Ok(()) } else { bail!(error) }
    }
}
