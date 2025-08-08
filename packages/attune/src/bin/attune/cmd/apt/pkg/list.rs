use std::process::ExitCode;

use clap::Args;

use crate::{
    config::Config,
    http::{ResponseDropStatus, ResponseRequiresBody},
};
use attune::server::pkg::list::{PackageListParams, PackageListResponse};

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
    let res = crate::http::get_with_query::<PackageListResponse, _>(
        &ctx,
        "/api/v0/packages",
        &PackageListParams {
            repository: command.repository,
            distribution: command.distribution,
            component: command.component,
            name: command.name,
            version: command.version,
            architecture: command.architecture,
        },
    )
    .await
    .require_body()
    .drop_status();
    match res {
        Ok(packages) => {
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
            println!("{table}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Error listing packages: {}", error.message);
            ExitCode::FAILURE
        }
    }
}
