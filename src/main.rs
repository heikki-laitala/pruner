mod cli;
mod context;
mod db;
mod indexer;
mod languages;
mod parser;
mod query;
mod tokens;

fn main() -> anyhow::Result<()> {
    cli::run()
}
