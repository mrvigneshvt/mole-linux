use std::env;
use std::path::{Path, PathBuf};

/// Resolved XDG user directories. Invalid or relative XDG overrides fall back
/// to the specification defaults beneath `home`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct XdgPaths {
    pub home: PathBuf,
    pub config: PathBuf,
    pub data: PathBuf,
    pub state: PathBuf,
    pub cache: PathBuf,
}

impl XdgPaths {
    pub fn from_env() -> Option<Self> {
        let home = env::var_os("HOME").map(PathBuf::from)?;
        Self::from_values(
            home,
            env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
            env::var_os("XDG_DATA_HOME").map(PathBuf::from),
            env::var_os("XDG_STATE_HOME").map(PathBuf::from),
            env::var_os("XDG_CACHE_HOME").map(PathBuf::from),
        )
        .ok()
    }

    pub fn from_values(
        home: PathBuf,
        config: Option<PathBuf>,
        data: Option<PathBuf>,
        state: Option<PathBuf>,
        cache: Option<PathBuf>,
    ) -> Result<Self, &'static str> {
        if !home.is_absolute()
            || home
                .components()
                .any(|part| part == std::path::Component::ParentDir)
        {
            return Err("HOME must be absolute and contain no parent traversal");
        }
        Ok(Self {
            config: xdg_or_default(config, &home.join(".config")),
            data: xdg_or_default(data, &home.join(".local/share")),
            state: xdg_or_default(state, &home.join(".local/state")),
            cache: xdg_or_default(cache, &home.join(".cache")),
            home,
        })
    }

    pub fn roots(&self) -> [&Path; 4] {
        [&self.config, &self.data, &self.state, &self.cache]
    }
}

fn xdg_or_default(value: Option<PathBuf>, fallback: &Path) -> PathBuf {
    value
        .filter(|path| {
            path.is_absolute()
                && !path
                    .components()
                    .any(|part| part == std::path::Component::ParentDir)
                && !path
                    .as_os_str()
                    .as_encoded_bytes()
                    .iter()
                    .any(|byte| *byte < 0x20 || *byte == 0x7f)
        })
        .unwrap_or_else(|| fallback.to_path_buf())
}
