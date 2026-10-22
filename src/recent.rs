//! Recently opened projects, newest first, kept in a small JSON list.
use std::path::{Path, PathBuf};

const LIMIT: usize = 10;

fn home() -> Option<PathBuf> {
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
    #[cfg(not(windows))]
    let home = std::env::var_os("HOME");
    home.map(PathBuf::from)
}

/// Recent projects in the platform's application configuration directory.
pub fn store() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    let base = home()?.join("Library/Application Support");
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| home().map(|home| home.join("AppData/Roaming")))?;
    #[cfg(not(any(target_os = "macos", windows)))]
    let base = {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home().map(|home| home.join(".config")))?
    };
    Some(base.join("give/recent.json"))
}

/// The stored list without projects that no longer exist.
pub fn load(file: &Path) -> Vec<PathBuf> {
    let Ok(text) = std::fs::read_to_string(file) else {
        return Vec::new();
    };
    let paths: Vec<String> = serde_json::from_str(&text).unwrap_or_default();
    paths
        .into_iter()
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .take(LIMIT)
        .collect()
}

pub fn save(file: &Path, projects: &[PathBuf]) -> anyhow::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let paths: Vec<_> = projects.iter().map(|p| p.to_string_lossy()).collect();
    std::fs::write(file, serde_json::to_string_pretty(&paths)?)?;
    Ok(())
}

/// Moves `project` to the front, dropping its older entry and the overflow.
pub fn remember(projects: &mut Vec<PathBuf>, project: &Path) {
    projects.retain(|p| p != project);
    projects.insert(0, project.to_path_buf());
    projects.truncate(LIMIT);
}

/// The path with the home directory shortened to `~`.
pub fn display(path: &Path) -> String {
    home()
        .and_then(|home| path.strip_prefix(home).ok())
        .map(|rest| Path::new("~").join(rest).to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembers_newest_first_without_duplicates() {
        let mut projects = vec![PathBuf::from("/a"), PathBuf::from("/b")];
        remember(&mut projects, Path::new("/b"));
        assert_eq!(projects, [PathBuf::from("/b"), PathBuf::from("/a")]);
        for i in 0..20 {
            remember(&mut projects, Path::new(&format!("/p{i}")));
        }
        assert_eq!(projects.len(), LIMIT);
        assert_eq!(projects[0], PathBuf::from("/p19"));
    }

    #[test]
    fn round_trips_and_skips_missing_projects() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("nested/recent.json");
        let projects = vec![dir.path().to_path_buf(), dir.path().join("gone")];
        save(&file, &projects).unwrap();
        assert_eq!(load(&file), [dir.path().to_path_buf()]);
        assert!(load(&dir.path().join("missing.json")).is_empty());
    }
}
