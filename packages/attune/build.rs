fn main() {
    println!("cargo::rerun-if-env-changed=ATTUNE_DATABASE_URL");
    if let Ok(attune_database_url) = std::env::var("ATTUNE_DATABASE_URL") {
        // This sets the environment variable for sqlx's test macros.
        println!("cargo::rustc-env=DATABASE_URL={attune_database_url}");
    }
}
