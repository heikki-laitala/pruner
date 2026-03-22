use crate::db::{Pool, get_user, create_session};

pub fn start(pool: Pool) {
    println!("Server starting on :8080");
    handle_requests(pool);
}

fn handle_requests(pool: Pool) {
    let user = get_user(&pool, "admin");
    if let Some(u) = user {
        let session = create_session(&pool, u.id);
        println!("Session: {}", session.token);
    }
}

pub fn health_check() -> &'static str {
    "ok"
}
