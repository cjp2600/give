//! Launch another independent give instance using the running installation.
use anyhow::Context as _;
use std::{
    path::Path,
    process::{Command, Stdio},
};

pub(crate) fn launch(project: &Path) -> anyhow::Result<()> {
    let executable = std::env::current_exe().context("locating the give executable")?;
    launch_from(&executable, project)
}

fn launch_from(executable: &Path, project: &Path) -> anyhow::Result<()> {
    let (mut command, wait_for_launcher) = launch_command(executable, project);
    command.stdin(Stdio::null()).stdout(Stdio::null());
    if wait_for_launcher {
        let output = command.output().context("starting a new give instance")?;
        anyhow::ensure!(
            output.status.success(),
            "macOS could not launch give: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    } else {
        let mut child = command
            .stderr(Stdio::null())
            .spawn()
            .context("starting a new give process")?;
        // Reap the child when it exits without tying its lifetime to this window.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
    Ok(())
}

fn launch_command(executable: &Path, project: &Path) -> (Command, bool) {
    #[cfg(target_os = "macos")]
    if let Some(bundle) = executable
        .parent()
        .filter(|path| path.ends_with("Contents/MacOS"))
        .and_then(|path| path.parent()?.parent())
        .filter(|path| path.extension().is_some_and(|ext| ext == "app"))
    {
        // Launch Services needs -n to create a process instead of activating the
        // existing instance. Use this exact bundle, even with other installs.
        let mut command = Command::new("/usr/bin/open");
        command.arg("-n").arg(bundle).arg("--args").arg(project);
        return (command, true);
    }
    let mut command = Command::new(executable);
    command.arg(project);
    (command, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "macos")]
    use std::ffi::OsStr;

    #[cfg(unix)]
    #[test]
    fn launches_an_independent_process_with_the_project_path_intact() {
        use std::{
            fs,
            os::unix::fs::PermissionsExt,
            time::{Duration, Instant},
        };

        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("give");
        let project = directory.path().join("Пример with spaces; $HOME");
        fs::create_dir(&project).unwrap();
        fs::write(
            &executable,
            "#!/bin/sh\nprintf '%s\\n' \"$$\" \"$#\" \"$1\" > \"$1/launched\"\n",
        )
        .unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        launch_from(&executable, &project).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let output = loop {
            if let Ok(output) = fs::read_to_string(project.join("launched"))
                && output.lines().count() == 3
            {
                break output;
            }
            assert!(
                Instant::now() < deadline,
                "child did not receive the project"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        let lines = output.lines().collect::<Vec<_>>();
        assert_ne!(lines[0].parse::<u32>().unwrap(), std::process::id());
        assert_eq!(lines[1], "1");
        assert_eq!(lines[2], project.to_str().unwrap());
    }

    #[test]
    fn source_build_launches_same_executable_with_one_project_argument() {
        let executable = Path::new("/some checkout/target/debug/give");
        let project = Path::new("/projects/Пример with spaces; $HOME");
        let (command, wait) = launch_command(executable, project);
        assert_eq!(command.get_program(), executable.as_os_str());
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [project.as_os_str()]
        );
        assert!(!wait);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn mac_bundle_launches_fresh_instance_of_exact_installation() {
        let bundle = OsStr::new("/Applications/My give.app");
        let project = OsStr::new("/projects/Пример with spaces");
        let (command, wait) = launch_command(
            Path::new(bundle).join("Contents/MacOS/give").as_path(),
            Path::new(project),
        );
        assert_eq!(command.get_program(), "/usr/bin/open");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [OsStr::new("-n"), bundle, OsStr::new("--args"), project]
        );
        assert!(wait);
    }
}
