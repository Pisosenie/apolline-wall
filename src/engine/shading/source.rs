use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

const LIMIT: u64 = 1024 * 1024;

pub(super) fn read(path: &Path) -> Result<String, String> {
    let fail = |error| format!("{}: {error}", path.display());
    let resolved = path.canonicalize().map_err(fail)?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(resolved)
        .map_err(fail)?;
    let metadata = file.metadata().map_err(fail)?;
    if !metadata.is_file() || metadata.len() > LIMIT {
        return Err(format!(
            "{}: shader requires a regular UTF-8 file no larger than 1 MiB",
            path.display()
        ));
    }
    let mut text = String::new();
    file.take(LIMIT + 1)
        .read_to_string(&mut text)
        .map_err(fail)?;
    if text.len() as u64 > LIMIT {
        return Err(format!("{}: shader grew past 1 MiB", path.display()));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shader_sources_are_bounded_regular_utf8_files() {
        let path = std::env::temp_dir().join(format!("wall-shader-source-{}", std::process::id()));
        std::fs::write(&path, "void mainImage() {}\n").unwrap();
        assert_eq!(read(&path).unwrap(), "void mainImage() {}\n");
        std::fs::write(&path, [255]).unwrap();
        assert!(read(&path).is_err());
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(LIMIT + 1)
            .unwrap();
        assert!(read(&path).unwrap_err().contains("1 MiB"));
        assert!(read(&std::env::temp_dir()).is_err());
        assert!(read(Path::new("/dev/zero")).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
