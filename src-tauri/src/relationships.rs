use crate::{
    database::{new_id, now, one, required_text},
    world::{rows, Result},
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};

pub fn kinds(db: &Connection) -> Result<Vec<Value>> {
    let mut kinds=rows(db,"SELECT *,1 AS custom,'custom' AS \"group\" FROM relationship_kind ORDER BY sort_order,label",[])?;
    for kind in &mut kinds {
        kind["custom"] = json!(true);
    }
    kinds.extend(serde_json::from_str::<Vec<Value>>(include_str!(
        "../resources/relationships.json"
    ))?);
    Ok(kinds)
}
pub fn for_entity(db: &Connection, id: &str) -> Result<Vec<Value>> {
    let kinds = kinds(db)?;
    let mut relationships = rows(
        db,
        "SELECT * FROM relationship WHERE source_id=?1 OR target_id=?1 ORDER BY created_at,id",
        [id],
    )?;
    for relationship in &mut relationships {
        let outgoing = relationship["source_id"] == id;
        let other = if outgoing {
            &relationship["target_id"]
        } else {
            &relationship["source_id"]
        };
        let other_id = other.as_str().ok_or("Missing relationship target")?;
        let resolved=one(db,"SELECT e.id,e.name,e.type_id,t.name AS type_name FROM entity e JOIN entity_type t ON e.type_id=t.id WHERE e.id=?1",other_id)?;
        let definition = kinds.iter().find(|k| k["key"] == relationship["kind"]);
        let label = if outgoing || relationship["symmetrical"] == true {
            definition
                .map(|d| d["label"].clone())
                .unwrap_or_else(|| relationship["kind"].clone())
        } else {
            definition
                .and_then(|d| d.get("inverse_label"))
                .filter(|s| s.is_string())
                .cloned()
                .unwrap_or_else(|| relationship["inverse_kind"].clone())
        };
        relationship["label"] = label;
        relationship["other"] = resolved;
        relationship["outgoing"] = json!(outgoing);
    }
    Ok(relationships)
}
#[tauri::command(async)]
pub fn relation_kinds(state: tauri::State<crate::Session>) -> crate::Reply<Vec<Value>> {
    state.read(|w| kinds(&w.db))
}
#[tauri::command(async)]
pub fn relation_kind_create(
    state: tauri::State<crate::Session>,
    input: Value,
) -> crate::Reply<Value> {
    state.read(|w|w.transaction(|db|{
        let label=required_text(&input,"label")?.trim();
        let inverse=input["inverse_label"].as_str().filter(|s|!s.trim().is_empty());
        let key=input["key"].as_str().map(String::from).unwrap_or_else(||format!("custom_{}",new_id()));
        let inverse_key=inverse.map(|_|format!("{key}_inverse"));
        db.execute("INSERT INTO relationship_kind(key,label,inverse_key,inverse_label,symmetrical,sort_order,created_at)
            VALUES(?1,?2,?3,?4,?5,(SELECT coalesce(max(sort_order),0)+1 FROM relationship_kind),?6)",
            params![key,label,inverse_key.as_deref().unwrap_or(""),inverse.unwrap_or(""),inverse.is_none(),now()])?;
        Ok(json!({"key":key,"label":label,"inverse_key":inverse_key,"inverse_label":inverse,"symmetrical":inverse.is_none(),"custom":true,"group":"custom"}))
    }))
}
#[tauri::command(async)]
pub fn relation_kind_delete(state: tauri::State<crate::Session>, key: String) -> crate::Reply<()> {
    state.read(|w| {
        w.transaction(|db| {
            db.execute("DELETE FROM relationship_kind WHERE key=?1", [key])?;
            Ok(())
        })
    })
}
#[tauri::command(async)]
pub fn relationship_create(
    state: tauri::State<crate::Session>,
    input: Value,
) -> crate::Reply<Value> {
    state.read(|w|w.transaction(|db|{
        let source=required_text(&input,"source_id")?;let target=required_text(&input,"target_id")?;
        let key=required_text(&input,"kind")?;
        let catalog=kinds(db)?;
        let definition=catalog.iter().find(|k|k["key"]==key);
        let inverse=input["inverse_kind"].as_str().or_else(||definition.and_then(|d|d["inverse_key"].as_str())).unwrap_or("");
        let symmetrical=input["symmetrical"].as_bool().or_else(||definition.and_then(|d|d["symmetrical"].as_bool())).unwrap_or(inverse.is_empty());
        let id=new_id();
        db.execute("INSERT INTO relationship(id,source_id,target_id,kind,inverse_kind,symmetrical,note,start_date,end_date,created_at)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![id,source,target,key,inverse,symmetrical,
                input["note"].as_str().unwrap_or(""),input.get("start_date").filter(|v|!v.is_null()).map(Value::to_string).unwrap_or_default(),
                input.get("end_date").filter(|v|!v.is_null()).map(Value::to_string).unwrap_or_default(),now()])?;
        one(db,"SELECT * FROM relationship WHERE id=?1",&id)
    }))
}
#[tauri::command(async)]
pub fn relationship_delete(state: tauri::State<crate::Session>, id: String) -> crate::Reply<()> {
    state.read(|w| {
        w.transaction(|db| {
            db.execute("DELETE FROM relationship WHERE id=?1", [id])?;
            Ok(())
        })
    })
}
