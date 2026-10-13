use tpt_home_macros::bp_route;

#[bp_route(fetch, "/api/notes")]
async fn list() {}

fn main() {}
