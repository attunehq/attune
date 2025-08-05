use std::{boxed::Box, env, fs};
use xshell::{Shell, cmd};
use indoc::indoc;

/// Configuration for smoke tests.
#[derive(Debug, Clone)]
struct TestConfig {
    api_endpoint: String,
    api_token: String,
    cli_path: String,
    gpg_key_id: String,
}

#[test]
fn smoke() {
    println!("\n========== RUNNING ATTUNE CLI SMOKE TESTS ==========");
    
    // Set up shared shell and configuration upfront.
    let sh = Shell::new().unwrap();
    
    // Get API configuration and set environment variables once
    let api_endpoint = env::var("ATTUNE_API_ENDPOINT").unwrap_or_else(|_| "http://localhost:3000".to_string());
    let api_token = env::var("ATTUNE_API_TOKEN").unwrap_or_else(|_| "INSECURE_TEST_TOKEN".to_string());
    sh.set_var("ATTUNE_API_ENDPOINT", &api_endpoint);
    sh.set_var("ATTUNE_API_TOKEN", &api_token);
    
    // Get CLI binary path.
    let cli_path = env::var("ATTUNE_CLI_PATH").expect(
        "ATTUNE_CLI_PATH environment variable is required. Set it to the path of your CLI binary.\n\
         Example: export ATTUNE_CLI_PATH=/target/release/attune",
    );
    
    // Set up GPG key for testing.
    println!("\n========== SMOKE TEST: Set Up GPG Key =========");
    let gpg_key_id = set_up_gpg_key(&sh);
    
    // Create test configuration
    let config = TestConfig {
        api_endpoint,
        api_token,
        cli_path,
        gpg_key_id,
    };
    
    println!("Test configuration:");
    println!("  API Endpoint: {}", config.api_endpoint);
    println!("  API Token: {}", config.api_token);
    println!("  CLI Path: {}", config.cli_path);
    println!("  GPG Key ID: {}", config.gpg_key_id);

    // Run tests in order and ensure GPG key cleanup happens regardless of test outcome.
    let test_result = std::panic::catch_unwind(|| {
        println!("\n========== SMOKE TEST: Check Attune CLI Exists =========");
        test_cli_binary_exists(&sh, &config);
        println!("\n========== SMOKE TEST: Repository Create (2 repos) =========");
        test_repo_create_multiple(&sh, &config);
        println!("\n========== SMOKE TEST: Repository List (verify both repos) =========");
        test_repo_list_multiple(&sh, &config);
        println!("\n========== SMOKE TEST: Repository Delete =========");
        test_repo_delete(&sh, &config);
        println!("\n========== SMOKE TEST: Verify Repository Deleted =========");
        test_repo_delete_verification(&sh, &config);
        println!("\n========== SMOKE TEST: Package Add =========");
        test_pkg_add(&sh, &config);
        println!("\n========== SMOKE TEST: Package Delete =========");
        test_pkg_delete(&sh, &config);
    });
    
    // Clean up GPG key regardless of test outcome.
    println!("\n========== SMOKE TEST: GPG Key Cleanup =========");
    cleanup_gpg_key(&sh, &config.gpg_key_id);
    
    // Check if tests passed and report final result.
    // TODO: Here and elsewhere, wrap in a declarative macro: https://doc.rust-lang.org/reference/macros-by-example.html.
    match test_result {
        Ok(_) => println!("\n========== ALL SMOKE TESTS COMPLETED SUCCESSFULLY =========="),
        Err(_) => {
            eprintln!("\n========== SMOKE TESTS FAILED - GPG KEY CLEANED UP ==========");
            std::panic::resume_unwind(Box::new("Smoke tests failed"));
        }
    }
}


/// Helper function to set up GPG key for testing.
fn set_up_gpg_key(sh: &Shell) -> String {
    println!("Setting up GPG key for testing...");
    
    // Create GPG key configuration file.
    let gpg_config = indoc! {"
        Key-Type: EDDSA
        Key-Curve: Ed25519
        Subkey-Type: ECDH
        Subkey-Curve: Cv25519
        Name-Real: Attune Test
        Name-Email: test@attunehq.com
        Expire-Date: 0
        %no-protection
        %commit
    "};
    
    const CONFIG_PATH: &str = "/tmp/gpg_key_config.txt";
    
    match fs::write(CONFIG_PATH, gpg_config) {
        Ok(_) => println!("✅ GPG config file created successfully"),
        Err(e) => {
            eprintln!("❌ Failed to create GPG config file: {e}");
            panic!("GPG config file creation failed");
        }
    }
    
    // Generate GPG key using batch mode and capture output.
    println!("Generating GPG key (this may take a moment)...");
    let gpg_cmd = cmd!(sh, "gpg --batch --generate-key {CONFIG_PATH}");
    let gpg_generate_result = gpg_cmd.output();
    
    match gpg_generate_result {
        Ok(output) => {
            if !output.status.success() {
                eprintln!(
                    indoc! {"
                        ❌ GPG key generation failed with exit code: {}
                        stdout: {}
                        stderr: {}
                    "},
                    output.status,
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
                panic!("GPG key generation failed. Make sure GPG is installed and configured.");
            }
            
            println!("✅ GPG key generated successfully");
            let combined_output = format!("{}{}", 
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            println!("GPG output:\n{combined_output}");
            
            // Parse the GPG output to find the key ID from the revocation certificate message.
            // TODO: Use nom to parse the output.
            for line in combined_output.lines() {
                if line.contains("revocation certificate stored as") && line.contains(".rev'") {
                    // Extract the key ID from the filename.
                    if let Some(start) = line.rfind('/') {
                        if let Some(end) = line.rfind(".rev'") {
                            let key_id = line[(start + 1)..end].to_string();
                            println!("✅ Extracted GPG key ID from revocation certificate: {key_id}");
                            
                            // Clean up config file.
                            if let Err(e) = fs::remove_file(CONFIG_PATH) {
                                eprintln!("⚠️  Warning: Could not clean up GPG config file: {e}");
                            }
                            
                            // Set the GPG_KEY_ID environment variable.
                            sh.set_var("GPG_KEY_ID", &key_id);
                            println!("✅ GPG_KEY_ID environment variable set");
                            
                            return key_id;
                        }
                    }
                }
            }
            eprintln!("❌ Could not find GPG key ID in revocation certificate output:\n{combined_output}");
            panic!("GPG key ID not found in generation output");
        }
        Err(e) => {
            eprintln!("❌ GPG key generation failed: {e}");
            panic!("GPG key generation failed. Make sure GPG is installed and configured.");
        }
    };
}

/// Helper function to clean up GPG key after testing.
fn cleanup_gpg_key(sh: &Shell, key_id: &str) {
    println!("Cleaning up GPG key: {key_id}");
    
    let cleanup_result = cmd!(sh, "gpg --batch --yes --delete-secret-and-public-key {key_id}").run();
    
    match cleanup_result {
        Ok(_) => {
            println!("✅ GPG key {key_id} deleted successfully");
        }
        Err(e) => {
            eprintln!(
                indoc! {"
                    ⚠️  Warning: Failed to delete GPG key {}: {}
                       You may need to manually clean up the key using:
                       gpg --delete-secret-and-public-key --yes {}
                "},
                key_id, e, key_id
            );
        }
    }
}

/// Test that the CLI binary exists and is executable.
fn test_cli_binary_exists(sh: &Shell, config: &TestConfig) {

    let cli_path = &config.cli_path;
    let version_result = cmd!(sh, "{cli_path} --help").run();

    match version_result {
        Ok(_) => {
            println!("CLI binary is accessible and executable");
            println!("✅ Test completed successfully!\n");
        }
        Err(e) => {
            eprintln!(
                indoc! {"
                    ❌ CLI binary test failed: {}
                    Make sure the CLI is built and the path is correct.
                    Current CLI path: {}
                    You can set ATTUNE_CLI_PATH environment variable to specify the correct path.
                "},
                e, config.cli_path
            );
            panic!("CLI binary not accessible");
        }
    }
}

/// Test creating multiple repositories.
fn test_repo_create_multiple(sh: &Shell, config: &TestConfig) {

    // Create two different Debian repos for testing.
    const REPO_NAMES: [&str; 2] = ["debian-test-repo-1", "debian-test-repo-2"];
    let cli_path = &config.cli_path;

    for repo_name in &REPO_NAMES {
        println!("Creating Debian repo: {repo_name}");
        
        let create_result = cmd!(sh, "{cli_path} apt repository create {repo_name}").run();

        match create_result {
            Ok(_) => println!("✅ Repo '{repo_name}' creation command executed successfully"),
            Err(e) => {
                eprintln!("❌ Repo creation failed for '{repo_name}': {e}");
                panic!("Repo creation failed. Check your environment variables and CLI binary path.");
            }
        }
    }
    
    println!("✅ Both repositories created successfully!\n");
}

/// Test listing repositories to verify both exist.
fn test_repo_list_multiple(sh: &Shell, config: &TestConfig) {
    const EXPECTED_REPOS: [&str; 2] = ["debian-test-repo-1", "debian-test-repo-2"];
    let cli_path = &config.cli_path;

    println!("Listing repos to verify both repositories exist...");
    let list_output = cmd!(sh, "{cli_path} apt repository list").read();

    match list_output {
        Ok(output) => {
            println!("Repo list output:");
            println!("{output}");

            let mut found_repos = Vec::new();
            for repo_name in &EXPECTED_REPOS {
                if output.contains(repo_name) {
                    println!("✅ Repo '{repo_name}' found in list");
                    found_repos.push(*repo_name);
                } else {
                    eprintln!("❌ Repo '{repo_name}' not found in list");
                }
            }
            // TODO: Here and elsewhere, use println! for machine readable output and eprintln! for human readable output.
            if found_repos.len() == EXPECTED_REPOS.len() {
                println!("✅ All expected repositories found in list");
                println!("✅ Test completed successfully!\n");
            } else {
                eprintln!("❌ Not all expected repositories found. Expected: {:?}, Found: {:?}", EXPECTED_REPOS, found_repos);
                eprintln!("Available repos:\n{output}");
                panic!("Not all created repos found in list");
            }
        }
        Err(e) => {
            eprintln!("❌ Failed to list repos: {e}");
            panic!("Repo listing failed. Check your environment variables and CLI binary path.");
        }
    }
}

/// Test deleting a repository.
fn test_repo_delete(sh: &Shell, config: &TestConfig) {
    const REPO_TO_DELETE: &str = "debian-test-repo-2";
    let cli_path = &config.cli_path;

    println!("Deleting repository: {REPO_TO_DELETE}");
    
    let delete_result = cmd!(sh, "{cli_path} apt repository delete {REPO_TO_DELETE} --yes").run();

    match delete_result {
        Ok(_) => {
            println!("✅ Repo '{REPO_TO_DELETE}' deletion command executed successfully");
            println!("✅ Test completed successfully!\n");
        }
        Err(e) => {
            eprintln!("❌ Repo deletion failed for '{REPO_TO_DELETE}': {e}");
            panic!("Repo deletion failed. Check your environment variables and CLI binary path.");
        }
    }
}

/// Test verifying that the deleted repository is no longer in the list.
fn test_repo_delete_verification(sh: &Shell, config: &TestConfig) {
    const DELETED_REPO: &str = "debian-test-repo-2";
    const REMAINING_REPO: &str = "debian-test-repo-1";
    let cli_path = &config.cli_path;

    println!("Verifying repository deletion by listing repos...");
    let list_output = cmd!(sh, "{cli_path} apt repository list").read();

    match list_output {
        Ok(output) => {
            println!("Repo list output after deletion:");
            println!("{output}");

            let deleted_repo_exists = output.contains(DELETED_REPO);
            let remaining_repo_exists = output.contains(REMAINING_REPO);

            if !deleted_repo_exists && remaining_repo_exists {
                println!("✅ Deleted repo '{DELETED_REPO}' no longer in list");
                println!("✅ Remaining repo '{REMAINING_REPO}' still in list");
                println!("✅ Test completed successfully!\n");
            } else {
                if deleted_repo_exists {
                    eprintln!("❌ Deleted repo '{DELETED_REPO}' still appears in list");
                }
                if !remaining_repo_exists {
                    eprintln!("❌ Remaining repo '{REMAINING_REPO}' not found in list");
                }
                eprintln!("Available repos:\n{output}");
                panic!("Repository deletion verification failed");
            }
        }
        Err(e) => {
            eprintln!("❌ Failed to list repos for verification: {e}");
            panic!("Repo listing failed during deletion verification. Check your environment variables and CLI binary path.");
        }
    }
}

/// Test package addition to repository.
fn test_pkg_add(sh: &Shell, config: &TestConfig) {

    // Test packages to download.
    const TEST_PACKAGES: [(&str, &str, &str, &str); 4] = [
        (
            "attune-test-package",
            "2.0.0", 
            "amd64",
            "https://github.com/attunehq/attune-test-package/releases/download/v2.0.0/attune-test-package_2.0.0_linux_amd64.deb",
        ),
        (
            "attune-test-package",
            "2.0.0",
            "arm64", 
            "https://github.com/attunehq/attune-test-package/releases/download/v2.0.0/attune-test-package_2.0.0_linux_arm64.deb",
        ),
        (
            "attune-test-package",
            "1.0.3",
            "amd64",
            "https://github.com/attunehq/attune-test-package/releases/download/v1.0.3/attune-test-package_1.0.3_linux_amd64.deb",
        ),
        (
            "attune-test-package",
            "1.0.3",
            "arm64",
            "https://github.com/attunehq/attune-test-package/releases/download/v1.0.3/attune-test-package_1.0.3_linux_arm64.deb",
        ),
    ];

    println!("Testing package add with {} packages", TEST_PACKAGES.len());

    // Use the remaining repository from the delete test
    let repo_name = "debian-test-repo-1";
    let distribution = "stable";
    let component = "main";
    let cli_path = &config.cli_path;
    
    // Use the GPG key ID from config
    let key_id = &config.gpg_key_id;
    println!("\nUsing repository: {repo_name}, distribution: {distribution}, component: {component}");
    println!("Using GPG key ID: {key_id}");

    // Download and add each package.
    for (package_name, version, arch, url) in TEST_PACKAGES.iter() {
        println!("\nTesting package add with {package_name} {version} {arch} ({url})...");

        let filename = format!("{package_name}_{version}_linux_{arch}.deb");
        let filepath = format!("/tmp/{filename}");

        // Download the package.
        println!("  Downloading package to {filepath}...");
        let download_result = cmd!(sh, "curl -L -o {filepath} {url}").run();

        match download_result {
            Ok(_) => {
                println!("  ✅ Package downloaded successfully");

                // Verify file exists.
                match fs::metadata(&filepath) {
                    Ok(metadata) => {
                        let size = metadata.len();
                        if size > 1000 {
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

        // Add the package to the repository using the new command structure.
        println!("  Adding package to repository...");
        let add_result = cmd!(sh, "{cli_path} apt package add {filepath} --repo {repo_name} --distribution {distribution} --component {component} --key-id {key_id}").run();

        match add_result {
            Ok(_) => {
                println!("  ✅ Package added to repository successfully");
            }
            Err(e) => {
                eprintln!(
                    indoc! {"
                        ❌ Package add failed: {}
                        Make sure the repository exists and the CLI command syntax is correct
                    "},
                    e
                );
                panic!("Failed to add package {package_name} {version} {arch} to repository");
            }
        }

        // Clean up downloaded file.
        if let Err(e) = fs::remove_file(&filepath) {
            eprintln!("  ⚠️  Warning: Could not clean up downloaded file {filepath}: {e}");
        }
    }
}

/// Test package deletion from repository.
fn test_pkg_delete(sh: &Shell, config: &TestConfig) {

    // Test deleting some of the packages we added.
    let packages_to_delete = [
        ("attune-test-package", "2.0.0", "amd64"),
        ("attune-test-package", "1.0.3", "arm64"),
    ];

    println!("Testing package delete with {} packages", packages_to_delete.len());

    // Use the same repository configuration as package add.
    const REPO_NAME: &str = "debian-test-repo-1";
    const DISTRIBUTION: &str = "stable";
    const COMPONENT: &str = "main";
    let cli_path = &config.cli_path;
    
    // Use the GPG key ID from config.
    let key_id = &config.gpg_key_id;
    println!("\nUsing repository: {REPO_NAME}, distribution: {DISTRIBUTION}, component: {COMPONENT}");
    println!("Using GPG key ID: {key_id}");

    // Delete each specified package.
    for (package_name, version, arch) in packages_to_delete.iter() {
        println!("\nTesting package delete with {package_name} {version} {arch}...");

        // Delete the package from the repository using the new command structure.
        println!("  Deleting package from repository...");
        let delete_result = cmd!(sh, "{cli_path} apt package delete --repo {REPO_NAME} --distribution {DISTRIBUTION} --component {COMPONENT} --key-id {key_id} --package {package_name} --version {version} --architecture {arch}").run();

        match delete_result {
            Ok(_) => {
                println!("  ✅ Package {package_name} {version} {arch} deleted from repository successfully");
            }
            Err(e) => {
                eprintln!(
                    indoc! {"
                        ❌ Package delete failed: {}
                        Make sure the repository exists, the package was added, and the CLI command syntax is correct
                    "},
                    e
                );
                panic!("Failed to delete package {package_name} {version} {arch} from repository");
            }
        }
    }
    
    println!("✅ Package deletion tests completed successfully!\n");
}
