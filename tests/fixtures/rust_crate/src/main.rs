mod server;
mod db;

fn main() {
    let pool = db::create_pool("postgres://localhost/app");
    server::start(pool);
}
