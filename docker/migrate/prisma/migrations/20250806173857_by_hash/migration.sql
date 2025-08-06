-- CreateTable
CREATE TABLE "debian_repository_by_hash_cleanup" (
    "id" BIGSERIAL NOT NULL,
    "component_id" BIGINT NOT NULL,
    "architecture" "debian_repository_architecture" NOT NULL,
    "s3_bucket" TEXT NOT NULL,
    "s3_prefix" TEXT NOT NULL,
    "md5sum" TEXT NOT NULL,
    "sha1sum" TEXT NOT NULL,
    "sha256sum" TEXT NOT NULL,
    "expires_at" TIMESTAMPTZ(6) NOT NULL,
    "created_at" TIMESTAMPTZ(6) NOT NULL DEFAULT CURRENT_TIMESTAMP,

    CONSTRAINT "debian_repository_by_hash_cleanup_pkey" PRIMARY KEY ("id")
);

-- CreateIndex
CREATE INDEX "debian_repository_by_hash_cleanup_component_id_architecture_idx" ON "debian_repository_by_hash_cleanup"("component_id", "architecture", "expires_at");

-- CreateIndex
CREATE INDEX "debian_repository_by_hash_cleanup_expires_at_idx" ON "debian_repository_by_hash_cleanup"("expires_at");
