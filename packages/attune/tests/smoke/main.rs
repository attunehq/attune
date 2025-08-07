use indoc::indoc;
use std::{boxed::Box, env, fs};
use testcontainers::{ImageExt, core::ExecCommand};
use tokio::io::AsyncReadExt;
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
    use testcontainers::{GenericImage, runners::AsyncRunner};

    println!("Testing end-to-end APT package installation...");

    // Set up the runtime for async operations.
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");

    let test_result = rt.block_on(async {
        // Start Ubuntu container with network access to host.
        // Use sleep to keep the container running.
        let ubuntu_image = GenericImage::new("ubuntu", "latest")
            .with_network("host")
            .with_cmd(["sleep", "infinity"]);

        println!("  Starting Ubuntu container...");
        let container = ubuntu_image
            .start()
            .await
            .expect("Failed to start Ubuntu container");

        println!("  ✅ Ubuntu container started");

        // Install required packages.
        println!("  Installing dependencies...");
        const DEPS_CMD: &str = "export DEBIAN_FRONTEND=noninteractive && apt-get update && apt-get install -y curl gpg ca-certificates";
        let deps_cmd = DEPS_CMD;
        let deps_result = container
            .exec(ExecCommand::new(["bash", "-c", deps_cmd]))
            .await;

        match deps_result {
            Ok(mut output) => {
                let exit_code = output.exit_code().await.unwrap_or(Some(-1));
                println!("  Debug: dependency installation exit code: {exit_code:?}");

                // Read output regardless of exit code to see what happened.
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                let _ = output.stdout().read_to_end(&mut stdout).await;
                let _ = output.stderr().read_to_end(&mut stderr).await;

                let stdout_str = String::from_utf8_lossy(&stdout);
                let stderr_str = String::from_utf8_lossy(&stderr);

                // Accept both Some(0) and None as success since testcontainers may not track exit codes properly.
                if exit_code != Some(0) && exit_code.is_some() {
                    eprintln!(
                        indoc! {"
                            ❌ Failed to install dependencies:
                            stdout: {}
                            stderr: {}
                        "},
                        stdout_str,
                        stderr_str
                    );
                    return Err("Failed to install dependencies");
                } else {
                    println!("  ✅ Dependencies installed successfully");
                    println!("  Install summary: {} packages processed", stdout_str.matches("Setting up").count());

                    // Show any warnings in stderr.
                    if !stderr_str.trim().is_empty() {
                        println!("  Warnings:\n{}", stderr_str.trim());
                    }
                }
            }
            Err(e) => {
                eprintln!("  ❌ Failed to execute dependency installation: {e}");
                return Err("Failed to execute dependency installation");
            }
        }

        // Verify the installed packages are actually available.
        println!("  Verifying installed packages...");
        let verify_cmds = ["curl --version", "gpg --version"];

        for cmd in &verify_cmds {
            let verify_result = container
                .exec(ExecCommand::new(["bash", "-c", cmd]))
                .await;

            match verify_result {
                Ok(mut output) => {
                    let exit_code = output.exit_code().await.unwrap_or(Some(-1));
                    if exit_code == Some(0) || exit_code.is_none() {
                        let mut stdout = Vec::new();
                        let _ = output.stdout().read_to_end(&mut stdout).await;
                        let version_info = String::from_utf8_lossy(&stdout);
                        let first_line = version_info.lines().next().unwrap_or("unknown");
                        println!("  ✅ {}: {}", cmd.split_whitespace().next().unwrap(), first_line);
                    } else {
                        println!("  ❌ {} not working properly", cmd.split_whitespace().next().unwrap());
                        return Err("Package verification failed");
                    }
                }
                Err(e) => {
                    println!("  ❌ Failed to verify {}: {}", cmd.split_whitespace().next().unwrap(), e);
                    return Err("Package verification failed");
                }
            }
        }

        println!("  ✅ All dependencies verified and working");

        // Step 2: Export GPG public key from host and configure in container.
        println!("  Setting up GPG key...");
        let key_id = &config.gpg_key_id;

        // Export the GPG public key from the host system.
        let public_key_result = cmd!(sh, "gpg --armor --export {key_id}").read();
        let public_key = match public_key_result {
            Ok(key) if !key.trim().is_empty() => {
                println!("  ✅ GPG public key exported from host (length: {} chars)", key.len());
                key
            }
            Ok(_) => {
                eprintln!("  ❌ GPG key export returned empty data");
                return Err("GPG key export failed - empty data");
            }
            Err(e) => {
                eprintln!("  ❌ Failed to export GPG key: {e}");
                return Err("Failed to export GPG key");
            }
        };

        // Import the GPG key into the container's APT keyring
        let key_setup_cmd = format!(
            "cat > /etc/apt/trusted.gpg.d/attune.asc << 'EOF'\n{public_key}\nEOF"
        );

        let key_result = container
            .exec(ExecCommand::new(["bash", "-c", &key_setup_cmd]))
            .await;

        match key_result {
            Ok(mut output) => {
                let exit_code = output.exit_code().await.unwrap_or(Some(-1));
                if exit_code != Some(0) && exit_code.is_some() {
                    let mut stderr = Vec::new();
                    let _ = output.stderr().read_to_end(&mut stderr).await;

                    eprintln!(
                        indoc! {"
                            ❌ Failed to set up GPG key:
                            stderr: {}
                        "},
                        String::from_utf8_lossy(&stderr)
                    );
                    return Err("Failed to set up GPG key");
                } else {
                    println!("  ✅ GPG key added to container's APT keyring");
                }
            }
            Err(e) => {
                eprintln!("  ❌ Failed to execute GPG key setup: {e}");
                return Err("Failed to execute GPG key setup");
            }
        }

        // Verify the key was added properly
        let key_verify = container
            .exec(ExecCommand::new(["ls", "-la", "/etc/apt/trusted.gpg.d/"]))
            .await;

        if let Ok(mut output) = key_verify {
            let mut stdout = Vec::new();
            let _ = output.stdout().read_to_end(&mut stdout).await;
            let files = String::from_utf8_lossy(&stdout);
            if files.contains("attune.asc") {
                println!("  ✅ GPG key file verified in keyring directory");
            } else {
                println!("  ❌ GPG key file not found in keyring directory");
                return Err("GPG key file verification failed");
            }
        }

        // Step 3: Configure APT sources list.
        println!("  Configuring APT repository...");

        // Detect container architecture for proper package selection.
        let arch_result = container
            .exec(ExecCommand::new(["dpkg", "--print-architecture"]))
            .await;

        let container_arch = if let Ok(mut output) = arch_result {
            let mut stdout = Vec::new();
            let _ = output.stdout().read_to_end(&mut stdout).await;
            String::from_utf8_lossy(&stdout).trim().to_string()
        } else {
            "amd64".to_string() // fallback.
        };

        println!("  Container architecture: {container_arch}");

        // Configure the repository URL (MinIO direct access).
        const DISTRIBUTION: &str = "stable";
        const COMPONENT: &str = "main";

        // Use the known S3 prefix for debian-test-repo-1 (path-style with tenant ID).
        const REPOSITORY_URL: &str = "http://localhost:9000/attune-dev-0/1/56c3adb4af8bf6505b258bc7543458bc45e1b7a5de78f259aaa0b6d448d83174";

        // Create the APT sources list entry.
        let sources_content = format!("deb [arch={container_arch}] {REPOSITORY_URL} {DISTRIBUTION} {COMPONENT}");
        let sources_cmd = format!(
            "cat > /etc/apt/sources.list.d/attune-test.list << 'EOF'\n{sources_content}\nEOF"
        );

        println!("  Writing sources file with content: {sources_content}");

        let create_sources_result = container
            .exec(ExecCommand::new(["bash", "-c", &sources_cmd]))
            .await;

        match create_sources_result {
            Ok(mut output) => {
                let exit_code = output.exit_code().await.unwrap_or(Some(-1));
                if exit_code != Some(0) && exit_code.is_some() {
                    let mut stderr = Vec::new();
                    let _ = output.stderr().read_to_end(&mut stderr).await;
                    eprintln!("  ❌ Failed to create sources file: {}", String::from_utf8_lossy(&stderr));
                    return Err("Failed to create sources file");
                } else {
                    println!("  ✅ Sources file created");
                }
            }
            Err(e) => {
                eprintln!("  ❌ Failed to create sources file: {e}");
                return Err("Failed to create sources file");
            }
        }

        // Verify the sources file was created correctly.
        let sources_verify = container
            .exec(ExecCommand::new(["cat", "/etc/apt/sources.list.d/attune-test.list"]))
            .await;

        match sources_verify {
            Ok(mut output) => {
                let mut stdout = Vec::new();
                let _ = output.stdout().read_to_end(&mut stdout).await;
                let content = String::from_utf8_lossy(&stdout);
                println!("  Sources file content: {}", content.trim());

                if content.contains("attune-dev-0") && content.contains(&container_arch) && content.contains(DISTRIBUTION) {
                    println!("  ✅ Sources file verified with correct content");
                } else {
                    println!("  ❌ Sources file content verification failed");
                    println!("    Expected: attune-dev-0, {container_arch}, {DISTRIBUTION}");
                    return Err("Sources file content verification failed");
                }
            }
            Err(e) => {
                println!("  ❌ Failed to read sources file: {e}");
                return Err("Failed to read sources file");
            }
        }

        // Update APT package lists to include the new repository.
        println!("  Updating APT package lists...");
        let apt_update_result = container
            .exec(ExecCommand::new(["bash", "-c", "apt-get update"]))
            .await;

        match apt_update_result {
            Ok(mut output) => {
                let exit_code = output.exit_code().await.unwrap_or(Some(-1));
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                let _ = output.stdout().read_to_end(&mut stdout).await;
                let _ = output.stderr().read_to_end(&mut stderr).await;

                let stdout_str = String::from_utf8_lossy(&stdout);
                let stderr_str = String::from_utf8_lossy(&stderr);

                if exit_code != Some(0) && exit_code.is_some() {
                    eprintln!(
                        indoc! {"
                            ❌ Failed to update APT package lists:
                            stdout: {}
                            stderr: {}
                        "},
                        stdout_str,
                        stderr_str
                    );
                    return Err("Failed to update APT package lists");
                } else {
                    println!("  ✅ APT package lists updated successfully");

                    // Show if our repository was accessed.
                    if stdout_str.contains("attune") || stdout_str.contains("56c3adb4af8bf6505b258bc7543458bc45e1b7a5de78f259aaa0b6d448d83174") {
                        println!("  ✅ Attune repository was accessed during update");
                    } else {
                        println!("  ⚠️  Attune repository not mentioned in update output");
                    }

                    // Show any warnings.
                    if !stderr_str.trim().is_empty() {
                        println!("  Update warnings:\n{}", stderr_str.trim());
                    }
                }
            }
            Err(e) => {
                eprintln!("  ❌ Failed to execute APT update: {e}");
                return Err("Failed to execute APT update");
            }
        }

        // Step 4: Install the attune-test-package and verify it works
        println!("  Installing attune-test-package...");

        // Choose version based on architecture: amd64 gets v1.0.3, arm64 gets v2.0.0
        let version = match container_arch.as_str() {
            "amd64" => "1.0.3",
            "arm64" => "2.0.0",
            _ => "1.0.3", // fallback to amd64 version
        };

        println!("  Installing attune-test-package version {version} for {container_arch} architecture");
        let install_cmd = format!("export DEBIAN_FRONTEND=noninteractive && apt-get install -y attune-test-package={version}");
        let install_result = container
            .exec(ExecCommand::new(["bash", "-c", &install_cmd]))
            .await;

        match install_result {
            Ok(mut output) => {
                let exit_code = output.exit_code().await.unwrap_or(Some(-1));
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                let _ = output.stdout().read_to_end(&mut stdout).await;
                let _ = output.stderr().read_to_end(&mut stderr).await;

                let stdout_str = String::from_utf8_lossy(&stdout);
                let stderr_str = String::from_utf8_lossy(&stderr);

                if exit_code != Some(0) && exit_code.is_some() {
                    eprintln!(
                        indoc! {"
                            ❌ Failed to install attune-test-package:
                            stdout: {}
                            stderr: {}
                        "},
                        stdout_str,
                        stderr_str
                    );
                    return Err("Failed to install attune-test-package");
                } else {
                    println!("  ✅ attune-test-package installed successfully");

                    // Show installation summary.
                    if stdout_str.contains("Setting up attune-test-package") {
                        println!("  ✅ Package setup completed");
                    }

                    // Show any warnings.
                    if !stderr_str.trim().is_empty() {
                        println!("  Install warnings:\n{}", stderr_str.trim());
                    }
                }
            }
            Err(e) => {
                eprintln!("  ❌ Failed to execute package installation: {e}");
                return Err("Failed to execute package installation");
            }
        }

        // Verify the installed package works.
        println!("  Verifying attune-test-package installation...");
        let verify_result = container
            .exec(ExecCommand::new(["attune-test-package", "--version"]))
            .await;

        match verify_result {
            Ok(mut output) => {
                let exit_code = output.exit_code().await.unwrap_or(Some(-1));
                if exit_code == Some(0) || exit_code.is_none() {
                    let mut stdout = Vec::new();
                    let _ = output.stdout().read_to_end(&mut stdout).await;
                    let version_output = String::from_utf8_lossy(&stdout).trim().to_string();

                    // Verify the correct version based on architecture
                    let expected_version = match container_arch.as_str() {
                        "amd64" => "1.0.3",
                        "arm64" => "2.0.0",
                        _ => "1.0.3", // fallback
                    };

                    if version_output.contains(expected_version) {
                        println!("  ✅ attune-test-package v{expected_version} verified for {container_arch} architecture: {version_output}");
                    } else {
                        println!("  ❌ Version mismatch - expected {expected_version} for {container_arch}, got: {version_output}");
                        return Err("Package version verification failed");
                    }
                } else {
                    println!("  ❌ attune-test-package --version command failed");
                    return Err("Package verification command failed");
                }
            }
            Err(e) => {
                println!("  ❌ Failed to verify package installation: {e}");
                return Err("Package verification failed");
            }
        }

        println!("  ✅ End-to-end APT package installation test completed successfully!");

        Ok(())
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
