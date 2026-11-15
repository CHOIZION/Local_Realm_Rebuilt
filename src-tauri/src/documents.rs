use crate::{
    database::{new_id, now, one},
    world::{rows, Result, World},
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};

pub fn body_from_file(raw: &str) -> &str {
    if let Some(rest) = raw.strip_prefix("---\n") {
        if let Some((_, body)) = rest.split_once("\n---\n") {
            return body.strip_prefix('\n').unwrap_or(body);
        }
    }
    raw
}

fn render_file(document: &Value, body: &str) -> Result<Vec<u8>> {
    // A JSON string is also a valid YAML scalar, including embedded newlines and colons.
    let mut header = format!(
        "---\nid: {}\ntitle: {}\nkind: {}\nupdated: {}\n",
        document["id"].as_str().ok_or("Missing document id")?,
        serde_json::to_string(&document["title"])?,
        document["kind"].as_str().ok_or("Missing document kind")?,
        document["updated_at"]
            .as_str()
            .ok_or("Missing document timestamp")?
    );
    if let Some(entity) = document["entity_id"].as_str() {
        header.push_str(&format!("entity: {entity}\n"));
    }
    Ok(format!("{header}---\n\n{body}").into_bytes())
}

pub fn queue_body(db: &Connection, document: &Value, body: &str) -> Result<()> {
    World::queue_file(
        db,
        document["path"].as_str().ok_or("Missing document path")?,
        &render_file(document, body)?,
    )
}

pub fn create_in(
    db: &Connection,
    kind: &str,
    title: &str,
    entity_id: Option<&str>,
) -> Result<Value> {
    let id = new_id();
    let path = match kind {
        "article" => format!("content/custom/{id}.md"),
        "chapter" => format!("manuscripts/{id}.md"),
        "note" => format!("content/custom/notes/{id}.md"),
        _ => return Err("Unknown document kind".into()),
    };
    let time = now();
    db.execute(
        "INSERT INTO document(id,entity_id,kind,title,path,created_at,updated_at,sort_order)
        VALUES (?1,?2,?3,?4,?5,?6,?6,(SELECT coalesce(max(sort_order),0)+1 FROM document))",
        params![id, entity_id, kind, title, path, time],
    )?;
    let document = one(db, "SELECT * FROM document WHERE id=?1", &id)?;
    World::queue_file(db, &path, &render_file(&document, "")?)?;
    Ok(document)
}

pub fn save_in(world: &World, db: &Connection, id: &str, body: &str) -> Result<Value> {
    let mut document = one(db, "SELECT * FROM document WHERE id=?1", id)?;
    let previous = world.document(id)?;
    if previous["body"].as_str() == Some(body) {
        return Ok(previous);
    }
    db.execute(
        "INSERT INTO revision(id,document_id,body,word_count,created_at) VALUES(?1,?2,?3,?4,?5)",
        params![
            new_id(),
            id,
            previous["body"].as_str(),
            previous["word_count"].as_i64(),
            now()
        ],
    )?;
    document["updated_at"] = json!(now());
    document["word_count"] = json!(word_count(body));
    db.execute(
        "UPDATE document SET word_count=?1,updated_at=?2 WHERE id=?3",
        params![
            document["word_count"].as_u64(),
            document["updated_at"].as_str(),
            id
        ],
    )?;
    if document["kind"] == "chapter" {
        let delta = document["word_count"].as_i64().unwrap_or(0)
            - previous["word_count"].as_i64().unwrap_or(0);
        db.execute(
            "INSERT INTO writing_day(day,manuscript_id,words)
            SELECT ?1,manuscript_id,?2 FROM chapter WHERE document_id=?3
            ON CONFLICT(day,manuscript_id) DO UPDATE SET words=words+excluded.words",
            params![
                chrono::Local::now().format("%Y-%m-%d").to_string(),
                delta,
                id
            ],
        )?;
    }
    if let Some(entity) = document["entity_id"].as_str() {
        db.execute(
            "UPDATE entity SET updated_at=?1 WHERE id=?2",
            params![now(), entity],
        )?;
    }
    World::queue_file(
        db,
        document["path"].as_str().ok_or("Missing document path")?,
        &render_file(&document, body)?,
    )?;
    crate::search::index_document(db, &document, body)?;
    document["body"] = json!(body);
    Ok(document)
}

pub fn rename_in(world: &World, db: &Connection, id: &str, title: &str) -> Result<Value> {
    if title.trim().is_empty() {
        return Err("Enter a document title".into());
    }
    let previous = world.document(id)?;
    db.execute(
        "UPDATE document SET title=?1,updated_at=?2 WHERE id=?3",
        params![title, now(), id],
    )?;
    let document = one(db, "SELECT * FROM document WHERE id=?1", id)?;
    let body = previous["body"].as_str().ok_or("Missing document body")?;
    World::queue_file(
        db,
        document["path"].as_str().ok_or("Missing document path")?,
        &render_file(&document, body)?,
    )?;
    crate::search::index_document(db, &document, body)?;
    Ok(document)
}

pub fn cjk(character: char) -> bool {
    matches!(character as u32, 0x3040..=0x30ff | 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff | 0x20000..=0x3134f)
}

pub fn plain_text(body: &str) -> String {
    use pulldown_cmark::{Event, Parser};
    let mut text = String::new();
    for event in Parser::new(body) {
        match event {
            Event::Text(value) | Event::Code(value) => text.push_str(&value),
            Event::SoftBreak | Event::HardBreak | Event::End(_) => text.push(' '),
            _ => {}
        }
    }
    text
}

pub fn word_count(body: &str) -> usize {
    let text = plain_text(body);
    let mut count = 0;
    let mut word = false;
    for character in text.chars() {
        if cjk(character) {
            count += 1;
            word = false;
        } else if character.is_alphanumeric() {
            if !word {
                count += 1;
            }
            word = true;
        } else if character != '\'' && character != '’' {
            word = false;
        }
    }
    count
}

pub fn preview(body: &str) -> String {
    let options = pulldown_cmark::Options::ENABLE_TABLES
        | pulldown_cmark::Options::ENABLE_STRIKETHROUGH
        | pulldown_cmark::Options::ENABLE_TASKLISTS;
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, pulldown_cmark::Parser::new_ext(body, options));
    let mut sanitizer = ammonia::Builder::default();
    sanitizer.add_url_schemes(["entity"]);
    sanitizer.clean(&html).to_string()
}

#[tauri::command(async)]
pub fn document_save(
    state: tauri::State<crate::Session>,
    id: String,
    body: String,
) -> crate::Reply<Value> {
    state.read(|w| w.transaction(|db| save_in(w, db, &id, &body)))
}
#[tauri::command(async)]
pub fn note_create(state: tauri::State<crate::Session>, title: String) -> crate::Reply<Value> {
    state.read(|w| {
        w.transaction(|db| {
            if title.trim().is_empty() {
                return Err("Enter a note title".into());
            }
            let note = create_in(db, "note", title.trim(), None)?;
            crate::search::index_document(db, &note, "")?;
            Ok(note)
        })
    })
}
#[tauri::command(async)]
pub fn note_rename(
    state: tauri::State<crate::Session>,
    id: String,
    title: String,
) -> crate::Reply<Value> {
    state.read(|w| w.transaction(|db| rename_in(w, db, &id, &title)))
}
#[tauri::command(async)]
pub fn revision_list(
    state: tauri::State<crate::Session>,
    document_id: String,
) -> crate::Reply<Vec<Value>> {
    state.read(|w|rows(&w.db,"SELECT id,document_id,word_count,created_at FROM revision WHERE document_id=?1 ORDER BY created_at DESC,rowid DESC",[document_id]))
}
#[tauri::command(async)]
pub fn revision_body(state: tauri::State<crate::Session>, id: String) -> crate::Reply<String> {
    state.read(|w| {
        Ok(w.db.query_row("SELECT body FROM revision WHERE id=?1", [id], |r| r.get(0))?)
    })
}
#[tauri::command(async)]
pub fn revision_restore(state: tauri::State<crate::Session>, id: String) -> crate::Reply<Value> {
    state.read(|w| {
        w.transaction(|db| {
            let revision = one(db, "SELECT * FROM revision WHERE id=?1", &id)?;
            save_in(
                w,
                db,
                revision["document_id"]
                    .as_str()
                    .ok_or("Missing revision document")?,
                revision["body"].as_str().ok_or("Missing revision body")?,
            )
        })
    })
}
#[tauri::command(async)]
pub fn markdown_preview(body: String) -> String {
    preview(&body)
}
