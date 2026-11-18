//! Compare actual application planning/execution in independently built binaries.
use std::{fs, process::Command};
use serde_json::Value;

fn normalize(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                if ["id", "time", "scheduledFor", "due"].contains(&key.as_str()) && !value.is_null() {
                    *value = Value::String("<timestamp-or-id>".into());
                } else { normalize(value); }
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
        if home.exists() { fs::remove_dir_all(&home).unwrap(); }
        fs::create_dir(&home).unwrap();
        // Cargo holds its target lock while this test runs. Execute the two
        // library test artifacts directly rather than recursively invoking Cargo.
        // Build both with cargo test --lib [--features tiny-model] first.
        let deps = std::env::current_exe().expect("locate integration test executable")
            .parent().expect("integration test deps directory").to_path_buf();
        let variant = if tiny { "tiny-model" } else { "default" };
        let binary = registered_binary(&deps, variant).unwrap_or_else(|e| panic!("{e}"));
        let mut command = Command::new(binary);
        command.args(["--exact", "result_context_tests::rule_only_probe"])
            .env("VOXI_EQUIVALENCE_HOME", &home)
            .env_remove("VOXI_PLAN_MODEL_DIR").env_remove("VOXI_TINY_MODEL_DIR");
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
fn registered_binary(deps: &std::path::Path, variant: &str) -> Result<std::path::PathBuf, String> {
    let instruction = format!("Run cargo test --locked --lib{} first, with the same target directory and profile.",
        if variant == "tiny-model" { " --features tiny-model" } else { "" });
    let manifest = deps.join(format!("voxi-equivalence-{variant}.json"));
    let binary: std::path::PathBuf = serde_json::from_slice(&fs::read(&manifest)
        .map_err(|e| format!("Missing {variant} library test registration: {e}. {instruction}"))?)
        .map_err(|e| format!("Invalid {variant} registration: {e}. {instruction}"))?;
    let built = fs::metadata(&binary).and_then(|m| m.modified())
        .map_err(|e| format!("Missing {variant} binary: {e}. {instruction}"))?;
    fn newest(path: &std::path::Path) -> std::io::Result<std::time::SystemTime> {
        let mut time = fs::metadata(path)?.modified()?;
        if path.is_dir() {
            for entry in fs::read_dir(path)? { time = time.max(newest(&entry?.path())?); }
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
    fs::File::options().write(true).open(&binary).unwrap()
        .set_times(fs::FileTimes::new().set_modified(std::time::SystemTime::UNIX_EPOCH)).unwrap();
    assert!(registered_binary(dir.path(), "default").unwrap_err().contains("Stale default library binary"));
}
