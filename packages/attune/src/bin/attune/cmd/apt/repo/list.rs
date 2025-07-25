use axum::http::StatusCode;
use clap::Args;
use tabled::settings::Style;

use crate::config::Config;
use attune::{
    api::ErrorResponse,
    server::repo::list::{ListRepositoryRequest, ListRepositoryResponse},
};

#[derive(Args)]
pub struct RepoListCommand {}

pub async fn run(ctx: Config, _command: RepoListCommand) {
    let res = ctx
        .client
        .get(ctx.endpoint.join("/api/v0/repositories").unwrap())
        .json(&ListRepositoryRequest {})
        .send()
        .await
        .expect("Could not send API request");
    match res.status() {
        StatusCode::OK => {
            let repo = res
                .json::<ListRepositoryResponse>()
                .await
                .expect("Could not parse response");
            let mut builder = tabled::builder::Builder::new();
            builder.push_record(["Repositories".to_string()]);
            for repo in repo.repositories {
                builder.push_record([&repo.name]);
            }
            let mut table = builder.build();
            table.with(Style::markdown());
            println!("{table}");
        }
        _ => {
            let error = res
                .json::<ErrorResponse>()
                .await
                .expect("Could not parse error response");
            eprintln!("Error listing repositories: {}", error.message);
        }
    }
}
