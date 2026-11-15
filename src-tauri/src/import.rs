//! Imports are scanned without changing the realm, then applied in one database transaction.
use crate::{
    database::{new_id, now, one},
    world::{rows, Result, World},
};
use rusqlite::params;
use serde_json::{json, Map, Value};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
};

struct Note {
    name: String,
    body: String,
    group: String,
    fields: Map<String, Value>,
    tags: Vec<String>,
    source: PathBuf,
}
struct Sources {
    notes: Vec<Note>,
    skipped: Vec<String>,
    _temporary: Vec<tempfile::TempDir>,
}

fn decode_link(value: &str) -> String {
    percent_encoding::percent_decode_str(value)
        .decode_utf8_lossy()
        .split('#')
        .next()
        .unwrap_or("")
        .replace('\\', "/")
}
fn title_from_path(path: &Path) -> String {
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled");
    // Notion appends an export UUID to page names.
    if let Some((title, tail)) = name.rsplit_once(' ') {
        if tail.len() == 32 && tail.chars().all(|c| c.is_ascii_hexdigit()) {
            return title.to_owned();
        }
    }
    name.to_owned()
}
pub fn docx_markdown(path: &Path) -> Result<String> {
    let mut archive = zip::ZipArchive::new(fs::File::open(path)?)?;
    let mut xml = String::new();
    archive
        .by_name("word/document.xml")?
        .take(64 * 1024 * 1024)
        .read_to_string(&mut xml)?;
    let mut reader = quick_xml::Reader::from_str(&xml);
    let mut output = String::new();
    let mut paragraph = String::new();
    let mut heading = 0usize;
    let mut text = false;
    loop {
        use quick_xml::events::Event;
        match reader.read_event()? {
            Event::Start(e) => match e.local_name().as_ref() {
                b"p" => {
                    paragraph.clear();
                    heading = 0;
                }
                b"t" => text = true,
                _ => {}
            },
            Event::Empty(e) => match e.local_name().as_ref() {
                b"pStyle" => {
                    for attribute in e.attributes() {
                        let a = attribute?;
                        if a.key.local_name().as_ref() == b"val" {
                            let value = a
                                .decode_and_unescape_value(reader.decoder())?
                                .to_lowercase();
                            if let Some(level) = value
                                .strip_prefix("heading")
                                .and_then(|s| s.parse::<usize>().ok())
                            {
                                heading = level.min(6);
                            }
                        }
                    }
                }
                b"tab" => paragraph.push('\t'),
                b"br" | b"cr" => paragraph.push('\n'),
                _ => {}
            },
            Event::Text(e) if text => {
                paragraph.push_str(&quick_xml::escape::unescape(&e.decode()?)?)
            }
            Event::GeneralRef(e) if text => {
                let reference = e.decode()?;
                paragraph.push_str(&quick_xml::escape::unescape(&format!("&{reference};"))?);
            }
            Event::End(e) => match e.local_name().as_ref() {
                b"t" => text = false,
                b"p" if !paragraph.trim().is_empty() => {
                    if heading > 0 {
                        output.push_str(&"#".repeat(heading));
                        output.push(' ');
                    }
                    output.push_str(&paragraph);
                    output.push_str("\n\n");
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(output)
}
fn parse_note(path: &Path, root: &Path, raw: &str, options: &Value) -> Result<Note> {
    let normalized = raw.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let mut fields = Map::new();
    let mut body = normalized.as_str();
    if let Some(rest) = normalized.strip_prefix("---\n") {
        if let Some((header, remainder)) = rest.split_once("\n---\n") {
            let value: Value = serde_yaml::from_str(header)?;
            if let Some(object) = value.as_object() {
                fields = object.clone();
            }
            body = remainder.trim_start_matches('\n');
        }
    }
    let mut name = fields
        .remove("title")
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| title_from_path(path));
    if let Some(first) = body.lines().next().and_then(|l| l.strip_prefix("# ")) {
        name = first.trim().to_owned();
        body = body
            .split_once('\n')
            .map(|(_, b)| b.trim_start_matches('\n'))
            .unwrap_or("");
    }
    let relative = path.strip_prefix(root).unwrap_or(path);
    let folder = relative
        .parent()
        .and_then(|p| p.components().next())
        .and_then(|c| c.as_os_str().to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("Notes");
    let group = fields
        .remove("type")
        .or_else(|| fields.remove("category"))
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| folder.to_owned());
    let mut tags = match fields.remove("tags") {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(Value::as_str)
            .map(|s| s.trim_start_matches('#').to_owned())
            .collect(),
        Some(Value::String(s)) => s
            .split([',', ' '])
            .filter(|s| !s.is_empty())
            .map(|s| s.trim_start_matches('#').to_owned())
            .collect(),
        _ => Vec::new(),
    };
    if options["folders_as_tags"] == true && folder != "Notes" {
        tags.push(folder.into());
    }
    fields.remove("id");
    Ok(Note {
        name,
        body: body.to_owned(),
        group,
        fields,
        tags,
        source: path.to_owned(),
    })
}
fn split_note(note: Note, level: usize) -> Vec<Note> {
    if level == 0 {
        return vec![note];
    }
    let prefix = format!("{} ", "#".repeat(level.min(6)));
    let mut result = Vec::new();
    let mut title = note.name.clone();
    let mut body = String::new();
    for line in note.body.lines() {
        if let Some(next) = line.strip_prefix(&prefix) {
            if !body.trim().is_empty() {
                result.push(Note {
                    name: title.clone(),
                    body: std::mem::take(&mut body),
                    group: note.group.clone(),
                    fields: note.fields.clone(),
                    tags: note.tags.clone(),
                    source: note.source.clone(),
                });
            }
            title = next.to_owned();
        } else {
            body.push_str(line);
            body.push('\n');
        }
    }
    if !body.trim().is_empty() || result.is_empty() {
        result.push(Note {
            name: title,
            body,
            group: note.group,
            fields: note.fields,
            tags: note.tags,
            source: note.source,
        });
    }
    result
}
fn sources(options: &Value) -> Result<Sources> {
    let mut result = Sources {
        notes: Vec::new(),
        skipped: Vec::new(),
        _temporary: Vec::new(),
    };
    let paths = options["paths"]
        .as_array()
        .filter(|p| !p.is_empty())
        .ok_or("Choose documents to import")?;
    let mut files = Vec::new();
    let mut total = 0u64;
    for selected in paths {
        let mut path =
            PathBuf::from(selected.as_str().ok_or("Invalid source path")?).canonicalize()?;
        if path
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("zip"))
        {
            let temporary = tempfile::tempdir()?;
            crate::archives::extract(&path, temporary.path())?;
            path = temporary.path().to_owned();
            result._temporary.push(temporary);
        }
        if path.is_dir() {
            for entry in walkdir::WalkDir::new(&path)
                .follow_links(false)
                .into_iter()
                .filter_entry(|e| {
                    e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.')
                })
            {
                let entry = entry?;
                if entry.file_type().is_file() {
                    files.push((entry.path().to_owned(), path.clone()));
                }
            }
        } else {
            files.push((
                path.clone(),
                path.parent().ok_or("Missing source folder")?.to_owned(),
            ));
        }
    }
    files.sort();
    files.dedup();
    for (path, root) in files {
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        if !["md", "markdown", "txt", "docx", "csv"].contains(&ext.as_str()) {
            continue;
        }
        let size = fs::metadata(&path)?.len();
        total += size;
        if size > 64 * 1024 * 1024 || total > 512 * 1024 * 1024 {
            return Err("Import exceeds the document size limit".into());
        }
        if ext == "csv" {
            let mut reader = csv::Reader::from_path(&path)?;
            let headers = reader.headers()?.clone();
            for row in reader.records() {
                let row = row?;
                let mut fields = Map::new();
                for (key, value) in headers.iter().zip(row.iter()).skip(1) {
                    if !value.is_empty() {
                        fields.insert(key.into(), json!(value));
                    }
                }
                result.notes.push(Note {
                    name: row.get(0).unwrap_or("Untitled").to_owned(),
                    body: String::new(),
                    group: title_from_path(&path),
                    fields,
                    tags: vec![],
                    source: path.clone(),
                });
            }
        } else {
            let raw = if ext == "docx" {
                docx_markdown(&path)?
            } else {
                fs::read_to_string(&path)?
            };
            let note = parse_note(&path, &root, &raw, options)?;
            result.notes.extend(split_note(
                note,
                options["split"].as_u64().unwrap_or(0) as usize,
            ));
        }
        if result.notes.len() > 100_000 {
            return Err("Import contains too many notes".into());
        }
    }
    if result.notes.is_empty() {
        return Err("No supported documents were found".into());
    }
    Ok(result)
}
fn field_kind(value: &Value) -> &'static str {
    match value {
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_i64() => "integer",
        Value::Number(_) => "decimal",
        Value::Array(_) => "multi_select",
        Value::String(s) if s.starts_with("[[") => "entity_ref",
        Value::String(s) if s.starts_with("https://") || s.starts_with("http://") => "url",
        Value::String(s) if s.contains('\n') => "long_text",
        _ => "short_text",
    }
}
fn wiki_links(body: &str) -> Result<Vec<(String, String)>> {
    let regex = regex::Regex::new(r"!?\[\[([^\]|]+)(?:\|([^\]]+))?\]\]")?;
    Ok(regex
        .captures_iter(body)
        .filter(|c| !c[0].starts_with('!'))
        .map(|c| {
            (
                decode_link(&c[1]),
                c.get(2).map(|s| s.as_str()).unwrap_or(&c[1]).to_owned(),
            )
        })
        .collect())
}
pub fn scan(world: &World, options: &Value) -> Result<Value> {
    let source = sources(options)?;
    let entries = world.entities(&json!({"limit":i64::MAX}))?;
    let existing: HashSet<_> = entries
        .iter()
        .filter_map(|e| e["name"].as_str())
        .map(str::to_lowercase)
        .collect();
    let incoming: HashSet<_> = source.notes.iter().map(|n| n.name.to_lowercase()).collect();
    let mut groups: BTreeMap<String, Vec<&Note>> = BTreeMap::new();
    let mut duplicate = Vec::new();
    let mut missing = HashSet::new();
    let mut links = 0;
    let mut pictures = 0;
    for note in &source.notes {
        groups.entry(note.group.clone()).or_default().push(note);
        if existing.contains(&note.name.to_lowercase()) {
            duplicate.push(note.name.clone());
        }
        pictures += note.body.matches("![").count();
        for (link, _) in wiki_links(&note.body)? {
            links += 1;
            let name = title_from_path(Path::new(&link));
            if !existing.contains(&name.to_lowercase()) && !incoming.contains(&name.to_lowercase())
            {
                missing.insert(name);
            }
        }
    }
    let mut described = Vec::new();
    for (group, notes) in groups {
        let category=rows(&world.db,"SELECT id FROM entity_type WHERE name=?1 COLLATE NOCASE OR plural=?1 COLLATE NOCASE ORDER BY name=?1 DESC LIMIT 1",[&group])?.pop();
        let mut fields: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        for note in &notes {
            for (key, value) in &note.fields {
                fields.entry(key.clone()).or_default().push(value.clone());
            }
        }
        let existing_fields = if let Some(id) = category.as_ref().and_then(|c| c["id"].as_str()) {
            crate::schema::properties(&world.db, id)?
        } else {
            vec![]
        };
        let fields:Vec<Value>=fields.into_iter().map(|(label,values)|json!({"label":label,"kind":field_kind(&values[0]),"samples":values.iter().take(3).map(|v|v.as_str().map(str::to_owned).unwrap_or_else(||v.to_string())).collect::<Vec<_>>(),"existing":existing_fields.iter().any(|p|p["label"]==label)})).collect();
        described.push(json!({"key":group,"label":group,"new_type":group,"type_id":category.map(|c|c["id"].clone()),"notes":notes.len(),"fields":fields}));
    }
    let mut missing: Vec<_> = missing.into_iter().collect();
    missing.sort();
    duplicate.sort();
    duplicate.dedup();
    Ok(
        json!({"groups":described,"notes":source.notes.len(),"pictures":pictures,"links":links,"missing":missing,"existing":duplicate,"skipped":source.skipped}),
    )
}
fn slug(label: &str) -> String {
    let value: String = label
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let value = value.trim_matches('_');
    if value.is_empty() {
        "field".into()
    } else {
        value.into()
    }
}
fn coerce(value: &Value, kind: &str, lookup: &HashMap<String, String>) -> Value {
    let string = value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string());
    match kind {
        "integer" => string
            .parse::<i64>()
            .map(Value::from)
            .unwrap_or(Value::Null),
        "decimal" | "rating" => string
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .map(Value::from)
            .unwrap_or(Value::Null),
        "boolean" => {
            json!(value == true || ["true", "yes", "1"].contains(&string.to_lowercase().as_str()))
        }
        "multi_select" => {
            if value.is_array() {
                value.clone()
            } else {
                json!(string
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>())
            }
        }
        "entity_ref" => lookup
            .get(&string.trim_matches(['[', ']']).to_lowercase())
            .map(|id| json!(id))
            .unwrap_or(Value::Null),
        "entity_ref_multi" => json!(string
            .split(',')
            .filter_map(|s| lookup.get(&s.trim().trim_matches(['[', ']']).to_lowercase()))
            .collect::<Vec<_>>()),
        _ => json!(string),
    }
}
pub fn run(world: &World, options: &Value, choices: &Value) -> Result<Value> {
    let source = sources(options)?;
    let snapshot = format!(
        "backups/{}-before-import.zip",
        chrono::Utc::now().format("%Y%m%d-%H%M%S-%3f")
    );
    crate::archives::snapshot(world, &world.output_file(&snapshot)?)?;
    let result=world.transaction(|db| {
        let mut lookup:HashMap<String,String>=rows(db,"SELECT id,name FROM entity ORDER BY created_at",[])?.iter().filter_map(|e|Some((e["name"].as_str()?.to_lowercase(),e["id"].as_str()?.to_owned()))).collect();
        let mut groups=HashMap::new();let mut field_maps:HashMap<String,Vec<(String,String,String)>>=HashMap::new();
        let mut counts=json!({"created":0,"merged":0,"kept":0,"notes":0,"chapters":0,"types":0,"fields":0,"pictures":0,"links":0,"unlinked":0,"stubs":0,"manuscript_id":null,"snapshot":snapshot});
        let bump=|counts:&mut Value,key:&str|{counts[key]=json!(counts[key].as_u64().unwrap_or(0)+1);};
        let target=options["target"].as_str().unwrap_or("entries");
        if target=="manuscript" {
            let id=new_id();let title=source.notes.first().map(|n|title_from_path(&n.source)).unwrap_or_else(||"Imported manuscript".into());
            db.execute("INSERT INTO manuscript(id,name,created_at,updated_at) VALUES(?1,?2,?3,?3)",params![id,title,now()])?;counts["manuscript_id"]=json!(id);
        }
        if target=="entries" {
            for group in choices["groups"].as_array().ok_or("Choose where imported groups belong")? {
                let key=group["key"].as_str().ok_or("Missing import group")?;
                if group["skip"]==true{continue;}
                let id=if let Some(id)=group["type_id"].as_str().filter(|s|!s.is_empty()){one(db,"SELECT id FROM entity_type WHERE id=?1",id)?;id.to_owned()}else{
                    let name=group["new_type"].as_str().filter(|s|!s.trim().is_empty()).unwrap_or(key);
                    if let Some(category)=rows(db,"SELECT id FROM entity_type WHERE name=?1 COLLATE NOCASE",[name])?.pop(){category["id"].as_str().ok_or("Missing category")?.to_owned()}else{
                        let id=new_id();db.execute("INSERT INTO entity_type(id,name,plural,icon,created_at,updated_at) VALUES(?1,?2,?2,'file',?3,?3)",params![id,name,now()])?;bump(&mut counts,"types");id
                    }
                };
                let mut mapping=Vec::new();
                for field in group["fields"].as_array().into_iter().flatten() {
                    let label=field["label"].as_str().ok_or("Missing field label")?;let kind=field["kind"].as_str().unwrap_or("short_text");
                    if kind=="skip"{continue;}
                    if !crate::schema::DATA_TYPES.contains(&kind){return Err("Unsupported import field type".into());}
                    let definitions=crate::schema::properties(db,&id)?;
                    let property=definitions.iter().find(|p|p["label"].as_str().is_some_and(|s|s.eq_ignore_ascii_case(label)));
                    let (key,actual_kind)=if let Some(p)=property{(p["key"].as_str().ok_or("Missing field key")?.to_owned(),p["data_type"].as_str().unwrap_or(kind).to_owned())}else{
                        let base=slug(label);let mut key=base.clone();let mut index=2;while definitions.iter().any(|p|p["key"]==key){key=format!("{base}_{index}");index+=1;}
                        db.execute("INSERT INTO property_definition(id,type_id,key,label,data_type,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?6)",params![new_id(),id,key,label,kind,now()])?;bump(&mut counts,"fields");(key,kind.into())
                    };
                    mapping.push((label.to_owned(),key,actual_kind));
                }
                groups.insert(key.to_owned(),id);field_maps.insert(key.to_owned(),mapping);
            }
        }
        let mut work=Vec::new();
        for note in &source.notes {
            if target=="entries" {
                let Some(category)=groups.get(&note.group)else{continue;};
                if let Some(existing)=lookup.get(&note.name.to_lowercase()) {
                    if choices["existing"]!="merge"{bump(&mut counts,"kept");continue;}
                    let document=one(db,"SELECT id FROM document WHERE entity_id=?1",existing)?;
                    work.push((note,Some(existing.clone()),document["id"].as_str().ok_or("Missing document")?.to_owned(),true));bump(&mut counts,"merged");
                }else{
                    let id=crate::entities::create_in(db,category,&note.name,"")?;lookup.insert(note.name.to_lowercase(),id.clone());
                    let document=one(db,"SELECT id FROM document WHERE entity_id=?1",&id)?;
                    work.push((note,Some(id),document["id"].as_str().ok_or("Missing document")?.to_owned(),false));bump(&mut counts,"created");
                }
            }else if target=="manuscript" {
                let chapter=crate::manuscripts::chapter_create_in(db,counts["manuscript_id"].as_str().ok_or("Missing manuscript")?,None,&note.name)?;
                let document=one(db,"SELECT document_id FROM chapter WHERE id=?1",&chapter)?;
                work.push((note,None,document["document_id"].as_str().ok_or("Missing document")?.to_owned(),false));bump(&mut counts,"chapters");
            }else if target=="notes" {
                let document=crate::documents::create_in(db,"note",&note.name,None)?;work.push((note,None,document["id"].as_str().ok_or("Missing note")?.to_owned(),false));bump(&mut counts,"notes");
            }else{return Err("Unknown import destination".into());}
        }
        if choices["missing"]=="stub" || choices["missing"]=="stubs" {
            let category=choices["stub_type_id"].as_str().ok_or("Choose a category for missing entries")?;
            for (note,_,_,_) in &work {for (link,_) in wiki_links(&note.body)? {let name=title_from_path(Path::new(&link));if let std::collections::hash_map::Entry::Vacant(entry)=lookup.entry(name.to_lowercase()){let id=crate::entities::create_in(db,category,&name,"")?;entry.insert(id);bump(&mut counts,"stubs");}}}
        }
        let wiki=regex::Regex::new(r"(!?)\[\[([^\]|]+)(?:\|([^\]]+))?\]\]")?;
        let markdown=regex::Regex::new(r"(!?)\[([^\]]*)\]\(([^)]+)\)")?;
        let mut pictures:HashMap<PathBuf,String>=HashMap::new();
        for (note,entity,document,merge) in work {
            let mut body=wiki.replace_all(&note.body,|c:&regex::Captures| {
                let link=decode_link(&c[2]);let label=c.get(3).map(|m|m.as_str()).unwrap_or(&c[2]);
                if &c[1]=="!"{return format!("![{}]({link})",label);}
                let name=title_from_path(Path::new(&link));
                if let Some(id)=lookup.get(&name.to_lowercase()){bump(&mut counts,"links");format!("[{label}](entity://{id})")}else{bump(&mut counts,"unlinked");label.to_owned()}
            }).into_owned();
            let mut replacements=Vec::new();
            for capture in markdown.captures_iter(&body) {
                let link=decode_link(&capture[3]);if link.contains("://")||link.starts_with('#'){continue;}
                if &capture[1]=="!" {
                    let path=note.source.parent().ok_or("Missing source directory")?.join(&link);
                    if !path.is_file(){continue;}
                    let path=path.canonicalize()?;
                    let relative=if let Some(existing)=pictures.get(&path){existing.clone()}else{
                        let bytes=fs::read(&path)?;if bytes.len()>128*1024*1024{return Err("Imported picture is too large".into());}
                        let id=new_id();let relative=format!("media/images/{id}-{}",crate::export::filename(path.file_name().and_then(|s|s.to_str()).unwrap_or("picture")));
                        let mime=mime_guess::from_path(&path).first_or_octet_stream().to_string();
                        if !mime.starts_with("image/"){continue;}
                        db.execute("INSERT INTO media(id,filename,path,mime,kind,bytes,caption,created_at) VALUES(?1,?2,?3,?4,'image',?5,'',?6)",params![id,path.file_name().and_then(|s|s.to_str()),relative,mime,bytes.len(),now()])?;
                        World::queue_file(db,&relative,&bytes)?;pictures.insert(path,relative.clone());bump(&mut counts,"pictures");relative
                    };
                    replacements.push((capture[0].to_owned(),format!("![{}]({relative})",&capture[2])));
                }else {
                    let name=title_from_path(Path::new(&link));if let Some(id)=lookup.get(&name.to_lowercase()){replacements.push((capture[0].to_owned(),format!("[{}](entity://{id})",&capture[2])));bump(&mut counts,"links");}
                }
            }
            for (from,to) in replacements{body=body.replace(&from,&to);}
            if let Some(entity)=entity {
                let mut values=Map::new();
                if let Some(mapping)=field_maps.get(&note.group){for (label,key,kind) in mapping {if let Some(value)=note.fields.get(label){values.insert(key.clone(),coerce(value,kind,&lookup));}}}
                let mut tags=note.tags.clone();if merge {let old=crate::entities::get(world,&entity)?;for tag in old["tags"].as_array().into_iter().flatten().filter_map(Value::as_str){if !tags.iter().any(|s|s==tag){tags.push(tag.into());}}}
                crate::entities::update_in(world,db,&entity,&json!({"tags":tags,"properties":values}))?;
            }
            if merge {let previous=world.document(&document)?;body=format!("{}\n\n{body}",previous["body"].as_str().unwrap_or(""));}
            crate::documents::save_in(world,db,&document,&body)?;
        }
        Ok(counts)
    })?;
    Ok(result)
}
#[tauri::command(async)]
pub fn import_scan(state: tauri::State<crate::Session>, options: Value) -> crate::Reply<Value> {
    state.read(|w| scan(w, &options))
}
#[tauri::command(async)]
pub fn import_run(
    state: tauri::State<crate::Session>,
    options: Value,
    choices: Value,
) -> crate::Reply<Value> {
    state.read(|w| run(w, &options, &choices))
}
