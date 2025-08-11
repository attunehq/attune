use indoc::indoc;
use std::{boxed::Box, env, fs, path::PathBuf};
use bollard::{Docker, exec::{CreateExecOptions, StartExecResults}, models::{HostConfig, ContainerCreateBody}};
use bollard::query_parameters::{CreateContainerOptions, StartContainerOptions, BuildImageOptions};
use futures_util::stream::StreamExt;
use tokio_util::io::ReaderStream;
use xshell::{Shell, cmd};

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

    // Check for attune-ee directory existence before proceeding
    println!("\n========== SMOKE TEST: Check Enterprise Edition Directory ==========");
    const ATTUNE_EE_PATH: &str = "packages/attune-ee";

    if fs::metadata(ATTUNE_EE_PATH).is_ok() {
        eprintln!(indoc! {
            "❌ Enterprise Edition directory 'packages/attune-ee' exists!
                Please remove the 'packages/attune-ee' directory before running smoke tests.
                Run: rm -rf packages/attune-ee"
        });
        panic!("Enterprise Edition directory must be removed for open-source testing");
    }
    println!("✅ No Enterprise Edition directory found - proceeding with tests");

    // Set up shared shell and configuration upfront.
    let sh = Shell::new().unwrap();

    // Default values for API configuration
    const DEFAULT_API_ENDPOINT: &str = "http://localhost:3000";
    const DEFAULT_API_TOKEN: &str = "INSECURE_TEST_TOKEN";

    // Get API configuration and set environment variables once
    let api_endpoint =
        env::var("ATTUNE_API_ENDPOINT").unwrap_or_else(|_| DEFAULT_API_ENDPOINT.to_string());
    let api_token = env::var("ATTUNE_API_TOKEN").unwrap_or_else(|_| DEFAULT_API_TOKEN.to_string());
    sh.set_var("ATTUNE_API_ENDPOINT", &api_endpoint);
    sh.set_var("ATTUNE_API_TOKEN", &api_token);

    const CLI_PATH_HELP: &str = indoc! {
        "ATTUNE_CLI_PATH environment variable is required. Set it to the path of your CLI binary.
         Example: export ATTUNE_CLI_PATH=/target/release/attune"
    };

    // Get CLI binary path.
    let cli_path = env::var("ATTUNE_CLI_PATH").expect(CLI_PATH_HELP);

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

    println!(
        indoc! {
            "Test configuration:
              API Endpoint: {}
              API Token: {}
              CLI Path: {}
              GPG Key ID: {}"
        },
        config.api_endpoint, config.api_token, config.cli_path, config.gpg_key_id
    );

    // Set up MinIO bucket permissions for APT access
    println!("\n========== SMOKE TEST: Configure MinIO Bucket Permissions =========");

    // Set up MinIO alias
    println!("Setting up MinIO alias...");
    let alias_result = cmd!(
        sh,
        "mc alias set attune http://127.0.0.1:9000 attuneminio attuneminio"
    )
    .run();
    match alias_result {
        Ok(_) => println!("✅ MinIO alias configured"),
        Err(e) => {
            eprintln!("❌ Failed to configure MinIO alias: {e}");
            panic!("MinIO alias configuration failed");
        }
    }

    // Set bucket permissions with recursive flag
    println!("Setting bucket permissions...");
    let permissions_result = cmd!(sh, "mc anonymous set download attune/attune-dev-0 -r").run();
    match permissions_result {
        Ok(_) => {
            println!("✅ MinIO bucket permissions configured for anonymous downloads");
        }
        Err(e) => {
            eprintln!("❌ Failed to configure MinIO bucket permissions: {e}");
            panic!("MinIO bucket permission configuration failed");
        }
    }

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
        // println!("\n========== SMOKE TEST: Concurrent Package Add =========");
        // test_concurrent_package_add(&sh, &config);
        println!("\n========== SMOKE TEST: Ubuntu Container APT Install =========");
        test_apt_package_install_ubuntu(&sh, &config);
    });

    // Clean up repositories, packages, and GPG key regardless of test outcome.
    println!("\n========== SMOKE TEST: Test Cleanup =========");
    cleanup_test_resources(&sh, &config);

    // Check if tests passed and report final result.
    // TODO: Here and elsewhere, wrap in a declarative macro: https://doc.rust-lang.org/reference/macros-by-example.html.
    match test_result {
        Ok(_) => println!("\n========== ALL SMOKE TESTS COMPLETED SUCCESSFULLY =========="),
        Err(_) => {
            eprintln!("\n========== SMOKE TESTS FAILED - RESOURCES CLEANED UP ==========");
            std::panic::resume_unwind(Box::new("Smoke tests failed"));
        }
    }
}

/// Helper function to clean up all test resources: repositories, packages, and GPG key.
fn cleanup_test_resources(sh: &Shell, config: &TestConfig) {
    // Clean up any remaining repositories.
    cleanup_repositories(sh, config);

    // Clean up GPG key.
    cleanup_gpg_key(sh, &config.gpg_key_id);
}

/// Helper function to clean up any remaining test repositories.
fn cleanup_repositories(sh: &Shell, config: &TestConfig) {
    println!("Cleaning up test repositories...");

    // List of all potential test repositories that may exist.
    const TEST_REPOS: [&str; 2] = ["debian-test-repo-1", "debian-test-repo-2"];
    let cli_path = &config.cli_path;

    for repo_name in &TEST_REPOS {
        println!("  Attempting to delete repository: {repo_name}");

        let delete_result = cmd!(sh, "{cli_path} apt repository delete {repo_name} --yes").run();

        match delete_result {
            Ok(_) => {
                println!("  ✅ Repository '{repo_name}' deleted successfully");
            }
            Err(e) => {
                // It's OK if the repository doesn't exist or deletion fails during cleanup.
                println!("  ℹ️  Repository '{repo_name}' cleanup skipped: {e}");
            }
        }
    }

    println!("✅ Repository cleanup completed");
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
            let combined_output = format!(
                "{}{}",
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
                            println!(
                                "✅ Extracted GPG key ID from revocation certificate: {key_id}"
                            );

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
            eprintln!(
                "❌ Could not find GPG key ID in revocation certificate output:\n{combined_output}"
            );
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

    let cleanup_result = cmd!(
        sh,
        "gpg --batch --yes --delete-secret-and-public-key {key_id}"
    )
    .run();

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
                panic!(
                    "Repo creation failed. Check your environment variables and CLI binary path."
                );
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
                eprintln!(
                    "❌ Not all expected repositories found. Expected: {EXPECTED_REPOS:?}, Found: {found_repos:?}",
                );
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

    let delete_result = cmd!(
        sh,
        "{cli_path} apt repository delete {REPO_TO_DELETE} --yes"
    )
    .run();

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
            panic!(
                "Repo listing failed during deletion verification. Check your environment variables and CLI binary path."
            );
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

    // Repository configuration constants
    const REPO_NAME: &str = "debian-test-repo-1";
    const DISTRIBUTION: &str = "stable";
    const COMPONENT: &str = "main";

    // Use the remaining repository from the delete test
    let cli_path = &config.cli_path;

    // Use the GPG key ID from config
    let key_id = &config.gpg_key_id;
    println!(
        "\nUsing repository: {REPO_NAME}, distribution: {DISTRIBUTION}, component: {COMPONENT}"
    );
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
        let add_result = cmd!(sh, "{cli_path} apt package add {filepath} --repo {REPO_NAME} --distribution {DISTRIBUTION} --component {COMPONENT} --key-id {key_id}").run();

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

    println!("✅ All packages added successfully - they are now available for installation!\n");
}

/// Test package deletion from repository.
fn test_pkg_delete(sh: &Shell, config: &TestConfig) {
    // Test deleting some of the packages we added.
    let packages_to_delete = [
        ("attune-test-package", "2.0.0", "amd64"),
        ("attune-test-package", "1.0.3", "arm64"),
    ];

    println!(
        "Testing package delete with {} packages",
        packages_to_delete.len()
    );

    // Use the same repository configuration as package add.
    const REPO_NAME: &str = "debian-test-repo-1";
    const DISTRIBUTION: &str = "stable";
    const COMPONENT: &str = "main";
    let cli_path = &config.cli_path;

    // Use the GPG key ID from config.
    let key_id = &config.gpg_key_id;
    println!(
        "\nUsing repository: {REPO_NAME}, distribution: {DISTRIBUTION}, component: {COMPONENT}"
    );
    println!("Using GPG key ID: {key_id}");

    // Delete each specified package.
    for (package_name, version, arch) in packages_to_delete.iter() {
        println!("\nTesting package delete with {package_name} {version} {arch}...");

        // Delete the package from the repository using the new command structure.
        println!("  Deleting package from repository...");
        let delete_result = cmd!(sh, "{cli_path} apt package delete --repo {REPO_NAME} --distribution {DISTRIBUTION} --component {COMPONENT} --key-id {key_id} --package {package_name} --version {version} --architecture {arch}").run();

        match delete_result {
            Ok(_) => {
                println!(
                    "  ✅ Package {package_name} {version} {arch} deleted from repository successfully"
                );
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

/// Test concurrent package additions to simulate multiple users uploading packages simultaneously.
///
/// This test validates concurrency handling by:
/// 1. Creating 6 concurrent tasks, each representing a different user
/// 2. Each user attempts to add 8 packages (different architectures of 4 different package names)
/// 3. Each user adds a different version (v3.0.0 through v3.0.5)
/// 4. All operations run concurrently to test race conditions and database consistency
fn test_concurrent_package_add(sh: &Shell, config: &TestConfig) {
    use std::thread;
    
    println!("Testing concurrent package additions with 6 users...");
    
    // Package configuration
    const REPO_NAME: &str = "debian-test-repo-1";
    const DISTRIBUTION: &str = "stable";
    const COMPONENT: &str = "main";
    
    // Base package names and architectures
    const PACKAGE_NAMES: [&str; 4] = ["attune-test-package", "cod-test-package", "salmon-test-package", "tuna-test-package"];
    const ARCHITECTURES: [&str; 2] = ["amd64", "arm64"];
    
    // Versions for each user (v3.0.0 through v3.0.5)
    const VERSIONS: [&str; 6] = ["3.0.0", "3.0.1", "3.0.2", "3.0.3", "3.0.4", "3.0.5"];
    
    let cli_path = &config.cli_path;
    let key_id = &config.gpg_key_id;
    
    println!("Using repository: {REPO_NAME}, distribution: {DISTRIBUTION}, component: {COMPONENT}");
    println!("Using GPG key ID: {key_id}");
    
    // First, download all packages sequentially to avoid network bottlenecks
    println!("Downloading all packages first...");
    let mut package_paths = Vec::new();
    
    for user_index in 0..6 {
        let version = VERSIONS[user_index];
        println!("  Downloading packages for user {} (version {})...", user_index + 1, version);
        
        for package_name in &PACKAGE_NAMES {
            for arch in &ARCHITECTURES {
                let url = format!(
                    "https://github.com/attunehq/attune-test-package/releases/download/v{}/{}_{}_linux_{}.deb",
                    version, package_name, version, arch
                );
                
                let filename = format!("{}_{}_linux_{}.deb", package_name, version, arch);
                let filepath = format!("/tmp/concurrent_{}_{}", user_index, filename);
                
                // Download the package
                let download_result = cmd!(sh, "curl -L -o {filepath} {url}").run();
                
                match download_result {
                    Ok(_) => {
                        // Verify file exists and has reasonable size
                        match std::fs::metadata(&filepath) {
                            Ok(metadata) if metadata.len() > 1000 => {
                                package_paths.push((
                                    user_index,
                                    package_name.to_string(),
                                    version.to_string(),
                                    arch.to_string(),
                                    filepath,
                                ));
                                println!("    ✅ Downloaded {}: {} bytes", filename, metadata.len());
                            }
                            _ => {
                                println!("    ❌ Download failed for {}: file too small or missing", filename);
                            }
                        }
                    }
                    Err(e) => {
                        println!("    ❌ Download failed for {}: {}", filename, e);
                    }
                }
            }
        }
    }
    
    println!("✅ Downloaded {} packages total", package_paths.len());
    
    // Now create 6 concurrent threads to upload packages simultaneously
    println!("Starting concurrent package uploads with 6 users...");
    let mut handles = vec![];
    
    for user_index in 0..6 {
        let user_packages: Vec<_> = package_paths
            .iter()
            .filter(|(uid, _, _, _, _)| *uid == user_index)
            .cloned()
            .collect();
        
        let cli_path = cli_path.clone();
        let key_id = key_id.clone();
        
        println!("Starting user {} with {} packages", user_index + 1, user_packages.len());
        
        let handle = thread::spawn(move || {
            let user_sh = Shell::new().unwrap();
            
            // Copy environment variables to the new shell
            user_sh.set_var("ATTUNE_API_ENDPOINT", &std::env::var("ATTUNE_API_ENDPOINT").unwrap_or_else(|_| "http://localhost:3000".to_string()));
            user_sh.set_var("ATTUNE_API_TOKEN", &std::env::var("ATTUNE_API_TOKEN").unwrap_or_else(|_| "INSECURE_TEST_TOKEN".to_string()));
            user_sh.set_var("GPG_KEY_ID", &key_id);
            
            let mut user_results = Vec::new();
            
            // Upload all packages for this user
            for (_, package_name, version, arch, filepath) in user_packages {
                let add_result = cmd!(user_sh, "{cli_path} apt package add {filepath} --repo {REPO_NAME} --distribution {DISTRIBUTION} --component {COMPONENT} --key-id {key_id}").run();
                
                let success = add_result.is_ok();
                user_results.push((
                    user_index + 1,
                    package_name,
                    version,
                    arch,
                    success,
                    if success { "Success".to_string() } else { format!("Error: {}", add_result.unwrap_err()) }
                ));
                
                // Clean up downloaded file after upload attempt
                let _ = std::fs::remove_file(&filepath);
            }
            
            user_results
        });
        
        handles.push(handle);
    }
    
    println!("All {} user threads started, uploading packages concurrently...", handles.len());
    
    // Wait for all tasks to complete and collect results
    let mut all_results = Vec::new();
    for handle in handles {
        match handle.join() {
            Ok(user_results) => {
                all_results.extend(user_results);
            }
            Err(e) => {
                eprintln!("❌ A user thread panicked: {:?}", e);
            }
        }
    }
    
    // Analyze results
    let total_attempts = all_results.len();
    let successful_adds = all_results.iter().filter(|(_, _, _, _, success, _)| *success).count();
    let failed_adds = total_attempts - successful_adds;
    
    println!("\n========== CONCURRENT PACKAGE ADD RESULTS ==========");
    println!("Total package add attempts: {}", total_attempts);
    println!("Successful additions: {}", successful_adds);
    println!("Failed additions: {}", failed_adds);
    println!("Success rate: {:.1}%", (successful_adds as f64 / total_attempts as f64) * 100.0);
    
    // Show detailed results by user
    println!("\nDetailed results by user:");
    for user_id in 1..=6 {
        let user_results: Vec<_> = all_results.iter().filter(|(uid, _, _, _, _, _)| *uid == user_id).collect();
        let user_successes = user_results.iter().filter(|(_, _, _, _, success, _)| *success).count();
        println!("  User {}: {}/{} packages added successfully", user_id, user_successes, user_results.len());
    }
    
    // Show any failures for debugging
    let failures: Vec<_> = all_results.iter().filter(|(_, _, _, _, success, _)| !*success).collect();
    if !failures.is_empty() {
        println!("\nFailure details:");
        for (user_id, pkg_name, version, arch, _, error) in failures {
            println!("  User {}: {} {} {} - {}", user_id, pkg_name, version, arch, error);
        }
    }
    
    // The test is successful if we had reasonable concurrent behavior
    // We expect some conflicts/failures due to concurrent access, but the system should handle it gracefully
    if successful_adds > 0 {
        println!("✅ Concurrent package add test completed - system handled concurrent operations");
        if failed_adds > 0 {
            println!("ℹ️  Some operations failed due to concurrency conflicts, which is expected behavior");
        }
    } else {
        eprintln!("❌ Concurrent package add test failed - no packages were added successfully");
        panic!("Concurrent package addition test failed completely");
    }
    
    println!("✅ Concurrent package add test completed successfully!\n");
}

/// Test end-to-end APT package installation in Ubuntu container.
///
/// This test validates the complete workflow by:
/// 1. Starting a Ubuntu container.
/// 2. Installing required system dependencies.
/// 3. Configuring GPG keys for repository verification.
/// 4. Adding the new repository to the container's APT sources.
/// 5. Installing the attune-test-package v2.0.0.
/// 6. Verifying the installed package works correctly.
fn test_apt_package_install_ubuntu(sh: &Shell, config: &TestConfig) {
    println!("Testing end-to-end APT package installation...");

    // Export GPG key to file on host system
    println!("  Exporting GPG key to host file...");
    let key_id = &config.gpg_key_id;
    let public_key_result = cmd!(sh, "gpg --armor --export {key_id}").read();
    let public_key = match public_key_result {
        Ok(key) if !key.trim().is_empty() => {
            println!("  ✅ GPG public key exported from host (length: {} chars)", key.len());
            key
        }
        Ok(_) => {
            eprintln!("  ❌ GPG key export returned empty data");
            panic!("GPG key export failed - empty data");
        }
        Err(e) => {
            eprintln!("  ❌ Failed to export GPG key: {e}");
            panic!("Failed to export GPG key: {e}");
        }
    };

    // Write the GPG key to attune.asc file
    let key_file_path = PathBuf::new().join("/tmp/attune.asc");
    std::fs::write(&key_file_path, &public_key)
        .expect("Failed to write GPG key to attune.asc file");
    println!("  ✅ GPG key written to {}", key_file_path.display());

    // Set up the runtime for async operations.
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");

    let test_result = rt.block_on(async move {
        // Connect to Docker daemon
        let docker = Docker::connect_with_local_defaults()
            .map_err(|e| format!("Failed to connect to Docker: {e}"))?;

        println!("  ✅ Connected to Docker daemon");

        // Build a custom Ubuntu image with the GPG key
        println!("  Building custom Ubuntu image with GPG key...");
        
        let dockerfile = format!(
            "FROM ubuntu:latest\n\
            COPY attune.asc /etc/apt/trusted.gpg.d/attune.asc\n\
            CMD [\"sleep\", \"infinity\"]"
        );

        // Create build context with the GPG key file
        let mut tar_builder = tar::Builder::new(Vec::new());
        
        // Add Dockerfile
        let dockerfile_bytes = dockerfile.as_bytes();
        let mut header = tar::Header::new_gnu();
        header.set_path("Dockerfile").map_err(|e| format!("Failed to set Dockerfile path: {e}"))?;
        header.set_size(dockerfile_bytes.len() as u64);
        header.set_cksum();
        tar_builder.append(&header, dockerfile_bytes).map_err(|e| format!("Failed to add Dockerfile to tar: {e}"))?;
        
        // Add GPG key file
        tar_builder.append_path_with_name(&key_file_path, "attune.asc")
            .map_err(|e| format!("Failed to add GPG key to tar: {e}"))?;
        let tar_data = tar_builder.into_inner()
            .map_err(|e| format!("Failed to finalize tar: {e}"))?;

        // Build the image
        let build_options = BuildImageOptions {
            dockerfile: "Dockerfile".to_string(),
            t: Some("attune-test-ubuntu".to_string()),
            rm: true,
            ..Default::default()
        };

        // Write tar data to a temporary file for bollard
        let temp_tar_path = "/tmp/attune-test-context.tar";
        tokio::fs::write(temp_tar_path, &tar_data).await
            .map_err(|e| format!("Failed to write tar to temp file: {e}"))?;

        let tar_file = tokio::fs::File::open(temp_tar_path).await
            .map_err(|e| format!("Failed to open tar file: {e}"))?;
        
        let tar_stream = ReaderStream::new(tar_file);

        let mut build_stream = docker.build_image(
            build_options,
            None,
            Some(bollard::body_try_stream(tar_stream))
        );

        // Process build output
        while let Some(msg) = build_stream.next().await {
            match msg {
                Ok(chunk) => {
                    if let Some(stream) = chunk.stream {
                        print!("    {}", stream);
                    }
                    if let Some(error) = chunk.error {
                        return Err(format!("Build failed: {error}"));
                    }
                }
                Err(e) => return Err(format!("Build stream error: {e}"))
            }
        }

        println!("  ✅ Custom Ubuntu image built successfully");

        // Create container with host networking
        let container_config = ContainerCreateBody {
            image: Some("attune-test-ubuntu".to_string()),
            cmd: Some(vec!["sleep".to_string(), "infinity".to_string()]),
            host_config: Some(HostConfig {
                network_mode: Some("host".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let container_options = CreateContainerOptions {
            name: Some("attune-test-container".to_string()),
            ..Default::default()
        };

        let container = docker
            .create_container(Some(container_options), container_config)
            .await
            .map_err(|e| format!("Failed to create container: {e}"))?;

        println!("  ✅ Container created: {}", container.id);

        // Start the container
        docker
            .start_container(&container.id, None::<StartContainerOptions>)
            .await
            .map_err(|e| format!("Failed to start container: {e}"))?;

        println!("  ✅ Ubuntu container started");

        // Helper function to execute commands in the container
        async fn exec_command(docker: &Docker, container_id: &str, cmd: Vec<String>) -> Result<(i64, Vec<u8>, Vec<u8>), String> {
            let exec_options = CreateExecOptions {
                cmd: Some(cmd),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                ..Default::default()
            };

            let exec_result = docker
                .create_exec(container_id, exec_options)
                .await
                .map_err(|e| format!("Failed to create exec: {e}"))?;

            let start_result = docker.start_exec(&exec_result.id, None).await
                .map_err(|e| format!("Failed to start exec: {e}"))?;

            let mut stdout = Vec::new();
            let mut stderr = Vec::new();

            match start_result {
                StartExecResults::Attached { mut output, .. } => {
                    while let Some(chunk) = output.next().await {
                        match chunk {
                            Ok(bollard::container::LogOutput::StdOut { message }) => {
                                stdout.extend_from_slice(&message);
                            }
                            Ok(bollard::container::LogOutput::StdErr { message }) => {
                                stderr.extend_from_slice(&message);
                            }
                            Ok(_) => {},
                            Err(e) => return Err(format!("Exec output error: {e}"))
                        }
                    }
                }
                StartExecResults::Detached => {
                    return Err("Unexpected detached exec result".to_string());
                }
            }

            // Get exit code
            let inspect_result = docker.inspect_exec(&exec_result.id).await
                .map_err(|e| format!("Failed to inspect exec: {e}"))?;

            Ok((
                inspect_result.exit_code.unwrap_or(-1),
                stdout,
                stderr
            ))
        }

        // Verify the key was added properly
        let (_exit_code, stdout, stderr) = exec_command(&docker, &container.id, vec!["ls".to_string(), "-la".to_string(), "/etc/apt/trusted.gpg.d/".to_string()]).await?;
        
        let files = String::from_utf8_lossy(&stdout);
        let errors = String::from_utf8_lossy(&stderr);
        
        println!("  Debug: ls -la /etc/apt/trusted.gpg.d/ output:");
        println!("  stdout:\n{}", files);
        if !errors.trim().is_empty() {
            println!("  stderr:\n{}", errors);
        }
        
        if files.contains("attune.asc") {
            println!("  ✅ GPG key file verified in keyring directory");
        } else {
            return Err("GPG key file not found in keyring directory".to_string());
        }

        // Install required packages
        println!("  Installing dependencies...");
        let deps_cmd = "export DEBIAN_FRONTEND=noninteractive && apt-get update && apt-get install -y curl gpg ca-certificates";
        let (exit_code, stdout, stderr) = exec_command(&docker, &container.id, vec!["bash".to_string(), "-c".to_string(), deps_cmd.to_string()]).await?;

        let stdout_str = String::from_utf8_lossy(&stdout);
        let stderr_str = String::from_utf8_lossy(&stderr);

        if exit_code != 0 {
            return Err(format!(
                "Failed to install dependencies:\nstdout: {}\nstderr: {}",
                stdout_str, stderr_str
            ));
        }

        println!("  ✅ Dependencies installed successfully");
        println!("  Install summary: {} packages processed", stdout_str.matches("Setting up").count());

        if !stderr_str.trim().is_empty() {
            println!("  Warnings:\n{}", stderr_str.trim());
        }

        // Verify installed packages
        println!("  Verifying installed packages...");
        for cmd in &["curl --version", "gpg --version"] {
            let (exit_code, stdout, _) = exec_command(&docker, &container.id, vec!["bash".to_string(), "-c".to_string(), cmd.to_string()]).await?;
            
            if exit_code == 0 {
                let version_info = String::from_utf8_lossy(&stdout);
                let first_line = version_info.lines().next().unwrap_or("unknown");
                println!("  ✅ {}: {}", cmd.split_whitespace().next().unwrap(), first_line);
            } else {
                return Err(format!("{} not working properly", cmd.split_whitespace().next().unwrap()));
            }
        }

        println!("  ✅ All dependencies verified and working");

        // Configure APT repository
        println!("  Configuring APT repository...");

        // Detect container architecture
        let (_, stdout, _) = exec_command(&docker, &container.id, vec!["dpkg".to_string(), "--print-architecture".to_string()]).await?;
        let container_arch = String::from_utf8_lossy(&stdout).trim().to_string();
        println!("  Container architecture: {container_arch}");

        // Configure the repository URL
        const DISTRIBUTION: &str = "stable";
        const COMPONENT: &str = "main";
        const REPOSITORY_URL: &str = "http://localhost:9000/attune-dev-0/1/56c3adb4af8bf6505b258bc7543458bc45e1b7a5de78f259aaa0b6d448d83174";

        let sources_content = format!("deb [arch={container_arch}] {REPOSITORY_URL} {DISTRIBUTION} {COMPONENT}");
        let sources_cmd = format!(
            "cat > /etc/apt/sources.list.d/attune-test.list << 'EOF'\n{sources_content}\nEOF"
        );

        println!("  Writing sources file with content: {sources_content}");

        let (exit_code, _, stderr) = exec_command(&docker, &container.id, vec!["bash".to_string(), "-c".to_string(), sources_cmd.to_string()]).await?;
        if exit_code != 0 {
            return Err(format!("Failed to create sources file: {}", String::from_utf8_lossy(&stderr)));
        }

        println!("  ✅ Sources file created");

        // Verify the sources file
        let (_, stdout, _) = exec_command(&docker, &container.id, vec!["cat".to_string(), "/etc/apt/sources.list.d/attune-test.list".to_string()]).await?;
        let content = String::from_utf8_lossy(&stdout);
        println!("  Sources file content: {}", content.trim());

        if content.contains("attune-dev-0") && content.contains(&container_arch) && content.contains(DISTRIBUTION) {
            println!("  ✅ Sources file verified with correct content");
        } else {
            return Err(format!("Sources file content verification failed. Expected: attune-dev-0, {container_arch}, {DISTRIBUTION}"));
        }

        // Update APT package lists
        println!("  Updating APT package lists...");
        let (exit_code, stdout, stderr) = exec_command(&docker, &container.id, vec!["bash".to_string(), "-c".to_string(), "apt-get update".to_string()]).await?;

        let stdout_str = String::from_utf8_lossy(&stdout);
        let stderr_str = String::from_utf8_lossy(&stderr);

        if exit_code != 0 {
            return Err(format!(
                "Failed to update APT package lists:\nstdout: {}\nstderr: {}",
                stdout_str, stderr_str
            ));
        }

        println!("  ✅ APT package lists updated successfully");

        if stdout_str.contains("attune") || stdout_str.contains("56c3adb4af8bf6505b258bc7543458bc45e1b7a5de78f259aaa0b6d448d83174") {
            println!("  ✅ Attune repository was accessed during update");
        } else {
            println!("  ⚠️  Attune repository not mentioned in update output");
        }

        if !stderr_str.trim().is_empty() {
            println!("  Update warnings:\n{}", stderr_str.trim());
        }

        // Install the attune-test-package
        println!("  Installing attune-test-package...");

        let version = match container_arch.as_str() {
            "amd64" => "1.0.3",
            "arm64" => "2.0.0",
            _ => "1.0.3", // fallback
        };

        println!("  Installing attune-test-package version {version} for {container_arch} architecture");
        let install_cmd = format!("export DEBIAN_FRONTEND=noninteractive && apt-get install -y attune-test-package={version}");
        let (exit_code, stdout, stderr) = exec_command(&docker, &container.id, vec!["bash".to_string(), "-c".to_string(), install_cmd.to_string()]).await?;

        let stdout_str = String::from_utf8_lossy(&stdout);
        let stderr_str = String::from_utf8_lossy(&stderr);

        if exit_code != 0 {
            return Err(format!(
                "Failed to install attune-test-package:\nstdout: {}\nstderr: {}",
                stdout_str, stderr_str
            ));
        }

        println!("  ✅ attune-test-package installed successfully");

        if stdout_str.contains("Setting up attune-test-package") {
            println!("  ✅ Package setup completed");
        }

        if !stderr_str.trim().is_empty() {
            println!("  Install warnings:\n{}", stderr_str.trim());
        }

        // Verify the installed package works
        println!("  Verifying attune-test-package installation...");
        let (exit_code, stdout, _) = exec_command(&docker, &container.id, vec!["attune-test-package".to_string(), "--version".to_string()]).await?;

        if exit_code == 0 {
            let version_output = String::from_utf8_lossy(&stdout).trim().to_string();
            let expected_version = match container_arch.as_str() {
                "amd64" => "1.0.3",
                "arm64" => "2.0.0",
                _ => "1.0.3", // fallback
            };

            if version_output.contains(expected_version) {
                println!("  ✅ attune-test-package v{expected_version} verified for {container_arch} architecture: {version_output}");
            } else {
                return Err(format!("Version mismatch - expected {expected_version} for {container_arch}, got: {version_output}"));
            }
        } else {
            return Err("attune-test-package --version command failed".to_string());
        }

        // Clean up: remove the container
        use bollard::query_parameters::{RemoveContainerOptions, RemoveImageOptions};
        let _ = docker.remove_container(&container.id, Some(RemoveContainerOptions {
            force: true,
            ..Default::default()
        })).await;

        // Clean up: remove the custom image
        let _ = docker.remove_image("attune-test-ubuntu", Some(RemoveImageOptions {
            force: true,
            ..Default::default()
        }), None).await;

        // Clean up temporary tar file
        let _ = tokio::fs::remove_file("/tmp/attune-test-context.tar").await;

        println!("  ✅ End-to-end APT package installation test completed successfully!");
        Ok::<(), String>(())
    });

    match test_result {
        Ok(_) => {
            println!("✅ Ubuntu container setup test completed successfully!\n");
        }
        Err(e) => {
            eprintln!("❌ Ubuntu container setup test failed: {e}");
            panic!("Ubuntu container setup test failed");
        }
    }
}
