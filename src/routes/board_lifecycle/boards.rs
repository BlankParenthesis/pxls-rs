use std::sync::Arc;

use serde::Deserialize;
use tokio::sync::RwLock;

use warp::http::StatusCode;
use warp::reject::Rejection;
use warp::reply::Reply;
use warp::Filter;

use crate::BoardDataMap;
use crate::filter::header::authorization::authorized;
use crate::permissions::Permission;
use crate::routes::core::{EventPacket, Connections};
use crate::database::{BoardListSpecifier, BoardSpecifier, Database, DbConn, Palette, Specifier};

#[derive(Deserialize, Debug)]
pub struct BoardInfoPost {
	pub name: String,
	pub shape: Vec<Vec<usize>>,
	pub palette: Palette,
	pub max_pixels_available: u32,
}

pub fn post(
	events_sockets: Arc<RwLock<Connections>>,
	boards: BoardDataMap,
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	BoardListSpecifier::path()
		.and(warp::post())
		.and(warp::body::json())
		.and(authorized(db.clone(), Permission::BoardsPost.into()))
		.then(move |_, data: BoardInfoPost, _, connection: DbConn| {
			let boards = Arc::clone(&boards);
			let events_sockets = Arc::clone(&events_sockets);
			let db = Arc::clone(&db);
			async move {
				let board = connection.create_board(
					data.name,
					data.shape,
					data.palette,
					data.max_pixels_available,
					db,
				).await?;

				let id = board.info.id;

				let mut boards = boards.write().await;
				boards.insert(id, Arc::new(RwLock::new(Some(board))));

				let board = boards.get(&id).expect("Board went missing from list during creation")
					.read().await;
				let board = board.as_ref().expect("Board went missing during creation");

				let reference = board.reference();
				let packet = EventPacket::BoardCreated {
					board: reference.clone(),
				};
				events_sockets.read().await.send(&packet).await;

				Ok::<_, StatusCode>(reference.created())
			}
		})
}

#[derive(Deserialize, Debug)]
pub struct BoardInfoPatch {
	pub name: Option<String>,
	pub shape: Option<Vec<Vec<usize>>>,
	pub palette: Option<Palette>,
	pub max_pixels_available: Option<u32>,
}

pub fn patch(
	boards: BoardDataMap,
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	BoardSpecifier::path()
		.and(warp::patch())
		// TODO: require application/merge-patch+json type?
		.and(warp::body::json())
		.and(authorized(db, Permission::BoardsPatch.into()))
		.then(move |board: BoardSpecifier, patch: BoardInfoPatch, _, connection: DbConn| {
			let boards = boards.clone();
			async move {
				let boards = boards.read().await;
				let board = boards.get(&board).ok_or(StatusCode::NOT_FOUND)?;
				let mut board = board.write().await;
				let board = board.as_mut()
					.expect("Board went missing when patching");

				board.update_info(
					patch.name,
					patch.shape,
					patch.palette,
					patch.max_pixels_available,
					&connection,
				).await?;
				
				// TODO: is "created" correct?
				Ok::<_, StatusCode>(board.reference().created())
			}
		})
}

pub fn delete(
	events_sockets: Arc<RwLock<Connections>>,
	boards: BoardDataMap,
	db: Arc<Database>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	BoardSpecifier::path()
		.and(warp::delete())
		.and(authorized(db, Permission::BoardsDelete.into()))
		.then(move |board: BoardSpecifier, _, connection: DbConn| {
			let boards = boards.clone();
			let events_sockets = events_sockets.clone();
			async move {
				let mut boards = boards.write().await;
				let board = boards.remove(&board).ok_or(StatusCode::NOT_FOUND)?;
				let mut board = board.write().await;
				let board = board.take()
					.expect("Board went missing during deletion");

				let packet = EventPacket::BoardDeleted {
					board: board.info.id,
				};
				events_sockets.read().await.send(&packet).await;
				
				board.delete(&connection).await
					.map(|_| StatusCode::NO_CONTENT)
					.map_err(StatusCode::from)
			}
		})
}
