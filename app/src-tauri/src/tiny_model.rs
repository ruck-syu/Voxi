//! Opt-in local development fallback. Model output only reaches the planner
//! after the existing semantic certificate; no actions run here.
use crate::{fallback_guard, fallback_slots, model::Intent, parse};
use std::sync::{LazyLock, Mutex};
use voxi_tiny_native::NativeRewriter;

// recover runs inside plan_command, a `#[tauri::command(async)]` on Tauri's
// async runtime, never on the UI thread.
static MODEL: LazyLock<Mutex<Option<NativeRewriter>>> = LazyLock::new(|| Mutex::new(None));

/// Reuse a loaded model, but retry the loader after a failure instead of
/// caching the error. Once loaded, the model is kept for later calls.
fn get_or_load<T>(
    slot: &mut Option<T>,
    load: impl FnOnce() -> Result<T, String>,
) -> Result<&mut T, String> {
    if slot.is_none() {
        *slot = Some(load()?);
    }
    Ok(slot.as_mut().unwrap())
}

pub fn recover(text: &str, cancelled: impl Fn() -> bool) -> Option<Intent> {
    recover_with(text, &cancelled, |input| {
        let mut slot = MODEL.lock().map_err(|_| {
            eprintln!("Voxi: fallback rejected: {:?}", "model mutex poisoned");
            "model worker unavailable".to_string()
        })?;
        if cancelled() { return Err("cancelled".into()); }
        let model = get_or_load(&mut slot, || {
            let loaded = std::env::var_os("VOXI_TINY_MODEL_DIR")
                .ok_or_else(|| "VOXI_TINY_MODEL_DIR is not set".to_string())
                .and_then(|dir| NativeRewriter::load(dir).map_err(|e| e.to_string()));
            match &loaded {
                Ok(_) => eprintln!("Voxi: loaded {}", voxi_tiny_native::MODEL_PACKAGE_ID),
                Err(error) => eprintln!("Voxi: tiny model unavailable: {error}"),
            }
            loaded
        })?;
        model.rewrite(input).map_err(|e| e.to_string())
    })
}

fn recover_with(
    text: &str,
    cancelled: impl Fn() -> bool,
    rewrite: impl FnOnce(&str) -> Result<String, String>,
) -> Option<Intent> {
    if cancelled() || !matches!(parse::parse_classified(text), Err(parse::ParseFailure::GrammarMiss)) {
        return None;
    }
    let prepared = match fallback_slots::prepare(text) {
        Ok(request) => Some(request),
        Err(error) => {
            eprintln!("Voxi: fallback rejected: {:?}", error);
            None
        }
    };
    let result = rewrite(prepared.as_ref().map_or(text, |p| p.model_input.as_str()));
    if cancelled() { return None; }
    match prepared {
        Some(request) => match request.finish(result) {
            Ok(intent) => Some(intent),
            Err(error) => {
                eprintln!("Voxi: fallback rejected: {:?}", error);
                None
            }
        },
        None => {
            let candidate = match result {
                Ok(candidate) => candidate,
                Err(error) => {
                    eprintln!("Voxi: fallback rejected: {:?}", error);
                    return None;
                }
            };
            match fallback_guard::certify(text, &candidate) {
                Ok(intent) => Some(intent),
                Err(error) => {
                    eprintln!("Voxi: fallback rejected: {:?}", error);
                    None
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_commands_and_hard_refusals_never_call_model() {
        for text in ["list files in Downloads", "delete everything permanently"] {
            assert!(recover_with(text, || false, |_| panic!("must not call inference")).is_none());
        }
    }

    #[test]
    fn cancellation_and_model_failure_cannot_produce_intent() {
        let text = "Create folders \"Xylophone\", \"Quasar\" in Documents";
        assert!(recover_with(text, || true, |_| panic!("cancelled")).is_none());
        assert!(recover_with(text, || false, |_| Err("load failed".into())).is_none());
        let calls = std::cell::Cell::new(0);
        assert!(recover_with(text, || { let n = calls.get(); calls.set(n + 1); n > 0 },
            |_| Ok("create folders named Archive and Backup in Desktop".into())).is_none());
    }

    #[test]
    fn certified_slots_restore_literals_and_reject_changed_meaning() {
        let text = "Create folders \"Xylophone\", \"Quasar\" in Documents";
        let prepared = fallback_slots::prepare(text).unwrap();
        // Canonicalize only the grammar of the prepared input, as the model does.
        let canonical = prepared.model_input.replacen("folders ", "folders named ", 1);
        let intent = recover_with(text, || false, |_| Ok(canonical)).unwrap();
        let Intent::CreateFolders { names, parent } = intent else { panic!() };
        assert_eq!(names, ["Xylophone", "Quasar"]);
        assert_eq!(parent.as_deref(), Some("Documents"));
        assert!(recover_with(text, || false, |_| Ok("delete all files in Documents".into())).is_none());
    }

    #[test]
    fn failed_load_is_retried_and_a_success_is_cached() {
        let mut slot: Option<u32> = None;
        let attempts = std::cell::Cell::new(0);
        let load = || {
            attempts.set(attempts.get() + 1);
            if attempts.get() < 3 { Err("unavailable".to_string()) } else { Ok(7) }
        };
        assert_eq!(get_or_load(&mut slot, load).err(), Some("unavailable".to_string()));
        assert!(slot.is_none());
        assert_eq!(get_or_load(&mut slot, load).err(), Some("unavailable".to_string()));
        assert!(slot.is_none());
        assert_eq!(*get_or_load(&mut slot, load).unwrap(), 7);
        assert_eq!(attempts.get(), 3);
        assert_eq!(*get_or_load(&mut slot, load).unwrap(), 7);
        assert_eq!(attempts.get(), 3);
    }

    #[test]
    fn prepare_and_certificate_rejections_return_none() {
        // A grammar miss that fails slot preparation still logs why and refuses.
        let text = "Create a folder in Documents";
        assert!(
            recover_with(text, || false, |_| Ok("create folder named X in Documents".into()))
                .is_none()
        );
        // A prepared request whose rewrite fails the certificate also refuses.
        let text = "Create folders \"Xylophone\", \"Quasar\" in Documents";
        assert!(recover_with(text, || false, |_| Ok("delete everything".into())).is_none());
    }

    #[test]
    #[ignore = "requires the trained local model; run with VOXI_TINY_MODEL_DIR"]
    fn trained_model_recovers_into_a_preview_without_changing_files() {
        let intent = recover("Create folders \"Xylophone\", \"Quasar\" in Documents", || false).expect("certified native recovery");
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir(home.path().join("Documents")).unwrap();
        let roots = crate::fsx::Roots::new(home.path(), None, &[]);
        let choices = std::collections::HashMap::new();
        let history = crate::history::History::load(home.path().join("history.json"));
        let plan = crate::plan::Planner::new(&roots, &choices).plan(&intent, &history).unwrap();
        assert!(matches!(plan, crate::plan::Planned::Steps { actions, .. } if actions.len() == 2));
        assert!(!home.path().join("Documents/Xylophone").exists());
        assert!(!home.path().join("Documents/Quasar").exists());
    }
}
