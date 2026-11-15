use crate::{
    database::now,
    world::{Result, World},
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

pub fn get(db: &Connection, key: &str, default: Value) -> Result<Value> {
    let value: Option<String> = db
        .query_row("SELECT value FROM world_meta WHERE key=?1", [key], |r| {
            r.get(0)
        })
        .optional()?;
    value
        .map(|v| serde_json::from_str(&v))
        .transpose()
        .map(|v| v.unwrap_or(default))
        .map_err(Into::into)
}
pub fn set(db: &Connection, key: &str, value: &Value) -> Result<()> {
    db.execute("INSERT INTO world_meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![key,value.to_string()])?;
    Ok(())
}
fn edit_manifest(world: &World, key: &str, value: &str) -> Result<Value> {
    world.transaction(|db| {
        let mut manifest: Value =
            serde_json::from_slice(&std::fs::read(world.root.join("world.json"))?)?;
        manifest[key] = json!(value);
        manifest["updated_at"] = json!(now());
        World::queue_file(db, "world.json", &serde_json::to_vec_pretty(&manifest)?)?;
        Ok(())
    })?;
    world.info()
}
#[tauri::command(async)]
pub fn world_rename(state: tauri::State<crate::Session>, name: String) -> crate::Reply<Value> {
    state.read(|w| {
        if name.trim().is_empty() {
            return Err("Enter a realm name".into());
        }
        edit_manifest(w, "name", name.trim())
    })
}
#[tauri::command(async)]
pub fn world_describe(
    state: tauri::State<crate::Session>,
    description: String,
) -> crate::Reply<Value> {
    state.read(|w| edit_manifest(w, "description", &description))
}
#[tauri::command(async)]
pub fn dictionary_get(state: tauri::State<crate::Session>) -> crate::Reply<Vec<String>> {
    state.read(|w| {
        let path = w.root.join("dictionary.txt");
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(text
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect())
    })
}
#[tauri::command(async)]
pub fn dictionary_set(
    state: tauri::State<crate::Session>,
    words: Vec<String>,
) -> crate::Reply<Vec<String>> {
    state.read(|w| {
        w.transaction(|db| {
            let mut sorted = std::collections::BTreeSet::new();
            for word in words {
                let word = word.trim();
                if word.is_empty() {
                    continue;
                }
                if word.split_whitespace().count() != 1 || word.chars().count() > 64 {
                    return Err(
                        "Dictionary words must be single words of at most 64 characters".into(),
                    );
                }
                sorted.insert(word.to_owned());
            }
            let words = sorted.into_iter().collect::<Vec<_>>();
            World::queue_file(db, "dictionary.txt", words.join("\n").as_bytes())?;
            Ok(words)
        })
    })
}
#[tauri::command(async)]
pub fn world_background_get(state: tauri::State<crate::Session>) -> crate::Reply<Value> {
    state.read(|w| get(&w.db, "background", Value::Null))
}
#[tauri::command(async)]
pub fn world_background_set(
    state: tauri::State<crate::Session>,
    value: Value,
) -> crate::Reply<Value> {
    state.read(|w| {
        w.transaction(|db| {
            set(db, "background", &value)?;
            Ok(value)
        })
    })
}
#[tauri::command(async)]
pub fn graph_arrangement(state: tauri::State<crate::Session>) -> crate::Reply<Value> {
    state.read(|w| get(&w.db, "graph_arrangement", json!({})))
}
#[tauri::command(async)]
pub fn graph_arrangement_save(
    state: tauri::State<crate::Session>,
    arrangement: Value,
) -> crate::Reply<()> {
    state.read(|w| w.transaction(|db| set(db, "graph_arrangement", &arrangement)))
}
#[tauri::command(async)]
pub fn reveal_in_file_manager(path: String) -> crate::Reply<()> {
    let path = std::path::Path::new(&path)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    std::process::Command::new("explorer.exe")
        .arg(dunce::simplified(&path))
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}
