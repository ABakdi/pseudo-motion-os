//! Virtual file system (Architecture spec §4.6).
//!
//! A POSIX-like tree held in kernel memory and persisted write-through by the
//! platform (OPFS on the web — the platform drains [`VfsOp`]s each frame).
//! `/sys` is synthetic: kernel state exposed as readable files. Everything
//! else is real persistent bytes.

use pmos_abi::DirEntry;
use std::collections::{BTreeMap, BTreeSet};

/// Persistence operations for the platform to mirror into real storage.
#[derive(Debug, Clone)]
pub enum VfsOp {
    Write(String, Vec<u8>),
    Delete(String),
    Mkdir(String),
}

pub struct Vfs {
    files: BTreeMap<String, Vec<u8>>,
    dirs: BTreeSet<String>,
    /// Ops awaiting persistence (drained by the platform).
    pub dirty: Vec<VfsOp>,
    /// Set once the platform finished loading persisted state.
    pub ready: bool,
    /// Live system stats surfaced under /sys.
    pub sys_fps: f32,
    /// In-browser LLM tier this machine can handle (0 Fast · 1 Balanced ·
    /// 2 Quality), probed by the platform at boot from RAM + GPU limits.
    pub sys_llm_tier: u8,
}

fn normalize(path: &str) -> String {
    let mut out = String::from("/");
    for part in path.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            // Pop one segment; never above root.
            if let Some(idx) = out.trim_end_matches('/').rfind('/') {
                out.truncate(idx + 1);
            }
            continue;
        }
        if !out.ends_with('/') {
            out.push('/');
        }
        out.push_str(part);
    }
    if out.len() > 1 && out.ends_with('/') {
        out.pop();
    }
    out
}

impl Vfs {
    pub fn new() -> Self {
        let mut v = Self {
            files: BTreeMap::new(),
            dirs: BTreeSet::new(),
            dirty: Vec::new(),
            ready: false,
            sys_fps: 0.0,
            sys_llm_tier: 1,
        };
        for d in [
            "/",
            "/home",
            "/apps",
            "/notes",
            "/notes/inbox",
            "/notes/daily",
            "/sys",
        ] {
            v.dirs.insert(d.to_string());
        }
        v
    }

    /// Load one persisted file at boot (bypasses the dirty queue).
    pub fn load(&mut self, path: &str, bytes: Vec<u8>) {
        let path = normalize(path);
        self.ensure_parents(&path, false);
        self.files.insert(path, bytes);
    }

    fn ensure_parents(&mut self, path: &str, persist: bool) {
        let mut cur = String::new();
        let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
        for part in &parts[..parts.len().saturating_sub(1)] {
            cur.push('/');
            cur.push_str(part);
            if self.dirs.insert(cur.clone()) && persist {
                self.dirty.push(VfsOp::Mkdir(cur.clone()));
            }
        }
    }

    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        let path = normalize(path);
        if let Some(sys) = self.read_sys(&path) {
            return Some(sys);
        }
        self.files.get(&path).cloned()
    }

    fn read_sys(&self, path: &str) -> Option<Vec<u8>> {
        match path {
            "/sys/fps" => Some(format!("{:.1}\n", self.sys_fps).into_bytes()),
            "/sys/abi" => Some(format!("{:?}\n", pmos_abi::ABI_VERSION).into_bytes()),
            "/sys/llm_tier" => Some(format!("{}\n", self.sys_llm_tier).into_bytes()),
            _ => None,
        }
    }

    pub fn write(&mut self, path: &str, bytes: Vec<u8>) -> Result<(), &'static str> {
        let path = normalize(path);
        if path.starts_with("/sys") {
            return Err("/sys is read-only");
        }
        if bytes.len() > 4 * 1024 * 1024 {
            return Err("file too large (max 4 MB)");
        }
        self.ensure_parents(&path, true);
        self.files.insert(path.clone(), bytes.clone());
        self.dirty.push(VfsOp::Write(path, bytes));
        Ok(())
    }

    pub fn delete(&mut self, path: &str) -> Result<(), &'static str> {
        let path = normalize(path);
        if path.starts_with("/sys") {
            return Err("/sys is read-only");
        }
        if self.files.remove(&path).is_some() {
            self.dirty.push(VfsOp::Delete(path));
            return Ok(());
        }
        if self.dirs.contains(&path) {
            // Only empty directories.
            let prefix = format!("{path}/");
            if self.files.keys().any(|f| f.starts_with(&prefix))
                || self.dirs.iter().any(|d| d.starts_with(&prefix))
            {
                return Err("directory not empty");
            }
            self.dirs.remove(&path);
            self.dirty.push(VfsOp::Delete(path));
            return Ok(());
        }
        Err("not found")
    }

    pub fn mkdir(&mut self, path: &str) -> Result<(), &'static str> {
        let path = normalize(path);
        if path.starts_with("/sys") {
            return Err("/sys is read-only");
        }
        self.ensure_parents(&format!("{path}/x"), true);
        if self.dirs.insert(path.clone()) {
            self.dirty.push(VfsOp::Mkdir(path));
        }
        Ok(())
    }

    pub fn exists_dir(&self, path: &str) -> bool {
        self.dirs.contains(&normalize(path))
    }

    pub fn list(&self, path: &str) -> Option<Vec<DirEntry>> {
        let path = normalize(path);
        if path == "/sys" {
            return Some(
                ["fps", "abi", "llm_tier"]
                    .iter()
                    .map(|n| DirEntry {
                        name: n.to_string(),
                        dir: false,
                        size: 0,
                    })
                    .collect(),
            );
        }
        if !self.dirs.contains(&path) {
            return None;
        }
        let prefix = if path == "/" {
            "/".to_string()
        } else {
            format!("{path}/")
        };
        let mut out: Vec<DirEntry> = Vec::new();
        for d in &self.dirs {
            if let Some(rest) = d.strip_prefix(&prefix) {
                if !rest.is_empty() && !rest.contains('/') {
                    out.push(DirEntry {
                        name: rest.to_string(),
                        dir: true,
                        size: 0,
                    });
                }
            }
        }
        for (f, bytes) in &self.files {
            if let Some(rest) = f.strip_prefix(&prefix) {
                if !rest.is_empty() && !rest.contains('/') {
                    out.push(DirEntry {
                        name: rest.to_string(),
                        dir: false,
                        size: bytes.len() as u64,
                    });
                }
            }
        }
        out.sort_by(|a, b| (!a.dir, &a.name).cmp(&(!b.dir, &b.name)));
        Some(out)
    }

    /// All file paths under a prefix (used by Notes for backlink scans).
    pub fn files_under(&self, prefix: &str) -> Vec<String> {
        let prefix = normalize(prefix);
        let p = if prefix == "/" {
            "/".to_string()
        } else {
            format!("{prefix}/")
        };
        self.files
            .keys()
            .filter(|k| k.starts_with(&p))
            .cloned()
            .collect()
    }
}

impl Default for Vfs {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_read_list_delete() {
        let mut v = Vfs::new();
        v.write("/notes/hello.md", b"hi [[world]]".to_vec())
            .unwrap();
        assert_eq!(v.read("/notes/hello.md").unwrap(), b"hi [[world]]");
        let entries = v.list("/notes").unwrap();
        assert!(entries.iter().any(|e| e.name == "hello.md" && !e.dir));
        assert!(entries.iter().any(|e| e.name == "inbox" && e.dir));
        v.delete("/notes/hello.md").unwrap();
        assert!(v.read("/notes/hello.md").is_none());
        // Ops recorded for persistence.
        assert!(v
            .dirty
            .iter()
            .any(|o| matches!(o, VfsOp::Write(p, _) if p == "/notes/hello.md")));
        assert!(v
            .dirty
            .iter()
            .any(|o| matches!(o, VfsOp::Delete(p) if p == "/notes/hello.md")));
    }

    #[test]
    fn sys_is_synthetic_and_readonly() {
        let mut v = Vfs::new();
        v.sys_fps = 60.0;
        assert_eq!(v.read("/sys/fps").unwrap(), b"60.0\n");
        assert!(v.write("/sys/fps", vec![]).is_err());
    }

    #[test]
    fn nested_write_creates_parents() {
        let mut v = Vfs::new();
        v.write("/home/a/b/c.txt", b"x".to_vec()).unwrap();
        assert!(v.exists_dir("/home/a/b"));
        assert!(v.list("/home/a").unwrap().iter().any(|e| e.name == "b"));
    }
}
