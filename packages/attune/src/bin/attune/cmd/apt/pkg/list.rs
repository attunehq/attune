use std::process::ExitCode;

use axum::http::StatusCode;
use clap::Args;

use crate::config::Config;
use attune::{
    api::ErrorResponse,
    server::pkg::list::{PackageListParams, PackageListResponse},
};

#[derive(Args, Debug)]
pub struct PkgListCommand {
    #[arg(short, long)]
    repository: Option<String>,
    #[arg(short, long)]
    distribution: Option<String>,
    #[arg(short, long)]
    component: Option<String>,
    #[arg(short, long)]
    name: Option<String>,
    #[arg(short, long)]
    version: Option<String>,
    #[arg(short, long)]
    architecture: Option<String>,
}

pub async fn run(ctx: Config, command: PkgListCommand) -> ExitCode {
    let res = ctx
        .client
        .get(ctx.endpoint.join("/api/v0/packages").unwrap())
        .query(&PackageListParams {
            repository: command.repository,
            distribution: command.distribution,
            component: command.component,
            name: command.name,
            version: command.version,
            architecture: command.architecture,
        })
        .send()
        .await
        .expect("Could not send API request");
    match res.status() {
        StatusCode::OK => {
            let packages = res
                .json::<PackageListResponse>()
                .await
                .expect("Could not parse response");
            let mut builder = tabled::builder::Builder::new();
            builder.push_record([
                "Package",
                "Version",
                "Architecture",
                "Repository",
                "Distribution",
                "Component",
            ]);
            for package in packages.packages {
                builder.push_record([
                    package.name,
                    package.version,
                    package.architecture,
                    package.repository,
                    package.distribution,
                    package.component,
                ]);
            }
            let table = builder.build();
            println!("{}", table);
            ExitCode::SUCCESS
        }
        _ => {
            let error = res
                .json::<ErrorResponse>()
                .await
                .expect("Could not parse error response");
            eprintln!("Error listing packages: {}", error.message);
            ExitCode::FAILURE
        }
    }
}
