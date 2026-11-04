//! The operating system's credential store, as the last place the API key is looked for.
//!
//! **Why it exists.** A key file is protected by file permissions; the Keychain (macOS) and Credential
//! Manager (Windows) encrypt it at rest and tie it to the signed-in user, and the MCP host's configuration then holds
//! nothing at all. It is the best place on a personal machine, and it is only ever an *option*: the lookup order in
//! `main` is flag, environment, key file, then this, so an operator who set one of the others is never second-guessed.
//!
//! **Only where there is always one.** On Linux the Secret Service needs a desktop session and a system library, and
//! a headless server has neither, so there this module reports "no credential store here" and the key file is the
//! route. A missing entry is not an error (it is "not set"); a store that cannot be reached is, and is logged
//! without any key in it.
//!
//! It is written to by the binary itself (`--store-key`, key on stdin) rather than by an installer script calling an
//! OS tool, so the key never appears on a command line and the entry belongs to the program that reads it.

use crate::config::Secret;

/// The entry's name in the store. One key per user: a second dRofus account would need its own entry.
pub const SERVICE: &str = "drofus-mcp";
pub const ACCOUNT: &str = "api-key";

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, ACCOUNT).map_err(|e| format!("the OS credential store could not be opened: {e}"))
}

/// Whether this build can use an OS credential store at all.
pub const fn available() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}

/// The stored key, `None` when there is none (or no store on this platform).
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn load() -> Result<Option<Secret>, String> {
    match entry()?.get_password() {
        Ok(key) if key.trim().is_empty() => Ok(None),
        Ok(key) => Ok(Some(Secret::new(key.trim()))),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("the API key could not be read from the OS credential store: {e}")),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn load() -> Result<Option<Secret>, String> {
    Ok(None)
}

/// Keep `key` in the store, replacing any earlier one.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn store(key: &str) -> Result<(), String> {
    entry()?
        .set_password(key.trim())
        .map_err(|e| format!("the API key could not be stored in the OS credential store: {e}"))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn store(_key: &str) -> Result<(), String> {
    Err(no_store())
}

/// Remove the stored key; removing one that is not there is fine.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn clear() -> Result<(), String> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("the API key could not be removed from the OS credential store: {e}")),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn clear() -> Result<(), String> {
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn no_store() -> String {
    "there is no OS credential store on this platform; keep the key in a file and set DROFUS_API_KEY_FILE".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real store is never touched by a unit test (it would write to the developer's Keychain); the live
    /// round trip is a manual check. What can be pinned here is the platform contract.
    #[test]
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    fn where_there_is_no_store_it_says_so_and_reads_nothing() {
        assert!(!available());
        assert_eq!(load().unwrap(), None);
        assert!(store("k").unwrap_err().contains("DROFUS_API_KEY_FILE"));
        assert!(clear().is_ok());
    }

    #[test]
    fn the_entry_is_named_for_this_program() {
        assert_eq!((SERVICE, ACCOUNT), ("drofus-mcp", "api-key"));
    }
}
