use crate::world::{Result, World};
use std::{fs::File, io::BufReader};

pub struct Grid {
    pub width: u32,
    pub height: u32,
    pub palette: Vec<u8>,
    transparency: Option<Vec<u8>>,
    pub cells: Vec<u8>,
}

pub fn read(world: &World, id: &str) -> Result<Grid> {
    let (width, height): (u32, u32) = world.db.query_row(
        "SELECT width,height FROM map WHERE id=? AND kind='painted'",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if id.contains(['/', '\\', ':']) {
        return Err("Invalid map id".into());
    }
    let path = world.local_file(&format!("maps/{id}.png"))?;
    let mut reader = png::Decoder::new(BufReader::new(File::open(&path)?)).read_info()?;
    let info = reader.info();
    if (info.width, info.height, info.color_type, info.bit_depth)
        != (width, height, png::ColorType::Indexed, png::BitDepth::Eight)
    {
        return Err("Map dimensions or palette encoding do not match the realm".into());
    }
    let palette = info
        .palette
        .as_deref()
        .ok_or("Missing biome palette")?
        .to_vec();
    let transparency = info.trns.as_deref().map(<[u8]>::to_vec);
    let mut cells = vec![0; reader.output_buffer_size().ok_or("Map is too large")?];
    let frame = reader.next_frame(&mut cells)?;
    cells.truncate(frame.buffer_size());
    Ok(Grid {
        width,
        height,
        palette,
        transparency,
        cells,
    })
}

pub fn save(world: &World, id: &str, cells: &[u8]) -> Result<()> {
    let grid = read(world, id)?;
    if cells.len() != grid.cells.len() {
        return Err("Map grid length does not match its dimensions".into());
    }
    if cells == grid.cells {
        return Ok(());
    }
    if cells
        .iter()
        .any(|&cell| usize::from(cell) >= grid.palette.len() / 3)
    {
        return Err("Map contains an unknown palette entry".into());
    }
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, grid.width, grid.height);
        encoder.set_color(png::ColorType::Indexed);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_palette(grid.palette);
        if let Some(alpha) = grid.transparency {
            encoder.set_trns(alpha);
        }
        let mut writer = encoder.write_header()?;
        writer.write_image_data(cells)?;
        writer.finish()?;
    }
    world.transaction(|db| {
        World::queue_file(db, &format!("maps/{id}.png"), &bytes)?;
        db.execute(
            "UPDATE map SET updated_at=?1 WHERE id=?2",
            [&crate::database::now(), id],
        )?;
        Ok(())
    })
}
