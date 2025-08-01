import "process";
import { buildIAMAuthenticationConnectionString } from "./rds"

/**
 * Runs Attune database migrations. The environment variable `ATTUNE_MIGRATION_AUTHENTICATION_METHOD`
 * can be specified to control how authentication is handled. The `ATTUNE_DATABASE_URL` variable
 * provides the connection string.
 *
 * This should generally be executed via `npm run migrate:autoauth`.
 *
 * The `ATTUNE_MIGRATION_LOG_CONNECTION_STRING_INSECURE_DO_NOT_SET_THIS` variable can be set to `true`
 * to log the connection string. This may include the plaintext password, and should only be used with
 * temporary credentials for debugging purposes.
 *
 * `ATTUNE_MIGRATION_AUTHENTICATION_METHOD` values:
 * * `rds-iam` Use [AWS RDS IAM authentication](https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/UsingWithRDS.IAMDBAuth.html)
 * * Any other value uses authentication provided via the connection string (e.g. password, password file, mTLS, etc.)
 */
async function main() {
    const providedConnectionString = process.env["ATTUNE_DATABASE_URL"];
    if (!providedConnectionString) throw new Error("ATTUNE_DATABASE_URL was not provided");

    const authMethod = process.env["ATTUNE_MIGRATION_AUTHENTICATION_METHOD"];
    switch (authMethod) {
        case "rds-iam":
            console.log("Using AWS RDS IAM authentication");
            process.env["ATTUNE_DATABASE_URL"] = await buildIAMAuthenticationConnectionString(providedConnectionString);
            break
        default:
            console.log("Using connection string authentication");
    }

    const logConnectionString = process.env["ATTUNE_MIGRATION_LOG_CONNECTION_STRING_INSECURE_DO_NOT_SET_THIS"];
    if (logConnectionString === "true")
        console.log(process.env["ATTUNE_DATABASE_URL"]);

    // `execve` requires a filepath to execute. Use `env`, masquerading as `npm`
    // to find and execute `npm` with the provided `process.env` variables.
    // This will not return unless there is an error loading/jumping to the
    // `env` code. The current process is replaced with `env`, which in turn
    // replaces itself with `npm`.
    process.execve!("/usr/bin/env", ["npm", "npm", "run", "migrate"], process.env);
}

await main();
