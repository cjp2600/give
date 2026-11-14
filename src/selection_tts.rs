//! Send a terminal block to the user's Selection TTS service (`spp`).
pub fn read(text: &str) -> Result<(), String> {
    #[cfg(test)]
    {
        // UI tests exercise the menu without triggering the user's real reader.
        REQUESTS.with(|requests| requests.borrow_mut().push(text.to_owned()));
        Ok(())
    }
    #[cfg(all(target_os = "macos", not(test)))]
    {
        let home =
            std::env::var_os("HOME").ok_or("Cannot find read-selection: HOME is missing.")?;
        run(
            &std::path::PathBuf::from(home).join(".local/bin/read-selection"),
            text,
        )
    }
    #[cfg(all(not(target_os = "macos"), not(test)))]
    {
        let _ = text;
        Err("Selection TTS is available on macOS.".into())
    }
}

#[cfg(any(target_os = "macos", test))]
fn run(program: &std::path::Path, text: &str) -> Result<(), String> {
    use std::{
        io::Write,
        process::{Command, Stdio},
        time::Duration,
    };
    use wait_timeout::ChildExt;

    let mut child = Command::new(program)
        .arg("--stdin")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Block copied. Could not start read-selection: {error}"))?;
    let written = child.stdin.take().unwrap().write_all(text.as_bytes());
    if let Err(error) = written {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!(
            "Block copied. Could not send text to Selection TTS: {error}"
        ));
    }
    let wait_error = match child.wait_timeout(Duration::from_secs(5)) {
        Ok(Some(_)) => None,
        Ok(None) => Some("Block copied. Selection TTS did not respond within 5 seconds.".into()),
        Err(error) => Some(format!(
            "Block copied. Could not wait for Selection TTS: {error}"
        )),
    };
    if let Some(error) = wait_error {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("Could not read Selection TTS response: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let detail = String::from_utf8_lossy(&output.stderr)
            .chars()
            .take(500)
            .collect::<String>();
        Err(format!(
            "Block copied. Selection TTS could not read it: {}",
            detail.trim()
        ))
    }
}

#[cfg(test)]
thread_local! {
    static REQUESTS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(all(test, unix))]
pub fn requests() -> usize {
    REQUESTS.with(|requests| requests.borrow().len())
}

#[cfg(all(test, unix))]
pub fn last_request() -> Option<String> {
    REQUESTS.with(|requests| requests.borrow().last().cloned())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn client(directory: &std::path::Path, exit: i32) -> std::path::PathBuf {
        let path = directory.join("read-selection");
        std::fs::write(&path, format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$(dirname \"$0\")/args\"\ncat > \"$(dirname \"$0\")/input\"\necho 'test daemon response' >&2\nexit {exit}\n"
        )).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[test]
    fn passes_exact_unicode_text_through_stdin_without_shell_interpretation() {
        let directory = tempfile::tempdir().unwrap();
        let program = client(directory.path(), 0);
        let text = "Привет, ёлочка\n$HOME $(not-a-command) `literal` \"quotes\"\n";
        run(&program, text).unwrap();
        assert_eq!(
            std::fs::read(directory.path().join("input")).unwrap(),
            text.as_bytes()
        );
        assert_eq!(
            std::fs::read_to_string(directory.path().join("args")).unwrap(),
            "--stdin\n"
        );
    }

    #[test]
    fn reports_client_failure_instead_of_silently_claiming_speech_started() {
        let directory = tempfile::tempdir().unwrap();
        let program = client(directory.path(), 1);
        assert!(
            run(&program, "текст")
                .unwrap_err()
                .contains("test daemon response")
        );
    }
}
