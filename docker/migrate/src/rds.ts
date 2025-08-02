import "process";
import { Signer } from "@aws-sdk/rds-signer";

/**
 * Given the provided connection string, generate a password using AWS IAM Authentication add it to the connection string.
 *
 * Password is only good for 15 minutes. If migrations take longer than this, they will fail.
 *
 * @param connectionString The connection string used to connect to the database, without a password
 * @returns An identical connection string, but with the password included
 */
export async function buildIAMAuthenticationConnectionString(connectionString: string): Promise<string> {
    const connectionURL = new URL(connectionString);
    if (!connectionURL.hostname) {
        connectionURL.hostname = process.env["PGHOST"] || "localhost";
    }
    if (!connectionURL.port) {
        connectionURL.port = process.env["PGPORT"] || "5432";
    }
    if (!connectionURL.username) {
        connectionURL.username = process.env["PGUSER"] || "postgres";
    }

    const signer = new Signer({
        hostname: connectionURL.hostname,
        port: +connectionURL.port,
        username: connectionURL.username,
        profile: process.env["ATTUNE_RDS_IAM_PROFILE"] || undefined,
    });

    connectionURL.password = encodeURIComponent(await signer.getAuthToken());
    return connectionURL.toString();
}
