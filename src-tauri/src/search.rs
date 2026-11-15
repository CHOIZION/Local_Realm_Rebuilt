use crate::{
    documents::{cjk, plain_text},
    world::{rows, Result, World},
};
use rusqlite::{params, Connection};
use serde_json::Value;
use std::sync::LazyLock;

fn tokenize(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    for character in text.chars() {
        if cjk(character) {
            result.push(' ');
            result.push(character);
            result.push(' ');
        } else {
            result.push(character);
        }
    }
    result
}

fn replace_index(db: &Connection, kind: &str, id: &str, fields: [&str; 5]) -> Result<()> {
    let [title, aliases, body, tags, props] = fields;
    db.execute(
        "DELETE FROM search_index WHERE kind=?1 AND ref_id=?2",
        params![kind, id],
    )?;
    db.execute("INSERT INTO search_index(kind,ref_id,title,aliases,body,tags,props) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![kind,id,tokenize(title),tokenize(aliases),tokenize(body),tokenize(tags),tokenize(props)])?;
    Ok(())
}

static ENTITY_LINK: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"entity://([A-Za-z0-9_-]+)").expect("constant entity URL pattern")
});

pub fn index_document(db: &Connection, document: &Value, body: &str) -> Result<()> {
    let id = document["id"].as_str().ok_or("Missing document id")?;
    let entity = document["entity_id"].as_str();
    let source_kind = if entity.is_some() {
        "entity"
    } else {
        "document"
    };
    let source_id = entity.unwrap_or(id);
    db.execute(
        "DELETE FROM link WHERE source_kind=?1 AND source_id=?2 AND context='body'",
        params![source_kind, source_id],
    )?;
    for link in ENTITY_LINK.captures_iter(body) {
        db.execute(
            "INSERT OR IGNORE INTO link(source_kind,source_id,target_id,context)
            SELECT ?1,?2,id,'body' FROM entity WHERE id=?3",
            params![source_kind, source_id, &link[1]],
        )?;
    }
    if let Some(entity) = entity {
        index_entity(db, entity, Some(body))?;
    } else {
        replace_index(
            db,
            "document",
            id,
            [
                document["title"].as_str().unwrap_or(""),
                "",
                &plain_text(body),
                "",
                "",
            ],
        )?;
    }
    Ok(())
}

pub fn index_entity(db: &Connection, id: &str, body: Option<&str>) -> Result<()> {
    let row = crate::database::one(db, "SELECT * FROM entity WHERE id=?1", id)?;
    let aliases: String = db.query_row(
        "SELECT coalesce(group_concat(name,' '),'') FROM alias WHERE entity_id=?1",
        [id],
        |r| r.get(0),
    )?;
    let tags:String=db.query_row("SELECT coalesce(group_concat(t.name,' '),'') FROM entity_tag et JOIN tag t ON t.id=et.tag_id WHERE et.entity_id=?1",[id],|r|r.get(0))?;
    let properties: String = db.query_row(
        "SELECT coalesce(group_concat(value,' '),'') FROM property_value WHERE entity_id=?1",
        [id],
        |r| r.get(0),
    )?;
    let summary = row["summary"].as_str().unwrap_or("");
    let text = body
        .map(|body| format!("{summary} {}", plain_text(body)))
        .unwrap_or_else(|| summary.into());
    replace_index(
        db,
        "entity",
        id,
        [
            row["name"].as_str().unwrap_or(""),
            &aliases,
            &text,
            &tags,
            &properties,
        ],
    )
}

pub fn refresh_entity(world: &World, db: &Connection, id: &str) -> Result<()> {
    use rusqlite::OptionalExtension;
    let document: Option<String> = db
        .query_row("SELECT id FROM document WHERE entity_id=?1", [id], |r| {
            r.get(0)
        })
        .optional()?;
    let body = document.map(|id| world.document(&id)).transpose()?;
    index_entity(db, id, body.as_ref().and_then(|d| d["body"].as_str()))
}

pub fn rebuild(world: &World) -> Result<()> {
    world.transaction(|db|{
        db.execute("DELETE FROM search_index",[])?;
        for document in rows(db,"SELECT * FROM document ORDER BY id",[])?{
            let contents=world.document(document["id"].as_str().ok_or("Missing document id")?)?;
            db.execute("UPDATE document SET word_count=?1 WHERE id=?2",rusqlite::params![crate::documents::word_count(contents["body"].as_str().unwrap_or("")),document["id"].as_str()])?;
            index_document(db,&document,contents["body"].as_str().unwrap_or(""))?;
        }
        for row in rows(db,"SELECT id FROM entity WHERE NOT EXISTS(SELECT 1 FROM document WHERE entity_id=entity.id)",[])?{
            index_entity(db,row["id"].as_str().ok_or("Missing entity id")?,None)?;
        }
        db.execute("DELETE FROM pending_work WHERE task='text'",[])?;
        Ok(())
    })
}

pub fn query(world: &World, query: &str, limit: usize) -> Result<Vec<Value>> {
    if query.trim().is_empty() {
        return Ok(vec![]);
    }
    static TERMS: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r#"(?:(type|tag):)?(?:"([^"]+)"|(\S+))"#)
            .expect("constant search term pattern")
    });
    let mut types = Vec::new();
    let mut tags = Vec::new();
    let mut terms = Vec::new();
    for capture in TERMS.captures_iter(query) {
        let text = capture
            .get(2)
            .or_else(|| capture.get(3))
            .map(|m| m.as_str())
            .unwrap_or("");
        match capture.get(1).map(|m| m.as_str()) {
            Some("type") => types.push(text.to_lowercase()),
            Some("tag") => tags.push(text.to_lowercase()),
            _ => {
                for term in tokenize(text).split_whitespace() {
                    terms.push(format!("\"{}\"*", term.replace('"', "\"\"")));
                }
            }
        }
    }
    let expression = terms.join(" AND ");
    let mut result = if expression.is_empty() {
        rows(&world.db,"SELECT e.id AS entity_id,NULL AS document_id,e.name AS title,e.summary AS snippet,t.name AS type_name
            FROM entity e JOIN entity_type t ON t.id=e.type_id ORDER BY e.name COLLATE NOCASE",[])?
    } else {
        rows(&world.db,"SELECT e.id AS entity_id,CASE WHEN s.kind='document' THEN d.id ELSE NULL END AS document_id,
            coalesce(e.name,d.title) AS title,t.name AS type_name,d.kind AS document_kind,
            snippet(search_index,-1,'','', '…',28) AS snippet
            FROM search_index s LEFT JOIN entity e ON s.kind='entity' AND e.id=s.ref_id
            LEFT JOIN document d ON s.kind='document' AND d.id=s.ref_id
            LEFT JOIN entity_type t ON t.id=e.type_id
            WHERE search_index MATCH ?1 ORDER BY bm25(search_index),s.ref_id",[expression])?
    };
    if !types.is_empty() || !tags.is_empty() {
        let mut filtered = Vec::new();
        for row in result {
            let id = row["entity_id"].as_str().unwrap_or("");
            let ancestors=rows(&world.db,"WITH RECURSIVE ancestors(id,name,plural,parent_id) AS (
                SELECT t.id,t.name,t.plural,t.parent_id FROM entity_type t JOIN entity e ON e.type_id=t.id WHERE e.id=?1
                UNION SELECT t.id,t.name,t.plural,t.parent_id FROM entity_type t JOIN ancestors a ON t.id=a.parent_id)
                SELECT * FROM ancestors",[id])?;
            let type_match = types.iter().all(|target| {
                ancestors.iter().any(|t| {
                    [
                        t["name"].as_str().unwrap_or(""),
                        t["plural"].as_str().unwrap_or(""),
                    ]
                    .iter()
                    .any(|n| n.to_lowercase() == *target)
                })
            });
            let entity_tags=rows(&world.db,"SELECT t.name FROM tag t JOIN entity_tag et ON et.tag_id=t.id WHERE et.entity_id=?1",[id])?;
            let tag_match = tags.iter().all(|tag| {
                entity_tags
                    .iter()
                    .any(|t| t["name"].as_str().unwrap_or("").to_lowercase() == *tag)
            });
            if type_match && tag_match {
                filtered.push(row);
            }
        }
        result = filtered;
    }
    result.truncate(limit.min(10000));
    Ok(result)
}

#[tauri::command(async)]
pub fn search_query(
    state: tauri::State<crate::Session>,
    query: String,
    limit: Option<usize>,
) -> crate::Reply<Vec<Value>> {
    state.read(|w| self::query(w, &query, limit.unwrap_or(40)))
}
#[tauri::command(async)]
pub fn search_rebuild(state: tauri::State<crate::Session>) -> crate::Reply<()> {
    state.read(rebuild)
}
