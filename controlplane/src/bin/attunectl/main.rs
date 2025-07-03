use clap::{Args, Parser, Subcommand};
use sha2::{Digest as _, Sha256};
use sqlx::PgPool;
use tracing_subscriber::{
    fmt::format::FmtSpan, layer::SubscriberExt as _, util::SubscriberInitExt as _,
};

#[derive(Parser)]
#[command(name = "attunectl", about = "Attune utilities CLI")]
struct CLI {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(name = "init", about = "Initialize Attune")]
    Init,
    #[command(name = "token", about = "Manage API tokens")]
    Token(TokenCommand),
}

#[derive(Args)]
struct TokenCommand {
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
        Command::Init => handle_init(db).await,
        Command::Token(command) => handle_token(command, db).await,
    }
}

async fn handle_init(db: PgPool) {
    println!("Initializing...");
    match sqlx::query!("SELECT id FROM attune_tenant WHERE id = 1")
        .fetch_optional(&db)
        .await
        .expect("could not find local tenant")
    {
        Some(_) => {
            println!("Attune is already initialized");
        }
        None => {
            sqlx::query!(
                "INSERT INTO attune_tenant (display_name, subdomain, updated_at) VALUES ($1, $2, NOW()) RETURNING id",
                "local",
                "",
            )
            .fetch_one(&db)
            .await
            .expect("could not add tenant");
            println!("Done!");
        }
    }
}

async fn handle_token(command: TokenCommand, db: PgPool) {
    let local_tenant = sqlx::query!("SELECT id FROM attune_tenant LIMIT 1")
        .fetch_one(&db)
        .await
        .expect("could not find local tenant");
    match command.subcommand {
        TokenSubcommand::Add { name, value } => {
            let token = sqlx::query!(
                "INSERT INTO attune_tenant_api_token (tenant_id, name, token, updated_at) VALUES ($1, $2, $3, NOW()) RETURNING id",
                local_tenant.id,
                name,
                Sha256::digest(value).as_slice().to_vec(),
            )
            .fetch_one(&db)
            .await;
            match token {
                Ok(token) => println!("Added token with ID {}", token.id),
                Err(e) => println!(
                    "Could not add token. Maybe you need to run `attunectl init`?\n\nDetails: {}",
                    e
                ),
            }
        }
        TokenSubcommand::List => {
            let tokens = sqlx::query!(
                "SELECT id, name, token FROM attune_tenant_api_token WHERE tenant_id = $1",
                local_tenant.id
            )
            .fetch_all(&db)
            .await
            .expect("could not list tokens");
            let mut builder = tabled::builder::Builder::new();
            builder.push_record([
                "ID".to_string(),
                "Name".to_string(),
                "SHA-256(token)".to_string(),
            ]);
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
