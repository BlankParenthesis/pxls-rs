use std::{collections::HashMap, ops::Deref, sync::Arc};

use fragile::Fragile;
use ouroboros::self_referencing;
use parking_lot::{RwLockReadGuard, RwLockWriteGuard};

use super::*;
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
				let board = PassableBoardTryBuilder {
					boards,
					lock_builder: |boards| Ok(boards.read()),
					board_builder: |lock| lock.get(&id).ok_or(()),
				}
				.try_build();

				board.map_err(|_| warp::reject::not_found())
			}
		})
	}

	pub fn prepare_delete(
		boards: &BoardDataMap
	) -> impl Filter<Extract = (Fragile<PendingDelete>,), Error = Rejection> + Clone {
		let boards = Arc::clone(boards);
		warp::path::param().and_then(move |board_id: usize| {
			let boards = Arc::clone(&boards);

			async move {
				let writable = PendingDeleteBuilder {
					board_id,
					boards,
					lock_builder: |boards| boards.write(),
				}
				.build();

				if writable
					.borrow_lock()
					.contains_key(&board_id)
				{
					Ok(Fragile::new(writable))
				} else {
					Err(warp::reject::not_found())
				}
			}
		})
	}
}
