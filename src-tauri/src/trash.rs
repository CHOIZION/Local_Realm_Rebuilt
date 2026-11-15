use crate::{
    database::{new_id, now},
    world::{rows, Result, World},
};
use rusqlite::{params, params_from_iter, types::Value as SqlValue, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashSet;

#[derive(Serialize, Deserialize)]
struct SavedRow {
    table: String,
    key: Map<String, Value>,
    values: Map<String, Value>,
    removed: bool,
}
#[derive(Serialize, Deserialize)]
struct Payload {
    version: u32,
    rows: Vec<SavedRow>,
    files: Vec<(String, Vec<u8>)>,
}

fn quoted(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}
fn sql_value(value: &Value) -> SqlValue {
    match value {
        Value::Null => SqlValue::Null,
        Value::Number(n) => n
            .as_i64()
            .map(SqlValue::Integer)
            .unwrap_or_else(|| SqlValue::Real(n.as_f64().unwrap_or_default())),
        Value::String(s) => SqlValue::Text(s.clone()),
        _ => SqlValue::Text(value.to_string()),
    }
}
fn condition(key: &Map<String, Value>) -> (String, Vec<SqlValue>) {
    (
        key.keys()
            .map(|k| format!("{} IS ?", quoted(k)))
            .collect::<Vec<_>>()
            .join(" AND "),
        key.values().map(sql_value).collect(),
    )
}
fn raw_rows(db: &Connection, sql: &str, args: Vec<SqlValue>) -> Result<Vec<Map<String, Value>>> {
    let mut statement = db.prepare(sql)?;
    let names = statement
        .column_names()
        .iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>();
    let mut cursor = statement.query(params_from_iter(args))?;
    let mut result = Vec::new();
    while let Some(row) = cursor.next()? {
        let mut values = Map::new();
        for (i, name) in names.iter().enumerate() {
            use rusqlite::types::ValueRef;
            values.insert(
                name.clone(),
                match row.get_ref(i)? {
                    ValueRef::Null => Value::Null,
                    ValueRef::Integer(n) => json!(n),
                    ValueRef::Real(n) => json!(n),
                    ValueRef::Text(bytes) => json!(std::str::from_utf8(bytes)?),
                    ValueRef::Blob(_) => return Err("Unexpected blob in realm record".into()),
                },
            );
        }
        result.push(values);
    }
    Ok(result)
}
fn primary_key(
    db: &Connection,
    table: &str,
    row: &Map<String, Value>,
) -> Result<Map<String, Value>> {
    let mut key = Map::new();
    for column in rows(db, &format!("PRAGMA table_info({})", quoted(table)), [])? {
        if column["pk"].as_i64().unwrap_or(0) > 0 {
            let name = column["name"].as_str().ok_or("Missing column name")?;
            key.insert(
                name.into(),
                row.get(name).ok_or("Missing primary key")?.clone(),
            );
        }
    }
    if key.is_empty() {
        return Err(format!("Table {table} has no primary key").into());
    }
    Ok(key)
}
fn related_rows(
    db: &Connection,
    table: &str,
    row: Map<String, Value>,
    saved: &mut Vec<SavedRow>,
    seen: &mut HashSet<String>,
) -> Result<()> {
    let key = primary_key(db, table, &row)?;
    let identity = format!("{table}:{}", serde_json::to_string(&key)?);
    if !seen.insert(identity) {
        let existing = saved
            .iter_mut()
            .find(|item| item.table == table && item.key == key)
            .ok_or("Missing captured row")?;
        if existing.removed {
            return Ok(());
        }
        existing.removed = true;
    } else {
        saved.push(SavedRow {
            table: table.into(),
            key,
            values: row.clone(),
            removed: true,
        });
    }
    let tables=rows(db,"SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE 'search_index%' AND name NOT LIKE '__local_realm_%' AND name!='trash'",[])?;
    for child in tables {
        let child = child["name"].as_str().ok_or("Missing table name")?;
        for foreign in rows(
            db,
            &format!("PRAGMA foreign_key_list({})", quoted(child)),
            [],
        )? {
            if foreign["table"] != table {
                continue;
            }
            let column = foreign["from"].as_str().ok_or("Missing foreign key")?;
            let parent = foreign["to"].as_str().ok_or("Missing referenced key")?;
            let value = row.get(parent).ok_or("Missing referenced column")?;
            if value.is_null() {
                continue;
            }
            let children = raw_rows(
                db,
                &format!("SELECT * FROM {} WHERE {}=?", quoted(child), quoted(column)),
                vec![sql_value(value)],
            )?;
            match foreign["on_delete"].as_str() {
                Some("CASCADE") => {
                    for value in children {
                        related_rows(db, child, value, saved, seen)?;
                    }
                }
                Some("SET NULL") => {
                    for value in children {
                        let key = primary_key(db, child, &value)?;
                        let identity = format!("{child}:{}", serde_json::to_string(&key)?);
                        if seen.insert(identity) {
                            saved.push(SavedRow {
                                table: child.into(),
                                key,
                                values: value,
                                removed: false,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

pub fn delete(world: &World, table: &str, kind: &str, id: &str) -> Result<()> {
    world.transaction(|db|{
        let row=raw_rows(db,&format!("SELECT * FROM {} WHERE id=?",quoted(table)),vec![SqlValue::Text(id.into())])?
            .pop().ok_or("Record not found")?;
        let name=row.get("name").or_else(||row.get("title")).and_then(Value::as_str).unwrap_or("").to_owned();
        let mut saved=Vec::new();
        related_rows(db,table,row,&mut saved,&mut HashSet::new())?;
        let documents = saved.iter().filter(|row| row.table == "chapter" && row.removed)
            .filter_map(|row| row.values.get("document_id").and_then(Value::as_str).map(String::from))
            .collect::<Vec<_>>();
        let mut seen = saved.iter().map(|row| Ok(format!("{}:{}", row.table, serde_json::to_string(&row.key)?)))
            .collect::<Result<HashSet<_>>>()?;
        for document in &documents {
            let row = raw_rows(db, "SELECT * FROM document WHERE id=?", vec![SqlValue::Text(document.clone())])?
                .pop().ok_or("Missing chapter document")?;
            related_rows(db, "document", row, &mut saved, &mut seen)?;
        }
        let mut files=Vec::new();
        for row in &saved{
            if row.table=="document"&&row.removed{
                let path=row.values.get("path").and_then(Value::as_str).ok_or("Missing document path")?;
                let text=std::fs::read(world.local_file(path)?)?;
                files.push((path.into(),text));
                World::queue_remove(db,path)?;
            }
            if row.table=="map" && row.removed && row.values.get("kind").and_then(Value::as_str)==Some("painted") {
                let map_id=row.values.get("id").and_then(Value::as_str).ok_or("Missing map id")?;
                for filename in [format!("maps/{map_id}.png"),format!("maps/{map_id}.height.png")] {
                    if world.output_file(&filename)?.exists() {
                        files.push((filename.clone(),std::fs::read(world.local_file(&filename)?)?));
                        World::queue_remove(db,&filename)?;
                    }
                }
            }
        }
        db.execute(&format!("DELETE FROM {} WHERE id=?1",quoted(table)),[id])?;
        for document in &documents { db.execute("DELETE FROM document WHERE id=?1", [document])?; }
        // Keep only columns actually changed by a foreign-key SET NULL action.
        for row in saved.iter_mut().filter(|r|!r.removed){
            let (filter,args)=condition(&row.key);
            if let Some(current)=raw_rows(db,&format!("SELECT * FROM {} WHERE {filter}",quoted(&row.table)),args)?.pop(){
                row.values.retain(|column,value|current.get(column)!=Some(value));
            }else{row.removed=true;}
        }
        db.execute("DELETE FROM search_index WHERE (kind='entity' AND ref_id NOT IN (SELECT id FROM entity))
            OR (kind='document' AND ref_id NOT IN (SELECT id FROM document))",[])?;
        db.execute("DELETE FROM link WHERE (source_kind='entity' AND source_id NOT IN (SELECT id FROM entity))
            OR (source_kind='document' AND source_id NOT IN (SELECT id FROM document))",[])?;
        let payload=Payload{version:1,rows:saved,files};
        db.execute("INSERT INTO trash(id,kind,ref_id,name,deleted_at,payload) VALUES(?1,?2,?3,?4,?5,?6)",
            params![new_id(),kind,id,name,now(),serde_json::to_string(&payload)?])?;
        Ok(())
    })
}

pub fn restore(world: &World, id: &str) -> Result<Value> {
    let result = world.transaction(|db| {
        let entry = crate::database::one(db, "SELECT * FROM trash WHERE id=?1", id)?;
        let payload: Payload =
            serde_json::from_str(entry["payload"].as_str().ok_or("Missing trash payload")?)?;
        if payload.version != 1 {
            return Err("Unsupported trash payload version".into());
        }
        db.pragma_update(None, "defer_foreign_keys", true)?;
        for row in &payload.rows {
            if row.removed {
                let columns = row
                    .values
                    .keys()
                    .map(|k| quoted(k))
                    .collect::<Vec<_>>()
                    .join(",");
                let placeholders = vec!["?"; row.values.len()].join(",");
                db.execute(
                    &format!(
                        "INSERT INTO {}({columns}) VALUES({placeholders})",
                        quoted(&row.table)
                    ),
                    params_from_iter(row.values.values().map(sql_value)),
                )?;
            } else {
                let (filter, key_args) = condition(&row.key);
                for (column, value) in &row.values {
                    let mut args = vec![sql_value(value)];
                    args.extend(key_args.clone());
                    db.execute(
                        &format!(
                            "UPDATE {} SET {}=? WHERE {filter} AND {} IS NULL",
                            quoted(&row.table),
                            quoted(column),
                            quoted(column)
                        ),
                        params_from_iter(args),
                    )?;
                }
            }
        }
        for (path, text) in &payload.files {
            let target = world.output_file(path)?;
            if target.exists() {
                return Err(format!("A file already exists at {path}").into());
            }
            World::queue_file(db, path, text)?;
        }
        db.execute("DELETE FROM trash WHERE id=?1", [id])?;
        Ok(json!({"kind":entry["kind"],"ref_id":entry["ref_id"],"name":entry["name"]}))
    })?;
    crate::search::rebuild(world)?;
    Ok(result)
}
#[tauri::command(async)]
pub fn entity_delete(state: tauri::State<crate::Session>, id: String) -> crate::Reply<()> {
    state.read(|w| delete(w, "entity", "entity", &id))
}
#[tauri::command(async)]
pub fn document_delete(state: tauri::State<crate::Session>, id: String) -> crate::Reply<()> {
    state.read(|w| {
        let doc = crate::database::one(&w.db, "SELECT * FROM document WHERE id=?1", &id)?;
        if doc["kind"] != "note" {
            return Err("Delete this document through its entry or chapter".into());
        }
        delete(w, "document", "note", &id)
    })
}
#[tauri::command(async)]
pub fn trash_list(state: tauri::State<crate::Session>) -> crate::Reply<Vec<Value>> {
    state.read(|w| {
        rows(
            &w.db,
            "SELECT id,kind,ref_id,name,deleted_at FROM trash ORDER BY deleted_at DESC,rowid DESC",
            [],
        )
    })
}
#[tauri::command(async)]
pub fn trash_restore(state: tauri::State<crate::Session>, id: String) -> crate::Reply<Value> {
    state.read(|w| restore(w, &id))
}
#[tauri::command(async)]
pub fn trash_purge(state: tauri::State<crate::Session>, id: Option<String>) -> crate::Reply<()> {
    state.read(|w| {
        w.transaction(|db| {
            db.execute("DELETE FROM trash WHERE ?1 IS NULL OR id=?1", [id])?;
            Ok(())
        })
    })
}
