---
sidebar_position: 1
---

# Getting Started

This guide provides detailed steps for getting started with Attune Publishing.

## Prerequisites

Before you begin, ensure you have the following installed:

- **Docker**: Required for running the Attune control plane and required services (PostgreSQL and MinIO)

## 1. Install the Attune CLI

You have two options for installing the Attune CLI:

### Option A: Install from packages (Recommended)

**macOS:**
```bash
# Add the Attune Homebrew tap
brew tap attunehq/attune

# Install Attune CLI
brew install attune
```

**Linux:**
```bash
# Download the latest .deb package from the GitHub releases page:
# https://github.com/attunehq/attune/releases/latest

# Install using apt
sudo apt install ./attune_VERSION_Linux_X64.deb
```

### Option B: Build from source

If you prefer to build from source, you'll need **Rust** installed:

```bash
# Clone the repository first
git clone https://github.com/attunehq/attune.git
cd attune

# Build and install from source
cargo install --path ./packages/attune
```

## 2. Clone the Repository (if not already done)

If you installed via packages, you'll still need the repository for the control plane:

```bash
git clone https://github.com/attunehq/attune.git
cd attune
```

## 3. Set Up Environment Variables

Copy the example environment file and modify it as needed:

```bash
cp .env.example .env
```

Make sure the values in the `.env` file match your local setup. The defaults should work with the Docker Compose configuration.

Consider using [direnv](https://direnv.net/) to manage environment variables.

## 4. Spin Up Docker Containers

Start the Attune control plane, PostgreSQL and MinIO in the background:

```bash
docker compose up -d
```

This will start:
- Attune control plane on port 3000
- PostgreSQL on port 5432 (default credentials: attune/attune)
- MinIO on ports 9000/9001 (default credentials: attuneminio/attuneminio)

You can check if the containers are running with:

```bash
docker compose ps
```

## 5. Generate a GPG Key

This step is only required if you don't already have a GPG key.

```bash
gpg --generate-key
```

Get the key ID of the secret key you'd like to use to sign. This should be a 40 character hexadecimal string.

```bash
gpg --list-secret-keys
```

## 6. Test Your Setup

Create a new Debian repository:

```bash
attune repo create -u 'http://localhost:9000/debian'
```

This should return a repository ID that you'll use in the next step.

Add a package to your newly created repository:

```bash
attune repo -r 1 pkg add path-to-package -i gpg-key-id
```

Replace `1` with the repository ID from the previous step, `path-to-package` with the path to the package you want to add, and `gpg-key-id` with the key ID of the secret key you'd like to use to sign.