fn main() {
    if cfg!(debug_assertions) {
        println!("cargo::rerun-if-env-changed=ATTUNE_DATABASE_URL");
        if let Ok(attune_database_url) = std::env::var("ATTUNE_DATABASE_URL") {
            println!("cargo::rustc-env=DATABASE_URL={attune_database_url}");
        }
    }
}
