use super::*;

use fragile::Fragile;
use ouroboros::self_referencing;

// TODO: these are async locks. Fancy. Only problem is that this filter runs
// into a nasty problem when PassableBoardAsyncTryBuilder is not passable over
// an async boundary as Send. It needs to be awaited to build, so I'm stumped on
// how to manage that. If you get any ideas on how to fix that, it would be nice
// to not block threads on reading a board in the filters.

//use tokio::sync::{RwLock, RwLockReadGuard};

use std::sync::{RwLockReadGuard, RwLockWriteGuard};
use std::{
	collections::HashMap,
	sync::Arc,
};

use std::ops::Deref;

use crate::{BoardDataMap, BoardRef};
use crate::database::model::Placement;

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
			fields.lock.remove(fields.board_id)
				.expect("board went missing")
		})
	}
}

pub mod path {
	use crate::database::Connection;

use super::*;

	pub fn read(
		boards: &BoardDataMap
	) -> impl Filter<Extract = (Fragile<PassableBoard>,), Error = Rejection> + Clone {
		let boards = Arc::clone(boards);
		warp::path::param().and_then(move |id: usize| {
			let boards = Arc::clone(&boards);

			async move {
				let board = PassableBoardTryBuilder {
					boards,
					lock_builder: |boards| Ok(boards.read().unwrap()),
					board_builder: |lock| lock.get(&id).ok_or(()),
				}
				.try_build();

				// FIXME: Fragile is not sufficient here.
				// I assume that this might actually get sent across threads and
				// this will be an issue when that happens.
				// I don't know what magic ouroboros does that makes
				// PassableBoard not Send, but the next step could be declaring
				// `unsafe impl Send for PassableBoard {}`.
				// I have no idea what makes it not Send, so this is almost
				// certainly unsound as well.
				// It would be a shame if none of this works out because this
				// Filter has the power to make things much more elegant but 🤷.
				board
					.map(Fragile::new)
					.map_err(|_| warp::reject::not_found())
			}
		})
	}

	pub fn prepare_delete(
		boards: &BoardDataMap
	) -> impl Filter<Extract = (Fragile<PendingDelete>,), Error = Rejection> + Clone {
		let boards = Arc::clone(&boards);
		warp::path::param().and_then(move |board_id: usize| {
			let boards = Arc::clone(&boards);

			async move {
				let writable = PendingDeleteBuilder {
					board_id,
					boards,
					lock_builder: |boards| boards.write().unwrap(),
				}.build();

				if writable.borrow_lock().contains_key(&board_id) {
					Ok(Fragile::new(writable))
				} else {
					Err(warp::reject::not_found())
				}

			}
		})
	}
}
