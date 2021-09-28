use diesel::r2d2::ConnectionManager as Manager;

use diesel::PgConnection;

pub type Connection = r2d2::PooledConnection<Manager<PgConnection>>;
pub type Pool = r2d2::Pool<Manager<PgConnection>>;

pub mod schema;
pub mod model;
pub mod queries;
