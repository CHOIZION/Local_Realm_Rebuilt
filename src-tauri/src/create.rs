use super::world::{Result, World};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::{collections::HashMap, fs, path::Path};
use uuid::Uuid;

const DIRECTORIES: &[&str] = &[
    "backups",
    "canvases",
    "content/characters",
    "content/custom",
    "content/events",
    "content/factions",
    "content/locations",
    "dialogues",
    "exports",
    "manuscripts",
    "maps",
    "media/audio",
    "media/brushes",
    "media/documents",
    "media/images",
    "media/models",
    "media/video",
    "plugins",
    "tables",
    "themes",
    "timelines",
];

fn new_id() -> String {
    Uuid::new_v4().to_string()
}

pub fn create(path: &Path, name: &str) -> Result<World> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Enter a realm name".into());
    }
    let parent = path.parent().ok_or("Choose a folder for the new realm")?;
    if path.exists() && (!path.is_dir() || fs::read_dir(path)?.next().is_some()) {
        return Err("Choose an empty realm folder".into());
    }
    let temp = tempfile::Builder::new()
        .prefix(".localrealm-create-")
        .tempdir_in(parent)?;
    for dir in DIRECTORIES {
        fs::create_dir_all(temp.path().join(dir))?;
    }
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let mut db = Connection::open(temp.path().join("world.db"))?;
    db.pragma_update(None, "foreign_keys", "ON")?;
    let tx = db.transaction()?;
    for sql in crate::migrations::ALL {
        tx.execute_batch(sql)?;
    }
    let categories: Vec<Value> =
        serde_json::from_str(include_str!("../resources/categories.json"))?;
    let ids: HashMap<_, _> = categories
        .iter()
        .map(|c| (c["name"].as_str().unwrap(), new_id()))
        .collect();
    for cat in &categories {
        let category = cat["name"].as_str().ok_or("Invalid category template")?;
        let parent_id = cat["parent"].as_str().and_then(|p| ids.get(p));
        tx.execute("INSERT INTO entity_type (id,parent_id,name,plural,icon,color,description,sort_order,created_at,updated_at)
            VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?9)", params![ids[category],parent_id,category,
            cat["plural"].as_str(),cat["icon"].as_str(),cat["color"].as_str(),cat["description"].as_str(),cat["sort_order"].as_i64(),now])?;
    }
    for cat in &categories {
        let category = cat["name"].as_str().unwrap();
        for prop in cat["properties"]
            .as_array()
            .ok_or("Missing property template")?
        {
            let mut config = prop["config"].clone();
            if let Some(target) = config
                .as_object_mut()
                .and_then(|o| o.remove("target_category"))
            {
                config["target_type_id"] =
                    json!(ids[target.as_str().ok_or("Invalid target category")?]);
            }
            tx.execute("INSERT INTO property_definition (id,type_id,key,label,data_type,config,hint,required,sort_order,created_at,updated_at)
                VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10)",params![new_id(),ids[category],prop["key"].as_str(),prop["label"].as_str(),
                prop["data_type"].as_str(),config.to_string(),prop["hint"].as_str(),prop["required"].as_i64(),prop["sort_order"].as_i64(),now])?;
        }
    }
    tx.execute("DELETE FROM pending_work WHERE task='text'", [])?;
    tx.pragma_update(None, "user_version", 20)?;
    tx.commit()?;
    drop(db);
    fs::write(
        temp.path().join("world.json"),
        serde_json::to_vec_pretty(&json!({"id":new_id(),"name":name,"description":"",
        "schema_version":20,"app":"Local Realm","app_version":"1.9.0","created_at":now,"updated_at":now}))?,
    )?;
    fs::write(
        temp.path().join("README.md"),
        include_str!("../resources/world-readme.md").replacen("{{name}}", name, 1),
    )?;
    // remove_dir only removes an empty directory; concurrent additions remain protected.
    let existing_empty = path.exists();
    if existing_empty {
        fs::remove_dir(path)?;
    }
    if let Err(error) = fs::rename(temp.path(), path) {
        if existing_empty {
            let _ = fs::create_dir(path);
        }
        return Err(error.into());
    }
    World::open(path)
}
