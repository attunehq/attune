BEGIN;

-- Drop and recreate foreign keys with CASCADE behavior

-- 1. debian_repository_architecture
ALTER TABLE debian_repository_architecture 
    DROP CONSTRAINT debian_repository_architecture_repository_id_fkey,
    ADD CONSTRAINT debian_repository_architecture_repository_id_fkey 
        FOREIGN KEY (repository_id) 
        REFERENCES debian_repository(id) 
        ON UPDATE CASCADE 
        ON DELETE CASCADE;

-- 2. debian_repository_component
ALTER TABLE debian_repository_component
    DROP CONSTRAINT debian_repository_component_repository_id_fkey,
    ADD CONSTRAINT debian_repository_component_repository_id_fkey 
        FOREIGN KEY (repository_id) 
        REFERENCES debian_repository(id) 
        ON UPDATE CASCADE 
        ON DELETE CASCADE;

-- 3. debian_repository_index_release
ALTER TABLE debian_repository_index_release
    DROP CONSTRAINT debian_repository_index_release_repository_id_fkey,
    ADD CONSTRAINT debian_repository_index_release_repository_id_fkey 
        FOREIGN KEY (repository_id) 
        REFERENCES debian_repository(id) 
        ON UPDATE CASCADE 
        ON DELETE CASCADE;

-- 4. debian_repository_index_packages
ALTER TABLE debian_repository_index_packages
    DROP CONSTRAINT debian_repository_index_packages_repository_id_fkey,
    ADD CONSTRAINT debian_repository_index_packages_repository_id_fkey 
        FOREIGN KEY (repository_id) 
        REFERENCES debian_repository(id) 
        ON UPDATE CASCADE 
        ON DELETE CASCADE;

ALTER TABLE debian_repository_index_packages
    DROP CONSTRAINT debian_repository_index_packages_component_id_fkey,
    ADD CONSTRAINT debian_repository_index_packages_component_id_fkey 
        FOREIGN KEY (component_id) 
        REFERENCES debian_repository_component(id) 
        ON UPDATE CASCADE 
        ON DELETE CASCADE;

ALTER TABLE debian_repository_index_packages
    DROP CONSTRAINT debian_repository_index_packages_architecture_id_fkey,
    ADD CONSTRAINT debian_repository_index_packages_architecture_id_fkey 
        FOREIGN KEY (architecture_id) 
        REFERENCES debian_repository_architecture(id) 
        ON UPDATE CASCADE 
        ON DELETE CASCADE;

-- 5. debian_repository_index_contents
ALTER TABLE debian_repository_index_contents
    DROP CONSTRAINT debian_repository_index_contents_repository_id_fkey,
    ADD CONSTRAINT debian_repository_index_contents_repository_id_fkey 
        FOREIGN KEY (repository_id) 
        REFERENCES debian_repository(id) 
        ON UPDATE CASCADE 
        ON DELETE CASCADE;

ALTER TABLE debian_repository_index_contents
    DROP CONSTRAINT debian_repository_index_contents_component_id_fkey,
    ADD CONSTRAINT debian_repository_index_contents_component_id_fkey 
        FOREIGN KEY (component_id) 
        REFERENCES debian_repository_component(id) 
        ON UPDATE CASCADE 
        ON DELETE CASCADE;

-- 6. debian_repository_package
ALTER TABLE debian_repository_package
    DROP CONSTRAINT debian_repository_package_repository_id_fkey,
    ADD CONSTRAINT debian_repository_package_repository_id_fkey 
        FOREIGN KEY (repository_id) 
        REFERENCES debian_repository(id) 
        ON UPDATE CASCADE 
        ON DELETE CASCADE;

ALTER TABLE debian_repository_package
    DROP CONSTRAINT debian_repository_package_architecture_id_fkey,
    ADD CONSTRAINT debian_repository_package_architecture_id_fkey 
        FOREIGN KEY (architecture_id) 
        REFERENCES debian_repository_architecture(id) 
        ON UPDATE CASCADE 
        ON DELETE CASCADE;

ALTER TABLE debian_repository_package
    DROP CONSTRAINT debian_repository_package_component_id_fkey,
    ADD CONSTRAINT debian_repository_package_component_id_fkey 
        FOREIGN KEY (component_id) 
        REFERENCES debian_repository_component(id) 
        ON UPDATE CASCADE 
        ON DELETE CASCADE;

COMMIT;
