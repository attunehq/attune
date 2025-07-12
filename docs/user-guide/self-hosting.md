# Self-hosting Attune

You can self-host your own Attune control plane and backing storage.

## Setting up the Attune control plane

1. Create a `.env` file containing the environment variables you've configured. Use [`.env.example`](../../.env.example) as a template and reference for the available variables. **You should set `ATTUNE_API_TOKEN` to your own secret API token.**
2. Run `docker compose up` in the root directory of the Attune repository. This will start local instances of Postgres, Minio, and Attune as Docker containers.

### Using your own database and data plane

If you'd like to use your own providers for Postgres and Minio, you can configure the control plane to use them by setting the `ATTUNE_DATABASE_URL` and `AWS_ENDPOINT_URL_S3` environment variables. For details, see [`.env.example`](../../.env.example).

## Setting up the CLI

You can download the CLI from [GitHub Releases](https://github.com/attunehq/attune/releases).

To use the CLI, you need to configure two environment variables:

- `ATTUNE_API_TOKEN`: This should be set to the same value as the `ATTUNE_API_TOKEN` of the control plane.
- `ATTUNE_API_ENDPOINT`: This should be set to the base URL of the control plane (e.g. `http://localhost:3000`).

Like for the control plane, you can see details about the available environment variables in [`.env.example`](../../.env.example).
