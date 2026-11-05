use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::PathBuf;

pub fn path() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(root) => PathBuf::from(root).join("apolline-wall"),
        None => std::env::temp_dir().join(format!("apolline-wall-{}", unsafe { libc::geteuid() })),
    }
}

/// Never trust a pre-existing link or another user's directory in a shared temporary folder.
pub fn ensure() -> Result<PathBuf, String> {
    ensure_at(path())
}

fn ensure_at(path: PathBuf) -> Result<PathBuf, String> {
    match std::fs::DirBuilder::new().mode(0o700).create(&path) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(err) => return Err(format!("{}: {err}", path.display())),
    }
    let metadata = std::fs::symlink_metadata(&path).map_err(|err| err.to_string())?;
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(format!(
            "{} must be a directory owned by you with permissions 0700",
            path.display()
        ));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn runtime_directory_refuses_links_and_public_permissions() {
        let root = std::env::temp_dir().join(format!("wall-runtime-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let private = root.join("private");
        assert_eq!(ensure_at(private.clone()).unwrap(), private);
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(ensure_at(private.clone()).is_err());
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o700)).unwrap();
        let link = root.join("link");
        std::os::unix::fs::symlink(&private, &link).unwrap();
        assert!(ensure_at(link).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
