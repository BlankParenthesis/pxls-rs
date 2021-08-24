use rusqlite::Result;
use actix_web::web::{Bytes, BytesMut, BufMut};
use std::sync::Mutex;

use crate::objects::board::{Board, BoardData, BoardInfo};
use crate::objects::color::Color;
use crate::objects::placement::Placement;

type Connection = r2d2::PooledConnection<r2d2_sqlite::SqliteConnectionManager>;

pub fn init(connection: Connection) -> Result<()> {
	connection.execute(include_str!("sql/setup.sql"), [])?;
	connection.execute(include_str!("sql/create_palette_table.sql"), [])?;
	connection.execute(include_str!("sql/create_color_table.sql"), [])?;
	connection.execute(include_str!("sql/create_board_table.sql"), [])?;
	connection.execute(include_str!("sql/create_placement_table.sql"), [])?;

	Ok(())
}

pub fn load_boards(connection: Connection) -> Result<Vec<Board>> {
	connection.prepare("SELECT `id`, `name`, `created_at`, `shape`, `palette`, `mask`, `initial` FROM `board`")?
		.query_map([], |board| {
			let board_id: usize = board.get(0)?;
			let board_name: String = board.get(1)?;
			let board_created_at: u64 = board.get(2)?;
			let board_shape_json: String = board.get(3)?;
			let board_palette_id: usize = board.get(4)?;
			let board_mask: Vec<u8> = board.get(5)?;
			let board_initial: Vec<u8> = board.get(6)?;

			let mut colors: Vec<(usize, Color)> = connection
				.prepare("SELECT `index`, `name`, `value` FROM `color` WHERE `palette` = ?1")?
				.query_map([board_palette_id], |color| Ok(
					(
						color.get::<_, usize>(0)?, 
						Color {
							name: color.get(1)?,
							value: color.get(2)?,
						}
					)
				))?
				.collect::<Result<_>>()?;

			colors.sort_by_key(|(index, _)| *index);
			let palette = colors.into_iter()
				.map(|(_, color)| color)
				.collect();

			let info = BoardInfo {
				name: board_name,
				created_at: board_created_at,
				// TODO: propagate error rather than unwrapping
				shape: serde_json::de::from_str(board_shape_json.as_str()).unwrap(),
				palette
			};

			let [width, height] = info.shape[0];
			let size = width * height;
			assert_eq!(size, board_mask.len());
			assert_eq!(size, board_initial.len());
			let mut color_data = BytesMut::from(&board_initial[..]);
			let mut timestamp_data = BytesMut::from(&vec![0; size * 4][..]);

			let placements: Vec<Placement> = connection
				.prepare(include_str!("sql/current_placements.sql"))?
				.query_map([board_id], |placement| Ok(Placement {
					position: placement.get(0)?,
					color: placement.get(1)?,
					modified: placement.get(2)?,
				}))?
				.collect::<Result<_>>()?;
			for placement in placements {
				let index = placement.position;
				color_data[index] = placement.color;
				let timestamp_slice = &mut timestamp_data[index * 4..index * 4 + 4];
				timestamp_slice.as_mut().put_u32_le(placement.modified);
			};

			let data = Mutex::new(BoardData {
				colors: color_data,
				timestamps: timestamp_data,
				mask: Bytes::from(board_mask),
				initial: Bytes::from(board_initial),
			});

			Ok(Board { info, data })
		})?
		.collect()
}