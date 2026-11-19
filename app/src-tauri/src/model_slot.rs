//! Loading the optional native model, shared by the dev-only fallbacks.
use std::ffi::OsString;
use std::fmt::Display;

/// Reuse a loaded value, but retry the loader after a failure instead of
/// caching the error. Once loaded, the value is kept for later calls.
pub fn get_or_load<T>(slot: &mut Option<T>, load: impl FnOnce() -> Result<T, String>) -> Result<&mut T, String> {
    if slot.is_none() {
        *slot = Some(load()?);
    }
    Ok(slot.as_mut().expect("slot was just filled"))
}

/// Loads a model package from the folder named by `var`, logging the outcome.
pub fn load_from_env<T, E: Display>(
    var: &str,
    label: &str,
    package_id: &str,
    load: impl FnOnce(OsString) -> Result<T, E>,
) -> Result<T, String> {
    let loaded = std::env::var_os(var).ok_or_else(|| format!("{var} is not set")).and_then(|dir| load(dir).map_err(|e| e.to_string()));
    match &loaded {
        Ok(_) => eprintln!("Voxi: loaded {package_id}"),
        Err(error) => eprintln!("Voxi: {label} unavailable: {error}"),
    }
    loaded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_load_is_retried_and_a_success_is_cached() {
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
        assert_eq!(attempts.get(), 3); // cached Ok is reused
    }

    #[test]
    fn a_missing_variable_names_itself() {
        let err = load_from_env("VOXI_TEST_UNSET_MODEL_DIR", "test model", "pkg", |_| Ok::<_, String>(())).unwrap_err();
        assert_eq!(err, "VOXI_TEST_UNSET_MODEL_DIR is not set");
    }
}
