//! Small raw state-file backend used by chip models. State bytes stay opaque to this layer.
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct StateFile { path: PathBuf, file: File }

impl StateFile {
    /// Open an existing exact-sized state file. `Ok(None)` means it does not exist yet.
    pub fn open(path: impl AsRef<Path>, expected: usize) -> Result<Option<(Self, Vec<u8>)>, String> {
        let path = path.as_ref();
        let mut file = match OpenOptions::new().read(true).write(true).open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("open {}: {}", path.display(), e)),
        };
        let actual = file.metadata().map_err(|e| format!("stat {}: {}", path.display(), e))?.len() as usize;
        if actual != expected { return Err(format!("{}: state size is {} bytes, expected {}", path.display(), actual, expected)); }
        let mut bytes = vec![0; expected];
        file.read_exact(&mut bytes).map_err(|e| format!("read {}: {}", path.display(), e))?;
        Ok(Some((StateFile { path: path.to_path_buf(), file }, bytes)))
    }

    /// Atomically publish initial state without modifying any seed artifact.
    pub fn create(path: impl AsRef<Path>, bytes: &[u8]) -> Result<Self, String> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {}", parent.display(), e))?; }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("state");
        let tmp = path.with_file_name(format!(".{}.{}.tmp", name, std::process::id()));
        let mut opts = OpenOptions::new(); opts.write(true).create_new(true);
        #[cfg(unix)]
        { use std::os::unix::fs::OpenOptionsExt; opts.mode(0o600); }
        let result = (|| {
            let mut file = opts.open(&tmp).map_err(|e| format!("create {}: {}", tmp.display(), e))?;
            file.write_all(bytes).map_err(|e| format!("write {}: {}", tmp.display(), e))?;
            file.sync_all().map_err(|e| format!("flush {}: {}", tmp.display(), e))?;
            std::fs::rename(&tmp, path).map_err(|e| format!("publish {}: {}", path.display(), e))?;
            OpenOptions::new().read(true).write(true).open(path).map_err(|e| format!("open {}: {}", path.display(), e))
        })();
        if result.is_err() { let _ = std::fs::remove_file(&tmp); }
        result.map(|file| StateFile { path: path.to_path_buf(), file })
    }

    pub fn write_range(&mut self, offset: usize, bytes: &[u8]) -> Result<(), String> {
        self.file.seek(SeekFrom::Start(offset as u64)).map_err(|e| format!("seek {}: {}", self.path.display(), e))?;
        self.file.write_all(bytes).map_err(|e| format!("write {}: {}", self.path.display(), e))?;
        self.file.flush().map_err(|e| format!("flush {}: {}", self.path.display(), e))
    }
    pub fn sync(&mut self) -> Result<(), String> { self.file.sync_data().map_err(|e| format!("sync {}: {}", self.path.display(), e)) }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn path(name: &str) -> PathBuf { std::env::temp_dir().join(format!("esp32sim-storage-{}-{}", std::process::id(), name)) }
    #[test]
    fn creates_reopens_and_updates_exact_state() {
        let path = path("roundtrip.bin"); let _ = std::fs::remove_file(&path);
        let mut state = StateFile::create(&path, &[0xff; 16]).unwrap(); state.write_range(4, &[1, 2, 3]).unwrap(); state.sync().unwrap(); drop(state);
        let (_, bytes) = StateFile::open(&path, 16).unwrap().unwrap(); assert_eq!(&bytes[4..7], &[1, 2, 3]); std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn rejects_wrong_size() {
        let path = path("size.bin"); let _ = std::fs::remove_file(&path); std::fs::write(&path, [0u8; 3]).unwrap();
        assert!(StateFile::open(&path, 4).unwrap_err().contains("expected 4")); std::fs::remove_file(path).unwrap();
    }
}
