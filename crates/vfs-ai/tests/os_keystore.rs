//! VFS-28 AC-1 (crate part, R2): a key saved in the OS credential store (macOS login keychain,
//! Windows Credential Manager) reads back, then is deleted. A throwaway account is used so a
//! real saved key is never touched.

use vfs_ai::{CredentialBackend, OsStore, StoreError};

/// VFS-28 AC-1: the stored credential reads back, and is gone after deletion.
#[test]
fn credential_reads_back_and_deletes() {
    let account = format!(
        "vfs-ai-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let store = OsStore::new();
    let secret = "sk-test-os-store-roundtrip-4321";
    store.set(&account, secret).expect("save in the OS store");
    let read = store.get(&account);
    store.delete(&account).expect("delete from the OS store");
    assert_eq!(read.as_deref(), Ok(secret));
    assert_eq!(store.get(&account), Err(StoreError::NoEntry));
}
