use std::path::PathBuf;

use directories::{BaseDirs, ProjectDirs};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanderePaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
}

impl PanderePaths {
    pub fn telegram_session_path(&self) -> PathBuf {
        self.data_dir.join("telegram").join("session.sqlite")
    }

    pub fn plugin_data_dir(&self, plugin_id: &str) -> PathBuf {
        self.data_dir.join("plugins").join(plugin_id)
    }
}

pub fn pandere_paths() -> PanderePaths {
    if let Some(project_dirs) = ProjectDirs::from("", "", "pandere") {
        return PanderePaths {
            config_dir: project_dirs.config_dir().to_path_buf(),
            data_dir: project_dirs.data_local_dir().to_path_buf(),
            cache_dir: project_dirs.cache_dir().to_path_buf(),
        };
    }

    fallback_paths()
}

fn fallback_paths() -> PanderePaths {
    if let Some(base_dirs) = BaseDirs::new() {
        let home = base_dirs.home_dir().join(".pandere");
        return PanderePaths {
            config_dir: home.join("config"),
            data_dir: home.join("data"),
            cache_dir: home.join("cache"),
        };
    }

    let root = PathBuf::from(".pandere");
    PanderePaths {
        config_dir: root.join("config"),
        data_dir: root.join("data"),
        cache_dir: root.join("cache"),
    }
}
