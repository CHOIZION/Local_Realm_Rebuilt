//! Deterministic terrain generation and palette-preserving map image storage.
use crate::{
    database::{now, one},
    world::{Result, World},
};
use rusqlite::params;
use serde_json::{json, Value};
use std::{
    fs::File,
    io::{BufReader, Cursor},
    path::Path,
};

const MAX_CELLS: u64 = 4096 * 4096;

fn dimensions(options: &Value) -> Result<(u32, u32)> {
    let width = options["cols"].as_u64().unwrap_or(512);
    let height = options["rows"].as_u64().unwrap_or(256);
    if width < 16 || height < 16 || width > 4096 || height > 4096 || width * height > MAX_CELLS {
        return Err("Map dimensions must be 16–4096 with at most 16,777,216 cells".into());
    }
    Ok((width as u32, height as u32))
}
fn color(text: &str) -> Result<[u8; 3]> {
    let hex = text
        .strip_prefix('#')
        .ok_or("Expected a hexadecimal biome color")?;
    if hex.len() != 6 || !hex.is_ascii() {
        return Err("Expected a six-digit biome color".into());
    }
    Ok([
        u8::from_str_radix(&hex[0..2], 16)?,
        u8::from_str_radix(&hex[2..4], 16)?,
        u8::from_str_radix(&hex[4..6], 16)?,
    ])
}
fn palette(biomes: &Value) -> Result<Vec<u8>> {
    let mut result = vec![0u8; 768];
    let mut seen = [false; 256];
    for biome in biomes
        .as_array()
        .filter(|a| !a.is_empty())
        .ok_or("A map needs biomes")?
    {
        let id = biome["id"]
            .as_u64()
            .filter(|n| *n < 256)
            .ok_or("Biome id must be 0–255")? as usize;
        if seen[id] {
            return Err("Repeated biome id".into());
        }
        seen[id] = true;
        if biome["name"].as_str().is_none_or(|n| n.trim().is_empty()) {
            return Err("Give the biome a name".into());
        }
        result[id * 3..id * 3 + 3].copy_from_slice(&color(
            biome["color"].as_str().ok_or("Missing biome color")?,
        )?);
    }
    Ok(result)
}
fn png_indexed(width: u32, height: u32, cells: &[u8], biomes: &Value) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, width, height);
    encoder.set_color(png::ColorType::Indexed);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_palette(palette(biomes)?);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(cells)?;
    writer.finish()?;
    Ok(bytes)
}
fn png_height(width: u32, height: u32, heights: &[f32]) -> Result<Vec<u8>> {
    let pixels: Vec<u8> = heights
        .iter()
        .flat_map(|v| ((v.clamp(0., 1.) * 65535.).round() as u16).to_be_bytes())
        .collect();
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, width, height);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::Sixteen);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&pixels)?;
    writer.finish()?;
    Ok(bytes)
}

fn hash(x: i32, y: i32, seed: u32) -> f32 {
    let mut v = (x as u32).wrapping_mul(0x9e3779b9) ^ (y as u32).wrapping_mul(0x85ebca6b) ^ seed;
    v ^= v >> 16;
    v = v.wrapping_mul(0x7feb352d);
    v ^= v >> 15;
    v = v.wrapping_mul(0x846ca68b);
    v ^= v >> 16;
    v as f32 / u32::MAX as f32
}
fn noise(x: f32, y: f32, seed: u32, wrap: Option<i32>) -> f32 {
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let ease = |t: f32| t * t * t * (t * (t * 6. - 15.) + 10.);
    let (fx, fy) = (ease(x - x.floor()), ease(y - y.floor()));
    let at = |x: i32, y: i32| hash(wrap.map(|n| x.rem_euclid(n)).unwrap_or(x), y, seed);
    let low = at(ix, iy) * (1. - fx) + at(ix + 1, iy) * fx;
    let high = at(ix, iy + 1) * (1. - fx) + at(ix + 1, iy + 1) * fx;
    low * (1. - fy) + high * fy
}
fn heightfield(width: u32, height: u32, seed: u32, shape: &Value) -> Vec<f32> {
    let kind = shape["kind"].as_str().unwrap_or("flat");
    let wrap = matches!(
        kind,
        "globe" | "hollow" | "ring" | "roche" | "cylinder" | "oneill"
    );
    let mut heights = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        let v = y as f32 / height as f32;
        for x in 0..width {
            let u = x as f32 / width as f32;
            let mut elevation = 0.;
            let mut weight = 1.;
            let mut total = 0.;
            for octave in 0..6u32 {
                let frequency = 4u32 << octave;
                elevation += weight
                    * noise(
                        u * frequency as f32,
                        v * frequency as f32 * height as f32 / width as f32,
                        seed.wrapping_add(octave * 173),
                        wrap.then_some(frequency as i32),
                    );
                total += weight;
                weight *= 0.5;
            }
            elevation /= total;
            if kind == "islands" {
                let edge = ((u * 2. - 1.).abs().max((v * 2. - 1.).abs())).powi(3);
                elevation -= edge * 0.4 + 0.04;
            } else if kind == "flat" || kind == "disc" {
                let edge = if kind == "disc" {
                    ((u * 2. - 1.).powi(2) + (v * 2. - 1.).powi(2)).sqrt()
                } else {
                    (u * 2. - 1.).abs().max((v * 2. - 1.).abs())
                };
                elevation -= (edge - 0.7).max(0.) * 0.7;
            }
            heights.push(elevation.clamp(0., 1.));
        }
    }
    heights
}

fn erode(heights: &mut [f32], width: usize, height: usize, options: &Value) {
    let rounds = match options["erosion"].as_str().unwrap_or("light") {
        "none" => 0,
        "heavy" => 16,
        "medium" => 8,
        _ => 3,
    };
    let mut delta = vec![0f32; heights.len()];
    // Thermal erosion transfers sediment downhill; the total height is conserved.
    for _ in 0..rounds {
        delta.fill(0.);
        for y in 1..height - 1 {
            for x in 1..width - 1 {
                let i = y * width + x;
                let neighbors = [i - 1, i + 1, i - width, i + width];
                let lowest = *neighbors
                    .iter()
                    .min_by(|&&a, &&b| heights[a].total_cmp(&heights[b]))
                    .unwrap();
                let slope = heights[i] - heights[lowest];
                if slope > 0.006 {
                    let moved = (slope - 0.006) * 0.25;
                    delta[i] -= moved;
                    delta[lowest] += moved;
                }
            }
        }
        for (h, d) in heights.iter_mut().zip(&delta) {
            *h += d;
        }
    }
}

fn classify(heights: &[f32], width: u32, height: u32, seed: u32, terrain: &Value) -> Vec<u8> {
    let mut cells = Vec::with_capacity(heights.len());
    for y in 0..height {
        for x in 0..width {
            let elevation = heights[(y * width + x) as usize];
            let latitude = (2. * y as f32 / height as f32 - 1.).abs();
            let moisture = noise(
                x as f32 / width as f32 * 8.,
                y as f32 / height as f32 * 6.,
                seed ^ 0x456731,
                None,
            );
            let biome = if elevation < 0.40 {
                0
            } else if elevation < 0.46 {
                1
            } else if elevation < 0.475 {
                2
            } else if elevation > 0.73 || latitude > 0.94 {
                10
            } else if elevation > 0.65 {
                9
            } else if latitude > 0.80 {
                8
            } else if moisture < 0.28 {
                6
            } else if moisture > 0.73 && elevation < 0.52 {
                7
            } else if moisture > 0.58 && latitude < 0.35 {
                5
            } else if moisture > 0.47 {
                4
            } else {
                3
            };
            cells.push(biome);
        }
    }
    // Trace highland runoff to the sea without recursion or unbounded loops.
    let rivers = terrain["rivers"].as_str().unwrap_or("some");
    let stride = match rivers {
        "none" => 0,
        "many" => 13,
        _ => 29,
    };
    if stride > 0 {
        let mut visit = vec![0u32; heights.len()];
        let mut route = 0u32;
        for y in (1..height as usize - 1).step_by(stride) {
            for x in (1..width as usize - 1).step_by(stride) {
                let mut i = y * width as usize + x;
                if heights[i] < 0.60 || hash(x as i32, y as i32, seed) < 0.65 {
                    continue;
                }
                route += 1;
                for _ in 0..width as usize + height as usize {
                    if cells[i] <= 1 || visit[i] == route {
                        break;
                    }
                    visit[i] = route;
                    let (px, py) = (i % width as usize, i / width as usize);
                    if px == 0 || py == 0 || px + 1 == width as usize || py + 1 == height as usize {
                        break;
                    }
                    let neighbors = [i - 1, i + 1, i - width as usize, i + width as usize];
                    let next = *neighbors
                        .iter()
                        .min_by(|&&a, &&b| heights[a].total_cmp(&heights[b]))
                        .unwrap();
                    if heights[next] >= heights[i] {
                        if terrain["lakes"] != false {
                            cells[i] = 12;
                        }
                        break;
                    }
                    cells[i] = 12;
                    i = next;
                }
            }
        }
    }
    cells
}

fn store(
    world: &World,
    id: &str,
    width: u32,
    height: u32,
    cells: &[u8],
    heights: &[f32],
    config: &Value,
) -> Result<()> {
    let image = png_indexed(width, height, cells, &config["painter"]["biomes"])?;
    let relief = png_height(width, height, heights)?;
    world.transaction(|db| {
        World::queue_file(db, &format!("maps/{id}.png"), &image)?;
        World::queue_file(db, &format!("maps/{id}.height.png"), &relief)?;
        db.execute(
            "UPDATE map SET width=?1,height=?2,kind='painted',config=?3,updated_at=?4 WHERE id=?5",
            params![width, height, config.to_string(), now(), id],
        )?;
        Ok(())
    })
}
pub fn create(world: &World, name: &str, options: &Value) -> Result<Value> {
    let (width, height) = dimensions(options)?;
    let biomes: Value = serde_json::from_str(include_str!("../resources/biomes.json"))?;
    let shape = options
        .get("shape")
        .cloned()
        .unwrap_or_else(|| json!({"kind":"flat"}));
    let terrain = options
        .get("terrain")
        .cloned()
        .unwrap_or_else(|| json!({"erosion":"light","rivers":"some","lakes":true,"wind":"none"}));
    let seed = options["seed"]
        .as_u64()
        .unwrap_or_else(|| uuid::Uuid::new_v4().as_u128() as u64) as u32;
    let (heights, cells) = match options["fill"].as_str().unwrap_or("continents") {
        "ocean" => (
            vec![0.2; (width * height) as usize],
            vec![0; (width * height) as usize],
        ),
        "land" => (
            vec![0.55; (width * height) as usize],
            vec![3; (width * height) as usize],
        ),
        "continents" => {
            let mut h = heightfield(width, height, seed, &shape);
            erode(&mut h, width as usize, height as usize, &terrain);
            let c = classify(&h, width, height, seed, &terrain);
            (h, c)
        }
        _ => return Err("Unknown map fill".into()),
    };
    let config = json!({"painter":{"biomes":biomes,"shape":shape,"terrain":terrain,"seed":seed}});
    let png = png_indexed(width, height, &cells, &biomes)?;
    let relief = png_height(width, height, &heights)?;
    let id = world.transaction(|db| {
        let id = crate::maps::create_in(db, name)?;
        db.execute(
            "UPDATE map SET width=?1,height=?2,kind='painted',config=?3 WHERE id=?4",
            params![width, height, config.to_string(), id],
        )?;
        World::queue_file(db, &format!("maps/{id}.png"), &png)?;
        World::queue_file(db, &format!("maps/{id}.height.png"), &relief)?;
        Ok(id)
    })?;
    crate::maps::get(world, &id)
}
fn read_height(world: &World, id: &str, width: u32, height: u32) -> Result<Vec<f32>> {
    let path = world.local_file(&format!("maps/{id}.height.png"))?;
    let mut reader = png::Decoder::new(BufReader::new(File::open(path)?)).read_info()?;
    let info = reader.info();
    if info.width != width
        || info.height != height
        || info.color_type != png::ColorType::Grayscale
        || ![png::BitDepth::Eight, png::BitDepth::Sixteen].contains(&info.bit_depth)
    {
        return Err("Invalid relief image".into());
    }
    let sixteen = info.bit_depth == png::BitDepth::Sixteen;
    let mut data = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or("Relief image is too large")?
    ];
    reader.next_frame(&mut data)?;
    Ok(if sixteen {
        data.as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_be_bytes([b[0], b[1]]) as f32 / 65535.)
            .collect()
    } else {
        data.iter().map(|&b| b as f32 / 255.).collect()
    })
}
pub fn relief(world: &World, id: &str) -> Result<Vec<u8>> {
    let grid = crate::grid::read(world, id)?;
    let heights = read_height(world, id, grid.width, grid.height)?;
    Ok(heights
        .iter()
        .zip(grid.cells)
        .map(|(height, biome)| {
            if biome == 0 || biome == 1 || biome == 12 {
                0
            } else {
                ((height - 0.46).max(0.) / 0.54 * 255.)
                    .round()
                    .clamp(0., 255.) as u8
            }
        })
        .collect())
}
pub fn generate(
    world: &World,
    id: &str,
    seed: Option<u32>,
    terrain: Option<Value>,
    erosion_only: bool,
) -> Result<Vec<u8>> {
    let map = one(&world.db, "SELECT * FROM map WHERE id=?1", id)?;
    if map["kind"] != "painted" {
        return Err("This is not a painted map".into());
    }
    let (width, height) = dimensions(&json!({"cols":map["width"],"rows":map["height"]}))?;
    let mut config = map["config"].clone();
    let seed = seed.unwrap_or_else(|| {
        erosion_only
            .then(|| config["painter"]["seed"].as_u64())
            .flatten()
            .unwrap_or_else(|| uuid::Uuid::new_v4().as_u128() as u64) as u32
    });
    if let Some(terrain) = terrain {
        config["painter"]["terrain"] = terrain;
    }
    config["painter"]["seed"] = json!(seed);
    let mut heights = if erosion_only {
        read_height(world, id, width, height)?
    } else {
        heightfield(width, height, seed, &config["painter"]["shape"])
    };
    erode(
        &mut heights,
        width as usize,
        height as usize,
        &config["painter"]["terrain"],
    );
    let cells = classify(&heights, width, height, seed, &config["painter"]["terrain"]);
    store(world, id, width, height, &cells, &heights, &config)?;
    Ok(cells)
}
#[tauri::command(async)]
pub fn map_create_painted(
    state: tauri::State<crate::Session>,
    name: String,
    options: Value,
) -> crate::Reply<Value> {
    state.read(|w| create(w, &name, &options))
}
#[tauri::command(async)]
pub fn map_generate(
    state: tauri::State<crate::Session>,
    id: String,
    seed: Option<u32>,
    terrain: Option<Value>,
) -> crate::Reply<tauri::ipc::Response> {
    state.read(|w| {
        Ok(tauri::ipc::Response::new(generate(
            w, &id, seed, terrain, false,
        )?))
    })
}
#[tauri::command(async)]
pub fn map_erode(
    state: tauri::State<crate::Session>,
    id: String,
    terrain: Value,
) -> crate::Reply<tauri::ipc::Response> {
    state.read(|w| {
        Ok(tauri::ipc::Response::new(generate(
            w,
            &id,
            None,
            Some(terrain),
            true,
        )?))
    })
}
#[tauri::command(async)]
pub fn map_relief(
    state: tauri::State<crate::Session>,
    id: String,
) -> crate::Reply<tauri::ipc::Response> {
    state.read(|w| Ok(tauri::ipc::Response::new(relief(w, &id)?)))
}
#[tauri::command(async)]
pub fn map_shape_set(
    state: tauri::State<crate::Session>,
    id: String,
    shape: Value,
) -> crate::Reply<Value> {
    state.read(|w| {
        let kind = shape["kind"].as_str().ok_or("Missing world shape")?;
        if ![
            "flat", "globe", "hollow", "roche", "ring", "dice", "disc", "islands", "cylinder",
            "oneill",
        ]
        .contains(&kind)
        {
            return Err("Unknown world shape".into());
        }
        let mut map = one(&w.db, "SELECT * FROM map WHERE id=?1", &id)?;
        map["config"]["painter"]["shape"] = shape;
        w.transaction(|db| {
            db.execute(
                "UPDATE map SET config=?1,updated_at=?2 WHERE id=?3",
                params![map["config"].to_string(), now(), id],
            )?;
            Ok(())
        })?;
        crate::maps::get(w, &id)
    })
}
#[tauri::command(async)]
pub fn map_biomes_set(
    state: tauri::State<crate::Session>,
    id: String,
    biomes: Value,
) -> crate::Reply<Value> {
    state.read(|w| {
        palette(&biomes)?;
        let grid = crate::grid::read(w, &id)?;
        let mut map = one(&w.db, "SELECT * FROM map WHERE id=?1", &id)?;
        map["config"]["painter"]["biomes"] = biomes.clone();
        let image = png_indexed(grid.width, grid.height, &grid.cells, &biomes)?;
        w.transaction(|db| {
            World::queue_file(db, &format!("maps/{id}.png"), &image)?;
            db.execute(
                "UPDATE map SET config=?1,updated_at=?2 WHERE id=?3",
                params![map["config"].to_string(), now(), id],
            )?;
            Ok(())
        })?;
        crate::maps::get(w, &id)
    })
}
#[tauri::command(async)]
pub fn map_export_image(
    state: tauri::State<crate::Session>,
    id: String,
    destination: String,
    scale: Option<u32>,
) -> crate::Reply<String> {
    state.read(|w| {
        let grid = crate::grid::read(w, &id)?;
        let scale = scale.unwrap_or(1);
        if scale == 0
            || scale > 16
            || u64::from(grid.width) * u64::from(grid.height) * u64::from(scale).pow(2)
                > 100_000_000
        {
            return Err("Export image is too large".into());
        }
        let rgb: Vec<u8> = grid
            .cells
            .iter()
            .flat_map(|&index| {
                grid.palette[index as usize * 3..index as usize * 3 + 3]
                    .iter()
                    .copied()
            })
            .collect();
        let image =
            image::RgbImage::from_raw(grid.width, grid.height, rgb).ok_or("Invalid image size")?;
        let scaled = image::imageops::resize(
            &image,
            grid.width * scale,
            grid.height * scale,
            image::imageops::FilterType::Nearest,
        );
        let mut bytes = Cursor::new(Vec::new());
        scaled.write_to(&mut bytes, image::ImageFormat::Png)?;
        crate::world::write_atomic(Path::new(&destination), bytes.get_ref())?;
        Ok(destination)
    })
}
