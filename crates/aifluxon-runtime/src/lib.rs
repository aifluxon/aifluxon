pub mod budget;
pub mod context;
mod continuation;
pub mod coordinator;
pub mod json_store;
pub mod operations;
pub mod run_table;
pub mod runtime;
pub mod skills;
pub mod state;
pub mod terminal;
pub mod tools;

pub use budget::*;
pub use context::*;
pub use coordinator::*;
pub use json_store::*;
pub use operations::*;
pub use run_table::*;
pub use runtime::*;
pub use skills::*;
pub use state::*;
pub use terminal::*;
pub use tools::*;

#[cfg(test)]
mod crate_boundary {
    #[test]
    fn runtime_manifest_forbids_host_and_platform_dependencies() {
        let manifest = include_str!("../Cargo.toml");
        for forbidden in [
            "tauri",
            "windows-sys",
            "nix",
            "portable-pty",
            "aifluxon-auth",
        ] {
            assert!(
                !manifest.contains(forbidden),
                "aifluxon-runtime must not depend on {forbidden}"
            );
        }
    }
}
