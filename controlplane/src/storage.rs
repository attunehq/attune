use aws_sdk_s3::Client;

#[derive(Debug)]
pub struct S3Store {
    client: Client,
    bucket_name: String,
}

impl S3Store {
    fn new() -> Self {
        todo!()
    }

    // (repo_prefix, distribution, release_files { contents, clearsigned, detached })
    fn upload_release_files() {}

    fn upload_package() {}

    fn upload_package_index() {}
}
