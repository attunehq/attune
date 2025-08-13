use std::process::ExitCode;

use axum::http::StatusCode;
use clap::Args;

use crate::config::Config;
use attune::{
    api::ErrorResponse,
    server::repo::create::{CreateRepositoryRequest, CreateRepositoryResponse},
};

#[derive(Args, Debug)]
pub struct RepoCreateCommand {
    /// A name that uniquely identifies this repository.
    name: String,

    /// Output in JSON format.
    #[arg(long)]
    json: bool,
}

pub async fn run(ctx: Config, command: RepoCreateCommand) -> ExitCode {
    let res = ctx
        .client
        .post(ctx.endpoint.join("/api/v0/repositories").unwrap())
        .json(&CreateRepositoryRequest { name: command.name })
        .send()
        .await
        .expect("Could not send API request");
    match res.status() {
        StatusCode::OK => {
            let res = res
                .json::<CreateRepositoryResponse>()
                .await
                .expect("Could not parse response");
            // TODO: In the managed cloud version of this CLI, we should hide
            // the S3 bucket and prefix fields because they're irrelevant.
            if command.json {
                println!("{}", serde_json::to_string_pretty(&res).unwrap());
                return ExitCode::SUCCESS;
            }
            println!(
                "Repository {:?} created in bucket {:?} at prefix {:?}",
                res.name, res.s3_bucket, res.s3_prefix
            );
            ExitCode::SUCCESS
        }
        _ => {
            let error = res
                .json::<ErrorResponse>()
                .await
                .expect("Could not parse error response");
            eprintln!("Error creating repository: {}", error.message);
            ExitCode::FAILURE
        }
    }
}
