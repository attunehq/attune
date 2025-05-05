BEGIN;

ALTER TABLE debian_repository ADD COLUMN s3_bucket TEXT;
UPDATE debian_repository SET s3_bucket = 'attune-dev-0';
ALTER TABLE debian_repository ALTER COLUMN s3_bucket SET NOT NULL;

ALTER TABLE debian_repository ADD COLUMN s3_prefix TEXT;
UPDATE debian_repository SET s3_prefix = '';
ALTER TABLE debian_repository ALTER COLUMN s3_prefix SET NOT NULL;

CREATE UNIQUE INDEX ON debian_repository (s3_bucket, s3_prefix, distribution);

COMMIT;
