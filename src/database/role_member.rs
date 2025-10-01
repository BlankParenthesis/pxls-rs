use sea_orm::TryInsertResult;
use sea_orm::{ConnectionTrait, EntityTrait, Set, StreamTrait, TransactionTrait};

use crate::filter::response::reference::Reference;

use super::entities::*;
use super::user::{User, UserSpecifier};
use super::role::{Role, RoleSpecifier};
use super::{Connection, DbResult, DbInsertResult, InsertError};

#[derive(Debug)]
pub struct RoleMember {
	user: Reference<User>,
	role: Reference<Role>,
}

impl<C: TransactionTrait + ConnectionTrait + StreamTrait> Connection<C> {
	pub async fn create_role_member(
		&self,
		user: &UserSpecifier,
		role: &RoleSpecifier,
	) -> DbInsertResult<RoleMember> {
		let role_member = role_member::ActiveModel {
			role: Set(role.0),
			member: Set(user.0),
		};
		
		let transaction = self.begin().await?;
		
		let user = transaction.get_user(user).await?
			.map(Reference::from)
			.ok_or(InsertError::MissingDependency)?;
		
		let role = transaction.get_role(role).await?
			.map(Reference::from)
			.ok_or(InsertError::MissingDependency)?;
		
		let insert = role_member::Entity::insert(role_member)
			.on_conflict_do_nothing()
			.exec(&transaction.connection).await?;
		
		transaction.commit().await?;
		
		match insert {
			TryInsertResult::Inserted(_) => Ok(RoleMember { user, role }),
			TryInsertResult::Empty => Err(InsertError::MissingDependency),
			TryInsertResult::Conflicted => Err(InsertError::AlreadyExists),
		}
	}
	
	pub async fn delete_role_member(
		&self,
		user: &UserSpecifier,
		role: &RoleSpecifier,
	) -> DbResult<bool> {
		let role_member = role_member::ActiveModel {
			role: Set(role.0),
			member: Set(user.0),
		};
		
		let delete = role_member::Entity::delete(role_member)
			.exec(&self.connection).await?;
		
		Ok(delete.rows_affected > 0)
	}
}
