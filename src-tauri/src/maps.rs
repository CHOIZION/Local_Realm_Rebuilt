//! Map metadata and editable overlays. Coordinates remain normalized when artwork changes.
use crate::{
    database::{new_id, now, one, required_text, update_fields},
    world::{rows, Result, World},
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const MARKER: &[&str] = &[
    "layer_id",
    "entity_id",
    "label",
    "note",
    "x",
    "y",
    "icon",
    "color",
];
const REGION: &[&str] = &[
    "layer_id",
    "entity_id",
    "label",
    "kind",
    "points",
    "color",
    "opacity",
];
const STAMP: &[&str] = &[
    "layer_id",
    "image",
    "label",
    "x",
    "y",
    "width",
    "aspect",
    "rotation",
    "flip",
    "tint",
    "opacity",
    "sort_order",
];

pub fn get(world: &World, id: &str) -> Result<Value> {
    let mut map = one(&world.db, "SELECT m.*,f.path AS media_path FROM map m LEFT JOIN media f ON f.id=m.media_id WHERE m.id=?1", id)?;
    for (key, sql) in [
        ("layers", "SELECT * FROM map_layer WHERE map_id=?1 ORDER BY sort_order,id"),
        ("markers", "SELECT m.*,e.name AS entity_name FROM map_marker m LEFT JOIN entity e ON e.id=m.entity_id WHERE map_id=?1 ORDER BY m.created_at,m.id"),
        ("regions", "SELECT r.*,e.name AS entity_name FROM map_region r LEFT JOIN entity e ON e.id=r.entity_id WHERE map_id=?1 ORDER BY r.sort_order,r.id"),
        ("stamps", "SELECT * FROM map_stamp WHERE map_id=?1 ORDER BY sort_order,id"),
    ] {
        map[key] = json!(rows(&world.db, sql, [id])?);
    }
    for region in map["regions"].as_array_mut().ok_or("Missing regions")? {
        let outlines = rows(&world.db, "SELECT * FROM map_region_outline WHERE region_id=?1 ORDER BY json_extract(date,'$.year'),json_extract(date,'$.month'),json_extract(date,'$.day'),json_extract(date,'$.hour'),json_extract(date,'$.minute')", [region["id"].as_str()])?;
        region["outlines"] = json!(outlines);
    }
    Ok(map)
}

pub fn create_in(db: &Connection, name: &str) -> Result<String> {
    if name.trim().is_empty() {
        return Err("Give the map a name".into());
    }
    let id = new_id();
    db.execute("INSERT INTO map(id,name,sort_order,created_at,updated_at) VALUES(?1,?2,(SELECT coalesce(max(sort_order),0)+100 FROM map),?3,?3)", params![id,name.trim(),now()])?;
    layer_in(db, &id, "Places")?;
    Ok(id)
}

pub fn layer_in(db: &Connection, map_id: &str, name: &str) -> Result<Value> {
    if name.trim().is_empty() {
        return Err("Give the layer a name".into());
    }
    let id = new_id();
    db.execute("INSERT INTO map_layer(id,map_id,name,sort_order) VALUES(?1,?2,?3,(SELECT coalesce(max(sort_order),0)+100 FROM map_layer WHERE map_id=?2))", params![id,map_id,name.trim()])?;
    one(db, "SELECT * FROM map_layer WHERE id=?1", &id)
}

fn validate_points(points: &Value) -> Result<()> {
    let points = points.as_array().ok_or("Points must be an array")?;
    if points.len() > 1_000_000 {
        return Err("Too many points".into());
    }
    for point in points {
        let pair = point
            .as_array()
            .filter(|p| p.len() == 2)
            .ok_or("Expected [x,y] point")?;
        if !pair.iter().all(|v| v.as_f64().is_some_and(f64::is_finite)) {
            return Err("Point coordinates must be finite numbers".into());
        }
    }
    Ok(())
}

fn validate_feature(db: &Connection, map_id: &str, input: &Value) -> Result<()> {
    if let Some(layer) = input["layer_id"].as_str() {
        let belongs: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM map_layer WHERE id=?1 AND map_id=?2)",
            params![layer, map_id],
            |r| r.get(0),
        )?;
        if !belongs {
            return Err("The layer belongs to another map".into());
        }
    }
    for key in ["x", "y", "rotation", "width", "aspect", "opacity"] {
        if let Some(value) = input.get(key) {
            let number = value
                .as_f64()
                .filter(|v| v.is_finite())
                .ok_or_else(|| format!("Invalid {key}"))?;
            if (["width", "aspect"].contains(&key) && number <= 0.0)
                || (key == "opacity" && !(0.0..=1.0).contains(&number))
            {
                return Err(format!("Invalid {key}").into());
            }
        }
    }
    if let Some(points) = input.get("points") {
        validate_points(points)?;
    }
    if let Some(kind) = input.get("kind") {
        if kind != "area" && kind != "route" {
            return Err("Invalid region kind".into());
        }
    }
    if let Some(image) = input.get("image") {
        stamp_path(image.as_str().ok_or("Invalid stamp image")?)?;
    }
    Ok(())
}

pub fn feature_create(world: &World, table: &str, map_id: &str, input: &Value) -> Result<Value> {
    world.transaction(|db| {
        validate_feature(db,map_id,input)?;
        let id = new_id();
        let timestamp = now();
        let fields = match table {
            "map_marker" => {
                db.execute("INSERT INTO map_marker(id,map_id,x,y,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?5)",params![id,map_id,input["x"].as_f64().ok_or("Missing x")?,input["y"].as_f64().ok_or("Missing y")?,timestamp])?;
                MARKER
            }
            "map_region" => {
                db.execute("INSERT INTO map_region(id,map_id,created_at,updated_at) VALUES(?1,?2,?3,?3)",params![id,map_id,timestamp])?;
                REGION
            }
            "map_stamp" => {
                let image = required_text(input,"image")?;
                world.local_file(&stamp_path(image)?)?;
                db.execute("INSERT INTO map_stamp(id,map_id,image,x,y,width,created_at,updated_at,sort_order) VALUES(?1,?2,?3,?4,?5,?6,?7,?7,(SELECT coalesce(max(sort_order),0)+1 FROM map_stamp WHERE map_id=?2))",params![id,map_id,image,input["x"].as_f64().ok_or("Missing x")?,input["y"].as_f64().ok_or("Missing y")?,input["width"].as_f64().ok_or("Missing width")?,timestamp])?;
                STAMP
            }
            _ => return Err("Unknown map feature".into()),
        };
        update_fields(db,table,&id,input,fields,true)?;
        update_fields(db,table,&id,input,&["valid_from","valid_until"],false)?;
        one(db,&format!("SELECT * FROM {table} WHERE id=?1"),&id)
    })
}

fn feature_update(
    world: &World,
    table: &str,
    id: &str,
    input: &Value,
    fields: &[&str],
) -> Result<Value> {
    world.transaction(|db| {
        let current = one(db, &format!("SELECT * FROM {table} WHERE id=?1"), id)?;
        validate_feature(db, current["map_id"].as_str().ok_or("Missing map")?, input)?;
        if let Some(image) = input["image"].as_str() {
            world.local_file(&stamp_path(image)?)?;
        }
        update_fields(db, table, id, input, fields, true)?;
        one(db, &format!("SELECT * FROM {table} WHERE id=?1"), id)
    })
}

fn stamp_path(name: &str) -> Result<String> {
    if name.is_empty() || name.contains(['/', '\\', ':']) || name == "." || name == ".." {
        return Err("Invalid stamp image name".into());
    }
    Ok(format!("maps/stamps/{name}"))
}

pub fn stamp_image_put(world: &World, bytes: &[u8]) -> Result<String> {
    if bytes.len() > 64 * 1024 * 1024 {
        return Err("Stamp image is too large".into());
    }
    let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)?;
    if image.width() == 0
        || image.height() == 0
        || u64::from(image.width()) * u64::from(image.height()) > 16_777_216
    {
        return Err("Stamp dimensions are too large".into());
    }
    let name = format!("{:x}.png", Sha256::digest(bytes));
    world.transaction(|db| World::queue_file(db, &stamp_path(&name)?, bytes))?;
    Ok(name)
}

#[tauri::command(async)]
pub fn map_create(state: tauri::State<crate::Session>, name: String) -> crate::Reply<Value> {
    state.read(|w| {
        let id = w.transaction(|db| create_in(db, &name))?;
        get(w, &id)
    })
}
#[tauri::command(async)]
pub fn map_update(
    state: tauri::State<crate::Session>,
    id: String,
    name: String,
    description: String,
    config: Option<Value>,
) -> crate::Reply<Value> {
    state.read(|w| {
        if name.trim().is_empty() {
            return Err("Give the map a name".into());
        }
        w.transaction(|db| {
            let mut input = json!({"name":name.trim(),"description":description});
            if let Some(config) = config {
                input["config"] = config;
            }
            update_fields(
                db,
                "map",
                &id,
                &input,
                &["name", "description", "config"],
                true,
            )
        })?;
        get(w, &id)
    })
}
#[tauri::command(async)]
pub fn map_set_image(
    state: tauri::State<crate::Session>,
    id: String,
    media_id: String,
    width: u32,
    height: u32,
) -> crate::Reply<Value> {
    state.read(|w| {
        if width == 0 || height == 0 {
            return Err("Image dimensions must be positive".into());
        }
        w.transaction(|db| {
            update_fields(
                db,
                "map",
                &id,
                &json!({"media_id":media_id,"width":width,"height":height}),
                &["media_id", "width", "height"],
                true,
            )
        })?;
        get(w, &id)
    })
}
#[tauri::command(async)]
pub fn map_delete(state: tauri::State<crate::Session>, id: String) -> crate::Reply<()> {
    state.read(|w| crate::trash::delete(w, "map", "map", &id))
}
#[tauri::command(async)]
pub fn map_layer_create(
    state: tauri::State<crate::Session>,
    map_id: String,
    name: String,
) -> crate::Reply<Value> {
    state.read(|w| w.transaction(|db| layer_in(db, &map_id, &name)))
}
#[tauri::command(async)]
pub fn map_layer_update(
    state: tauri::State<crate::Session>,
    id: String,
    name: String,
    visible: bool,
    locked: bool,
    opacity: f64,
    color: String,
) -> crate::Reply<Value> {
    state.read(|w|w.transaction(|db| {
        if name.trim().is_empty() || !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) { return Err("Invalid layer settings".into()); }
        update_fields(db,"map_layer",&id,&json!({"name":name.trim(),"visible":visible,"locked":locked,"opacity":opacity,"color":color}),&["name","visible","locked","opacity","color"],false)?;
        one(db,"SELECT * FROM map_layer WHERE id=?1",&id)
    }))
}

macro_rules! feature_commands {
    ($create:ident,$update:ident,$delete:ident,$table:literal,$fields:ident) => {
        #[tauri::command(async)]
        pub fn $create(
            state: tauri::State<crate::Session>,
            map_id: String,
            input: Value,
        ) -> crate::Reply<Value> {
            state.read(|w| feature_create(w, $table, &map_id, &input))
        }
        #[tauri::command(async)]
        pub fn $update(
            state: tauri::State<crate::Session>,
            id: String,
            input: Value,
        ) -> crate::Reply<Value> {
            state.read(|w| feature_update(w, $table, &id, &input, $fields))
        }
        #[tauri::command(async)]
        pub fn $delete(state: tauri::State<crate::Session>, id: String) -> crate::Reply<()> {
            state.read(|w| {
                w.transaction(|db| {
                    db.execute(concat!("DELETE FROM ", $table, " WHERE id=?1"), [id])?;
                    Ok(())
                })
            })
        }
    };
}
feature_commands!(
    map_marker_create,
    map_marker_update,
    map_marker_delete,
    "map_marker",
    MARKER
);
feature_commands!(
    map_region_create,
    map_region_update,
    map_region_delete,
    "map_region",
    REGION
);
feature_commands!(
    map_stamp_create,
    map_stamp_update,
    map_stamp_delete,
    "map_stamp",
    STAMP
);

#[tauri::command(async)]
pub fn map_layer_delete(state: tauri::State<crate::Session>, id: String) -> crate::Reply<()> {
    state.read(|w| {
        w.transaction(|db| {
            db.execute("DELETE FROM map_layer WHERE id=?1", [id])?;
            Ok(())
        })
    })
}
#[tauri::command(async)]
pub fn map_marker_move(
    state: tauri::State<crate::Session>,
    id: String,
    x: f64,
    y: f64,
) -> crate::Reply<Value> {
    state.read(|w| feature_update(w, "map_marker", &id, &json!({"x":x,"y":y}), MARKER))
}
#[tauri::command(async)]
pub fn map_stamp_image_put(
    state: tauri::State<crate::Session>,
    request: tauri::ipc::Request<'_>,
) -> crate::Reply<String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("Expected PNG bytes".into());
    };
    state.read(|w| stamp_image_put(w, bytes))
}
#[tauri::command(async)]
pub fn map_stamp_image_get(
    state: tauri::State<crate::Session>,
    name: String,
) -> crate::Reply<tauri::ipc::Response> {
    state.read(|w| {
        Ok(tauri::ipc::Response::new(std::fs::read(
            w.local_file(&stamp_path(&name)?)?,
        )?))
    })
}
#[tauri::command(async)]
pub fn map_feature_dates(
    state: tauri::State<crate::Session>,
    kind: String,
    id: String,
    from: Option<Value>,
    until: Option<Value>,
) -> crate::Reply<()> {
    state.read(|w| {
        w.transaction(|db| {
            let table = match kind.as_str() {
                "marker" => "map_marker",
                "region" => "map_region",
                "stamp" => "map_stamp",
                _ => return Err("Unknown map feature".into()),
            };
            update_fields(
                db,
                table,
                &id,
                &json!({"valid_from":from,"valid_until":until}),
                &["valid_from", "valid_until"],
                true,
            )
        })
    })
}
#[tauri::command(async)]
pub fn map_outline_add(
    state: tauri::State<crate::Session>,
    region_id: String,
    date: Value,
    points: Value,
) -> crate::Reply<Value> {
    state.read(|w|w.transaction(|db| {
        validate_points(&points)?;
        let id=new_id();
        db.execute("INSERT INTO map_region_outline(id,region_id,date,points,created_at) VALUES(?1,?2,?3,?4,?5)",params![id,region_id,date.to_string(),points.to_string(),now()])?;
        one(db,"SELECT * FROM map_region_outline WHERE id=?1",&id)
    }))
}
#[tauri::command(async)]
pub fn map_outline_update(
    state: tauri::State<crate::Session>,
    id: String,
    date: Value,
    points: Value,
    color: Option<String>,
) -> crate::Reply<Value> {
    state.read(|w| {
        w.transaction(|db| {
            validate_points(&points)?;
            let mut input = json!({"date":date,"points":points});
            if let Some(color) = color {
                input["color"] = json!(color);
            }
            update_fields(
                db,
                "map_region_outline",
                &id,
                &input,
                &["date", "points", "color"],
                false,
            )?;
            one(db, "SELECT * FROM map_region_outline WHERE id=?1", &id)
        })
    })
}
#[tauri::command(async)]
pub fn map_outline_delete(state: tauri::State<crate::Session>, id: String) -> crate::Reply<()> {
    state.read(|w| {
        w.transaction(|db| {
            db.execute("DELETE FROM map_region_outline WHERE id=?1", [id])?;
            Ok(())
        })
    })
}
#[tauri::command(async)]
pub fn map_placements(
    state: tauri::State<crate::Session>,
    entity_id: String,
) -> crate::Reply<Vec<Value>> {
    state.read(|w|rows(&w.db,"SELECT p.id,'marker' AS kind,p.map_id,m.name AS map_name,p.label,p.x,p.y FROM map_marker p JOIN map m ON m.id=p.map_id WHERE p.entity_id=?1 UNION ALL SELECT r.id,r.kind,r.map_id,m.name AS map_name,r.label,NULL,NULL FROM map_region r JOIN map m ON m.id=r.map_id WHERE r.entity_id=?1 ORDER BY map_name,label",[entity_id]))
}
