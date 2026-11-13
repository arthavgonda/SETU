use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Paths {
    pub data: PathBuf,
    pub models: PathBuf,
    pub logs: PathBuf,
    pub runtime: PathBuf,
    pub crash: PathBuf,
    pub assets: PathBuf,
    pub bin: PathBuf,
}

fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

impl Paths {
    pub fn detect() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let data = env_dir("SETU_DATA_DIR").unwrap_or_else(|| {
            if cfg!(target_os = "macos") {
                home.join("Library/Application Support/Setu")
            } else if cfg!(windows) {
                dirs::data_local_dir().unwrap_or(home.clone()).join("Setu")
            } else {
                dirs::data_dir().unwrap_or(home.join(".local/share")).join("setu")
            }
        });
        let logs = if cfg!(target_os = "macos") {
            home.join("Library/Logs/Setu")
        } else if cfg!(windows) {
            data.join("logs")
        } else {
            dirs::state_dir().unwrap_or(home.join(".local/state")).join("setu/logs")
        };
        let runtime = env_dir("SETU_RUNTIME_DIR").unwrap_or_else(|| {
            if cfg!(target_os = "linux") {
                env_dir("XDG_RUNTIME_DIR").map(|d| d.join("setu")).unwrap_or_else(|| PathBuf::from(format!("/tmp/setu-{}", uid())))
            } else if cfg!(target_os = "macos") {
                std::env::temp_dir().join("setu")
            } else {
                data.join("run")
            }
        });
        let bin = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(|| PathBuf::from("."));
        let assets = env_dir("SETU_ASSETS")
            .or_else(|| {
                [bin.join("../share/setu"), bin.join("../Resources"), bin.join("share")]
                    .into_iter()
                    .find(|p| p.join("models").is_dir())
            })
            .unwrap_or_else(|| PathBuf::from(env!("SETU_DEV_ASSETS")));
        Self { models: data.join("models"), crash: data.join("crash"), data, logs, runtime, assets, bin }
    }

    pub fn ensure(&self) -> std::io::Result<()> {
        for d in [&self.data, &self.models, &self.logs, &self.crash] {
            std::fs::create_dir_all(d)?;
        }
        std::fs::create_dir_all(&self.runtime)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.runtime, std::fs::Permissions::from_mode(0o700))?;
            std::fs::set_permissions(&self.data, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }

    pub fn socket(&self) -> PathBuf {
        self.runtime.join("daemon.sock")
    }

    pub fn db(&self) -> PathBuf {
        self.data.join("setu.db")
    }

    pub fn bundled(&self, id: &str) -> PathBuf {
        self.assets.join("models").join(id)
    }

    pub fn engine_exe(&self) -> PathBuf {
        let name = if cfg!(windows) { "setu-engine.exe" } else { "setu-engine" };
        let here = self.bin.join(name);
        if here.is_file() { here } else { PathBuf::from(name) }
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "data": self.data, "logs": self.logs, "models": self.models, "runtime": self.runtime
        })
    }
}

pub fn uid() -> u32 {
    #[cfg(unix)]
    {
        unsafe { libc::getuid() }
    }
    #[cfg(not(unix))]
    {
        0
    }
}
