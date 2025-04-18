# Attune
[![GitHub license](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/attunehq/attune/blob/main/LICENSE)

Attune is a tool for securely publishing and hosting Linux packages.

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

### Requirements

- Docker (`docker`)
- GnuPG (`gpg`)

### Setup

```bash
############## First, we need to set up the Attune backend.

# 1. Clone the repository.
git clone git@github.com:attunehq/attune.git && cd attune

# 2. Set up environment variables by copying from .env.example. You should
#    modify these values as needed to suit your deployment, especially
#    ATTUNE_SECRET.
cp .env.example .env

# 3. Start the control plane and supporting services.
docker compose up -d

############## Now, we'll install the CLI and set up a repository.

# 4. Install the Attune CLI by downloading it from GitHub Releases.

# 5. Create a repository.
attune repo create -u 'http://localhost:9000/debian' -d bookworm

# 6. Prepare a `.deb` package to host. One way you can do this is by downloading
#    a sample package e.g. `https://cdn.teleport.dev/teleport_17.4.4_arm64.deb`.

# 7. Add the package to the repository
attune repo pkg -r 1 add -c stable/v17 ~/Downloads/teleport_17.4.4_arm64.deb

############## Lastly, we'll use a GPG key to sign and deploy the repository.

# 8. If needed, generate a GPG key.
gpg --generate-key

# 9. Get the key ID of the secret key you'd like to use to sign.
gpg --list-secret-keys

# 10. Export your signing key (replace $KEYID with your key ID, which is the 40
#     character string next to the `sec` entries in each key).
gpg --armor --export-secret-keys --output demo-key.asc $KEYID

# 11. Sign and deploy the repository.
attune repo -r 1 sync -k demo-key.asc

############## Congratulations, you're done!
```

For more detailed setup instructions and configuration options, refer to the [user guide](./docs/user-guide/README.md).

## License

Attune is [Apache 2 licensed](./LICENSE).
