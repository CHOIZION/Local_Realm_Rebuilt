use crate::{
    database::{new_id, now, one, update_fields},
    world::{rows, Result, World},
};
use rusqlite::params;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

pub fn columns(world: &World, type_id: Option<&str>) -> Result<Vec<Value>> {
    let mut columns = Vec::new();
    for (key, label) in [
        ("name", "Name"),
        ("type", "Category"),
        ("summary", "Summary"),
        ("tags", "Tags"),
        ("aliases", "Aliases"),
        ("created_at", "Created"),
        ("updated_at", "Updated"),
    ] {
        columns.push(json!({"key":key,"label":label,"numeric":false,"data_type":"short_text"}));
    }
    let properties = match type_id {
        Some(id) => crate::schema::properties(&world.db, id)?,
        None => rows(
            &world.db,
            "SELECT * FROM property_definition ORDER BY sort_order,key,id",
            [],
        )?,
    };
    let mut keys: HashSet<String> = columns
        .iter()
        .filter_map(|c| c["key"].as_str().map(str::to_owned))
        .collect();
    for p in properties {
        let key = p["key"].as_str().ok_or("Missing property key")?;
        let data_type = p["data_type"].as_str().unwrap_or("");
        if data_type == "section" || !keys.insert(key.into()) {
            continue;
        }
        columns.push(json!({"key":key,"label":p["label"],"numeric":(["integer","decimal","rating"].contains(&data_type)),"data_type":data_type,"property_id":p["id"]}));
    }
    Ok(columns)
}
fn text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Array(a) => a
            .iter()
            .map(text)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", "),
        Value::Bool(b) => if *b { "Yes" } else { "No" }.into(),
        _ => value.to_string(),
    }
}
fn cell(value: Value, data_type: &str, names: &HashMap<String, Value>) -> Value {
    let mut result = json!({"text":text(&value),"number":value.as_f64(),"refs":[],"value":value});
    if ["entity_ref", "entity_ref_multi"].contains(&data_type) {
        let ids = if let Some(id) = value.as_str() {
            vec![id.to_owned()]
        } else {
            value
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default()
        };
        let refs: Vec<Value> = ids
            .into_iter()
            .map(|id| {
                names
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| json!({"id":id,"name":"Missing entry","missing":true}))
            })
            .collect();
        result["text"] = json!(refs
            .iter()
            .filter_map(|r| r["name"].as_str())
            .collect::<Vec<_>>()
            .join(", "));
        result["refs"] = json!(refs);
    }
    result
}
fn matches_filter(cells: &Map<String, Value>, filter: &Value) -> Result<bool> {
    let key = filter["key"].as_str().ok_or("Missing filter column")?;
    let current = cells.get(key).cloned().unwrap_or(Value::Null);
    let value = &filter["value"];
    let actual = current["text"].as_str().unwrap_or("").to_lowercase();
    let expected = text(value).to_lowercase();
    let compare_number = || -> Option<std::cmp::Ordering> {
        let left = current["number"].as_f64().or_else(|| actual.parse().ok())?;
        let right = value.as_f64().or_else(|| expected.parse().ok())?;
        left.partial_cmp(&right)
    };
    use std::cmp::Ordering::*;
    Ok(match filter["op"].as_str().unwrap_or("contains") {
        "contains" => actual.contains(&expected),
        "equals" => actual == expected,
        "not_equals" => actual != expected,
        "empty" => actual.is_empty(),
        "not_empty" => !actual.is_empty(),
        "gt" => compare_number() == Some(Greater),
        "gte" => matches!(compare_number(), Some(Greater | Equal)),
        "lt" => compare_number() == Some(Less),
        "lte" => matches!(compare_number(), Some(Less | Equal)),
        _ => return Err("Unknown table filter".into()),
    })
}

pub fn run(world: &World, type_id: Option<&str>, config: &Value) -> Result<Value> {
    let available = columns(world, type_id)?;
    let selected = config["columns"]
        .as_array()
        .filter(|a| !a.is_empty())
        .cloned()
        .unwrap_or_else(|| vec![json!("name"), json!("type"), json!("summary")]);
    let columns: Vec<Value> = selected
        .iter()
        .filter_map(|key| available.iter().find(|c| c["key"] == *key).cloned())
        .collect();
    let names:HashMap<String,Value>=rows(&world.db,"SELECT e.id,e.name,t.name AS type_name,0 AS missing FROM entity e JOIN entity_type t ON t.id=e.type_id",[])?.into_iter().filter_map(|n|Some((n["id"].as_str()?.to_owned(),n))).collect();
    let mut allowed = HashSet::new();
    if let Some(id) = type_id {
        one(&world.db, "SELECT id FROM entity_type WHERE id=?1", id)?;
        allowed.insert(id.to_owned());
        if config["include_subtypes"] != false {
            for category in rows(&world.db,"WITH RECURSIVE descendants(id) AS (SELECT id FROM entity_type WHERE id=?1 UNION SELECT t.id FROM entity_type t JOIN descendants d ON t.parent_id=d.id) SELECT id FROM descendants",[id])? {
                if let Some(id)=category["id"].as_str(){allowed.insert(id.into());}
            }
        }
    }
    let all = world.entities(&json!({"limit":i64::MAX,"sort":"name"}))?;
    let mut properties: HashMap<String, HashMap<String, Value>> = HashMap::new();
    for p in rows(&world.db,"SELECT v.entity_id,d.id AS property_id,v.value FROM property_value v JOIN property_definition d ON d.id=v.property_id",[])? {
        properties.entry(p["entity_id"].as_str().ok_or("Missing entity id")?.into()).or_default().insert(p["property_id"].as_str().ok_or("Missing property id")?.into(),p["value"].clone());
    }
    let mut definitions: HashMap<String, Vec<Value>> = HashMap::new();
    let mut result = Vec::new();
    for entity in all {
        let category = entity["type_id"].as_str().ok_or("Missing category")?;
        if !allowed.is_empty() && !allowed.contains(category) {
            continue;
        }
        let id = entity["id"].as_str().ok_or("Missing entity id")?;
        if !definitions.contains_key(category) {
            definitions.insert(
                category.into(),
                crate::schema::properties(&world.db, category)?,
            );
        }
        let mut cells = Map::new();
        for key in [
            "name",
            "summary",
            "tags",
            "aliases",
            "created_at",
            "updated_at",
        ] {
            cells.insert(key.into(), cell(entity[key].clone(), "short_text", &names));
        }
        cells.insert(
            "type".into(),
            cell(entity["type_name"].clone(), "short_text", &names),
        );
        for p in &definitions[category] {
            let key = p["key"].as_str().ok_or("Missing property key")?;
            if cells.contains_key(key) {
                continue;
            }
            let value = properties
                .get(id)
                .and_then(|pmap| pmap.get(p["id"].as_str().unwrap_or("")))
                .cloned()
                .unwrap_or(Value::Null);
            cells.insert(
                key.into(),
                cell(value, p["data_type"].as_str().unwrap_or(""), &names),
            );
        }
        let mut matched = true;
        if let Some(filters) = config["filters"].as_array() {
            for filter in filters {
                if !matches_filter(&cells, filter)? {
                    matched = false;
                    break;
                }
            }
        }
        if !matched {
            continue;
        }
        if let Some(search) = config["search"].as_str().filter(|s| !s.is_empty()) {
            let needle = search.to_lowercase();
            if !cells.values().any(|c| {
                c["text"]
                    .as_str()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&needle)
            }) {
                continue;
            }
        }
        let group = config["group_by"]
            .as_str()
            .and_then(|key| cells.get(key))
            .and_then(|c| c["text"].as_str())
            .unwrap_or("")
            .to_string();
        result.push(json!({"id":id,"name":entity["name"],"type_id":category,"type_name":entity["type_name"],"cells":cells,"group":group}));
    }
    let sort_key = config["sort"]["key"].as_str().unwrap_or("name");
    let descending = config["sort"]["direction"] == "desc";
    result.sort_by(|a, b| {
        let group = a["group"].as_str().cmp(&b["group"].as_str());
        if !group.is_eq() {
            return group;
        }
        let left = &a["cells"][sort_key];
        let right = &b["cells"][sort_key];
        let order = match (left["number"].as_f64(), right["number"].as_f64()) {
            (Some(a), Some(b)) => a.total_cmp(&b),
            _ => left["text"]
                .as_str()
                .unwrap_or("")
                .to_lowercase()
                .cmp(&right["text"].as_str().unwrap_or("").to_lowercase()),
        };
        (if descending { order.reverse() } else { order })
            .then_with(|| a["id"].as_str().cmp(&b["id"].as_str()))
    });
    let mut totals = Map::new();
    for column in columns.iter().filter(|c| c["numeric"] == true) {
        let key = column["key"].as_str().ok_or("Missing column key")?;
        totals.insert(
            key.into(),
            json!(result
                .iter()
                .filter_map(|r| r["cells"][key]["number"].as_f64())
                .sum::<f64>()),
        );
    }
    Ok(json!({"columns":columns,"rows":result,"total":result.len(),"totals":totals}))
}
#[tauri::command(async)]
pub fn table_columns(
    state: tauri::State<crate::Session>,
    type_id: Option<String>,
) -> crate::Reply<Vec<Value>> {
    state.read(|w| columns(w, type_id.as_deref()))
}
#[tauri::command(async)]
pub fn table_preview(
    state: tauri::State<crate::Session>,
    type_id: Option<String>,
    config: Value,
) -> crate::Reply<Value> {
    state.read(|w| run(w, type_id.as_deref(), &config))
}
#[tauri::command(async)]
pub fn table_run(state: tauri::State<crate::Session>, id: String) -> crate::Reply<Value> {
    state.read(|w| {
        let table = one(&w.db, "SELECT * FROM table_view WHERE id=?1", &id)?;
        run(w, table["type_id"].as_str(), &table["config"])
    })
}
#[tauri::command(async)]
pub fn table_get(state: tauri::State<crate::Session>, id: String) -> crate::Reply<Value> {
    state.read(|w| one(&w.db, "SELECT * FROM table_view WHERE id=?1", &id))
}
#[tauri::command(async)]
pub fn table_create(
    state: tauri::State<crate::Session>,
    name: String,
    type_id: Option<String>,
) -> crate::Reply<Value> {
    state.read(|w|w.transaction(|db| {
        if name.trim().is_empty(){return Err("Give the table a name".into());}
        let id=new_id();let config=json!({"columns":["name","type","summary"],"filters":[],"include_subtypes":true});
        db.execute("INSERT INTO table_view(id,name,type_id,config,sort_order,created_at,updated_at) VALUES(?1,?2,?3,?4,(SELECT coalesce(max(sort_order),0)+100 FROM table_view),?5,?5)",params![id,name.trim(),type_id,config.to_string(),now()])?;
        one(db,"SELECT * FROM table_view WHERE id=?1",&id)
    }))
}
#[tauri::command(async)]
pub fn table_update(
    state: tauri::State<crate::Session>,
    id: String,
    name: String,
    type_id: Option<String>,
    config: Value,
) -> crate::Reply<Value> {
    state.read(|w| {
        w.transaction(|db| {
            if name.trim().is_empty() {
                return Err("Give the table a name".into());
            }
            update_fields(
                db,
                "table_view",
                &id,
                &json!({"name":name.trim(),"type_id":type_id,"config":config}),
                &["name", "type_id", "config"],
                true,
            )?;
            one(db, "SELECT * FROM table_view WHERE id=?1", &id)
        })
    })
}
#[tauri::command(async)]
pub fn table_delete(state: tauri::State<crate::Session>, id: String) -> crate::Reply<()> {
    state.read(|w| crate::trash::delete(w, "table_view", "table", &id))
}
