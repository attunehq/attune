use std::process::ExitCode;

use clap::Args;
use tabled::settings::Style;

use crate::{
    config::Config,
    http::{NoBody, ResponseDropStatus, ResponseRequiresBody},
};
use attune::server::repo::list::ListRepositoryResponse;

#[derive(Args, Debug)]
pub struct RepoListCommand {}

pub async fn run(ctx: Config, _command: RepoListCommand) -> ExitCode {
    let res = crate::http::get::<ListRepositoryResponse, _>(&ctx, "/api/v0/repositories", &NoBody)
        .await
        .require_body()
        .drop_status();
    match res {
        Ok(repo) => {
            let mut builder = tabled::builder::Builder::new();
            builder.push_record(["Name".to_string()]);
            for repo in repo.repositories {
                builder.push_record([&repo.name]);
            }
            let mut table = builder.build();
            table.with(Style::modern());
            println!("{table}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Error listing repositories: {}", error.message);
            ExitCode::FAILURE
        }
    }
}
