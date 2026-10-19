//! Linux cleanup rule catalog. M4.1 establishes the shared contract; later
//! clean units register evidence-backed rules here.

pub mod automation;
pub mod browser;
pub mod clean;
pub mod flatpak;
pub mod editor_cache;
pub mod journal;
pub mod optimize;
pub mod orphans;
pub mod owner_cache;
pub mod project_cache;
pub mod python;
pub mod system;
