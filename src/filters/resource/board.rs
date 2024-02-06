use std::{collections::HashMap, ops::Deref, sync::Arc};

use ouroboros::self_referencing;
use tokio::sync::{RwLockReadGuard, RwLockWriteGuard};

use warp::{Filter, Rejection};
use crate::{BoardDataMap, BoardRef};

#[self_referencing]
pub struct PassableBoard {
	boards: BoardDataMap,
	#[covariant]
	#[borrows(boards)]
	lock: RwLockReadGuard<'this, HashMap<usize, BoardRef>>,
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
	board_id: usize,
	boards: BoardDataMap,
	#[covariant]
	#[borrows(boards)]
	lock: RwLockWriteGuard<'this, HashMap<usize, BoardRef>>,
}

impl PendingDelete {
	pub fn perform(&mut self) -> BoardRef {
		self.with_mut(|fields| {
			fields
				.lock
				.remove(fields.board_id)
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
		warp::path::param().and_then(move |id: usize| {
			let boards = Arc::clone(&boards);

			async move {
				PassableBoardAsyncSendTryBuilder {
					boards,
					lock_builder: |boards| Box::pin(async move { Ok(boards.read().await) }),
					board_builder: |lock| Box::pin(async move { lock.get(&id).ok_or(()) }),
				}.try_build().await
				.map_err(|_| warp::reject::not_found())
			}
		})
	}

	pub fn prepare_delete(
		boards: &BoardDataMap
	) -> impl Filter<Extract = (PendingDelete,), Error = Rejection> + Clone {
		let boards = Arc::clone(boards);
		warp::path::param().and_then(move |board_id: usize| {
			let boards = Arc::clone(&boards);

			async move {
				let writable = Box::pin(PendingDeleteAsyncSendBuilder {
					board_id,
					boards,
					lock_builder: |boards| Box::pin(async move { boards.write().await }),
				}.build()).await;

				if writable.borrow_lock().contains_key(&board_id) {
					Ok(writable)
				} else {
					Err(warp::reject::not_found())
				}
			}
		})
	}
}
