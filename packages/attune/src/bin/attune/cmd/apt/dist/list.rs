use clap::Args;
use tabled::settings::Style;

use crate::{
    cmd::apt::dist::{build_distribution_url, handle_api_response},
    config::Config,
};
use attune::server::repo::dist::list::ListDistributionsResponse;

#[derive(Args, Debug)]
pub struct ListArgs {
    /// The name of the repository.
    #[arg(long)]
    repo: String,
}

pub async fn run(ctx: Config, args: ListArgs) -> Result<String, String> {
    let url = build_distribution_url(&ctx, &args.repo, None);
    let response = ctx
        .client
        .get(url)
        .send()
        .await
        .map(handle_api_response::<ListDistributionsResponse>)
        .map_err(|err| format!("Failed to send request: {err}"))?
        .await?;

    if response.distributions.is_empty() {
        return Ok(format!(
            "No distributions found in repository {:?}",
            args.repo
        ));
    }

    let mut builder = tabled::builder::Builder::new();
    builder.push_record([
        "Name",
        "Suite",
        "Codename",
        "Description",
        "Origin",
        "Label",
        "Version",
    ]);
    for dist in response.distributions {
        builder.push_record([
            dist.distribution,
            dist.suite,
            dist.codename,
            dist.description.unwrap_or_else(String::new),
            dist.origin.unwrap_or_else(String::new),
            dist.label.unwrap_or_else(String::new),
            dist.version.unwrap_or_else(String::new),
        ]);
    }

    let mut table = builder.build();
    table.with(Style::modern());
    Ok(table.to_string())
}
