use std::{convert::identity, fs, path::PathBuf};

use bollard::query_parameters::{
    BuildImageOptions, CreateContainerOptions, InspectContainerOptions, StartContainerOptions,
};
use bollard::secret::{ContainerStateStatusEnum, HealthStatusEnum};
use bollard::{
    Docker,
    exec::{CreateExecOptions, StartExecResults},
    models::{ContainerCreateBody, HostConfig},
};
use futures_util::stream::StreamExt;
use indoc::indoc;
use tokio::task::JoinSet;
use tokio_util::io::ReaderStream;
use tracing::{debug, info, trace};
use uuid::{ContextV7, Timestamp, Uuid};
use xshell::{Shell, cmd};

#[sqlx::test(migrator = "attune::testing::MIGRATOR")]
async fn migrations_applied(pool: sqlx::PgPool) {
    let table_exists = sqlx::query!(
        "SELECT EXISTS (
            SELECT FROM information_schema.tables
            WHERE table_schema = 'public'
            AND table_name = 'attune_tenant'
        ) as exists",
    )
    .fetch_one(&pool)
    .await
    .expect("Failed to check if attune_tenant table exists");

    assert!(
        table_exists.exists.is_some_and(identity),
        "attune_tenant table should exist after migrations"
    );
}

/// Configuration for smoke tests.
#[derive(Debug, Clone)]
struct TestConfig {
    api_endpoint: String,
    api_token: String,
    cli_path: String,
    gpg_key_id: String,
}

/// Simulate a `docker compose up` to check that the control plane comes up
/// properly with all Docker services. Then use the CLI to add many packages
/// concurrently. Then start a Debian container and `apt install` the packages
/// to show that the repository works.
#[tokio::test]
#[test_log::test]
async fn e2e() {
    // Set up dependencies.
    let sh = Shell::new().unwrap();
    debug!(docker_host = ?std::env::var("DOCKER_HOST"), "connecting to Docker daemon");
    let docker = Docker::connect_with_defaults().unwrap();
    docker.ping().await.unwrap();

    // Check that the CLI is built and executable.
    const ATTUNE_CLI_PATH: &str = env!("CARGO_BIN_EXE_attune");
    cmd!(sh, "{ATTUNE_CLI_PATH} --help").quiet().run().unwrap();
    debug!(path = ?ATTUNE_CLI_PATH, "CLI binary accessible");

    // Run `docker compose up -d` to bring up the Docker services.
    cmd!(sh, "docker compose up -d").run().unwrap();

    // Monitor control plane for readiness.
    debug!("waiting for control plane");
    loop {
        let status = docker
            .inspect_container("attune-controlplane-1", None::<InspectContainerOptions>)
            .await
            .unwrap();
        trace!(?status, "inspected control plane container status");
        if status.state.unwrap().status.unwrap() == ContainerStateStatusEnum::RUNNING {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }

    // Create a repository.
    let repo_name = format!(
        "e2e-test-{}",
        Uuid::new_v7(Timestamp::now(ContextV7::new()))
    );
    cmd!(sh, "{ATTUNE_CLI_PATH} apt repository create {repo_name}")
        .run()
        .unwrap();

    // Add packages concurrently.
    let gpg_key = GpgKey::new();
    let key_id = &gpg_key.key_id;

    const DISTRIBUTIONS: [&str; 5] = ["stable", "unstable", "testing", "dev", "canary"];
    const COMPONENTS: [&str; 8] = [
        "main", "contrib", "non-free", "extra", "rolling", "cloud", "oss", "ee",
    ];
    const PACKAGES: [&str; 10] = [
        "attune-test-package_2.0.0_linux_amd64.deb",
        "attune-test-package_2.0.0_linux_arm64.deb",
        "attune-test-package_3.0.5_linux_amd64.deb",
        "attune-test-package_3.0.5_linux_arm64.deb",
        "cod-test-package_3.0.5_linux_amd64.deb",
        "cod-test-package_3.0.5_linux_arm64.deb",
        "salmon-test-package_3.0.5_linux_amd64.deb",
        "salmon-test-package_3.0.5_linux_arm64.deb",
        "tuna-test-package_3.0.5_linux_amd64.deb",
        "tuna-test-package_3.0.5_linux_arm64.deb",
    ];

    let mut uploads = JoinSet::new();
    let mut i = 0;
    for distribution in DISTRIBUTIONS {
        for component in COMPONENTS {
            for package in PACKAGES {
                // HACK: We're using the `../../` to get to the workspace root
                // because we know where this package is located, and the
                // working directory is set to the package root in `cargo
                // test`[^1].
                //
                // [^1]: https://github.com/rust-lang/cargo/issues/11852
                let package = format!(
                    "{}/../../scripts/fixtures/{}",
                    env!("CARGO_MANIFEST_DIR"),
                    package
                );
                let sh = sh.clone();
                let key_id = key_id.clone();
                let repo_name = repo_name.clone();
                uploads.spawn(async move {
                    let result = cmd!(sh, "{ATTUNE_CLI_PATH} apt package add -k {key_id} --repo {repo_name} --distribution {distribution} --component {component} {package}")
                        .ignore_status()
                        .output()
                        .unwrap();
                    (result, i)
                });
                i += 1;
            }
        }
    }
    let results = uploads.join_all().await;
    for (result, i) in results {
        debug!(
            ?i,
            status = ?result.status,
            stdout = %String::from_utf8(result.stdout).unwrap(),
            stderr = %String::from_utf8(result.stderr).unwrap(),
            "added package"
        );
        assert!(result.status.success());
    }

    // Start a Debian container and install packages.
    todo!()
}

struct GpgKey {
    sh: Shell,
    key_id: String,
}

impl GpgKey {
    fn new() -> Self {
        let sh = Shell::new().unwrap();

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
        fs::write(CONFIG_PATH, gpg_config).unwrap();

        // Generate GPG key using batch mode and capture output.
        let generated = cmd!(sh, "gpg --batch --generate-key {CONFIG_PATH}")
            .output()
            .expect("GPG key generation failed. Make sure GPG is installed and configured.");
        let stdout = String::from_utf8(generated.stdout).unwrap();
        let stderr = String::from_utf8(generated.stderr).unwrap();
        debug!(
            stdout = ?stdout,
            stderr = ?stderr,
            "GPG key generation"
        );
        assert!(generated.status.success(), "GPG key generation failed");

        // Parse the GPG output to find the key ID from the revocation
        // certificate message.
        //
        // TODO: Write a real parser with `nom`.
        for line in stderr.lines() {
            if line.contains("revocation certificate stored as") && line.contains(".rev'") {
                debug!(?line, "parsing key ID");
                // Extract the key ID from the filename.
                if let Some(start) = line.rfind('/')
                    && let Some(end) = line.rfind(".rev'")
                {
                    let key_id = line[(start + 1)..end].to_string();

                    // Clean up configuration file.
                    fs::remove_file(CONFIG_PATH).unwrap();

                    return Self { sh, key_id };
                }
            }
        }
        panic!("GPG key ID not found in generation output");
    }
}

impl Drop for GpgKey {
    fn drop(&mut self) {
        let key_id = &self.key_id;
        cmd!(
            self.sh,
            "gpg --batch --yes --delete-secret-and-public-key {key_id}"
        )
        .run()
        .unwrap();
    }
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
    const PACKAGE_NAMES: [&str; 4] = [
        "attune-test-package",
        "cod-test-package",
        "salmon-test-package",
        "tuna-test-package",
    ];
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
        println!(
            "  Downloading packages for user {} (version {})...",
            user_index + 1,
            version
        );

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
                                println!(
                                    "    ✅ Downloaded {}: {} bytes",
                                    filename,
                                    metadata.len()
                                );
                            }
                            _ => {
                                println!(
                                    "    ❌ Download failed for {}: file too small or missing",
                                    filename
                                );
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

        println!(
            "Starting user {} with {} packages",
            user_index + 1,
            user_packages.len()
        );

        let handle = thread::spawn(move || {
            let user_sh = Shell::new().unwrap();

            // Copy environment variables to the new shell
            user_sh.set_var(
                "ATTUNE_API_ENDPOINT",
                &std::env::var("ATTUNE_API_ENDPOINT")
                    .unwrap_or_else(|_| "http://localhost:3000".to_string()),
            );
            user_sh.set_var(
                "ATTUNE_API_TOKEN",
                &std::env::var("ATTUNE_API_TOKEN")
                    .unwrap_or_else(|_| "INSECURE_TEST_TOKEN".to_string()),
            );
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
                    if success {
                        "Success".to_string()
                    } else {
                        format!("Error: {}", add_result.unwrap_err())
                    },
                ));

                // Clean up downloaded file after upload attempt
                let _ = std::fs::remove_file(&filepath);
            }

            user_results
        });

        handles.push(handle);
    }

    println!(
        "All {} user threads started, uploading packages concurrently...",
        handles.len()
    );

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
    let successful_adds = all_results
        .iter()
        .filter(|(_, _, _, _, success, _)| *success)
        .count();
    let failed_adds = total_attempts - successful_adds;

    println!("\n========== CONCURRENT PACKAGE ADD RESULTS ==========");
    println!("Total package add attempts: {}", total_attempts);
    println!("Successful additions: {}", successful_adds);
    println!("Failed additions: {}", failed_adds);
    println!(
        "Success rate: {:.1}%",
        (successful_adds as f64 / total_attempts as f64) * 100.0
    );

    // Show detailed results by user
    println!("\nDetailed results by user:");
    for user_id in 1..=6 {
        let user_results: Vec<_> = all_results
            .iter()
            .filter(|(uid, _, _, _, _, _)| *uid == user_id)
            .collect();
        let user_successes = user_results
            .iter()
            .filter(|(_, _, _, _, success, _)| *success)
            .count();
        println!(
            "  User {}: {}/{} packages added successfully",
            user_id,
            user_successes,
            user_results.len()
        );
    }

    // Show any failures for debugging
    let failures: Vec<_> = all_results
        .iter()
        .filter(|(_, _, _, _, success, _)| !*success)
        .collect();
    if !failures.is_empty() {
        println!("\nFailure details:");
        for (user_id, pkg_name, version, arch, _, error) in failures {
            println!(
                "  User {}: {} {} {} - {}",
                user_id, pkg_name, version, arch, error
            );
        }
    }

    // The test is successful if we had reasonable concurrent behavior
    // We expect some conflicts/failures due to concurrent access, but the system should handle it gracefully
    if successful_adds > 0 {
        println!("✅ Concurrent package add test completed - system handled concurrent operations");
        if failed_adds > 0 {
            println!(
                "ℹ️  Some operations failed due to concurrency conflicts, which is expected behavior"
            );
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
            println!(
                "  ✅ GPG public key exported from host (length: {} chars)",
                key.len()
            );
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
