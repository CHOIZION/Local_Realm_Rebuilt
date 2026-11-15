//! Self-contained, static representations of maps and saved views for shared wikis.
use crate::{
    export::escape,
    world::{rows, Result, World},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs,
};

fn data_image(world: &World, path: &str) -> Result<String> {
    let file = world.local_file(path)?;
    Ok(format!(
        "data:{};base64,{}",
        mime_guess::from_path(&file).first_or_octet_stream(),
        STANDARD.encode(fs::read(file)?)
    ))
}
fn feature_visible(feature: &Value, layers: &HashMap<String, Value>) -> bool {
    feature["layer_id"]
        .as_str()
        .and_then(|id| layers.get(id))
        .is_none_or(|layer| layer["visible"] != false)
}
fn feature_opacity(feature: &Value, layers: &HashMap<String, Value>) -> f64 {
    feature["opacity"].as_f64().unwrap_or(1.)
        * feature["layer_id"]
            .as_str()
            .and_then(|id| layers.get(id))
            .and_then(|l| l["opacity"].as_f64())
            .unwrap_or(1.)
}
pub fn map_svg(world: &World, id: &str, allowed: &HashSet<String>) -> Result<String> {
    let map = crate::maps::get(world, id)?;
    let width = map["width"].as_f64().unwrap_or(0.).max(1.);
    let height = map["height"].as_f64().unwrap_or(0.).max(1.);
    let background = if map["kind"] == "painted" {
        Some(format!("maps/{id}.png"))
    } else {
        map["media_path"].as_str().map(str::to_owned)
    };
    let mut svg=format!("<svg xmlns=\"http://www.w3.org/2000/svg\" role=\"img\" viewBox=\"0 0 {width} {height}\"><title>{}</title><rect width=\"100%\" height=\"100%\" fill=\"#eee\"/>",escape(map["name"].as_str().unwrap_or("Map")));
    if let Some(path) = background {
        svg.push_str(&format!(
            "<image href=\"{}\" width=\"{width}\" height=\"{height}\"/>",
            data_image(world, &path)?
        ));
    }
    let layers: HashMap<String, Value> = map["layers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|l| Some((l["id"].as_str()?.into(), l.clone())))
        .collect();
    for region in map["regions"]
        .as_array()
        .into_iter()
        .flatten()
        .rev()
        .filter(|r| feature_visible(r, &layers))
    {
        let points = region["points"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| {
                Some(format!(
                    "{},{}",
                    p[0].as_f64()? * width,
                    p[1].as_f64()? * height
                ))
            })
            .collect::<Vec<_>>()
            .join(" ");
        let color = escape(
            region["color"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or("#bd7040"),
        );
        let opacity = feature_opacity(region, &layers);
        if region["kind"] == "route" {
            svg.push_str(&format!("<polyline points=\"{points}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"{}\" opacity=\"{opacity}\"/>",width/500.));
        } else {
            svg.push_str(&format!("<polygon points=\"{points}\" fill=\"{color}\" fill-opacity=\"{opacity}\" stroke=\"{color}\" stroke-width=\"{}\"/>",width/1000.));
        }
    }
    for stamp in map["stamps"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| feature_visible(r, &layers))
    {
        let Some(image) = stamp["image"].as_str() else {
            continue;
        };
        let x = stamp["x"].as_f64().unwrap_or(0.) * width;
        let y = stamp["y"].as_f64().unwrap_or(0.) * height;
        let w = stamp["width"].as_f64().unwrap_or(0.1) * width;
        let h = w * stamp["aspect"].as_f64().unwrap_or(1.);
        let rotation = stamp["rotation"].as_f64().unwrap_or(0.);
        let flip = if stamp["flip"] == true { -1 } else { 1 };
        svg.push_str(&format!("<g transform=\"translate({x} {y}) rotate({rotation}) scale({flip} 1)\" opacity=\"{}\"><image href=\"{}\" x=\"{}\" y=\"{}\" width=\"{w}\" height=\"{h}\"/></g>",feature_opacity(stamp,&layers),data_image(world,&format!("maps/stamps/{image}"))?,-w/2.,-h/2.));
    }
    for marker in map["markers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| feature_visible(r, &layers))
        .filter(|r| {
            r["entity_id"]
                .as_str()
                .is_none_or(|id| allowed.contains(id))
        })
    {
        let x = marker["x"].as_f64().unwrap_or(0.) * width;
        let y = marker["y"].as_f64().unwrap_or(0.) * height;
        let label = marker["label"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| marker["entity_name"].as_str())
            .unwrap_or("Pin");
        let color = escape(
            marker["color"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or("#a23e39"),
        );
        svg.push_str(&format!("<g opacity=\"{}\"><circle cx=\"{x}\" cy=\"{y}\" r=\"{}\" fill=\"{color}\" stroke=\"white\" stroke-width=\"{}\"/><text x=\"{}\" y=\"{y}\" font-family=\"sans-serif\" font-size=\"{}\" paint-order=\"stroke\" stroke=\"white\" stroke-width=\"{}\" fill=\"#222\">{}</text></g>",feature_opacity(marker,&layers),width/180.,width/700.,x+width/100.,width/70.,width/700.,escape(label)));
    }
    svg.push_str("</svg>");
    Ok(svg)
}
pub fn html(world: &World, request: &Value, allowed: &HashSet<String>) -> Result<(String, String)> {
    let omitted: HashSet<_> = request["leave_out"]["views"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let mut content = String::new();
    let mut nav = String::new();
    for (kind, table) in [
        ("map", "map"),
        ("timeline", "timeline"),
        ("tree", "relationship_tree"),
        ("table", "table_view"),
    ] {
        for view in rows(
            &world.db,
            &format!("SELECT id,name FROM {table} ORDER BY sort_order,name"),
            [],
        )? {
            let id = view["id"].as_str().ok_or("Missing view id")?;
            let key = format!("{kind}:{id}");
            if omitted.contains(key.as_str()) {
                continue;
            }
            let title = view["name"].as_str().unwrap_or("Untitled");
            let anchor = format!("{kind}-{id}");
            content.push_str(&format!(
                "<article id=\"{}\"><h2>{}</h2>",
                escape(&anchor),
                escape(title)
            ));
            nav.push_str(&format!(
                "<a href=\"#{}\">{}</a>",
                escape(&anchor),
                escape(title)
            ));
            match kind {
                "map" => content.push_str(&map_svg(world, id, allowed)?),
                "timeline" => {
                    let timeline = crate::timeline::get(world, id, &json!({}))?;
                    content.push_str("<table><thead><tr><th>Date</th><th>Event</th><th>Description</th></tr></thead><tbody>");
                    for event in timeline["events"].as_array().into_iter().flatten() {
                        if event["entities"].as_array().is_some_and(|entities| {
                            entities
                                .iter()
                                .any(|e| !allowed.contains(e["id"].as_str().unwrap_or("")))
                        }) {
                            continue;
                        }
                        content.push_str(&format!(
                            "<tr><td>{}{}</td><td>{}</td><td>{}</td></tr>",
                            escape(event["start_label"].as_str().unwrap_or("")),
                            event["end_label"]
                                .as_str()
                                .map(|end| format!(" – {}", escape(end)))
                                .unwrap_or_default(),
                            escape(event["title"].as_str().unwrap_or("")),
                            escape(event["summary"].as_str().unwrap_or(""))
                        ));
                    }
                    content.push_str("</tbody></table>");
                }
                "table" => {
                    let table = crate::database::one(
                        &world.db,
                        "SELECT * FROM table_view WHERE id=?1",
                        id,
                    )?;
                    let result =
                        crate::tables::run(world, table["type_id"].as_str(), &table["config"])?;
                    content.push_str("<table><thead><tr>");
                    for column in result["columns"].as_array().into_iter().flatten() {
                        content.push_str(&format!(
                            "<th>{}</th>",
                            escape(column["label"].as_str().unwrap_or(""))
                        ));
                    }
                    content.push_str("</tr></thead><tbody>");
                    for row in result["rows"].as_array().into_iter().flatten() {
                        if !allowed.contains(row["id"].as_str().unwrap_or("")) {
                            continue;
                        }
                        content.push_str("<tr>");
                        for column in result["columns"].as_array().into_iter().flatten() {
                            let key = column["key"].as_str().unwrap_or("");
                            content.push_str(&format!(
                                "<td>{}</td>",
                                escape(row["cells"][key]["text"].as_str().unwrap_or(""))
                            ));
                        }
                        content.push_str("</tr>");
                    }
                    content.push_str("</tbody></table>");
                }
                "tree" => {
                    let tree = crate::graph::tree_build_for(world, id)?;
                    let nodes = tree["nodes"].as_array().ok_or("Missing tree nodes")?;
                    let names: HashMap<_, _> = nodes
                        .iter()
                        .filter_map(|n| Some((n["id"].as_str()?, n["name"].as_str()?)))
                        .collect();
                    content.push_str("<ul>");
                    for edge in tree["edges"].as_array().into_iter().flatten() {
                        let source = edge["source"].as_str().unwrap_or("");
                        let target = edge["target"].as_str().unwrap_or("");
                        if !allowed.contains(source) || !allowed.contains(target) {
                            continue;
                        }
                        content.push_str(&format!(
                            "<li>{} — {} → {}</li>",
                            escape(names.get(source).copied().unwrap_or("")),
                            escape(edge["label"].as_str().unwrap_or("")),
                            escape(names.get(target).copied().unwrap_or(""))
                        ));
                    }
                    content.push_str("</ul>");
                }
                _ => {}
            }
            content.push_str("</article>");
        }
    }
    Ok((content, nav))
}
