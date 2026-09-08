use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

/// Replace a file through a sibling temporary file so readers never observe a
/// partially-written TOML document.
pub fn atomic_write(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let permissions = match fs::metadata(path) {
        Ok(metadata) => Some(metadata.permissions()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => None,
        Err(err) => return Err(err.into()),
    };

    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let temporary = parent.join(format!(".{name}.tmp-{}-{stamp}", std::process::id()));

    let result = (|| -> io::Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&temporary)?;
        file.write_all(contents)?;
        if let Some(permissions) = permissions {
            #[cfg(unix)]
            let permissions = fs::Permissions::from_mode(permissions.mode() & 0o777);
            file.set_permissions(permissions)?;
        }
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

    #[cfg(unix)]
    #[test]
    fn replacements_preserve_permissions_and_new_files_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = crate::test_support::TestDir::new("storage-permissions");
        let path = dir.path().join("state.toml");
        for mode in [0o600, 0o640] {
            fs::write(&path, "original").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            atomic_write(&path, b"replacement").unwrap();
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                mode
            );
        }
        let new = dir.path().join("new.toml");
        atomic_write(&new, b"private").unwrap();
        assert_eq!(
            fs::metadata(new).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

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
