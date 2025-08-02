import "process";
import minimist from "minimist";
import { buildIAMAuthenticationConnectionString } from "./rds.js"

const ENV_DANGEROUSLY_LOG_CONNECTION_STRING = "ATTUNE_MIGRATION_LOG_CONNECTION_STRING_INSECURE_DO_NOT_SET_THIS";
const ENV_ATTUNE_DATABASE_URL = "ATTUNE_DATABASE_URL";

const ARG_AUTH_METHOD = "auth-method";
const ARG_AUTH_METHOD_RDSIAM = "rds-iam";

const LOG_CONNECTION_STRING = process.env[ENV_DANGEROUSLY_LOG_CONNECTION_STRING] === "true";

/**
 * Runs Attune database migrations. The `ATTUNE_DATABASE_URL` variable provides the connection string.
 *
 * The `auth-method` argument can be used to specify the authentication method. The default if not set
 * is to use authentication provided via the connection string (e.g. password, password file, mTLS, etc.).
 *
 * `auth-method` values:
 * - `rds-iam` Use [AWS RDS IAM authentication](https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/UsingWithRDS.IAMDBAuth.html)
 *
 * ## Debugging
 *
 * Set the `ATTUNE_MIGRATION_LOG_CONNECTION_STRING_INSECURE_DO_NOT_SET_THIS` variable to `true`
 * to log the connection string. This may include the plaintext password, and should only be used with
 * temporary credentials for debugging purposes.
 */
async function main() {
    const providedConnectionString = process.env[ENV_ATTUNE_DATABASE_URL];
    if (!providedConnectionString) {
        throw new Error("ATTUNE_DATABASE_URL was not provided");
    }

    const argv = minimist(process.argv.slice(2));
    const authMethod = argv[ARG_AUTH_METHOD];

    if (LOG_CONNECTION_STRING) {
        console.log("Using auth-method: ", authMethod);
        console.log("Provided ATTUNE_DATABASE_URL: ", providedConnectionString);
    }

    // We set the `ATTUNE_DATABASE_URL` environment variable to the resolved connection string
    // so that when we run `npm run migrate` below it will use the resolved connection string.
    switch (authMethod) {
        case ARG_AUTH_METHOD_RDSIAM:
            console.log("Using AWS RDS IAM authentication");
            process.env[ENV_ATTUNE_DATABASE_URL] = await buildIAMAuthenticationConnectionString(providedConnectionString);
            if (LOG_CONNECTION_STRING) {
                console.log("Resolved ATTUNE_DATABASE_URL: ", process.env[ENV_ATTUNE_DATABASE_URL]);
            }
            break;
        default:
            console.log("Using connection string authentication");
    }

    // `execve` requires a filepath to execute. Use `env`, masquerading as `npm`
    // to find and execute `npm` with the provided `process.env` variables.
    // This will not return unless there is an error loading/jumping to the
    // `env` code. The current process is replaced with `env`, which in turn
    // replaces itself with `npm`.
    process.execve!("/usr/bin/env", ["npm", "npm", "run", "migrate"], process.env);
}

await main();
