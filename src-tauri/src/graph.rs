use crate::{
    database::{new_id, now, one, required_text, update_fields},
    world::{rows, Result, World},
};
use rusqlite::params;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

fn strings(value: &Value) -> HashSet<String> {
    value
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

pub fn build(world: &World, query: &Value) -> Result<Value> {
    let mut nodes=rows(&world.db,"SELECT e.id,e.name,e.summary,e.type_id,t.name AS type_name,t.color,t.icon,m.path AS image_path,e.image_frame FROM entity e JOIN entity_type t ON t.id=e.type_id LEFT JOIN media m ON m.id=e.image_media_id ORDER BY e.name COLLATE NOCASE,e.id",[])?;
    let catalog = crate::relationships::kinds(&world.db)?;
    let labels: HashMap<_, _> = catalog
        .iter()
        .filter_map(|k| Some((k["key"].as_str()?, k["label"].as_str()?)))
        .collect();
    let mut edges = Vec::new();
    if query["include_relationships"] != false {
        for relation in rows(&world.db, "SELECT * FROM relationship ORDER BY id", [])? {
            let kind = relation["kind"].as_str().unwrap_or("");
            edges.push(json!({"id":relation["id"],"source":relation["source_id"],"target":relation["target_id"],"kind":kind,"label":labels.get(kind).copied().unwrap_or(kind),"symmetrical":relation["symmetrical"],"origin":"relationship"}));
        }
    }
    if query["include_properties"] != false || query["include_articles"] != false {
        for link in rows(
            &world.db,
            "SELECT * FROM link WHERE source_kind='entity' ORDER BY source_id,target_id,context",
            [],
        )? {
            let body = link["context"] == "body";
            if (body && query["include_articles"] == false)
                || (!body && query["include_properties"] == false)
            {
                continue;
            }
            let label = if body {
                "Article link"
            } else {
                link["context"].as_str().unwrap_or("Property")
            };
            let kind = if body { "article" } else { "property" };
            edges.push(json!({"id":format!("{kind}:{}:{}:{}",link["source_id"].as_str().unwrap_or(""),link["target_id"].as_str().unwrap_or(""),label),"source":link["source_id"],"target":link["target_id"],"kind":kind,"label":label,"symmetrical":false,"origin":kind}));
        }
    }
    let kinds: BTreeSet<String> = edges
        .iter()
        .filter_map(|e| e["kind"].as_str().map(str::to_owned))
        .collect();
    let filter_kinds = strings(&query["kinds"]);
    if !filter_kinds.is_empty() {
        edges.retain(|e| e["kind"].as_str().is_some_and(|k| filter_kinds.contains(k)));
    }
    let filter_types = strings(&query["type_ids"]);
    if !filter_types.is_empty() {
        nodes.retain(|n| {
            n["type_id"]
                .as_str()
                .is_some_and(|t| filter_types.contains(t))
        });
    }
    let hidden = strings(&query["hidden"]);
    let pruned: Vec<Value> = nodes
        .iter()
        .filter(|n| n["id"].as_str().is_some_and(|id| hidden.contains(id)))
        .cloned()
        .collect();
    nodes.retain(|n| n["id"].as_str().is_some_and(|id| !hidden.contains(id)));
    let available: HashSet<_> = nodes
        .iter()
        .filter_map(|n| n["id"].as_str().map(str::to_owned))
        .collect();
    edges.retain(|e| {
        e["source"]
            .as_str()
            .is_some_and(|id| available.contains(id))
            && e["target"]
                .as_str()
                .is_some_and(|id| available.contains(id))
    });
    let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();
    for edge in &edges {
        let a = edge["source"].as_str().ok_or("Missing edge source")?;
        let b = edge["target"].as_str().ok_or("Missing edge target")?;
        adjacency.entry(a.into()).or_default().push(b.into());
        adjacency.entry(b.into()).or_default().push(a.into());
    }
    let focus = query["focus"].as_str();
    if let Some(focus) = focus {
        let depth = query["depth"].as_u64().unwrap_or(2).min(10000);
        let mut reached = HashSet::from([focus.to_owned()]);
        let mut pending = VecDeque::from([(focus.to_owned(), 0)]);
        while let Some((id, level)) = pending.pop_front() {
            if depth != 0 && level >= depth {
                continue;
            }
            if let Some(neighbors) = adjacency.get(&id) {
                for next in neighbors {
                    if reached.insert(next.clone()) {
                        pending.push_back((next.clone(), level + 1));
                    }
                }
            }
        }
        nodes.retain(|n| n["id"].as_str().is_some_and(|id| reached.contains(id)));
    }
    for node in &mut nodes {
        let id = node["id"].as_str().ok_or("Missing node id")?;
        let degree = adjacency.get(id).map(Vec::len).unwrap_or(0);
        let is_focus = Some(id) == focus;
        node["degree"] = json!(degree);
        node["focus"] = json!(is_focus);
        node["root"] = json!(is_focus);
    }
    let limit = query["limit"].as_u64().unwrap_or(2500).clamp(1, 10000) as usize;
    let truncated = nodes.len() > limit;
    if truncated {
        nodes.sort_by(|a, b| {
            b["focus"]
                .as_bool()
                .cmp(&a["focus"].as_bool())
                .then_with(|| b["degree"].as_u64().cmp(&a["degree"].as_u64()))
                .then_with(|| a["name"].as_str().cmp(&b["name"].as_str()))
        });
        nodes.truncate(limit);
    }
    let selected: HashSet<_> = nodes.iter().filter_map(|n| n["id"].as_str()).collect();
    edges.retain(|e| {
        e["source"].as_str().is_some_and(|id| selected.contains(id))
            && e["target"].as_str().is_some_and(|id| selected.contains(id))
    });
    Ok(json!({"nodes":nodes,"edges":edges,"kinds":kinds,"truncated":truncated,"pruned":pruned}))
}

pub fn get_tree(world: &World, id: &str) -> Result<Value> {
    one(&world.db,"SELECT t.*,e.name AS root_name FROM relationship_tree t LEFT JOIN entity e ON e.id=t.root_entity_id WHERE t.id=?1",id)
}
pub fn tree_build_for(world: &World, id: &str) -> Result<Value> {
    let tree = get_tree(world, id)?;
    if tree["root_entity_id"].is_null() {
        return Ok(json!({"nodes":[],"edges":[],"kinds":[],"pruned":[],"truncated":false}));
    }
    build(
        world,
        &json!({"focus":tree["root_entity_id"],"depth":tree["config"]["depth"].as_u64().unwrap_or(0),"hidden":tree["config"]["hidden"],"kinds":tree["kinds"],"include_properties":false,"include_articles":false}),
    )
}
#[tauri::command(async)]
pub fn graph_build(
    state: tauri::State<crate::Session>,
    query: Option<Value>,
) -> crate::Reply<Value> {
    state.read(|w| build(w, &query.unwrap_or_else(|| json!({}))))
}
#[tauri::command(async)]
pub fn graph_degree(state: tauri::State<crate::Session>, entity_id: String) -> crate::Reply<i64> {
    state.read(|w|Ok(w.db.query_row("SELECT (SELECT count(*) FROM relationship WHERE source_id=?1 OR target_id=?1)+(SELECT count(*) FROM link WHERE (source_kind='entity' AND source_id=?1) OR target_id=?1)",[entity_id],|r|r.get(0))?))
}
#[tauri::command(async)]
pub fn tree_get(state: tauri::State<crate::Session>, id: String) -> crate::Reply<Value> {
    state.read(|w| get_tree(w, &id))
}
#[tauri::command(async)]
pub fn tree_build(state: tauri::State<crate::Session>, id: String) -> crate::Reply<Value> {
    state.read(|w| tree_build_for(w, &id))
}
#[tauri::command(async)]
pub fn tree_create(
    state: tauri::State<crate::Session>,
    name: String,
    root_entity_id: Option<String>,
) -> crate::Reply<Value> {
    state.read(|w| {
        if name.trim().is_empty(){return Err("Give the tree a name".into());}
        let id=w.transaction(|db| {
            let id=new_id();let config=json!({"depth":0,"orientation":"down","hidden":[],"positions":{}});
            db.execute("INSERT INTO relationship_tree(id,name,root_entity_id,config,sort_order,created_at,updated_at) VALUES(?1,?2,?3,?4,(SELECT coalesce(max(sort_order),0)+100 FROM relationship_tree),?5,?5)",params![id,name.trim(),root_entity_id,config.to_string(),now()])?;
            Ok(id)
        })?;
        get_tree(w,&id)
    })
}
#[tauri::command(async)]
pub fn tree_update(
    state: tauri::State<crate::Session>,
    id: String,
    input: Value,
) -> crate::Reply<Value> {
    state.read(|w| {
        if input.get("name").is_some() {
            required_text(&input, "name")?;
        }
        w.transaction(|db| {
            update_fields(
                db,
                "relationship_tree",
                &id,
                &input,
                &["name", "description", "root_entity_id", "kinds", "config"],
                true,
            )
        })?;
        get_tree(w, &id)
    })
}
#[tauri::command(async)]
pub fn tree_delete(state: tauri::State<crate::Session>, id: String) -> crate::Reply<()> {
    state.read(|w| crate::trash::delete(w, "relationship_tree", "tree", &id))
}
#[tauri::command(async)]
pub fn tree_kinds(state: tauri::State<crate::Session>) -> crate::Reply<Vec<Value>> {
    state.read(|w| {
        let catalog = crate::relationships::kinds(&w.db)?;
        let mut kinds = rows(
            &w.db,
            "SELECT kind,count(*) AS count FROM relationship GROUP BY kind ORDER BY kind",
            [],
        )?;
        for kind in &mut kinds {
            kind["label"] = catalog
                .iter()
                .find(|k| k["key"] == kind["kind"])
                .map(|k| k["label"].clone())
                .unwrap_or_else(|| kind["kind"].clone());
        }
        Ok(kinds)
    })
}
#[tauri::command(async)]
pub fn tree_memberships(
    state: tauri::State<crate::Session>,
    entity_id: String,
) -> crate::Reply<Vec<Value>> {
    state.read(|w| {
        let mut found = Vec::new();
        for mut tree in rows(
            &w.db,
            "SELECT id,name FROM relationship_tree ORDER BY sort_order,name",
            [],
        )? {
            let graph = tree_build_for(w, tree["id"].as_str().ok_or("Missing tree id")?)?;
            if graph["nodes"]
                .as_array()
                .is_some_and(|nodes| nodes.iter().any(|n| n["id"] == entity_id))
            {
                tree["tree_id"] = tree["id"].clone();
                tree["tree_name"] = tree["name"].clone();
                found.push(tree);
            }
        }
        Ok(found)
    })
}
