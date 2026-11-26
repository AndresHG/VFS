//! The VFS-29 embedding worker: loads the local model, reads requests on stdin, answers on
//! stdout. Started by VFS; not meant to be run by hand.

fn main() {
    vfs_semantic::worker::worker_main();
}
