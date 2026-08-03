use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Result;

/// Replace a file through a sibling temporary file so readers never observe a
/// partially-written TOML document.
pub fn atomic_write(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;

    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let temporary = parent.join(format!(".{name}.tmp-{}-{stamp}", std::process::id()));

    let result = (|| -> io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(contents)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        sync_directory(parent)?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(Into::into)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::atomic_write;

    #[test]
    fn replaces_file_without_leaving_temporary_files() {
        let dir = std::env::temp_dir().join(format!("optionterm-storage-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("state.toml");

        atomic_write(&path, b"first").expect("write first state");
        atomic_write(&path, b"second").expect("replace state");

        assert_eq!(fs::read_to_string(&path).expect("read state"), "second");
        assert_eq!(fs::read_dir(&dir).expect("read temp dir").count(), 1);
        fs::remove_dir_all(dir).expect("remove temp dir");
    }
}
