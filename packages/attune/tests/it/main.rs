use std::{convert::identity, fs};

use bollard::Docker;
use bollard::query_parameters::InspectContainerOptions;
use bollard::secret::{ContainerStateStatusEnum, HealthStatusEnum};
use dotenv::dotenv;
use indoc::indoc;
use testcontainers::core::{CmdWaitFor, ExecCommand};
use testcontainers::runners::{AsyncBuilder, AsyncRunner};
use testcontainers::{ContainerAsync, GenericBuildableImage, GenericImage, ImageExt};
use tokio::task::JoinSet;
use tracing::{debug, trace};
use tracing_subscriber::{
    fmt::format::FmtSpan, layer::SubscriberExt as _, util::SubscriberInitExt as _,
};
use uuid::{ContextV7, Timestamp, Uuid};
use xshell::{Shell, cmd};

use attune_macros::workspace_root;

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

macro_rules! sh_exec {
    ($sh: expr, $cmd: literal) => {{
        let result = cmd!($sh, $cmd).ignore_status().output().unwrap();
        let stdout = String::from_utf8(result.stdout).unwrap();
        let stderr = String::from_utf8(result.stderr).unwrap();
        debug!(%stdout, %stderr, $cmd);
        (stdout, stderr, result.status)
    }};
}

async fn container_exec(
    container: &ContainerAsync<GenericImage>,
    cmd: &Vec<&str>,
) -> (Vec<u8>, Vec<u8>, i64) {
    let mut result = container
        .exec(ExecCommand::new(cmd.clone()).with_cmd_ready_condition(CmdWaitFor::exit()))
        .await
        .unwrap();
    let stdout = result.stdout_to_vec().await.unwrap();
    let stderr = result.stderr_to_vec().await.unwrap();
    let exit_code = result.exit_code().await.unwrap().unwrap();

    debug!(
        stdout = %String::from_utf8_lossy(&stdout),
        stderr = %String::from_utf8_lossy(&stderr),
        ?exit_code,
        "{}",
        cmd.join(" ")
    );
    (stdout, stderr, exit_code)
}

/// Simulate a `docker compose up` to check that the control plane comes up
/// properly with all Docker services. Then use the CLI to add many packages
/// concurrently. Then start a Debian container and `apt install` the packages
/// to show that the repository works.
#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn e2e() {
    // We use this instead of `test_log` for this specific test because the
    // extra diagnostic information (e.g. event timestamps) is very helpful for
    // debugging a test of this complexity.
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
                .with_file(true)
                .with_line_number(true)
                .with_target(true)
                .with_thread_ids(true)
                .with_thread_names(true)
                .with_writer(std::io::stderr)
                .pretty(),
        )
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // Set up dependencies.
    dotenv().ok();
    let sh = Shell::new().unwrap();
    debug!(docker_host = ?std::env::var("DOCKER_HOST"), "connecting to Docker daemon");
    let docker = Docker::connect_with_defaults().unwrap();
    docker.ping().await.unwrap();
    const WORKSPACE_ROOT: &str = workspace_root!();

    // Check that the CLI is built and executable.
    const ATTUNE_CLI_PATH: &str = env!("CARGO_BIN_EXE_attune");
    let (_, _, exit_code) = sh_exec!(&sh, "{ATTUNE_CLI_PATH} --help");
    assert!(exit_code.success());
    debug!(path = ?ATTUNE_CLI_PATH, "CLI binary accessible");

    // Run `docker compose up -d` to bring up the Docker services.
    //
    // TODO: This is extremely slow in CI, because CI is really slow to build
    // Docker images. But we've already built these images locally to test, so
    // maybe we should push these up to a shared cache registry? Can we do that
    // transparently?
    let (_, _, exit_code) = sh_exec!(&sh, "docker compose up --build --force-recreate --detach");
    assert!(exit_code.success());

    // Monitor control plane for readiness.
    debug!("waiting for control plane");
    loop {
        let inspected = docker
            .inspect_container("attune-controlplane-1", None::<InspectContainerOptions>)
            .await
            .unwrap();
        trace!(?inspected, "inspected control plane container status");
        let container_state = inspected.state.unwrap();
        if container_state.status.unwrap() == ContainerStateStatusEnum::RUNNING
            && container_state.health.unwrap().status.unwrap() == HealthStatusEnum::HEALTHY
        {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }

    // Create a repository.
    let repo = AttuneRepository::new(sh.clone(), ATTUNE_CLI_PATH.to_string());

    // Add packages concurrently.
    let gpg_key = GpgKey::new();
    let key_id = &gpg_key.key_id;
    let (pubkey, _, exit_code) = sh_exec!(&sh, "gpg --armor --export {key_id}");
    assert!(exit_code.success());

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
                let package = format!("{}/scripts/fixtures/{}", WORKSPACE_ROOT, package);
                let sh = sh.clone();
                let key_id = key_id.clone();
                let repo_name = repo.name.clone();
                uploads.spawn(async move {
                    trace!(?i, "started upload");
                    let result = cmd!(sh, "{ATTUNE_CLI_PATH} apt package add -k {key_id} --repo {repo_name} --distribution {distribution} --component {component} {package}")
                        .env("RUST_LOG", "attune=debug")
                        .ignore_status()
                        .output()
                        .unwrap();
                    (i, result)
                });
                i += 1;
                trace!(?i, "scheduled upload");
            }
        }
    }
    debug!("waiting for uploads to complete");

    let results = uploads.join_all().await;
    debug!("all uploads completed");

    // First, emit server logs for debugging. Note that the stdout is already
    // logged by `sh_exec!`.
    let (_, _, exit_code) = sh_exec!(&sh, "docker compose logs --timestamps controlplane");
    assert!(exit_code.success());

    // Then, check the upload results.
    for (i, result) in results.clone() {
        // We log every result out first and _then_ check for success, so that
        // we can see all the logs and piece together what happened on failure.
        debug!(
            ?i,
            status = ?result.status,
            stdout = %String::from_utf8(result.stdout).unwrap(),
            stderr = %String::from_utf8(result.stderr).unwrap(),
            "apt pkg add logs");
    }
    for (i, result) in results {
        debug!(?i, success = result.status.success(), "apt pkg add result");
        assert!(result.status.success());
    }

    // Start a Debian container and install packages.
    let image = GenericBuildableImage::new("attune-testinstall", "latest")
        .with_dockerfile(format!("{}/docker/testinstall/Dockerfile", WORKSPACE_ROOT))
        .build_image()
        .await
        .unwrap();
    debug!(?image, "built testinstall image");

    let repo_url = format!(
        "http://localhost:9000/{}/{}",
        repo.s3_bucket, repo.s3_prefix
    );
    let container = image
        .with_cmd(vec!["sleep", "infinity"])
        .with_copy_to("/etc/apt/keyrings/attune.asc", pubkey.into_bytes())
        .with_copy_to(
            "/etc/apt/sources.list",
            format!("deb [signed-by=/etc/apt/keyrings/attune.asc] {repo_url} stable main\n",)
                .into_bytes(),
        )
        .with_network("host")
        .start()
        .await
        .unwrap();
    debug!(?container, "created testinstall container");

    debug!("waiting for container to start");
    loop {
        let status = container.is_running().await;
        debug!(?status, "container status");
        if status.unwrap() {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }

    debug!("running apt-get update");
    let (_, _, exit_code) = container_exec(&container, &vec!["apt-get", "update"]).await;
    assert_eq!(exit_code, 0);

    debug!("installing attune-test-package");
    let (_, _, exit_code) = container_exec(
        &container,
        &vec!["apt-get", "install", "-y", "attune-test-package"],
    )
    .await;
    assert_eq!(exit_code, 0);

    debug!("verifying installed package");
    let (_, _, exit_code) =
        container_exec(&container, &vec!["attune-test-package", "--version"]).await;
    assert_eq!(exit_code, 0);
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
        let (_, stderr, exit_code) = sh_exec!(&sh, "gpg --batch --generate-key {CONFIG_PATH}");
        assert!(exit_code.success(), "GPG key generation failed");

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
        sh_exec!(
            &self.sh,
            "gpg --batch --yes --delete-secret-and-public-key {key_id}"
        );
    }
}

struct AttuneRepository {
    sh: Shell,
    cli_path: String,

    name: String,
    s3_bucket: String,
    s3_prefix: String,
}

impl AttuneRepository {
    fn new(sh: Shell, cli_path: String) -> Self {
        let name = format!(
            "e2e-test-{}",
            Uuid::new_v7(Timestamp::now(ContextV7::new()))
        );
        let (stdout, _, _) = sh_exec!(&sh, "{cli_path} apt repository create --json {name}");
        let res = serde_json::from_slice::<attune::server::repo::create::CreateRepositoryResponse>(
            stdout.as_bytes(),
        )
        .unwrap();
        Self {
            sh,
            cli_path,
            name,
            s3_bucket: res.s3_bucket,
            s3_prefix: res.s3_prefix,
        }
    }
}

impl Drop for AttuneRepository {
    fn drop(&mut self) {
        let cli_path = &self.cli_path;
        let repo_name = &self.name;
        sh_exec!(&self.sh, "{cli_path} apt repository delete -y {repo_name}");
    }
}
