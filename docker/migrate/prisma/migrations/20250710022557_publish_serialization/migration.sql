/*
  Warnings:

  - Added the required column `fingerprint` to the `debian_repository_release` table without a default value. This is not possible if the table is not empty.

*/
-- AlterTable
ALTER TABLE "debian_repository_release" ADD COLUMN     "fingerprint" TEXT NOT NULL;
