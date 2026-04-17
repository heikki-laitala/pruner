mod budget;
mod cli;
mod context;
mod db;
mod indexer;
mod languages;
mod parser;
mod query;
mod synonyms;
mod tokens;
mod uninstall;
mod upgrade;

fn main() -> anyhow::Result<()> {
    cli::run()
}
