use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::Statement;

mod entities;
#[cfg(feature = "migrate-ldap")]
mod ldap;

#[cfg(not(feature = "migrate-ldap"))]
mod dummy;
#[cfg(not(feature = "migrate-ldap"))]
use dummy as ldap;

use super::{col, id};

#[derive(Iden)]
enum Placement {
	Table,
	#[iden = "user_id"]
	UserId,
}

#[derive(Iden)]
enum Notice {
	Table,
	Author,
	#[iden = "new_author_id"]
	NewAuthorId,
}

#[derive(Iden)]
enum BoardNotice {
	Table,
	Author,
	#[iden = "new_author_id"]
	NewAuthorId,
}

#[derive(Iden)]
enum Report {
	Table,
	Reporter,
}

#[derive(Iden)]
enum Ban {
	Table,
	#[iden = "user_id"]
	UserId,
	Issuer,
}

#[derive(Iden)]
enum UserId {
	Table,
	Id,
	Uid,
}

#[derive(Iden)]
enum User {
	Table,
	Id,
	Subject,
	Name,
	#[iden = "created_at"]
	CreatedAt,
}

#[derive(Iden)]
enum UserTmp {
	Table,
	Subject,
	Username,
	#[iden = "created_at"]
	CreatedAt,
}

#[derive(Iden)]
enum Role {
	Table,
	Id,
	Name,
	Icon,
	Permissions,
}

#[derive(Iden)]
enum RoleMember {
	Table,
	Role,
	Member,
}

#[derive(Iden)]
enum RoleMemberTmp {
	Table,
	Role,
	Member,
}

#[derive(Iden)]
enum Faction {
	Table,
	Id,
	Name,
	Icon,
	#[iden = "created_at"]
	CreatedAt,
	#[iden = "cn_tmp"]
	CnTmp,
}

#[derive(Iden)]
enum FactionMember {
	Table,
	Faction,
	Member,
	Owner,
	Invited,
	Imposed,
}

#[derive(Iden)]
enum FactionMemberTmp {
	Table,
	Faction,
	Member,
	Owner,
}

pub struct MigrateLdapUsers;
impl MigrationName for MigrateLdapUsers {
	fn name(&self) -> &str {
		"m12_remove_ldap_users"
	}
}

#[async_trait::async_trait]
impl MigrationTrait for MigrateLdapUsers {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		let create_user_table = Table::create()
			.table(User::Table)
			.col(id!(User::Id).integer())
			.col(col!(User::Subject).string().unique_key())
			.col(ColumnDef::new(User::Name).string().null())
			.col(ColumnDef::new(User::CreatedAt).big_integer().null())
			.to_owned();

		let copy_uid_data = Query::insert()
			.into_table(User::Table)
			.columns([User::Id, User::Subject])
			.select_from(
				Query::select()
					.distinct()
					.column(UserId::Id)
					.column(UserId::Uid)
					.from(UserId::Table)
					.to_owned()
			).unwrap()
			.to_owned();
		
		let fix_user_index = Statement::from_string(
			manager.get_database_backend(),
			r#"SELECT SETVAL('user_id_seq', (SELECT MAX(id) FROM "user"));"#,
		);
		
		let drop_uid_mapping_table = Table::drop()
			.table(UserId::Table)
			.to_owned();

		let alter_placement_drop_foreign_key = Table::alter()
			.table(Placement::Table)
			.drop_foreign_key(Alias::new("placement_user_id_fkey"))
			.to_owned();

		let alter_report_drop_foreign_key = Table::alter()
			.table(Report::Table)
			.drop_foreign_key(Alias::new("report_reporter_fkey"))
			.to_owned();

		let alter_bans_drop_foreign_keys = Table::alter()
			.table(Ban::Table)
			.drop_foreign_key(Alias::new("ban_user_id_fkey"))
			.drop_foreign_key(Alias::new("ban_issuer_fkey"))
			.to_owned();
		
		let alter_notice_add_new_ids = Table::alter()
			.table(Notice::Table)
			.add_column(ColumnDef::new(Notice::NewAuthorId).null().integer())
			.add_foreign_key(
				TableForeignKey::new()
					.from_tbl(Notice::Table)
					.from_col(Notice::NewAuthorId)
					.to_tbl(User::Table)
					.to_col(User::Id)
			)
			.to_owned();
		
		let insert_notice_populate_new_ids = Query::update()
			.table(Notice::Table)
			.value(
				Notice::NewAuthorId,
				SimpleExpr::SubQuery(None, Box::new(
					Query::select()
						.column(UserId::Id)
						.from(UserId::Table)
						.and_where(
							SimpleExpr::Column(UserId::Uid.into_column_ref())
								.eq(Notice::Author.into_column_ref())
						)
						.to_owned()
						.into_sub_query_statement()
				))
			)
			.to_owned();
		
		let alter_notice_drop_old_ids = Table::alter()
			.table(Notice::Table)
			.drop_column(Notice::Author)
			.to_owned();
		
		let alter_notice_rename = Table::alter()
			.table(Notice::Table)
			.rename_column(Notice::NewAuthorId, Notice::Author)
			.to_owned();
		
		
		let alter_board_notice_add_new_ids = Table::alter()
			.table(BoardNotice::Table)
			.add_column(ColumnDef::new(BoardNotice::NewAuthorId).null().integer())
			.add_foreign_key(
				TableForeignKey::new()
					.from_tbl(BoardNotice::Table)
					.from_col(BoardNotice::NewAuthorId)
					.to_tbl(User::Table)
					.to_col(User::Id)
			)
			.to_owned();
		
		let insert_board_notice_populate_new_ids = Query::update()
			.table(BoardNotice::Table)
			.value(
				BoardNotice::NewAuthorId,
				SimpleExpr::SubQuery(None, Box::new(
					Query::select()
						.column(UserId::Id)
						.from(UserId::Table)
						.and_where(
							SimpleExpr::Column(UserId::Uid.into_column_ref())
								.eq(BoardNotice::Author.into_column_ref())
						)
						.to_owned()
						.into_sub_query_statement()
				))
			)
			.to_owned();
		
		let alter_board_notice_drop_old_ids = Table::alter()
			.table(BoardNotice::Table)
			.drop_column(BoardNotice::Author)
			.to_owned();
		
		let alter_board_notice_rename = Table::alter()
			.table(BoardNotice::Table)
			.rename_column(BoardNotice::NewAuthorId, BoardNotice::Author)
			.to_owned();
		
		
		let alter_placement_add_foreign_key = Table::alter()
			.table(Placement::Table)
			.add_foreign_key(
				TableForeignKey::new()
					.from_tbl(Placement::Table)
					.from_col(Placement::UserId)
					.to_tbl(User::Table)
					.to_col(User::Id)
			)
			.to_owned();
		
		let alter_report_add_foreign_key = Table::alter()
			.table(Report::Table)
			.add_foreign_key(
				TableForeignKey::new()
					.from_tbl(Report::Table)
					.from_col(Report::Reporter)
					.to_tbl(User::Table)
					.to_col(User::Id)
			)
			.to_owned();
		
		let alter_bans_add_foreign_keys = Table::alter()
			.table(Ban::Table)
			.add_foreign_key(
				TableForeignKey::new()
					.from_tbl(Ban::Table)
					.from_col(Ban::UserId)
					.to_tbl(User::Table)
					.to_col(User::Id)
			)
			.add_foreign_key(
				TableForeignKey::new()
					.from_tbl(Ban::Table)
					.from_col(Ban::Issuer)
					.to_tbl(User::Table)
					.to_col(User::Id)
			)
			.to_owned();
		
		manager.create_table(create_user_table).await?;
		manager.exec_stmt(copy_uid_data).await?;
		manager.get_connection().execute(fix_user_index).await?;
		
		manager.alter_table(alter_notice_add_new_ids).await?;
		manager.exec_stmt(insert_notice_populate_new_ids).await?;
		manager.alter_table(alter_notice_drop_old_ids).await?;
		manager.alter_table(alter_notice_rename).await?;
		
		manager.alter_table(alter_board_notice_add_new_ids).await?;
		manager.exec_stmt(insert_board_notice_populate_new_ids).await?;
		manager.alter_table(alter_board_notice_drop_old_ids).await?;
		manager.alter_table(alter_board_notice_rename).await?;

		manager.alter_table(alter_placement_drop_foreign_key).await?;
		manager.alter_table(alter_report_drop_foreign_key).await?;
		manager.alter_table(alter_bans_drop_foreign_keys).await?;
		
		manager.alter_table(alter_placement_add_foreign_key).await?;
		manager.alter_table(alter_report_add_foreign_key).await?;
		manager.alter_table(alter_bans_add_foreign_keys).await?;

		manager.drop_table(drop_uid_mapping_table).await?;
		
		let mut ldap = ldap::Connection::new().await
			.expect("Failed to connect to ldap (required to migrate data)");
		let users = ldap.load_users().await
			.expect("Failed to load users from ldap for migration");
		
		// TODO: create a table, bulk insert users, copy over to users table
		let create_tmp_table = Table::create()
			.table(UserTmp::Table)
			.col(col!(UserTmp::Subject).string().unique_key())
			.col(col!(UserTmp::Username).string())
			.col(col!(UserTmp::CreatedAt).big_integer())
			.to_owned();
		
		let mut insert_user_ldap_data = Query::insert();
		
		insert_user_ldap_data.into_table(UserTmp::Table)
			.columns([UserTmp::Subject, UserTmp::Username, UserTmp::CreatedAt]);
		
		let has_users = !users.is_empty();
		
		for entities::User { id, name, created_at } in users {
			insert_user_ldap_data
				.values([id.into(), name.into(), created_at.into()])
				.unwrap();
		}
		
		let copy_ldap_data = Query::update()
			.table(User::Table)
			.value(User::Name, Expr::col((UserTmp::Table, UserTmp::Username)))
			.value(User::CreatedAt, Expr::col((UserTmp::Table, UserTmp::CreatedAt)))
			.from(UserTmp::Table)
			.cond_where(Expr::col((User::Table, User::Subject))
				.eq(Expr::col((UserTmp::Table, UserTmp::Subject))))
			.to_owned();
		
		let delete_rogue_data = Query::delete()
			.from_table(User::Table)
			.cond_where(Expr::col((User::Table, User::Name)).is_null())
			.to_owned();
		
		let drop_tmp_table = Table::drop()
			.table(UserTmp::Table)
			.to_owned();
		
		manager.create_table(create_tmp_table).await?;
		if has_users {
			manager.exec_stmt(insert_user_ldap_data).await?;
		}
		manager.exec_stmt(copy_ldap_data).await?;
		manager.exec_stmt(delete_rogue_data).await?;
		manager.drop_table(drop_tmp_table).await?;
		
		let make_user_columns_not_null = Table::alter()
			.table(User::Table)
			.modify_column(col!(User::Name).string())
			.modify_column(col!(User::CreatedAt).big_integer())
			.to_owned();
		
		manager.alter_table(make_user_columns_not_null).await?;

		Ok(())
	}

	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {

		let create_uid_mapping_table = Table::create()
			.table(UserId::Table)
			.col(id!(UserId::Id).integer())
			.col(col!(UserId::Uid).string().unique_key())
			.to_owned();
		
		let copy_uid_data = Query::insert()
			.into_table(UserId::Table)
			.columns([UserId::Id, UserId::Uid])
			.select_from(
				Query::select()
					.distinct()
					.column(User::Id)
					.column(User::Subject)
					.from(User::Table)
					.to_owned()
			).unwrap()
			.to_owned();
		
		let drop_user_table = Table::drop()
			.table(User::Table)
			.to_owned();
		
		let alter_placement_drop_foreign_key = Table::alter()
			.table(Placement::Table)
			.drop_foreign_key(Alias::new("placement_user_id_fkey"))
			.to_owned();
		
		let alter_report_drop_foreign_key = Table::alter()
			.table(Report::Table)
			.drop_foreign_key(Alias::new("report_reporter_fkey"))
			.to_owned();
		
		let alter_bans_drop_foreign_keys = Table::alter()
			.table(Ban::Table)
			.drop_foreign_key(Alias::new("ban_user_id_fkey"))
			.drop_foreign_key(Alias::new("ban_issuer_fkey"))
			.to_owned();
		
		
		let alter_notice_add_new_ids = Table::alter()
			.table(Notice::Table)
			.add_column(ColumnDef::new(Notice::NewAuthorId).null().string())
			.to_owned();
		
		let insert_notice_populate_new_ids = Query::update()
			.table(Notice::Table)
			.value(
				Notice::NewAuthorId,
				SimpleExpr::SubQuery(None, Box::new(
					Query::select()
						.column(UserId::Uid)
						.from(UserId::Table)
						.and_where(
							SimpleExpr::Column(UserId::Id.into_column_ref())
								.eq(Notice::Author.into_column_ref())
						)
						.to_owned()
						.into_sub_query_statement()
				))
			)
			.to_owned();
		
		let alter_notice_drop_old_ids = Table::alter()
			.table(Notice::Table)
			.drop_column(Notice::Author)
			.to_owned();
		
		let alter_notice_rename = Table::alter()
			.table(Notice::Table)
			.rename_column(Notice::NewAuthorId, Notice::Author)
			.to_owned();
		
		
		let alter_board_notice_add_new_ids = Table::alter()
			.table(BoardNotice::Table)
			.add_column(ColumnDef::new(BoardNotice::NewAuthorId).null().string())
			.to_owned();
		
		let insert_board_notice_populate_new_ids = Query::update()
			.table(BoardNotice::Table)
			.value(
				BoardNotice::NewAuthorId,
				SimpleExpr::SubQuery(None, Box::new(
					Query::select()
						.column(UserId::Uid)
						.from(UserId::Table)
						.and_where(
							SimpleExpr::Column(UserId::Id.into_column_ref())
								.eq(BoardNotice::Author.into_column_ref())
						)
						.to_owned()
						.into_sub_query_statement()
				))
			)
			.to_owned();
		
		let alter_board_notice_drop_old_ids = Table::alter()
			.table(BoardNotice::Table)
			.drop_column(BoardNotice::Author)
			.to_owned();
		
		let alter_board_notice_rename = Table::alter()
			.table(BoardNotice::Table)
			.rename_column(BoardNotice::NewAuthorId, BoardNotice::Author)
			.to_owned();
		
		
		let alter_placement_add_foreign_key = Table::alter()
			.table(Placement::Table)
			.add_foreign_key(
				TableForeignKey::new()
					.from_tbl(Placement::Table)
					.from_col(Placement::UserId)
					.to_tbl(UserId::Table)
					.to_col(UserId::Id)
			)
			.to_owned();
		
		let alter_report_add_foreign_key = Table::alter()
			.table(Report::Table)
			.add_foreign_key(
				TableForeignKey::new()
					.from_tbl(Report::Table)
					.from_col(Report::Reporter)
					.to_tbl(UserId::Table)
					.to_col(UserId::Id)
			)
			.to_owned();
		
		let alter_bans_add_foreign_keys = Table::alter()
			.table(Ban::Table)
			.add_foreign_key(
				TableForeignKey::new()
					.from_tbl(Ban::Table)
					.from_col(Ban::UserId)
					.to_tbl(UserId::Table)
					.to_col(UserId::Id)
			)
			.add_foreign_key(
				TableForeignKey::new()
					.from_tbl(Ban::Table)
					.from_col(Ban::Issuer)
					.to_tbl(UserId::Table)
					.to_col(UserId::Id)
			)
			.to_owned();
		
		manager.create_table(create_uid_mapping_table).await?;
		manager.exec_stmt(copy_uid_data).await?;

		manager.alter_table(alter_placement_drop_foreign_key).await?;
		manager.alter_table(alter_report_drop_foreign_key).await?;
		manager.alter_table(alter_bans_drop_foreign_keys).await?;
		
		manager.alter_table(alter_notice_add_new_ids).await?;
		manager.exec_stmt(insert_notice_populate_new_ids).await?;
		manager.alter_table(alter_notice_drop_old_ids).await?;
		manager.alter_table(alter_notice_rename).await?;
		
		manager.alter_table(alter_board_notice_add_new_ids).await?;
		manager.exec_stmt(insert_board_notice_populate_new_ids).await?;
		manager.alter_table(alter_board_notice_drop_old_ids).await?;
		manager.alter_table(alter_board_notice_rename).await?;

		manager.alter_table(alter_placement_add_foreign_key).await?;
		manager.alter_table(alter_report_add_foreign_key).await?;
		manager.alter_table(alter_bans_add_foreign_keys).await?;

		manager.drop_table(drop_user_table).await?;
		
		Ok(())
	}
}


pub struct MigrateLdapRoles;
impl MigrationName for MigrateLdapRoles {
	fn name(&self) -> &str {
		"m12_remove_ldap_roles"
	}
}

#[async_trait::async_trait]
impl MigrationTrait for MigrateLdapRoles {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		
		let create_role_table = Table::create()
			.table(Role::Table)
			.col(id!(Role::Id).integer())
			.col(col!(Role::Name).string().unique_key())
			.col(ColumnDef::new(Role::Icon).string().null())
			.col(col!(Role::Permissions).string())
			.to_owned();
		
		let create_role_member_table = Table::create()
			.table(RoleMember::Table)
			.col(col!(RoleMember::Role).integer())
			.col(col!(RoleMember::Member).integer())
			.primary_key(Index::create().col(RoleMember::Role).col(RoleMember::Member))
			.foreign_key(
				ForeignKey::create()
					.from_col(RoleMember::Role)
					.to_tbl(Role::Table)
					.to_col(Role::Id))
			.foreign_key(
				ForeignKey::create()
					.from_col(RoleMember::Member)
					.to_tbl(User::Table)
					.to_col(User::Id))
			.to_owned();
		
		let create_role_member_tmp_table = Table::create()
			.table(RoleMemberTmp::Table)
			.col(col!(RoleMemberTmp::Role).string())
			.col(col!(RoleMemberTmp::Member).string())
			.to_owned();
		
		let mut ldap = ldap::Connection::new().await
			.expect("Failed to connect to ldap (required to migrate data)");
		let roles = ldap.load_roles().await
			.expect("Failed to load roles from ldap for migration");
		
		let has_roles = !roles.is_empty();
		
		let mut insert_role_ldap_data = Query::insert();
		
		insert_role_ldap_data.into_table(Role::Table)
			.columns([Role::Name, Role::Icon, Role::Permissions]);
		
		for entities::Role { name, icon, permissions } in roles {
			insert_role_ldap_data
				.values([
					name.into(),
					icon.map(String::from).into(),
					permissions.join(",").into()
				])
				.unwrap();
		}
		
		let role_members = ldap.load_role_members().await
			.expect("Failed to load role members from ldap for migration");
		
		let has_role_members = !role_members.is_empty();
		
		let mut insert_role_member_tmp_ldap_data = Query::insert();
		
		insert_role_member_tmp_ldap_data.into_table(RoleMemberTmp::Table)
			.columns([RoleMemberTmp::Role, RoleMemberTmp::Member]);
		
		for entities::RoleMembers { role, users } in role_members {
			for user in users {
				insert_role_member_tmp_ldap_data
					.values([role.clone().into(), user.into()])
					.unwrap();
			}
		}
		
		let transfer_role_member_ldap_data = Query::insert()
			.into_table(RoleMember::Table)
			.columns([RoleMember::Role, RoleMember::Member])
			.select_from(
				Query::select()
					.column((Role::Table, Role::Id))
					.column((User::Table, User::Id))
					.from(RoleMemberTmp::Table)
					.join(
						JoinType::LeftJoin,
						Role::Table,
						Expr::col((Role::Table, Role::Name))
							.eq(Expr::col((RoleMemberTmp::Table, RoleMemberTmp::Role)))
					)
					.join(
						JoinType::LeftJoin,
						User::Table,
						Expr::col((User::Table, User::Subject))
							.eq(Expr::col((RoleMemberTmp::Table, RoleMemberTmp::Member)))
					)
					.to_owned()
			)
			.unwrap()
			.to_owned();
		
		let drop_role_member_tmp_table = Table::drop()
			.table(RoleMemberTmp::Table)
			.to_owned();
		
		manager.create_table(create_role_table).await?;
		manager.create_table(create_role_member_table).await?;
		manager.create_table(create_role_member_tmp_table).await?;
		if has_roles {
			manager.exec_stmt(insert_role_ldap_data).await?;
		}
		if has_role_members {
			manager.exec_stmt(insert_role_member_tmp_ldap_data).await?;
		}
		manager.exec_stmt(transfer_role_member_ldap_data).await?;
		manager.drop_table(drop_role_member_tmp_table).await?;
		
		Ok(())
	}
	
	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		
		let drop_role_member_table = Table::drop()
			.table(RoleMember::Table)
			.to_owned();
		
		let drop_role_table = Table::drop()
			.table(Role::Table)
			.to_owned();
		
		manager.drop_table(drop_role_member_table).await?;
		manager.drop_table(drop_role_table).await?;
		
		Ok(())
	}
}


pub struct MigrateLdapFactions;
impl MigrationName for MigrateLdapFactions {
	fn name(&self) -> &str {
		"m12_remove_ldap_factions"
	}
}

#[async_trait::async_trait]
impl MigrationTrait for MigrateLdapFactions {
	async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		
		let create_faction_table = Table::create()
			.table(Faction::Table)
			.col(id!(Faction::Id).integer())
			.col(col!(Faction::Name).string().unique_key())
			.col(ColumnDef::new(Faction::Icon).string().null())
			.col(col!(Faction::CreatedAt).big_integer())
			.col(col!(Faction::CnTmp).string())
			.to_owned();
		
		let create_faction_member_table = Table::create()
			.table(FactionMember::Table)
			.col(col!(FactionMember::Faction).integer())
			.col(col!(FactionMember::Member).integer())
			.col(col!(FactionMember::Owner).boolean())
			.col(col!(FactionMember::Imposed).boolean().default(true))
			.col(col!(FactionMember::Invited).boolean().default(true))
			.primary_key(Index::create().col(FactionMember::Faction).col(FactionMember::Member))
			.foreign_key(
				ForeignKey::create()
					.from_col(FactionMember::Faction)
					.to_tbl(Faction::Table)
					.to_col(Faction::Id))
			.foreign_key(
				ForeignKey::create()
					.from_col(FactionMember::Member)
					.to_tbl(User::Table)
					.to_col(User::Id))
			.to_owned();
		
		let create_faction_member_table_tmp = Table::create()
			.table(FactionMemberTmp::Table)
			.col(col!(FactionMemberTmp::Faction).string())
			.col(col!(FactionMemberTmp::Member).string())
			.col(col!(FactionMemberTmp::Owner).boolean())
			.to_owned();
		
		let mut ldap = ldap::Connection::new().await
			.expect("Failed to connect to ldap (required to migrate data)");
		let factions = ldap.load_factions().await
			.expect("Failed to load factions from ldap for migration");
		
		let has_factions = !factions.is_empty();
		
		let mut insert_faction_ldap_data = Query::insert();
		
		insert_faction_ldap_data.into_table(Faction::Table)
			.columns([
				Faction::Name,
				Faction::Icon,
				Faction::CreatedAt,
				Faction::CnTmp,
			]);
		
		for entities::Faction { cn, name, icon, created_at } in factions {
			insert_faction_ldap_data
				.values([
					name.into(),
					icon.map(String::from).into(),
					created_at.into(),
					cn.into(),
				])
				.unwrap();
		}
		
		let faction_members = ldap.load_faction_members().await
			.expect("Failed to load faction members from ldap for migration");
		
		let has_faction_members = !faction_members.is_empty();
		
		let mut insert_faction_member_tmp_ldap_data = Query::insert();
		
		insert_faction_member_tmp_ldap_data.into_table(FactionMemberTmp::Table)
			.columns([
				FactionMemberTmp::Faction,
				FactionMemberTmp::Member,
				FactionMemberTmp::Owner,
			]);
		
		for entities::FactionMembers { faction, users } in faction_members {
			for entities::FactionMember { user, owner } in users {
				insert_faction_member_tmp_ldap_data
					.values([faction.clone().into(), user.into(), owner.into()])
					.unwrap();
			}
		}
		
		let transfer_faction_member_ldap_data = Query::insert()
			.into_table(FactionMember::Table)
			.columns([
				FactionMember::Faction,
				FactionMember::Member,
				FactionMember::Owner,
			])
			.select_from(
				Query::select()
					.column((Faction::Table, Faction::Id))
					.column((User::Table, User::Id))
					.column((FactionMemberTmp::Table, FactionMemberTmp::Owner))
					.from(FactionMemberTmp::Table)
					.join(
						JoinType::LeftJoin,
						Faction::Table,
						Expr::col((Faction::Table, Faction::CnTmp))
							.eq(Expr::col((FactionMemberTmp::Table, FactionMemberTmp::Faction)))
					)
					.join(
						JoinType::LeftJoin,
						User::Table,
						Expr::col((User::Table, User::Subject))
							.eq(Expr::col((FactionMemberTmp::Table, FactionMemberTmp::Member)))
					)
					.to_owned()
			)
			.unwrap()
			.to_owned();
		
		let drop_faction_member_tmp_table = Table::drop()
			.table(FactionMemberTmp::Table)
			.to_owned();
		
		let alter_faction_remove_cd_tmp = Table::alter()
			.table(Faction::Table)
			.drop_column(Faction::CnTmp)
			.to_owned();
		
		let alter_faction_member_remove_defaults = Table::alter()
			.table(FactionMember::Table)
			.modify_column(col!(FactionMember::Imposed).boolean().default(None::<String>))
			.modify_column(col!(FactionMember::Invited).boolean().default(None::<String>))
			.to_owned();
		
		manager.create_table(create_faction_table).await?;
		manager.create_table(create_faction_member_table).await?;
		manager.create_table(create_faction_member_table_tmp).await?;
		if has_factions {
			manager.exec_stmt(insert_faction_ldap_data).await?;
		}
		if has_faction_members {
			manager.exec_stmt(insert_faction_member_tmp_ldap_data).await?;
		}
		manager.exec_stmt(transfer_faction_member_ldap_data).await?;
		manager.drop_table(drop_faction_member_tmp_table).await?;
		manager.exec_stmt(alter_faction_remove_cd_tmp).await?;
		manager.exec_stmt(alter_faction_member_remove_defaults).await?;
		
		Ok(())
	}
	
	async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
		
		let drop_faction_table = Table::drop()
			.table(Faction::Table)
			.to_owned();
		
		let drop_faction_member_table = Table::drop()
			.table(FactionMember::Table)
			.to_owned();
		
		manager.drop_table(drop_faction_member_table).await?;
		manager.drop_table(drop_faction_table).await?;
		
		Ok(())
	}
}
