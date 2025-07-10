-- CreateEnum
CREATE TYPE "debian_repository_architecture" AS ENUM ('amd64', 'arm64', 'armel', 'armhf', 'i386', 'ppc64el', 'riscv64', 's390x', 'alpha', 'arm', 'avr32', 'hppa', 'hurd-i386', 'hurd-amd64', 'ia64', 'kfreebsd-amd64', 'kfreebsd-i386', 'loong64', 'm32', 'm68k', 'mips', 'mipsel', 'mips64el', 'netbsd-i386', 'netbsd-alpha', 'or1k', 'powerpc', 'powerpcspe', 'ppc64', 's390', 'sparc', 'sparc64', 'sh4', 'x32');

-- CreateEnum
CREATE TYPE "debian_repository_package_staging_status" AS ENUM ('add', 'remove');

-- CreateEnum
CREATE TYPE "debian_repository_index_compression" AS ENUM ('xz', 'gz', 'bz2', 'lzma');

-- CreateTable
CREATE TABLE "attune_tenant" (
    "id" BIGSERIAL NOT NULL,
    "display_name" TEXT NOT NULL,
    "subdomain" TEXT NOT NULL,
    "created_at" TIMESTAMPTZ(6) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" TIMESTAMPTZ(6) NOT NULL,

    CONSTRAINT "attune_tenant_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "attune_tenant_api_token" (
    "id" BIGSERIAL NOT NULL,
    "tenant_id" BIGINT NOT NULL,
    "name" TEXT NOT NULL,
    "token" BYTEA NOT NULL,
    "created_at" TIMESTAMPTZ(6) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" TIMESTAMPTZ(6) NOT NULL DEFAULT CURRENT_TIMESTAMP,

    CONSTRAINT "attune_tenant_api_token_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "debian_repository" (
    "id" BIGSERIAL NOT NULL,
    "tenant_id" BIGINT NOT NULL,
    "uri" TEXT NOT NULL,
    "s3_bucket" TEXT NOT NULL,
    "s3_prefix" TEXT NOT NULL,
    "created_at" TIMESTAMPTZ(6) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" TIMESTAMPTZ(6) NOT NULL,

    CONSTRAINT "debian_repository_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "debian_repository_release" (
    "id" BIGSERIAL NOT NULL,
    "repository_id" BIGINT NOT NULL,
    "distribution" TEXT NOT NULL,
    "description" TEXT,
    "origin" TEXT,
    "label" TEXT,
    "version" TEXT,
    "suite" TEXT NOT NULL,
    "codename" TEXT NOT NULL,
    "contents" TEXT NOT NULL,
    "clearsigned" TEXT,
    "detached" TEXT,
    "created_at" TIMESTAMPTZ(6) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" TIMESTAMPTZ(6) NOT NULL,

    CONSTRAINT "debian_repository_release_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "debian_repository_component" (
    "id" BIGSERIAL NOT NULL,
    "release_id" BIGINT NOT NULL,
    "name" TEXT NOT NULL,
    "created_at" TIMESTAMPTZ(6) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" TIMESTAMPTZ(6) NOT NULL,

    CONSTRAINT "debian_repository_component_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "debian_repository_package" (
    "id" BIGSERIAL NOT NULL,
    "component_id" BIGINT NOT NULL,
    "architecture" "debian_repository_architecture" NOT NULL,
    "staging_status" "debian_repository_package_staging_status",
    "package" TEXT NOT NULL,
    "version" TEXT NOT NULL,
    "priority" TEXT,
    "section" TEXT,
    "installed_size" BIGINT,
    "maintainer" TEXT NOT NULL,
    "description" TEXT NOT NULL,
    "homepage" TEXT,
    "paragraph" JSONB NOT NULL,
    "depends" TEXT,
    "recommends" TEXT,
    "conflicts" TEXT,
    "provides" TEXT,
    "replaces" TEXT,
    "filename" TEXT NOT NULL,
    "size" BIGINT NOT NULL,
    "md5sum" TEXT NOT NULL,
    "sha1sum" TEXT NOT NULL,
    "sha256sum" TEXT NOT NULL,
    "created_at" TIMESTAMPTZ(6) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" TIMESTAMPTZ(6) NOT NULL,

    CONSTRAINT "debian_repository_package_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "debian_repository_index_packages" (
    "id" BIGSERIAL NOT NULL,
    "component_id" BIGINT NOT NULL,
    "architecture" "debian_repository_architecture" NOT NULL,
    "compression" "debian_repository_index_compression",
    "size" BIGINT NOT NULL,
    "contents" BYTEA NOT NULL,
    "md5sum" TEXT NOT NULL,
    "sha1sum" TEXT NOT NULL,
    "sha256sum" TEXT NOT NULL,
    "created_at" TIMESTAMPTZ(6) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" TIMESTAMPTZ(6) NOT NULL,

    CONSTRAINT "debian_repository_index_packages_pkey" PRIMARY KEY ("id")
);

-- CreateIndex
CREATE UNIQUE INDEX "attune_tenant_subdomain_key" ON "attune_tenant"("subdomain");

-- CreateIndex
CREATE UNIQUE INDEX "attune_tenant_api_token_token_key" ON "attune_tenant_api_token"("token");

-- CreateIndex
CREATE UNIQUE INDEX "debian_repository_uri_key" ON "debian_repository"("uri");

-- CreateIndex
CREATE UNIQUE INDEX "debian_repository_release_repository_id_distribution_key" ON "debian_repository_release"("repository_id", "distribution");

-- CreateIndex
CREATE UNIQUE INDEX "debian_repository_component_release_id_name_key" ON "debian_repository_component"("release_id", "name");

-- CreateIndex
CREATE UNIQUE INDEX "debian_repository_package_component_id_package_version_arch_key" ON "debian_repository_package"("component_id", "package", "version", "architecture");

-- CreateIndex
CREATE UNIQUE INDEX "debian_repository_index_packages_component_id_architecture_key" ON "debian_repository_index_packages"("component_id", "architecture");

-- AddForeignKey
ALTER TABLE "attune_tenant_api_token" ADD CONSTRAINT "attune_tenant_api_token_tenant_id_fkey" FOREIGN KEY ("tenant_id") REFERENCES "attune_tenant"("id") ON DELETE CASCADE ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "debian_repository" ADD CONSTRAINT "debian_repository_tenant_id_fkey" FOREIGN KEY ("tenant_id") REFERENCES "attune_tenant"("id") ON DELETE CASCADE ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "debian_repository_release" ADD CONSTRAINT "debian_repository_release_repository_id_fkey" FOREIGN KEY ("repository_id") REFERENCES "debian_repository"("id") ON DELETE CASCADE ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "debian_repository_component" ADD CONSTRAINT "debian_repository_component_release_id_fkey" FOREIGN KEY ("release_id") REFERENCES "debian_repository_release"("id") ON DELETE CASCADE ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "debian_repository_package" ADD CONSTRAINT "debian_repository_package_component_id_fkey" FOREIGN KEY ("component_id") REFERENCES "debian_repository_component"("id") ON DELETE CASCADE ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "debian_repository_index_packages" ADD CONSTRAINT "debian_repository_index_packages_component_id_fkey" FOREIGN KEY ("component_id") REFERENCES "debian_repository_component"("id") ON DELETE CASCADE ON UPDATE CASCADE;
