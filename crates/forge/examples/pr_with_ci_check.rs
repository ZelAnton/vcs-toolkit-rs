//! Opens a GitHub pull request and reports its current CI outcome.
//!
//! See `crates/core/docs/cookbook.md` — "Open a PR and wait for CI".
//!
//! ```no_run
//! # async fn run() -> vcs_forge::Result<()> {
//! let forge = vcs_forge::Forge::github(".");
//! let authenticated = forge.auth_status().await?;
//! # let _ = authenticated;
//! # Ok(())
//! # }
//! ```

use vcs_forge::{CiStatus, Forge, PrCreate};

#[tokio::main]
async fn main() -> vcs_forge::Result<()> {
    let forge = Forge::github(".");
    if !forge.auth_status().await? {
        eprintln!("GitHub authentication is required; run `gh auth login`");
        return Ok(());
    }

    let branch = "feature";
    let spec = PrCreate::new("Add the feature", "Implements the feature.")
        .source(branch)
        .target("main");
    let url = forge.pr_create(spec).await?;
    println!("opened {url}");

    let number = url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .and_then(|part| part.parse::<u64>().ok())
        .ok_or_else(|| vcs_forge::Error::InvalidInput(format!("no PR number in {url}")))?;

    loop {
        match forge.pr_checks(number).await? {
            CiStatus::Passing => {
                println!("CI passed");
                break;
            }
            CiStatus::Failing => {
                println!("CI failed");
                break;
            }
            CiStatus::Pending | CiStatus::None => {
                println!("CI is not complete; checking again soon");
                std::thread::sleep(std::time::Duration::from_secs(15));
            }
            _ => {
                println!("CI status is not recognized by this version");
                break;
            }
        }
    }

    Ok(())
}
