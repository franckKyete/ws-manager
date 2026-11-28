pub mod collector;
pub mod config;
pub mod db;
pub mod export;
pub mod focus;
pub mod ids;
pub mod model;
pub mod presence;
pub mod presence_wayland;
pub mod rollup;
pub mod snapshot;
pub mod sync;

pub use config::{ActivityConfig, ActivityPaths, ProjectActivityConfig};
pub use db::ActivityDb;
pub use model::*;

pub fn default_paths() -> ActivityPaths {
    ActivityPaths::discover()
}
pub fn load_activity_config() -> Result<ActivityConfig, String> {
    ActivityConfig::load()
}
pub fn open_db(paths: &ActivityPaths) -> Result<ActivityDb, String> {
    paths.ensure_dirs()?;
    ActivityDb::open(&paths.db_path)
}
pub fn open_db_readonly(paths: &ActivityPaths) -> Result<ActivityDb, String> {
    ActivityDb::open_readonly(&paths.db_path)
}
