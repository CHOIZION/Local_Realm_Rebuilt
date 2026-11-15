//! Consistent SQLite snapshots and transactional, path-checked ZIP restoration.
use crate::world::{Result, World};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{self},
    path::{Component, Path, PathBuf},
};
use zip::{write::SimpleFileOptions, ZipArchive, ZipWriter};

fn backup_path(world: &World, filename: &str) -> Result<PathBuf> {
    if filename.is_empty() || filename.contains(['/', '\\', ':']) || !filename.ends_with(".zip") {
        return Err("Invalid snapshot filename".into());
    }
    world.output_file(&format!("backups/{filename}"))
}
pub fn snapshot(world: &World, destination: &Path) -> Result<()> {
    world.flush_files()?;
    let parent = destination
        .parent()
        .ok_or("Missing destination directory")?;
    let db_dir = tempfile::tempdir()?;
    let db_copy = db_dir.path().join("world.db");
    world
        .db
        .execute("VACUUM INTO ?1", [db_copy.to_string_lossy().as_ref()])?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    let temporary_path = temp.path().to_owned();
    {
        let mut archive = ZipWriter::new(temp.as_file_mut());
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o644);
        let mut entries = Vec::new();
        for entry in walkdir::WalkDir::new(&world.root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                if e.depth() == 0 {
                    return true;
                }
                let name = e.file_name().to_string_lossy();
                !(e.depth() == 1 && ["backups", "exports", ".git"].contains(&name.as_ref()))
                    && !name.starts_with(".localrealm-")
            })
        {
            let entry = entry?;
            if entry.file_type().is_symlink() {
                return Err("Snapshots cannot follow links outside a realm".into());
            }
            if !entry.file_type().is_file() {
                continue;
            }
            if entry.path() == temporary_path || entry.path() == destination {
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(&world.root)?
                .to_string_lossy()
                .replace('\\', "/");
            if [
                "world.db",
                "world.db-wal",
                "world.db-shm",
                "world.db-journal",
            ]
            .contains(&relative.as_str())
            {
                continue;
            }
            entries.push((relative, entry.path().to_owned()));
        }
        entries.push(("world.db".into(), db_copy));
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, path) in entries {
            archive.start_file(name, options)?;
            io::copy(&mut File::open(path)?, &mut archive)?;
        }
        archive.finish()?;
    }
    temp.as_file().sync_all()?;
    temp.persist(destination)?;
    Ok(())
}

pub fn extract(archive_path: &Path, destination: &Path) -> Result<()> {
    let root = destination.canonicalize()?;
    let mut archive = ZipArchive::new(File::open(archive_path)?)?;
    if archive.len() > 100_000 {
        return Err("Archive contains too many files".into());
    }
    let mut total = 0u64;
    let mut names = HashSet::new();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index)?;
        let name = file.name().replace('\\', "/");
        let part = Path::new(&name);
        if name.contains(':')
            || part
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
            || name.is_empty()
        {
            return Err("Archive contains an unsafe file path".into());
        }
        if !names.insert(name.to_lowercase()) {
            return Err("Archive contains duplicate paths".into());
        }
        if file.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            return Err("Archive contains a symbolic link".into());
        }
        total = total
            .checked_add(file.size())
            .ok_or("Archive is too large")?;
        if total > 50 * 1024 * 1024 * 1024 || file.size() > 10 * 1024 * 1024 * 1024 {
            return Err("Archive exceeds the restoration size limit".into());
        }
        let target = root.join(part);
        if file.is_dir() {
            fs::create_dir_all(&target)?;
            continue;
        }
        fs::create_dir_all(target.parent().ok_or("Missing archive parent")?)?;
        let mut output = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(target)?;
        // Read to EOF so the ZIP reader verifies the entry's CRC.
        let copied = io::copy(&mut file, &mut output)?;
        if copied != file.size() {
            return Err("Archive entry length does not match its header".into());
        }
        output.sync_all()?;
    }
    Ok(())
}

pub fn restore(archive: &Path, destination: &Path) -> Result<String> {
    let parent = destination.parent().ok_or("Choose a destination folder")?;
    if destination.exists()
        && (!destination.is_dir() || fs::read_dir(destination)?.next().is_some())
    {
        return Err("Restore into an empty folder".into());
    }
    let staged = tempfile::Builder::new()
        .prefix(".localrealm-restore-")
        .tempdir_in(parent)?;
    extract(archive, staged.path())?;
    let restored = World::open(staged.path())?;
    let integrity: String = restored
        .db
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if integrity != "ok" {
        return Err("The snapshot database failed its integrity check".into());
    }
    if restored
        .db
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
            r.get::<_, i64>(0)
        })?
        != 0
    {
        return Err("The snapshot contains broken database references".into());
    }
    drop(restored);
    if destination.exists() {
        fs::remove_dir(destination)?;
    }
    if let Err(error) = fs::rename(staged.path(), destination) {
        let _ = fs::create_dir(destination);
        return Err(error.into());
    }
    Ok(dunce::simplified(&destination.canonicalize()?)
        .to_string_lossy()
        .into_owned())
}

fn metadata(path: &Path) -> Result<Value> {
    let stat = fs::metadata(path)?;
    let created: chrono::DateTime<chrono::Utc> = stat.modified()?.into();
    let filename = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("Invalid snapshot name")?;
    Ok(
        json!({"filename":filename,"bytes":stat.len(),"created_at":created.to_rfc3339(),"label":filename.trim_end_matches(".zip")}),
    )
}
#[tauri::command(async)]
pub fn backup_create(
    state: tauri::State<crate::Session>,
    label: Option<String>,
) -> crate::Reply<Value> {
    state.read(|w| {
        let label = label.unwrap_or_default();
        let safe: String = label
            .chars()
            .filter(|c| c.is_alphanumeric() || [' ', '-', '_'].contains(c))
            .take(80)
            .collect();
        let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S-%3f");
        let filename = if safe.trim().is_empty() {
            format!("{stamp}.zip")
        } else {
            format!("{stamp}-{}.zip", safe.trim())
        };
        let path = backup_path(w, &filename)?;
        snapshot(w, &path)?;
        metadata(&path)
    })
}
#[tauri::command(async)]
pub fn backup_list(state: tauri::State<crate::Session>) -> crate::Reply<Vec<Value>> {
    state.read(|w| {
        let directory = w
            .output_file("backups/.list")?
            .parent()
            .ok_or("Missing backups directory")?
            .to_owned();
        let mut result = Vec::new();
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            if entry.file_type()?.is_file() && entry.path().extension().is_some_and(|e| e == "zip")
            {
                result.push(metadata(&entry.path())?);
            }
        }
        result.sort_by(|a, b| b["created_at"].as_str().cmp(&a["created_at"].as_str()));
        Ok(result)
    })
}
#[tauri::command(async)]
pub fn backup_delete(state: tauri::State<crate::Session>, filename: String) -> crate::Reply<()> {
    state.read(|w| {
        fs::remove_file(backup_path(w, &filename)?)?;
        Ok(())
    })
}
#[tauri::command(async)]
pub fn backup_restore(
    state: tauri::State<crate::Session>,
    filename: String,
    destination: String,
) -> crate::Reply<String> {
    state.read(|w| restore(&backup_path(w, &filename)?, Path::new(&destination)))
}
