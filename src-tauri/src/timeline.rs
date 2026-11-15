use crate::{
    calendar::{self, Calendar},
    database::{new_id, now, one, required_text, update_fields},
    world::{rows, Result, World},
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

pub fn events(db: &Connection, mut events: Vec<Value>) -> Result<Vec<Value>> {
    let mut calendars = HashMap::new();
    for event in &mut events {
        let timeline_id = event["timeline_id"]
            .as_str()
            .ok_or("Missing timeline")?
            .to_string();
        if !calendars.contains_key(&timeline_id) {
            let timeline = one(db, "SELECT * FROM timeline WHERE id=?1", &timeline_id)?;
            let record = calendar::resolve(db, timeline["calendar_id"].as_str())?;
            calendars.insert(
                timeline_id.clone(),
                (
                    timeline["name"].clone(),
                    Calendar::new(&record["definition"])?,
                ),
            );
        }
        let (name, calendar) = &calendars[&timeline_id];
        event["timeline_name"] = name.clone();
        event["start_label"] = json!(calendar.format(&event["start_date"])?);
        event["end_label"] = if event["end_date"].is_null() {
            Value::Null
        } else {
            json!(calendar.format(&event["end_date"])?)
        };
        event["entities"]=json!(rows(db,"SELECT e.id,e.name,e.type_id,t.name AS type_name,t.color,t.icon FROM timeline_event_entity x JOIN entity e ON e.id=x.entity_id JOIN entity_type t ON t.id=e.type_id WHERE x.event_id=?1 ORDER BY x.sort_order,e.name",[event["id"].as_str()])?);
        event["place"] = if let Some(marker) = event["marker_id"].as_str() {
            rows(db,"SELECT p.id AS marker_id,p.map_id,m.name AS map_name,p.x,p.y,coalesce(nullif(p.label,''),e.name,'Pin') AS label FROM map_marker p JOIN map m ON m.id=p.map_id LEFT JOIN entity e ON e.id=p.entity_id WHERE p.id=?1",[marker])?.pop().unwrap_or(Value::Null)
        } else {
            Value::Null
        };
    }
    Ok(events)
}

pub fn get(world: &World, id: &str, filter: &Value) -> Result<Value> {
    let mut timeline = one(&world.db, "SELECT * FROM timeline WHERE id=?1", id)?;
    let calendar =
        world.transaction(|db| calendar::resolve(db, timeline["calendar_id"].as_str()))?;
    timeline["calendar"] = calendar;
    let mut found=rows(&world.db,"SELECT e.* FROM timeline_event e WHERE timeline_id=?1
        AND (?2 IS NULL OR EXISTS(SELECT 1 FROM timeline_event_entity x WHERE x.event_id=e.id AND x.entity_id=?2))
        AND (?3 IS NULL OR EXISTS(SELECT 1 FROM timeline_event_entity x JOIN entity_tag et ON et.entity_id=x.entity_id JOIN tag t ON t.id=et.tag_id WHERE x.event_id=e.id AND (t.name=?3 COLLATE NOCASE OR t.id=?3)))
        ORDER BY start_minute,created_at,id",params![id,filter["entity_id"].as_str(),filter["tag"].as_str()])?;
    if let Some(text) = filter["text"].as_str().filter(|s| !s.is_empty()) {
        let text = text.to_lowercase();
        found.retain(|e| {
            e["title"]
                .as_str()
                .unwrap_or("")
                .to_lowercase()
                .contains(&text)
                || e["summary"]
                    .as_str()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&text)
        });
    }
    let start = found
        .iter()
        .filter_map(|e| e["start_minute"].as_i64())
        .min();
    let end = found
        .iter()
        .filter_map(|e| {
            e["end_minute"]
                .as_i64()
                .or_else(|| e["start_minute"].as_i64())
        })
        .max();
    timeline["range"] = json!({"from":start,"to":end});
    timeline["span"] = json!([start.unwrap_or(0), end.unwrap_or(525600)]);
    timeline["events"] = json!(events(&world.db, found)?);
    Ok(timeline)
}

pub fn create_in(db: &Connection, name: &str, calendar_id: Option<&str>) -> Result<String> {
    if name.trim().is_empty() {
        return Err("Give the timeline a name".into());
    }
    let calendar = calendar::resolve(db, calendar_id)?;
    let id = new_id();
    db.execute("INSERT INTO timeline(id,name,calendar_id,sort_order,created_at,updated_at) VALUES(?1,?2,?3,(SELECT coalesce(max(sort_order),0)+100 FROM timeline),?4,?4)",params![id,name.trim(),calendar["id"].as_str(),now()])?;
    Ok(id)
}

pub fn save_event(
    world: &World,
    timeline_id: &str,
    id: Option<&str>,
    input: &Value,
) -> Result<Value> {
    world.transaction(|db| {
        let timeline=one(db,"SELECT * FROM timeline WHERE id=?1",timeline_id)?;
        let record=calendar::resolve(db,timeline["calendar_id"].as_str())?;
        let calendar=Calendar::new(&record["definition"])?;
        let id=id.map(str::to_owned).unwrap_or_else(new_id);
        let mut merged=if let Some(existing)=rows(db,"SELECT * FROM timeline_event WHERE id=?1",[&id])?.pop(){existing}else{json!({"summary":"","kind":"event","end_date":null,"marker_id":null,"color":""})};
        if !merged["timeline_id"].is_null() && merged["timeline_id"]!=timeline_id {return Err("Event belongs to another timeline".into());}
        for (key,value) in input.as_object().ok_or("Expected event fields")? {merged[key]=value.clone();}
        let title=required_text(&merged,"title")?;
        let start=calendar.minute(&merged["start_date"])?;
        let kind=merged["kind"].as_str().ok_or("Missing event kind")?;
        if !["event","period"].contains(&kind){return Err("Invalid event kind".into());}
        let end=if kind=="period" {Some(calendar.minute(&merged["end_date"])?) }else{None};
        if end.is_some_and(|end|end<start){return Err("An event cannot end before it starts".into());}
        let end_date=end.map(|_|merged["end_date"].to_string());
        db.execute("INSERT INTO timeline_event(id,timeline_id,title,summary,kind,start_date,end_date,start_minute,end_minute,marker_id,color,created_at,updated_at)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?12)
            ON CONFLICT(id) DO UPDATE SET title=excluded.title,summary=excluded.summary,kind=excluded.kind,start_date=excluded.start_date,end_date=excluded.end_date,start_minute=excluded.start_minute,end_minute=excluded.end_minute,marker_id=excluded.marker_id,color=excluded.color,updated_at=excluded.updated_at",
            params![id,timeline_id,title.trim(),merged["summary"].as_str().unwrap_or(""),kind,merged["start_date"].to_string(),end_date,start,end,merged["marker_id"].as_str(),merged["color"].as_str().unwrap_or(""),now()])?;
        if let Some(ids)=input.get("entity_ids") {
            let ids=ids.as_array().ok_or("Expected entity ids")?;
            db.execute("DELETE FROM timeline_event_entity WHERE event_id=?1",[&id])?;
            let mut seen=HashSet::new();
            for (order,entity) in ids.iter().enumerate() {
                let entity=entity.as_str().ok_or("Invalid entity id")?;
                if seen.insert(entity) {db.execute("INSERT INTO timeline_event_entity(event_id,entity_id,sort_order) VALUES(?1,?2,?3)",params![id,entity,order])?;}
            }
            db.execute("UPDATE timeline_event SET entity_id=?1 WHERE id=?2",params![ids.first().and_then(Value::as_str),id])?;
        }
        events(db,vec![one(db,"SELECT * FROM timeline_event WHERE id=?1",&id)?])?.pop().ok_or_else(||"Missing event".into())
    })
}

#[tauri::command(async)]
pub fn timeline_get(
    state: tauri::State<crate::Session>,
    id: String,
    filter: Option<Value>,
) -> crate::Reply<Value> {
    state.read(|w| get(w, &id, &filter.unwrap_or_else(|| json!({}))))
}
#[tauri::command(async)]
pub fn timeline_create(
    state: tauri::State<crate::Session>,
    name: String,
    calendar_id: Option<String>,
) -> crate::Reply<Value> {
    state.read(|w| {
        let id = w.transaction(|db| create_in(db, &name, calendar_id.as_deref()))?;
        get(w, &id, &json!({}))
    })
}
#[tauri::command(async)]
pub fn timeline_update(
    state: tauri::State<crate::Session>,
    id: String,
    name: String,
    description: String,
    calendar_id: Option<String>,
) -> crate::Reply<Value> {
    state.read(|w| {
        if name.trim().is_empty() {
            return Err("Give the timeline a name".into());
        }
        w.transaction(|db| {
            calendar::resolve(db, calendar_id.as_deref())?;
            update_fields(
                db,
                "timeline",
                &id,
                &json!({"name":name.trim(),"description":description,"calendar_id":calendar_id}),
                &["name", "description", "calendar_id"],
                true,
            )?;
            calendar::recalculate(db)
        })?;
        get(w, &id, &json!({}))
    })
}
#[tauri::command(async)]
pub fn timeline_delete(state: tauri::State<crate::Session>, id: String) -> crate::Reply<()> {
    state.read(|w| crate::trash::delete(w, "timeline", "timeline", &id))
}
#[tauri::command(async)]
pub fn timeline_event_create(
    state: tauri::State<crate::Session>,
    timeline_id: String,
    input: Value,
) -> crate::Reply<Value> {
    state.read(|w| save_event(w, &timeline_id, None, &input))
}
#[tauri::command(async)]
pub fn timeline_event_update(
    state: tauri::State<crate::Session>,
    id: String,
    input: Value,
) -> crate::Reply<Value> {
    state.read(|w| {
        let current = one(&w.db, "SELECT * FROM timeline_event WHERE id=?1", &id)?;
        save_event(
            w,
            current["timeline_id"].as_str().ok_or("Missing timeline")?,
            Some(&id),
            &input,
        )
    })
}
#[tauri::command(async)]
pub fn timeline_event_delete(state: tauri::State<crate::Session>, id: String) -> crate::Reply<()> {
    state.read(|w| {
        w.transaction(|db| {
            db.execute("DELETE FROM timeline_event WHERE id=?1", [id])?;
            Ok(())
        })
    })
}
#[tauri::command(async)]
pub fn timeline_appearances(
    state: tauri::State<crate::Session>,
    entity_id: String,
) -> crate::Reply<Vec<Value>> {
    state.read(|w|events(&w.db,rows(&w.db,"SELECT e.* FROM timeline_event e WHERE EXISTS(SELECT 1 FROM timeline_event_entity x WHERE x.event_id=e.id AND x.entity_id=?1) ORDER BY start_minute",[entity_id])?))
}
#[tauri::command(async)]
pub fn map_pins(state: tauri::State<crate::Session>) -> crate::Reply<Vec<Value>> {
    state.read(|w|rows(&w.db,"SELECT p.id,p.id AS marker_id,p.map_id,m.name AS map_name,p.entity_id,coalesce(nullif(p.label,''),e.name,'Pin') AS label,p.x,p.y FROM map_marker p JOIN map m ON m.id=p.map_id LEFT JOIN entity e ON e.id=p.entity_id ORDER BY m.name,label",[]))
}
#[tauri::command(async)]
pub fn map_events(state: tauri::State<crate::Session>, map_id: String) -> crate::Reply<Vec<Value>> {
    state.read(|w| {
        let found=rows(&w.db,"SELECT e.*,p.id AS pin_id,p.x,p.y,CASE WHEN e.marker_id IS NULL THEN linked.name ELSE NULL END AS via
            FROM timeline_event e JOIN map_marker p ON p.id=e.marker_id OR (e.marker_id IS NULL AND EXISTS(SELECT 1 FROM timeline_event_entity x WHERE x.event_id=e.id AND x.entity_id=p.entity_id))
            LEFT JOIN entity linked ON linked.id=p.entity_id WHERE p.map_id=?1 ORDER BY e.start_minute,e.id,p.id",[map_id])?;
        let mut found=events(&w.db,found)?;
        for event in &mut found {event["marker_id"]=event["pin_id"].clone();}
        Ok(found)
    })
}
