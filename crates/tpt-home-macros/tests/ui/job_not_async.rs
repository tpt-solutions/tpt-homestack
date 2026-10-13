use tpt_home_macros::bp_job;

#[bp_job(every = "5m")]
fn sweep() -> Result<(), tpt_home_core::ApiError> {
    Ok(())
}

fn main() {}
