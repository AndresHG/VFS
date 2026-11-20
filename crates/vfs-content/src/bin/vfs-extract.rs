//! The VFS-2 extraction worker: reads requests on stdin, answers on stdout. Started by the
//! content indexer; not meant to be run by hand.

use vfs_content::worker::{worker_main, CappedAlloc};

#[global_allocator]
static ALLOC: CappedAlloc = CappedAlloc::new();

fn main() {
    worker_main(&ALLOC);
}
