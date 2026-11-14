//! Explicitly ask VoiceReader to read the clipboard, launching it if needed.
pub fn read_clipboard() -> Result<(), String> {
    #[cfg(test)]
    {
        // UI tests exercise the menu without triggering the user's real reader.
        REQUESTS.with(|count| count.set(count.get() + 1));
        Ok(())
    }
    #[cfg(all(target_os = "macos", not(test)))]
    {
        let result = std::process::Command::new("/usr/bin/open")
            .args([
                "-g",
                "-b",
                "app.voicereader.mac",
                "voicereader://read-clipboard",
            ])
            .output()
            .map_err(|error| format!("Block copied. Could not open VoiceReader: {error}"))?;
        if result.status.success() {
            Ok(())
        } else {
            Err("Block copied. Install or start the updated VoiceReader and try again.".into())
        }
    }
    #[cfg(all(not(target_os = "macos"), not(test)))]
    Err("VoiceReader is available on macOS.".into())
}

#[cfg(test)]
thread_local! {
    static REQUESTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(all(test, unix))]
pub fn requests() -> usize {
    REQUESTS.with(std::cell::Cell::get)
}
