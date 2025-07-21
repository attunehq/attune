use clap::Parser;

/// Attune CLI
///
/// Attune is the easiest way to securely publish Linux packages.
#[derive(Parser)]
#[command(name = "attune", max_term_width = 80)]
struct Args {
    /// Attune API token.
    #[arg(long, env = "ATTUNE_API_TOKEN")]
    api_token: Option<String>,
}

fn main() {
    let args = Args::parse();

    println!("Hello, world!");
}
