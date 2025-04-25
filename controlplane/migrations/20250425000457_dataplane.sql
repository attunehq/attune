BEGIN;

ALTER TABLE debian_repository ADD COLUMN s3_bucket TEXT NOT NULL;
ALTER TABLE debian_repository ADD COLUMN s3_prefix TEXT NOT NULL;
ALTER TABLE debian_repository ADD UNIQUE INDEX (s3_bucket, s3_prefix);

COMMIT;
