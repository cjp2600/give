use give::{
    catalog::{self, Paste},
    clipboard_file::Image,
};
use std::{fs, path::Path};

#[test]
fn paste_uses_the_folder_or_the_files_parent_including_root() {
    for (selected, directory, expected) in [
        (Some("assets"), true, "assets"),
        (Some("assets/nested"), true, "assets/nested"),
        (Some("assets/photo.png"), false, "assets"),
        (Some("README.md"), false, ""),
        (None, false, ""),
    ] {
        assert_eq!(
            catalog::paste_directory(selected.map(Path::new), directory),
            Path::new(expected)
        );
    }
}

#[test]
fn external_files_keep_names_bytes_and_permissions_without_overwriting() {
    let source = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("assets")).unwrap();
    let original = source.path().join("фото with spaces.png");
    let bytes = [0, 255, 128, 10, 0];
    fs::write(&original, bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&original, fs::Permissions::from_mode(0o754)).unwrap();
    }
    for name in ["фото with spaces.png", "фото with spaces (1).png"] {
        fs::write(root.path().join("assets").join(name), "existing").unwrap();
    }
    let pasted = catalog::paste(
        root.path(),
        Path::new("assets"),
        Paste::Files(vec![original.clone()]),
    )
    .unwrap();
    assert_eq!(pasted[0].file_name().unwrap(), "фото with spaces (2).png");
    assert_eq!(fs::read(&pasted[0]).unwrap(), bytes);
    assert_eq!(fs::read(&original).unwrap(), bytes);
    assert_eq!(
        fs::read(root.path().join("assets/фото with spaces.png")).unwrap(),
        b"existing"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&pasted[0]).unwrap().permissions().mode() & 0o777,
            0o754
        );
    }
    assert_eq!(fs::read_dir(root.path().join("assets")).unwrap().count(), 3);
}

#[test]
fn multiple_files_and_repeated_paste_into_the_same_directory_are_copies() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("a.txt");
    let b = root.path().join(".hidden");
    fs::write(&a, "a").unwrap();
    fs::write(&b, "b").unwrap();
    let pasted = catalog::paste(
        root.path(),
        Path::new(""),
        Paste::Files(vec![a.clone(), b.clone()]),
    )
    .unwrap();
    assert_eq!(pasted[0].file_name().unwrap(), "a (1).txt");
    assert_eq!(pasted[1].file_name().unwrap(), ".hidden (1)");
    assert_eq!(fs::read_to_string(a).unwrap(), "a");
    assert_eq!(fs::read_to_string(b).unwrap(), "b");
}

#[test]
fn raw_images_get_unique_names_with_their_actual_format_and_bytes() {
    let root = tempfile::tempdir().unwrap();
    let images = vec![
        Image {
            extension: "png",
            bytes: vec![137, 80, 78, 71],
        },
        Image {
            extension: "png",
            bytes: vec![1, 2, 3],
        },
        Image {
            extension: "tiff",
            bytes: vec![73, 73, 42, 0],
        },
    ];
    let pasted = catalog::paste(root.path(), Path::new(""), Paste::Images(images)).unwrap();
    for (path, name, bytes) in [
        (&pasted[0], "image.png", &[137, 80, 78, 71][..]),
        (&pasted[1], "image (1).png", &[1, 2, 3][..]),
        (&pasted[2], "image.tiff", &[73, 73, 42, 0][..]),
    ] {
        assert_eq!(path.file_name().unwrap(), name);
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}

#[test]
fn invalid_selection_does_not_leave_partial_copies_or_temporary_files() {
    let source = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let original = source.path().join("file.txt");
    fs::write(&original, "hello").unwrap();
    assert!(
        catalog::paste(
            root.path(),
            Path::new(""),
            Paste::Files(vec![original, source.path().join("missing")])
        )
        .is_err()
    );
    assert!(
        catalog::paste(
            root.path(),
            Path::new(""),
            Paste::Files(vec![source.path().to_owned()])
        )
        .is_err()
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    assert!(catalog::trash(root.path(), Path::new("")).is_err());
    fs::create_dir(root.path().join("folder")).unwrap();
    assert!(catalog::trash(root.path(), Path::new("folder")).is_err());
    assert!(root.path().join("folder").is_dir());
}

#[cfg(unix)]
#[test]
fn dangling_symlinks_are_not_overwritten_and_outside_destinations_are_rejected() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    symlink(root.path().join("missing"), root.path().join("image.png")).unwrap();
    symlink(outside.path(), root.path().join("outside")).unwrap();
    let image = || {
        Paste::Images(vec![Image {
            extension: "png",
            bytes: vec![1],
        }])
    };
    let pasted = catalog::paste(root.path(), Path::new(""), image()).unwrap();
    assert_eq!(pasted[0].file_name().unwrap(), "image (1).png");
    assert!(
        fs::symlink_metadata(root.path().join("image.png"))
            .unwrap()
            .is_symlink()
    );
    for directory in [Path::new("outside"), Path::new("../"), outside.path()] {
        assert!(catalog::paste(root.path(), directory, image()).is_err());
    }
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
}
