use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub struct PidFileGuard {
    path: PathBuf,
}

impl PidFileGuard {
    pub fn create(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create pid directory {}", parent.display()))?;
        }

        fs::write(path, std::process::id().to_string())
            .with_context(|| format!("failed to write pid file {}", path.display()))?;

        Ok(Self {
            path: path.to_path_buf(),
        })
    }
}

impl Drop for PidFileGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub fn read_pid(path: impl AsRef<Path>) -> Result<u32> {
    let path = path.as_ref();
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read pid file {}", path.display()))?;
    let pid = raw
        .trim()
        .parse::<u32>()
        .with_context(|| format!("invalid pid file {}", path.display()))?;
    Ok(pid)
}

pub fn pid_alive(pid: u32) -> bool {
    Path::new("/proc").join(pid.to_string()).exists()
}

pub fn send_sighup(pid: u32) -> Result<()> {
    let result = unsafe { libc::kill(pid as i32, libc::SIGHUP) };
    if result == 0 {
        Ok(())
    } else {
        Err(anyhow!(std::io::Error::last_os_error()))
    }
}
