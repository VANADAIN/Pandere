use std::{fs, io, path::PathBuf};

use directories::{BaseDirs, ProjectDirs};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanderePaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
}

impl PanderePaths {
    pub fn plugin_install_dir(&self) -> PathBuf {
        self.data_dir.join("plugins")
    }

    pub fn telegram_session_path(&self) -> PathBuf {
        self.data_dir.join("telegram").join("session.sqlite")
    }

    pub fn plugin_data_dir(&self, plugin_id: &str) -> PathBuf {
        self.plugin_install_dir().join(plugin_id)
    }

    pub fn media_cache_dir(&self) -> PathBuf {
        self.cache_dir.join("media")
    }

    pub fn ensure_exists(&self) -> io::Result<()> {
        fs::create_dir_all(&self.config_dir)?;
        fs::create_dir_all(&self.data_dir)?;
        fs::create_dir_all(&self.cache_dir)?;
        fs::create_dir_all(self.plugin_install_dir())?;
        fs::create_dir_all(self.media_cache_dir())?;
        Ok(())
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
