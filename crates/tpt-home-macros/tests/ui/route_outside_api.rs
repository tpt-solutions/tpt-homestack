use tpt_home_macros::bp_route;

#[bp_route(get, "/notes")]
async fn list() {}

fn main() {}
