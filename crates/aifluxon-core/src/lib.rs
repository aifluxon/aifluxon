pub mod artifact;
pub mod authority;
pub mod capability;
pub mod error;
pub mod event;
pub mod hash;
pub mod ids;
pub mod message;
pub mod operation;
pub mod provider;
pub mod run;
pub mod skill;
pub mod tool;

pub use artifact::*;
pub use authority::*;
pub use capability::*;
pub use error::*;
pub use event::*;
pub use hash::content_hash;
pub use ids::*;
pub use message::*;
pub use operation::*;
pub use provider::*;
pub use run::*;
pub use skill::*;
pub use tool::*;

#[cfg(test)]
mod crate_boundary {
    #[test]
    fn core_manifest_forbids_host_and_platform_dependencies() {
        let manifest = include_str!("../Cargo.toml");
        for forbidden in [
            "tauri",
            "windows-sys",
            "nix",
            "portable-pty",
            "reqwest",
            "rusqlite",
        ] {
            assert!(
                !manifest.contains(forbidden),
                "aifluxon-core must not depend on {forbidden}"
            );
        }
    }
}
