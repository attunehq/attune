use axum::http::StatusCode;
use clap::Args;

use crate::config::Config;
use attune::{
    api::ErrorResponse,
    server::repo::create::{CreateRepositoryRequest, CreateRepositoryResponse},
};

#[derive(Args)]
pub struct RepoCreateCommand {
    /// A name that uniquely identifies this repository.
    name: String,
}

pub async fn handle_repo_create(ctx: Config, command: RepoCreateCommand) {
    let res = ctx
        .client
        .post(ctx.endpoint.join("/api/v0/repositories").unwrap())
        .json(&CreateRepositoryRequest { name: command.name })
        .send()
        .await
        .expect("Could not send API request");
    match res.status() {
        StatusCode::OK => {
            let repo = res
                .json::<CreateRepositoryResponse>()
                .await
                .expect("Could not parse response");
            println!("Repository created: {}", repo.name);
        }
        _ => {
            let error = res
                .json::<ErrorResponse>()
                .await
                .expect("Could not parse error response");
            eprintln!("Error creating repository: {}", error.message);
        }
    }
}
