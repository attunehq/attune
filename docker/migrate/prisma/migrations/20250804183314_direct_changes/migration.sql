/*
  Warnings:

  - You are about to drop the column `component_id` on the `debian_repository_package` table. All the data in the column will be lost.
  - You are about to drop the column `filename` on the `debian_repository_package` table. All the data in the column will be lost.
  - You are about to drop the column `staging_status` on the `debian_repository_package` table. All the data in the column will be lost.
  - You are about to drop the column `fingerprint` on the `debian_repository_release` table. All the data in the column will be lost.
  - A unique constraint covering the columns `[tenant_id,name]` on the table `debian_repository` will be added. If there are existing duplicate values, this will fail.
  - A unique constraint covering the columns `[tenant_id,package,version,architecture]` on the table `debian_repository_package` will be added. If there are existing duplicate values, this will fail.
  - A unique constraint covering the columns `[tenant_id,sha256sum]` on the table `debian_repository_package` will be added. If there are existing duplicate values, this will fail.
  - Added the required column `name` to the `debian_repository` table without a default value. This is not possible if the table is not empty.
  - Added the required column `s3_bucket` to the `debian_repository_package` table without a default value. This is not possible if the table is not empty.
  - Added the required column `tenant_id` to the `debian_repository_package` table without a default value. This is not possible if the table is not empty.

*/
-- DropForeignKey
ALTER TABLE "debian_repository_package" DROP CONSTRAINT "debian_repository_package_component_id_fkey";

-- DropIndex
DROP INDEX "debian_repository_package_component_id_package_version_arch_key";

-- AlterTable
ALTER TABLE "debian_repository" ADD COLUMN     "name" TEXT NOT NULL,
ALTER COLUMN "uri" DROP NOT NULL;

-- AlterTable
ALTER TABLE "debian_repository_package" DROP COLUMN "component_id",
DROP COLUMN "filename",
DROP COLUMN "staging_status",
ADD COLUMN     "s3_bucket" TEXT NOT NULL,
ADD COLUMN     "tenant_id" BIGINT NOT NULL;

-- AlterTable
ALTER TABLE "debian_repository_release" DROP COLUMN "fingerprint";

-- DropEnum
DROP TYPE "debian_repository_package_staging_status";

-- CreateTable
CREATE TABLE "debian_repository_component_package" (
    "component_id" BIGINT NOT NULL,
    "package_id" BIGINT NOT NULL,
    "filename" TEXT NOT NULL,
    "created_at" TIMESTAMPTZ(6) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "updated_at" TIMESTAMPTZ(6) NOT NULL,

    CONSTRAINT "debian_repository_component_package_pkey" PRIMARY KEY ("component_id","package_id")
);

-- CreateIndex
CREATE UNIQUE INDEX "debian_repository_tenant_id_name_key" ON "debian_repository"("tenant_id", "name");

-- CreateIndex
CREATE UNIQUE INDEX "debian_repository_package_tenant_id_package_version_archite_key" ON "debian_repository_package"("tenant_id", "package", "version", "architecture");

-- CreateIndex
CREATE UNIQUE INDEX "debian_repository_package_tenant_id_sha256sum_key" ON "debian_repository_package"("tenant_id", "sha256sum");

-- AddForeignKey
ALTER TABLE "debian_repository_component_package" ADD CONSTRAINT "debian_repository_component_package_component_id_fkey" FOREIGN KEY ("component_id") REFERENCES "debian_repository_component"("id") ON DELETE CASCADE ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "debian_repository_component_package" ADD CONSTRAINT "debian_repository_component_package_package_id_fkey" FOREIGN KEY ("package_id") REFERENCES "debian_repository_package"("id") ON DELETE CASCADE ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "debian_repository_package" ADD CONSTRAINT "debian_repository_package_tenant_id_fkey" FOREIGN KEY ("tenant_id") REFERENCES "attune_tenant"("id") ON DELETE CASCADE ON UPDATE CASCADE;
