use xshell::{cmd, Shell};
use std::env;

/// Helper function to get the CLI binary path.
fn get_cli_binary() -> String {
    env::var("ATTUNE_CLI_PATH").expect(
        "ATTUNE_CLI_PATH environment variable is required. Set it to the path of your CLI binary.\n\
         Example: export ATTUNE_CLI_PATH=/Users/xinding/go/bin/attune"
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
fn test_repo_create() {
    println!("\n========== SMOKE TEST: Repository Create =========");
    
    let sh = Shell::new().unwrap();
    setup_test_env(&sh);
    
    let cli = get_cli_binary();
    
    // Use fixed Debian repo name for testing.
    let debian_repo_url = "http://localhost:9000/debian";
    
    println!("Testing with Debian repo URL: {}", debian_repo_url);
    
    // Step 1: Create a new Debian repo.
    println!("\nStep 1: Creating Debian repo...");
    let create_result = cmd!(sh, "{cli} repo create -u {debian_repo_url}")
        .run();
    
    match create_result {
        Ok(_) => println!("✅ Repo creation command executed successfully"),
        Err(e) => {
            eprintln!("❌ Repo creation failed: {}", e);
            panic!("Repo creation failed. Check your environment variables and CLI binary path.");
        }
    }
    
    // Step 2: List repos and verify our repo exists.
    println!("\nStep 2: Listing repos to verify creation...");
    let list_output = cmd!(sh, "{cli} repo list")
        .read();
    
    match list_output {
        Ok(output) => {
            println!("Repo list output:");
            println!("{}", output);
            
            if output.contains(debian_repo_url) {
                println!("✅ Repo '{}' found in list", debian_repo_url);
                println!("🎉 Test completed successfully!\n");
            } else {
                eprintln!("❌ Repo '{}' not found in list", debian_repo_url);
                eprintln!("Available repos:\n{}", output);
                panic!("Created repo not found in list");
            }
        }
        Err(e) => {
            eprintln!("❌ Failed to list repos: {}", e);
            panic!("Repo listing failed. Check your environment variables and CLI binary path.");
        }
    }
}

#[test]
fn test_cli_binary_exists() {
    println!("\n========== SMOKE TEST: CLI Binary Accessibility =========");
    
    let cli = get_cli_binary();
    let sh = Shell::new().unwrap();
    
    println!("Testing CLI binary at: {}", cli);
    
    // Test that the CLI binary exists and can be executed.
    println!("\nRunning CLI help command...");
    let version_result = cmd!(sh, "{cli} --help")
        .run();
    
    match version_result {
        Ok(_) => {
            println!("✅ CLI binary is accessible and executable");
            println!("🎉 Test completed successfully!\n");
        }
        Err(e) => {
            eprintln!("❌ CLI binary test failed: {}", e);
            eprintln!("Make sure the CLI is built and the path is correct.");
            eprintln!("Current CLI path: {}", cli);
            eprintln!("You can set ATTUNE_CLI_PATH environment variable to specify the correct path.");
            panic!("CLI binary not accessible");
        }
    }
}
