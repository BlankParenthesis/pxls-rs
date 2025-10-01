use std::{collections::HashMap, ops::Deref, sync::Arc};

use ouroboros::self_referencing;
use tokio::sync::{RwLockReadGuard, RwLockWriteGuard};

use warp::{Filter, Rejection};
use crate::database::{BoardSpecifier, Specifier};
use crate::{BoardDataMap, BoardRef};

#[self_referencing]
pub struct PassableBoard {
	boards: BoardDataMap,
	#[covariant]
	#[borrows(boards)]
	lock: RwLockReadGuard<'this, HashMap<BoardSpecifier, BoardRef>>,
	#[borrows(lock)]
	board: &'this BoardRef,
}

impl Deref for PassableBoard {
	type Target = BoardRef;

	fn deref(&self) -> &Self::Target {
		self.borrow_board()
	}
}

#[self_referencing]
pub struct PendingDelete {
	board: BoardSpecifier,
	boards: BoardDataMap,
	#[covariant]
	#[borrows(boards)]
	lock: RwLockWriteGuard<'this, HashMap<BoardSpecifier, BoardRef>>,
}

impl PendingDelete {
	pub fn perform(&mut self) -> BoardRef {
		self.with_mut(|fields| {
			fields
				.lock
				.remove(fields.board)
				.expect("board went missing")
		})
	}
}

pub mod path {
	use super::*;

	pub fn read(
		boards: &BoardDataMap
	) -> impl Filter<Extract = (PassableBoard,), Error = Rejection> + Clone {
		let boards = Arc::clone(boards);
		warp::path::param().and_then(move |board: i32| {
			let specifier = BoardSpecifier(board);
			let boards = Arc::clone(&boards);

			async move {
				PassableBoardAsyncSendTryBuilder {
					boards,
					lock_builder: |boards| Box::pin(async move { Ok(boards.read().await) }),
					board_builder: |lock| Box::pin(async move { lock.get(&specifier).ok_or(()) }),
				}.try_build().await
				.map_err(|_| warp::reject::not_found())
			}
		})
	}

	pub fn prepare_delete(
		boards: &BoardDataMap
	) -> impl Filter<Extract = (PendingDelete,), Error = Rejection> + Clone {
		let boards = Arc::clone(boards);
		BoardSpecifier::path().and_then(move |board: BoardSpecifier| {
			let boards = Arc::clone(&boards);

			async move {
				let writable = Box::pin(PendingDeleteAsyncSendBuilder {
					board,
					boards,
					lock_builder: |boards| Box::pin(async move { boards.write().await }),
				}.build()).await;

				if writable.borrow_lock().contains_key(&board) {
					Ok(writable)
				} else {
					Err(warp::reject::not_found())
				}
			}
		})
	}
}
