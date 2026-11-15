use crate::{
    database::{new_id, now, one},
    world::{rows, Result, World},
};
use rusqlite::params;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tauri::Manager;

fn kind(mime: &str, extension: &str) -> &'static str {
    if extension == "abr" {
        "brush"
    } else if mime.starts_with("image/") {
        "image"
    } else if mime.starts_with("audio/") {
        "audio"
    } else if mime.starts_with("video/") {
        "video"
    } else if ["obj", "gltf", "glb", "fbx", "stl"].contains(&extension) {
        "model"
    } else if mime.starts_with("text/") || ["pdf", "docx", "epub", "odt"].contains(&extension) {
        "document"
    } else {
        "other"
    }
}
pub fn import(world: &World, path: &Path, caption: &str) -> Result<Value> {
    let bytes = fs::read(path)?;
    let sha = format!("{:x}", Sha256::digest(&bytes));
    if let Some(existing) = rows(
        &world.db,
        "SELECT * FROM media WHERE sha256=?1 ORDER BY created_at LIMIT 1",
        [&sha],
    )?
    .pop()
    {
        return Ok(existing);
    }
    let filename = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("Invalid file name")?;
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mime = if extension == "abr" {
        "application/x-photoshop-abr".to_owned()
    } else {
        mime_guess::from_path(path)
            .first_or_octet_stream()
            .to_string()
    };
    let kind = kind(&mime, &extension);
    let folder = match kind {
        "image" => "images",
        "audio" => "audio",
        "video" => "video",
        "model" => "models",
        "brush" => "brushes",
        _ => "documents",
    };
    let id = new_id();
    let relative = if extension.is_empty() {
        format!("media/{folder}/{id}")
    } else {
        format!("media/{folder}/{id}.{extension}")
    };
    world.transaction(|db| {
        db.execute(
            "INSERT INTO media(id,filename,path,mime,kind,bytes,sha256,caption,created_at)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                id,
                filename,
                relative,
                mime,
                kind,
                bytes.len(),
                sha,
                caption,
                now()
            ],
        )?;
        World::queue_file(db, &relative, &bytes)?;
        one(db, "SELECT * FROM media WHERE id=?1", &id)
    })
}
/// Scan each document once, regardless of the number of media files in the realm.
pub fn usage_index(world: &World) -> Result<std::collections::HashMap<String, Vec<Value>>> {
    use std::collections::{HashMap, HashSet};
    let media = rows(&world.db, "SELECT id,path FROM media", [])?;
    let mut index: HashMap<String, Vec<Value>> = HashMap::new();
    let mut patterns: HashMap<String, Vec<String>> = HashMap::new();
    for item in &media {
        let id = item["id"].as_str().ok_or("Missing media id")?;
        index.insert(id.into(), Vec::new());
        for field in ["path", "id"] {
            if let Some(pattern) = item[field].as_str().filter(|p| !p.is_empty()) {
                patterns.entry(pattern.into()).or_default().push(id.into());
            }
        }
    }
    if patterns.is_empty() {
        return Ok(index);
    }
    let patterns: Vec<_> = patterns.into_iter().collect();
    let matcher = aho_corasick::AhoCorasick::new(patterns.iter().map(|p| &p.0))?;
    let mut add = |text: &str, context: Value| {
        let mut used = HashSet::new();
        for found in matcher.find_overlapping_iter(text) {
            for id in &patterns[found.pattern().as_usize()].1 {
                if used.insert(id) {
                    index.get_mut(id).unwrap().push(context.clone());
                }
            }
        }
    };
    for entity in rows(&world.db, "SELECT e.id,e.name,e.image_media_id,m.media_id FROM entity e LEFT JOIN entity_media m ON m.entity_id=e.id", [])? {
        let text = format!("{} {}", entity["image_media_id"].as_str().unwrap_or(""), entity["media_id"].as_str().unwrap_or(""));
        add(&text, json!({"kind":"entity","id":entity["id"],"entity_id":entity["id"],"title":entity["name"],"name":entity["name"],"context":"attachment"}));
    }
    for property in rows(
        &world.db,
        "SELECT e.id,e.name,p.value FROM property_value p JOIN entity e ON e.id=p.entity_id",
        [],
    )? {
        add(
            &property["value"].to_string(),
            json!({"kind":"entity","id":property["id"],"entity_id":property["id"],"title":property["name"],"name":property["name"],"context":"property"}),
        );
    }
    for doc in rows(&world.db, "SELECT d.id,d.title,d.entity_id,c.manuscript_id FROM document d LEFT JOIN chapter c ON c.document_id=d.id", [])? {
        let contents = world.document(doc["id"].as_str().ok_or("Missing document id")?)?;
        add(
            contents["body"].as_str().unwrap_or(""),
            json!({"kind":"document","id":doc["id"],"document_id":doc["id"],"entity_id":doc["entity_id"],"manuscript_id":doc["manuscript_id"],"title":doc["title"],"name":doc["title"],"context":"body"}),
        );
    }
    for (table, fields) in [("map", "media_id,config"), ("manuscript", "ebook")] {
        for row in rows(
            &world.db,
            &format!("SELECT id,name,{fields} FROM {table}"),
            [],
        )? {
            add(
                &row.to_string(),
                json!({"kind":table,"id":row["id"],"title":row["name"],"name":row["name"],"context":"settings"}),
            );
        }
    }
    for contexts in index.values_mut() {
        for context in contexts.iter_mut() {
            context["entity_name"] = context["title"].clone();
            context["role"] = context["context"].clone();
        }
        let mut seen = HashSet::new();
        contexts.retain(|context| seen.insert(context.to_string()));
    }
    Ok(index)
}

pub fn usage(world: &World, id: &str) -> Result<Vec<Value>> {
    one(&world.db, "SELECT id FROM media WHERE id=?1", id)?;
    Ok(usage_index(world)?.remove(id).unwrap_or_default())
}

#[tauri::command(async)]
pub fn media_import(
    state: tauri::State<crate::Session>,
    path: String,
    caption: Option<String>,
) -> crate::Reply<Value> {
    state.read(|w| import(w, Path::new(&path), caption.as_deref().unwrap_or("")))
}
#[tauri::command(async)]
pub fn media_list(
    state: tauri::State<crate::Session>,
    kind: Option<String>,
) -> crate::Reply<Vec<Value>> {
    state.read(|w| {
        let mut files = rows(
            &w.db,
            "SELECT * FROM media WHERE ?1 IS NULL OR kind=?1 ORDER BY created_at DESC,id",
            [kind],
        )?;
        let usage = usage_index(w)?;
        for file in &mut files {
            file["usage_count"] = json!(usage
                .get(file["id"].as_str().ok_or("Missing media id")?)
                .map(Vec::len)
                .unwrap_or(0));
        }
        Ok(files)
    })
}
#[tauri::command(async)]
pub fn media_usage(state: tauri::State<crate::Session>, id: String) -> crate::Reply<Vec<Value>> {
    state.read(|w| usage(w, &id))
}
#[tauri::command(async)]
pub fn media_unused(state: tauri::State<crate::Session>) -> crate::Reply<Vec<Value>> {
    state.read(|w| {
        let mut unused = Vec::new();
        let used = usage_index(w)?;
        for media in rows(&w.db, "SELECT * FROM media ORDER BY created_at DESC", [])? {
            if used
                .get(media["id"].as_str().ok_or("Missing media id")?)
                .is_none_or(Vec::is_empty)
            {
                unused.push(media);
            }
        }
        Ok(unused)
    })
}
#[tauri::command(async)]
pub fn media_attach(
    state: tauri::State<crate::Session>,
    entity_id: String,
    media_id: String,
    role: Option<String>,
) -> crate::Reply<()> {
    state.read(|w| {
        w.transaction(|db| {
            db.execute(
                "INSERT OR IGNORE INTO entity_media(entity_id,media_id,role) VALUES(?1,?2,?3)",
                params![entity_id, media_id, role.as_deref().unwrap_or("attachment")],
            )?;
            Ok(())
        })
    })
}
#[tauri::command(async)]
pub fn media_detach(
    state: tauri::State<crate::Session>,
    entity_id: String,
    media_id: String,
) -> crate::Reply<()> {
    state.read(|w| {
        w.transaction(|db| {
            db.execute(
                "DELETE FROM entity_media WHERE entity_id=?1 AND media_id=?2",
                params![entity_id, media_id],
            )?;
            Ok(())
        })
    })
}
#[tauri::command(async)]
pub fn media_caption(
    state: tauri::State<crate::Session>,
    id: String,
    caption: String,
) -> crate::Reply<Value> {
    state.read(|w| {
        w.transaction(|db| {
            db.execute(
                "UPDATE media SET caption=?1 WHERE id=?2",
                params![caption, id],
            )?;
            one(db, "SELECT * FROM media WHERE id=?1", &id)
        })
    })
}
#[tauri::command(async)]
pub fn media_delete(
    state: tauri::State<crate::Session>,
    id: String,
    force: Option<bool>,
) -> crate::Reply<()> {
    state.read(|w| {
        if !force.unwrap_or(false) && !usage(w, &id)?.is_empty() {
            return Err("This file is used in the realm".into());
        }
        w.transaction(|db| {
            let media = one(db, "SELECT * FROM media WHERE id=?1", &id)?;
            db.execute("DELETE FROM media WHERE id=?1", [id])?;
            World::queue_remove(db, media["path"].as_str().ok_or("Missing media path")?)
        })
    })
}
#[tauri::command(async)]
pub fn media_path(
    app: tauri::AppHandle,
    state: tauri::State<crate::Session>,
    relative: String,
) -> crate::Reply<String> {
    state.read(|w| {
        let path = w.local_file(&relative)?;
        app.asset_protocol_scope().allow_file(&path)?;
        Ok(dunce::simplified(&path).to_string_lossy().into_owned())
    })
}
#[tauri::command(async)]
pub fn media_read_brush(
    state: tauri::State<crate::Session>,
    id: String,
) -> crate::Reply<tauri::ipc::Response> {
    state.read(|w| {
        let media = one(&w.db, "SELECT * FROM media WHERE id=?1", &id)?;
        Ok(tauri::ipc::Response::new(fs::read(w.local_file(
            media["path"].as_str().ok_or("Missing brush path")?,
        )?)?))
    })
}
fn brushes(app: &tauri::AppHandle) -> Result<PathBuf> {
    let directory = app.path().resource_dir()?.join("brushes");
    Ok(directory)
}
#[tauri::command(async)]
pub fn brush_library_list(app: tauri::AppHandle) -> crate::Reply<Vec<Value>> {
    (||->Result<_>{
        let path=brushes(&app)?;
        if !path.exists(){return Ok(vec![]);}
        let mut result=Vec::new();
        for entry in fs::read_dir(path)?{
            let entry=entry?;if !entry.file_type()?.is_file(){continue;}
            let path=entry.path();
            let extension=path.extension().and_then(|s|s.to_str()).unwrap_or("").to_lowercase();
            if !["abr","png","jpg","jpeg"].contains(&extension.as_str()){continue;}
            let file=entry.file_name().to_string_lossy().into_owned();
            let mime=if extension=="abr" {"application/x-photoshop-abr".to_owned()}else{mime_guess::from_path(&path).first_or_octet_stream().to_string()};
            result.push(json!({"file":file,"name":path.file_stem().and_then(|s|s.to_str()).unwrap_or("Brush"),"bytes":entry.metadata()?.len(),"mime":mime}));
        }
        result.sort_by(|a,b|a["name"].as_str().cmp(&b["name"].as_str()));
        Ok(result)
    })().map_err(|e|e.to_string())
}
#[tauri::command(async)]
pub fn brush_library_read(
    app: tauri::AppHandle,
    file: String,
) -> crate::Reply<tauri::ipc::Response> {
    (|| -> Result<_> {
        if Path::new(&file).components().count() != 1 || file.contains(['/', '\\', ':']) {
            return Err("Invalid brush file name".into());
        }
        let directory = brushes(&app)?.canonicalize()?;
        let path = directory.join(file).canonicalize()?;
        if !path.starts_with(directory) {
            return Err("Brush outside the library".into());
        }
        Ok(tauri::ipc::Response::new(fs::read(path)?))
    })()
    .map_err(|e| e.to_string())
}
