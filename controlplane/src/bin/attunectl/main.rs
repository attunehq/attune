use clap::{Args, Parser, Subcommand};
use sha2::{Digest as _, Sha256};
use sqlx::PgPool;
use tracing_subscriber::{
    fmt::format::FmtSpan, layer::SubscriberExt as _, util::SubscriberInitExt as _,
};

use attune_controlplane::auth;

#[derive(Parser)]
#[command(name = "attunectl", about = "Attune utilities CLI")]
struct CLI {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(name = "tenant", about = "Manage tenants")]
    Tenant(TenantCommand),
    #[command(name = "token", about = "Manage API tokens")]
    Token(TokenCommand),
}

#[derive(Args)]
struct TenantCommand {
    #[command(subcommand)]
    subcommand: TenantSubcommand,
}

#[derive(Subcommand)]
enum TenantSubcommand {
    #[command(name = "add", about = "Add a new tenant")]
    Add {
        #[arg(short = 'd', long, help = "Display name for the tenant")]
        display_name: String,
        #[arg(short = 'u', long, help = "Subdomain for the tenant")]
        subdomain: String,
        #[arg(
            short = 's',
            help = "S3 prefix for the tenant (default: SHA-256 hash of subdomain)"
        )]
        s3_prefix: Option<String>,
    },
    #[command(name = "list", alias = "ls", about = "List all tenants")]
    List,
    #[command(name = "edit", about = "Edit a tenant")]
    Edit {
        #[arg(short = 't', long, help = "ID of tenant to edit")]
        tenant_id: i64,
        #[arg(short = 'd', long, help = "New display name for the tenant")]
        display_name: Option<String>,
        #[arg(short = 'u', long, help = "New subdomain for the tenant")]
        subdomain: Option<String>,
        #[arg(short = 's', long, help = "New S3 prefix for the tenant")]
        s3_prefix: Option<String>,
    },
    #[command(name = "remove", alias = "rm", about = "Remove a tenant")]
    Remove {
        #[arg(short = 't', long, help = "ID of tenant to remove")]
        tenant_id: i64,
    },
}

#[derive(Args)]
struct TokenCommand {
    #[arg(short = 't', long, help = "ID of tenant to manage")]
    tenant_id: i64,

    #[command(subcommand)]
    subcommand: TokenSubcommand,
}

#[derive(Subcommand)]
enum TokenSubcommand {
    #[command(name = "add", about = "Add a new API token")]
    Add {
        #[arg(short = 'n', long, help = "Name for the API token")]
        name: String,
        #[arg(short = 'v', long, help = "Value for the API token")]
        value: String,
        #[arg(
            short = 's',
            long,
            help = "Control plane secret (used to encrypt API token for storage)"
        )]
        secret: String,
    },
    #[command(name = "list", alias = "ls", about = "List all API tokens")]
    List,
    #[command(name = "remove", alias = "rm", about = "Remove an API token")]
    Remove {
        #[arg(short = 'i', long, help = "ID of API token to remove")]
        token_id: i64,
    },
}

#[tokio::main]
async fn main() {
    // Initialize tracing.
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
                .with_file(true)
                .with_line_number(true)
                .with_target(true)
                .with_thread_ids(true)
                .with_thread_names(true)
                .with_writer(std::io::stderr)
                .pretty(),
        )
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // Parse CLI arguments.
    let cli = CLI::parse();

    // Initialize upstream dependencies.
    let db_url = std::env::var("ATTUNE_DATABASE_URL").expect("ATTUNE_DATABASE_URL not set");
    let db = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .expect("could not connect to database");

    // Execute command.
    match cli.command {
        Command::Tenant(command) => handle_tenant(command, db).await,
        Command::Token(command) => handle_token(command, db).await,
    }
}

async fn handle_tenant(command: TenantCommand, db: PgPool) {
    match command.subcommand {
        TenantSubcommand::Add {
            display_name,
            s3_prefix,
            subdomain,
        } => {
            let tenant = sqlx::query!(
                "INSERT INTO attune_tenant (display_name, subdomain, s3_prefix) VALUES ($1, $2, $3) RETURNING id",
                display_name,
                subdomain,
                s3_prefix.unwrap_or(hex::encode(Sha256::digest(&subdomain))),
            )
            .fetch_one(&db)
            .await
            .expect("could not add tenant");
            println!("Added tenant with ID {}", tenant.id);
        }
        TenantSubcommand::List => {
            let tenants =
                sqlx::query!("SELECT id, display_name, subdomain, s3_prefix FROM attune_tenant")
                    .fetch_all(&db)
                    .await
                    .expect("could not list tenants");
            let mut builder = tabled::builder::Builder::new();
            builder.push_record([
                "ID".to_string(),
                "Display Name".to_string(),
                "Subdomain".to_string(),
                "S3 Prefix".to_string(),
            ]);
            for tenant in tenants {
                builder.push_record([
                    tenant.id.to_string(),
                    tenant.display_name,
                    tenant.subdomain,
                    tenant.s3_prefix,
                ]);
            }
            let table = builder.build();
            println!("{}", table.to_string());
        }
        TenantSubcommand::Edit {
            tenant_id,
            display_name,
            subdomain,
            s3_prefix,
        } => {
            let updated = sqlx::query!(
                r#"
                UPDATE attune_tenant
                SET display_name = COALESCE($2, old_tenant.display_name),
                    subdomain = COALESCE($3, old_tenant.subdomain),
                    s3_prefix = COALESCE($4, old_tenant.s3_prefix)
                FROM (
                    SELECT display_name,
                           subdomain,
                           s3_prefix
                    FROM attune_tenant
                    WHERE id = $1
                ) AS old_tenant
                WHERE id = $1
                RETURNING id, attune_tenant.display_name, attune_tenant.subdomain, attune_tenant.s3_prefix
                "#,
                tenant_id,
                display_name,
                subdomain,
                s3_prefix,
            )
            .fetch_one(&db)
            .await
            .expect("could not edit tenant");
            println!("Edited tenant with ID {} to {:?}", tenant_id, updated);
        }
        TenantSubcommand::Remove { tenant_id } => {
            sqlx::query!("DELETE FROM attune_tenant WHERE id = $1", tenant_id)
                .execute(&db)
                .await
                .expect("could not remove tenant");
            println!("Removed tenant with ID {}", tenant_id);
        }
    }
}

async fn handle_token(command: TokenCommand, db: PgPool) {
    let tenant_id = command.tenant_id;
    match command.subcommand {
        TokenSubcommand::Add {
            name,
            value,
            secret,
        } => {
            let hashed_token = auth::hash_token(&secret, &value);
            let token = sqlx::query!(
                "INSERT INTO attune_tenant_api_token (tenant_id, name, token) VALUES ($1, $2, $3) RETURNING id",
                tenant_id,
                name,
                hashed_token,
            )
            .fetch_one(&db)
            .await
            .expect("could not add token");
            println!("Added token with ID {}", token.id);
        }
        TokenSubcommand::List => {
            let tokens = sqlx::query!(
                "SELECT id, name, token FROM attune_tenant_api_token WHERE tenant_id = $1",
                tenant_id
            )
            .fetch_all(&db)
            .await
            .expect("could not list tokens");
            let mut builder = tabled::builder::Builder::new();
            builder.push_record(["ID".to_string(), "Name".to_string(), "Value".to_string()]);
            for token in tokens {
                builder.push_record([token.id.to_string(), token.name, hex::encode(token.token)]);
            }
            let table = builder.build();
            println!("{}", table.to_string());
        }
        TokenSubcommand::Remove { token_id } => {
            sqlx::query!(
                "DELETE FROM attune_tenant_api_token WHERE id = $1",
                token_id
            )
            .execute(&db)
            .await
            .expect("could not remove token");
            println!("Removed token with ID {}", token_id);
        }
    }
}
