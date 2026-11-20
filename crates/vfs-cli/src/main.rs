//! `vfs` command line. Placeholder until T7 adds the subcommands
//! (index, search, gen-db, gen-tree, bench, stats).

fn main() {
    eprintln!(
        "vfs {} (index schema {}): no subcommands yet",
        env!("CARGO_PKG_VERSION"),
        vfs_core::db::SCHEMA_VERSION
    );
    std::process::exit(2);
}
