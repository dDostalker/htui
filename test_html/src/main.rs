use anyhow::Result;
use test_html::helloworld::test_hello_world;

fn main() -> Result<()> {
    test_hello_world().unwrap();
    Ok(())
}
