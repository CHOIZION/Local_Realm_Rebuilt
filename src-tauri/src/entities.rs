use crate::{
    database::{new_id, now, one, required_text, update_fields},
    world::{rows, Result, World},
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};

const ENTITY_SELECT:&str="SELECT e.*,t.name AS type_name,t.plural AS type_plural,t.icon AS type_icon,
    t.color AS type_color,t.layout AS type_layout,m.path AS image_path
    FROM entity e JOIN entity_type t ON e.type_id=t.id LEFT JOIN media m ON m.id=e.image_media_id WHERE e.id=?1";

pub fn get(world: &World, id: &str) -> Result<Value> {
    let db = &world.db;
    let mut entity = one(db, ENTITY_SELECT, id)?;
    entity["aliases"] = json!(rows(
        db,
        "SELECT name FROM alias WHERE entity_id=?1 ORDER BY name COLLATE NOCASE",
        [id]
    )?
    .iter()
    .map(|r| r["name"].clone())
    .collect::<Vec<_>>());
    entity["tags"]=json!(rows(db,"SELECT t.name FROM tag t JOIN entity_tag et ON et.tag_id=t.id WHERE et.entity_id=?1 ORDER BY t.name COLLATE NOCASE",[id])?
        .iter().map(|r|r["name"].clone()).collect::<Vec<_>>());
    let mut properties =
        crate::schema::properties(db, entity["type_id"].as_str().ok_or("Missing category")?)?;
    let values = rows(
        db,
        "SELECT property_id,value FROM property_value WHERE entity_id=?1",
        [id],
    )?;
    for property in &mut properties {
        let value = values
            .iter()
            .find(|r| r["property_id"] == property["id"])
            .map(|r| r["value"].clone())
            .unwrap_or(Value::Null);
        let mut references = Vec::new();
        if ["entity_ref", "entity_ref_multi"]
            .contains(&property["data_type"].as_str().unwrap_or(""))
        {
            let ids = if let Some(array) = value.as_array() {
                array.clone()
            } else {
                vec![value.clone()]
            };
            for id in ids.iter().filter_map(Value::as_str) {
                let found=rows(db,"SELECT e.id,e.name,t.name AS type_name FROM entity e JOIN entity_type t ON t.id=e.type_id WHERE e.id=?1",[id])?.pop();
                references.push(found.unwrap_or_else(
                    || json!({"id":id,"name":"Deleted entry","type_name":"","missing":true}),
                ));
            }
        }
        property["value"] = value;
        property["refs"] = json!(references);
    }
    entity["properties"] = json!(properties);
    let document = rows(db, "SELECT * FROM document WHERE entity_id=?1", [id])?.pop();
    entity["body"] = match &document {
        Some(document) => {
            world.document(document["id"].as_str().ok_or("Missing document id")?)?["body"].clone()
        }
        None => json!(""),
    };
    entity["document"] = document.unwrap_or(Value::Null);
    entity["relationships"] = json!(crate::relationships::for_entity(db, id)?);
    entity["backlinks"] = json!(backlinks(db, id)?);
    entity["media"]=json!(rows(db,"SELECT m.*,em.role FROM media m JOIN entity_media em ON em.media_id=m.id WHERE em.entity_id=?1 ORDER BY em.sort_order,m.filename",[id])?);
    Ok(entity)
}

pub fn backlinks(db: &Connection, id: &str) -> Result<Vec<Value>> {
    rows(db,"SELECT l.*,coalesce(e.name,d.title) AS title,
        CASE WHEN l.source_kind='entity' THEN e.id ELSE d.entity_id END AS entity_id
        FROM link l LEFT JOIN entity e ON l.source_kind='entity' AND e.id=l.source_id
        LEFT JOIN document d ON l.source_kind='document' AND d.id=l.source_id
        WHERE l.target_id=?1 AND (e.id IS NOT NULL OR d.id IS NOT NULL) ORDER BY title COLLATE NOCASE",[id])
}

pub fn create_in(db: &Connection, type_id: &str, name: &str, summary: &str) -> Result<String> {
    if name.trim().is_empty() {
        return Err("Enter an entry name".into());
    }
    let id = new_id();
    let time = now();
    db.execute("INSERT INTO entity(id,type_id,name,summary,created_at,updated_at,sort_order)
        VALUES(?1,?2,?3,?4,?5,?5,(SELECT coalesce(max(sort_order),0)+1 FROM entity WHERE type_id=?2))",
        params![id,type_id,name.trim(),summary,time])?;
    let document = crate::documents::create_in(db, "article", name.trim(), Some(&id))?;
    crate::search::index_document(db, &document, "")?;
    Ok(id)
}

fn set_properties(db: &Connection, id: &str, type_id: &str, values: &Value) -> Result<()> {
    let definitions = crate::schema::properties(db, type_id)?;
    for (key, value) in values
        .as_object()
        .ok_or("Expected property values object")?
    {
        let definition = definitions
            .iter()
            .find(|d| d["key"] == *key || d["id"] == *key)
            .ok_or_else(|| format!("Unknown property {key}"))?;
        let property_id = definition["id"].as_str().ok_or("Missing property id")?;
        let kind = definition["data_type"].as_str().unwrap_or("");
        if !value.is_null() {
            let valid = match kind {
                "integer" => value.is_i64() || value.is_u64(),
                "decimal" | "rating" => value.is_number(),
                "boolean" => value.is_boolean(),
                "multi_select" | "entity_ref_multi" => value
                    .as_array()
                    .is_some_and(|a| a.iter().all(Value::is_string)),
                "coordinates" | "world_date" => value.is_object(),
                "section" => false,
                _ => value.is_string(),
            };
            if !valid {
                return Err(format!("Invalid value for {}", definition["label"]).into());
            }
        }
        db.execute(
            "DELETE FROM link WHERE source_kind='entity' AND source_id=?1 AND context=?2",
            params![id, definition["label"].as_str()],
        )?;
        if value.is_null() {
            db.execute(
                "DELETE FROM property_value WHERE entity_id=?1 AND property_id=?2",
                params![id, property_id],
            )?;
        } else {
            db.execute(
                "INSERT INTO property_value(entity_id,property_id,value) VALUES(?1,?2,?3)
                ON CONFLICT(entity_id,property_id) DO UPDATE SET value=excluded.value",
                params![id, property_id, value.to_string()],
            )?;
        }
        if ["entity_ref", "entity_ref_multi"].contains(&kind) {
            let ids = if let Some(array) = value.as_array() {
                array.clone()
            } else {
                vec![value.clone()]
            };
            for target in ids.iter().filter_map(Value::as_str) {
                db.execute(
                    "INSERT OR IGNORE INTO link(source_kind,source_id,target_id,context)
                    SELECT 'entity',?1,id,?2 FROM entity WHERE id=?3",
                    params![id, definition["label"].as_str(), target],
                )?;
            }
        }
    }
    Ok(())
}

fn set_names(db: &Connection, id: &str, values: &Value, tags: bool) -> Result<()> {
    let names = values.as_array().ok_or("Expected names array")?;
    let table = if tags { "entity_tag" } else { "alias" };
    db.execute(&format!("DELETE FROM {table} WHERE entity_id=?1"), [id])?;
    for name in names {
        let text = name.as_str().ok_or("Expected a name string")?.trim();
        if text.is_empty() {
            continue;
        }
        if tags {
            db.execute(
                "INSERT OR IGNORE INTO tag(id,name,color) VALUES(?1,?2,'')",
                params![new_id(), text],
            )?;
            db.execute("INSERT OR IGNORE INTO entity_tag(entity_id,tag_id) SELECT ?1,id FROM tag WHERE name=?2 COLLATE NOCASE",params![id,text])?;
        } else {
            db.execute(
                "INSERT OR IGNORE INTO alias(id,entity_id,name) VALUES(?1,?2,?3)",
                params![new_id(), id, text],
            )?;
        }
    }
    Ok(())
}

pub fn update_in(world: &World, db: &Connection, id: &str, patch: &Value) -> Result<()> {
    let old = one(db, "SELECT name FROM entity WHERE id=?1", id)?;
    if patch.get("name").is_some() {
        required_text(patch, "name")?;
    }
    update_fields(
        db,
        "entity",
        id,
        patch,
        &[
            "name",
            "summary",
            "type_id",
            "image_media_id",
            "image_frame",
        ],
        true,
    )?;
    let entity = one(db, "SELECT * FROM entity WHERE id=?1", id)?;
    if let Some(values) = patch.get("properties") {
        set_properties(
            db,
            id,
            entity["type_id"].as_str().ok_or("Missing category")?,
            values,
        )?;
    }
    if let Some(tags) = patch.get("tags") {
        set_names(db, id, tags, true)?;
    }
    if let Some(aliases) = patch.get("aliases") {
        set_names(db, id, aliases, false)?;
    }
    if let Some(name) = patch["name"].as_str() {
        if let Some(document) = rows(db, "SELECT id FROM document WHERE entity_id=?1", [id])?.pop()
        {
            crate::documents::rename_in(
                world,
                db,
                document["id"].as_str().ok_or("Missing document id")?,
                name,
            )?;
        }
        if old["name"] != name {
            let pattern = regex::Regex::new(&format!(
                r"\[{}\]\(entity://{}\)",
                regex::escape(old["name"].as_str().unwrap_or("")),
                regex::escape(id)
            ))?;
            let replacement = format!(
                "[{}](entity://{id})",
                name.replace('[', "\\[").replace(']', "\\]")
            );
            let linked=rows(db,"SELECT DISTINCT d.id FROM link l JOIN document d ON (l.source_kind='document' AND d.id=l.source_id) OR (l.source_kind='entity' AND d.entity_id=l.source_id) WHERE l.target_id=?1 AND l.context='body'",[id])?;
            for document in linked {
                let document_id = document["id"].as_str().ok_or("Missing linked document")?;
                let doc = world.document(document_id)?;
                let body = doc["body"].as_str().unwrap_or("");
                let renamed = pattern.replace_all(body, regex::NoExpand(&replacement));
                if renamed != body {
                    crate::documents::save_in(world, db, document_id, &renamed)?;
                }
            }
        }
    }
    crate::search::refresh_entity(world, db, id)?;
    Ok(())
}

pub fn update(world: &World, id: &str, patch: &Value) -> Result<Value> {
    world.transaction(|db| update_in(world, db, id, patch))?;
    get(world, id)
}

#[tauri::command(async)]
pub fn entity_get(state: tauri::State<crate::Session>, id: String) -> crate::Reply<Value> {
    state.read(|w| get(w, &id))
}
#[tauri::command(async)]
pub fn entity_create(
    state: tauri::State<crate::Session>,
    type_id: String,
    name: String,
    summary: Option<String>,
) -> crate::Reply<Value> {
    state.read(|w| {
        let id =
            w.transaction(|db| create_in(db, &type_id, &name, summary.as_deref().unwrap_or("")))?;
        get(w, &id)
    })
}
#[tauri::command(async)]
pub fn entity_update(
    state: tauri::State<crate::Session>,
    id: String,
    patch: Value,
) -> crate::Reply<Value> {
    state.read(|w| update(w, &id, &patch))
}
#[tauri::command(async)]
pub fn entity_set_properties(
    state: tauri::State<crate::Session>,
    id: String,
    values: Value,
) -> crate::Reply<Value> {
    state.read(|w| update(w, &id, &json!({"properties":values})))
}
#[tauri::command(async)]
pub fn entity_backlinks(
    state: tauri::State<crate::Session>,
    id: String,
) -> crate::Reply<Vec<Value>> {
    state.read(|w| backlinks(&w.db, &id))
}
#[tauri::command(async)]
pub fn entity_lookup(
    state: tauri::State<crate::Session>,
    query: String,
    limit: Option<usize>,
) -> crate::Reply<Vec<Value>> {
    state.read(|w|{
        let pattern=format!("%{}%",query.replace('!',"!!").replace('%',"!%").replace('_',"!_"));
        rows(&w.db,"SELECT e.*,t.name AS type_name,t.icon AS type_icon,t.color AS type_color,m.path AS image_path,
            json('[]') AS tags FROM entity e JOIN entity_type t ON t.id=e.type_id LEFT JOIN media m ON m.id=e.image_media_id
            WHERE e.name LIKE ?1 ESCAPE '!' OR EXISTS(SELECT 1 FROM alias WHERE entity_id=e.id AND name LIKE ?1 ESCAPE '!')
            ORDER BY e.name COLLATE NOCASE LIMIT ?2",params![pattern,limit.unwrap_or(12).min(10000)])
    })
}
#[tauri::command(async)]
pub fn entity_duplicate(state: tauri::State<crate::Session>, id: String) -> crate::Reply<Value> {
    state.read(|w|{
        let original=get(w,&id)?;
        let new_id=w.transaction(|db|{
            let new_id=create_in(db,original["type_id"].as_str().ok_or("Missing category")?,
                &format!("{} (copy)",original["name"].as_str().unwrap_or("")),original["summary"].as_str().unwrap_or(""))?;
            db.execute("INSERT INTO property_value(entity_id,property_id,value) SELECT ?1,property_id,value FROM property_value WHERE entity_id=?2",params![new_id,id])?;
            db.execute("INSERT INTO link(source_kind,source_id,target_id,context) SELECT 'entity',?1,target_id,context FROM link WHERE source_kind='entity' AND source_id=?2 AND context!='body'",params![new_id,id])?;
            db.execute("INSERT INTO entity_media(entity_id,media_id,role,sort_order) SELECT ?1,media_id,role,sort_order FROM entity_media WHERE entity_id=?2",params![new_id,id])?;
            set_names(db,&new_id,&original["tags"],true)?;
            set_names(db,&new_id,&original["aliases"],false)?;
            update_fields(db,"entity",&new_id,&original,&["image_media_id","image_frame"],true)?;
            let document=one(db,"SELECT * FROM document WHERE entity_id=?1",&new_id)?;
            let body=original["body"].as_str().unwrap_or("");
            // The newly created file is still in the transaction's pending-write table.
            let mut document=document;
            document["word_count"]=json!(crate::documents::word_count(body));
            db.execute("UPDATE document SET word_count=?1 WHERE id=?2",params![document["word_count"].as_u64(),document["id"].as_str()])?;
            crate::documents::queue_body(db,&document,body)?;
            crate::search::index_document(db,&document,body)?;
            Ok(new_id)
        })?;
        get(w,&new_id)
    })
}
