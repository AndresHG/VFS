//! VFS-28 AC-3: saves, reads or deletes one item in the OS credential store through the same
//! [`OsStore`] the app uses, so `scripts/macos_signing.py ab-check` can sign two builds with one
//! identity and check that build B reads what build A saved without a prompt.
//!
//!     keychain_probe set <account> <secret>
//!     keychain_probe get <account>      # prints the secret
//!     keychain_probe delete <account>
//!
//! `VFS_PROBE_BUILD` is baked in at compile time, so two builds are two different binaries
//! (two code hashes), like two releases of the app.

use std::process::ExitCode;

use vfs_ai::{CredentialBackend, OsStore, StoreError};

const BUILD: Option<&str> = option_env!("VFS_PROBE_BUILD");

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let store = OsStore::new();
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["set", account, secret] => store.set(account, secret).map(|()| String::new()),
        ["get", account] => store.get(account),
        ["delete", account] => store.delete(account).map(|()| String::new()),
        _ => {
            eprintln!(
                "usage: keychain_probe set <account> <secret> | get <account> | delete <account>"
            );
            return ExitCode::from(2);
        }
    };
    let build = BUILD.unwrap_or("unset");
    match result {
        Ok(out) => {
            if !out.is_empty() {
                println!("{out}");
            }
            eprintln!("build {build}: ok");
            ExitCode::SUCCESS
        }
        Err(StoreError::NoEntry) => {
            eprintln!("build {build}: no entry");
            ExitCode::from(3)
        }
        Err(StoreError::Failure(why)) => {
            eprintln!("build {build}: {why}");
            ExitCode::FAILURE
        }
    }
}
