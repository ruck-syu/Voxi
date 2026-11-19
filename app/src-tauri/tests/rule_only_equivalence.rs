//! Compare actual application planning/execution in independently built binaries.
use serde_json::Value;
use std::{fs, process::Command};

fn normalize(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                if ["id", "time", "scheduledFor", "due"].contains(&key.as_str()) && !value.is_null() {
                    *value = Value::String("<timestamp-or-id>".into());
                } else {
                    normalize(value);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(normalize),
        _ => {}
    }
}

#[test]
fn serialized_public_outcomes_match_without_models() {
    let dir = tempfile::tempdir().unwrap();
    // Both processes get the exact same paths; paths and errors are not normalized.
    let home = dir.path().join("fixture");
    let mut results = Vec::new();
    for tiny in [false, true] {
        if home.exists() {
            fs::remove_dir_all(&home).unwrap();
        }
        fs::create_dir(&home).unwrap();
        // Cargo holds its target lock while this test runs. Execute the two
        // library test artifacts directly rather than recursively invoking Cargo.
        // Build both with cargo test --lib [--features tiny-model] first.
        // Library and integration test binaries can live in different
        // per-artifact folders (newer Cargo build-dir layout), so registrations
        // go in the shared profile folder.
        let deps = profile_dir(&std::env::current_exe().expect("locate integration test executable"));
        let variant = if tiny { "tiny-model" } else { "default" };
        let binary = registered_binary(&deps, variant).unwrap_or_else(|e| panic!("{e}"));
        let mut command = Command::new(binary);
        command
            .args(["--exact", "result_context_tests::rule_only_probe"])
            .env("VOXI_EQUIVALENCE_HOME", &home)
            .env_remove("VOXI_PLAN_MODEL_DIR")
            .env_remove("VOXI_TINY_MODEL_DIR");
        let output = command.output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let mut value: Value = serde_json::from_slice(&fs::read(home.join("outcomes.json")).unwrap()).unwrap();
        normalize(&mut value);
        results.push(value);
    }
    assert_eq!(results[0], results[1], "default/tiny-model public outcomes or side effects differ");
}

// The lib suites register their exact executable after Cargo builds them. This
// remains deterministic even when deps contains stale artifacts with the same
// test names, and works with any target directory/profile.
/// The profile folder (e.g. `target/debug`) above a test binary, in both the
/// `debug/deps/<bin>` and `debug/build/<pkg>/<hash>/out/<bin>` layouts.
fn profile_dir(exe: &std::path::Path) -> std::path::PathBuf {
    exe.ancestors()
        .skip(1)
        .find(|dir| matches!(dir.file_name().and_then(|n| n.to_str()), Some("deps" | "build")))
        .and_then(std::path::Path::parent)
        .unwrap_or_else(|| exe.parent().expect("test binary folder"))
        .to_path_buf()
}

fn registered_binary(deps: &std::path::Path, variant: &str) -> Result<std::path::PathBuf, String> {
    let instruction = format!(
        "Run cargo test --locked --lib{} first, with the same target directory and profile.",
        if variant == "tiny-model" { " --features tiny-model" } else { "" }
    );
    let manifest = deps.join(format!("voxi-equivalence-{variant}.json"));
    let binary: std::path::PathBuf = serde_json::from_slice(
        &fs::read(&manifest).map_err(|e| format!("Missing {variant} library test registration: {e}. {instruction}"))?,
    )
    .map_err(|e| format!("Invalid {variant} registration: {e}. {instruction}"))?;
    let built = fs::metadata(&binary).and_then(|m| m.modified()).map_err(|e| format!("Missing {variant} binary: {e}. {instruction}"))?;
    fn newest(path: &std::path::Path) -> std::io::Result<std::time::SystemTime> {
        let mut time = fs::metadata(path)?.modified()?;
        if path.is_dir() {
            for entry in fs::read_dir(path)? {
                time = time.max(newest(&entry?.path())?);
            }
        }
        Ok(time)
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for source in ["src", "Cargo.toml", "Cargo.lock", "build.rs"] {
        if newest(&root.join(source)).map_err(|e| e.to_string())? > built {
            return Err(format!("Stale {variant} library binary (older than {source}). {instruction}"));
        }
    }
    Ok(binary)
}

#[test]
fn registrations_use_the_shared_profile_folder_in_both_cargo_layouts() {
    let old = std::path::Path::new("/t/debug/deps/voxi_lib-1");
    let new = std::path::Path::new("/t/debug/build/voxi/abc/out/voxi_lib-abc");
    assert_eq!(profile_dir(old), std::path::Path::new("/t/debug"));
    assert_eq!(profile_dir(new), std::path::Path::new("/t/debug"));
}

#[test]
fn missing_variant_has_actionable_build_instructions() {
    let dir = tempfile::tempdir().unwrap();
    let error = registered_binary(dir.path(), "tiny-model").unwrap_err();
    assert!(error.contains("cargo test --locked --lib --features tiny-model"));
    assert!(error.contains("same target directory and profile"));
}

#[test]
fn registration_selects_exact_artifact_and_rejects_stale_binary() {
    let dir = tempfile::tempdir().unwrap();
    let binary = dir.path().join("registered");
    fs::write(&binary, "current library binary").unwrap();
    fs::write(dir.path().join("voxi_lib-newer-unregistered"), "unrelated").unwrap();
    fs::write(dir.path().join("voxi-equivalence-default.json"), serde_json::to_vec(&binary).unwrap()).unwrap();
    assert_eq!(registered_binary(dir.path(), "default").unwrap(), binary);
    fs::File::options()
        .write(true)
        .open(&binary)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(std::time::SystemTime::UNIX_EPOCH))
        .unwrap();
    assert!(registered_binary(dir.path(), "default").unwrap_err().contains("Stale default library binary"));
}
