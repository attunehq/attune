# Attune Publishing Self-Hosting Guide

This guide provides detailed steps for self-hosting Attune Publishing.

## Prerequisites

Before you begin, ensure you have the following installed:

- **Docker**: Required for running the Attune control plane and backing services (PostgreSQL and MinIO)
- **Go**: Required for building the Attune CLI

## 1. Clone the Repository

```bash
git clone https://github.com/attunehq/attune.git
cd attune
```

## 2. Set Up Environment Variables

Copy the example environment file and modify it as needed:

```bash
cp .env.example .env
```

Make sure the values in the `.env` file match your local setup. The defaults should work with the Docker Compose configuration.

Consider using direnv](https://direnv.net/) to manage environment variables.

## 3. Spin Up Docker Containers

Start the required services (Attune control plane, PostgreSQL and MinIO) in the background:

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

## 4. Build and install the CLI

Navigate to the CLI directory and build the Go binary:

```bash
cd cli
go build -o attune ./cmd/attune
```

Move the CLI binary to your Go path:

```bash
mv attune ~/go/bin
```

Make sure `~/go/bin` is in your PATH. If not, add it:

```bash
export PATH=$PATH:~/go/bin
```

## 11. Test Basic CLI Functionality

Set the required environment variables for the CLI:

```bash
export ATTUNE_API_TOKEN=your-value  # The token value you created earlier
export ATTUNE_API_ENDPOINT=http://localhost:3000
```

Test that the CLI can connect to the control plane:

```bash
attune repo list
```

If this returns an empty list without errors, your setup is working correctly.

## 13. Create a Test Repository

Create a new repository pointing to the MinIO instance:

```bash
attune repo create -u 'http://localhost:9000/debian'
```

This should return a repository ID that you'll use in the next step.

## 14. Add a Package to the Repository

Add a package to your newly created repository:

```bash
attune repo pkg -r 1 add path-to-package
```

Replace `1` with the repository ID from the previous step and `path-to-package` with the path to the package you want to add.

## Troubleshooting Tips

If you encounter issues:

1. Check the logs of the Docker containers:
   ```bash
   docker compose logs postgres
   docker compose logs minio
   ```

2. Use `RUST_LOG=debug` to get more detailed logs from the control plane.

3. For CLI issues, try running with the `-v` flag for verbose output:
   ```bash
   attune -v repo list
   ```

4. Remember to check error messages carefully - they often contain valuable information about what went wrong and how to fix it.

5. Use console.log() or println!() statements to debug specific parts of the code if needed.

6. For database-related issues, you can connect directly to the PostgreSQL database:
   ```bash
   psql -h localhost -U attune -d attune
   ```

7. For MinIO issues, you can access the web interface at http://localhost:9001 with the credentials specified in docker-compose.yml.
