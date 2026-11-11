//! # vibe-local-core
//!
//! Hardware probing, MoE-vs-Dense recommendation, Hugging Face GGUF discovery
//! (SSRF-guarded), zero-dependency GGUF parsing, the 10-pillar benchmark
//! catalog math and the unified SQLite persistence layer for Vibe Local.
//!
//! Licensed under Apache-2.0.

pub mod catalog;
pub mod db;
pub mod gguf;
pub mod hardware;
pub mod hf_crawler;
pub mod recommender;
pub mod util;

pub use catalog::{Catalog, CatalogEntry, PILLARS};
pub use db::{Db, SharedDb};
pub use hardware::{HardwareProbe, HardwareReport};
pub use recommender::{Recommendation, Tier, recommend};

/// Resolve the database path: explicit override > portable USB > default dir.
pub fn resolve_db_path(explicit: Option<&std::path::Path>, portable: bool) -> std::path::PathBuf {
    if let Some(p) = explicit {
        return p.to_path_buf();
    }
    if portable {
        let info = hardware::locate_portable();
        if let Some(root) = info.root {
            return std::path::PathBuf::from(root).join("vibe_local.db");
        }
    }
    hardware::default_data_dir().join("vibe_local.db")
}

/// Resolve the models directory matching the database location.
pub fn resolve_models_dir(
    explicit: Option<&std::path::Path>,
    portable: bool,
) -> std::path::PathBuf {
    if let Some(p) = explicit {
        return p.to_path_buf();
    }
    if portable {
        let info = hardware::locate_portable();
        if let Some(root) = info.root {
            return std::path::PathBuf::from(root).join("models");
        }
    }
    hardware::default_data_dir().join("models")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_path_resolution() {
        let explicit = std::path::Path::new("/tmp/x.db");
        assert_eq!(resolve_db_path(Some(explicit), false), explicit);
        let default = resolve_db_path(None, false);
        assert!(default.ends_with("vibe_local.db"));
    }
}
