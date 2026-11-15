use crate::{
    database::{new_id, now, one, update_fields},
    world::{rows, Result, World},
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};

const CHAPTER_SQL:&str="SELECT c.*,d.title,d.word_count,d.path,e.name AS pov_name
    FROM chapter c JOIN document d ON d.id=c.document_id LEFT JOIN entity e ON e.id=c.pov_entity_id WHERE c.id=?1";

pub fn get(world: &World, id: &str) -> Result<Value> {
    let db = &world.db;
    let mut book = one(db, "SELECT * FROM manuscript WHERE id=?1", id)?;
    let chapters = rows(
        db,
        "SELECT c.*,d.title,d.word_count,d.path,e.name AS pov_name FROM chapter c
        JOIN document d ON d.id=c.document_id LEFT JOIN entity e ON e.id=c.pov_entity_id
        LEFT JOIN manuscript_part p ON p.id=c.part_id WHERE c.manuscript_id=?1
        ORDER BY coalesce(p.sort_order,2147483647),c.sort_order,c.id",
        [id],
    )?;
    book["word_count"] = json!(chapters
        .iter()
        .map(|c| c["word_count"].as_u64().unwrap_or(0))
        .sum::<u64>());
    book["chapter_count"] = json!(chapters.len());
    book["chapters"] = json!(chapters);
    book["parts"] = json!(rows(
        db,
        "SELECT * FROM manuscript_part WHERE manuscript_id=?1 ORDER BY sort_order,id",
        [id]
    )?);
    book["recent_days"]=json!(rows(db,"SELECT day,words FROM writing_day WHERE manuscript_id=?1 AND day>=date('now','localtime','-29 days') ORDER BY day",[id])?);
    book["today_words"]=json!(db.query_row("SELECT coalesce(sum(words),0) FROM writing_day WHERE manuscript_id=?1 AND day=date('now','localtime')",[id],|r|r.get::<_,i64>(0))?);
    Ok(book)
}
pub fn chapter_create_in(
    db: &Connection,
    manuscript_id: &str,
    part_id: Option<&str>,
    title: &str,
) -> Result<String> {
    one(db, "SELECT id FROM manuscript WHERE id=?1", manuscript_id)?;
    check_part(db, manuscript_id, part_id)?;
    let document = crate::documents::create_in(db, "chapter", title, None)?;
    let id = new_id();
    db.execute("INSERT INTO chapter(id,manuscript_id,part_id,document_id,sort_order,created_at,updated_at)
        VALUES(?1,?2,?3,?4,(SELECT coalesce(max(sort_order),0)+1 FROM chapter WHERE manuscript_id=?2 AND part_id IS ?3),?5,?5)",
        params![id,manuscript_id,part_id,document["id"].as_str(),now()])?;
    crate::search::index_document(db, &document, "")?;
    Ok(id)
}
fn check_part(db: &Connection, book: &str, part: Option<&str>) -> Result<()> {
    if let Some(part) = part {
        if one(db, "SELECT * FROM manuscript_part WHERE id=?1", part)?["manuscript_id"] != book {
            return Err("Part belongs to a different manuscript".into());
        }
    }
    Ok(())
}
#[tauri::command(async)]
pub fn manuscript_get(state: tauri::State<crate::Session>, id: String) -> crate::Reply<Value> {
    state.read(|w| get(w, &id))
}
#[tauri::command(async)]
pub fn manuscript_create(state: tauri::State<crate::Session>, name: String) -> crate::Reply<Value> {
    state.read(|w| {
        let id = w.transaction(|db| {
            if name.trim().is_empty() {
                return Err("Enter a manuscript name".into());
            }
            let id = new_id();
            db.execute(
                "INSERT INTO manuscript(id,name,created_at,updated_at,sort_order)
                VALUES(?1,?2,?3,?3,(SELECT coalesce(max(sort_order),0)+1 FROM manuscript))",
                params![id, name.trim(), now()],
            )?;
            Ok(id)
        })?;
        get(w, &id)
    })
}
#[tauri::command(async)]
pub fn manuscript_update(
    state: tauri::State<crate::Session>,
    id: String,
    input: Value,
) -> crate::Reply<Value> {
    state.read(|w| {
        w.transaction(|db| {
            if input.get("name").is_some() {
                crate::database::required_text(&input, "name")?;
            }
            update_fields(
                db,
                "manuscript",
                &id,
                &input,
                &["name", "description", "goal_words", "compile_font"],
                true,
            )
        })?;
        get(w, &id)
    })
}
#[tauri::command(async)]
pub fn manuscript_set_layout(
    state: tauri::State<crate::Session>,
    id: String,
    layout: Value,
) -> crate::Reply<Value> {
    state.read(|w| {
        w.transaction(|db| {
            update_fields(
                db,
                "manuscript",
                &id,
                &json!({"layout":layout}),
                &["layout"],
                true,
            )
        })?;
        get(w, &id)
    })
}
#[tauri::command(async)]
pub fn manuscript_set_ebook(
    state: tauri::State<crate::Session>,
    id: String,
    options: Value,
) -> crate::Reply<Value> {
    state.read(|w| {
        w.transaction(|db| {
            update_fields(
                db,
                "manuscript",
                &id,
                &json!({"ebook":options}),
                &["ebook"],
                true,
            )
        })?;
        get(w, &id)
    })
}
#[tauri::command(async)]
pub fn manuscript_delete(state: tauri::State<crate::Session>, id: String) -> crate::Reply<()> {
    state.read(|w| crate::trash::delete(w, "manuscript", "manuscript", &id))
}
#[tauri::command(async)]
pub fn chapter_create(
    state: tauri::State<crate::Session>,
    manuscript_id: String,
    part_id: Option<String>,
    title: String,
) -> crate::Reply<Value> {
    state.read(|w| {
        let id =
            w.transaction(|db| chapter_create_in(db, &manuscript_id, part_id.as_deref(), &title))?;
        one(&w.db, CHAPTER_SQL, &id)
    })
}
#[tauri::command(async)]
pub fn chapter_get(state: tauri::State<crate::Session>, id: String) -> crate::Reply<Value> {
    state.read(|w| one(&w.db, CHAPTER_SQL, &id))
}
#[tauri::command(async)]
pub fn chapter_update(
    state: tauri::State<crate::Session>,
    id: String,
    input: Value,
) -> crate::Reply<Value> {
    state.read(|w| {
        w.transaction(|db| {
            let chapter = one(db, "SELECT * FROM chapter WHERE id=?1", &id)?;
            if let Some(title) = input["title"].as_str() {
                crate::documents::rename_in(
                    w,
                    db,
                    chapter["document_id"]
                        .as_str()
                        .ok_or("Missing chapter document")?,
                    title,
                )?;
            }
            if let Some(status) = input["status"].as_str() {
                if !["outline", "draft", "revised", "final"].contains(&status) {
                    return Err("Unknown chapter status".into());
                }
            }
            update_fields(
                db,
                "chapter",
                &id,
                &input,
                &["pov_entity_id", "status", "notes", "goal_words"],
                true,
            )
        })?;
        one(&w.db, CHAPTER_SQL, &id)
    })
}
#[tauri::command(async)]
pub fn chapter_move(
    state: tauri::State<crate::Session>,
    id: String,
    part_id: Option<String>,
    index: usize,
) -> crate::Reply<()> {
    state.read(|w|w.transaction(|db|{
        let chapter=one(db,"SELECT * FROM chapter WHERE id=?1",&id)?;
        let book=chapter["manuscript_id"].as_str().ok_or("Missing manuscript")?;
        check_part(db,book,part_id.as_deref())?;
        db.execute("UPDATE chapter SET part_id=?1,updated_at=?2 WHERE id=?3",params![part_id,now(),id])?;
        let mut order=rows(db,"SELECT id FROM chapter WHERE manuscript_id=?1 AND part_id IS ?2 AND id!=?3 ORDER BY sort_order,id",
            params![book,part_id,id])?.iter().filter_map(|r|r["id"].as_str().map(String::from)).collect::<Vec<_>>();
        order.insert(index.min(order.len()),id);
        crate::database::reorder(db,"chapter",&order)
    }))
}
#[tauri::command(async)]
pub fn chapter_delete(state: tauri::State<crate::Session>, id: String) -> crate::Reply<()> {
    state.read(|w| crate::trash::delete(w, "chapter", "chapter", &id))
}
#[tauri::command(async)]
pub fn part_create(
    state: tauri::State<crate::Session>,
    manuscript_id: String,
    name: String,
) -> crate::Reply<Value> {
    state.read(|w|w.transaction(|db|{
        let id=new_id();db.execute("INSERT INTO manuscript_part(id,manuscript_id,name,sort_order,created_at,updated_at)
            VALUES(?1,?2,?3,(SELECT coalesce(max(sort_order),0)+1 FROM manuscript_part WHERE manuscript_id=?2),?4,?4)",params![id,manuscript_id,name,now()])?;
        one(db,"SELECT * FROM manuscript_part WHERE id=?1",&id)
    }))
}
#[tauri::command(async)]
pub fn part_rename(
    state: tauri::State<crate::Session>,
    id: String,
    name: String,
) -> crate::Reply<Value> {
    state.read(|w| {
        w.transaction(|db| {
            update_fields(
                db,
                "manuscript_part",
                &id,
                &json!({"name":name}),
                &["name"],
                true,
            )?;
            one(db, "SELECT * FROM manuscript_part WHERE id=?1", &id)
        })
    })
}
#[tauri::command(async)]
pub fn part_delete(state: tauri::State<crate::Session>, id: String) -> crate::Reply<()> {
    state.read(|w| {
        w.transaction(|db| {
            db.execute("DELETE FROM manuscript_part WHERE id=?1", [id])?;
            Ok(())
        })
    })
}
#[tauri::command(async)]
pub fn manuscript_statuses() -> Vec<&'static str> {
    vec!["outline", "draft", "revised", "final"]
}
#[tauri::command(async)]
pub fn manuscript_reports(state: tauri::State<crate::Session>, id: String) -> crate::Reply<Value> {
    state.read(|w|{
        let book=get(w,&id)?;
        let pov=rows(&w.db,"SELECT c.pov_entity_id AS entity_id,e.name,count(*) AS chapters,sum(d.word_count) AS words
            FROM chapter c JOIN document d ON d.id=c.document_id LEFT JOIN entity e ON e.id=c.pov_entity_id
            WHERE c.manuscript_id=?1 GROUP BY c.pov_entity_id ORDER BY words DESC",[&id])?;
        let appearances=rows(&w.db,"SELECT l.target_id AS entity_id,e.name,count(DISTINCT c.id) AS chapters
            FROM chapter c JOIN link l ON l.source_kind='document' AND l.source_id=c.document_id
            JOIN entity e ON e.id=l.target_id WHERE c.manuscript_id=?1 GROUP BY l.target_id ORDER BY chapters DESC,e.name",[&id])?;
        Ok(json!({"total_words":book["word_count"],"total_chapters":book["chapter_count"],"pov":pov,"appearances":appearances}))
    })
}
