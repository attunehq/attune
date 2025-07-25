import { Signer } from "@aws-sdk/rds-signer";
import { parse as parseConnectionString } from 'pg-connection-string';
import { env, execve } from "process";

async function main() {
    const providedConnectionString = env["ATTUNE_DATABASE_URL"];
    if (!providedConnectionString) throw new Error("ATTUNE_DATABASE_URL was not provided");

    const connectionInfo = parseConnectionString(providedConnectionString);

    const signer = new Signer({
        hostname: connectionInfo.host ?? env["PGHOST"] ?? "localhost",
        port: connectionInfo.port ? +connectionInfo.port : env["PGPORT"] ? +env["PGPORT"] : 5432,
        username: connectionInfo.user ?? env["PGUSER"] ?? "postgres",
        profile: env["ATTUNE_RDS_IAM_PROFILE"] ?? null
    })

    // Append to the connection string to avoid needing to rebuild it
    const token = await signer.getAuthToken()
    let passwordConnectionString = providedConnectionString;
    if (connectionInfo.options)
        passwordConnectionString += "&";
    else
        passwordConnectionString += "?";
    passwordConnectionString += "password=" + encodeURIComponent(token);

    // Run the migrations
    env["ATTUNE_DATABASE_URL"] = passwordConnectionString
    execve("npm", ["run", "migrate"], env)
}

main();
