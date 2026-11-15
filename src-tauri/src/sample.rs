//! The bundled reference realm contains only the original application's generated sample.
use crate::world::{write_atomic, Result, World};
use serde_json::{json, Value};
use std::{fs, path::Path};

pub fn create(path: &Path) -> Result<()> {
    let temporary = tempfile::tempdir()?;
    let archive = temporary.path().join("sample.zip");
    fs::write(&archive, include_bytes!("../resources/sample.zip"))?;
    crate::archives::restore(&archive, path)?;
    let world = World::open(path)?;
    let mut manifest: Value = serde_json::from_slice(&fs::read(world.root.join("world.json"))?)?;
    manifest["id"] = json!(crate::database::new_id());
    manifest["created_at"] = json!(crate::database::now());
    manifest["updated_at"] = manifest["created_at"].clone();
    write_atomic(
        &world.root.join("world.json"),
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(())
}
#[tauri::command(async)]
pub fn world_create_sample(
    app: tauri::AppHandle,
    state: tauri::State<crate::Session>,
    path: String,
) -> crate::Reply<Value> {
    create(Path::new(&path)).map_err(|e| e.to_string())?;
    crate::world_open(app, state, path)
}
