use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use enum_map::{EnumMap, Enum};
use enumset::{EnumSet, EnumSetType};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde::de::Visitor;
use tokio::sync::RwLock;
use warp::ws::Ws;
use warp::{Reply, Rejection};
use warp::Filter;
use warp::http::Uri;

use crate::board::Board;
use crate::database::{UsersDatabase, Role, User};
use crate::filter::response::reference::Reference;
use crate::permissions::Permission;
use crate::routes::placement_statistics::users::UserStats;
use crate::routes::reports::reports::Report;
use crate::routes::site_notices::notices::Notice;
use crate::routes::user_bans::users::Ban;
use crate::socket::ServerPacket;

type Socket = crate::socket::Socket<Subscription>;

// TODO: move this elsewhere
#[derive(Default)]
pub struct Connections {
	by_uid: HashMap<Option<String>, HashSet<Arc<Socket>>>,
	by_subscription: EnumMap<Subscription, HashSet<Arc<Socket>>>,
}

#[allow(clippy::mutable_key_type)]
impl Connections {
	async fn insert_socket(&mut self, socket: Arc<Socket>) {
		let user_id = socket.user_id().await;
		let sockets = self.by_uid.entry(user_id).or_default();
		sockets.insert(socket.clone());

		for subscription in socket.subscriptions {
			self.by_subscription[subscription].insert(socket.clone());
		}
	}

	async fn remove_socket(&mut self, socket: &Arc<Socket>) {
		let user_id = socket.user_id().await;
		let sockets = self.by_uid.entry(user_id.clone()).or_default();
		sockets.remove(socket);
		if sockets.is_empty() {
			self.by_uid.remove(&user_id);
		}

		for subscription in socket.subscriptions {
			self.by_subscription[subscription].remove(socket);
		}
	}

	pub async fn send<'l>(&self, packet: &EventPacket<'l>) {
		let empty_set = HashSet::default();

		match packet {
			EventPacket::AccessUpdate { user_id, .. } => {
				let sockets = self.by_uid.get(user_id)
					.unwrap_or(&empty_set);
				for socket in sockets {
					if socket.subscriptions.contains(Subscription::Access) {
						socket.send(packet).await;
					}
				}
			},
			EventPacket::BoardCreated { .. } |
			EventPacket::BoardDeleted { .. } => {
				for socket in self.by_subscription[Subscription::Boards].iter() {
					socket.send(packet).await
				}
			},
			EventPacket::RoleCreated { .. } |
			EventPacket::RoleUpdated { .. } |
			EventPacket::RoleDeleted { .. } => {
				for socket in self.by_subscription[Subscription::Roles].iter() {
					socket.send(packet).await
				}
			},
			EventPacket::UserRolesUpdated { user_id, .. } => {
				for socket in self.by_subscription[Subscription::UsersRoles].iter() {
					socket.send(packet).await
				}

				let sockets = self.by_uid.get(user_id)
					.unwrap_or(&empty_set);
				for socket in sockets {
					if socket.subscriptions.contains(Subscription::UsersCurrentRoles) {
						socket.send(packet).await;
					}
				}
			},
			EventPacket::UserUpdated { user_id, ..  } => {
				for socket in self.by_subscription[Subscription::Users].iter() {
					socket.send(packet).await
				}

				let sockets = self.by_uid.get(&Some(user_id.clone()))
					.unwrap_or(&empty_set);
				for socket in sockets {
					if socket.subscriptions.contains(Subscription::UsersCurrent) {
						socket.send(packet).await;
					}
				}
			},
			EventPacket::SiteNoticeCreated { .. } => {
				for socket in self.by_subscription[Subscription::Notices].iter() {
					socket.send(packet).await
				}
			},
			EventPacket::SiteNoticeUpdated { .. } => {
				for socket in self.by_subscription[Subscription::Notices].iter() {
					socket.send(packet).await
				}
			},
			EventPacket::SiteNoticeDeleted { .. } => {
				for socket in self.by_subscription[Subscription::Notices].iter() {
					socket.send(packet).await
				}
			},
			EventPacket::ReportCreated { reporter, .. } => {
				for socket in self.by_subscription[Subscription::Reports].iter() {
					socket.send(packet).await
				}

				let sockets = self.by_uid.get(reporter).unwrap_or(&empty_set);
				for socket in sockets {
					if socket.subscriptions.contains(Subscription::ReportsOwned) {
						socket.send(packet).await;
					}
				}
			},
			EventPacket::ReportUpdated { reporter, .. } => {
				for socket in self.by_subscription[Subscription::Reports].iter() {
					socket.send(packet).await
				}

				let sockets = self.by_uid.get(reporter).unwrap_or(&empty_set);
				for socket in sockets {
					if socket.subscriptions.contains(Subscription::ReportsOwned) {
						socket.send(packet).await;
					}
				}
			},
			EventPacket::ReportDeleted { reporter, .. } => {
				for socket in self.by_subscription[Subscription::Reports].iter() {
					socket.send(packet).await
				}
				
				let sockets = self.by_uid.get(reporter).unwrap_or(&empty_set);
				for socket in sockets {
					if socket.subscriptions.contains(Subscription::ReportsOwned) {
						socket.send(packet).await;
					}
				}
			},
			EventPacket::StatsUpdated { user, .. } => {
				let sockets = self.by_uid.get(user).unwrap_or(&empty_set);
				for socket in sockets {
					if socket.subscriptions.contains(Subscription::Statistics) {
						socket.send(packet).await;
					}
				}
			},
			EventPacket::UserBanCreated { user, .. } => {
				for socket in self.by_subscription[Subscription::UsersBans].iter() {
					socket.send(packet).await
				}
				
				let user = Some(user.clone());
				let sockets = self.by_uid.get(&user).unwrap_or(&empty_set);
				for socket in sockets {
					if socket.subscriptions.contains(Subscription::UsersCurrentBans) {
						socket.send(packet).await;
					}
				}
			},
			EventPacket::UserBanUpdated { user, .. } => {
				for socket in self.by_subscription[Subscription::UsersBans].iter() {
					socket.send(packet).await
				}
				
				let user = Some(user.clone());
				let sockets = self.by_uid.get(&user).unwrap_or(&empty_set);
				for socket in sockets {
					if socket.subscriptions.contains(Subscription::UsersCurrentBans) {
						socket.send(packet).await;
					}
				}
			},
			EventPacket::UserBanDeleted { user, .. } => {
				for socket in self.by_subscription[Subscription::UsersBans].iter() {
					socket.send(packet).await
				}
				
				let user = Some(user.clone());
				let sockets = self.by_uid.get(&user).unwrap_or(&empty_set);
				for socket in sockets {
					if socket.subscriptions.contains(Subscription::UsersCurrentBans) {
						socket.send(packet).await;
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
		user_id: Option<String>,
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
		user_id: Option<String>,
		#[serde(with = "http_serde::uri")]
		user: Uri,
	},
	UserUpdated {
		#[serde(skip_serializing)]
		user_id: String,
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
		reporter: Option<String>,
		report: Reference<Report>,
	},
	ReportUpdated {
		#[serde(skip_serializing)]
		reporter: Option<String>,
		report: Reference<Report>,
	},
	ReportDeleted {
		#[serde(skip_serializing)]
		reporter: Option<String>,
		#[serde(with = "http_serde::uri")]
		report: Uri,
	},
	StatsUpdated {
		#[serde(skip_serializing)]
		user: Option<String>,
		stats: UserStats,
	},
	UserBanCreated {
		#[serde(skip_serializing)]
		user: String,
		ban: Reference<Ban>,
	},
	UserBanUpdated {
		#[serde(skip_serializing)]
		user: String,
		ban: Reference<Ban>,
	},
	UserBanDeleted {
		#[serde(skip_serializing)]
		user: String,
		#[serde(with = "http_serde::uri")]
		ban: Uri,
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
}

impl Subscription {
	pub fn to_current(self) -> Option<Subscription> {
		match self {
			Subscription::UsersRoles => Some(Subscription::UsersCurrentRoles),
			Subscription::Users => Some(Subscription::UsersCurrent),
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

pub fn events(
	connections: Arc<RwLock<Connections>>,
	users_db: Arc<UsersDatabase>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
	warp::path("events")
		.and(warp::path::end())
		.and(warp::ws())
		.and(serde_qs::warp::query(Default::default()))
		.map(move |ws: Ws, options: SocketOptions| {
			options.subscribe
				.ok_or(StatusCode::UNPROCESSABLE_ENTITY)
				.and_then(|subscriptions| {
					
					if subscriptions.is_empty() {
						return Err(StatusCode::UNPROCESSABLE_ENTITY);
					}
					// check mutually exclusive permissions
					for subscription in subscriptions {
						if let Some(current) = subscription.to_current() {
							if subscriptions.contains(current) {
								return Err(StatusCode::UNPROCESSABLE_ENTITY);
							}
						}
					}

					let anonymous = !options.authenticate.unwrap_or(false);

					if anonymous {
						let permissions = Permission::defaults();
						let has_permissions = subscriptions.iter()
							.map(Permission::from)
							.all(|p| permissions.contains(p));
					
						if !has_permissions {
							return Err(StatusCode::FORBIDDEN);
						}
					}
				
					let users_db = Arc::clone(&users_db);
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
				})
		})
		.recover(|rejection: Rejection| async {
			if let Some(err) = rejection.find::<serde_qs::Error>() {
				Ok(StatusCode::UNPROCESSABLE_ENTITY.into_response())
			} else {
				Err(rejection)
			}
		})
}
