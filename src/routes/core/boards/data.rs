use crate::filters::body::patch::BinaryPatch;

use super::*;

pub fn get_colors(
	boards: BoardDataMap,
	database_pool: Arc<Pool>,
) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {

	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path("data"))
		.and(warp::path("colors"))
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::any().and(range::range()).or(range::default()).unify())
		.and(authorization::bearer().and_then(with_permission(Permission::BoardsDataGet)))
		.and(database::connection(database_pool))
		.map(|board: PassableBoard, range: Range, _user, connection| {
			// TODO: content disposition
			let board = board.read();
			let mut colors_data = board
				.as_ref()
				.unwrap()
				.read(SectorBuffer::Colors, &connection);
	
			range.respond_with(&mut colors_data)
		})
}

pub fn get_timestamps(
	boards: BoardDataMap,
	database_pool: Arc<Pool>,
) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {

	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path("data"))
		.and(warp::path("timestamps"))
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::any().and(range::range()).or(range::default()).unify())
		.and(authorization::bearer().and_then(with_permission(Permission::BoardsDataGet)))
		.and(database::connection(database_pool))
		.map(|board: PassableBoard, range: Range, _user, connection| {
			// TODO: content disposition
			let board = board.read();
			let mut timestamp_data = board
				.as_ref()
				.unwrap()
				.read(SectorBuffer::Timestamps, &connection);
	
			range.respond_with(&mut timestamp_data)
		})
}

pub fn get_mask(
	boards: BoardDataMap,
	database_pool: Arc<Pool>,
) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {

	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path("data"))
		.and(warp::path("mask"))
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::any().and(range::range()).or(range::default()).unify())
		.and(authorization::bearer().and_then(with_permission(Permission::BoardsDataGet)))
		.and(database::connection(database_pool))
		.map(|board: PassableBoard, range: Range, _user, connection| {
			// TODO: content disposition
			let board = board.read();
			let mut mask_data = board
				.as_ref()
				.unwrap()
				.read(SectorBuffer::Mask, &connection);
	
			range.respond_with(&mut mask_data)
		})
}

pub fn get_initial(
	boards: BoardDataMap,
	database_pool: Arc<Pool>,
) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {

	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path("data"))
		.and(warp::path("initial"))
		.and(warp::path::end())
		.and(warp::get())
		.and(warp::any().and(range::range()).or(range::default()).unify())
		.and(authorization::bearer().and_then(with_permission(Permission::BoardsDataGet)))
		.and(database::connection(database_pool))
		.map(|board: PassableBoard, range: Range, _user, connection| {
			// TODO: content disposition
			let board = board.read();
			let mut initial_data = board
				.as_ref()
				.unwrap()
				.read(SectorBuffer::Initial, &connection);
	
			range.respond_with(&mut initial_data)
		})
}

pub fn patch_initial(
	boards: BoardDataMap,
	database_pool: Arc<Pool>,
) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {

	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path("data"))
		.and(warp::path("initial"))
		.and(warp::path::end())
		.and(warp::patch())
		.and(authorization::bearer().and_then(with_permission(Permission::BoardsDataPatch)))
		.and(patch::bytes())
		.and(database::connection(database_pool))
		.map(|board: PassableBoard, _user, patch: BinaryPatch, connection| {
			// TODO: content disposition
			let board = board.write();
			let patch_result = board
				.as_ref()
				.unwrap()
				.try_patch_initial(&patch, &connection);

			match patch_result {
				Ok(_) => StatusCode::NO_CONTENT.into_response(),
				Err(e) => reply::with_status(e, StatusCode::CONFLICT).into_response(),
			}
		})
}

pub fn patch_mask(
	boards: BoardDataMap,
	database_pool: Arc<Pool>,
) -> impl Filter<Extract = impl Reply, Error = Rejection> + Clone {

	warp::path("boards")
		.and(board::path::read(&boards))
		.and(warp::path("data"))
		.and(warp::path("mask"))
		.and(warp::path::end())
		.and(warp::patch())
		.and(authorization::bearer().and_then(with_permission(Permission::BoardsDataPatch)))
		.and(patch::bytes())
		.and(database::connection(database_pool))
		.map(|board: PassableBoard, _user, patch: BinaryPatch, connection| {
			// TODO: content disposition
			let board = board.write();
			let patch_result = board
				.as_ref()
				.unwrap()
				.try_patch_mask(&patch, &connection);

			match patch_result {
				Ok(_) => StatusCode::NO_CONTENT.into_response(),
				Err(e) => reply::with_status(e, StatusCode::CONFLICT).into_response(),
			}
		})
}
