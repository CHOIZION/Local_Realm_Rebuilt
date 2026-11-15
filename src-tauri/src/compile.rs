//! Portable manuscript outputs. Page counts and PDF output share the same line layout.
use crate::{
    export::{escape, filename},
    world::{write_atomic, Result, World},
};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::{Cursor, Write},
    path::{Path, PathBuf},
};
use zip::{write::SimpleFileOptions, ZipWriter};

#[derive(Clone, Default)]
struct Run {
    text: String,
    bold: bool,
    italic: bool,
}
#[derive(Clone, Default)]
struct Paragraph {
    runs: Vec<Run>,
    heading: bool,
}
impl Paragraph {
    fn text(&self) -> String {
        self.runs.iter().map(|r| r.text.as_str()).collect()
    }
}
struct Chapter {
    id: String,
    title: String,
    body: String,
    paragraphs: Vec<Paragraph>,
}
struct Book {
    record: Value,
    chapters: Vec<Chapter>,
}

fn paragraphs(markdown: &str) -> Vec<Paragraph> {
    let mut result = Vec::new();
    let mut current = Paragraph::default();
    let mut bold = 0usize;
    let mut italic = 0usize;
    for event in Parser::new_ext(
        markdown,
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS,
    ) {
        match event {
            Event::Start(Tag::Strong) => bold += 1,
            Event::End(TagEnd::Strong) => bold = bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => italic += 1,
            Event::End(TagEnd::Emphasis) => italic = italic.saturating_sub(1),
            Event::Start(Tag::Heading { .. }) => current.heading = true,
            Event::Start(Tag::Item) => current.runs.push(Run {
                text: "• ".into(),
                bold: false,
                italic: false,
            }),
            Event::Text(text) | Event::Code(text) => current.runs.push(Run {
                text: text.into_string(),
                bold: bold > 0,
                italic: italic > 0,
            }),
            Event::SoftBreak => current.runs.push(Run {
                text: " ".into(),
                bold: false,
                italic: false,
            }),
            Event::HardBreak => current.runs.push(Run {
                text: "\n".into(),
                bold: false,
                italic: false,
            }),
            Event::End(
                TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::CodeBlock | TagEnd::TableRow,
            ) => {
                if !current.runs.is_empty() {
                    result.push(std::mem::take(&mut current));
                }
            }
            Event::End(TagEnd::TableCell) => current.runs.push(Run {
                text: "\t".into(),
                bold: false,
                italic: false,
            }),
            Event::Rule => {
                if !current.runs.is_empty() {
                    result.push(std::mem::take(&mut current));
                }
                result.push(Paragraph {
                    runs: vec![Run {
                        text: "* * *".into(),
                        bold: false,
                        italic: false,
                    }],
                    heading: false,
                });
            }
            _ => {}
        }
    }
    if !current.runs.is_empty() {
        result.push(current);
    }
    result
}
fn load(world: &World, id: &str) -> Result<Book> {
    let record = crate::manuscripts::get(world, id)?;
    let mut chapters = Vec::new();
    for chapter in record["chapters"].as_array().ok_or("Missing chapters")? {
        let doc = world.document(
            chapter["document_id"]
                .as_str()
                .ok_or("Missing chapter document")?,
        )?;
        let body = doc["body"].as_str().unwrap_or("").to_owned();
        chapters.push(Chapter {
            id: chapter["id"].as_str().ok_or("Missing chapter id")?.into(),
            title: doc["title"].as_str().unwrap_or("Untitled").into(),
            paragraphs: paragraphs(&body),
            body,
        });
    }
    Ok(Book { record, chapters })
}
#[derive(Clone, Copy)]
struct Layout {
    width: f32,
    height: f32,
    font_size: f32,
    line: f32,
    margin: f32,
    indent: f32,
}
fn layout(record: &Value) -> Result<Layout> {
    let value = &record["layout"];
    let (width, height) = match value["paper"].as_str().unwrap_or("letter") {
        "letter" => (612., 792.),
        "a4" => (595.276, 841.89),
        "a5" => (419.528, 595.276),
        "trade" | "6x9" => (432., 648.),
        _ => return Err("Unknown paper size".into()),
    };
    let font_size = value["font_size"].as_f64().unwrap_or(12.) as f32;
    let line = value["line_spacing"].as_f64().unwrap_or(2.) as f32;
    let margin = value["margin"].as_f64().unwrap_or(72.) as f32;
    let indent = value["indent"].as_f64().unwrap_or(36.) as f32;
    if !(6.0..=48.0).contains(&font_size)
        || !(0.8..=4.).contains(&line)
        || margin < 0.
        || margin * 2. >= width - 50.
        || margin * 2. >= height - 50.
        || indent < 0.
        || indent > width - margin * 2. - 10.
    {
        return Err("Invalid manuscript page layout".into());
    }
    Ok(Layout {
        width,
        height,
        font_size,
        line: font_size * line,
        margin,
        indent,
    })
}
fn font_path(book: &Book, font: &str) -> Result<PathBuf> {
    let cjk = book.record["name"]
        .as_str()
        .unwrap_or("")
        .chars()
        .any(|c| c as u32 > 0x2e7f)
        || book.chapters.iter().any(|c| {
            c.title
                .chars()
                .chain(c.body.chars())
                .any(|c| c as u32 > 0x2e7f)
        });
    let file = if cjk {
        "malgun.ttf"
    } else {
        match font {
            "times" => "times.ttf",
            "helvetica" => "arial.ttf",
            _ => "cour.ttf",
        }
    };
    let directory = std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:/Windows"))
        .join("Fonts");
    let preferred = directory.join(file);
    if preferred.is_file() {
        return Ok(preferred);
    }
    if cjk {
        return Err("Use bundled Korean print font".into());
    }
    for fallback in [
        directory.join("arial.ttf"),
        PathBuf::from("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"),
    ] {
        if fallback.is_file() {
            return Ok(fallback);
        }
    }
    Err("No usable print font is installed".into())
}
fn font_bytes(book: &Book, font: &str) -> Result<Vec<u8>> {
    match font_path(book, font) {
        Ok(path) => Ok(fs::read(path)?),
        Err(_) => Ok(include_bytes!("../resources/fonts/NanumGothic-Regular.ttf").to_vec()),
    }
}

fn width(face: &ttf_parser::Face<'_>, text: &str, size: f32) -> f32 {
    text.chars()
        .map(|c| {
            face.glyph_index(c)
                .and_then(|id| face.glyph_hor_advance(id))
                .unwrap_or(face.units_per_em() / 2) as f32
        })
        .sum::<f32>()
        * size
        / face.units_per_em() as f32
}
fn wrap(face: &ttf_parser::Face<'_>, text: &str, size: f32, max: f32, indent: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut last_space = None;
    for c in text.chars() {
        if c == '\n' {
            lines.push(std::mem::take(&mut line));
            last_space = None;
            continue;
        }
        if c.is_whitespace() {
            last_space = Some(line.len());
        }
        line.push(if c == '\t' { ' ' } else { c });
        let allowed = max - if lines.is_empty() { indent } else { 0. };
        if width(face, &line, size) > allowed && line.chars().count() > 1 {
            if let Some(space) = last_space.filter(|&i| i > 0) {
                let rest = line[space..].trim_start().to_owned();
                line.truncate(space);
                lines.push(std::mem::replace(&mut line, rest));
            } else {
                let tail = line.pop().unwrap();
                lines.push(std::mem::take(&mut line));
                line.push(tail);
            }
            last_space = None;
        }
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}
struct PrintLine {
    text: String,
    runs: Vec<Run>,
    x: f32,
    y: f32,
    size: f32,
}
struct Page {
    chapter: Option<String>,
    lines: Vec<PrintLine>,
}
fn paginate(book: &Book, face: &ttf_parser::Face<'_>, layout: Layout) -> Vec<Page> {
    let title = book.record["name"].as_str().unwrap_or("Manuscript");
    let title_lines = wrap(
        face,
        title,
        layout.font_size * 1.5,
        layout.width - 2. * layout.margin,
        0.,
    );
    let mut pages = vec![Page {
        chapter: None,
        lines: title_lines
            .into_iter()
            .enumerate()
            .map(|(index, text)| PrintLine {
                runs: vec![Run {
                    text: text.clone(),
                    bold: true,
                    italic: false,
                }],
                text,
                x: layout.margin,
                y: layout.height * 0.66 - index as f32 * layout.line * 1.5,
                size: layout.font_size * 1.5,
            })
            .collect(),
    }];
    for chapter in &book.chapters {
        let mut page = Page {
            chapter: Some(chapter.id.clone()),
            lines: Vec::new(),
        };
        let mut y = layout.height - layout.margin;
        let heading = Paragraph {
            runs: vec![Run {
                text: chapter.title.clone(),
                bold: true,
                italic: false,
            }],
            heading: true,
        };
        for paragraph in std::iter::once(&heading).chain(&chapter.paragraphs) {
            let size = if paragraph.heading {
                layout.font_size * 1.25
            } else {
                layout.font_size
            };
            let indent = if paragraph.heading { 0. } else { layout.indent };
            let original = paragraph.text();
            let mut consumed = 0;
            for (index, text) in wrap(
                face,
                &paragraph.text(),
                size,
                layout.width - 2. * layout.margin,
                indent,
            )
            .into_iter()
            .enumerate()
            {
                if y - layout.line < layout.margin {
                    pages.push(page);
                    page = Page {
                        chapter: Some(chapter.id.clone()),
                        lines: Vec::new(),
                    };
                    y = layout.height - layout.margin;
                }
                y -= layout.line;
                page.lines.push(PrintLine {
                    runs: line_runs(paragraph, &original, &text, &mut consumed),
                    text,
                    x: layout.margin + if index == 0 { indent } else { 0. },
                    y,
                    size,
                });
            }
            if paragraph.heading {
                y -= layout.line;
            }
        }
        pages.push(page);
    }
    pages
}
fn line_runs(paragraph: &Paragraph, original: &str, line: &str, consumed: &mut usize) -> Vec<Run> {
    let Some(offset) = original[*consumed..].find(line) else {
        return vec![Run {
            text: line.into(),
            bold: paragraph.heading,
            italic: false,
        }];
    };
    let start = *consumed + offset;
    let end = start + line.len();
    *consumed = end;
    let mut result = Vec::new();
    let mut position = 0;
    for run in &paragraph.runs {
        let next = position + run.text.len();
        if start < next && end > position {
            result.push(Run {
                text: run.text
                    [start.saturating_sub(position)..(end - position).min(run.text.len())]
                    .into(),
                bold: run.bold || paragraph.heading,
                italic: run.italic,
            });
        }
        position = next;
    }
    result
}
pub fn pages(world: &World, id: &str) -> Result<Value> {
    let book = load(world, id)?;
    let layout = layout(&book.record)?;
    let font = book.record["compile_font"].as_str().unwrap_or("courier");
    let bytes = font_bytes(&book, font)?;
    let face = ttf_parser::Face::parse(&bytes, 0)?;
    let pages = paginate(&book, &face, layout);
    let chapters:Vec<_>=book.chapters.iter().map(|c|json!({"id":c.id,"pages":pages.iter().filter(|p|p.chapter.as_deref()==Some(&c.id)).count()})).collect();
    Ok(json!({"total":pages.len(),"chapters":chapters}))
}
fn zip_entry(
    zip: &mut ZipWriter<Cursor<Vec<u8>>>,
    name: &str,
    bytes: &[u8],
    stored: bool,
) -> Result<()> {
    zip.start_file(
        name,
        SimpleFileOptions::default().compression_method(if stored {
            zip::CompressionMethod::Stored
        } else {
            zip::CompressionMethod::Deflated
        }),
    )?;
    zip.write_all(bytes)?;
    Ok(())
}
fn docx(book: &Book, font: &str) -> Result<Vec<u8>> {
    let layout = layout(&book.record)?;
    let font = match font {
        "times" => "Times New Roman",
        "helvetica" => "Arial",
        _ => "Courier New",
    };
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>"#,
    );
    let title = Paragraph {
        runs: vec![Run {
            text: book.record["name"].as_str().unwrap_or("Manuscript").into(),
            bold: true,
            italic: false,
        }],
        heading: true,
    };
    let mut paragraph_xml = |p: &Paragraph, page_break: bool| {
        xml.push_str("<w:p><w:pPr>");
        if page_break {
            xml.push_str("<w:pageBreakBefore/>");
        }
        if p.heading {
            xml.push_str("<w:pStyle w:val=\"Heading1\"/>");
        }
        xml.push_str(&format!("<w:spacing w:line=\"{}\" w:lineRule=\"auto\" w:after=\"0\"/><w:ind w:firstLine=\"{}\"/><w:jc w:val=\"{}\"/></w:pPr>",(layout.line/layout.font_size*240.) as u32,if p.heading{0}else{(layout.indent*20.) as u32},if book.record["layout"]["align"]=="justify"{"both"}else{"left"}));
        for run in &p.runs {
            xml.push_str(&format!("<w:r><w:rPr><w:rFonts w:ascii=\"{font}\" w:hAnsi=\"{font}\" w:eastAsia=\"Malgun Gothic\"/><w:sz w:val=\"{}\"/>{}{}</w:rPr>",(layout.font_size*if p.heading{2.5}else{2.}) as u32,if run.bold||p.heading{"<w:b/>"}else{""},if run.italic{"<w:i/>"}else{""}));
            for (i, line) in run.text.split('\n').enumerate() {
                if i > 0 {
                    xml.push_str("<w:br/>");
                }
                xml.push_str(&format!(
                    "<w:t xml:space=\"preserve\">{}</w:t>",
                    escape(line)
                ));
            }
            xml.push_str("</w:r>");
        }
        xml.push_str("</w:p>");
    };
    paragraph_xml(&title, false);
    for chapter in &book.chapters {
        paragraph_xml(
            &Paragraph {
                runs: vec![Run {
                    text: chapter.title.clone(),
                    bold: true,
                    italic: false,
                }],
                heading: true,
            },
            true,
        );
        for paragraph in &chapter.paragraphs {
            paragraph_xml(paragraph, false);
        }
    }
    let margin = (layout.margin * 20.) as u32;
    xml.push_str(&format!("<w:sectPr><w:pgSz w:w=\"{}\" w:h=\"{}\"/><w:pgMar w:top=\"{margin}\" w:right=\"{margin}\" w:bottom=\"{margin}\" w:left=\"{margin}\" w:header=\"360\" w:footer=\"360\"/></w:sectPr></w:body></w:document>",(layout.width*20.) as u32,(layout.height*20.) as u32));
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip_entry(&mut zip,"[Content_Types].xml",br#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>"#,false)?;
    zip_entry(&mut zip,"_rels/.rels",br#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,false)?;
    zip_entry(&mut zip,"word/_rels/document.xml.rels",br#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="styles" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#,false)?;
    zip_entry(&mut zip,"word/styles.xml",br#"<?xml version="1.0" encoding="UTF-8"?><w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:pPr><w:keepNext/><w:outlineLvl w:val="0"/></w:pPr></w:style></w:styles>"#,false)?;
    zip_entry(&mut zip, "word/document.xml", xml.as_bytes(), false)?;
    Ok(zip.finish()?.into_inner())
}

fn pdf(book: &Book, font: &str) -> Result<Vec<u8>> {
    use lopdf::{
        content::{Content, Operation},
        dictionary, Document, Object, Stream,
    };
    let layout = layout(&book.record)?;
    let bytes = font_bytes(book, font)?;
    let face = ttf_parser::Face::parse(&bytes, 0)?;
    let pages = paginate(book, &face, layout);
    let mut glyphs = BTreeMap::new();
    for page in &pages {
        for line in &page.lines {
            for character in line.text.chars() {
                let glyph = face
                    .glyph_index(character)
                    .ok_or_else(|| format!("The print font has no glyph for {character}"))?;
                glyphs.entry(glyph.0).or_insert(character);
            }
        }
    }
    for character in '0'..='9' {
        if let Some(glyph) = face.glyph_index(character) {
            glyphs.entry(glyph.0).or_insert(character);
        }
    }
    let mut pdf = Document::with_version("1.7");
    let pages_id = pdf.new_object_id();
    let font_file = pdf.add_object(Stream::new(
        dictionary! {"Length1"=>bytes.len() as i64},
        bytes.clone(),
    ));
    let scale = 1000. / face.units_per_em() as f32;
    let bbox = face.global_bounding_box();
    let descriptor=pdf.add_object(dictionary!{"Type"=>"FontDescriptor","FontName"=>"LocalRealmPrint","Flags"=>32,"FontBBox"=>vec![Object::Real(bbox.x_min as f32*scale),Object::Real(bbox.y_min as f32*scale),Object::Real(bbox.x_max as f32*scale),Object::Real(bbox.y_max as f32*scale)],"ItalicAngle"=>0,"Ascent"=>face.ascender() as f32*scale,"Descent"=>face.descender() as f32*scale,"CapHeight"=>face.capital_height().unwrap_or(face.ascender()) as f32*scale,"StemV"=>80,"FontFile2"=>font_file});
    let mut widths = Vec::new();
    for &glyph in glyphs.keys() {
        widths.push(Object::Integer(glyph as i64));
        widths.push(Object::Array(vec![Object::Real(
            face.glyph_hor_advance(ttf_parser::GlyphId(glyph))
                .unwrap_or(0) as f32
                * scale,
        )]));
    }
    let descendant=pdf.add_object(dictionary!{"Type"=>"Font","Subtype"=>"CIDFontType2","BaseFont"=>"LocalRealmPrint","CIDSystemInfo"=>dictionary!{"Registry"=>Object::string_literal("Adobe"),"Ordering"=>Object::string_literal("Identity"),"Supplement"=>0},"FontDescriptor"=>descriptor,"DW"=>1000,"W"=>Object::Array(widths),"CIDToGIDMap"=>"Identity"});
    let mut cmap=String::from("/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /LocalRealmUnicode def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n");
    let mappings: Vec<_> = glyphs.iter().collect();
    for chunk in mappings.chunks(100) {
        cmap.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for (glyph, character) in chunk {
            let mut buf = [0u16; 2];
            let encoded = character.encode_utf16(&mut buf);
            cmap.push_str(&format!(
                "<{glyph:04X}> <{}>\n",
                encoded
                    .iter()
                    .map(|n| format!("{n:04X}"))
                    .collect::<String>()
            ));
        }
        cmap.push_str("endbfchar\n");
    }
    cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend");
    let unicode = pdf.add_object(Stream::new(dictionary! {}, cmap.into_bytes()));
    let font_id=pdf.add_object(dictionary!{"Type"=>"Font","Subtype"=>"Type0","BaseFont"=>"LocalRealmPrint","Encoding"=>"Identity-H","DescendantFonts"=>vec![Object::Reference(descendant)],"ToUnicode"=>unicode});
    let resources = pdf.add_object(dictionary! {"Font"=>dictionary!{"F1"=>font_id}});
    let mut children = Vec::new();
    for (index, page) in pages.iter().enumerate() {
        let mut operations = Vec::new();
        let number = PrintLine {
            text: (index + 1).to_string(),
            runs: vec![Run {
                text: (index + 1).to_string(),
                bold: false,
                italic: false,
            }],
            x: layout.width / 2.,
            y: layout.margin / 2.,
            size: layout.font_size,
        };
        for line in page.lines.iter().chain(std::iter::once(&number)) {
            let mut x = line.x;
            for run in &line.runs {
                let encoded: Vec<u8> = run
                    .text
                    .chars()
                    .filter_map(|c| face.glyph_index(c))
                    .flat_map(|g| g.0.to_be_bytes())
                    .collect();
                operations.extend([
                    Operation::new("w", vec![Object::Real(0.2)]),
                    Operation::new("BT", vec![]),
                    Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), line.size.into()]),
                    Operation::new("Tr", vec![Object::Integer(if run.bold { 2 } else { 0 })]),
                    Operation::new(
                        "Tm",
                        vec![
                            1.into(),
                            0.into(),
                            Object::Real(if run.italic { 0.2 } else { 0. }),
                            1.into(),
                            x.into(),
                            line.y.into(),
                        ],
                    ),
                    Operation::new(
                        "Tj",
                        vec![Object::String(encoded, lopdf::StringFormat::Hexadecimal)],
                    ),
                    Operation::new("ET", vec![]),
                ]);
                x += width(&face, &run.text, line.size);
            }
        }
        let content = pdf.add_object(Stream::new(
            dictionary! {},
            Content { operations }.encode()?,
        ));
        let id=pdf.add_object(dictionary!{"Type"=>"Page","Parent"=>pages_id,"MediaBox"=>vec![Object::Integer(0),Object::Integer(0),Object::Real(layout.width),Object::Real(layout.height)],"Resources"=>resources,"Contents"=>content});
        children.push(Object::Reference(id));
    }
    pdf.objects.insert(
        pages_id,
        Object::Dictionary(
            dictionary! {"Type"=>"Pages","Kids"=>children,"Count"=>pages.len() as i64},
        ),
    );
    let catalog = pdf.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages_id});
    pdf.trailer.set("Root", catalog);
    pdf.compress();
    let mut output = Vec::new();
    pdf.save_to(&mut output)?;
    Ok(output)
}

fn epub(world: &World, book: &Book) -> Result<Vec<u8>> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip_entry(&mut zip, "mimetype", b"application/epub+zip", true)?;
    zip_entry(&mut zip,"META-INF/container.xml",br#"<?xml version="1.0" encoding="UTF-8"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#,false)?;
    let options = &book.record["ebook"];
    let title = book.record["name"].as_str().unwrap_or("Manuscript");
    let language = options["language"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or("en");
    let xhtml = |title: &str, body: &str| {
        format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" lang=\"{}\"><head><title>{}</title><link rel=\"stylesheet\" type=\"text/css\" href=\"style.css\"/></head><body>{body}</body></html>",escape(language),escape(title))
    };
    zip_entry(&mut zip,"OEBPS/style.css",b"body{line-height:1.5}h1,h2{page-break-after:avoid}p{text-indent:1.5em;margin:0}img{max-width:100%}.title{text-align:center;margin-top:25%}nav ol{list-style:none}blockquote{margin-left:1em}",false)?;
    let mut manifest=String::from("<item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/><item id=\"css\" href=\"style.css\" media-type=\"text/css\"/>");
    let mut spine = String::new();
    let mut nav = String::new();
    let mut resources: HashMap<String, String> = HashMap::new();
    let mut sections = vec![(
        "title".to_owned(),
        title.to_owned(),
        format!(
            "<div class=\"title\"><h1>{}</h1><p>{}</p></div>",
            escape(title),
            escape(options["author"].as_str().unwrap_or(""))
        ),
    )];
    for key in ["copyright", "dedication"] {
        if let Some(text) = options[key].as_str().filter(|s| !s.is_empty()) {
            sections.push((key.into(), key.into(), crate::documents::preview(text)));
        }
    }
    for (index, chapter) in book.chapters.iter().enumerate() {
        sections.push((
            format!("chapter-{index}"),
            chapter.title.clone(),
            format!(
                "<h1>{}</h1>{}",
                escape(&chapter.title),
                crate::documents::preview(&chapter.body)
            ),
        ));
    }
    let image_pattern = regex::Regex::new(r#"src="(media/[^"]+)""#)?;
    let void_pattern = regex::Regex::new(r"<(img|br|hr)(\s[^>]*)?>")?;
    for (key, name, mut html) in sections {
        let mut replacements = Vec::new();
        for capture in image_pattern.captures_iter(&html) {
            let relative = &capture[1];
            let resource = if let Some(existing) = resources.get(relative) {
                existing.clone()
            } else {
                let source = world.local_file(relative)?;
                let resource = format!(
                    "image-{}.{}",
                    resources.len(),
                    source.extension().and_then(|s| s.to_str()).unwrap_or("png")
                );
                zip_entry(
                    &mut zip,
                    &format!("OEBPS/{resource}"),
                    &fs::read(&source)?,
                    false,
                )?;
                manifest.push_str(&format!(
                    "<item id=\"image-{}\" href=\"{}\" media-type=\"{}\"/>",
                    resources.len(),
                    escape(&resource),
                    mime_guess::from_path(source).first_or_octet_stream()
                ));
                resources.insert(relative.into(), resource.clone());
                resource
            };
            replacements.push((capture[0].to_owned(), format!("src=\"{resource}\"")));
        }
        for (from, to) in replacements {
            html = html.replace(&from, &to);
        }
        html = void_pattern.replace_all(&html, "<$1$2 />").into_owned();
        zip_entry(
            &mut zip,
            &format!("OEBPS/{key}.xhtml"),
            xhtml(&name, &html).as_bytes(),
            false,
        )?;
        manifest.push_str(&format!(
            "<item id=\"{key}\" href=\"{key}.xhtml\" media-type=\"application/xhtml+xml\"/>"
        ));
        spine.push_str(&format!("<itemref idref=\"{key}\"/>"));
        nav.push_str(&format!(
            "<li><a href=\"{key}.xhtml\">{}</a></li>",
            escape(&name)
        ));
    }
    let navigation = xhtml(
        "Contents",
        &format!("<nav epub:type=\"toc\" id=\"toc\"><h1>Contents</h1><ol>{nav}</ol></nav>"),
    );
    zip_entry(&mut zip, "OEBPS/nav.xhtml", navigation.as_bytes(), false)?;
    let opf=format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"book-id\"><metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:identifier id=\"book-id\">urn:uuid:{}</dc:identifier><dc:title>{}</dc:title><dc:language>{}</dc:language><dc:creator>{}</dc:creator><dc:publisher>{}</dc:publisher><dc:description>{}</dc:description><meta property=\"dcterms:modified\">{}</meta></metadata><manifest>{manifest}</manifest><spine>{spine}</spine></package>",escape(book.record["id"].as_str().unwrap_or("")),escape(title),escape(language),escape(options["author"].as_str().unwrap_or("")),escape(options["publisher"].as_str().unwrap_or("")),escape(options["description"].as_str().unwrap_or("")),chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ"));
    zip_entry(&mut zip, "OEBPS/content.opf", opf.as_bytes(), false)?;
    Ok(zip.finish()?.into_inner())
}
pub fn compile(
    world: &World,
    id: &str,
    format: &str,
    destination: &Path,
    font: Option<&str>,
) -> Result<Value> {
    let book = load(world, id)?;
    let font = font.unwrap_or_else(|| book.record["compile_font"].as_str().unwrap_or("courier"));
    let (extension, bytes) = match format {
        "docx" => ("docx", docx(&book, font)?),
        "pdf" => ("pdf", pdf(&book, font)?),
        "epub" => ("epub", epub(world, &book)?),
        "markdown" => {
            let mut text = format!(
                "# {}\n\n",
                book.record["name"].as_str().unwrap_or("Manuscript")
            );
            for chapter in &book.chapters {
                text.push_str(&format!("## {}\n\n{}\n\n", chapter.title, chapter.body));
            }
            ("md", text.into_bytes())
        }
        _ => return Err("Unknown manuscript format".into()),
    };
    let path = destination.join(format!(
        "{}.{}",
        filename(book.record["name"].as_str().unwrap_or("Manuscript")),
        extension
    ));
    write_atomic(&path, &bytes)?;
    Ok(json!({"path":path.to_string_lossy(),"warning":null,"bytes":bytes.len()}))
}
#[tauri::command(async)]
pub fn manuscript_pages(state: tauri::State<crate::Session>, id: String) -> crate::Reply<Value> {
    state.read(|w| pages(w, &id))
}
#[tauri::command(async)]
pub fn manuscript_compile(
    state: tauri::State<crate::Session>,
    id: String,
    format: String,
    destination: String,
    font: Option<String>,
) -> crate::Reply<Value> {
    state.read(|w| compile(w, &id, &format, Path::new(&destination), font.as_deref()))
}
#[tauri::command(async)]
pub fn manuscript_formats() -> Value {
    json!([{"key":"docx","label":"Word (.docx)"},{"key":"pdf","label":"PDF"},{"key":"epub","label":"EPUB"},{"key":"markdown","label":"Markdown"}])
}
#[tauri::command(async)]
pub fn manuscript_fonts() -> Value {
    json!([{"key":"courier","label":"Courier"},{"key":"times","label":"Times"},{"key":"helvetica","label":"Helvetica"}])
}
