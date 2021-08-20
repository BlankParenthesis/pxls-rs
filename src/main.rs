#[macro_use] extern crate rocket;
#[macro_use] extern crate lazy_static;

extern crate serde;
extern crate async_trait;

#[macro_use] mod access;
mod routes;

#[launch]
fn rocket() -> _ {
    rocket::build()
        .mount("/", routes![
            routes::core::info::info,
            routes::core::access::access,
            routes::core::boards::list,
            routes::core::boards::get,
            routes::core::boards::get_default,
        ])
}