use proc_macro::{Literal, TokenStream, TokenTree};
use quote::quote;
use std::path::Path;
use std::{fs, path::PathBuf};
use syn::{parse_macro_input, LitStr};

/// Return a string literal containing the absolute path of the workspace root.
///
/// Usage:
/// ```ignore
/// let workspace_root = workspace_root!();
/// ```
#[proc_macro]
pub fn workspace_root(input: TokenStream) -> TokenStream {
    assert!(
        input.is_empty(),
        "workspace_root!() does not take any arguments"
    );

    // HACK: We're using the `../../` to get to the workspace root because we
    // know where this package is located, and the working directory is set to
    // the package root in `cargo test`[^1].
    //
    // [^1]: https://github.com/rust-lang/cargo/issues/11852
    let workspace_root = format!("{}/../..", env!("CARGO_MANIFEST_DIR"));
    TokenTree::Literal(Literal::string(&workspace_root)).into()
}

/// Generate a static migrator from Prisma migrations directory. The provided
/// path should be relative to the workspace root.
///
/// Usage:
/// ```ignore
/// prisma_migrate!("path/to/prisma/migrations")
/// ```
///
/// This generates a `pub static MIGRATOR: sqlx::migrate::Migrator` that can be
/// used with sqlx's migration system.
#[proc_macro]
pub fn prisma_migrate(input: TokenStream) -> TokenStream {
    let input_lit = parse_macro_input!(input as LitStr);
    let migrations_dir = PathBuf::from(input_lit.value());
    if !migrations_dir.exists() {
        return syn::Error::new_spanned(
            input_lit,
            format!("Migrations directory not found: {migrations_dir:?}"),
        )
        .to_compile_error()
        .into();
    }

    let entries = match collect_migrations(&migrations_dir) {
        Ok(entries) => entries,
        Err(err) => {
            return syn::Error::new_spanned(input_lit, format!("Failed to read migrations: {err}"))
                .to_compile_error()
                .into();
        }
    };

    // It's a big pain to do checksums today, so we're putting that off until they
    // are proven to be worthwhile.
    let migrations = entries.into_iter().map(|migration| {
        let version = migration.version;
        let description = migration.description;
        let path = migration.path;
        quote! {
            sqlx::migrate::Migration {
                version: #version,
                description: std::borrow::Cow::Borrowed(#description),
                migration_type: sqlx::migrate::MigrationType::Simple,
                sql: std::borrow::Cow::Borrowed(include_str!(#path)),
                checksum: std::borrow::Cow::Borrowed(&[]),
                no_tx: false,
            }
        }
    });

    let expanded = quote! {
        pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate::Migrator {
            migrations: std::borrow::Cow::Borrowed(&[
                #(#migrations),*
            ]),
            ignore_missing: false,
            locking: true,
            no_tx: false,
        };
    };

    TokenStream::from(expanded)
}

struct PrismaMigration {
    version: i64,
    description: String,
    path: String,
}

fn collect_migrations(dir: &Path) -> Result<Vec<PrismaMigration>, std::io::Error> {
    let mut entries = fs::read_dir(dir)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|e| e.file_name());

    let mut migrations = Vec::new();
    for entry in entries {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let dir = match path.file_name().and_then(|n| n.to_str()) {
            Some(name) => name,
            None => continue,
        };

        let path = path.join("migration.sql");
        if !path.exists() {
            continue;
        }

        // Parse migration directory name format: {timestamp}_{description}
        let Some((version, description)) = dir.split_once('_') else {
            continue;
        };

        let version = match version.parse::<i64>() {
            Ok(v) => v,
            Err(_) => continue,
        };

        // TODO: This is the path from the macro call-site to the migration
        // file, which we need because we retrieve the content of the migration
        // using `include_str!` (which takes a path relative to the source file
        // invoking the macro).
        //
        // Note that this is current hardcoded, and may not work if the macro is
        // called from different modules!
        //
        // The right way to fix this is to adjust the path using the call-site
        // file path. We can do this by going from the current call site path
        // (via `proc_macro::Span::local_file`) to the workspace root (via
        // `cargo metadata`), and then from the workspace root to the migrations
        // directory.
        let path = format!("../../../../{}", path.to_string_lossy());
        let description = description.to_string();
        migrations.push(PrismaMigration {
            version,
            description,
            path,
        });
    }

    Ok(migrations)
}
