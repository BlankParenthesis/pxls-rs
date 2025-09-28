use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use enum_map::{EnumMap, Enum};
use enumset::{EnumSet, EnumSetType};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde::de::Visitor;
use tokio::sync::RwLock;
use warp::reject::Reject;
use warp::ws::Ws;
use warp::{Reply, Rejection};
use warp::Filter;
use warp::http::Uri;

use crate::board::Board;
use crate::database::{Ban, BoardsDatabase, Database, DatabaseError, Faction, FactionMember, Role, User, UserSpecifier};
use crate::filter::response::reference::Reference;
use crate::permissions::Permission;
use crate::routes::placement_statistics::users::UserStats;
use crate::routes::reports::reports::Report;
use crate::routes::site_notices::notices::Notice;
use crate::socket::ServerPacket;

type Socket = crate::socket::Socket<Subscription>;

// TODO: move this elsewhere
#[derive(Default)]
pub struct Connections {
	by_uid: HashMap<Option<UserSpecifier>, HashSet<Arc<Socket>>>,
	by_subscription: EnumMap<Subscription, HashSet<Arc<Socket>>>,
}

#[allow(clippy::mutable_key_type)]
impl Connections {
	async fn insert_socket(&mut self, socket: Arc<Socket>) {
		let user = socket.user().await;
		let sockets = self.by_uid.entry(user).or_default();
		sockets.insert(socket.clone());

		for subscription in socket.subscriptions {
			self.by_subscription[subscription].insert(socket.clone());
		}
	}

	async fn remove_socket(&mut self, socket: &Arc<Socket>) {
		let user = socket.user().await;
		let sockets = self.by_uid.entry(user).or_default();
		sockets.remove(socket);
		if sockets.is_empty() {
			self.by_uid.remove(&user);
		}

		for subscription in socket.subscriptions {
			self.by_subscription[subscription].remove(socket);
		}
	}

	pub async fn send<'l>(&self, packet: &EventPacket<'l>) {
		let empty_set = HashSet::default();

		match packet {
			EventPacket::AccessUpdate { user, .. } => {
				let sockets = self.by_uid.get(user)
					.unwrap_or(&empty_set);
				
				let serialized = packet.serialize_packet();
				
				for socket in sockets {
					if socket.subscriptions.contains(Subscription::Access) {
						socket.send(&serialized).await;
					}
				}
			},
			EventPacket::BoardCreated { .. } |
			EventPacket::BoardDeleted { .. } => {
				let serialized = packet.serialize_packet();
				for socket in self.by_subscription[Subscription::Boards].iter() {
					socket.send(&serialized).await
				}
			},
			EventPacket::RoleCreated { .. } |
			EventPacket::RoleUpdated { .. } |
			EventPacket::RoleDeleted { .. } => {
				let serialized = packet.serialize_packet();
				for socket in self.by_subscription[Subscription::Roles].iter() {
					socket.send(&serialized).await
				}
			},
			EventPacket::UserRolesUpdated { specifier, .. } => {
				let serialized = packet.serialize_packet();
				for socket in self.by_subscription[Subscription::UsersRoles].iter() {
					socket.send(&serialized).await
				}

				let sockets = self.by_uid.get(specifier)
					.unwrap_or(&empty_set);
				for socket in sockets {
					if socket.subscriptions.contains(Subscription::UsersCurrentRoles) {
						socket.send(&serialized).await;
					}
				}
			},
			EventPacket::UserUpdated { user } => {
				let serialized = packet.serialize_packet();
				for socket in self.by_subscription[Subscription::Users].iter() {
					socket.send(&serialized).await
				}

				let sockets = self.by_uid.get(&Some(user.view.specifier()))
					.unwrap_or(&empty_set);
				for socket in sockets {
					if socket.subscriptions.contains(Subscription::UsersCurrent) {
						socket.send(&serialized).await;
					}
				}
			},
			EventPacket::SiteNoticeCreated { .. } |
			EventPacket::SiteNoticeUpdated { .. } |
			EventPacket::SiteNoticeDeleted { .. } => {
				let serialized = packet.serialize_packet();
				for socket in self.by_subscription[Subscription::Notices].iter() {
					socket.send(&serialized).await
				}
			},
			EventPacket::ReportCreated { reporter, .. } |
			EventPacket::ReportUpdated { reporter, .. } |
			EventPacket::ReportDeleted { reporter, .. } => {
				let serialized = packet.serialize_packet();
				for socket in self.by_subscription[Subscription::Reports].iter() {
					socket.send(&serialized).await
				}
				
				let sockets = self.by_uid.get(reporter).unwrap_or(&empty_set);
				for socket in sockets {
					if socket.subscriptions.contains(Subscription::ReportsOwned) {
						socket.send(&serialized).await;
					}
				}
			},
			EventPacket::StatsUpdated { user, .. } => {
				let serialized = packet.serialize_packet();
				let sockets = self.by_uid.get(user).unwrap_or(&empty_set);
				for socket in sockets {
					if socket.subscriptions.contains(Subscription::Statistics) {
						socket.send(&serialized).await;
					}
				}
			},
			EventPacket::UserBanCreated { user, .. } |
			EventPacket::UserBanUpdated { user, .. } |
			EventPacket::UserBanDeleted { user, .. } => {
				let serialized = packet.serialize_packet();
				for socket in self.by_subscription[Subscription::UsersBans].iter() {
					socket.send(&serialized).await
				}
				
				let sockets = self.by_uid.get(&Some(*user)).unwrap_or(&empty_set);
				for socket in sockets {
					if socket.subscriptions.contains(Subscription::UsersCurrentBans) {
						socket.send(&serialized).await;
					}
				}
			},
			EventPacket::FactionCreated { members, .. } |
			EventPacket::FactionUpdated { members, .. } |
			EventPacket::FactionDeleted { members, .. } => {
				let serialized = packet.serialize_packet();
				for socket in self.by_subscription[Subscription::Factions].iter() {
					socket.send(&serialized).await
				}

				for member in members {
					let user = Some(*member);
					let sockets = self.by_uid.get(&user).unwrap_or(&empty_set);
					for socket in sockets {
						if socket.subscriptions.contains(Subscription::FactionsCurrent) {
							socket.send(&serialized).await
						}
					}
				}
			},
			EventPacket::FactionMemberUpdated { owners, user, .. } => {
				let serialized = packet.serialize_packet();
				for socket in self.by_subscription[Subscription::FactionsMembers].iter() {
					socket.send(&serialized).await
				}

				for owner in owners {
					let user = Some(*owner);
					let sockets = self.by_uid.get(&user).unwrap_or(&empty_set);
					for socket in sockets {
						if socket.subscriptions.contains(Subscription::FactionsCurrentMembers) {
							socket.send(&serialized).await
						}
					}
				}

				if !owners.contains(user) {
					let user = Some(*user);
					let sockets = self.by_uid.get(&user).unwrap_or(&empty_set);
					for socket in sockets {
						if socket.subscriptions.contains(Subscription::FactionsCurrentMembers) {
							socket.send(&serialized).await
						}
					}
				}
			},
		};

	}
}

#[derive(Serialize)]
#[serde(tag = "type")]
#[serde(rename_all = "kebab-case")]
pub enum EventPacket<'l> {
	AccessUpdate {
		#[serde(skip_serializing)]
		user: Option<UserSpecifier>,
		permissions: EnumSet<Permission>,
	},
	BoardCreated {
		board: Reference<&'l Board>,
	},
	BoardDeleted {
		#[serde(with = "http_serde::uri")]
		board: Uri,
	},
	RoleCreated {
		role: Reference<Role>,
	},
	RoleUpdated {
		role: Reference<Role>,
	},
	RoleDeleted {
		#[serde(with = "http_serde::uri")]
		role: Uri,
	},
	UserRolesUpdated {
		#[serde(skip_serializing)]
		specifier: Option<UserSpecifier>,
		#[serde(with = "http_serde::uri")]
		user: Uri,
	},
	UserUpdated {
		user: Reference<User>,
	},
	SiteNoticeCreated {
		notice: Reference<Notice>,
	},
	SiteNoticeUpdated {
		notice: Reference<Notice>,
	},
	SiteNoticeDeleted {
		#[serde(with = "http_serde::uri")]
		notice: Uri,
	},
	ReportCreated {
		#[serde(skip_serializing)]
		reporter: Option<UserSpecifier>,
		report: Reference<Report>,
	},
	ReportUpdated {
		#[serde(skip_serializing)]
		reporter: Option<UserSpecifier>,
		report: Reference<Report>,
	},
	ReportDeleted {
		#[serde(skip_serializing)]
		reporter: Option<UserSpecifier>,
		#[serde(with = "http_serde::uri")]
		report: Uri,
	},
	StatsUpdated {
		#[serde(skip_serializing)]
		user: Option<UserSpecifier>,
		stats: UserStats,
	},
	UserBanCreated {
		#[serde(skip_serializing)]
		user: UserSpecifier,
		ban: Reference<Ban>,
	},
	UserBanUpdated {
		#[serde(skip_serializing)]
		user: UserSpecifier,
		ban: Reference<Ban>,
	},
	UserBanDeleted {
		#[serde(skip_serializing)]
		user: UserSpecifier,
		#[serde(with = "http_serde::uri")]
		ban: Uri,
	},
	FactionCreated {
		#[serde(skip_serializing)]
		members: Vec<UserSpecifier>,
		faction: Reference<Faction>,
	},
	FactionUpdated {
		#[serde(skip_serializing)]
		members: Vec<UserSpecifier>,
		faction: Reference<Faction>,
	},
	FactionDeleted {
		#[serde(skip_serializing)]
		members: Vec<UserSpecifier>,
		#[serde(with = "http_serde::uri")]
		faction: Uri,
	},
	FactionMemberUpdated {
		#[serde(skip_serializing)]
		owners: Vec<UserSpecifier>,
		#[serde(skip_serializing)]
		user: UserSpecifier,
		faction: Reference<Faction>,
		member: Reference<FactionMember>,
	},
}

impl<'l> ServerPacket for EventPacket<'l> {}

#[derive(Debug, EnumSetType, Enum)]
#[enumset(serialize_repr = "list")]
enum Subscription {
	Access,
	Boards,
	Roles,
	UsersRoles,
	UsersCurrentRoles,
	Users,
	UsersCurrent,
	Notices,
	Reports,
	ReportsOwned,
	Statistics,
	UsersBans,
	UsersCurrentBans,
	Factions,
	FactionsCurrent,
	FactionsMembers,
	FactionsCurrentMembers,
}

impl Subscription {
	pub fn to_current(self) -> Option<Subscription> {
		match self {
			Subscription::UsersRoles => Some(Subscription::UsersCurrentRoles),
			Subscription::Users => Some(Subscription::UsersCurrent),
			Subscription::Factions => Some(Subscription::FactionsCurrent),
			Subscription::FactionsMembers => Some(Subscription::FactionsCurrentMembers),
			_ => None,
		}
	}
}

impl From<Subscription> for Permission {
	fn from(value: Subscription) -> Self {
		match value {
			Subscription::Access => Permission::EventsAccess,
			Subscription::Boards => Permission::EventsBoards,
			Subscription::Roles => Permission::EventsRoles,
			Subscription::UsersRoles => Permission::EventsUsersRoles,
			Subscription::UsersCurrentRoles => Permission::EventsUsersCurrentRoles,
			Subscription::Users => Permission::EventsUsers,
			Subscription::UsersCurrent => Permission::EventsUsersCurrent,
			Subscription::Notices => Permission::EventsNotices,
			Subscription::Reports => Permission::EventsReports,
			Subscription::ReportsOwned => Permission::EventsReportsOwned,
			Subscription::Statistics => Permission::EventsStatistics,
			Subscription::UsersBans => Permission::EventsUsersBans,
			Subscription::UsersCurrentBans => Permission::EventsUsersCurrentBans,
			Subscription::Factions => Permission::EventsFactions,
			Subscription::FactionsCurrent => Permission::EventsFactionsCurrent,
			Subscription::FactionsMembers => Permission::EventsFactionsMembers,
			Subscription::FactionsCurrentMembers => Permission::EventsFactionsCurrentMembers,
		}
	}
}

impl TryFrom<&str> for Subscription {
	type Error = ();

	fn try_from(value: &str) -> Result<Self, Self::Error> {
		match value {
			"access" => Ok(Subscription::Access),
			"boards" => Ok(Subscription::Boards),
			"roles" => Ok(Subscription::Roles),
			"users.roles" => Ok(Subscription::UsersRoles),
			"users.current.roles" => Ok(Subscription::UsersCurrentRoles),
			"users" => Ok(Subscription::Users),
			"users.current" => Ok(Subscription::UsersCurrent),
			"statistics" => Ok(Subscription::Statistics),
			"factions" => Ok(Subscription::Factions),
			"factions.current" => Ok(Subscription::FactionsCurrent),
			"factions.members" => Ok(Subscription::FactionsMembers),
			"factions.current.members" => Ok(Subscription::FactionsCurrentMembers),
			_ => Err(()),
		}
	}
}

impl<'de> Deserialize<'de> for Subscription {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where D: serde::Deserializer<'de> {
		struct V;
		impl<'de> Visitor<'de> for V {
			type Value = Subscription;

			fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
				write!(f, "A valid subscription string")
			}

			fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
			where E: serde::de::Error, {
				Subscription::try_from(v)
					.map_err(|()| {
						format!("Invalid subscription string \"{}\"", v)
					})
					.map_err(E::custom)
			}
		}

		deserializer.deserialize_str(V)
	}
}

// NOTE: this is needed for the correct deserialization to be set on enumtype
impl serde::Serialize for Subscription {
	fn serialize<S>(&self, _: S) -> Result<S::Ok, S::Error>
	where S: serde::Serializer {
		unimplemented!()
	}
}

#[derive(Debug, Deserialize)]
struct SocketOptions {
	subscribe: Option<EnumSet<Subscription>>,
	authenticate: Option<bool>,
}

#[derive(Debug)]
pub enum SocketError {
	MissingSubscriptions,
	ConflictingPermissions,
	MissingPermissions,
	DatabaseConnectionError(sea_orm::DbErr),
	DatabaseError(DatabaseError),
}

impl Reject for SocketError {}

impl From<&SocketError> for StatusCode {
	fn from(value: &SocketError) -> Self {
		match value {
			SocketError::MissingSubscriptions => StatusCode::UNPROCESSABLE_ENTITY,
			SocketError::ConflictingPermissions => StatusCode::UNPROCESSABLE_ENTITY,
			SocketError::MissingPermissions => StatusCode::FORBIDDEN,
			SocketError::DatabaseConnectionError(err) => StatusCode::INTERNAL_SERVER_ERROR,
			SocketError::DatabaseError(err) => StatusCode::INTERNAL_SERVER_ERROR,
		}
	}
}

impl Reply for SocketError {
	fn into_response(self) -> warp::reply::Response {
		StatusCode::from(&self).into_response()
	}
}

pub fn events(
	connections: Arc<RwLock<Connections>>,
	db: Arc<BoardsDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("events")
		.and(warp::path::end())
		.and(warp::ws())
		.and(serde_qs::warp::query(Default::default()))
		.and_then(move |ws: Ws, options: SocketOptions| {
			let connections = Arc::clone(&connections);
			let db = Arc::clone(&db);
			async move {
				let subscriptions = options.subscribe
					.ok_or(SocketError::MissingSubscriptions)
					.map_err(Rejection::from)?;
				
				if subscriptions.is_empty() {
					return Err(Rejection::from(SocketError::MissingSubscriptions));
				}
				// check mutually exclusive permissions
				for subscription in subscriptions {
					if let Some(current) = subscription.to_current() {
						if subscriptions.contains(current) {
							return Err(Rejection::from(SocketError::ConflictingPermissions));
						}
					}
				}

				let anonymous = !options.authenticate.unwrap_or(false);

				if anonymous {
					let connection = db.connection().await
						.map_err(SocketError::DatabaseConnectionError)
						.map_err(Rejection::from)?;

					let permissions = connection.anonymous_permissions().await
						.map_err(SocketError::DatabaseError)
						.map_err(Rejection::from)?;

					let has_permissions = subscriptions.iter()
						.map(Permission::from)
						.all(|p| permissions.contains(p));
				
					if !has_permissions {
						return Err(Rejection::from(SocketError::MissingPermissions));
					}
				}
			
				let users_db = Arc::clone(&db);
				let connections_init = Arc::clone(&connections);
				let connections_shutdown = Arc::clone(&connections);

				Ok(ws.on_upgrade(move |websocket| async move {
					let connect_result = Socket::connect(
						websocket,
						subscriptions,
						users_db,
						anonymous,
					).await;

					let socket = match connect_result {
						Ok(socket) => socket,
						Err(_) => return,
					};
					
					socket.init(|socket| async move {
						// add socket to connections
						let mut connections = connections_init.write().await;
						connections.insert_socket(socket).await;
					}).await.shutdown(|socket| async move {
						// remove socket from connections
						let mut connections = connections_shutdown.write().await;
						connections.remove_socket(&socket).await;
					}).await;
				}))
			}
		})
		.recover(|rejection: Rejection| async {
			if let Some(err) = rejection.find::<serde_qs::Error>() {
				Ok(StatusCode::UNPROCESSABLE_ENTITY.into_response())
			} else if let Some(err) = rejection.find::<SocketError>() {
				Ok(StatusCode::from(err).into_response())
			} else {
				Err(rejection)
			}
		})
}
