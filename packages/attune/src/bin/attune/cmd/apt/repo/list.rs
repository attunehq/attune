use std::process::ExitCode;

use axum::http::StatusCode;
use clap::Args;
use tabled::settings::Style;

use crate::config::Config;
use attune::{
    api::ErrorResponse,
    server::repo::list::{ListRepositoryRequest, ListRepositoryResponse},
};

#[derive(Args, Debug)]
pub struct RepoListCommand {
    /// Output in JSON format.
    #[arg(long)]
    json: bool,

    /// Filter repositories by name (substring match).
    #[arg(long)]
    name: Option<String>,
}

pub async fn run(ctx: Config, cmd: RepoListCommand) -> ExitCode {
    let res = ctx
        .client
        .get(ctx.endpoint.join("/api/v0/repositories").unwrap())
        .json(&ListRepositoryRequest { name: cmd.name })
        .send()
        .await
        .expect("Could not send API request");
    match res.status() {
        StatusCode::OK => {
            let res = res
                .json::<ListRepositoryResponse>()
                .await
                .expect("Could not parse response");
            // TODO: In the managed cloud version of this CLI, we should hide
            // the S3 bucket and prefix fields because they're irrelevant.
            if cmd.json {
                println!("{}", serde_json::to_string_pretty(&res).unwrap());
                return ExitCode::SUCCESS;
            }
            let mut builder = tabled::builder::Builder::new();
            builder.push_record([
                String::from("Name"),
                String::from("S3 bucket"),
                String::from("S3 prefix"),
            ]);
            for repo in res.repositories {
                builder.push_record([&repo.name, &repo.s3_bucket, &repo.s3_prefix]);
            }
            let mut table = builder.build();
            table.with(Style::modern());
            println!("{table}");
            ExitCode::SUCCESS
        }
        _ => {
            let error = res
                .json::<ErrorResponse>()
                .await
                .expect("Could not parse error response");
            eprintln!("Error listing repositories: {}", error.message);
            ExitCode::FAILURE
        }
    }
}
