mod budget;
mod cli;
mod context;
mod db;
mod import_resolver;
mod indexer;
mod languages;
mod parser;
mod query;
mod tokens;
mod uninstall;
mod upgrade;

fn main() -> anyhow::Result<()> {
    cli::run()
}
