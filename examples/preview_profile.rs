//! Reproducible local timings. Run on the Mac with `cargo run --example preview_profile`.
use give::{
    git::{Repository, parse_status},
    preview_cache::{Key, PreviewCache},
    tree,
};
use std::{collections::HashSet, fs, hint::black_box, process::Command, time::Instant};

fn main() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "--initial-branch=main"]);
    git(&["config", "user.name", "Give profile"]);
    git(&["config", "user.email", "give@example.invalid"]);
    git(&["config", "commit.gpgsign", "false"]);
    git(&["config", "core.hooksPath", "/dev/null"]);
    let text = (0..5500)
        .map(|i| format!("let value_{i} = {i}; // source line\n"))
        .collect::<String>();
    fs::write(dir.path().join("large.rs"), &text)?;
    git(&["add", "--all"]);
    git(&["commit", "-m", "Initial"]);
    fs::write(
        dir.path().join("large.rs"),
        text.replace("let value_2500 = 2500;", "let added_value = 42;"),
    )?;
    let repo = Repository::open(dir.path())?;
    let start = Instant::now();
    let snapshot = repo.snapshot()?;
    println!("Snapshot: {:.2} ms", start.elapsed().as_secs_f64() * 1000.);
    let file = &snapshot.files[0];
    let key = Key {
        path: file.path.clone(),
        version: snapshot.versions[&file.path],
    };
    let mut cache = PreviewCache::default();
    let slot = cache.slot(key.clone());
    let start = Instant::now();
    let pair = slot.load(&repo, file).map_err(anyhow::Error::msg)?;
    println!(
        "Cold content ({} lines, both modes): {:.2} ms",
        pair.normal.lines.len(),
        start.elapsed().as_secs_f64() * 1000.
    );
    let start = Instant::now();
    slot.highlight_range(false, 0..48);
    println!(
        "Lazy syntax, first 48 rows: {:.2} ms",
        start.elapsed().as_secs_f64() * 1000.
    );
    let start = Instant::now();
    for i in 0..10000 {
        let slot = cache.slot(key.clone());
        let pair = slot.ready().unwrap().as_ref().unwrap();
        black_box((pair.mode(i % 2 == 0), slot.styles(i % 2 == 0)));
    }
    println!(
        "Cached open / D, mean of 10,000 lookups: {:.4} ms (excludes GPUI paint)",
        start.elapsed().as_secs_f64() * 1000. / 10000.
    );
    let status = (0..20000)
        .map(|i| format!(" M crates/{}/src/module_{i}.rs\0", i / 100))
        .collect::<String>();
    let files = parse_status(status.as_bytes())?;
    let start = Instant::now();
    let rows = tree::build_tree(&files, &HashSet::new());
    println!(
        "20,000-file tree ({} rows, background): {:.2} ms",
        rows.len(),
        start.elapsed().as_secs_f64() * 1000.
    );
    println!(
        "Cached preview memory: {:.2} MiB",
        cache.bytes() as f64 / (1024. * 1024.)
    );
    Ok(())
}
