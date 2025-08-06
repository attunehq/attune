-- Setup script for testing multi-architecture package index generation
-- This creates a repository with packages in multiple architectures to verify
-- that each architecture's index only contains packages for that architecture.
--
-- TODO: we should probably split these into multiple fixtures for easier testing.

INSERT INTO attune_tenant (id, display_name, subdomain, created_at, updated_at)
VALUES (1, 'TEST_TENANT', 'test', NOW(), NOW())
ON CONFLICT (id) DO NOTHING;

INSERT INTO debian_repository (id, tenant_id, name, s3_bucket, s3_prefix, created_at, updated_at)
VALUES (1000, 1, 'test-multi-arch', 'attune-test-0', '1/test-multi-arch', NOW(), NOW());

INSERT INTO debian_repository_release (id, repository_id, distribution, description, origin, label, version, suite, codename, contents, created_at, updated_at)
VALUES (1000, 1000, 'stable', 'Test Distribution', 'Test Origin', 'Test Label', '1.0', 'stable', 'stable', 'dummy content', NOW(), NOW());

INSERT INTO debian_repository_component (id, release_id, name, created_at, updated_at)
VALUES (1000, 1000, 'main', NOW(), NOW());

INSERT INTO debian_repository_package (id, tenant_id, package, version, architecture, maintainer, description, paragraph, size, s3_bucket, md5sum, sha1sum, sha256sum, created_at, updated_at)
VALUES (
    1001,
    1,
    'test-package',
    '1.0.0',
    'amd64'::debian_repository_architecture,
    'test@example.com',
    'Test package for amd64',
    '{"Package": "test-package", "Version": "1.0.0", "Architecture": "amd64", "Maintainer": "test@example.com", "Description": "Test package for amd64"}'::jsonb,
    1024,
    'attune-test-0',
    'amd64md5sum',
    'amd64sha1sum',
    'amd64sha256sum',
    NOW(),
    NOW()
);

INSERT INTO debian_repository_package (id, tenant_id, package, version, architecture, maintainer, description, paragraph, size, s3_bucket, md5sum, sha1sum, sha256sum, created_at, updated_at)
VALUES (
    1002,
    1,
    'test-package',
    '1.0.0',
    'arm64'::debian_repository_architecture,
    'test@example.com',
    'Test package for arm64',
    '{"Package": "test-package", "Version": "1.0.0", "Architecture": "arm64", "Maintainer": "test@example.com", "Description": "Test package for arm64"}'::jsonb,
    1024,
    'attune-test-0',
    'arm64md5sum',
    'arm64sha1sum',
    'arm64sha256sum',
    NOW(),
    NOW()
);

INSERT INTO debian_repository_component_package (component_id, package_id, filename, created_at, updated_at)
VALUES
    (1000, 1001, 'pool/main/t/test-package/test-package_1.0.0_amd64.deb', NOW(), NOW()),
    (1000, 1002, 'pool/main/t/test-package/test-package_1.0.0_arm64.deb', NOW(), NOW());

INSERT INTO debian_repository_index_packages (component_id, architecture, compression, size, contents, md5sum, sha1sum, sha256sum, created_at, updated_at)
VALUES
    (1000, 'amd64'::debian_repository_architecture, NULL, 500,
     E'Package: test-package\nVersion: 1.0.0\nArchitecture: amd64\n\nPackage: test-package\nVersion: 1.0.0\nArchitecture: arm64\n'::bytea,
     'oldamd64md5', 'oldamd64sha1', 'oldamd64sha256', NOW(), NOW()),
    (1000, 'arm64'::debian_repository_architecture, NULL, 500,
     E'Package: test-package\nVersion: 1.0.0\nArchitecture: amd64\n\nPackage: test-package\nVersion: 1.0.0\nArchitecture: arm64\n'::bytea,
     'oldarm64md5', 'oldarm64sha1', 'oldarm64sha256', NOW(), NOW());
