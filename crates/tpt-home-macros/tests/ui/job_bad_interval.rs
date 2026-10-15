use tpt_home_macros::bp_job;

#[bp_job(every = "5x")]
async fn sweep() -> Result<(), tpt_home_core::ApiError> {
    Ok(())
}

fn main() {}
