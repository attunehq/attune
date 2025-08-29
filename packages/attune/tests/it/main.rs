use std::{
    collections::HashMap,
    fs,
    io::{BufRead as _, BufReader},
    process::{Command, ExitStatus, Stdio},
    thread::spawn,
};

use bollard::{
    Docker,
    query_parameters::InspectContainerOptions,
    secret::{ContainerStateStatusEnum, HealthStatusEnum},
};
use dotenv::dotenv;
use indoc::indoc;
use testcontainers::{
    core::{CmdWaitFor, ExecCommand},
    runners::{AsyncBuilder, AsyncRunner},
    {ContainerAsync, GenericBuildableImage, GenericImage, ImageExt},
};
use tokio::task::JoinSet;
use tracing::{debug, trace};
use tracing_subscriber::{
    fmt::format::FmtSpan, layer::SubscriberExt as _, util::SubscriberInitExt as _,
};
use uuid::{ContextV7, Timestamp, Uuid};

use attune_macros::workspace_root;

#[derive(Debug)]
struct Exec {
    prog: String,
    argv: Vec<String>,
    env: HashMap<String, String>,
    quiet: bool,
}

fn exec(cmd: impl AsRef<str>) -> (String, String, ExitStatus) {
    let mut args = cmd.as_ref().split(' ');
    let prog = args.next().unwrap().to_string();
    let argv = args.map(String::from).collect();
    exec_options(Exec {
        prog,
        argv,
        env: HashMap::new(),
        quiet: false,
    })
}

fn exec_options(exec: Exec) -> (String, String, ExitStatus) {
    if exec.quiet {
        trace!(?exec, "running command");
    } else {
        debug!(?exec, "running command");
    }
    let mut cmd = Command::new(exec.prog);
    cmd.args(exec.argv);
    cmd.envs(exec.env.into_iter().chain(std::env::vars()));

    let (stdout, stderr, status) = if exec.quiet {
        let output = cmd.output().unwrap();
        (
            String::from_utf8_lossy(&output.stdout).to_string(),
            String::from_utf8_lossy(&output.stderr).to_string(),
            output.status,
        )
    } else {
        let mut child = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        let child_stdout = child.stdout.take().unwrap();
        let stdout_reader = spawn(|| {
            let mut lines = Vec::new();
            let lines_reader = BufReader::new(child_stdout).lines();
            for line in lines_reader {
                let line = line.unwrap();
                println!("{line}");
                lines.push(line);
            }
            lines.join("\n")
        });
        let child_stderr = child.stderr.take().unwrap();
        let stderr_reader = spawn(|| {
            let mut lines = Vec::new();
            let lines_reader = BufReader::new(child_stderr).lines();
            for line in lines_reader {
                let line = line.unwrap();
                eprintln!("{line}");
                lines.push(line);
            }
            lines.join("\n")
        });

        let status = child.wait().unwrap();
        let stdout = stdout_reader.join().unwrap();
        let stderr = stderr_reader.join().unwrap();

        (stdout, stderr, status)
    };

    if exec.quiet {
        trace!(?status, "command completed");
    } else {
        debug!(?status, "command completed");
        trace!(%stdout, %stderr, "command output");
    }
    (stdout, stderr, status)
}

async fn container_exec(
    container: &ContainerAsync<GenericImage>,
    cmd: &Vec<&str>,
) -> (String, String, i64) {
    let mut result = container
        .exec(ExecCommand::new(cmd.clone()).with_cmd_ready_condition(CmdWaitFor::exit()))
        .await
        .unwrap();
    let stdout = String::from_utf8_lossy(&result.stdout_to_vec().await.unwrap()).to_string();
    let stderr = String::from_utf8_lossy(&result.stderr_to_vec().await.unwrap()).to_string();
    let exit_code = result.exit_code().await.unwrap().unwrap();

    debug!(%stdout, %stderr, ?exit_code, "{}", cmd.join(" "));
    (stdout, stderr, exit_code)
}

/// Do shared test setup, like initializing tracing and setting environment
/// variables.
async fn setup() {
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
    dotenv().ok();
}

/// Spin up control plane and upstream dependency Docker services.
async fn docker_compose_up_services() -> Docker {
    // Connect to Docker daemon.
    debug!(docker_host = ?std::env::var("DOCKER_HOST"), "connecting to Docker daemon");
    let docker = Docker::connect_with_defaults().unwrap();
    docker.ping().await.unwrap();

    // Run `docker compose up -d` to bring up the Docker services.
    //
    // TODO: This is extremely slow in CI, because CI is really slow to build
    // Docker images. But we've already built these images locally to test, so
    // maybe we should push these up to a shared cache registry? Can we do that
    // transparently?
    let (_, _, exit_code) = exec("docker compose up --build --force-recreate --detach");
    assert!(exit_code.success());

    // Monitor control plane for readiness.
    //
    // TODO: We use `bollard` to query the control plane service state, `xshell`
    // to actually shell out to `docker compose`, and `testcontainers` to
    // actually run a dynamic container for testing the Debian repository.
    // Ideally we'd standardize on one of these, since this seems pretty
    // confusing.
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
            return docker;
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
}

/// The root of the workspace. Useful for referencing package fixture files.
const WORKSPACE_ROOT: &str = workspace_root!();

/// The path to the `attune` CLI binary. Cargo always populates this environment
/// variable to correspond to the compilation mode in which the test is invoked
/// (i.e. either debug or release).
const ATTUNE_CLI_PATH: &str = env!("CARGO_BIN_EXE_attune");

/// Run end-to-end tests against the standard development Docker setup. This
/// also checks that the Docker images build, start, and communicate properly.
#[test_with::env(E2E_DOCKER)]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn e2e_docker() {
    docker_compose_up_services().await;
    e2e_run().await;
}

/// Run end-to-end tests against the control plane running on the host.
#[test_with::env(E2E_HOST)]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn e2e_host() {
    setup().await;
    e2e_run().await;
}

/// Use the CLI to add many packages concurrently. Then start a Debian container
/// and `apt install` the packages to show that the repository works.
async fn e2e_run() {
    // Check that the CLI is built and executable.
    let (_, _, exit_code) = exec(format!("{ATTUNE_CLI_PATH} --help"));
    assert!(exit_code.success());
    debug!(path = ?ATTUNE_CLI_PATH, "CLI binary accessible");

    // Create a repository.
    let repo = AttuneRepository::new(ATTUNE_CLI_PATH.to_string());

    // Add packages concurrently.
    let gpg_key = GpgKey::new();
    let key_id = &gpg_key.key_id;
    let (pubkey, _, exit_code) = exec(format!("gpg --armor --export {key_id}"));
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

    debug!("starting CLI uploads");
    let mut uploads = JoinSet::new();
    let mut i = 0;
    for distribution in DISTRIBUTIONS {
        for component in COMPONENTS {
            for package in PACKAGES {
                let package = format!("{}/scripts/fixtures/{}", WORKSPACE_ROOT, package);
                let key_id = key_id.clone();
                let repo_name = repo.name.clone();
                uploads.spawn(async move {
                    trace!(?i, "started upload");
                    let result = exec_options(Exec {
                        prog: ATTUNE_CLI_PATH.to_string(),
                        argv: vec![
                            "apt",
                            "package",
                            "add",
                            "--key-id",
                            &key_id,
                            "--repo",
                            &repo_name,
                            "--distribution",
                            distribution,
                            "--component",
                            component,
                            &package,
                        ]
                        .into_iter()
                        .map(String::from)
                        .collect(),
                        env: HashMap::from([(
                            String::from("RUST_LOG"),
                            String::from("attune=debug"),
                        )]),
                        quiet: true,
                    });
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

    // First, emit server logs for debugging.
    let (_, _, exit_code) = exec("docker compose logs --timestamps controlplane");
    assert!(exit_code.success());

    // Then, check the upload results.
    for (i, (stdout, stderr, status)) in results.clone() {
        // We log every result out first and _then_ check for success, so that
        // we can see all the logs and piece together what happened on failure.
        trace!(?i, ?status, %stdout, %stderr, "apt pkg add logs");
    }
    for (i, (_, _, status)) in results {
        trace!(?i, success = status.success(), "apt pkg add result");
        assert!(status.success());
    }
    debug!("all uploads successful");

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
    key_id: String,
}

impl GpgKey {
    fn new() -> Self {
        debug!("creating GPG key");

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
        let (_, stderr, exit_code) = exec(format!("gpg --batch --generate-key {CONFIG_PATH}"));
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

                    debug!(?key_id, "created GPG key");
                    return Self { key_id };
                }
            }
        }
        panic!("GPG key ID not found in generation output");
    }
}

impl Drop for GpgKey {
    fn drop(&mut self) {
        debug!("deleting GPG key");
        let key_id = self.key_id.clone();
        exec(format!(
            "gpg --batch --yes --delete-secret-and-public-key {key_id}"
        ));
        debug!("deleted GPG key");
    }
}

struct AttuneRepository {
    cli_path: String,

    name: String,
    s3_bucket: String,
    s3_prefix: String,
}

impl AttuneRepository {
    fn new(cli_path: String) -> Self {
        debug!("creating repository");
        let name = format!(
            "e2e-test-{}",
            Uuid::new_v7(Timestamp::now(ContextV7::new()))
        );
        let (stdout, _, _) = exec(format!("{cli_path} apt repository create --json {name}"));
        let res = serde_json::from_slice::<attune::server::repo::create::CreateRepositoryResponse>(
            stdout.as_bytes(),
        )
        .unwrap();
        debug!(?res, "created repository");
        Self {
            cli_path,
            name,
            s3_bucket: res.s3_bucket,
            s3_prefix: res.s3_prefix,
        }
    }
}

impl Drop for AttuneRepository {
    fn drop(&mut self) {
        debug!("deleting repository");
        let cli_path = self.cli_path.clone();
        let repo_name = self.name.clone();
        exec(format!("{cli_path} apt repository delete -y {repo_name}"));
        debug!("deleted repository");
    }
}
