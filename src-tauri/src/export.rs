use crate::{
    database::{new_id, now, one},
    world::{rows, write_atomic, Result, World},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use rusqlite::params;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
};

pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
pub fn filename(name: &str) -> String {
    let name: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .take(100)
        .collect();
    let trimmed = name.trim_matches([' ', '.']);
    let name = if trimmed.is_empty() {
        "Untitled"
    } else {
        trimmed
    };
    if [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ]
    .contains(&name.split('.').next().unwrap_or("").to_uppercase().as_str())
    {
        format!("_{name}")
    } else {
        name.to_owned()
    }
}
fn set(value: &Value) -> HashSet<String> {
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
fn selected(world: &World, request: &Value) -> Result<Vec<Value>> {
    let omitted = &request["leave_out"];
    let ids = set(&omitted["entities"]);
    let types = set(&omitted["types"]);
    let tags = set(&omitted["tags"]);
    let mut result = Vec::new();
    for entry in world.entities(&json!({"limit":i64::MAX}))? {
        if ids.contains(entry["id"].as_str().unwrap_or(""))
            || types.contains(entry["type_id"].as_str().unwrap_or(""))
            || entry["tags"].as_array().is_some_and(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .any(|tag| tags.contains(tag))
            })
        {
            continue;
        }
        result.push(crate::entities::get(
            world,
            entry["id"].as_str().ok_or("Missing entity")?,
        )?);
    }
    Ok(result)
}
pub fn bundle(world: &World) -> Result<Value> {
    let mut types = rows(
        &world.db,
        "SELECT * FROM entity_type ORDER BY sort_order,name",
        [],
    )?;
    for kind in &mut types {
        kind["properties"] = json!(rows(
            &world.db,
            "SELECT * FROM property_definition WHERE type_id=?1 ORDER BY sort_order,id",
            [kind["id"].as_str()]
        )?);
    }
    let mut media = rows(&world.db, "SELECT * FROM media ORDER BY id", [])?;
    for item in &mut media {
        item["data"] = json!(STANDARD.encode(fs::read(
            world.local_file(item["path"].as_str().ok_or("Missing media path")?)?
        )?));
    }
    let manifest: Value = serde_json::from_slice(&fs::read(world.root.join("world.json"))?)?;
    Ok(
        json!({"format":"local-realm","version":1,"world":manifest,"types":types,"entities":selected(world,&json!({}))?,"relationships":rows(&world.db,"SELECT * FROM relationship ORDER BY id",[])?,"media":media}),
    )
}

fn rewrite(value: &mut Value, ids: &HashMap<String, String>) {
    match value {
        Value::String(text) => {
            if let Some(replacement) = ids.get(text) {
                *text = replacement.clone();
            } else {
                for (from, to) in ids {
                    if from != to {
                        *text =
                            text.replace(&format!("entity://{from}"), &format!("entity://{to}"));
                    }
                }
            }
        }
        Value::Array(array) => {
            for item in array {
                rewrite(item, ids);
            }
        }
        Value::Object(object) => {
            for item in object.values_mut() {
                rewrite(item, ids);
            }
        }
        _ => {}
    }
}
pub fn import_bundle(world: &World, bundle: &Value) -> Result<Value> {
    let types = bundle["types"]
        .as_array()
        .ok_or("Bundle is missing categories")?;
    let entities = bundle["entities"]
        .as_array()
        .ok_or("Bundle is missing entries")?;
    if entities.len() > 100_000 || types.len() > 10_000 {
        return Err("Bundle is too large".into());
    }
    world.transaction(|db| {
        let mut ids=HashMap::new();let mut new_types=0usize;let mut count=0usize;let mut skipped=Vec::new();
        let mut created_types = HashSet::new();
        for category in types {
            let old=category["id"].as_str().ok_or("Category without id")?;
            let name=crate::database::required_text(category,"name")?;
            let existing=rows(db,"SELECT id FROM entity_type WHERE name=?1 COLLATE NOCASE",[name])?.pop();
            let id=existing.and_then(|v|v["id"].as_str().map(str::to_owned)).unwrap_or_else(new_id);
            let added=db.execute("INSERT OR IGNORE INTO entity_type(id,name,plural,icon,color,description,sort_order,created_at,updated_at,layout) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?8,?9)",params![id,name,category["plural"].as_str().unwrap_or(name),category["icon"].as_str().unwrap_or("file"),category["color"].as_str().unwrap_or(""),category["description"].as_str().unwrap_or(""),category["sort_order"].as_i64().unwrap_or(0),now(),category.get("layout").cloned().unwrap_or_else(||json!({})).to_string()])?;
            if added != 0 { created_types.insert(id.clone()); }
            new_types+=added;ids.insert(old.into(),id);
        }
        for category in types {
            let id=ids[category["id"].as_str().unwrap()].clone();
            // Existing category ancestry is retained. New categories can point to any imported parent.
            if let Some(parent)=category["parent_id"].as_str().and_then(|p|ids.get(p)) {
                if created_types.contains(&id) {
                    crate::schema::check_parent(db, &id, Some(parent))?;
                    db.execute("UPDATE entity_type SET parent_id=?1 WHERE id=?2",params![parent,id])?;
                }
            }
            if let Some(properties)=category["properties"].as_array() {for property in properties {
                let key=crate::database::required_text(property,"key")?;
                let data_type=crate::database::required_text(property,"data_type")?;
                if !crate::schema::DATA_TYPES.contains(&data_type){return Err("Unknown property type in bundle".into());}
                let mut config=property.get("config").cloned().unwrap_or_else(||json!({}));rewrite(&mut config,&ids);
                let property_id=rows(db,"SELECT id FROM property_definition WHERE type_id=?1 AND key=?2",params![id,key])?.pop().and_then(|p|p["id"].as_str().map(str::to_owned)).unwrap_or_else(new_id);
                db.execute("INSERT OR IGNORE INTO property_definition(id,type_id,key,label,data_type,config,hint,required,sort_order,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10)",params![property_id,id,key,property["label"].as_str().unwrap_or(key),data_type,config.to_string(),property["hint"].as_str().unwrap_or(""),property["required"].as_bool().unwrap_or(false),property["sort_order"].as_i64().unwrap_or(0),now()])?;
                if let Some(old)=property["id"].as_str(){ids.insert(old.into(),property_id);}
            }}
        }
        let mut imported=Vec::new();
        for entity in entities {
            let old=entity["id"].as_str().ok_or("Entry without id")?;
            let category=ids.get(entity["type_id"].as_str().ok_or("Entry without category")?).ok_or("Missing imported category")?;
            let name=crate::database::required_text(entity,"name")?;
            if let Some(existing)=rows(db,"SELECT id FROM entity WHERE id=?1 OR (type_id=?2 AND name=?3 COLLATE NOCASE) ORDER BY id=?1 DESC LIMIT 1",params![old,category,name])?.pop() {
                ids.insert(old.into(),existing["id"].as_str().ok_or("Missing entry id")?.into());skipped.push(name.to_owned());continue;
            }
            let id=crate::entities::create_in(db,category,name,entity["summary"].as_str().unwrap_or(""))?;
            ids.insert(old.into(),id.clone());imported.push((id,entity));count+=1;
        }
        if let Some(media)=bundle["media"].as_array(){for item in media {
            let Some(data)=item["data"].as_str()else{continue;};
            if data.len()>256*1024*1024{return Err("Embedded media is too large".into());}
            let bytes=STANDARD.decode(data)?;
            let old=item["id"].as_str().ok_or("Media without id")?;
            let digest = format!("{:x}", Sha256::digest(&bytes));
            if let Some(existing) = rows(db, "SELECT id,path FROM media WHERE sha256=?1 LIMIT 1", [&digest])?.pop() {
                ids.insert(old.into(), existing["id"].as_str().ok_or("Missing media id")?.into());
                if let Some(old_path) = item["path"].as_str() {
                    ids.insert(old_path.into(), existing["path"].as_str().ok_or("Missing media path")?.into());
                }
                continue;
            }
            let id=new_id();let path=format!("media/documents/{id}-{}",filename(item["filename"].as_str().unwrap_or("file")));
            db.execute("INSERT INTO media(id,filename,path,mime,kind,bytes,sha256,caption,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![id,item["filename"].as_str().unwrap_or("file"),path,item["mime"].as_str().unwrap_or("application/octet-stream"),item["kind"].as_str().unwrap_or("other"),bytes.len(),digest,item["caption"].as_str().unwrap_or(""),now()])?;
            World::queue_file(db,&path,&bytes)?;ids.insert(old.into(),id);
            if let Some(old_path)=item["path"].as_str(){ids.insert(old_path.into(),path);}
        }}
        for (id,entity) in imported {
            let mut patch=json!({"tags":entity.get("tags").cloned().unwrap_or_else(||json!([])),"aliases":entity.get("aliases").cloned().unwrap_or_else(||json!([])),"image_media_id":entity["image_media_id"],"image_frame":entity["image_frame"],"properties":{}});
            if let Some(properties)=entity["properties"].as_array(){for p in properties {if p["data_type"]!="section" {if let Some(key)=p["key"].as_str(){patch["properties"][key]=p["value"].clone();}}}}
            else if entity["properties"].is_object(){patch["properties"]=entity["properties"].clone();}
            rewrite(&mut patch,&ids);
            if patch["image_media_id"].as_str().is_some_and(|id|!ids.values().any(|v|v==id)){patch["image_media_id"]=Value::Null;}
            crate::entities::update_in(world,db,&id,&patch)?;
            if let Some(attachments) = entity["media"].as_array() {
                for (index, attachment) in attachments.iter().enumerate() {
                    if let Some(media_id) = attachment["id"].as_str().and_then(|old| ids.get(old)) {
                        db.execute("INSERT OR IGNORE INTO entity_media(entity_id,media_id,role,sort_order) VALUES(?1,?2,?3,?4)", params![id,media_id,attachment["role"].as_str().unwrap_or("attachment"),index as i64])?;
                    }
                }
            }
            let document=one(db,"SELECT id FROM document WHERE entity_id=?1",&id)?;
            let mut body=entity.get("body").cloned().unwrap_or_else(||json!(""));rewrite(&mut body,&ids);
            let mut body=body.as_str().unwrap_or("").to_owned();
            for (old,new) in &ids {if old.starts_with("media/"){body=body.replace(old,new);}}
            crate::documents::save_in(world,db,document["id"].as_str().ok_or("Missing document id")?,&body)?;
        }
        if let Some(relationships)=bundle["relationships"].as_array(){for relation in relationships {
            let source=relation["source_id"].as_str().and_then(|s|ids.get(s));let target=relation["target_id"].as_str().and_then(|s|ids.get(s));
            if let (Some(source),Some(target))=(source,target) {
                let kind=relation["kind"].as_str().unwrap_or("RELATED_TO");
                db.execute("INSERT INTO relationship(id,source_id,target_id,kind,inverse_kind,symmetrical,note,start_date,end_date,created_at) SELECT ?1,?2,?3,?4,?5,?6,?7,?8,?9,?10 WHERE NOT EXISTS(SELECT 1 FROM relationship WHERE source_id=?2 AND target_id=?3 AND kind=?4)",params![new_id(),source,target,kind,relation["inverse_kind"].as_str().unwrap_or(""),relation["symmetrical"].as_bool().unwrap_or(false),relation["note"].as_str().unwrap_or(""),if relation["start_date"].is_null(){String::new()}else{relation["start_date"].to_string()},if relation["end_date"].is_null(){String::new()}else{relation["end_date"].to_string()},now()])?;
            }
        }}
        Ok(json!({"entities":count,"types":new_types,"skipped":skipped}))
    })
}

pub fn html_body(world: &World, body: &str, max_image: u32) -> Result<String> {
    let mut html = crate::documents::preview(body);
    html = html.replace("entity://", "#entity-");
    let regex = regex::Regex::new(r#"src="([^"]+)""#)?;
    let mut replacements = Vec::new();
    for capture in regex.captures_iter(&html) {
        let path = &capture[1];
        if path.starts_with("media/") {
            let full = world.local_file(path)?;
            let mut bytes = fs::read(&full)?;
            let mut mime = mime_guess::from_path(&full)
                .first_or_octet_stream()
                .to_string();
            if max_image > 0 && mime.starts_with("image/") {
                if let Ok(image) = image::load_from_memory(&bytes) {
                    if image.width() > max_image || image.height() > max_image {
                        let reduced = image.thumbnail(max_image, max_image);
                        let mut output = std::io::Cursor::new(Vec::new());
                        reduced.write_to(&mut output, image::ImageFormat::Png)?;
                        bytes = output.into_inner();
                        mime = "image/png".into();
                    }
                }
            }
            replacements.push((
                capture[0].to_owned(),
                format!("src=\"data:{mime};base64,{}\"", STANDARD.encode(bytes)),
            ));
        }
    }
    for (from, to) in replacements {
        html = html.replace(&from, &to);
    }
    Ok(html)
}
fn scalar(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(scalar).collect::<Vec<_>>().join(", "),
        _ => value.to_string(),
    }
}
fn csv_cell(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
fn html_document(name: &str, content: &str, navigation: &str) -> String {
    format!("<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><style>{}</style><body><aside><h1>{}</h1><label>Find an entry<input id=\"find\" type=\"search\"></label><nav>{navigation}</nav></aside><main>{content}</main><script>document.querySelector('#find').addEventListener('input',e=>{{const q=e.target.value.toLocaleLowerCase();document.querySelectorAll('nav a').forEach(a=>a.hidden=!a.textContent.toLocaleLowerCase().includes(q));}});</script></body></html>",escape(name),include_str!("../resources/wiki.css"),escape(name))
}
pub fn export(world: &World, request: &Value) -> Result<Value> {
    let format = crate::database::required_text(request, "format")?;
    let destination = Path::new(crate::database::required_text(request, "destination")?);
    let entities = selected(world, request)?;
    if format == "zip" {
        crate::archives::snapshot(world, destination)?;
    } else if format == "json" {
        write_atomic(destination, &serde_json::to_vec_pretty(&bundle(world)?)?)?;
    } else if ["html", "site", "markdown", "csv"].contains(&format) {
        let name = world.info()?["name"].as_str().unwrap_or("Realm").to_owned();
        let max_image = request["max_image"].as_u64().unwrap_or(0).min(8192) as u32;
        let mut content = String::new();
        let mut navigation = String::new();
        if format != "html" {
            fs::create_dir_all(destination)?;
        }
        let mut markdown_index = format!("# {name}\n\n");
        let entry_files: HashMap<String, String> = entities
            .iter()
            .filter_map(|e| {
                let id = e["id"].as_str()?;
                Some((
                    id.into(),
                    format!(
                        "{}-{id}.md",
                        filename(e["name"].as_str().unwrap_or("Untitled"))
                    ),
                ))
            })
            .collect();
        for entry in &entities {
            let id = entry["id"].as_str().ok_or("Missing entity id")?;
            let title = entry["name"].as_str().unwrap_or("Untitled");
            let body = entry["body"].as_str().unwrap_or("");
            let mut properties = String::new();
            if let Some(fields) = entry["properties"].as_array() {
                for p in fields {
                    if !p["value"].is_null() && p["data_type"] != "section" {
                        properties.push_str(&format!(
                            "<dt>{}</dt><dd>{}</dd>",
                            escape(p["label"].as_str().unwrap_or("")),
                            escape(&scalar(&p["value"]))
                        ));
                    }
                }
            }
            content.push_str(&format!("<article id=\"entity-{}\"><p class=\"category\">{}</p><h2>{}</h2><p>{}</p><dl>{properties}</dl>{}</article>",escape(id),escape(entry["type_name"].as_str().unwrap_or("")),escape(title),escape(entry["summary"].as_str().unwrap_or("")),html_body(world,body,max_image)?));
            navigation.push_str(&format!(
                "<a href=\"#entity-{}\">{}</a>",
                escape(id),
                escape(title)
            ));
            if format == "markdown" {
                let filename = &entry_files[id];
                let mut body = body.to_owned();
                for (target, file) in &entry_files {
                    let encoded = percent_encoding::utf8_percent_encode(
                        file,
                        percent_encoding::NON_ALPHANUMERIC,
                    );
                    body = body.replace(&format!("entity://{target}"), &encoded.to_string());
                }
                // Copy only referenced media, retaining relative paths used by Markdown.
                for media in rows(&world.db, "SELECT path FROM media", [])? {
                    if let Some(path) = media["path"].as_str().filter(|p| body.contains(p)) {
                        let source = world.local_file(path)?;
                        write_atomic(&destination.join(path), &fs::read(source)?)?;
                    }
                }
                let markdown = format!(
                    "# {title}\n\n{}\n\n{body}\n",
                    entry["summary"].as_str().unwrap_or("")
                );
                write_atomic(&destination.join(filename), markdown.as_bytes())?;
                markdown_index.push_str(&format!("- [{title}]({filename})\n"));
            }
        }
        if format == "html" || format == "site" {
            let allowed: HashSet<String> = entry_files.keys().cloned().collect();
            let (views, view_navigation) = crate::share_views::html(world, request, &allowed)?;
            content.push_str(&views);
            navigation.push_str(&view_navigation);
            for key in request["include"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                if let Some(id) = key.strip_prefix("note:") {
                    let doc = world.document(id)?;
                    content.push_str(&format!(
                        "<article><h2>{}</h2>{}</article>",
                        escape(doc["title"].as_str().unwrap_or("")),
                        html_body(world, doc["body"].as_str().unwrap_or(""), max_image)?
                    ));
                } else if let Some(id) = key.strip_prefix("manuscript:") {
                    let book = crate::manuscripts::get(world, id)?;
                    content.push_str(&format!(
                        "<article><h2>{}</h2>",
                        escape(book["name"].as_str().unwrap_or(""))
                    ));
                    for chapter in book["chapters"].as_array().into_iter().flatten() {
                        let doc = world
                            .document(chapter["document_id"].as_str().ok_or("Missing chapter")?)?;
                        content.push_str(&format!(
                            "<h3>{}</h3>{}",
                            escape(doc["title"].as_str().unwrap_or("")),
                            html_body(world, doc["body"].as_str().unwrap_or(""), max_image)?
                        ));
                    }
                    content.push_str("</article>");
                }
            }
            let target = if format == "site" {
                destination.join("index.html")
            } else {
                destination.to_owned()
            };
            write_atomic(
                &target,
                html_document(&name, &content, &navigation).as_bytes(),
            )?;
        }
        if format == "markdown" {
            write_atomic(&destination.join("README.md"), markdown_index.as_bytes())?;
        }
        if format == "csv" {
            let mut groups: HashMap<String, Vec<&Value>> = HashMap::new();
            for entity in &entities {
                groups
                    .entry(entity["type_id"].as_str().ok_or("Missing category")?.into())
                    .or_default()
                    .push(entity);
            }
            for (kind, entries) in groups {
                let properties = crate::schema::properties(&world.db, &kind)?;
                let fields: Vec<_> = properties
                    .iter()
                    .filter(|p| p["data_type"] != "section")
                    .collect();
                let mut header = vec![
                    "Name".to_owned(),
                    "Summary".to_owned(),
                    "Tags".to_owned(),
                    "Article".to_owned(),
                ];
                header.extend(
                    fields
                        .iter()
                        .map(|p| p["label"].as_str().unwrap_or("").to_owned()),
                );
                let mut csv = String::from("\u{feff}");
                csv.push_str(
                    &header
                        .iter()
                        .map(|s| csv_cell(s))
                        .collect::<Vec<_>>()
                        .join(","),
                );
                csv.push_str("\r\n");
                for entity in &entries {
                    let mut values = vec![
                        scalar(&entity["name"]),
                        scalar(&entity["summary"]),
                        scalar(&entity["tags"]),
                        scalar(&entity["body"]),
                    ];
                    for field in &fields {
                        let value = entity["properties"]
                            .as_array()
                            .and_then(|a| a.iter().find(|p| p["id"] == field["id"]))
                            .map(|p| &p["value"])
                            .unwrap_or(&Value::Null);
                        values.push(scalar(value));
                    }
                    csv.push_str(
                        &values
                            .iter()
                            .map(|s| csv_cell(s))
                            .collect::<Vec<_>>()
                            .join(","),
                    );
                    csv.push_str("\r\n");
                }
                write_atomic(
                    &destination.join(format!(
                        "{}-{kind}.csv",
                        filename(entries[0]["type_name"].as_str().unwrap_or("Entries"))
                    )),
                    csv.as_bytes(),
                )?;
            }
        }
    } else {
        return Err("Unknown export format".into());
    }
    Ok(json!({"path":destination.to_string_lossy(),"entities":entities.len(),"format":format}))
}
#[tauri::command(async)]
pub fn export_world(state: tauri::State<crate::Session>, request: Value) -> crate::Reply<Value> {
    state.read(|w| export(w, &request))
}
#[tauri::command(async)]
pub fn import_world(state: tauri::State<crate::Session>, path: String) -> crate::Reply<Value> {
    state.read(|w| import_bundle(w, &serde_json::from_slice(&fs::read(path)?)?))
}
