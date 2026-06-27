//! Common test utilities.

use std::path::PathBuf;
use std::fs;

/// Temp dir pentru teste. Creează un subdirector într-un director temporar.
/// E șters automat la drop.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("neurograph_test_{}_{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("Failed to create temp dir");
        TempDir { path }
    }

    pub fn path(&self) -> &str {
        self.path.to_str().expect("Non-UTF8 path")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
