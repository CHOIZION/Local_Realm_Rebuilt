use crate::{
    database::{new_id, now, one, required_text, update_fields},
    world::{rows, Result},
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::collections::HashSet;

pub const DATA_TYPES: &[&str] = &[
    "short_text",
    "long_text",
    "integer",
    "decimal",
    "boolean",
    "date",
    "world_date",
    "color",
    "url",
    "dropdown",
    "multi_select",
    "entity_ref",
    "entity_ref_multi",
    "image",
    "file",
    "coordinates",
    "rating",
    "section",
];

pub fn properties(db: &Connection, type_id: &str) -> Result<Vec<Value>> {
    let mut all=rows(db,"WITH RECURSIVE lineage(id,parent_id,depth) AS (
        SELECT id,parent_id,0 FROM entity_type WHERE id=?1
        UNION ALL SELECT t.id,t.parent_id,l.depth+1 FROM entity_type t JOIN lineage l ON t.id=l.parent_id WHERE l.depth<256)
        SELECT p.*,t.name AS type_name,l.depth FROM property_definition p JOIN lineage l ON p.type_id=l.id
        JOIN entity_type t ON t.id=p.type_id ORDER BY l.depth,p.sort_order,p.id",[type_id])?;
    let mut keys = HashSet::new();
    all.retain(|p| keys.insert(p["key"].as_str().unwrap_or("").to_owned()));
    all.sort_by_key(|p| {
        (
            -p["depth"].as_i64().unwrap_or(0),
            p["sort_order"].as_i64().unwrap_or(0),
        )
    });
    for property in &mut all {
        property["inherited"] = json!(property["depth"].as_i64().unwrap_or(0) > 0);
        property.as_object_mut().unwrap().remove("depth");
    }
    Ok(all)
}

pub(crate) fn check_parent(db: &Connection, id: &str, parent: Option<&str>) -> Result<()> {
    if let Some(parent) = parent {
        one(db, "SELECT id FROM entity_type WHERE id=?1", parent)?;
        let cyclic:bool=db.query_row("WITH RECURSIVE ancestors(id,parent_id) AS (
            SELECT id,parent_id FROM entity_type WHERE id=?1 UNION SELECT t.id,t.parent_id FROM entity_type t JOIN ancestors a ON t.id=a.parent_id)
            SELECT EXISTS(SELECT 1 FROM ancestors WHERE id=?2)",params![parent,id],|r|r.get(0))?;
        if cyclic {
            return Err("A category cannot be its own ancestor".into());
        }
    }
    Ok(())
}

#[tauri::command(async)]
pub fn type_properties(
    state: tauri::State<crate::Session>,
    type_id: String,
) -> crate::Reply<Vec<Value>> {
    state.read(|w| properties(&w.db, &type_id))
}
#[tauri::command(async)]
pub fn type_create(state: tauri::State<crate::Session>, input: Value) -> crate::Reply<Value> {
    state.read(|w|w.transaction(|db|{
        let name=required_text(&input,"name")?.trim();
        let id=new_id();let time=now();
        check_parent(db,&id,input["parent_id"].as_str())?;
        db.execute("INSERT INTO entity_type(id,parent_id,name,plural,icon,color,description,sort_order,created_at,updated_at)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?9)",params![id,input["parent_id"].as_str(),name,
            input["plural"].as_str().unwrap_or(name),input["icon"].as_str().unwrap_or("file"),
            input["color"].as_str().unwrap_or(""),input["description"].as_str().unwrap_or(""),
            input["sort_order"].as_i64().unwrap_or(0),time])?;
        one(db,"SELECT t.*,0 AS entity_count FROM entity_type t WHERE id=?1",&id)
    }))
}
#[tauri::command(async)]
pub fn type_update(
    state: tauri::State<crate::Session>,
    id: String,
    input: Value,
) -> crate::Reply<Value> {
    state.read(|w|w.transaction(|db|{
        if input.get("name").is_some(){required_text(&input,"name")?;}
        if input.get("parent_id").is_some(){check_parent(db,&id,input["parent_id"].as_str())?;}
        update_fields(db,"entity_type",&id,&input,&["parent_id","name","plural","icon","color","description","sort_order"],true)?;
        one(db,"SELECT t.*,(SELECT count(*) FROM entity WHERE type_id=t.id) AS entity_count FROM entity_type t WHERE t.id=?1",&id)
    }))
}
#[tauri::command(async)]
pub fn type_set_layout(
    state: tauri::State<crate::Session>,
    id: String,
    layout: Value,
) -> crate::Reply<Value> {
    state.read(|w| {
        w.transaction(|db| {
            if !(layout.is_null() || layout.is_object()) {
                return Err("Expected category layout object".into());
            }
            update_fields(
                db,
                "entity_type",
                &id,
                &json!({"layout":layout}),
                &["layout"],
                true,
            )?;
            one(db, "SELECT * FROM entity_type WHERE id=?1", &id)
        })
    })
}
#[tauri::command(async)]
pub fn type_delete(state: tauri::State<crate::Session>, id: String) -> crate::Reply<()> {
    state.read(|w| {
        w.transaction(|db| {
            one(db, "SELECT id FROM entity_type WHERE id=?1", &id)?;
            db.execute("UPDATE entity_type SET parent_id=(SELECT parent_id FROM entity_type WHERE id=?1) WHERE parent_id=?1",[&id])?;
            db.execute("DELETE FROM entity_type WHERE id=?1", [id])?;
            Ok(())
        })
    })
}
fn validate_property(input: &Value) -> Result<()> {
    if let Some(kind) = input.get("data_type") {
        if !DATA_TYPES.contains(&kind.as_str().unwrap_or("")) {
            return Err("Unknown property data type".into());
        }
    }
    if input.get("label").is_some() {
        required_text(input, "label")?;
    }
    if let Some(config) = input.get("config") {
        if !config.is_object() {
            return Err("Expected property configuration object".into());
        }
    }
    Ok(())
}
#[tauri::command(async)]
pub fn property_create(
    state: tauri::State<crate::Session>,
    type_id: String,
    input: Value,
) -> crate::Reply<Value> {
    state.read(|w|w.transaction(|db|{
        validate_property(&input)?;
        let label=required_text(&input,"label")?.trim();
        let kind=required_text(&input,"data_type")?;
        let base=input["key"].as_str().unwrap_or(label).chars().map(|c|if c.is_alphanumeric(){c.to_ascii_lowercase()}else{'_'}).collect::<String>();
        let mut key=base.clone();let mut suffix=2;
        let inherited=properties(db,&type_id)?;
        while inherited.iter().any(|p|p["key"]==key){key=format!("{base}_{suffix}");suffix+=1;}
        let id=new_id();
        db.execute("INSERT INTO property_definition(id,type_id,key,label,data_type,config,hint,required,sort_order,created_at,updated_at)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8,(SELECT coalesce(max(sort_order),0)+1 FROM property_definition WHERE type_id=?2),?9,?9)",
            params![id,type_id,key,label,kind,input.get("config").unwrap_or(&json!({})).to_string(),
                input["hint"].as_str().unwrap_or(""),input["required"].as_bool().unwrap_or(false),now()])?;
        one(db,"SELECT * FROM property_definition WHERE id=?1",&id)
    }))
}
#[tauri::command(async)]
pub fn property_update(
    state: tauri::State<crate::Session>,
    id: String,
    input: Value,
) -> crate::Reply<Value> {
    state.read(|w| {
        w.transaction(|db| {
            validate_property(&input)?;
            update_fields(
                db,
                "property_definition",
                &id,
                &input,
                &[
                    "label",
                    "data_type",
                    "config",
                    "hint",
                    "required",
                    "sort_order",
                ],
                true,
            )?;
            one(db, "SELECT * FROM property_definition WHERE id=?1", &id)
        })
    })
}
#[tauri::command(async)]
pub fn property_delete(state: tauri::State<crate::Session>, id: String) -> crate::Reply<()> {
    state.read(|w| {
        w.transaction(|db| {
            let property = one(db, "SELECT * FROM property_definition WHERE id=?1", &id)?;
            db.execute(
                "DELETE FROM link WHERE source_kind='entity' AND context=?1 AND source_id IN
            (SELECT entity_id FROM property_value WHERE property_id=?2)",
                params![property["label"].as_str(), id],
            )?;
            db.execute("DELETE FROM property_definition WHERE id=?1", [id])?;
            Ok(())
        })
    })
}
#[tauri::command(async)]
pub fn property_data_types() -> Vec<&'static str> {
    DATA_TYPES.to_vec()
}
