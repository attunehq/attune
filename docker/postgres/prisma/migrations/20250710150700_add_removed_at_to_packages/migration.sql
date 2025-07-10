BEGIN;

-- Add removed_at timestamp column to debian_repository_package table.
ALTER TABLE debian_repository_package
ADD COLUMN removed_at TIMESTAMPTZ DEFAULT NULL;

-- Create an index for efficient querying of active packages.
CREATE INDEX idx_debian_repository_package_removed_at
ON debian_repository_package (removed_at)
WHERE removed_at IS NULL;

COMMIT;
