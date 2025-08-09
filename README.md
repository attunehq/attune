# Attune
[![GitHub license](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](https://github.com/attunehq/attune/blob/main/LICENSE)

Attune is a tool for securely publishing and hosting Linux packages.

[Join our Community Slack](https://join.slack.com/t/attunecommunityslack/shared_invite/zt-33fmjoepy-w5HUiI6P44G15QJlvYhQiw) to connect with other users and get help.

- **Flexible deployment:** Deploy a self-hosted package repository, or use our managed cloud infrastructure.
- **Secure by design:** Attune's CLI does repository index signing locally, so you can own your signing keys without ever exposing them.
- **Really, really fast:** Attune does incremental repository index rebuilds, so adding and removing packages is really fast.

Attune is available in two editions:
- **Community Edition:** Free and open source, perfect for individual developers and small teams
- **Enterprise Edition:** For organizations with advanced security and compliance needs, includes priority support and advanced security features

See our [pricing page](https://www.attunehq.com/pricing) for a detailed comparison of features between editions.

We currently support publishing APT (Debian and Ubuntu) repositories, with more [coming soon](https://attunehq.com).

## Quick Start

Here's how to set up an APT repository in about 5 minutes.

### Prerequisites

- **Docker**: Required for running the Attune control plane and required services (PostgreSQL and MinIO)
- **Rust**: Required for building the Attune CLI
- **GnuPG** (`gpg`): Required for signing packages

### Setup

```bash
# 1. Clone the repository
git clone https://github.com/attunehq/attune.git
cd attune

# 2. Set up environment variables
cp .env.example .env
# Modify values in .env as needed for your setup

# 3. Start the control plane and supporting services
docker compose up -d
# This starts:
# - Attune control plane on port 3000
# - PostgreSQL on port 5432 (default database: attune, default credentials: attune/attune)
# - MinIO on ports 9000/9001 (default credentials: attuneminio/attuneminio)

# 4. Build and install the CLI
cargo install --path ./packages/attune

# 5. Generate a GPG key (if you don't already have one)
gpg --generate-key

# 6. Get the key ID of your secret key
gpg --list-secret-keys
# Note the 40 character hexadecimal string next to the `sec` entries

# 7. Create a new repository
attune apt repo create example.com

# 8. Add a package to your repository
attune apt pkg add --repo example.com --key-id $YOUR_GPG_KEY_ID $PATH_TO_DEB
# Replace:
# - $YOUR_GPG_KEY_ID with your GPG key ID from step 6
# - $PATH_TO_DEB with the path to your .deb package
```

For more detailed setup instructions and configuration options, refer to the [self-hosting guide](./docs/user-guide/self-hosting.md).

## License

Attune is [Apache 2 licensed](./LICENSE).
