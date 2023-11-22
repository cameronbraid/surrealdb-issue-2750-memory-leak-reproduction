use std::time::Instant;

use color_eyre::eyre::Result;
use fake::Fake;
use fake::{Dummy, Faker};
use serde::{Deserialize, Serialize};
use surrealdb::{engine::any::Any, Surreal};
use surrealdb::{error, Error};
use tokio::task::JoinSet;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let surreal = surrealdb::engine::any::connect(format!("ws://localhost:12773"))
        .await
        .unwrap();

    surreal.use_ns("test").use_db("test").await.unwrap();

    for _ in 0..14 {
        let start = Instant::now();
        init_task(surreal.clone()).await;
        println!("Write Duration: {:?}", start.elapsed());

        let start = Instant::now();
        let mut join_set = JoinSet::new();
        for _ in 1..10_000 {
            join_set.spawn(run_task(surreal.clone()));
        }
        while let Some(data) = join_set.join_next().await {
            data.unwrap();
        }
        println!("Read Duration: {:?}", start.elapsed());
    }
    Ok(())
}

#[derive(Debug, Dummy, Serialize, Deserialize, Clone)]
pub struct Foo {
    pk: u64,
    str_1: String,
    str_2: String,
    str_3: String,
    paid: bool,
}

#[derive(Debug, Dummy, Serialize, Deserialize, Clone)]
pub struct Bar {
    pk: u64,
    str_1: String,
    str_2: String,
    str_3: String,
    paid: bool,
}

const NUM: u64 = 1000;

async fn run_task(db: Surreal<Any>) {
    let foo_id = Faker.fake::<u64>() % NUM;
    let bar_id = Faker.fake::<u64>() % NUM;
    db.query(
        "
        select (select ->has->bar from $parent) as bar, (select * from $parent) as foo from foo:[$foo_id] parallel ;

        select (select <-has<-foo from $parent) as foo, (select * from $parent) as bar from bar:[$bar_id] parallel ;

        "
    )
    .bind(("foo_id", foo_id))
    .bind(("bar_id", bar_id)).await
    .unwrap()
    .check()
    .unwrap();
}
async fn init_task(surreal: Surreal<Any>) {
    surreal
        .query("delete foo return none parallel; delete bar return none parallel")
        .await
        .unwrap()
        .check()
        .unwrap();

    let data: Vec<(Foo, Bar)> = (1..NUM + 1)
        .map(|id| {
            let mut foo = Faker.fake::<Foo>();
            foo.pk = id;
            let mut bar = Faker.fake::<Bar>();
            bar.pk = id;
            (foo, bar)
        })
        .collect();

    let mut join_set = JoinSet::new();
    for (foo, bar) in data {
        join_set.spawn({
            let db = surreal.clone();
            async move {
                let mut retries = 0;
                loop {
                    match db
                        .query(
                            "
                            create foo:[$foo.pk] content $foo return none; 
                            relate foo:[$foo.pk]->has->bar:[$bar.pk] return none;
                ",
                        )
                        .bind(("foo", &foo))
                        .await
                        .unwrap()
                        .check()
                    {
                        // retry https://github.com/surrealdb/surrealdb/issues/2862
                        Err(Error::Api(error::Api::Query(d)))
                            if d.contains("Resource busy:") && retries < 50 =>
                        {
                            // retry
                            retries += 1;
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        }
                        Ok(_) => {
                            // ok
                            break;
                        }
                        Err(e) => {
                            // fail
                            panic!("{:?}", e);
                        }
                    }
                }
            }
        });
        join_set.spawn({
            let db = surreal.clone();
            async move {
                let mut retries = 0;
                loop {
                    match db
                        .query("create bar:[$bar.pk] content $bar return none;")
                        .bind(("bar", &bar))
                        .await
                        .unwrap()
                        .check()
                    {
                        // retry https://github.com/surrealdb/surrealdb/issues/2862
                        Err(Error::Api(error::Api::Query(d)))
                            if d.contains("Resource busy:") && retries < 50 =>
                        {
                            // retry
                            retries += 1;
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        }
                        Ok(_) => {
                            // ok
                            break;
                        }
                        Err(e) => {
                            // fail
                            panic!("{:?}", e);
                        }
                    }
                }
            }
        });
    }

    while let Some(res) = join_set.join_next().await {
        res.unwrap();
    }
}
