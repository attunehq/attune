import "process";
import { Signer } from "@aws-sdk/rds-signer";
import { parse as parseConnectionString } from 'pg-connection-string';

/**
 * Given the provided connection string, generate a password using AWS IAM Authentication add it to the connection string.
 * 
 * Password is only good for 15 minutes. If migrations take longer than this, they will fail.
 * 
 * @param connectionString The connection string used to connect to the database, without a password
 * @returns An identical connection string, but with the password included
 */
export async function buildIAMAuthenticationConnectionString(connectionString: string): Promise<string> {
    const connectionInfo = parseConnectionString(connectionString);

    const signer = new Signer({
        hostname: connectionInfo.host ?? process.env["PGHOST"] ?? "localhost",
        port: connectionInfo.port ? +connectionInfo.port : process.env["PGPORT"] ? + process.env["PGPORT"] : 5432,
        username: connectionInfo.user ?? process.env["PGUSER"] ?? "postgres",
        profile: process.env["ATTUNE_RDS_IAM_PROFILE"] ?? null
    })

    // Append to the connection string to avoid needing to rebuild it
    const token = await signer.getAuthToken();
    return `${connectionString}${connectionInfo.options ? "?" : "&"}password=${encodeURIComponent(token)}`;
}
