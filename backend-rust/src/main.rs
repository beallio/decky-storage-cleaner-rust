//! Storage Cleaner backend: measures and deletes per-app shader cache and
//! compatibility data under `~/.steam/steam/steamapps`.

use std::{
    collections::HashSet,
    fs, io,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

use decky_plugin::{
    Plugin,
    serde_json::{Value, json},
};

const KINDS: [&str; 2] = ["shadercache", "compatdata"];

#[tokio::main(flavor = "current_thread")]
async fn main() -> io::Result<()> {
    Plugin::new()
        .method("scan", |context, args| async move {
            let root = kind_root(&context.env().decky_user_home, args.first())?;
            blocking(move || scan(&root)).await
        })
        .method("delete", |context, args| async move {
            let root = kind_root(&context.env().decky_user_home, args.first())?;
            let appids: Vec<String> = match args.get(1) {
                Some(Value::Array(items)) => items
                    .iter()
                    .map(|item| item.as_str().map(str::to_owned))
                    .collect::<Option<_>>()
                    .ok_or("appids must be strings")?,
                _ => return Err("appids must be an array".into()),
            };
            blocking(move || delete(&root, &appids)).await
        })
        .run()
        .await
}

async fn blocking(work: impl FnOnce() -> Value + Send + 'static) -> Result<Value, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| error.to_string())
}

/// Trust boundary: the frontend picks the kind, never a path.
fn kind_root(home: &Path, kind: Option<&Value>) -> Result<PathBuf, String> {
    match kind.and_then(Value::as_str) {
        _ if home.as_os_str().is_empty() => Err("DECKY_USER_HOME is not set".into()),
        Some(kind) if KINDS.contains(&kind) => Ok(home.join(".steam/steam/steamapps").join(kind)),
        _ => Err(format!("kind must be one of {KINDS:?}")),
    }
}

fn is_appid(name: &str) -> bool {
    !name.is_empty() && name.len() <= 20 && name.bytes().all(|byte| byte.is_ascii_digit())
}

/// Lists numeric app folders with their disk usage, largest first.
/// A missing root is an empty list: Steam creates it on demand.
fn scan(root: &Path) -> Value {
    let mut entries: Vec<(String, u64)> = fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| is_appid(name))
        .map(|appid| {
            let bytes = disk_usage(&root.join(&appid));
            (appid, bytes)
        })
        .collect();
    entries.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    json!({
        "entries": entries
            .into_iter()
            .map(|(appid, bytes)| json!({ "appid": appid, "bytes": bytes }))
            .collect::<Vec<_>>()
    })
}

/// Allocated bytes under `dir` like `du`: never follows symbolic links, and
/// counts a hard-linked file once. Unreadable folders are skipped.
fn disk_usage(dir: &Path) -> u64 {
    let mut total = fs::symlink_metadata(dir).map_or(0, |meta| meta.blocks() * 512);
    let mut seen = HashSet::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(read) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            // DirEntry::metadata does not follow symbolic links.
            let Ok(meta) = entry.metadata() else { continue };
            if meta.nlink() > 1 && !meta.is_dir() && !seen.insert((meta.dev(), meta.ino())) {
                continue;
            }
            total += meta.blocks() * 512;
            if meta.is_dir() {
                pending.push(entry.path());
            }
        }
    }
    total
}

/// Deletes each app folder. A symbolic link or a non-folder is refused, so a
/// delete cannot reach data outside `root`.
fn delete(root: &Path, appids: &[String]) -> Value {
    let mut deleted = Vec::new();
    let mut failed = Vec::new();
    for appid in appids {
        let result = if is_appid(appid) {
            let path = root.join(appid);
            match fs::symlink_metadata(&path) {
                Ok(meta) if meta.is_dir() => remove_app_dir(&path).map_err(|e| e.to_string()),
                Ok(_) => Err("not a folder".into()),
                Err(error) => Err(error.to_string()),
            }
        } else {
            Err("invalid appid".into())
        };
        match result {
            Ok(()) => deleted.push(json!(appid)),
            Err(error) => {
                log::warn!("delete {appid} failed: {error}");
                failed.push(json!({ "appid": appid, "error": error }));
            }
        }
    }
    json!({ "deleted": deleted, "failed": failed })
}

/// Some prefixes contain folders without owner write permission, which block
/// `remove_dir_all`. On that error, give the owner full access to every folder
/// inside (symbolic links are not followed) and try once more.
fn remove_app_dir(path: &Path) -> io::Result<()> {
    match fs::remove_dir_all(path) {
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            // shortcut: path-based walk; a folder swapped for a symlink between the check and the
            // chmod could redirect one chmod. The backend runs as the same user who owns the files,
            // so this grants nothing new; switch to openat/fchmod if it ever runs as root.
            let mut pending = vec![path.to_path_buf()];
            while let Some(dir) = pending.pop() {
                let Ok(meta) = fs::symlink_metadata(&dir) else {
                    continue;
                };
                if !meta.is_dir() {
                    continue;
                }
                let mode = meta.permissions().mode();
                if mode & 0o700 != 0o700 {
                    let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(mode | 0o700));
                }
                let Ok(read) = fs::read_dir(&dir) else {
                    continue;
                };
                pending.extend(
                    read.flatten()
                        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                        .map(|entry| entry.path()),
                );
            }
            fs::remove_dir_all(path)
        }
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn temp_root(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("storage-cleaner-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn write(path: &Path, len: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![1u8; len]).unwrap();
    }

    #[test]
    fn kind_root_accepts_only_known_kinds() {
        let home = Path::new("/home/deck");
        assert_eq!(
            kind_root(home, Some(&json!("compatdata"))).unwrap(),
            Path::new("/home/deck/.steam/steam/steamapps/compatdata")
        );
        for bad in [json!("../.."), json!("compatdata/1"), json!(""), json!(1)] {
            assert!(kind_root(home, Some(&bad)).is_err(), "{bad}");
        }
        assert!(kind_root(home, None).is_err());
    }

    #[test]
    fn scan_lists_numeric_folders_largest_first() {
        let root = temp_root("scan");
        write(&root.join("10/a"), 100);
        write(&root.join("20/deep/b"), 100_000);
        write(&root.join("not-an-app/c"), 100_000);
        write(&root.join("30"), 100_000); // a file, not a folder
        let outside = temp_root("scan-outside");
        write(&outside.join("big"), 1_000_000);
        symlink(&outside, root.join("40")).unwrap();
        symlink(&outside, root.join("10/link")).unwrap();

        let entries = scan(&root)["entries"].as_array().unwrap().clone();
        let ids: Vec<_> = entries
            .iter()
            .map(|e| e["appid"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["20", "10"]);
        let small = entries[1]["bytes"].as_u64().unwrap();
        assert!(small < 1_000_000, "followed a symlink: {small}");
        assert!(entries[0]["bytes"].as_u64().unwrap() >= 100_000);
        assert_eq!(scan(&root.join("missing")), json!({ "entries": [] }));
    }

    #[test]
    fn hard_links_count_once() {
        let root = temp_root("hardlink");
        write(&root.join("1/a"), 1_000_000);
        let once = disk_usage(&root.join("1"));
        fs::hard_link(root.join("1/a"), root.join("1/b")).unwrap();
        assert_eq!(disk_usage(&root.join("1")), once);
    }

    #[test]
    fn delete_removes_only_valid_app_folders() {
        let root = temp_root("delete");
        let outside = temp_root("delete-outside");
        write(&outside.join("keep"), 10);
        write(&root.join("1/x/y"), 10);
        write(&root.join("2"), 10);
        symlink(&outside, root.join("3")).unwrap();

        let ids = ["1", "2", "3", "4", "..", "1/../..", ""].map(String::from);
        let result = delete(&root, &ids);
        assert_eq!(result["deleted"], json!(["1"]));
        let failed: Vec<_> = result["failed"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["appid"].as_str().unwrap())
            .collect();
        assert_eq!(failed, ["2", "3", "4", "..", "1/../..", ""]);
        assert!(!root.join("1").exists());
        assert!(root.join("2").exists());
        assert!(outside.join("keep").exists());
    }

    #[test]
    fn delete_clears_read_only_folders_without_touching_link_targets() {
        let root = temp_root("readonly");
        let outside = temp_root("readonly-outside");
        write(&outside.join("keep"), 10);
        fs::set_permissions(&outside, fs::Permissions::from_mode(0o555)).unwrap();
        write(&root.join("5/pfx/ro/deep/f"), 10);
        write(&root.join("5/pfx/none/f"), 10);
        symlink(&outside, root.join("5/pfx/link")).unwrap();
        for (dir, mode) in [
            ("5/pfx/ro/deep", 0o555),
            ("5/pfx/ro", 0o555),
            ("5/pfx/none", 0o000),
            ("5", 0o555),
        ] {
            fs::set_permissions(root.join(dir), fs::Permissions::from_mode(mode)).unwrap();
        }

        assert_eq!(delete(&root, &["5".into()])["deleted"], json!(["5"]));
        assert!(!root.join("5").exists());
        assert_eq!(
            fs::metadata(&outside).unwrap().permissions().mode() & 0o777,
            0o555
        );
        assert!(outside.join("keep").exists());
        fs::set_permissions(&outside, fs::Permissions::from_mode(0o755)).unwrap();
    }
}
