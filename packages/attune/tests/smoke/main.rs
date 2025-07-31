use std::env;
use std::fs;
use xshell::{Shell, cmd};

/// Helper function to get the CLI binary path.
fn get_cli_binary() -> String {
    env::var("ATTUNE_CLI_PATH").expect(
        "ATTUNE_CLI_PATH environment variable is required. Set it to the path of your CLI binary.\n\
         Example: export ATTUNE_CLI_PATH=/Users/xinding/go/bin/attune",
    )
}

/// Helper function to set up environment variables for testing.
fn setup_test_env(sh: &Shell) {
    // Set API token.
    if let Ok(token) = env::var("ATTUNE_API_TOKEN") {
        sh.set_var("ATTUNE_API_TOKEN", token);
    } else {
        sh.set_var("ATTUNE_API_TOKEN", "INSECURE_TEST_TOKEN");
    }

    // Set API endpoint.
    if let Ok(endpoint) = env::var("ATTUNE_API_ENDPOINT") {
        sh.set_var("ATTUNE_API_ENDPOINT", endpoint);
    } else {
        sh.set_var("ATTUNE_API_ENDPOINT", "http://localhost:3000");
    }
}

#[test]
fn test_cli_binary_exists() {
    println!("\n========== SMOKE TEST: Attune CLI Exists =========");

    let cli = get_cli_binary();
    let sh = Shell::new().unwrap();

    let version_result = cmd!(sh, "{cli} --help").run();

    match version_result {
        Ok(_) => {
            println!("✅ CLI binary is accessible and executable");
            println!("✅ Test completed successfully!\n");
        }
        Err(e) => {
            eprintln!("❌ CLI binary test failed: {e}");
            eprintln!("Make sure the CLI is built and the path is correct.");
            eprintln!("Current CLI path: {cli}");
            eprintln!(
                "You can set ATTUNE_CLI_PATH environment variable to specify the correct path."
            );
            panic!("CLI binary not accessible");
        }
    }
}

#[test]
fn test_repo_create() {
    println!("\n========== SMOKE TEST: Repository Create =========");

    let sh = Shell::new().unwrap();
    setup_test_env(&sh);

    let cli = get_cli_binary();

    // Use fixed Debian repo name for testing.
    let debian_repo_url = "http://localhost:9000/debian";

    println!("Testing with Debian repo URL: {debian_repo_url}");

    // Step 1: Create a new Debian repo.
    println!("\nStep 1: Creating Debian repo...");
    let create_result = cmd!(sh, "{cli} repo create -u {debian_repo_url}").run();

    match create_result {
        Ok(_) => println!("✅ Repo creation command executed successfully"),
        Err(e) => {
            eprintln!("❌ Repo creation failed: {e}");
            panic!("Repo creation failed. Check your environment variables and CLI binary path.");
        }
    }

    // Step 2: List repos and verify our repo exists.
    println!("\nStep 2: Listing repos to verify creation...");
    let list_output = cmd!(sh, "{cli} repo list").read();

    match list_output {
        Ok(output) => {
            println!("Repo list output:");
            println!("{output}");

            if output.contains(debian_repo_url) {
                println!("✅ Repo '{debian_repo_url}' found in list");
                println!("✅ Test completed successfully!\n");
            } else {
                eprintln!("❌ Repo '{debian_repo_url}' not found in list");
                eprintln!("Available repos:\n{output}");
                panic!("Created repo not found in list");
            }
        }
        Err(e) => {
            eprintln!("❌ Failed to list repos: {e}");
            panic!("Repo listing failed. Check your environment variables and CLI binary path.");
        }
    }
}

#[test]
fn test_pkg_add() {
    println!("\n========== SMOKE TEST: Package Add ==========");

    let sh = Shell::new().unwrap();
    setup_test_env(&sh);

    let cli = get_cli_binary();

    // Test packages to download.
    let test_packages = [
        (
            "v2.0.0",
            "amd64",
            "https://github.com/attunehq/attune-test-package/releases/download/v2.0.0/attune-test-package_2.0.0_linux_amd64.deb",
        ),
        (
            "v2.0.0",
            "arm64",
            "https://github.com/attunehq/attune-test-package/releases/download/v2.0.0/attune-test-package_2.0.0_linux_arm64.deb",
        ),
        (
            "v1.0.3",
            "amd64",
            "https://github.com/attunehq/attune-test-package/releases/download/v1.0.3/attune-test-package_1.0.3_linux_amd64.deb",
        ),
        (
            "v1.0.3",
            "arm64",
            "https://github.com/attunehq/attune-test-package/releases/download/v1.0.3/attune-test-package_1.0.3_linux_arm64.deb",
        ),
    ];

    println!("Testing package add with {} packages", test_packages.len());

    // Step 1: Use repo ID 1 (for now).
    // TODO: Get repo ID from list command.
    let repo_id = "1";
    println!("\nStep 1: Using repository ID: {repo_id}");

    // Step 2: Download and add each package.
    for (version, arch, url) in test_packages.iter() {
        println!("\nStep 2: Testing pkg add with {version} {arch} ({url})...");

        let filename = format!("attune-test-package_{version}_{arch}.deb");
        let filepath = format!("/tmp/{filename}");

        // Download the package.
        println!("  Downloading package to {filepath}...");
        let download_result = cmd!(sh, "curl -L -o {filepath} {url}").run();

        match download_result {
            Ok(_) => {
                println!("  ✅ Package downloaded successfully");

                // Verify file exists and is reasonable sized.
                match fs::metadata(&filepath) {
                    Ok(metadata) => {
                        let size = metadata.len();
                        if size > 1000 {
                            // At least 1KB.
                            println!("  ✅ Package file size: {size} bytes");
                        } else {
                            eprintln!("  ❌ Package file too small: {size} bytes");
                            panic!("Downloaded package file is too small");
                        }
                    }
                    Err(e) => {
                        eprintln!("  ❌ Could not read package file metadata: {e}");
                        panic!("Package file not accessible after download");
                    }
                }
            }
            Err(e) => {
                eprintln!("  ❌ Package download failed: {e}");
                panic!("Failed to download package from {url}");
            }
        }

        // Add the package to the repository.
        println!("  Adding package to repository...");
        let add_result = cmd!(sh, "{cli} repo pkg add --repo-id {repo_id} {filepath}").run();

        match add_result {
            Ok(_) => {
                println!("  ✅ Package added to repository successfully");
            }
            Err(e) => {
                eprintln!("  ❌ Package add failed: {e}");
                eprintln!(
                    "  Make sure the repository exists and the CLI command syntax is correct"
                );
                panic!("Failed to add package {version} {arch} to repository");
            }
        }

        // Clean up downloaded file.
        if let Err(e) = fs::remove_file(&filepath) {
            eprintln!("  ⚠️  Warning: Could not clean up downloaded file {filepath}: {e}");
        }
    }
}

// TODO: Update existing smoke tests and add more once v0.2.0 is released.
