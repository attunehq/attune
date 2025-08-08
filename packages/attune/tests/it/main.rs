use std::convert::identity;

#[sqlx::test(migrator = "attune::testing::MIGRATOR")]
async fn migrations_applied(pool: sqlx::PgPool) {
    let table_exists = sqlx::query!(
        "SELECT EXISTS (
            SELECT FROM information_schema.tables
            WHERE table_schema = 'public'
            AND table_name = 'attune_tenant'
        ) as exists",
    )
    .fetch_one(&pool)
    .await
    .expect("Failed to check if attune_tenant table exists");

    assert!(
        table_exists.exists.is_some_and(identity),
        "attune_tenant table should exist after migrations"
    );
}
