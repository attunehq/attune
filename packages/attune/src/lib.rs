pub mod api;
pub mod apt;
pub mod cli;
pub mod server;

// We can't make the whole module `#[cfg(test)]`, because the `MIGRATOR` it
// needs is exported and required for the integration test (which is in another
// crate).
pub mod testing;
